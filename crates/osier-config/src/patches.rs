//! The patch clipboard: named patches kept in `<state>/patches/`.
//!
//! Each patch is a plain `<id>.patch` file next to an optional `<id>.toml` with its name,
//! origin and file list. A `.patch` dropped into the directory by hand shows up too, named
//! after its file.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::{ConfigError, Paths, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PatchMeta {
    pub name: String,
    /// Unix seconds.
    pub created: u64,
    pub source_repo: Option<PathBuf>,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredPatch {
    /// File stem, unique within the directory.
    pub id: String,
    pub meta: PatchMeta,
    pub path: PathBuf,
}

impl StoredPatch {
    pub fn text(&self) -> Result<String> {
        std::fs::read_to_string(&self.path).map_err(|source| ConfigError::Io {
            path: self.path.clone(),
            source,
        })
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Lower-case ASCII words joined by dashes, at most 40 chars; `patch` when nothing is left.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
        if out.len() >= 40 {
            break;
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() { "patch".into() } else { out }
}

fn meta_path(patch: &Path) -> PathBuf {
    patch.with_extension("toml")
}

/// Every stored patch, newest first.
pub fn list(paths: &Paths) -> Result<Vec<StoredPatch>> {
    let dir = paths.patches_dir();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(ConfigError::Io { path: dir, source }),
    };
    let mut out = Vec::new();
    for entry in entries.filter_map(std::result::Result::ok) {
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("patch") {
            continue;
        }
        let Some(id) = path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(str::to_string)
        else {
            continue;
        };
        let meta = match std::fs::read_to_string(meta_path(&path)) {
            Ok(text) => toml::from_str::<PatchMeta>(&text).unwrap_or_default(),
            Err(_) => PatchMeta::default(),
        };
        let mut meta = meta;
        if meta.name.is_empty() {
            meta.name = id.clone();
        }
        if meta.created == 0 {
            meta.created = std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
        }
        out.push(StoredPatch { id, meta, path });
    }
    out.sort_by(|a, b| {
        b.meta
            .created
            .cmp(&a.meta.created)
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(out)
}

/// Store a patch. The id is derived from the time and the name and made unique.
pub fn save(
    paths: &Paths,
    name: &str,
    source_repo: Option<&Path>,
    files: Vec<String>,
    text: &str,
) -> Result<StoredPatch> {
    let dir = paths.patches_dir();
    std::fs::create_dir_all(&dir).map_err(|source| ConfigError::Io {
        path: dir.clone(),
        source,
    })?;
    let created = now();
    let base = format!("{created}-{}", slug(name));
    let mut id = base.clone();
    let mut n = 1;
    while dir.join(format!("{id}.patch")).exists() {
        n += 1;
        id = format!("{base}-{n}");
    }
    let path = dir.join(format!("{id}.patch"));
    let name = if name.trim().is_empty() {
        id.clone()
    } else {
        name.trim().to_string()
    };
    let meta = PatchMeta {
        name,
        created,
        source_repo: source_repo.map(Path::to_path_buf),
        files,
    };
    std::fs::write(&path, text).map_err(|source| ConfigError::Io {
        path: path.clone(),
        source,
    })?;
    write_meta(&path, &meta)?;
    Ok(StoredPatch { id, meta, path })
}

fn write_meta(patch: &Path, meta: &PatchMeta) -> Result<()> {
    let text = toml::to_string_pretty(meta).map_err(|source| ConfigError::Serialise {
        what: "patch metadata",
        source,
    })?;
    let path = meta_path(patch);
    std::fs::write(&path, text).map_err(|source| ConfigError::Io { path, source })
}

pub fn rename(paths: &Paths, id: &str, name: &str) -> Result<()> {
    let Some(mut patch) = list(paths)?.into_iter().find(|p| p.id == id) else {
        return Ok(());
    };
    patch.meta.name = name.trim().to_string();
    write_meta(&patch.path, &patch.meta)
}

pub fn delete(paths: &Paths, id: &str) -> Result<()> {
    let Some(patch) = list(paths)?.into_iter().find(|p| p.id == id) else {
        return Ok(());
    };
    for p in [patch.path.clone(), meta_path(&patch.path)] {
        if let Err(source) = std::fs::remove_file(&p)
            && source.kind() != std::io::ErrorKind::NotFound
        {
            return Err(ConfigError::Io { path: p, source });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_list_rename_delete() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        assert!(list(&paths).unwrap().is_empty(), "missing dir is empty");
        let a = save(
            &paths,
            "Fix login",
            Some(Path::new("/r")),
            vec!["a.rs".into()],
            "diff --git a/a.rs b/a.rs\n",
        )
        .unwrap();
        let b = save(&paths, "Fix login", None, vec![], "x").unwrap();
        assert_ne!(a.id, b.id, "same name gets a unique id");
        assert!(a.id.ends_with("-fix-login"));
        let all = list(&paths).unwrap();
        assert_eq!(all.len(), 2);
        assert!(all.iter().any(|p| p.id == a.id && p.meta.files == ["a.rs"]));
        assert_eq!(
            all.iter().find(|p| p.id == a.id).unwrap().text().unwrap(),
            "diff --git a/a.rs b/a.rs\n"
        );

        rename(&paths, &a.id, "Login fix").unwrap();
        assert_eq!(
            list(&paths)
                .unwrap()
                .iter()
                .find(|p| p.id == a.id)
                .unwrap()
                .meta
                .name,
            "Login fix"
        );

        // A bare .patch file dropped into the folder is listed under its file name.
        std::fs::write(paths.patches_dir().join("manual.patch"), "diff\n").unwrap();
        let listed = list(&paths).unwrap();
        let manual = listed.iter().find(|p| p.id == "manual").unwrap();
        assert_eq!(manual.meta.name, "manual");
        assert!(manual.meta.created > 0);

        delete(&paths, &a.id).unwrap();
        delete(&paths, "manual").unwrap();
        assert_eq!(list(&paths).unwrap().len(), 1);
        assert!(!a.path.exists() && !a.path.with_extension("toml").exists());
    }

    #[test]
    fn slugs() {
        assert_eq!(slug("Fix the Login!!"), "fix-the-login");
        assert_eq!(slug("   "), "patch");
        assert_eq!(slug("ünïcode name"), "n-code-name");
        assert!(slug(&"x".repeat(100)).len() <= 40);
    }
}
