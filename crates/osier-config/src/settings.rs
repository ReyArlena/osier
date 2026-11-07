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
    pub keys: Keys,
    /// Keys this version does not know, preserved on save.
    #[serde(flatten)]
    pub extra: toml::Table,
}

/// Keyboard behaviour that is not a binding (bindings live in `keymap.toml`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Keys {
    /// How long the prefix key waits for the next key, in seconds. 0 waits until a key,
    /// Escape or a click.
    pub prefix_timeout_secs: u32,
}

impl Default for Keys {
    fn default() -> Self {
        Self {
            prefix_timeout_secs: 5,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    /// `dark`, `light` or `system`.
    pub mode: ThemeMode,
    pub dark_theme: String,
    pub light_theme: String,
    /// In System mode, what to show when the desktop states no light/dark preference (or has
    /// no settings portal): `dark` or `light`.
    pub system_fallback: SystemFallback,
    /// Follow Dux Shell (DankMaterialShell) colours. Unset means on when Dux Shell is
    /// detected; `true`/`false` is the user's explicit choice.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dux_shell_sync: Option<bool>,
    pub ui_font: Option<String>,
    pub mono_font: Option<String>,
    pub font_size: Option<f32>,
    /// How the sidebar marks a project with uncommitted changes.
    pub changes_indicator: ChangesIndicator,
    /// Show a `...` menu button at the end of list rows. Right-click opens the same menu
    /// either way.
    pub row_menu_buttons: bool,
    /// How long the pointer rests on something before its tooltip shows, in milliseconds.
    pub tooltip_delay_ms: u32,
    /// Selections, hovers and the sidebar highlight in a stronger tint of the theme's
    /// primary colour (like the switches), for more contrast. Off: the theme as designed.
    pub high_contrast: bool,
    /// A spinner and label in the status bar while Osier works quietly in the background
    /// (refreshes, project scans, GitHub lists, diff and branch lookups). Fetch, pull, push,
    /// clone and create always show.
    pub show_background_tasks: bool,
}

/// What System mode shows when the desktop has no light/dark preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SystemFallback {
    #[default]
    Dark,
    Light,
}

/// `icon` tints the project's folder icon, `dot` draws a dot before its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ChangesIndicator {
    #[default]
    Icon,
    Dot,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            mode: ThemeMode::System,
            dark_theme: "Catppuccin Mocha".into(),
            light_theme: "Catppuccin Latte".into(),
            system_fallback: SystemFallback::Dark,
            dux_shell_sync: None,
            ui_font: None,
            mono_font: None,
            font_size: None,
            changes_indicator: ChangesIndicator::Icon,
            row_menu_buttons: false,
            tooltip_delay_ms: 0,
            high_contrast: false,
            show_background_tasks: false,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Projects {
    /// Directories whose immediate children are scanned for repositories into Recents.
    pub scan_dirs: Vec<PathBuf>,
    pub scan_depth: Option<u8>,
    /// Badges (ahead/behind, uncommitted changes) on projects that are not open.
    pub recent_status: RecentStatusMode,
    /// Sweep interval for `recent_status = "check"`, 1 to 60 minutes.
    pub recent_status_minutes: u32,
    /// The tab to show instead of an empty Changes tab after switching project.
    pub empty_changes_tab: EmptyChangesTab,
}

impl Default for Projects {
    fn default() -> Self {
        Self {
            scan_dirs: Vec::new(),
            scan_depth: None,
            recent_status: RecentStatusMode::Never,
            recent_status_minutes: 2,
            empty_changes_tab: EmptyChangesTab::Log,
        }
    }
}

impl Projects {
    pub fn recent_status_interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs(u64::from(self.recent_status_minutes.clamp(1, 60)) * 60)
    }
}

/// Where to go when switching (or opening) a project while on the Changes tab and that
/// project has nothing to commit and no operation in progress. `Stay` keeps the Changes tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum EmptyChangesTab {
    Stay,
    #[default]
    Log,
    Branches,
    Stashes,
    Files,
    Patches,
}

/// How the sidebar learns the state of projects that are not open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RecentStatusMode {
    /// No badges, no cache, no watchers, no background git.
    #[default]
    Never,
    /// Values remembered from when the project was last open. No background git.
    Cached,
    /// Watch each repository's `.git`; re-read ahead/behind when it changes. No change marker.
    Watch,
    /// `Watch` plus a full status of every repository every N minutes and on window focus.
    Check,
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
    /// Where the Log puts branch and tag badges: before the subject or at the end of the line.
    pub log_badges: LogBadges,
    /// How the Log graph marks commits that are not on any remote yet.
    pub log_unpushed: UnpushedStyle,
}

/// `dim` fades unpushed commits in the graph, `colour` draws them in another colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum UnpushedStyle {
    #[default]
    Dim,
    #[serde(alias = "color")]
    Colour,
}

/// `left` puts ref badges before the commit subject, `right` at the end of the line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LogBadges {
    #[default]
    Left,
    Right,
}

impl Default for Git {
    fn default() -> Self {
        Self {
            log_page_size: 500,
            show_command_log: false,
            log_badges: LogBadges::Left,
            log_unpushed: UnpushedStyle::Dim,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Viewer {
    /// Fetch images referenced by http(s) URLs in markdown (README badges and the like).
    /// Off means only files inside the repository are shown.
    pub remote_images: bool,
    /// How markdown files open in the Files tab.
    pub markdown_default: MarkdownMode,
    /// Per extension overrides (`md`, `mdx`, ...), written when the Rendered/Code toggle is
    /// used on a file of that type.
    pub markdown_modes: std::collections::BTreeMap<String, MarkdownMode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum MarkdownMode {
    #[default]
    Rendered,
    Code,
}

impl Viewer {
    /// The mode for a file name, from its extension override or the default.
    pub fn markdown_mode_for(&self, path: &str) -> MarkdownMode {
        extension(path)
            .and_then(|e| self.markdown_modes.get(&e).copied())
            .unwrap_or(self.markdown_default)
    }
}

/// Lower-case extension of a path, without the dot.
pub fn extension(path: &str) -> Option<String> {
    let name = path.rsplit('/').next()?;
    let (stem, ext) = name.rsplit_once('.')?;
    (!stem.is_empty() && !ext.is_empty()).then(|| ext.to_ascii_lowercase())
}

impl Default for Viewer {
    fn default() -> Self {
        Self {
            remote_images: true,
            markdown_default: MarkdownMode::Rendered,
            markdown_modes: Default::default(),
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

    #[test]
    fn markdown_mode_by_extension() {
        let mut v = Viewer::default();
        assert_eq!(v.markdown_mode_for("README.md"), MarkdownMode::Rendered);
        v.markdown_modes.insert("mdx".into(), MarkdownMode::Code);
        assert_eq!(v.markdown_mode_for("docs/page.MDX"), MarkdownMode::Code);
        assert_eq!(v.markdown_mode_for("notes.md"), MarkdownMode::Rendered);
        v.markdown_default = MarkdownMode::Code;
        assert_eq!(v.markdown_mode_for("notes.md"), MarkdownMode::Code);
        assert_eq!(extension(".bashrc"), None);
        assert_eq!(extension("a/b.c/README"), None);
    }

    #[test]
    fn unpushed_style_defaults_to_dim_and_reads_colour_or_color() {
        assert_eq!(Settings::default().git.log_unpushed, UnpushedStyle::Dim);
        for v in ["colour", "color"] {
            let s: Settings = toml::from_str(&format!("[git]\nlog_unpushed = \"{v}\"\n")).unwrap();
            assert_eq!(s.git.log_unpushed, UnpushedStyle::Colour);
        }
    }

    #[test]
    fn log_badges_default_left_and_read_right() {
        assert_eq!(Settings::default().git.log_badges, LogBadges::Left);
        let s: Settings = toml::from_str("[git]\nlog_badges = \"right\"\n").unwrap();
        assert_eq!(s.git.log_badges, LogBadges::Right);
    }

    #[test]
    fn empty_changes_tab_defaults_to_log_and_reads_stay() {
        assert_eq!(
            Settings::default().projects.empty_changes_tab,
            EmptyChangesTab::Log
        );
        let s: Settings = toml::from_str("[projects]\nempty_changes_tab = \"stay\"\n").unwrap();
        assert_eq!(s.projects.empty_changes_tab, EmptyChangesTab::Stay);
        let s: Settings = toml::from_str("[projects]\nempty_changes_tab = \"branches\"\n").unwrap();
        assert_eq!(s.projects.empty_changes_tab, EmptyChangesTab::Branches);
    }

    #[test]
    fn changes_indicator_defaults_to_icon_and_reads_dot() {
        assert_eq!(
            Settings::default().appearance.changes_indicator,
            ChangesIndicator::Icon
        );
        let s: Settings = toml::from_str("[appearance]\nchanges_indicator = \"dot\"\n").unwrap();
        assert_eq!(s.appearance.changes_indicator, ChangesIndicator::Dot);
    }

    #[test]
    fn recent_status_defaults_off_and_clamps_the_interval() {
        let p = Settings::default().projects;
        assert_eq!(p.recent_status, RecentStatusMode::Never);
        let s: Settings =
            toml::from_str("[projects]\nrecent_status = \"check\"\nrecent_status_minutes = 600\n")
                .unwrap();
        assert_eq!(s.projects.recent_status, RecentStatusMode::Check);
        assert_eq!(s.projects.recent_status_interval().as_secs(), 3600);
    }
}
