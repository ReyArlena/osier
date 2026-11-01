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
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub status: RepoStatus,
    /// Commits on local branches that no remote branch has yet (Log graph marking).
    pub unpushed: std::collections::HashSet<String>,
    pub branches: Vec<Branch>,
    pub tags: Vec<osier_git::refs::Tag>,
    pub commits: Vec<Commit>,
    pub graph: Vec<GraphRow>,
    pub stashes: Vec<StashEntry>,
    pub remotes: Vec<Remote>,
    pub reflog: Vec<ReflogEntry>,
    /// Working tree listing for the Files tab: tracked plus untracked (not ignored).
    pub files: Vec<String>,
    pub worktrees: Vec<osier_git::ops::worktree::Worktree>,
    pub submodules: Vec<osier_git::submodule::Submodule>,
    /// Last five commit messages on HEAD, for the commit box history menu.
    pub recent_messages: Vec<String>,
    pub identity: Identity,
    pub in_progress: Option<InProgress>,
}

/// Which area of the Changes tab a file lives in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Area {
    Conflicted,
    Staged,
    Unstaged,
    Untracked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
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
    pub log_page_size: usize,
    /// When set, the Log shows this file's history (`log --follow`).
    pub log_path: Option<String>,
    pub task: Task<()>,
    watch: Option<crate::repo_watch::RepoWatch>,
    dirty_since: Option<Instant>,
    last_refresh: Instant,
}

/// The slow part of opening a project (finding the repository, starting its file watcher),
/// which is safe to build off the UI thread. [`Project::from_prepared`] finishes it.
pub struct Prepared {
    repo: Repository,
    watch: Option<crate::repo_watch::RepoWatch>,
}

impl Project {
    /// `watch_runner` drives the file watcher's own git calls; it should log only failures.
    pub fn open(
        runner: GitRunner,
        watch_runner: GitRunner,
        path: &Path,
        log_page_size: usize,
    ) -> Result<Self, String> {
        Ok(Self::from_prepared(
            Self::prepare(runner, watch_runner, path)?,
            log_page_size,
        ))
    }

    /// Find the repository at `path` and start watching it. Runs git; call it off the UI
    /// thread when opening many projects.
    pub fn prepare(
        runner: GitRunner,
        watch_runner: GitRunner,
        path: &Path,
    ) -> Result<Prepared, String> {
        let repo = Repository::discover(runner, path).map_err(|e| e.to_string())?;
        let watch = crate::repo_watch::RepoWatch::start(repo.with_runner(watch_runner));
        Ok(Prepared { repo, watch })
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
    pub fn set_watch_all(&mut self, all: bool) {
        if let Some(w) = self.watch.as_mut() {
            w.set_watch_all(all);
        }
    }

    pub fn mark_refreshed(&mut self) {
        self.last_refresh = Instant::now();
    }

    pub fn commit_at(&self, ix: usize) -> Option<&Commit> {
        self.snapshot.as_ref()?.commits.get(ix)
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
) -> Result<Snapshot, String> {
    let status = osier_git::status::status(repo, StatusOptions::default())
        .map_err(|e| format!("status: {e}"))?;
    let branches = osier_git::refs::branches(repo).map_err(|e| format!("branches: {e}"))?;
    let tags = osier_git::refs::tags(repo).map_err(|e| format!("tags: {e}"))?;
    let commits = osier_git::log::log(
        repo,
        &LogOptions {
            all: true,
            topo_order: true,
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
    let worktrees = osier_git::ops::worktree::list(repo).map_err(|e| format!("worktree: {e}"))?;
    let identity = osier_git::config::identity(repo).map_err(|e| format!("config: {e}"))?;
    let recent_messages = osier_git::log::recent_messages(repo, 5).unwrap_or_default();
    let unpushed = osier_git::log::unpushed(repo, 5000).unwrap_or_default();
    // A broken .gitmodules must not take the whole snapshot down.
    let submodules = osier_git::submodule::list(repo).unwrap_or_else(|e| {
        tracing::debug!(error = %e, "submodule listing unavailable");
        Vec::new()
    });
    Ok(Snapshot {
        status,
        branches,
        tags,
        commits,
        graph,
        stashes,
        remotes,
        reflog,
        files,
        worktrees,
        submodules,
        recent_messages,
        unpushed,
        identity,
        in_progress: repo.in_progress(),
    })
}
