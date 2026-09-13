//! User settings (`config.toml`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Paths, Result};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Settings {
    pub appearance: Appearance,
    pub editor: Editor,
    pub projects: Projects,
    pub diff: Diff,
    pub git: Git,
    pub auth: Auth,
    pub viewer: Viewer,
    /// Keys this version does not know, preserved on save.
    #[serde(flatten)]
    pub extra: toml::Table,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    /// `dark`, `light` or `system`.
    pub mode: ThemeMode,
    pub dark_theme: String,
    pub light_theme: String,
    /// Follow Dux Shell (DankMaterialShell) colours when available.
    pub dux_shell_sync: bool,
    pub ui_font: Option<String>,
    pub mono_font: Option<String>,
    pub font_size: Option<f32>,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            mode: ThemeMode::Dark,
            dark_theme: "Catppuccin Mocha".into(),
            light_theme: "Catppuccin Latte".into(),
            dux_shell_sync: false,
            ui_font: None,
            mono_font: None,
            font_size: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    #[default]
    Dark,
    Light,
    System,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Editor {
    /// Command to open a path. `{path}` is replaced; `{file}` and `{line}` when opening a file.
    /// `None` means the system default (`xdg-open`).
    pub command: Option<String>,
    pub terminal: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Projects {
    /// Directories whose immediate children are scanned for repositories into Recents.
    pub scan_dirs: Vec<PathBuf>,
    pub scan_depth: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Diff {
    pub context_lines: u32,
    pub line_numbers: bool,
    pub soft_wrap: bool,
    /// `unified` or `split`.
    pub mode: DiffMode,
    /// Files larger than this (bytes) ask before rendering.
    pub large_file_bytes: u64,
}

impl Default for Diff {
    fn default() -> Self {
        Self {
            context_lines: 3,
            line_numbers: true,
            soft_wrap: true,
            mode: DiffMode::Unified,
            large_file_bytes: 2 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DiffMode {
    #[default]
    Unified,
    Split,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Git {
    /// Commits loaded per page in the log.
    pub log_page_size: usize,
    /// Show the git command log panel on startup.
    pub show_command_log: bool,
}

impl Default for Git {
    fn default() -> Self {
        Self {
            log_page_size: 500,
            show_command_log: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Viewer {
    /// Fetch images referenced by http(s) URLs in markdown (README badges and the like).
    /// Off means only files inside the repository are shown.
    pub remote_images: bool,
}

impl Default for Viewer {
    fn default() -> Self {
        Self {
            remote_images: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Auth {
    /// Default gh account per host, used when a project has no choice recorded.
    pub default_accounts: std::collections::BTreeMap<String, String>,
    /// Route ssh and git prompts to Osier's own askpass window.
    pub askpass: bool,
}

impl Default for Auth {
    fn default() -> Self {
        Self {
            default_accounts: Default::default(),
            askpass: true,
        }
    }
}

impl Settings {
    pub fn load(paths: &Paths) -> Result<Self> {
        crate::read_toml(&paths.settings_file())
    }

    pub fn save(&self, paths: &Paths) -> Result<()> {
        crate::write_toml(&paths.settings_file(), "settings", self)
    }

    /// Write the defaults when no file exists yet, so users have something to edit.
    pub fn ensure_exists(paths: &Paths) -> Result<()> {
        let file = paths.settings_file();
        if file.exists() {
            return Ok(());
        }
        Self::default().save(paths)
    }

    pub fn file(paths: &Paths) -> PathBuf {
        paths.settings_file()
    }

    pub fn exists(path: &Path) -> bool {
        path.is_file()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_unknown_keys_survive() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        assert_eq!(Settings::load(&paths).unwrap(), Settings::default());
        Settings::ensure_exists(&paths).unwrap();
        let mut text = std::fs::read_to_string(paths.settings_file()).unwrap();
        assert!(text.contains("[appearance]"));
        text.push_str("\n[future]\nkey = 1\n");
        std::fs::write(paths.settings_file(), text).unwrap();
        let mut s = Settings::load(&paths).unwrap();
        assert!(s.extra.contains_key("future"));
        s.appearance.mode = ThemeMode::Light;
        s.save(&paths).unwrap();
        let again = Settings::load(&paths).unwrap();
        assert_eq!(again.appearance.mode, ThemeMode::Light);
        assert!(again.extra.contains_key("future"));
    }

    #[test]
    fn bad_toml_is_a_parse_error() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        std::fs::create_dir_all(&paths.config_dir).unwrap();
        std::fs::write(paths.settings_file(), "not = [valid").unwrap();
        assert!(matches!(
            Settings::load(&paths),
            Err(crate::ConfigError::Parse { .. })
        ));
    }
}
