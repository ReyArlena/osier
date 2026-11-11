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
    pub updates: Updates,
    pub github: GitHub,
    pub palette: Palette,
    /// Keys this version does not know, preserved on save.
    #[serde(flatten)]
    pub extra: toml::Table,
}

/// The command palette.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Palette {
    /// Searching the commands also lists matching rows of their submenus (Fetch, Push, Switch
    /// branch, Switch project…), after the commands themselves.
    pub search_submenus: bool,
    /// Each command's category (General, Remote, Tabs…) beside its shortcut.
    pub show_categories: bool,
}

/// GitHub lists Osier keeps ready (repositories to clone, organisations, templates,
/// `.gitignore` and licence lists).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GitHub {
    /// Refresh the lists in the background this often, in minutes; 0 only when a screen
    /// that shows them opens.
    pub list_refresh_minutes: u32,
}

impl Default for GitHub {
    fn default() -> Self {
        Self {
            list_refresh_minutes: 30,
        }
    }
}

/// Checking GitHub for a newer Osier release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Updates {
    /// Look for a newer release at startup (at most once a day) and show a status bar badge
    /// when there is one.
    pub check_on_startup: bool,
}

impl Default for Updates {
    fn default() -> Self {
        Self {
            check_on_startup: true,
        }
    }
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
    /// How long a notification stays in the corner before it slides away, in seconds. 0
    /// keeps it until it is closed.
    pub toast_seconds: u32,
    /// Selections, hovers and the sidebar highlight in a stronger tint of the theme's
    /// primary colour (like the switches), for more contrast. Off: the theme as designed.
    pub high_contrast: bool,
    /// Lift syntax colours that sit too close to the editor background (comments above all)
    /// until they read, whatever the theme. Off: the theme's colours as they are.
    pub syntax_contrast: bool,
    /// A spinner and label in the status bar while Osier works quietly in the background
    /// (refreshes, project scans, GitHub lists, diff and branch lookups). Fetch, pull, push,
    /// clone and create always show.
    pub show_background_tasks: bool,
    /// Dialogs, menus and toasts animating in, and spinners turning: `system`
    /// follows the desktop's reduced-motion setting, `on` and `off` override it.
    pub animations: Animations,
    /// How long a dialog takes to fade in, in milliseconds. gpui-kit's other transitions
    /// (switches, tabs, collapsing sections) scale with it.
    pub animation_ms: u32,
}

/// Whether the interface animates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Animations {
    /// Follow the desktop (on when it says nothing).
    #[default]
    System,
    On,
    Off,
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
            toast_seconds: 5,
            high_contrast: false,
            syntax_contrast: true,
            show_background_tasks: false,
            animations: Animations::System,
            animation_ms: 180,
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
    /// `unified` or `split`, for files without conflicts.
    pub mode: DiffMode,
    /// How conflicted files show: `three-way` (the merge tool) or `inline` (conflict blocks
    /// with accept buttons).
    pub conflicts: ConflictView,
    /// Whitespace differences to hide: `none`, `trim` (amount of whitespace and line ends)
    /// or `all`. Commits, comparisons and the merge tool; never stageable changes.
    pub whitespace: DiffWhitespace,
    /// Highlight the changed words inside changed lines.
    pub word_highlights: bool,
    /// The merge tool's panes scroll together.
    pub sync_scroll: bool,
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
            conflicts: ConflictView::ThreeWay,
            whitespace: DiffWhitespace::None,
            word_highlights: true,
            sync_scroll: true,
            large_file_bytes: 2 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DiffMode {
    #[default]
    Unified,
    // Development builds wrote "three-way" here before it moved to `conflicts`.
    #[serde(alias = "three-way")]
    Split,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DiffWhitespace {
    #[default]
    None,
    Trim,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ConflictView {
    /// Yours, the result and theirs side by side.
    #[default]
    ThreeWay,
    /// The working file's conflict blocks with Accept Current / Incoming / Both.
    Inline,
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
    /// After a pull deletes files that an ignore rule now matches (someone stopped tracking
    /// them), offer to write the local copies back from before the pull, untracked.
    pub offer_keep_ignored: bool,
    /// Show the Git config panel as a tab too (it is always in the project's sidebar menu).
    pub config_tab: bool,
}

/// `dim` fades unpushed commits in the graph (and their row's text a little), `colour` draws
/// them in another colour, `off` draws them like any other commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum UnpushedStyle {
    #[default]
    Dim,
    #[serde(alias = "color")]
    Colour,
    Off,
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
            offer_keep_ignored: true,
            config_tab: false,
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
    fn animations_default_to_the_system_and_read_their_names() {
        assert_eq!(
            Settings::default().appearance.animations,
            Animations::System
        );
        for (text, want) in [
            ("system", Animations::System),
            ("on", Animations::On),
            ("off", Animations::Off),
        ] {
            let s: Settings =
                toml::from_str(&format!("[appearance]\nanimations = \"{text}\"\n")).unwrap();
            assert_eq!(s.appearance.animations, want);
        }
        assert!(toml::from_str::<Settings>("[appearance]\nanimations = \"fast\"\n").is_err());
        assert_eq!(Settings::default().appearance.animation_ms, 180);
        let s: Settings = toml::from_str("[appearance]\ntoast_seconds = 2\n").unwrap();
        assert_eq!(s.appearance.toast_seconds, 2);
        assert_eq!(Settings::default().appearance.toast_seconds, 5);
        let s: Settings = toml::from_str("[appearance]\nanimation_ms = 120\n").unwrap();
        assert_eq!(s.appearance.animation_ms, 120);
    }

    #[test]
    fn github_lists_refresh_every_half_hour_by_default() {
        assert_eq!(Settings::default().github.list_refresh_minutes, 30);
        let s: Settings = toml::from_str("[github]\nlist_refresh_minutes = 0\n").unwrap();
        assert_eq!(s.github.list_refresh_minutes, 0);
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
        {
            let s: Settings = toml::from_str("[git]\nlog_unpushed = \"off\"\n").unwrap();
            assert_eq!(s.git.log_unpushed, UnpushedStyle::Off);
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
