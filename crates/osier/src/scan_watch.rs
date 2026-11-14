//! Notice when a repository appears in a scan directory, or goes away, so the sidebar and the
//! project palette list it without a manual rescan.
//!
//! Only what a scan reads is watched, never recursively (`osier_config::scan::watch_set`):
//! the scanned folders, for folders created, removed or renamed in them, and the plain
//! folders at the last level, for a `.git` appearing (`git init`). Busy repositories under a
//! scan directory cost nothing, and file churn in a plain folder is ignored. The tick loop
//! polls `take_due` and rescans; the rescan builds the watch again (off the UI thread) for
//! the folders as they are now.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use notify::Watcher as _;

/// Quiet time after the last event before a rescan: cloning or `git init` makes a burst of
/// events, and one scan after it is enough.
const SETTLE: Duration = Duration::from_millis(800);

pub(crate) struct ScanWatch {
    /// When the latest relevant event arrived; `None` when nothing is waiting.
    last_event: Arc<Mutex<Option<Instant>>>,
    _watcher: notify::RecommendedWatcher,
}

impl ScanWatch {
    /// Watch what a scan of `dirs` to `depth` reads. Blocking (it reads those folders and
    /// adds a watch per folder): call it off the UI thread. `None` when there is nothing to
    /// watch or the watcher cannot start; the manual rescan still works then.
    pub(crate) fn new(dirs: &[PathBuf], depth: u8) -> Option<Self> {
        let set = osier_config::scan::watch_set(dirs, depth);
        if set.scanned.is_empty() {
            return None;
        }
        let scanned: HashSet<PathBuf> = set.scanned.iter().cloned().collect();
        let last_event = Arc::new(Mutex::new(None));
        let stamp = last_event.clone();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let relevant = match res {
                Ok(event) => is_relevant(&event, &scanned),
                // An error may mean lost events: scan to be sure.
                Err(_) => true,
            };
            if relevant && let Ok(mut at) = stamp.lock() {
                *at = Some(Instant::now());
            }
        })
        .map_err(|error| tracing::warn!(%error, "cannot create scan directory watcher"))
        .ok()?;
        for folder in set.scanned.iter().chain(&set.leaves) {
            if let Err(error) = watcher.watch(folder, notify::RecursiveMode::NonRecursive) {
                tracing::debug!(%error, folder = %folder.display(), "cannot watch folder");
            }
        }
        Some(Self {
            last_event,
            _watcher: watcher,
        })
    }

    /// True once, when events have arrived and gone quiet for `SETTLE`.
    pub(crate) fn take_due(&self) -> bool {
        let Ok(mut at) = self.last_event.lock() else {
            return false;
        };
        match *at {
            Some(when) if when.elapsed() >= SETTLE => {
                *at = None;
                true
            }
            _ => false,
        }
    }
}

/// What can change the list: a `.git` appearing or going (in a scanned or a leaf folder),
/// or a folder created, removed or renamed in a scanned folder. Files coming and going, and
/// anything else in a leaf folder, are ignored.
fn is_relevant(event: &notify::Event, scanned: &HashSet<PathBuf>) -> bool {
    use notify::EventKind;
    use notify::event::{CreateKind, ModifyKind, RemoveKind};
    if event.need_rescan() {
        return true;
    }
    let structural = matches!(
        event.kind,
        EventKind::Any
            | EventKind::Create(_)
            | EventKind::Remove(_)
            | EventKind::Modify(ModifyKind::Name(_) | ModifyKind::Any)
    );
    if !structural {
        return false;
    }
    event.paths.iter().any(|p| {
        if p.file_name().is_some_and(|n| n == ".git") {
            return true;
        }
        if !p.parent().is_some_and(|d| scanned.contains(d)) {
            return false;
        }
        match event.kind {
            EventKind::Create(CreateKind::Folder)
            | EventKind::Remove(RemoveKind::Folder)
            | EventKind::Modify(ModifyKind::Name(_)) => true,
            EventKind::Create(CreateKind::File) | EventKind::Remove(RemoveKind::File) => false,
            // The backend did not say what it was: a folder now, or gone.
            _ => p.is_dir() || !p.exists(),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wait up to a few seconds for the watch to come due.
    fn comes_due(w: &ScanWatch) -> bool {
        let end = Instant::now() + Duration::from_secs(5);
        while Instant::now() < end {
            if w.take_due() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    /// Nothing comes due within a settle period and a margin.
    fn stays_quiet(w: &ScanWatch) -> bool {
        std::thread::sleep(SETTLE + Duration::from_millis(300));
        !w.take_due()
    }

    #[test]
    fn a_new_repository_and_git_init_in_a_plain_folder_come_due() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("plain")).unwrap();
        let w = ScanWatch::new(&[root.path().to_path_buf()], 1).unwrap();
        assert!(!w.take_due(), "nothing happened yet");

        std::fs::create_dir_all(root.path().join("fresh/.git")).unwrap();
        assert!(comes_due(&w), "a new folder in the scan directory");
        assert!(!w.take_due(), "due only once per burst");

        std::fs::create_dir(root.path().join("plain/.git")).unwrap();
        assert!(comes_due(&w), "git init in a plain folder");
    }

    #[test]
    fn a_removed_repository_comes_due() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("gone/.git")).unwrap();
        let w = ScanWatch::new(&[root.path().to_path_buf()], 1).unwrap();
        std::fs::remove_dir_all(root.path().join("gone")).unwrap();
        assert!(comes_due(&w));
    }

    #[test]
    fn files_beside_the_repositories_and_inside_plain_folders_are_ignored() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("plain")).unwrap();
        let (note, inner) = (
            root.path().join("notes.txt"),
            root.path().join("plain/a.txt"),
        );
        std::fs::write(&note, "a").unwrap();
        std::fs::write(&inner, "a").unwrap();
        let w = ScanWatch::new(&[root.path().to_path_buf()], 1).unwrap();
        std::fs::write(&note, "b").unwrap();
        std::fs::write(root.path().join("new.txt"), "x").unwrap();
        std::fs::remove_file(&inner).unwrap();
        std::fs::create_dir(root.path().join("plain/sub")).unwrap();
        assert!(stays_quiet(&w));
    }
}
