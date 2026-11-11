//! Background work in the status bar.
//!
//! - Work the user started and is waiting for (opening a project, a file, an editor, the
//!   rebase list, a patch) always shows, once it has run for [`USER_DELAY`]: the quick cases
//!   never flash a label. Started with [`Background::begin_user`] /
//!   [`Osier::in_background_user`].
//! - Quiet work (project scans, GitHub lists, refreshes, diff, blame and branch lookups,
//!   reloads) shows only when `appearance.show_background_tasks` is on.
//! - Fetch, pull, push, clone and create always show (`remote_job`), whatever the setting.
//!
//! Work that has its own flag (a project's `loading`, `diff_loading`, the recent-status
//! scanner's `busy`) is read from it; the rest counts itself in and out with
//! a [`BgGuard`] from [`Background::begin`].

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::app::Osier;
use gpui_kit::{AppContext as _, Context, Window};

pub(crate) const SCAN_DIRS: &str = "Scanning project directories";
pub(crate) const GH_ACCOUNTS: &str = "Loading GitHub accounts";
pub(crate) const GH_REPOS: &str = "Loading GitHub repositories";
pub(crate) const GH_LISTS: &str = "Loading GitHub lists";
pub(crate) const BRANCHES: &str = "Looking up branches";
pub(crate) const UPDATE_CHECK: &str = "Checking for updates";
pub(crate) const UPDATE_INSTALL: &str = "Updating Osier";
pub(crate) const FILES_BADGES: &str = "Reading file status";
pub(crate) const OPENING_PROJECT: &str = "Opening project";
pub(crate) const LOADING_FILE: &str = "Loading file";
pub(crate) const LISTING_COMMITS: &str = "Listing commits";
pub(crate) const READING_PATCHES: &str = "Reading patches";
pub(crate) const SAVING_PATCH: &str = "Saving patch";
pub(crate) const READING_GIT_CONFIG: &str = "Reading git config";
pub(crate) const READING_COMMIT: &str = "Reading commit";
pub(crate) const READING_THEMES: &str = "Reading themes";
pub(crate) const WATCHING_PROJECTS: &str = "Watching projects";
pub(crate) const READING_SETTINGS: &str = "Reading settings";
pub(crate) const OPENING_FOLDER: &str = "Opening folder";

/// Counted background work, shared with the tasks doing it: each holds a [`BgGuard`] and
/// counts itself out when it finishes or is dropped (a closed dialog cancels its task). The
/// tick loop repaints the status bar when the count changed.
#[derive(Clone, Default)]
pub(crate) struct Background(Arc<Inner>);

/// How long work the user started runs before the status bar names it.
pub(crate) const USER_DELAY: Duration = Duration::from_millis(150);

#[derive(Default)]
struct Inner {
    running: Mutex<Vec<Running>>,
    next_id: AtomicU64,
    version: AtomicU64,
}

struct Running {
    id: u64,
    what: &'static str,
    started: Instant,
    /// Started by the user, who is waiting for it: shown whatever the setting says.
    user: bool,
}

impl Background {
    /// Quiet work: shown only with "Show background tasks" on.
    pub(crate) fn begin(&self, what: &'static str) -> BgGuard {
        self.start(what, false)
    }

    /// Work the user started and is waiting for: always shown, after [`USER_DELAY`].
    pub(crate) fn begin_user(&self, what: &'static str) -> BgGuard {
        self.start(what, true)
    }

    fn start(&self, what: &'static str, user: bool) -> BgGuard {
        let id = self.0.next_id.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut r) = self.0.running.lock() {
            r.push(Running {
                id,
                what,
                started: Instant::now(),
                user,
            });
        }
        self.0.version.fetch_add(1, Ordering::Relaxed);
        BgGuard {
            background: self.clone(),
            id,
        }
    }

    /// Labels to show, in a stable order: user-started work that has run for [`USER_DELAY`],
    /// plus quiet work when `include_quiet`.
    pub(crate) fn running(&self, include_quiet: bool) -> Vec<&'static str> {
        self.0
            .running
            .lock()
            .map(|r| {
                r.iter()
                    .filter(|w| {
                        if w.user {
                            w.started.elapsed() >= USER_DELAY
                        } else {
                            include_quiet
                        }
                    })
                    .map(|w| w.what)
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(crate) fn version(&self) -> u64 {
        self.0.version.load(Ordering::Relaxed)
    }
}

/// One piece of running work; dropping it counts it out.
pub(crate) struct BgGuard {
    background: Background,
    id: u64,
}

impl Drop for BgGuard {
    fn drop(&mut self) {
        if let Ok(mut r) = self.background.0.running.lock() {
            r.retain(|w| w.id != self.id);
        }
        self.background.0.version.fetch_add(1, Ordering::Relaxed);
    }
}

impl Osier {
    /// The standard way to do blocking work for the app view: git, other processes, file
    /// reads and writes, anything that can take more than a fraction of a millisecond.
    ///
    /// `work` runs on the background executor, never on the UI thread. While it runs it is
    /// counted under `label`, shown in the status bar when "Show background tasks" is on (use
    /// [`Osier::in_background_user`] for work the user is waiting for). Its result comes back
    /// to `done` on the UI thread, which stores it and decides what to redraw: `cx.notify()`
    /// when the main view shows it, `redraw_settings` when only the settings dialog does.
    /// Nothing happens if the app view is gone by then.
    pub(crate) fn in_background<T: Send + 'static>(
        &mut self,
        label: &'static str,
        work: impl FnOnce() -> T + Send + 'static,
        done: impl FnOnce(&mut Osier, T, &mut Window, &mut Context<Osier>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let busy = self.background.begin(label);
        Self::spawn_in_window(busy, false, work, done, window, cx);
    }

    /// [`Osier::in_background`] for work the user started and is waiting for (opening a
    /// project, a file, an editor, a dialog's data): its label shows in the status bar
    /// whatever the setting, once it has taken longer than [`USER_DELAY`].
    pub(crate) fn in_background_user<T: Send + 'static>(
        &mut self,
        label: &'static str,
        work: impl FnOnce() -> T + Send + 'static,
        done: impl FnOnce(&mut Osier, T, &mut Window, &mut Context<Osier>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let busy = self.background.begin_user(label);
        Self::spawn_in_window(busy, true, work, done, window, cx);
    }

    fn spawn_in_window<T: Send + 'static>(
        busy: BgGuard,
        user: bool,
        work: impl FnOnce() -> T + Send + 'static,
        done: impl FnOnce(&mut Osier, T, &mut Window, &mut Context<Osier>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if user {
            Self::show_after_delay(cx);
        }
        cx.spawn_in(window, async move |this, cx| {
            let result = cx.background_spawn(async move { work() }).await;
            drop(busy);
            let _ = this.update_in(cx, |this, window, cx| {
                done(this, result, window, cx);
                if user {
                    // Take the label away again.
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Redraw the status bar once user-started work has run for [`USER_DELAY`], so its label
    /// appears (a no-op when it finished first: the label is gone by then).
    fn show_after_delay(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(USER_DELAY).await;
            let _ = this.update(cx, |_, cx| cx.notify());
        })
        .detach();
    }

    /// [`Osier::in_background`] for callers without a window.
    pub(crate) fn in_background_cx<T: Send + 'static>(
        &mut self,
        label: &'static str,
        work: impl FnOnce() -> T + Send + 'static,
        done: impl FnOnce(&mut Osier, T, &mut Context<Osier>) + 'static,
        cx: &mut Context<Self>,
    ) {
        let busy = Some(self.background.begin(label));
        Self::spawn_background(busy, false, work, done, cx);
    }

    /// [`Osier::in_background_user`] for callers without a window.
    pub(crate) fn in_background_user_cx<T: Send + 'static>(
        &mut self,
        label: &'static str,
        work: impl FnOnce() -> T + Send + 'static,
        done: impl FnOnce(&mut Osier, T, &mut Context<Osier>) + 'static,
        cx: &mut Context<Self>,
    ) {
        let busy = Some(self.background.begin_user(label));
        Self::show_after_delay(cx);
        Self::spawn_background(busy, true, work, done, cx);
    }

    /// [`Osier::in_background_cx`] that does not show in the status bar: routine checks on a
    /// timer, which would otherwise make the background-tasks label flicker every few seconds.
    pub(crate) fn in_background_quiet<T: Send + 'static>(
        &mut self,
        work: impl FnOnce() -> T + Send + 'static,
        done: impl FnOnce(&mut Osier, T, &mut Context<Osier>) + 'static,
        cx: &mut Context<Self>,
    ) {
        Self::spawn_background(None, false, work, done, cx);
    }

    fn spawn_background<T: Send + 'static>(
        busy: Option<BgGuard>,
        user: bool,
        work: impl FnOnce() -> T + Send + 'static,
        done: impl FnOnce(&mut Osier, T, &mut Context<Osier>) + 'static,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { work() }).await;
            drop(busy);
            let _ = this.update(cx, |this, cx| {
                done(this, result, cx);
                if user {
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// What is running now, in a stable order: work the user is waiting for always, quiet
    /// work only with "Show background tasks" on.
    pub(crate) fn background_labels(&self) -> Vec<String> {
        let quiet = self.settings.appearance.show_background_tasks;
        let mut out: Vec<String> = Vec::new();
        if !quiet {
            out.extend(self.background.running(false).into_iter().map(String::from));
            return out;
        }
        if self.active_project().is_some_and(|p| p.loading) {
            out.push("Refreshing".into());
        }
        if self.diff_loading {
            out.push("Loading diff".into());
        }
        if self
            .blame
            .as_ref()
            .is_some_and(|b| b.lines.is_none() && b.error.is_none())
        {
            out.push("Loading blame".into());
        }
        if self.recent_status.busy {
            let left = self.recent_status.queue.len();
            out.push(if left > 0 {
                format!("Checking project status ({left} left)")
            } else {
                "Checking project status".into()
            });
        }
        out.extend(self.background.running(true).into_iter().map(String::from));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::Background;

    #[test]
    fn user_work_shows_after_the_delay_whatever_the_setting() {
        let bg = Background::default();
        let _quiet = bg.begin("Quiet");
        let user = bg.begin_user("Waiting");
        assert!(
            bg.running(false).is_empty(),
            "nothing at first: no flash for quick work"
        );
        assert_eq!(bg.running(true), ["Quiet"]);
        std::thread::sleep(super::USER_DELAY + std::time::Duration::from_millis(20));
        assert_eq!(
            bg.running(false),
            ["Waiting"],
            "the setting does not hide it"
        );
        assert_eq!(bg.running(true), ["Quiet", "Waiting"]);
        drop(user);
        assert!(bg.running(false).is_empty());
    }

    #[test]
    fn guards_count_in_and_out() {
        let bg = Background::default();
        let v0 = bg.version();
        let a = bg.begin("A");
        let b = bg.begin("A");
        let c = bg.begin("B");
        assert_eq!(bg.running(true), ["A", "B"]);
        drop(a);
        assert_eq!(bg.running(true), ["A", "B"]);
        drop((b, c));
        assert!(bg.running(true).is_empty());
        assert!(bg.version() > v0);
    }
}
