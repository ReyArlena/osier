//! One open repository: loaded data, selection and background refresh.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use gpui_kit::Task;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use notify::{RecursiveMode, Watcher};
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
    pub branches: Vec<Branch>,
    pub commits: Vec<Commit>,
    pub graph: Vec<GraphRow>,
    pub stashes: Vec<StashEntry>,
    pub remotes: Vec<Remote>,
    pub reflog: Vec<ReflogEntry>,
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
    pub log_page_size: usize,
    pub task: Task<()>,
    watcher: Option<notify::RecommendedWatcher>,
    events: Option<mpsc::Receiver<()>>,
    dirty_since: Option<Instant>,
    last_refresh: Instant,
}

impl Project {
    pub fn open(runner: GitRunner, path: &Path, log_page_size: usize) -> Result<Self, String> {
        let repo = Repository::discover(runner, path).map_err(|e| e.to_string())?;
        let path = repo.workdir().to_path_buf();
        let name = repo.name();
        let mut project = Self {
            path,
            name,
            repo,
            snapshot: None,
            loading: false,
            error: None,
            revision: 0,
            selection: Selection::None,
            selected_commit: None,
            log_page_size,
            task: Task::ready(()),
            watcher: None,
            events: None,
            dirty_since: None,
            last_refresh: Instant::now(),
        };
        project.start_watcher();
        Ok(project)
    }

    fn start_watcher(&mut self) {
        let (tx, rx) = mpsc::channel::<()>();
        let git_dir = self.repo.git_dir().to_path_buf();
        let ignore = build_gitignore(&self.path, &git_dir);
        let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(event) = res {
                // Reads (our own git commands included) are not changes. Only content and
                // structure changes matter.
                let relevant = matches!(
                    event.kind,
                    notify::EventKind::Create(_)
                        | notify::EventKind::Modify(_)
                        | notify::EventKind::Remove(_)
                );
                // Lock files and editor temp files are noise. `.git/index` is kept: it is how
                // `git add` from a terminal shows up.
                // Ignored paths (build output, logs) never change git's view of the tree.
                // Paths inside `.git` are always relevant (index, HEAD, refs).
                let noise = event.paths.iter().all(|p| {
                    let name = p
                        .file_name()
                        .map(|n| n.to_string_lossy())
                        .unwrap_or_default();
                    if name.ends_with(".lock") || name.ends_with('~') || name.ends_with(".swp") {
                        return true;
                    }
                    if p.starts_with(&git_dir) {
                        return false;
                    }
                    ignore
                        .as_ref()
                        .is_some_and(|g| g.matched_path_or_any_parents(p, p.is_dir()).is_ignore())
                });
                if relevant && !noise {
                    tracing::trace!(kind = ?event.kind, paths = ?event.paths, "fs event");
                    let _ = tx.send(());
                }
            }
        });
        match watcher {
            Ok(mut w) => {
                if let Err(e) = w.watch(&self.path, RecursiveMode::Recursive) {
                    tracing::warn!(error = %e, path = %self.path.display(), "watch failed");
                }
                let git_dir = self.repo.git_dir().to_path_buf();
                if git_dir.starts_with(&self.path) {
                    // Already covered by the recursive watch.
                } else if let Err(e) = w.watch(&git_dir, RecursiveMode::Recursive) {
                    tracing::warn!(error = %e, "watch of git dir failed");
                }
                self.watcher = Some(w);
                self.events = Some(rx);
            }
            Err(e) => tracing::warn!(error = %e, "file watcher unavailable"),
        }
    }

    /// Drain watcher events. Returns true when a refresh is due (debounced).
    pub fn poll_watcher(&mut self) -> bool {
        let mut saw = false;
        if let Some(rx) = &self.events {
            while rx.try_recv().is_ok() {
                saw = true;
            }
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

    pub fn mark_refreshed(&mut self) {
        self.last_refresh = Instant::now();
    }

    pub fn commit_at(&self, ix: usize) -> Option<&Commit> {
        self.snapshot.as_ref()?.commits.get(ix)
    }
}

/// Matcher for the root `.gitignore` and `.git/info/exclude`. Nested ignore files are not
/// read; a change under one of those just costs an extra refresh.
fn build_gitignore(root: &Path, git_dir: &Path) -> Option<Gitignore> {
    let mut b = GitignoreBuilder::new(root);
    for file in [root.join(".gitignore"), git_dir.join("info/exclude")] {
        if file.is_file() {
            // A malformed line is reported but does not disable the rest.
            if let Some(e) = b.add(&file) {
                tracing::debug!(error = %e, file = %file.display(), "ignore file");
            }
        }
    }
    match b.build() {
        Ok(g) if !g.is_empty() => Some(g),
        Ok(_) => None,
        Err(e) => {
            tracing::debug!(error = %e, "gitignore matcher");
            None
        }
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
pub fn load_snapshot(repo: &Repository, log_page_size: usize) -> Result<Snapshot, String> {
    let status = osier_git::status::status(repo, StatusOptions::default())
        .map_err(|e| format!("status: {e}"))?;
    let branches = osier_git::refs::branches(repo).map_err(|e| format!("branches: {e}"))?;
    let commits = osier_git::log::log(
        repo,
        &LogOptions {
            all: true,
            topo_order: true,
            limit: Some(log_page_size),
            ..Default::default()
        },
    )
    .map_err(|e| format!("log: {e}"))?;
    let graph = osier_git::graph::layout(&commits);
    let stashes = osier_git::stash::list(repo).map_err(|e| format!("stash: {e}"))?;
    let remotes = osier_git::refs::remotes(repo).map_err(|e| format!("remotes: {e}"))?;
    let reflog = osier_git::reflog::list(repo, 200).map_err(|e| format!("reflog: {e}"))?;
    let identity = osier_git::config::identity(repo).map_err(|e| format!("config: {e}"))?;
    Ok(Snapshot {
        status,
        branches,
        commits,
        graph,
        stashes,
        remotes,
        reflog,
        identity,
        in_progress: repo.in_progress(),
    })
}
