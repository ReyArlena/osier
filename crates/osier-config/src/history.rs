//! Operation history: one JSON line per mutating operation, per repository.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::{ConfigError, Paths, Result};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// Unix seconds.
    pub time: u64,
    pub repo: PathBuf,
    /// Short human description, e.g. `Stage 2 files`.
    pub op: String,
    pub ok: bool,
    pub error: Option<String>,
    pub head_before: Option<String>,
    pub head_after: Option<String>,
}

impl HistoryEntry {
    pub fn now(repo: &Path, op: impl Into<String>) -> Self {
        Self {
            time: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            repo: repo.to_path_buf(),
            op: op.into(),
            ok: true,
            error: None,
            head_before: None,
            head_after: None,
        }
    }
}

/// File holding the history of one repository.
pub fn file_for(paths: &Paths, repo: &Path) -> PathBuf {
    let mut h = DefaultHasher::new();
    repo.hash(&mut h);
    let name = repo
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "repo".into());
    paths
        .history_dir()
        .join(format!("{name}-{:016x}.jsonl", h.finish()))
}

pub fn append(paths: &Paths, entry: &HistoryEntry) -> Result<()> {
    let file = file_for(paths, &entry.repo);
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(|source| ConfigError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let mut line = serde_json::to_string(entry).map_err(|e| ConfigError::Io {
        path: file.clone(),
        source: std::io::Error::other(e),
    })?;
    line.push('\n');
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file)
        .map_err(|source| ConfigError::Io {
            path: file.clone(),
            source,
        })?;
    f.write_all(line.as_bytes())
        .map_err(|source| ConfigError::Io { path: file, source })
}

/// Entries for `repo`, oldest first. Malformed lines are skipped.
pub fn read(paths: &Paths, repo: &Path) -> Result<Vec<HistoryEntry>> {
    let file = file_for(paths, repo);
    match std::fs::read_to_string(&file) {
        Ok(text) => Ok(text
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(source) => Err(ConfigError::Io { path: file, source }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        let repo = Path::new("/tmp/example");
        let mut e = HistoryEntry::now(repo, "Stage 1 file");
        append(&paths, &e).unwrap();
        e.ok = false;
        e.error = Some("boom".into());
        append(&paths, &e).unwrap();
        let back = read(&paths, repo).unwrap();
        assert_eq!(back.len(), 2);
        assert!(back[0].ok);
        assert_eq!(back[1].error.as_deref(), Some("boom"));
        assert!(read(&paths, Path::new("/tmp/other")).unwrap().is_empty());
    }
}
