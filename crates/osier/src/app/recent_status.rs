//! Badges for projects that are not open (pinned, recent, scanned), per the
//! `projects.recent_status` setting:
//!
//! - `never`: nothing runs and nothing is stored.
//! - `cached`: values recorded while a project was open, kept in `<cache>/project-status.toml`.
//! - `watch`: plus one shared watcher on each repository's `.git` (root, `refs/heads`,
//!   `refs/remotes`). A change there (commit, checkout, fetch from any tool) queues a refs-only
//!   ahead/behind check. No working-tree scan, so no change marker in this mode.
//! - `check`: plus a full `git status` of every repository every N minutes and when the window
//!   regains focus.
//!
//! Background git runs one process at a time on a background thread, with a timeout. Only
//! failed checks reach the command log (`FailuresOnly`): successes would flood it. The UI thread only moves results
//! into a map. Our own commands do not write to `.git` (`GIT_OPTIONAL_LOCKS=0`), so they never
//! retrigger the watcher.

use std::collections::{BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use gpui_kit::*;
use notify::Watcher as _;
use osier_config::settings::RecentStatusMode;
use osier_config::status_cache::{RepoBadge, StatusCache};
use osier_git::GitRunner;

use crate::app::Osier;
use crate::notices::WindowNoticeExt as _;

pub(crate) struct RecentStatus {
    pub mode: RecentStatusMode,
    cache: StatusCache,
    cache_dirty: bool,
    /// Repositories currently shown and not open, in a stable order.
    tracked: Vec<PathBuf>,
    /// Watch roots and the repository each belongs to.
    roots: Vec<(PathBuf, PathBuf)>,
    watcher: Option<notify::RecommendedWatcher>,
    /// Bumped whenever the listed repositories change: a watch set built for an older list is
    /// dropped (`install_watch`).
    watch_generation: u64,
    /// Watch roots the watcher reported, drained by the tick loop.
    touched: Arc<Mutex<BTreeSet<PathBuf>>>,
    /// `(repo, full)`: full = worktree status, otherwise refs only.
    pub(crate) queue: VecDeque<(PathBuf, bool)>,
    pub(crate) busy: bool,
    last_sweep: Option<Instant>,
    runner: GitRunner,
}

impl std::fmt::Debug for RecentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecentStatus")
            .field("mode", &self.mode)
            .field("tracked", &self.tracked.len())
            .field("queue", &self.queue.len())
            .finish_non_exhaustive()
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Directories to watch for one repository: the git dir itself (HEAD, index, FETCH_HEAD,
/// packed-refs) and the branch and remote-branch ref folders. Handles `.git` files
/// (worktrees, submodules) and their `commondir`.
pub(crate) fn watch_targets(repo: &Path) -> Vec<(PathBuf, notify::RecursiveMode)> {
    use notify::RecursiveMode::{NonRecursive, Recursive};
    let dot = repo.join(".git");
    let git_dir = if dot.is_dir() {
        dot
    } else if let Ok(text) = std::fs::read_to_string(&dot) {
        match text.trim().strip_prefix("gitdir:") {
            Some(p) => {
                let p = PathBuf::from(p.trim());
                if p.is_absolute() { p } else { repo.join(p) }
            }
            None => return Vec::new(),
        }
    } else {
        return Vec::new();
    };
    let common = std::fs::read_to_string(git_dir.join("commondir"))
        .ok()
        .map(|c| {
            let c = PathBuf::from(c.trim());
            if c.is_absolute() { c } else { git_dir.join(c) }
        })
        .unwrap_or_else(|| git_dir.clone());
    let mut out = vec![(git_dir.clone(), NonRecursive)];
    if common != git_dir {
        out.push((common.clone(), NonRecursive));
    }
    out.push((common.join("refs/heads"), Recursive));
    out.push((common.join("refs/remotes"), Recursive));
    out.retain(|(p, _)| p.is_dir());
    out
}

/// A watch set to build for the listed repositories.
pub(crate) struct WatchRebuild {
    generation: u64,
    tracked: Vec<PathBuf>,
    touched: Arc<Mutex<BTreeSet<PathBuf>>>,
}

/// A built watch set: the watcher and its roots (watched path, repository).
pub(crate) struct BuiltWatch {
    watcher: notify::RecommendedWatcher,
    roots: Vec<(PathBuf, PathBuf)>,
}

/// Create a watcher over every repository's refs (a few hundred small watches; notify walks
/// the recursive ones, and each repository's `.git` is inspected). Blocking: runs in the
/// background.
fn build_watch(tracked: &[PathBuf], touched: Arc<Mutex<BTreeSet<PathBuf>>>) -> Option<BuiltWatch> {
    let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(event) = res else { return };
        if !matches!(
            event.kind,
            notify::EventKind::Create(_)
                | notify::EventKind::Modify(_)
                | notify::EventKind::Remove(_)
        ) {
            return;
        }
        if let Ok(mut set) = touched.lock() {
            for p in event.paths {
                let is_lock = p.extension().is_some_and(|e| e == "lock");
                if !is_lock {
                    set.insert(p);
                }
            }
        }
    });
    let mut watcher = match watcher {
        Ok(w) => w,
        Err(error) => {
            tracing::warn!(%error, "cannot watch recent projects");
            return None;
        }
    };
    let mut roots = Vec::new();
    for repo in tracked {
        for (root, mode) in watch_targets(repo) {
            match watcher.watch(&root, mode) {
                Ok(()) => roots.push((root, repo.clone())),
                Err(error) => tracing::debug!(%error, path = %root.display(), "watch failed"),
            }
        }
    }
    Some(BuiltWatch { watcher, roots })
}

/// The repository a watcher event belongs to: the longest watch root that prefixes the path.
pub(crate) fn repo_for_event(roots: &[(PathBuf, PathBuf)], path: &Path) -> Option<PathBuf> {
    roots
        .iter()
        .filter(|(root, _)| path.starts_with(root))
        .max_by_key(|(root, _)| root.as_os_str().len())
        .map(|(_, repo)| repo.clone())
}

impl RecentStatus {
    /// `log` receives only failed checks (see `FailuresOnly`).
    pub fn new(
        mode: RecentStatusMode,
        paths: &osier_config::Paths,
        log: Arc<crate::command_log::UiCommandSink>,
    ) -> Self {
        let cache = if mode == RecentStatusMode::Never {
            StatusCache::default()
        } else {
            StatusCache::load(paths)
        };
        Self {
            mode,
            cache,
            cache_dirty: false,
            tracked: Vec::new(),
            roots: Vec::new(),
            watcher: None,
            watch_generation: 0,
            touched: Arc::new(Mutex::new(BTreeSet::new())),
            queue: VecDeque::new(),
            busy: false,
            last_sweep: None,
            runner: GitRunner::new(Arc::new(crate::command_log::FailuresOnly(log)))
                .with_timeout(Duration::from_secs(20)),
        }
    }

    pub fn badge(&self, repo: &Path) -> Option<RepoBadge> {
        if self.mode == RecentStatusMode::Never {
            return None;
        }
        self.cache.get(repo)
    }

    /// Whether the change marker is meaningful in this mode.
    pub fn shows_changes(&self) -> bool {
        matches!(
            self.mode,
            RecentStatusMode::Cached | RecentStatusMode::Check
        )
    }

    fn watching(&self) -> bool {
        matches!(self.mode, RecentStatusMode::Watch | RecentStatusMode::Check)
    }

    fn enqueue(&mut self, repo: PathBuf, full: bool) {
        match self.queue.iter_mut().find(|(r, _)| *r == repo) {
            // A full check also answers a refs check.
            Some(entry) => entry.1 |= full,
            None => self.queue.push_back((repo, full)),
        }
    }

    /// Start or stop everything for a new mode. Never: drop watcher, queue and in-memory cache
    /// (the file stays, it is only a hint).
    pub fn set_mode(&mut self, mode: RecentStatusMode, paths: &osier_config::Paths) {
        if mode == self.mode {
            return;
        }
        let was_never = self.mode == RecentStatusMode::Never;
        self.mode = mode;
        self.queue.clear();
        self.tracked.clear();
        self.roots.clear();
        self.watcher = None;
        self.last_sweep = None;
        if mode == RecentStatusMode::Never {
            self.cache = StatusCache::default();
            self.cache_dirty = false;
        } else if was_never {
            self.cache = StatusCache::load(paths);
        }
    }

    /// Remember what an open project looks like right now.
    pub fn record(&mut self, repo: &Path, dirty: bool, ahead: u32, behind: u32) {
        if self.mode == RecentStatusMode::Never {
            return;
        }
        let badge = RepoBadge {
            dirty: Some(dirty),
            ahead,
            behind,
            checked_at: now(),
        };
        let previous = self.cache.get(repo);
        if previous.map(|p| (p.dirty, p.ahead, p.behind)) != Some((badge.dirty, ahead, behind)) {
            self.cache.set(repo, badge);
            self.cache_dirty = true;
        }
    }

    /// Note the repositories currently listed and queue the new ones. When the list changed,
    /// returns the watch set to build (in the background, [`build_watch`]) and hand to
    /// [`RecentStatus::install_watch`].
    fn sync_tracked(&mut self, tracked: Vec<PathBuf>) -> Option<WatchRebuild> {
        if tracked == self.tracked {
            return None;
        }
        let added: Vec<PathBuf> = tracked
            .iter()
            .filter(|p| !self.tracked.contains(p))
            .cloned()
            .collect();
        self.tracked = tracked;
        if !self.watching() {
            return None;
        }
        let full = self.mode == RecentStatusMode::Check;
        for repo in added {
            self.enqueue(repo, full);
        }
        self.watch_generation += 1;
        Some(WatchRebuild {
            generation: self.watch_generation,
            tracked: self.tracked.clone(),
            touched: self.touched.clone(),
        })
    }

    /// Take a watch set built by [`build_watch`], unless the list changed again meanwhile.
    fn install_watch(&mut self, generation: u64, built: Option<BuiltWatch>) {
        if generation != self.watch_generation {
            return;
        }
        if let Some(built) = built {
            tracing::debug!(
                repos = self.tracked.len(),
                watches = built.roots.len(),
                "recent status watching"
            );
            self.roots = built.roots;
            self.watcher = Some(built.watcher);
        }
    }

    /// Queue every tracked repository (sweep, window focus).
    pub fn queue_all(&mut self) {
        if !self.watching() {
            return;
        }
        let full = self.mode == RecentStatusMode::Check;
        for repo in self.tracked.clone() {
            self.enqueue(repo, full);
        }
    }

    /// "Scan now", following the mode: refs only for `watch`, full status for `check` (the
    /// sweep timer restarts), and a one-off full check for `cached`, which never runs git on
    /// its own. Returns how many repositories were queued.
    pub fn scan_now(&mut self, tracked: Vec<PathBuf>) -> usize {
        let full = match self.mode {
            RecentStatusMode::Never => return 0,
            RecentStatusMode::Watch => false,
            RecentStatusMode::Check => {
                self.last_sweep = Some(Instant::now());
                true
            }
            RecentStatusMode::Cached => true,
        };
        let n = tracked.len();
        for repo in tracked {
            self.enqueue(repo, full);
        }
        n
    }
}

impl Osier {
    /// Repositories shown in the sidebar that are not open.
    /// Check in the background which listed projects' folders are gone (pinned, recent and
    /// scanned ones), every few seconds from the tick loop. The sidebar and the project
    /// switcher read [`Osier::project_missing`] instead of asking the disk on every frame, which
    /// could stall a frame on a sleeping disk or a dead network mount.
    pub(crate) fn check_missing_projects(&mut self, cx: &mut Context<Self>) {
        const EVERY: std::time::Duration = std::time::Duration::from_secs(5);
        if self.missing_check_running
            || self
                .missing_checked_at
                .is_some_and(|at| at.elapsed() < EVERY)
        {
            return;
        }
        self.missing_check_running = true;
        let paths = self.recent_status_targets();
        self.in_background_quiet(
            move || {
                paths
                    .into_iter()
                    .filter(|p| !p.is_dir())
                    .collect::<std::collections::HashSet<PathBuf>>()
            },
            |this, missing, cx| {
                this.missing_check_running = false;
                this.missing_checked_at = Some(std::time::Instant::now());
                if missing != this.missing_projects {
                    this.missing_projects = missing;
                    cx.notify();
                }
            },
            cx,
        );
    }

    /// Whether a listed project's folder was gone at the last check.
    pub(crate) fn project_missing(&self, path: &Path) -> bool {
        self.missing_projects.contains(path)
    }

    fn recent_status_targets(&self) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = Vec::new();
        let open = |p: &PathBuf| self.projects.iter().any(|o| &o.path == p);
        for p in self
            .state
            .pinned
            .iter()
            .chain(self.state.recent.iter())
            .chain(self.scanned.iter())
        {
            if !open(p) && !out.contains(p) {
                out.push(p.clone());
            }
        }
        out
    }

    pub(crate) fn apply_recent_status_mode(&mut self, cx: &mut Context<Self>) {
        let mode = self.settings.projects.recent_status;
        self.recent_status.set_mode(mode, &self.paths);
        // Seed the cache from open projects so closing one leaves a fresh value behind.
        if mode != RecentStatusMode::Never {
            for p in &self.projects {
                if let Some(s) = &p.snapshot {
                    let b = &s.status.branch;
                    self.recent_status.record(
                        &p.path,
                        !s.status.entries.is_empty(),
                        b.ahead,
                        b.behind,
                    );
                }
            }
        }
        cx.notify();
    }

    /// Driven by the tick loop. Cheap when idle: compares a short list and drains a set.
    pub(crate) fn pump_recent_status(&mut self, cx: &mut Context<Self>) {
        if self.recent_status.mode == RecentStatusMode::Never {
            return;
        }
        let targets = self.recent_status_targets();
        if let Some(rebuild) = self.recent_status.sync_tracked(targets) {
            let generation = rebuild.generation;
            self.in_background_cx(
                crate::background::WATCHING_PROJECTS,
                move || build_watch(&rebuild.tracked, rebuild.touched),
                move |this, built, _| this.recent_status.install_watch(generation, built),
                cx,
            );
        }

        // Watcher events to repositories, refs-only check (a fetch or commit elsewhere).
        let touched: Vec<PathBuf> = match self.recent_status.touched.lock() {
            Ok(mut set) => std::mem::take(&mut *set).into_iter().collect(),
            Err(_) => Vec::new(),
        };
        let full_on_event = self.recent_status.mode == RecentStatusMode::Check;
        for path in touched {
            if let Some(repo) = repo_for_event(&self.recent_status.roots, &path) {
                self.recent_status.enqueue(repo, full_on_event);
            }
        }

        if self.recent_status.mode == RecentStatusMode::Check {
            let interval = self.settings.projects.recent_status_interval();
            let due = self
                .recent_status
                .last_sweep
                .is_none_or(|t| t.elapsed() >= interval);
            if due {
                self.recent_status.last_sweep = Some(Instant::now());
                self.recent_status.queue_all();
            }
        }

        if !self.recent_status.busy
            && let Some((repo, full)) = self.recent_status.queue.pop_front()
        {
            self.recent_status.busy = true;
            if self.settings.appearance.show_background_tasks {
                cx.notify();
            }
            let runner = self.recent_status.runner.clone();
            cx.spawn(async move |this, cx| {
                let path = repo.clone();
                let started = Instant::now();
                let result = cx
                    .background_spawn(async move {
                        if !path.is_dir() {
                            return None;
                        }
                        let r = if full {
                            osier_git::status::quick_full(&runner, &path)
                        } else {
                            osier_git::status::quick_refs(&runner, &path)
                        };
                        r.ok()
                    })
                    .await;
                tracing::debug!(repo = %repo.display(), full, ms = started.elapsed().as_millis() as u64, "recent status");
                let _ = this.update(cx, |this, cx| {
                    this.recent_status.busy = false;
                    if this.settings.appearance.show_background_tasks {
                        cx.notify();
                    }
                    if let Some(q) = result {
                        let rs = &mut this.recent_status;
                        let previous = rs.cache.get(&repo).unwrap_or_default();
                        let badge = RepoBadge {
                            // A refs-only check keeps the last known change marker.
                            dirty: q.dirty.or(previous.dirty),
                            ahead: q.ahead,
                            behind: q.behind,
                            checked_at: now(),
                        };
                        if (badge.dirty, badge.ahead, badge.behind)
                            != (previous.dirty, previous.ahead, previous.behind)
                        {
                            rs.cache.set(&repo, badge);
                            rs.cache_dirty = true;
                            cx.notify();
                        }
                    }
                });
            })
            .detach();
        }

        if self.recent_status.cache_dirty && !self.recent_status.busy {
            self.recent_status.cache_dirty = false;
            let (cache, paths) = (self.recent_status.cache.clone(), self.paths.clone());
            self.saves.push(move || {
                if let Err(error) = cache.save(&paths) {
                    tracing::warn!(%error, "could not save the project status cache");
                }
            });
        }
    }

    /// The "Scan now" button and palette command.
    pub(crate) fn scan_recent_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let targets = self.recent_status_targets();
        let n = self.recent_status.scan_now(targets);
        if self.recent_status.mode == RecentStatusMode::Never {
            return;
        }
        window.push_notice(
            match n {
                0 => {
                    "No projects to check: pinned and recent projects that are not open are checked"
                        .to_string()
                }
                1 => "Checking 1 project".to_string(),
                n => format!("Checking {n} projects"),
            },
            cx,
        );
        self.pump_recent_status(cx);
    }

    /// Window focus came back: in `check` mode, re-check everything (edits made elsewhere).
    pub(crate) fn recent_status_on_focus(&mut self) {
        if self.recent_status.mode == RecentStatusMode::Check {
            self.recent_status.queue_all();
        }
    }

    pub(crate) fn recent_badge(&self, repo: &Path) -> Option<RepoBadge> {
        self.recent_status.badge(repo)
    }

    /// Test hook: how many checks are waiting.
    #[cfg(test)]
    pub(crate) fn recent_status_queue_len(&self) -> usize {
        self.recent_status.queue.len()
    }

    #[cfg(test)]
    pub(crate) fn recent_status_busy(&self) -> bool {
        self.recent_status.busy
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{repo_for_event, watch_targets};

    #[test]
    fn events_map_to_the_repository_of_the_longest_root() {
        let roots = vec![
            (PathBuf::from("/r/a/.git"), PathBuf::from("/r/a")),
            (PathBuf::from("/r/a/.git/refs/heads"), PathBuf::from("/r/a")),
            (PathBuf::from("/r/b/.git"), PathBuf::from("/r/b")),
        ];
        assert_eq!(
            repo_for_event(&roots, Path::new("/r/a/.git/refs/heads/main")),
            Some(PathBuf::from("/r/a"))
        );
        assert_eq!(
            repo_for_event(&roots, Path::new("/r/b/.git/FETCH_HEAD")),
            Some(PathBuf::from("/r/b"))
        );
        assert_eq!(repo_for_event(&roots, Path::new("/r/c/x")), None);
    }

    #[test]
    fn watch_targets_cover_git_dir_and_ref_folders() {
        let t = osier_git::testing::TempRepo::new();
        t.commit_file("a.txt", "a\n", "first");
        let targets: Vec<PathBuf> = watch_targets(t.path())
            .into_iter()
            .map(|(p, _)| p)
            .collect();
        assert!(targets.contains(&t.path().join(".git")));
        assert!(targets.contains(&t.path().join(".git/refs/heads")));
        let none = tempfile::tempdir().unwrap();
        assert!(watch_targets(none.path()).is_empty());
    }
}
