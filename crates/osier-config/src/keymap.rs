//! `keymap.toml`: the user's changes to Osier's default key bindings.
//!
//! Only overrides are stored, keyed by action id (see the app's keymap registry), so new
//! defaults in later versions still arrive and the file stays short in dotfiles:
//!
//! ```toml
//! [bindings]
//! refresh = ["f5"]                 # replaces the defaults
//! focus_project_search = ["/", "ctrl-f"]
//! open_settings = []               # unbound
//! ```

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{ConfigError, Paths, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct KeymapFile {
    pub bindings: BTreeMap<String, Vec<String>>,
}

const HEADER: &str = "# Osier key bindings: only your changes to the defaults.\n\
# Keys use gpui notation: ctrl-shift-p, alt-enter, f5, /, space. An empty list unbinds.\n\
# \"prefix /\" fires only after the prefix key (prefix_key), \"prefix? /\" with or without it.\n\
# Edited in Settings > Keymap or by hand; Osier reloads this file when it changes.\n\n";

impl KeymapFile {
    pub fn load(paths: &Paths) -> Result<Self> {
        crate::read_toml(&paths.keymap_file())
    }

    /// Write the file, or remove it when there are no overrides left.
    pub fn save(&self, paths: &Paths) -> Result<()> {
        let path = paths.keymap_file();
        if self.bindings.is_empty() {
            return match std::fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                    Err(ConfigError::Io { path, source: e })
                }
                _ => Ok(()),
            };
        }
        let body = toml::to_string_pretty(self).map_err(|source| ConfigError::Serialise {
            what: "keymap",
            source,
        })?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| ConfigError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, format!("{HEADER}{body}")).map_err(|source| ConfigError::Io {
            path: tmp.clone(),
            source,
        })?;
        std::fs::rename(&tmp, &path).map_err(|source| ConfigError::Io { path, source })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_keeps_order_and_empty_unbinds() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        assert!(
            KeymapFile::load(&paths).unwrap().bindings.is_empty(),
            "missing is empty"
        );
        let mut k = KeymapFile::default();
        k.bindings.insert("refresh".into(), vec!["f5".into()]);
        k.bindings.insert("open_settings".into(), vec![]);
        k.save(&paths).unwrap();
        let text = std::fs::read_to_string(paths.keymap_file()).unwrap();
        assert!(text.starts_with("# Osier key bindings"));
        assert_eq!(KeymapFile::load(&paths).unwrap(), k);
        KeymapFile::default().save(&paths).unwrap();
        assert!(!paths.keymap_file().exists(), "no overrides, no file");
    }
}
