//! Last known state of projects that are not open (`<cache>/project-status.toml`).
//!
//! Derived data: safe to delete, never synced. Written only when the
//! `projects.recent_status` setting is not `never`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Paths, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct RepoBadge {
    /// Uncommitted changes, when known.
    pub dirty: Option<bool>,
    pub ahead: u32,
    pub behind: u32,
    /// Unix seconds of the check.
    pub checked_at: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct StatusCache {
    /// Keyed by repository path.
    pub repos: BTreeMap<String, RepoBadge>,
}

impl StatusCache {
    pub fn file(paths: &Paths) -> PathBuf {
        paths.cache_dir().join("project-status.toml")
    }

    /// A missing or unreadable cache is an empty one: it is only a hint.
    pub fn load(paths: &Paths) -> Self {
        crate::read_toml(&Self::file(paths)).unwrap_or_default()
    }

    pub fn save(&self, paths: &Paths) -> Result<()> {
        crate::write_toml(&Self::file(paths), "project status cache", self)
    }

    pub fn get(&self, repo: &Path) -> Option<RepoBadge> {
        self.repos
            .get(&repo.to_string_lossy().into_owned())
            .copied()
    }

    pub fn set(&mut self, repo: &Path, badge: RepoBadge) {
        self.repos
            .insert(repo.to_string_lossy().into_owned(), badge);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_unreadable_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        assert!(StatusCache::load(&paths).repos.is_empty());
        let mut c = StatusCache::default();
        let badge = RepoBadge {
            dirty: Some(true),
            ahead: 2,
            behind: 1,
            checked_at: 5,
        };
        c.set(Path::new("/r/one"), badge);
        c.save(&paths).unwrap();
        assert_eq!(
            StatusCache::load(&paths).get(Path::new("/r/one")),
            Some(badge)
        );
        std::fs::write(StatusCache::file(&paths), "not = [toml").unwrap();
        assert!(StatusCache::load(&paths).repos.is_empty());
    }
}
