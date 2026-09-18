//! Notice when `config.toml` changes on disk (dotfile sync, an editor) so settings reload
//! without a restart. The flag is polled by the tick loop.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use notify::Watcher as _;

pub struct ConfigWatch {
    dirty: Arc<AtomicBool>,
    _watcher: notify::RecommendedWatcher,
}

impl ConfigWatch {
    /// Watch the directory holding `file` for changes to that file name.
    pub fn new(file: &Path) -> Option<Self> {
        let dir = file.parent()?.to_path_buf();
        let name = file.file_name()?.to_os_string();
        let dirty = Arc::new(AtomicBool::new(false));
        let flag = dirty.clone();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(event) = res
                && matches!(
                    event.kind,
                    notify::EventKind::Any
                        | notify::EventKind::Create(_)
                        | notify::EventKind::Modify(_)
                        | notify::EventKind::Remove(_)
                )
                && event
                    .paths
                    .iter()
                    .any(|p| p.file_name() == Some(name.as_os_str()))
            {
                flag.store(true, Ordering::Relaxed);
            }
        })
        .map_err(|error| tracing::warn!(%error, "cannot create config watcher"))
        .ok()?;
        watcher
            .watch(&dir, notify::RecursiveMode::NonRecursive)
            .map_err(|error| tracing::warn!(%error, dir = %dir.display(), "cannot watch config"))
            .ok()?;
        Some(Self {
            dirty,
            _watcher: watcher,
        })
    }

    pub fn take_dirty(&self) -> bool {
        self.dirty.swap(false, Ordering::Relaxed)
    }
}
