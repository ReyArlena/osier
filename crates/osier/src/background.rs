//! Quiet background work shown in the status bar when `appearance.show_background_tasks` is
//! on: project scans, GitHub lists, refreshes, diff, blame and branch lookups. Fetch, pull,
//! push, clone and create always show (`remote_job`), whatever the setting.
//!
//! Work that has its own flag (a project's `loading`, `diff_loading`, the recent-status
//! scanner's `busy`) is read from it; the rest counts itself in and out with
//! a [`BgGuard`] from [`Background::begin`].

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

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

#[derive(Default)]
struct Inner {
    running: Mutex<BTreeMap<&'static str, u32>>,
    version: AtomicU64,
}

impl Background {
    pub(crate) fn begin(&self, what: &'static str) -> BgGuard {
        if let Ok(mut r) = self.0.running.lock() {
            *r.entry(what).or_default() += 1;
        }
        self.0.version.fetch_add(1, Ordering::Relaxed);
        BgGuard {
            background: self.clone(),
            what,
        }
    }

    pub(crate) fn running(&self) -> Vec<&'static str> {
        self.0
            .running
            .lock()
            .map(|r| r.keys().copied().collect())
            .unwrap_or_default()
    }

    pub(crate) fn version(&self) -> u64 {
        self.0.version.load(Ordering::Relaxed)
    }
}

/// One piece of running work; dropping it counts it out.
pub(crate) struct BgGuard {
    background: Background,
    what: &'static str,
}

impl Drop for BgGuard {
    fn drop(&mut self) {
        if let Ok(mut r) = self.background.0.running.lock()
            && let Some(n) = r.get_mut(self.what)
        {
            *n = n.saturating_sub(1);
            if *n == 0 {
                r.remove(self.what);
            }
        }
        self.background.0.version.fetch_add(1, Ordering::Relaxed);
    }
}

impl Osier {
    /// The standard way to do blocking work for the app view: git, other processes, file
    /// reads and writes, anything that can take more than a fraction of a millisecond.
    ///
    /// `work` runs on the background executor, never on the UI thread. While it runs it is
    /// counted under `label` (the status bar's background-tasks spinner, when that setting is
    /// on). Its result comes back to `done` on the UI thread, which stores it and decides what
    /// to redraw: `cx.notify()` when the main view shows it, `redraw_settings` when only the
    /// settings dialog does. Nothing happens if the app view is gone by then.
    pub(crate) fn in_background<T: Send + 'static>(
        &mut self,
        label: &'static str,
        work: impl FnOnce() -> T + Send + 'static,
        done: impl FnOnce(&mut Osier, T, &mut Window, &mut Context<Osier>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let busy = self.background.begin(label);
        cx.spawn_in(window, async move |this, cx| {
            let result = cx.background_spawn(async move { work() }).await;
            drop(busy);
            let _ = this.update_in(cx, |this, window, cx| done(this, result, window, cx));
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
        Self::spawn_background(busy, work, done, cx);
    }

    /// [`Osier::in_background_cx`] that does not show in the status bar: routine checks on a
    /// timer, which would otherwise make the background-tasks label flicker every few seconds.
    pub(crate) fn in_background_quiet<T: Send + 'static>(
        &mut self,
        work: impl FnOnce() -> T + Send + 'static,
        done: impl FnOnce(&mut Osier, T, &mut Context<Osier>) + 'static,
        cx: &mut Context<Self>,
    ) {
        Self::spawn_background(None, work, done, cx);
    }

    fn spawn_background<T: Send + 'static>(
        busy: Option<BgGuard>,
        work: impl FnOnce() -> T + Send + 'static,
        done: impl FnOnce(&mut Osier, T, &mut Context<Osier>) + 'static,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { work() }).await;
            drop(busy);
            let _ = this.update(cx, |this, cx| done(this, result, cx));
        })
        .detach();
    }

    /// What is running now, in a stable order; empty when the setting is off.
    pub(crate) fn background_labels(&self) -> Vec<String> {
        if !self.settings.appearance.show_background_tasks {
            return Vec::new();
        }
        let mut out: Vec<String> = Vec::new();
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
        out.extend(self.background.running().into_iter().map(String::from));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::Background;

    #[test]
    fn guards_count_in_and_out() {
        let bg = Background::default();
        let v0 = bg.version();
        let a = bg.begin("A");
        let b = bg.begin("A");
        let c = bg.begin("B");
        assert_eq!(bg.running(), ["A", "B"]);
        drop(a);
        assert_eq!(bg.running(), ["A", "B"]);
        drop((b, c));
        assert!(bg.running().is_empty());
        assert!(bg.version() > v0);
    }
}
