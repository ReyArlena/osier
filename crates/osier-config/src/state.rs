//! Local state (`state.toml`): open projects, recents, layout. Not for dotfiles.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Paths, Result};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct State {
    /// Repositories shown under "Open", in sidebar order.
    pub open_projects: Vec<PathBuf>,
    /// Last active project.
    pub active_project: Option<PathBuf>,
    /// Most recent first.
    pub recent: Vec<PathBuf>,
    pub layout: Layout,
    /// Last window position and size (logical pixels). Compositors may ignore it.
    pub window: Option<WindowGeometry>,
    /// Recent commit messages per repository path, newest first.
    pub commit_messages: std::collections::BTreeMap<String, Vec<String>>,
    /// GitHub (gh) account chosen per repository path ("always use for this project").
    pub accounts: std::collections::BTreeMap<String, String>,
    /// Pull strategy remembered per repository path: `merge` or `rebase`.
    pub pull_strategies: std::collections::BTreeMap<String, String>,
    #[serde(flatten)]
    pub extra: toml::Table,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct WindowGeometry {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Layout {
    pub sidebar_visible: bool,
    pub center_width: Option<f32>,
    pub command_log_visible: bool,
    pub command_log_height: Option<f32>,
}

const MAX_RECENT: usize = 30;

impl State {
    pub fn load(paths: &Paths) -> Result<Self> {
        let mut s: Self = crate::read_toml(&paths.state_file())?;
        if s.layout == Layout::default() && s.open_projects.is_empty() {
            s.layout.sidebar_visible = true;
        }
        Ok(s)
    }

    pub fn save(&self, paths: &Paths) -> Result<()> {
        crate::write_toml(&paths.state_file(), "state", self)
    }

    /// Add to open projects (if new), make active, bump in recents.
    pub fn open(&mut self, path: &Path) {
        if !self.open_projects.iter().any(|p| p == path) {
            self.open_projects.push(path.to_path_buf());
        }
        self.active_project = Some(path.to_path_buf());
        self.touch_recent(path);
    }

    pub fn close(&mut self, path: &Path) {
        self.open_projects.retain(|p| p != path);
        if self.active_project.as_deref() == Some(path) {
            self.active_project = self.open_projects.first().cloned();
        }
    }

    pub fn touch_recent(&mut self, path: &Path) {
        self.recent.retain(|p| p != path);
        self.recent.insert(0, path.to_path_buf());
        self.recent.truncate(MAX_RECENT);
    }

    /// Remember a commit message for `repo` (deduplicated, newest first, capped).
    pub fn push_commit_message(&mut self, repo: &Path, message: &str) {
        let key = repo.to_string_lossy().into_owned();
        let list = self.commit_messages.entry(key).or_default();
        list.retain(|m| m != message);
        list.insert(0, message.to_string());
        list.truncate(20);
    }

    pub fn forget_recent(&mut self, path: &Path) {
        self.recent.retain(|p| p != path);
    }

    /// Recents that are not currently open, in order.
    pub fn recent_not_open(&self) -> impl Iterator<Item = &PathBuf> {
        self.recent
            .iter()
            .filter(|p| !self.open_projects.contains(p))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_close_and_recents() {
        let mut s = State::default();
        let a = PathBuf::from("/tmp/a");
        let b = PathBuf::from("/tmp/b");
        s.open(&a);
        s.open(&b);
        s.open(&a);
        assert_eq!(s.open_projects, vec![a.clone(), b.clone()]);
        assert_eq!(s.active_project.as_deref(), Some(a.as_path()));
        assert_eq!(s.recent, vec![a.clone(), b.clone()]);
        assert_eq!(s.recent_not_open().count(), 0);
        s.close(&a);
        assert_eq!(s.active_project.as_deref(), Some(b.as_path()));
        assert_eq!(s.recent_not_open().collect::<Vec<_>>(), vec![&a]);
    }

    #[test]
    fn persists() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        let mut s = State::load(&paths).unwrap();
        assert!(s.layout.sidebar_visible, "fresh state shows the sidebar");
        s.open(Path::new("/tmp/x"));
        s.save(&paths).unwrap();
        let again = State::load(&paths).unwrap();
        assert_eq!(again.open_projects, vec![PathBuf::from("/tmp/x")]);
    }
}
