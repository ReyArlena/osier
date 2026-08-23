//! XDG directories. Config is what users sync; state and cache stay local.

use std::path::{Path, PathBuf};

use crate::{ConfigError, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub config_dir: PathBuf,
    pub state_dir: PathBuf,
    pub cache_dir: PathBuf,
}

impl Paths {
    /// Resolve from the environment (`XDG_*` or the platform defaults). `OSIER_CONFIG_DIR`,
    /// `OSIER_STATE_DIR` and `OSIER_CACHE_DIR` override individual directories (used by tests).
    pub fn discover() -> Result<Self> {
        let dirs = directories::ProjectDirs::from("com", "reyarlena", "osier")
            .ok_or(ConfigError::NoHome)?;
        let env = |key: &str| std::env::var_os(key).map(PathBuf::from);
        Ok(Self {
            config_dir: env("OSIER_CONFIG_DIR").unwrap_or_else(|| dirs.config_dir().to_path_buf()),
            state_dir: env("OSIER_STATE_DIR").unwrap_or_else(|| {
                dirs.state_dir()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| dirs.data_local_dir().to_path_buf())
            }),
            cache_dir: env("OSIER_CACHE_DIR").unwrap_or_else(|| dirs.cache_dir().to_path_buf()),
        })
    }

    /// All three directories under one root. For tests.
    pub fn under(root: &Path) -> Self {
        Self {
            config_dir: root.join("config"),
            state_dir: root.join("state"),
            cache_dir: root.join("cache"),
        }
    }

    pub fn settings_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    pub fn themes_dir(&self) -> PathBuf {
        self.config_dir.join("themes")
    }

    pub fn state_file(&self) -> PathBuf {
        self.state_dir.join("state.toml")
    }

    pub fn history_dir(&self) -> PathBuf {
        self.state_dir.join("history")
    }

    pub fn patches_dir(&self) -> PathBuf {
        self.state_dir.join("patches")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discover_uses_xdg_layout_on_linux() {
        let p = Paths::discover().unwrap();
        assert!(p.config_dir.ends_with("osier"), "{:?}", p.config_dir);
        assert_eq!(p.settings_file().file_name().unwrap(), "config.toml");
    }
}
