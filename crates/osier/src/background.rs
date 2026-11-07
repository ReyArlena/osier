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

pub(crate) const SCAN_DIRS: &str = "Scanning project directories";
pub(crate) const GH_ACCOUNTS: &str = "Loading GitHub accounts";
pub(crate) const GH_REPOS: &str = "Loading GitHub repositories";
pub(crate) const GH_LISTS: &str = "Loading GitHub lists";
pub(crate) const BRANCHES: &str = "Looking up branches";

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
