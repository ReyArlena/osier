//! One open repository: loaded data, selection and background refresh.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gpui_kit::Task;
use osier_git::config::Identity;
use osier_git::graph::GraphRow;
use osier_git::log::{Commit, LogOptions};
use osier_git::reflog::ReflogEntry;
use osier_git::refs::{Branch, Remote};
use osier_git::repo::InProgress;
use osier_git::stash::StashEntry;
use osier_git::status::{RepoStatus, StatusOptions};
use osier_git::{GitRunner, Repository};

/// Everything read from the repository in one refresh.
///
/// The large lists are shared (`Arc`): views take them by pointer instead of copying them on
/// every frame, and caches can tell a new snapshot's list from the one they were built from.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub status: std::sync::Arc<RepoStatus>,
    /// Commits on local branches that no remote branch has yet (Log graph marking).
    pub unpushed: std::sync::Arc<std::collections::HashSet<String>>,
    pub branches: Vec<Branch>,
    pub tags: Vec<osier_git::refs::Tag>,
    pub commits: std::sync::Arc<Vec<Commit>>,
    pub graph: std::sync::Arc<Vec<GraphRow>>,
    /// Per graph row, which of its edges are links from unpushed commits (drawn marked).
    pub unpushed_edges: std::sync::Arc<Vec<Vec<bool>>>,
    pub stashes: Vec<StashEntry>,
    pub remotes: Vec<Remote>,
    pub reflog: Vec<ReflogEntry>,
    /// Working tree listing for the Files tab: tracked plus untracked (not ignored).
    /// Shared so the Files tab can tell a new listing from the one it last built from.
    pub files: std::sync::Arc<Vec<String>>,
    /// Ignored files and why, loaded only while the Files tab shows them ("Ignored" on);
    /// `None` otherwise.
    pub ignored: Option<std::sync::Arc<IgnoredFiles>>,
    pub worktrees: Vec<osier_git::ops::worktree::Worktree>,
    pub submodules: Vec<osier_git::submodule::Submodule>,
    /// Last five commit messages on HEAD, for the commit box history menu.
    pub recent_messages: Vec<String>,
    pub identity: Identity,
    pub in_progress: Option<InProgress>,
    /// Conflicted files whose working-tree copy still has conflict markers: "Mark resolved"
    /// is disabled for them.
    pub conflict_markers: std::collections::HashSet<String>,
}

/// Ignored files for the Files tab, with the rule that ignores each one.
#[derive(Debug, Clone, Default)]
pub struct IgnoredFiles {
    pub files: Vec<String>,
    /// Missing for a file when git could not say (the list still shows it).
    pub rules: std::collections::HashMap<String, osier_git::files::IgnoreRule>,
}

/// Which area of the Changes tab a file lives in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Area {
    Conflicted,
    Staged,
    Unstaged,
    Untracked,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Selection {
    #[default]
    None,
    File {
        path: String,
        area: Area,
    },
    Commit {
        hash: String,
    },
    /// Everything `to` changed relative to `from` (merge-base compare when `since_merge_base`).
    Compare {
        from: String,
        to: String,
        since_merge_base: bool,
    },
    /// A file from the Files tab, as it is in the working tree (`rev` = `None`) or at a
    /// revision.
    WorkFile {
        path: String,
        rev: Option<String>,
    },
    /// A stored patch from the Patches tab, read from its file.
    Patch {
        id: String,
        name: String,
        file: PathBuf,
    },
}

#[derive(Debug)]
pub struct Project {
    pub path: PathBuf,
    pub name: String,
    pub repo: Repository,
    pub snapshot: Option<Snapshot>,
    pub loading: bool,
    pub error: Option<String>,
    pub revision: u64,
    pub selection: Selection,
    pub selected_commit: Option<usize>,
    /// Second commit of a two-commit compare picked in the Log (shift/ctrl-click).
    pub compare_commit: Option<usize>,
    /// Commits picked in the Log, by hash: one on click, a range on shift-click, toggled one
    /// at a time on ctrl-click. Hashes, not rows, so new commits arriving do not move it.
    pub picked: Vec<String>,
    /// The tab the current selection (and the right pane) belongs to.
    pub pane_tab: Option<crate::app::Tab>,
    /// What the right pane showed on each other tab, restored when that tab comes back.
    pub panes: std::collections::HashMap<crate::app::Tab, TabPane>,
    /// Bumped whenever a refresh sees the refs move (branches or HEAD); part of
    /// `content_key` for selections that name refs.
    pub content_rev: u64,
    /// The open editor and blame while another project is active.
    pub parked_editor: Option<crate::edit_pane::FileEditor>,
    pub parked_blame: Option<crate::blame_view::BlameView>,
    pub log_page_size: usize,
    /// When set, the Log shows this file's history (`log --follow`).
    pub log_path: Option<String>,
    pub task: Task<()>,
    watch: Option<crate::repo_watch::RepoWatch>,
    dirty_since: Option<Instant>,
    last_refresh: Instant,
}

/// The right pane of one tab, kept while another tab is showing.
#[derive(Debug, Default)]
pub struct TabPane {
    pub selection: Selection,
    /// Log rows by hash (rows move when commits arrive while the tab is away).
    pub selected_hash: Option<String>,
    pub compare_hash: Option<String>,
    pub picked: Vec<String>,
    /// An open editor keeps its text: switching tabs never loses edits.
    pub editor: Option<crate::edit_pane::FileEditor>,
    pub blame: Option<crate::blame_view::BlameView>,
    /// The tab's diff view as it was left (scroll, expanded sections, selected lines), kept
    /// for the active project only.
    pub diff: Option<DiffCache>,
}

/// A tab's loaded diff view, reused when the tab comes back and its content cannot have
/// changed (see `Osier::sync_pane`).
pub struct DiffCache {
    pub state: gpui_kit::Entity<gpui_kit::component::diff::DiffState>,
    pub subscription: gpui_kit::Subscription,
    pub model: std::rc::Rc<Vec<osier_git::diff::DiffFile>>,
    pub files: usize,
    pub error: Option<String>,
    pub too_large: Option<u64>,
    pub viewer: Option<crate::viewer::Viewer>,
    /// `Project::content_key` of what it shows, when it was loaded.
    pub key: String,
}

impl std::fmt::Debug for DiffCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiffCache")
            .field("files", &self.files)
            .field("key", &self.key)
            .finish()
    }
}

/// The slow part of opening a project (finding the repository, starting its file watcher),
/// which is safe to build off the UI thread. [`Project::from_prepared`] finishes it.
pub struct Prepared {
    repo: Repository,
    watch: Option<crate::repo_watch::RepoWatch>,
}

impl Project {
    /// Find the repository at `path` and start watching it. Runs git; call it off the UI
    /// thread when opening many projects.
    pub fn prepare(
        runner: GitRunner,
        watch_runner: GitRunner,
        path: &Path,
    ) -> Result<Prepared, String> {
        Ok(Self::prepare_repo(
            Self::discover(runner, path)?,
            watch_runner,
        ))
    }

    /// The repository containing `path` (a file or folder anywhere inside it). Runs git.
    pub fn discover(runner: GitRunner, path: &Path) -> Result<Repository, String> {
        Repository::discover(runner, path).map_err(|e| e.to_string())
    }

    /// Start watching a repository found by [`Project::discover`]. `watch_runner` drives the
    /// file watcher's own git calls; it should log only failures.
    pub fn prepare_repo(repo: Repository, watch_runner: GitRunner) -> Prepared {
        let watch = crate::repo_watch::RepoWatch::start(repo.with_runner(watch_runner));
        Prepared { repo, watch }
    }

    pub fn from_prepared(prepared: Prepared, log_page_size: usize) -> Self {
        let Prepared { repo, watch } = prepared;
        let path = repo.workdir().to_path_buf();
        let name = repo.name();
        Self {
            path,
            name,
            repo,
            snapshot: None,
            loading: false,
            error: None,
            revision: 0,
            selection: Selection::None,
            selected_commit: None,
            compare_commit: None,
            picked: Vec::new(),
            pane_tab: None,
            panes: Default::default(),
            content_rev: 0,
            parked_editor: None,
            parked_blame: None,
            log_page_size,
            log_path: None,
            task: Task::ready(()),
            watch,
            dirty_since: None,
            last_refresh: Instant::now(),
        }
    }

    /// Drain watcher events. Returns true when a refresh is due (debounced).
    pub fn poll_watcher(&mut self) -> bool {
        let mut saw = self.watch.as_mut().is_some_and(|w| w.poll());
        // No watcher, or the watch limit was hit: refresh on a timer instead.
        if self.watch.as_ref().is_none_or(|w| w.degraded())
            && self.last_refresh.elapsed() >= Duration::from_secs(10)
        {
            saw = true;
        }
        if saw {
            self.dirty_since.get_or_insert_with(Instant::now);
        }
        match self.dirty_since {
            Some(t) if t.elapsed() >= Duration::from_millis(300) && !self.loading => {
                self.dirty_since = None;
                true
            }
            _ => false,
        }
    }

    /// Watch the whole tree, ignored files included (Files tab with "Show ignored" on).
    /// Returns true when this just switched it on: changes to ignored files went unseen
    /// until now, so the caller should refresh.
    pub fn set_watch_all(&mut self, all: bool) -> bool {
        self.watch
            .as_mut()
            .is_some_and(|w| w.set_watch_all(all) && all)
    }

    pub fn mark_refreshed(&mut self) {
        self.last_refresh = Instant::now();
    }

    pub fn commit_at(&self, ix: usize) -> Option<&Commit> {
        self.snapshot.as_ref()?.commits.get(ix)
    }

    /// The right pane a tab shows before anything is picked on it: the first change on
    /// Changes, the latest commit on Log, nothing elsewhere.
    pub fn default_pane(tab: crate::app::Tab, snapshot: Option<&Snapshot>) -> TabPane {
        use crate::app::Tab;
        let mut pane = TabPane::default();
        match (tab, snapshot) {
            (Tab::Changes, Some(s)) => {
                if let Some((path, area)) = crate::app::first_change(&s.status) {
                    pane.selection = Selection::File { path, area };
                }
            }
            (Tab::Log, Some(s)) => {
                if let Some(c) = s.commits.first() {
                    pane.selection = Selection::Commit {
                        hash: c.hash.clone(),
                    };
                    pane.selected_hash = Some(c.hash.clone());
                    pane.picked = vec![c.hash.clone()];
                }
            }
            _ => {}
        }
        pane
    }

    /// What the right pane's content for `selection` depends on, cheap to compute: the
    /// selection itself, plus the file's size and modified time and the index's for
    /// working-tree files, plus the refs for compares by name. A loaded diff is stale when
    /// this changes; fixed commits and patches never go stale.
    pub fn content_key(&self, selection: &Selection) -> String {
        fn stamp(path: &Path) -> Option<(u64, u128)> {
            let meta = std::fs::metadata(path).ok()?;
            let modified = meta
                .modified()
                .ok()?
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?;
            Some((meta.len(), modified.as_nanos()))
        }
        let work = |path: &str| stamp(&self.repo.workdir().join(path));
        let index = stamp(&self.repo.git_dir().join("index"));
        match selection {
            Selection::File { path, .. } => format!(
                "{selection:?}|{}|{:?}|{index:?}",
                self.content_rev,
                work(path)
            ),
            Selection::WorkFile { path, rev: None } => format!("{selection:?}|{:?}", work(path)),
            Selection::Compare { .. } => format!("{selection:?}|{}", self.content_rev),
            _ => format!("{selection:?}"),
        }
    }

    /// Where `selection` lands once `new` replaces the current snapshot, so the right pane
    /// never names something the lists no longer show. Unchanged while what it points at is
    /// still there. A Changes file that left its area (staged, unstaged, discarded,
    /// committed) moves on to the next file in that area, or the one above when it was the
    /// last, and follows the file into its new area when the area is now empty;
    /// `keep_file` (an editor is open on it) always follows the file. A dropped stash moves
    /// on to its neighbour the same way. A working-tree file that was deleted, or a Changes
    /// file with nowhere to go, gives `Selection::None`, which the tab's default fills.
    pub fn reconcile_selection(
        &self,
        selection: &Selection,
        new: &Snapshot,
        keep_file: bool,
    ) -> Selection {
        let old = self.snapshot.as_ref();
        match selection {
            Selection::File { path, area } => {
                let items = ordered_changes(&new.status);
                if items.iter().any(|(p, a)| p == path && a == area) {
                    return selection.clone();
                }
                let follow = || items.iter().find(|(p, _)| p == path).cloned();
                let next = if keep_file {
                    follow()
                } else {
                    old.and_then(|old| {
                        let was: Vec<String> = ordered_changes(&old.status)
                            .into_iter()
                            .filter(|(_, a)| a == area)
                            .map(|(p, _)| p)
                            .collect();
                        neighbour(&was, path, |p| {
                            items.iter().any(|(q, a)| q == p && a == area)
                        })
                        .map(|p| (p.clone(), *area))
                    })
                    .or_else(follow)
                };
                next.map_or(Selection::None, |(path, area)| Selection::File {
                    path,
                    area,
                })
            }
            Selection::Compare { to, .. }
                if old.is_some_and(|o| o.stashes.iter().any(|s| &s.oid == to))
                    && !new.stashes.iter().any(|s| &s.oid == to) =>
            {
                let was: Vec<String> = old
                    .map(|o| o.stashes.iter().map(|s| s.oid.clone()).collect())
                    .unwrap_or_default();
                neighbour(&was, to, |oid| new.stashes.iter().any(|s| &s.oid == oid)).map_or(
                    Selection::None,
                    |oid| Selection::Compare {
                        from: format!("{oid}^1"),
                        to: oid.clone(),
                        since_merge_base: false,
                    },
                )
            }
            Selection::WorkFile { path, rev: None } if !self.repo.workdir().join(path).exists() => {
                Selection::None
            }
            _ => selection.clone(),
        }
    }

    /// Row of a commit in the loaded log page.
    pub fn row_of(&self, hash: &str) -> Option<usize> {
        self.snapshot
            .as_ref()?
            .commits
            .iter()
            .position(|c| c.hash == hash)
    }

    /// Log rows of the picked commits, for highlighting. Falls back to the single selected
    /// commit (and a compare partner) when nothing is picked.
    pub fn picked_rows(&self) -> std::collections::HashSet<usize> {
        let Some(snapshot) = &self.snapshot else {
            return Default::default();
        };
        if self.picked.is_empty() {
            return self
                .selected_commit
                .into_iter()
                .chain(self.compare_commit)
                .collect();
        }
        snapshot
            .commits
            .iter()
            .enumerate()
            .filter(|(_, c)| self.picked.contains(&c.hash))
            .map(|(ix, _)| ix)
            .collect()
    }
}

/// Hashes of the commits a shift-click from row `a` to row `b` picks, in log order: those on
/// the line between the two ends (descendants of the older end that the newer one reaches),
/// so commits of other branches the date order puts in between are left out. When neither
/// end leads to the other in the loaded page, every row between them.
pub fn range_between(commits: &[Commit], a: usize, b: usize) -> Vec<String> {
    use std::collections::HashSet;
    let (newer, older) = (a.min(b), a.max(b));
    let Some(rows) = commits.get(newer..=older) else {
        return Vec::new();
    };
    // Children come before parents, so one pass down finds what the newer end reaches and
    // one pass up finds what leads to the older end.
    let mut reached: HashSet<&str> = HashSet::from([rows[0].hash.as_str()]);
    for c in rows {
        if reached.contains(c.hash.as_str()) {
            reached.extend(c.parents.iter().map(String::as_str));
        }
    }
    let oldest = &rows[rows.len() - 1].hash;
    if !reached.contains(oldest.as_str()) {
        return rows.iter().map(|c| c.hash.clone()).collect();
    }
    let mut leads: HashSet<&str> = HashSet::from([oldest.as_str()]);
    for c in rows.iter().rev() {
        if c.parents.iter().any(|p| leads.contains(p.as_str())) {
            leads.insert(c.hash.as_str());
        }
    }
    rows.iter()
        .filter(|c| reached.contains(c.hash.as_str()) && leads.contains(c.hash.as_str()))
        .map(|c| c.hash.clone())
        .collect()
}

/// What takes `gone`'s place in the list `was` once it has left: the nearest item after it
/// that `still` keeps, else the nearest one before it.
pub fn neighbour<'a, T: PartialEq>(
    was: &'a [T],
    gone: &T,
    still: impl Fn(&T) -> bool,
) -> Option<&'a T> {
    let ix = was.iter().position(|t| t == gone)?;
    was[ix + 1..]
        .iter()
        .find(|t| still(t))
        .or_else(|| was[..ix].iter().rev().find(|t| still(t)))
}

/// Files in the order the Changes tab shows them: conflicted, staged, unstaged, untracked.
pub fn ordered_changes(status: &RepoStatus) -> Vec<(String, Area)> {
    status
        .conflicted()
        .map(|e| (e.path.clone(), Area::Conflicted))
        .chain(status.staged().map(|e| (e.path.clone(), Area::Staged)))
        .chain(status.unstaged().map(|e| (e.path.clone(), Area::Unstaged)))
        .chain(
            status
                .untracked()
                .map(|e| (e.path.clone(), Area::Untracked)),
        )
        .collect()
}

/// Blocking read of everything the UI shows. Runs on the background executor.
pub fn load_snapshot(
    repo: &Repository,
    log_page_size: usize,
    log_path: Option<&str>,
    with_ignored: bool,
) -> Result<Snapshot, String> {
    let status = osier_git::status::status(repo, StatusOptions::default())
        .map_err(|e| format!("status: {e}"))?;
    let branches = osier_git::refs::branches(repo).map_err(|e| format!("branches: {e}"))?;
    let tags = osier_git::refs::tags(repo).map_err(|e| format!("tags: {e}"))?;
    let commits = osier_git::log::log(
        repo,
        &LogOptions {
            all: true,
            order: osier_git::log::LogOrder::Date,
            limit: Some(log_page_size),
            path: log_path.map(str::to_string),
            follow: log_path.is_some(),
            ..Default::default()
        },
    )
    .map_err(|e| format!("log: {e}"))?;
    let graph = osier_git::graph::layout(&commits);
    let stashes = osier_git::stash::list(repo).map_err(|e| format!("stash: {e}"))?;
    let remotes = osier_git::refs::remotes(repo).map_err(|e| format!("remotes: {e}"))?;
    let reflog = osier_git::reflog::list(repo, 200).map_err(|e| format!("reflog: {e}"))?;
    let mut files = osier_git::files::tracked(repo).map_err(|e| format!("ls-files: {e}"))?;
    files.extend(osier_git::files::untracked(repo).map_err(|e| format!("ls-files: {e}"))?);
    files.sort();
    files.dedup();
    // An empty list rather than an error: the Files tab asks again for a missing one.
    let ignored = with_ignored.then(|| {
        let files = osier_git::files::ignored(repo).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "ignored files unavailable");
            Vec::new()
        });
        let rules = osier_git::files::ignore_rules(repo, &files)
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, "ignore rules unavailable");
                Vec::new()
            })
            .into_iter()
            .collect();
        std::sync::Arc::new(IgnoredFiles { files, rules })
    });
    let worktrees = osier_git::ops::worktree::list(repo).map_err(|e| format!("worktree: {e}"))?;
    let identity = osier_git::config::identity(repo).map_err(|e| format!("config: {e}"))?;
    let recent_messages = osier_git::log::recent_messages(repo, 5).unwrap_or_default();
    let unpushed = osier_git::log::unpushed(repo, 5000).unwrap_or_default();
    let unpushed_edges =
        osier_git::graph::marked_edges(&graph, |ix| unpushed.contains(&commits[ix].hash));
    // A broken .gitmodules must not take the whole snapshot down.
    let submodules = osier_git::submodule::list(repo).unwrap_or_else(|e| {
        tracing::debug!(error = %e, "submodule listing unavailable");
        Vec::new()
    });
    let conflict_markers = status
        .conflicted()
        .filter(|e| osier_git::ops::integrate::file_has_conflict_markers(repo, &e.path))
        .map(|e| e.path.clone())
        .collect();
    Ok(Snapshot {
        status: std::sync::Arc::new(status),
        branches,
        tags,
        commits: std::sync::Arc::new(commits),
        graph: std::sync::Arc::new(graph),
        unpushed_edges: std::sync::Arc::new(unpushed_edges),
        stashes,
        remotes,
        reflog,
        files: std::sync::Arc::new(files),
        ignored,
        worktrees,
        submodules,
        recent_messages,
        unpushed: std::sync::Arc::new(unpushed),
        identity,
        in_progress: repo.in_progress(),
        conflict_markers,
    })
}

#[cfg(test)]
mod tests {
    use osier_git::log::Commit;

    use super::range_between;

    fn c(hash: &str, parents: &[&str]) -> Commit {
        Commit {
            hash: hash.into(),
            parents: parents.iter().map(|p| p.to_string()).collect(),
            author_name: String::new(),
            author_email: String::new(),
            authored_at: 0,
            committer_name: String::new(),
            committer_email: String::new(),
            committed_at: 0,
            refs: vec![],
            subject: String::new(),
            body: String::new(),
        }
    }

    #[test]
    fn range_follows_the_line_between_its_ends() {
        // Date order: main m2, m1 and side s2, s1 interleaved above base.
        let log = [
            c("m2", &["m1"]),
            c("s2", &["s1"]),
            c("m1", &["base"]),
            c("s1", &["base"]),
            c("base", &[]),
        ];
        assert_eq!(range_between(&log, 0, 2), ["m2", "m1"]);
        assert_eq!(range_between(&log, 4, 0), ["m2", "m1", "base"]);
        assert_eq!(range_between(&log, 1, 4), ["s2", "s1", "base"]);
    }

    #[test]
    fn range_keeps_both_sides_of_a_merge() {
        let log = [
            c("m", &["a", "f"]),
            c("f", &["b"]),
            c("a", &["b"]),
            c("b", &[]),
        ];
        assert_eq!(range_between(&log, 0, 3), ["m", "f", "a", "b"]);
    }

    #[test]
    fn unrelated_ends_pick_every_row_between() {
        let log = [
            c("m2", &["m1"]),
            c("s2", &["s1"]),
            c("m1", &[]),
            c("s1", &[]),
        ];
        assert_eq!(range_between(&log, 1, 2), ["s2", "m1"]);
        assert!(range_between(&log, 0, 9).is_empty());
    }
}
