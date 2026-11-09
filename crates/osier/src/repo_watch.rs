//! File watching for one open repository.
//!
//! The working tree is watched one directory at a time (non-recursive inotify watches) so that
//! ignored build output (`target/`, `node_modules/`) costs nothing. Which directories count as
//! ignored comes from git itself (`files::ignored_layout`): it applies nested `.gitignore`s,
//! `info/exclude`, the global excludes file and negations exactly as `git status` does, and it
//! never reports a directory as ignored while it holds a tracked file.
//!
//! Inside the git directory only what changes git's view is watched: the top level (HEAD,
//! index, packed-refs, FETCH_HEAD, merge and rebase state), `refs/` recursively, `logs/` (its
//! `HEAD` moves on every commit) and `info/` (exclude). `objects/` is never watched. Submodule
//! git dirs under `modules/` get the same treatment.
//!
//! The set is recomputed in the background ("resync") whenever something that can change it
//! happens: an ignore file, the index, `.gitmodules`, or a directory appearing, disappearing
//! or being renamed. Resyncs are debounced and never overlap.
//!
//! While the Files tab lists ignored files, the whole tree is watched recursively instead, so
//! that list updates live too. If the kernel's watch limit is reached, `degraded` turns on and
//! the caller falls back to timed refreshes.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use notify::{EventKind, RecursiveMode, Watcher};
use osier_git::Repository;

const RESYNC_DEBOUNCE: Duration = Duration::from_millis(500);

/// What one watcher event means for the project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    /// Changes nothing git reports (ignored files, lock and swap files, objects).
    Noise,
    /// Status may have changed: refresh.
    Refresh,
    /// The set of watched directories may have changed: refresh and resync.
    Resync,
}

pub(crate) struct RepoWatch {
    shared: Arc<Shared>,
    rx: mpsc::Receiver<notify::Event>,
    resync_due: Option<Instant>,
    #[cfg(test)]
    tx: mpsc::Sender<notify::Event>,
}

impl std::fmt::Debug for RepoWatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RepoWatch")
            .field("root", &self.shared.root)
            .finish_non_exhaustive()
    }
}

struct Shared {
    root: PathBuf,
    repo: Repository,
    state: Mutex<State>,
    /// A resync thread is running.
    running: AtomicBool,
    /// Another resync was requested while one ran.
    again: AtomicBool,
    /// A resync added watches: changes made before they existed may have been missed.
    added: AtomicBool,
    /// Watch the whole tree (Files tab showing ignored files).
    all: AtomicBool,
    /// The watch limit was hit at least once.
    degraded: AtomicBool,
}

struct State {
    watcher: notify::RecommendedWatcher,
    watched: BTreeMap<PathBuf, RecursiveMode>,
    /// Mode the current watches were made for.
    all: bool,
    ignored_files: HashSet<PathBuf>,
    ignored_dirs: Vec<PathBuf>,
    git_dirs: Vec<PathBuf>,
    excludes_file: Option<PathBuf>,
}

impl RepoWatch {
    /// Start watching. `repo` should log only failures: resyncs run `git ls-files` often.
    pub(crate) fn start(repo: Repository) -> Option<Self> {
        let (tx, rx) = mpsc::channel();
        #[cfg(test)]
        let test_tx = tx.clone();
        let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            match res {
                // Reads (our own git commands included) are not changes.
                Ok(event)
                    if event.need_rescan()
                        || matches!(
                            event.kind,
                            EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
                        ) =>
                {
                    let _ = tx.send(event);
                }
                Ok(_) => {}
                // An error from the backend may mean lost events: treat it as a rescan.
                Err(_) => {
                    let mut event = notify::Event::new(EventKind::Other);
                    event = event.set_flag(notify::event::Flag::Rescan);
                    let _ = tx.send(event);
                }
            }
        });
        let watcher = match watcher {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!(error = %e, "file watcher unavailable");
                return None;
            }
        };
        let git_dirs = git_dirs(&repo);
        let shared = Arc::new(Shared {
            root: repo.workdir().to_path_buf(),
            repo,
            state: Mutex::new(State {
                watcher,
                watched: BTreeMap::new(),
                all: false,
                ignored_files: HashSet::new(),
                ignored_dirs: Vec::new(),
                git_dirs,
                excludes_file: None,
            }),
            running: AtomicBool::new(false),
            again: AtomicBool::new(false),
            added: AtomicBool::new(false),
            all: AtomicBool::new(false),
            degraded: AtomicBool::new(false),
        });
        let watch = Self {
            shared,
            rx,
            resync_due: None,
            #[cfg(test)]
            tx: test_tx,
        };
        watch.spawn_resync();
        Some(watch)
    }

    /// Watch the whole tree recursively (true) or only what git cares about (false). Returns
    /// whether this changed the mode.
    pub(crate) fn set_watch_all(&mut self, all: bool) -> bool {
        let changed = self.shared.all.swap(all, Ordering::SeqCst) != all;
        if changed {
            self.spawn_resync();
        }
        changed
    }

    /// The kernel's watch limit was reached: some directories are not watched.
    pub(crate) fn degraded(&self) -> bool {
        self.shared.degraded.load(Ordering::SeqCst)
    }

    /// Drain events. Returns true when the project should refresh.
    pub(crate) fn poll(&mut self) -> bool {
        let mut refresh = false;
        let mut resync = false;
        let events: Vec<notify::Event> = self.rx.try_iter().collect();
        if !events.is_empty() {
            let all = self.shared.all.load(Ordering::SeqCst);
            let Ok(mut state) = self.shared.state.lock() else {
                return true;
            };
            for event in &events {
                // The kernel's event queue overflowed (or the backend failed): events were
                // lost, so look at everything again.
                if event.need_rescan() {
                    refresh = true;
                    resync = true;
                    continue;
                }
                for path in &event.paths {
                    match classify(&self.shared.root, &state, all, event.kind, path) {
                        Class::Noise => {}
                        Class::Refresh => refresh = true,
                        Class::Resync => {
                            refresh = true;
                            resync = true;
                        }
                    }
                    // notify drops the watch of a deleted directory itself; forget it too, so
                    // the next resync does not try to remove it again.
                    if matches!(event.kind, EventKind::Remove(_)) {
                        state.watched.remove(path);
                    }
                }
            }
        }
        if resync {
            self.resync_due.get_or_insert_with(Instant::now);
        }
        if self
            .resync_due
            .is_some_and(|t| t.elapsed() >= RESYNC_DEBOUNCE)
        {
            self.resync_due = None;
            self.spawn_resync();
        }
        if self.shared.added.swap(false, Ordering::SeqCst) {
            refresh = true;
        }
        refresh
    }

    fn spawn_resync(&self) {
        if self.shared.running.swap(true, Ordering::SeqCst) {
            self.shared.again.store(true, Ordering::SeqCst);
            return;
        }
        let shared = self.shared.clone();
        let spawned = std::thread::Builder::new()
            .name("osier-watch".into())
            .spawn(move || {
                loop {
                    shared.again.store(false, Ordering::SeqCst);
                    resync(&shared);
                    if !shared.again.load(Ordering::SeqCst) {
                        break;
                    }
                }
                shared.running.store(false, Ordering::SeqCst);
            });
        if spawned.is_err() {
            self.shared.running.store(false, Ordering::SeqCst);
        }
    }

    /// Test hook: wait until no resync is running.
    #[cfg(test)]
    pub(crate) fn settle(&mut self) {
        let start = Instant::now();
        while self.shared.running.load(Ordering::SeqCst)
            && start.elapsed() < Duration::from_secs(10)
        {
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Test hook: feed an event as if the backend had sent it.
    #[cfg(test)]
    pub(crate) fn inject(&self, event: notify::Event) {
        let _ = self.tx.send(event);
    }

    /// Test hook: the directories currently watched and how.
    #[cfg(test)]
    pub(crate) fn watched(&self) -> BTreeMap<PathBuf, RecursiveMode> {
        self.shared
            .state
            .lock()
            .map(|s| s.watched.clone())
            .unwrap_or_default()
    }
}

/// The repository's git dir, its common dir (linked worktrees) and submodule git dirs.
fn git_dirs(repo: &Repository) -> Vec<PathBuf> {
    let mut dirs = vec![repo.git_dir().to_path_buf()];
    let common = repo.common_dir().to_path_buf();
    if !dirs.contains(&common) {
        dirs.push(common.clone());
    }
    // Submodules keep their git dirs under `modules/` (nested ones under theirs).
    let mut stack = vec![common.join("modules")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !entry.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            if path.join("HEAD").is_file() {
                dirs.push(path.clone());
                stack.push(path.join("modules"));
            } else {
                // Submodule names may contain slashes: `modules/a/b`.
                stack.push(path);
            }
        }
    }
    dirs
}

fn classify(root: &Path, state: &State, all: bool, kind: EventKind, path: &Path) -> Class {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    if name.ends_with(".lock") || name.ends_with('~') || name.ends_with(".swp") {
        return Class::Noise;
    }
    if let Some(git_dir) = state.git_dirs.iter().find(|d| path.starts_with(d)) {
        let rel = path.strip_prefix(git_dir).unwrap_or(path);
        if rel.starts_with("objects") {
            return Class::Noise;
        }
        // The index changes which directories hold tracked files; info/exclude and new
        // submodule git dirs change the watch set directly.
        // `config` can change `core.excludesFile` or add submodules.
        if rel == Path::new("index")
            || rel == Path::new("config")
            || rel.starts_with("info")
            || rel.starts_with("modules")
        {
            return Class::Resync;
        }
        return Class::Refresh;
    }
    if state.excludes_file.as_deref() == Some(path) {
        return Class::Resync;
    }
    if !path.starts_with(root) {
        // The excludes file's directory has other files in it.
        return Class::Noise;
    }
    if name == ".gitignore" || name == ".gitmodules" {
        return Class::Resync;
    }
    if all {
        return Class::Refresh;
    }
    if state.ignored_files.contains(path) || state.ignored_dirs.iter().any(|d| path.starts_with(d))
    {
        return Class::Noise;
    }
    // A directory appeared, went away or moved: the watch set changes. A new file also
    // resyncs, so that if it is ignored (a fresh log file) its later writes are noise instead
    // of a refresh each time.
    let structural = match kind {
        EventKind::Create(_) => true,
        EventKind::Remove(_) => state.watched.contains_key(path),
        EventKind::Modify(notify::event::ModifyKind::Name(_)) => {
            path.is_dir() || state.watched.contains_key(path)
        }
        _ => false,
    };
    if structural {
        Class::Resync
    } else {
        Class::Refresh
    }
}

/// Recompute the watch set and apply the difference.
fn resync(shared: &Shared) {
    let all = shared.all.load(Ordering::SeqCst);
    let root = &shared.root;
    let git_dirs = git_dirs(&shared.repo);
    let excludes_file = osier_git::files::excludes_file(&shared.repo).ok().flatten();

    let mut want: BTreeMap<PathBuf, RecursiveMode> = BTreeMap::new();
    let mut ignored_files = HashSet::new();
    let mut ignored_dirs = Vec::new();
    if all {
        want.insert(root.clone(), RecursiveMode::Recursive);
    } else {
        let layout = match osier_git::files::ignored_layout(&shared.repo) {
            Ok(l) => l,
            Err(e) => {
                // Without git's answer, watch everything rather than miss changes.
                tracing::debug!(error = %e, "ignored layout unavailable, watching everything");
                osier_git::files::IgnoredLayout::default()
            }
        };
        ignored_dirs = layout.dirs.iter().map(|d| root.join(d)).collect();
        ignored_files = layout.files.iter().map(|f| root.join(f)).collect();
        let skip: HashSet<&PathBuf> = ignored_dirs.iter().collect();
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            want.insert(dir.clone(), RecursiveMode::NonRecursive);
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                // Symlinks are not followed, like git.
                if !entry.file_type().is_ok_and(|t| t.is_dir()) {
                    continue;
                }
                let path = entry.path();
                if entry.file_name() == ".git" || skip.contains(&path) {
                    continue;
                }
                stack.push(path);
            }
        }
    }
    for dir in &git_dirs {
        // Under a recursive root watch these are already covered.
        if all && dir.starts_with(root) {
            continue;
        }
        want.insert(dir.clone(), RecursiveMode::NonRecursive);
        want.insert(dir.join("refs"), RecursiveMode::Recursive);
        want.insert(dir.join("logs"), RecursiveMode::NonRecursive);
        want.insert(dir.join("info"), RecursiveMode::NonRecursive);
        want.insert(dir.join("modules"), RecursiveMode::NonRecursive);
    }
    if let Some(parent) = excludes_file.as_ref().and_then(|f| f.parent())
        && !(all && parent.starts_with(root))
    {
        want.insert(parent.to_path_buf(), RecursiveMode::NonRecursive);
    }
    want.retain(|p, _| p.is_dir());

    let Ok(mut state) = shared.state.lock() else {
        return;
    };
    let state = &mut *state;
    if state.all != all {
        // Switching between recursive and per-directory: start from nothing so overlapping
        // watches never share a descriptor.
        for path in std::mem::take(&mut state.watched).into_keys() {
            let _ = state.watcher.unwatch(&path);
        }
        state.all = all;
    }
    let stale: Vec<PathBuf> = state
        .watched
        .iter()
        .filter(|(p, m)| want.get(*p) != Some(*m))
        .map(|(p, _)| p.clone())
        .collect();
    for path in stale {
        let _ = state.watcher.unwatch(&path);
        state.watched.remove(&path);
    }
    let mut added = false;
    for (path, mode) in want {
        if state.watched.contains_key(&path) {
            continue;
        }
        match state.watcher.watch(&path, mode) {
            Ok(()) => {
                state.watched.insert(path, mode);
                added = true;
            }
            Err(e) if matches!(e.kind, notify::ErrorKind::MaxFilesWatch) => {
                if !shared.degraded.swap(true, Ordering::SeqCst) {
                    tracing::warn!(
                        root = %root.display(),
                        "inotify watch limit reached; this repository falls back to timed refreshes (raise fs.inotify.max_user_watches)"
                    );
                }
                break;
            }
            // Gone between the walk and the watch: the next resync sorts it out.
            Err(e) => tracing::debug!(error = %e, path = %path.display(), "watch failed"),
        }
    }
    state.ignored_files = ignored_files;
    state.ignored_dirs = ignored_dirs;
    state.git_dirs = git_dirs;
    state.excludes_file = excludes_file;
    if added {
        shared.added.store(true, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::RepoWatch;
    use notify::RecursiveMode;
    use osier_git::testing::TempRepo;
    use std::time::{Duration, Instant};

    /// Poll until a refresh is requested or `timeout` passes.
    fn refreshes(w: &mut RepoWatch, timeout: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if w.poll() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    /// Let pending events, debounced resyncs and their follow-up refreshes run out.
    fn quiesce(w: &mut RepoWatch) {
        let mut last = Instant::now();
        let start = Instant::now();
        while last.elapsed() < Duration::from_millis(900)
            && start.elapsed() < Duration::from_secs(10)
        {
            w.settle();
            if w.poll() {
                last = Instant::now();
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        w.settle();
    }

    fn repo() -> TempRepo {
        let t = TempRepo::new();
        t.commit_file(".gitignore", "target/\nvendor/\n*.log\n", "ignore");
        t.commit_file("src/main.rs", "fn main() {}\n", "src");
        t.write("target/debug/build/x", "x\n");
        t.write("vendor/kept.txt", "v\n");
        t.write("vendor/sub/other.txt", "o\n");
        t.git(["add", "-f", "vendor/kept.txt"]);
        t.git(["commit", "-q", "-m", "vendored"]);
        t
    }

    #[test]
    fn watches_what_git_reports_and_skips_ignored_folders_and_objects() {
        let t = repo();
        let mut w = RepoWatch::start(t.repo()).expect("watcher");
        quiesce(&mut w);
        let root = t.path();
        let watched = w.watched();
        let mode = |rel: &str| watched.get(&root.join(rel)).copied();
        assert_eq!(watched.get(root), Some(&RecursiveMode::NonRecursive));
        assert_eq!(mode("src"), Some(RecursiveMode::NonRecursive));
        assert_eq!(
            mode("vendor"),
            Some(RecursiveMode::NonRecursive),
            "holds a tracked file"
        );
        assert_eq!(mode("vendor/sub"), None, "ignored and untracked");
        assert_eq!(mode("target"), None);
        assert_eq!(mode("target/debug"), None);
        assert_eq!(mode(".git"), Some(RecursiveMode::NonRecursive));
        assert_eq!(mode(".git/refs"), Some(RecursiveMode::Recursive));
        assert_eq!(mode(".git/logs"), Some(RecursiveMode::NonRecursive));
        assert_eq!(mode(".git/objects"), None);
        assert!(
            !watched
                .keys()
                .any(|p| p.starts_with(root.join(".git/objects")))
        );
    }

    #[test]
    fn tracked_files_in_ignored_folders_refresh_but_ignored_output_does_not() {
        let t = repo();
        let mut w = RepoWatch::start(t.repo()).expect("watcher");
        quiesce(&mut w);
        // A new ignored file costs one refresh, then its writes are noise.
        t.write("debug.log", "l\n");
        quiesce(&mut w);
        t.write("debug.log", "l\nmore\n");
        t.write("target/debug/build/y", "y\n");
        assert!(
            !refreshes(&mut w, Duration::from_millis(700)),
            "ignored output is noise"
        );
        quiesce(&mut w);
        t.write("vendor/kept.txt", "changed\n");
        assert!(
            refreshes(&mut w, Duration::from_secs(3)),
            "a tracked file in an ignored folder"
        );
    }

    #[test]
    fn new_folders_are_watched_and_gitignore_changes_resync() {
        let t = repo();
        let mut w = RepoWatch::start(t.repo()).expect("watcher");
        quiesce(&mut w);
        t.write("src/new/deeper/file.rs", "x\n");
        assert!(refreshes(&mut w, Duration::from_secs(3)));
        quiesce(&mut w);
        assert!(w.watched().contains_key(&t.path().join("src/new/deeper")));
        t.write("src/new/deeper/file.rs", "y\n");
        assert!(
            refreshes(&mut w, Duration::from_secs(3)),
            "edits inside the new folder"
        );

        // Un-ignoring target/ starts watching it.
        t.write(".gitignore", "vendor/\n*.log\n");
        assert!(refreshes(&mut w, Duration::from_secs(3)));
        quiesce(&mut w);
        assert!(
            w.watched()
                .contains_key(&t.path().join("target/debug/build"))
        );
    }

    #[test]
    fn watch_all_covers_ignored_files_and_switches_back() {
        let t = repo();
        let mut w = RepoWatch::start(t.repo()).expect("watcher");
        quiesce(&mut w);
        w.set_watch_all(true);
        quiesce(&mut w);
        assert_eq!(w.watched().get(t.path()), Some(&RecursiveMode::Recursive));
        t.write("target/debug/build/z", "z\n");
        assert!(
            refreshes(&mut w, Duration::from_secs(3)),
            "ignored files count while shown"
        );

        w.set_watch_all(false);
        quiesce(&mut w);
        assert_eq!(
            w.watched().get(t.path()),
            Some(&RecursiveMode::NonRecursive)
        );
        t.write("target/debug/build/z", "zz\n");
        assert!(!refreshes(&mut w, Duration::from_millis(700)));
    }

    #[test]
    fn linked_worktrees_follow_the_shared_refs() {
        let t = repo();
        let dir = tempfile::tempdir().expect("tempdir");
        let wt = dir.path().join("wt");
        t.git([
            "worktree",
            "add",
            "-q",
            "-b",
            "side",
            wt.to_str().expect("utf8"),
        ]);
        let wt_repo =
            osier_git::Repository::discover(t.repo().runner().clone(), &wt).expect("worktree");
        let mut w = RepoWatch::start(wt_repo).expect("watcher");
        quiesce(&mut w);
        let watched = w.watched();
        assert!(
            watched.contains_key(&t.path().join(".git/refs")),
            "common refs"
        );
        assert!(
            watched
                .keys()
                .any(|p| p.starts_with(t.path().join(".git/worktrees")))
        );
        // A new branch made from the main checkout shows up in the worktree's project.
        t.git(["branch", "from-main"]);
        assert!(refreshes(&mut w, Duration::from_secs(3)));
    }

    #[test]
    fn lost_events_rescan_everything() {
        let t = repo();
        let mut w = RepoWatch::start(t.repo()).expect("watcher");
        quiesce(&mut w);
        // A folder the watcher never heard about, as after a queue overflow.
        std::fs::create_dir_all(t.path().join("src/missed")).expect("mkdir");
        let event =
            notify::Event::new(notify::EventKind::Other).set_flag(notify::event::Flag::Rescan);
        w.inject(event);
        assert!(w.poll(), "a rescan refreshes at once");
        quiesce(&mut w);
        assert!(w.watched().contains_key(&t.path().join("src/missed")));
    }

    #[test]
    fn commits_from_outside_refresh() {
        let t = repo();
        let mut w = RepoWatch::start(t.repo()).expect("watcher");
        quiesce(&mut w);
        t.write("src/main.rs", "fn main() { }\n");
        t.git(["commit", "-q", "-am", "edit"]);
        assert!(refreshes(&mut w, Duration::from_secs(3)));
    }
}
