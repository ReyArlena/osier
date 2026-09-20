//! Root view: owns projects, settings, state and the layout.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::diff::{DiffEvent, DiffFile, DiffMode, DiffState};
use gpui_kit::component::input::{InputEvent, TextareaState};
use gpui_kit::component::resizable::{ResizableState, h_resizable, resizable_panel};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use osier_config::settings::ThemeMode;
use osier_config::{Paths, Settings, State};
use osier_git::diff::{DiffOptions, DiffTarget};
use osier_git::{GitRunner, Repository};

use crate::command_log::UiCommandSink;
use crate::project::{Area, Project, Selection, load_snapshot};

gpui_kit::actions!(
    osier,
    [
        Quit,
        Refresh,
        OpenRepository,
        CloseProject,
        ToggleSidebar,
        ToggleCommandLog,
        ToggleTheme,
        OpenSettings,
        OpenCommandPalette,
        TabChanges,
        TabLog,
        TabBranches,
        TabStashes,
        TabFiles,
        TabPatches,
        SelectNext,
        SelectPrev,
        ToggleStageSelected,
        DiscardSelected,
        CommitStaged,
        Fetch,
        Pull,
        Push,
    ]
);

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-q", Quit, None),
        KeyBinding::new("ctrl-r", Refresh, None),
        KeyBinding::new("f5", Refresh, None),
        KeyBinding::new("ctrl-o", OpenRepository, None),
        KeyBinding::new("ctrl-w", CloseProject, None),
        KeyBinding::new("ctrl-b", ToggleSidebar, None),
        KeyBinding::new("ctrl-`", ToggleCommandLog, None),
        KeyBinding::new("ctrl-shift-t", ToggleTheme, None),
        KeyBinding::new("ctrl-,", OpenSettings, None),
        KeyBinding::new("ctrl-shift-a", OpenCommandPalette, None),
        KeyBinding::new("f1", OpenCommandPalette, None),
        KeyBinding::new("ctrl-1", TabChanges, None),
        KeyBinding::new("ctrl-2", TabLog, None),
        KeyBinding::new("ctrl-3", TabBranches, None),
        KeyBinding::new("ctrl-4", TabStashes, None),
        KeyBinding::new("ctrl-5", TabFiles, None),
        KeyBinding::new("ctrl-6", TabPatches, None),
        KeyBinding::new("ctrl-enter", CommitStaged, None),
        KeyBinding::new("ctrl-shift-f", Fetch, None),
        KeyBinding::new("ctrl-shift-l", Pull, None),
        KeyBinding::new("ctrl-shift-p", Push, None),
        KeyBinding::new("down", SelectNext, Some("OsierList")),
        KeyBinding::new("up", SelectPrev, Some("OsierList")),
        KeyBinding::new("j", SelectNext, Some("OsierList")),
        KeyBinding::new("k", SelectPrev, Some("OsierList")),
        KeyBinding::new("space", ToggleStageSelected, Some("OsierList")),
        KeyBinding::new("delete", DiscardSelected, Some("OsierList")),
    ]);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Changes,
    Log,
    Branches,
    Stashes,
    Files,
    Patches,
}

impl Tab {
    pub const ALL: [Tab; 6] = [
        Tab::Changes,
        Tab::Log,
        Tab::Branches,
        Tab::Stashes,
        Tab::Files,
        Tab::Patches,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Changes => "Changes",
            Tab::Log => "Log",
            Tab::Branches => "Branches",
            Tab::Stashes => "Stashes",
            Tab::Files => "Files",
            Tab::Patches => "Patches",
        }
    }
}

pub struct Osier {
    pub(crate) focus_handle: FocusHandle,
    pub(crate) paths: Paths,
    pub(crate) settings: Settings,
    pub(crate) state: State,
    pub(crate) runner: GitRunner,
    pub(crate) sink: Arc<UiCommandSink>,
    pub(crate) projects: Vec<Project>,
    pub(crate) active: Option<usize>,
    pub(crate) tab: Tab,
    /// Log tab shows the reflog instead of the commit list.
    pub(crate) show_reflog: bool,
    pub(crate) sidebar_visible: bool,
    pub(crate) command_log_visible: bool,
    pub(crate) expanded_command: Option<u64>,
    pub(crate) diff: Entity<DiffState>,
    /// Files in the diff pane, for the header counts.
    pub(crate) diff_files: usize,
    /// Our own parse of the same patch, for building hunk and line patches.
    pub(crate) diff_model: std::rc::Rc<Vec<osier_git::diff::DiffFile>>,
    pub(crate) diff_loading: bool,
    pub(crate) diff_error: Option<String>,
    pub(crate) diff_revision: u64,
    pub(crate) log_scroll: UniformListScrollHandle,
    pub(crate) global_error: Option<String>,
    /// Command log version last rendered, so the panel only re-renders on new records.
    log_version_seen: u64,
    /// Size of a file the diff refused to load, until "load anyway".
    pub(crate) diff_too_large: Option<u64>,
    pub(crate) diff_force_large: bool,
    /// Focus for keyboard navigation in the center lists.
    pub(crate) list_focus: FocusHandle,
    pub(crate) commit_message: Entity<TextareaState>,
    /// Editor shown in the diff pane for "edit result" / "edit file"; `None` when viewing.
    pub(crate) editor: Option<crate::edit_pane::FileEditor>,
    /// File viewer for the Files tab (code, markdown, image, binary).
    pub(crate) viewer: Option<crate::viewer::Viewer>,
    /// Markdown files show rendered by default; this toggles to source.
    pub(crate) markdown_as_code: bool,
    /// Filter box above the Files tree.
    pub(crate) files_filter: Entity<gpui_kit::component::input::InputState>,
    pub(crate) files_tree: Entity<gpui_kit::component::tree::TreeState>,
    pub(crate) show_ignored: bool,
    /// Hash of the path set the tree was built from, so expansion survives refreshes.
    pub(crate) files_tree_key: u64,
    pub(crate) amend: bool,
    pub(crate) signoff: bool,
    /// State changed by frequent events (bounds, pane drags); saved by the tick loop.
    state_dirty: bool,
    /// Running fetch/pull/push, if any.
    pub(crate) remote_job: Option<crate::remote_ops::RemoteJob>,
    /// gh accounts discovered at startup (no tokens).
    pub(crate) gh_accounts: Vec<osier_auth::GhAccount>,
    /// Repositories found under the configured scan directories.
    pub(crate) scanned: Vec<PathBuf>,
    /// Sidebar filter box.
    pub(crate) sidebar_filter: Entity<gpui_kit::component::input::InputState>,
    /// Projects whose account choice was "this time only" (not written to state on save).
    pub(crate) session_only_accounts: std::collections::BTreeSet<String>,
    /// The patch clipboard (`<state>/patches`), newest first.
    pub(crate) patches: Vec<osier_config::patches::StoredPatch>,
    pub(crate) patch_selected: Option<String>,
    /// Blame shown in the diff pane instead of the diff, until closed.
    pub(crate) blame: Option<crate::blame_view::BlameView>,
    /// Filled when the settings sheet opens.
    pub(crate) ssh_status: Option<osier_auth::SshStatus>,
    pub(crate) git_version: Option<String>,
    /// Focus target inside the settings sheet so Escape and its close button reach it.
    pub(crate) settings_focus: FocusHandle,
    pub(crate) config_watch: Option<crate::config_watch::ConfigWatch>,
    /// Narrow window: title-bar buttons drop their labels.
    pub(crate) compact: bool,
    /// Command log opened by the dev env var: do not persist that as a preference.
    command_log_from_env: bool,
    diff_task: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl Osier {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        paths: Paths,
        settings: Settings,
        state: State,
        runner: GitRunner,
        sink: Arc<UiCommandSink>,
        open_arg: Option<PathBuf>,
        startup_error: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        crate::themes::ThemeStore::ensure(paths.themes_dir(), true, cx);
        let patches = osier_config::patches::list(&paths).unwrap_or_else(|error| {
            tracing::warn!(%error, "could not list patches");
            Vec::new()
        });
        let config_watch = if cfg!(test) {
            None
        } else {
            crate::config_watch::ConfigWatch::new(&paths.settings_file())
        };
        let mode = match settings.diff.mode {
            osier_config::settings::DiffMode::Unified => DiffMode::Unified,
            osier_config::settings::DiffMode::Split => DiffMode::Split,
        };
        let diff = cx.new(|cx| {
            DiffState::new([], cx)
                .with_mode(mode)
                .with_context_lines(Some(settings.diff.context_lines as usize))
        });
        let commit_message = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(4)
                .placeholder("Commit message (ctrl-enter to commit)")
        });
        let files_filter = cx.new(|cx| {
            gpui_kit::component::input::InputState::new(window, cx).placeholder("Filter files")
        });
        let files_filter_subscription =
            cx.subscribe_in(&files_filter, window, |_, _, ev: &InputEvent, _, cx| {
                if matches!(ev, InputEvent::Change) {
                    cx.notify();
                }
            });
        let sidebar_filter = cx.new(|cx| {
            gpui_kit::component::input::InputState::new(window, cx).placeholder("Filter projects")
        });
        let filter_subscription = cx.subscribe_in(
            &sidebar_filter,
            window,
            |this, _, ev: &InputEvent, window, cx| match ev {
                InputEvent::Change => cx.notify(),
                InputEvent::PressEnter { .. } => this.open_first_filtered(window, cx),
                _ => {}
            },
        );
        let message_subscription =
            cx.subscribe_in(&commit_message, window, |_, _, ev: &InputEvent, _, cx| {
                if matches!(ev, InputEvent::Change) {
                    cx.notify();
                }
            });
        let mut this = Self {
            focus_handle: cx.focus_handle(),
            commit_message,
            editor: None,
            viewer: None,
            markdown_as_code: false,
            files_filter,
            files_tree: cx.new(|cx| gpui_kit::component::tree::TreeState::new(cx)),
            show_ignored: false,
            files_tree_key: 0,
            amend: false,
            signoff: false,
            sidebar_visible: state.layout.sidebar_visible,
            command_log_visible: state.layout.command_log_visible
                || settings.git.show_command_log
                || std::env::var_os("OSIER_SHOW_COMMAND_LOG").is_some(),
            command_log_from_env: std::env::var_os("OSIER_SHOW_COMMAND_LOG").is_some(),
            paths,
            settings,
            state,
            runner,
            sink,
            projects: Vec::new(),
            active: None,
            show_reflog: false,
            // Dev aid for screenshots: OSIER_START_TAB=log|branches|stashes|files|patches.
            tab: match std::env::var("OSIER_START_TAB").as_deref() {
                Ok("log") => Tab::Log,
                Ok("branches") => Tab::Branches,
                Ok("stashes") => Tab::Stashes,
                Ok("files") => Tab::Files,
                Ok("patches") => Tab::Patches,
                _ => Tab::Changes,
            },
            expanded_command: None,
            diff,
            diff_files: 0,
            diff_model: std::rc::Rc::new(Vec::new()),
            diff_loading: false,
            diff_error: None,
            diff_revision: 0,
            log_scroll: UniformListScrollHandle::new(),
            global_error: startup_error,
            log_version_seen: 0,
            diff_too_large: None,
            diff_force_large: false,
            list_focus: cx.focus_handle(),
            state_dirty: false,
            remote_job: None,
            gh_accounts: Vec::new(),
            scanned: Vec::new(),
            sidebar_filter,
            session_only_accounts: Default::default(),
            patches,
            patch_selected: None,
            blame: None,
            ssh_status: None,
            git_version: None,
            settings_focus: cx.focus_handle(),
            config_watch,
            compact: window.bounds().size.width < px(1500.),
            diff_task: Task::ready(()),
            _subscriptions: vec![
                message_subscription,
                filter_subscription,
                files_filter_subscription,
            ],
        };
        this.apply_appearance(cx);
        let diff_entity = this.diff.clone();
        this._subscriptions
            .push(cx.subscribe(&diff_entity, |_, _, ev: &DiffEvent, cx| {
                if matches!(
                    ev,
                    DiffEvent::ConflictResolved(..) | DiffEvent::SelectionChanged(_)
                ) {
                    cx.notify();
                }
            }));
        this._subscriptions
            .push(cx.observe_window_appearance(window, |this, _, cx| {
                if this.settings.appearance.mode == ThemeMode::System {
                    this.apply_appearance(cx);
                    cx.notify();
                }
            }));
        this._subscriptions
            .push(cx.observe_window_bounds(window, |this, window, cx| {
                let b = window.bounds();
                let compact = b.size.width < px(1500.);
                if compact != this.compact {
                    this.compact = compact;
                    cx.notify();
                }
                this.state.window = Some(osier_config::state::WindowGeometry {
                    x: f32::from(b.origin.x),
                    y: f32::from(b.origin.y),
                    width: f32::from(b.size.width),
                    height: f32::from(b.size.height),
                });
                this.state_dirty = true;
            }));

        // Restore open projects, then the CLI argument on top. Failures go to the status bar:
        // toasts need the window root, which does not exist yet during construction.
        let restore: Vec<PathBuf> = this.state.open_projects.clone();
        let mut failures = Vec::new();
        for p in restore {
            if let Err(e) = this.open_project(&p, false, cx) {
                failures.push(e);
            }
        }
        if !failures.is_empty() {
            this.global_error = Some(format!(
                "Could not reopen {}: {}",
                if failures.len() == 1 {
                    "a project".to_string()
                } else {
                    format!("{} projects", failures.len())
                },
                failures.join("; ")
            ));
        }
        if let Some(active) = this.state.active_project.clone()
            && let Some(ix) = this.projects.iter().position(|p| p.path == active)
        {
            this.activate(ix, window, cx);
        }
        if let Some(p) = open_arg {
            let p = std::fs::canonicalize(&p).unwrap_or(p);
            match this.open_project(&p, true, cx) {
                Ok(ix) => this.activate(ix, window, cx),
                Err(e) => this.global_error = Some(e),
            }
        }
        if this.active.is_none() && !this.projects.is_empty() {
            this.activate(0, window, cx);
        }
        // Dev aid for screenshots: OSIER_START_FILE=<path> opens that file in the viewer.
        if let Ok(file) = std::env::var("OSIER_START_FILE")
            && this.active.is_some()
        {
            this.tab = Tab::Files;
            this.select_work_file(file, None, cx);
        }
        this.update_title(window);
        this.start_watcher_loop(window, cx);
        // Dev aid for screenshots: OSIER_OPEN_SETTINGS=1 opens the settings sheet at startup.
        // The sheet needs gpui-kit's root, which exists only after this view is installed.
        if std::env::var_os("OSIER_OPEN_SETTINGS").is_some_and(|v| v == "1") {
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(300))
                    .await;
                let _ = this.update_in(cx, |this, window, cx| this.open_settings(window, cx));
            })
            .detach();
        }
        this.load_gh_accounts(cx);
        this.rescan_projects(cx);
        this
    }

    /// Scan the configured directories for repositories, off the UI thread.
    pub(crate) fn rescan_projects(&mut self, cx: &mut Context<Self>) {
        let dirs = self.settings.projects.scan_dirs.clone();
        if dirs.is_empty() {
            self.scanned.clear();
            return;
        }
        let depth = self.settings.projects.scan_depth.unwrap_or(1);
        cx.spawn(async move |this, cx| {
            let found = cx
                .background_spawn(async move { osier_config::scan::scan(&dirs, depth) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.scanned = found;
                cx.notify();
            });
        })
        .detach();
    }

    /// Lower-case filter text from the sidebar box.
    pub(crate) fn sidebar_filter_text(&self, cx: &App) -> String {
        self.sidebar_filter.read(cx).value().trim().to_lowercase()
    }

    /// Does `path` match the sidebar filter (name or any path segment)?
    pub(crate) fn matches_filter(path: &Path, filter: &str) -> bool {
        filter.is_empty() || path.to_string_lossy().to_lowercase().contains(filter)
    }

    /// Recents and scanned repositories that are not open, filtered, with their origin.
    /// Pinned repositories that are not open, filtered like the rest of the sidebar.
    pub(crate) fn pinned_entries(&self, cx: &App) -> Vec<PathBuf> {
        let filter = self.sidebar_filter_text(cx);
        self.state
            .pinned
            .iter()
            .filter(|p| !self.projects.iter().any(|o| &o.path == *p))
            .filter(|p| Self::matches_filter(p, &filter))
            .cloned()
            .collect()
    }

    pub(crate) fn recent_entries(&self, cx: &App) -> Vec<(PathBuf, Option<PathBuf>)> {
        let filter = self.sidebar_filter_text(cx);
        let mut out: Vec<(PathBuf, Option<PathBuf>)> = Vec::new();
        for p in self.state.recent_not_open() {
            if self.state.is_pinned(p) {
                continue;
            }
            if Self::matches_filter(p, &filter) {
                let provider =
                    osier_config::scan::provider(&self.settings.projects.scan_dirs, p).cloned();
                out.push((p.clone(), provider));
            }
        }
        for p in &self.scanned {
            if !self.state.open_projects.contains(p)
                && !self.state.is_pinned(p)
                && !out.iter().any(|(q, _)| q == p)
                && Self::matches_filter(p, &filter)
            {
                let provider =
                    osier_config::scan::provider(&self.settings.projects.scan_dirs, p).cloned();
                out.push((p.clone(), provider));
            }
        }
        out
    }

    /// Enter in the filter box: activate the first open match, else open the first recent.
    pub(crate) fn open_first_filtered(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let filter = self.sidebar_filter_text(cx);
        if filter.is_empty() {
            return;
        }
        if let Some(ix) = self
            .projects
            .iter()
            .position(|p| Self::matches_filter(&p.path, &filter))
        {
            self.activate(ix, window, cx);
            return;
        }
        if let Some((path, _)) = self.recent_entries(cx).into_iter().next() {
            self.open_project_notify(&path, window, cx);
        }
    }

    /// Move an open project up or down in the sidebar.
    pub(crate) fn move_project(&mut self, ix: usize, delta: isize, cx: &mut Context<Self>) {
        let len = self.projects.len();
        let Some(new_ix) = ix.checked_add_signed(delta).filter(|n| *n < len) else {
            return;
        };
        self.projects.swap(ix, new_ix);
        if let Some(a) = self.active {
            self.active = Some(if a == ix {
                new_ix
            } else if a == new_ix {
                ix
            } else {
                a
            });
        }
        self.state.open_projects = self.projects.iter().map(|p| p.path.clone()).collect();
        self.save_state();
        cx.notify();
    }

    // ----- appearance -----

    pub(crate) fn apply_appearance(&mut self, cx: &mut App) {
        use crate::themes::{DUX_DARK, DUX_LIGHT, ThemeStore};
        let a = &self.settings.appearance;
        let store = ThemeStore::global(cx);
        let dux = a.dux_shell_sync && store.dux_available();
        let dark = match a.mode {
            ThemeMode::Dark => true,
            ThemeMode::Light => false,
            ThemeMode::System => match store.dux_mode() {
                Some(mode) if dux => mode == osier_theme::Mode::Dark,
                _ => matches!(
                    cx.window_appearance(),
                    WindowAppearance::Dark | WindowAppearance::VibrantDark
                ),
            },
        };
        let name = match (dux, dark) {
            (true, true) => DUX_DARK,
            (true, false) => DUX_LIGHT,
            (false, true) => a.dark_theme.as_str(),
            (false, false) => a.light_theme.as_str(),
        };
        if !crate::themes::apply(name, a, cx) {
            tracing::warn!(name, "theme not found, using the bundled default");
            let fallback = if dark {
                "Catppuccin Mocha"
            } else {
                "Catppuccin Latte"
            };
            crate::themes::apply(fallback, a, cx);
        }
    }

    pub(crate) fn set_theme_mode(&mut self, mode: ThemeMode, cx: &mut Context<Self>) {
        self.settings.appearance.mode = mode;
        self.apply_appearance(cx);
        self.save_settings();
        cx.notify();
    }

    /// Pick a theme for its own mode (dark themes replace the dark choice, light the light).
    /// Picking an explicit theme turns Dux Shell sync off.
    pub(crate) fn set_theme(&mut self, name: &str, cx: &mut Context<Self>) {
        let Some(config) = crate::themes::ThemeStore::global(cx).get(name) else {
            return;
        };
        // Picking a theme of the other mode switches to that mode explicitly (System mode
        // would otherwise keep showing the old one).
        let showing_dark = cx.theme().mode.is_dark();
        let a = &mut self.settings.appearance;
        if config.mode.is_dark() {
            a.dark_theme = name.to_string();
            if !showing_dark {
                a.mode = ThemeMode::Dark;
            }
        } else {
            a.light_theme = name.to_string();
            if showing_dark {
                a.mode = ThemeMode::Light;
            }
        }
        a.dux_shell_sync = false;
        self.apply_appearance(cx);
        self.save_settings();
        cx.notify();
    }

    pub(crate) fn set_dux_sync(&mut self, on: bool, cx: &mut Context<Self>) {
        self.settings.appearance.dux_shell_sync = on;
        self.apply_appearance(cx);
        self.save_settings();
        cx.notify();
    }

    /// Reload themes when a theme file changed on disk. Returns true when something changed.
    fn poll_themes(&mut self, cx: &mut Context<Self>) -> bool {
        if !crate::themes::ThemeStore::global(cx).take_dirty() {
            return false;
        }
        crate::themes::ThemeStore::global_mut(cx).reload();
        self.apply_appearance(cx);
        true
    }

    pub(crate) fn open_themes_dir(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dir = self.paths.themes_dir();
        if let Err(error) = std::fs::create_dir_all(&dir) {
            tracing::warn!(%error, "could not create the themes directory");
        }
        self.open_externally(&dir, window, cx);
    }

    pub(crate) fn toggle_theme(&mut self, cx: &mut Context<Self>) {
        let dark = cx.theme().mode.is_dark();
        self.settings.appearance.mode = if dark {
            ThemeMode::Light
        } else {
            ThemeMode::Dark
        };
        self.apply_appearance(cx);
        self.save_settings();
        cx.notify();
    }

    pub(crate) fn save_settings(&self) {
        if let Err(error) = self.settings.save(&self.paths) {
            tracing::warn!(%error, "could not save settings");
        }
    }

    pub(crate) fn save_state(&mut self) {
        // Session-only account choices stay in memory.
        let mut snapshot = self.state.clone();
        for key in &self.session_only_accounts {
            snapshot.accounts.remove(key);
        }
        self.state.layout.sidebar_visible = self.sidebar_visible;
        snapshot.layout.sidebar_visible = self.sidebar_visible;
        if !self.command_log_from_env {
            self.state.layout.command_log_visible = self.command_log_visible;
            snapshot.layout.command_log_visible = self.command_log_visible;
        }
        if let Err(error) = snapshot.save(&self.paths) {
            tracing::warn!(%error, "could not save state");
        }
    }

    // ----- projects -----

    pub(crate) fn active_project(&self) -> Option<&Project> {
        self.active.and_then(|ix| self.projects.get(ix))
    }

    pub(crate) fn active_project_mut(&mut self) -> Option<&mut Project> {
        match self.active {
            Some(ix) => self.projects.get_mut(ix),
            None => None,
        }
    }

    /// Open (or re-activate) a repository at `path`. Returns its index. On failure the path
    /// is dropped from recents and the error is returned for the caller to show.
    pub(crate) fn open_project(
        &mut self,
        path: &Path,
        make_active: bool,
        cx: &mut Context<Self>,
    ) -> Result<usize, String> {
        if let Some(ix) = self.projects.iter().position(|p| p.path == path) {
            if make_active {
                self.active = Some(ix);
                self.state.open(&self.projects[ix].path);
                self.save_state();
            }
            return Ok(ix);
        }
        match Project::open(self.runner.clone(), path, self.settings.git.log_page_size) {
            Ok(project) => {
                let workdir = project.path.clone();
                self.projects.push(project);
                let ix = self.projects.len() - 1;
                self.state.open(&workdir);
                self.save_state();
                self.refresh_project(ix, cx);
                if make_active {
                    self.active = Some(ix);
                }
                Ok(ix)
            }
            Err(error) => {
                tracing::warn!(%error, path = %path.display(), "cannot open repository");
                self.state.forget_recent(path);
                self.state.close(path);
                self.save_state();
                cx.notify();
                Err(format!("{}: {error}", crate::format::tilde(path)))
            }
        }
    }

    /// Open from a user action: failures become a warning toast.
    pub(crate) fn open_project_notify(
        &mut self,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use gpui_kit::component::WindowExt as _;
        use gpui_kit::component::notification::Notification;
        match self.open_project(path, true, cx) {
            Ok(ix) => self.activate(ix, window, cx),
            Err(error) => window.push_notification(
                Notification::warning("Could not open repository")
                    .message(format!("{error}. It was removed from the recent list.")),
                cx,
            ),
        }
    }

    /// Forget a recent entry. Entries provided by a scan directory cannot be removed here.
    pub(crate) fn forget_recent(
        &mut self,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use gpui_kit::component::WindowExt as _;
        if let Some(dir) = osier_config::scan::provider(&self.settings.projects.scan_dirs, path) {
            let dir = crate::format::tilde(dir);
            let name = crate::format::tilde(path);
            self.confirm(
                "Provided by a scan directory",
                format!(
                    "{name} appears because {dir} is listed under projects.scan_dirs in config.toml. It cannot be removed on its own. To stop listing it, remove {dir} from the scan directories (Settings > Projects, or edit config.toml)."
                ),
                "OK",
                false,
                |_, _, _| {},
                window,
                cx,
            );
            return;
        }
        self.state.forget_recent(path);
        self.save_state();
        window.push_notification(
            format!("Removed {} from recents", crate::format::tilde(path)),
            cx,
        );
        cx.notify();
    }

    pub(crate) fn activate(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.projects.len() {
            return;
        }
        self.active = Some(ix);
        let path = self.projects[ix].path.clone();
        self.state.open(&path);
        self.save_state();
        self.reload_selection(cx);
        self.update_title(window);
        cx.notify();
    }

    pub(crate) fn close_project(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.projects.len() {
            return;
        }
        let project = self.projects.remove(ix);
        self.state.close(&project.path);
        self.active = match self.active {
            Some(a) if a == ix => {
                if self.projects.is_empty() {
                    None
                } else {
                    Some(ix.min(self.projects.len() - 1))
                }
            }
            Some(a) if a > ix => Some(a - 1),
            other => other,
        };
        if let Some(a) = self.active {
            let p = self.projects[a].path.clone();
            self.state.active_project = Some(p);
        }
        self.save_state();
        self.reload_selection(cx);
        self.update_title(window);
        cx.notify();
    }

    fn update_title(&self, window: &mut Window) {
        let title = match self.active_project() {
            Some(p) => format!("{} - Osier", p.name),
            None => "Osier".to_string(),
        };
        window.set_window_title(&title);
    }

    pub(crate) fn prompt_open_repository(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open repository".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                this.open_project_notify(&path, window, cx);
            });
        })
        .detach();
    }

    // ----- refresh -----

    pub(crate) fn refresh_active(&mut self, cx: &mut Context<Self>) {
        if let Some(ix) = self.active {
            self.refresh_project(ix, cx);
        }
    }

    pub(crate) fn refresh_project(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(project) = self.projects.get_mut(ix) else {
            return;
        };
        project.revision += 1;
        let revision = project.revision;
        project.loading = true;
        let repo = project.repo.clone();
        let path = project.path.clone();
        let page = project.log_page_size;
        let log_path = project.log_path.clone();
        project.task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { load_snapshot(&repo, page, log_path.as_deref()) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.install_snapshot(&path, revision, result, cx)
            });
        });
        cx.notify();
    }

    fn install_snapshot(
        &mut self,
        path: &Path,
        revision: u64,
        result: Result<crate::project::Snapshot, String>,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self.projects.iter().position(|p| p.path == path) else {
            return;
        };
        let is_active = self.active == Some(ix);
        let project = &mut self.projects[ix];
        if project.revision != revision {
            return;
        }
        project.loading = false;
        project.mark_refreshed();
        let mut reload = false;
        match result {
            Ok(snapshot) => {
                // Reload the diff only when the working tree or HEAD moved. Reloading resets the
                // diff view's scroll position, so avoid it for no-op refreshes.
                let status_changed = project
                    .snapshot
                    .as_ref()
                    .is_none_or(|old| old.status != snapshot.status);
                reload = match &project.selection {
                    Selection::None | Selection::Commit { .. } => false,
                    Selection::File { .. } | Selection::Compare { .. } => status_changed,
                    Selection::WorkFile { rev: None, .. } => status_changed,
                    Selection::WorkFile { .. } | Selection::Patch { .. } => false,
                };
                // Keep the selected commit by hash across refreshes.
                if let Selection::Commit { hash } = &project.selection {
                    project.selected_commit = snapshot.commits.iter().position(|c| &c.hash == hash);
                }
                // Drop a file selection that no longer exists.
                if let Selection::File { path, .. } = &project.selection
                    && !snapshot.status.entries.iter().any(|e| &e.path == path)
                {
                    project.selection = Selection::None;
                }
                // First load: select the first changed file so the diff pane is not empty.
                if project.selection == Selection::None
                    && let Some((path, area)) = first_change(&snapshot.status)
                {
                    project.selection = Selection::File { path, area };
                    reload = true;
                }
                project.snapshot = Some(snapshot);
                project.error = None;
            }
            Err(error) => project.error = Some(error),
        }
        if is_active && reload {
            self.reload_selection(cx);
        }
        cx.notify();
    }

    fn start_watcher_loop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(400))
                    .await;
                let alive = this.update_in(cx, |this, window, cx| {
                    let due: Vec<usize> = this
                        .projects
                        .iter_mut()
                        .enumerate()
                        .filter_map(|(ix, p)| p.poll_watcher().then_some(ix))
                        .collect();
                    for ix in due {
                        this.refresh_project(ix, cx);
                    }
                    if this.state_dirty {
                        this.state_dirty = false;
                        this.save_state();
                    }
                    if this.poll_remote_job() || this.poll_themes(cx) {
                        cx.notify();
                    }
                    if this.poll_config(window, cx) {
                        cx.notify();
                    }
                    // The command log panel shows new records as they land.
                    let version = this.sink.version();
                    if this.command_log_visible && version != this.log_version_seen {
                        this.log_version_seen = version;
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    // ----- selection and diff -----

    pub(crate) fn select_file(&mut self, path: String, area: Area, cx: &mut Context<Self>) {
        self.diff_force_large = false;
        self.editor = None;
        if let Some(p) = self.active_project_mut() {
            p.selection = Selection::File { path, area };
            p.selected_commit = None;
        }
        self.reload_selection(cx);
        cx.notify();
    }

    pub(crate) fn select_work_file(
        &mut self,
        path: String,
        rev: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.diff_force_large = false;
        self.editor = None;
        if let Some(p) = self.active_project_mut() {
            p.selection = Selection::WorkFile { path, rev };
            p.selected_commit = None;
        }
        self.reload_selection(cx);
        cx.notify();
    }

    pub(crate) fn select_compare(
        &mut self,
        from: String,
        to: String,
        since_merge_base: bool,
        cx: &mut Context<Self>,
    ) {
        self.diff_force_large = false;
        self.editor = None;
        if let Some(p) = self.active_project_mut() {
            p.selection = Selection::Compare {
                from,
                to,
                since_merge_base,
            };
            p.selected_commit = None;
        }
        self.reload_selection(cx);
        cx.notify();
    }

    pub(crate) fn select_commit(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.diff_force_large = false;
        self.editor = None;
        let Some(p) = self.active_project_mut() else {
            return;
        };
        let Some(commit) = p.commit_at(ix) else {
            return;
        };
        p.selection = Selection::Commit {
            hash: commit.hash.clone(),
        };
        p.selected_commit = Some(ix);
        p.compare_commit = None;
        self.log_scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
        self.reload_selection(cx);
        cx.notify();
    }

    /// Compare the selected commit with the one at `ix` (shift/ctrl-click or the row menu).
    /// The older commit (lower in the log) is the base. Without a selection, select `ix`.
    pub(crate) fn compare_with_selected(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(p) = self.active_project_mut() else {
            return;
        };
        let Some(anchor) = p.selected_commit.filter(|a| *a != ix) else {
            self.select_commit(ix, cx);
            return;
        };
        let (Some(a), Some(b)) = (p.commit_at(anchor), p.commit_at(ix)) else {
            return;
        };
        let (older, newer) = if anchor > ix { (a, b) } else { (b, a) };
        let (from, to) = (older.hash.clone(), newer.hash.clone());
        p.compare_commit = Some(ix);
        self.diff_force_large = false;
        self.editor = None;
        if let Some(p) = self.active_project_mut() {
            p.selection = Selection::Compare {
                from,
                to,
                since_merge_base: false,
            };
        }
        self.reload_selection(cx);
        cx.notify();
    }

    pub(crate) fn compare_with_hash(&mut self, hash: String, cx: &mut Context<Self>) {
        let ix = self
            .active_project()
            .and_then(|p| p.snapshot.as_ref())
            .and_then(|s| s.commits.iter().position(|c| c.hash == hash));
        if let Some(ix) = ix {
            self.compare_with_selected(ix, cx);
        }
    }

    /// Pin or unpin a repository in the sidebar's Pinned group.
    pub(crate) fn toggle_pin(&mut self, path: &Path, cx: &mut Context<Self>) {
        self.state.toggle_pin(path);
        self.save_state();
        cx.notify();
    }

    /// Drag and drop in the Open list: move the project at `from` to position `to`.
    pub(crate) fn move_project_to(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        let len = self.projects.len();
        if from >= len || to >= len || from == to {
            return;
        }
        let active_path = self.active.map(|a| self.projects[a].path.clone());
        let project = self.projects.remove(from);
        self.projects.insert(to, project);
        self.active = active_path.and_then(|p| self.projects.iter().position(|x| x.path == p));
        self.state.open_projects = self.projects.iter().map(|p| p.path.clone()).collect();
        self.save_state();
        cx.notify();
    }

    /// Load the diff for the active project's selection into the diff pane.
    pub(crate) fn reload_selection(&mut self, cx: &mut Context<Self>) {
        self.blame = None;
        self.diff_revision += 1;
        let revision = self.diff_revision;
        let context = self.settings.diff.context_lines;
        let Some(project) = self.active_project() else {
            self.diff_model = std::rc::Rc::new(Vec::new());
            self.set_diff_files(Vec::new(), cx);
            return;
        };
        let repo = project.repo.clone();
        let selection = project.selection.clone();
        if selection == Selection::None {
            self.diff_model = std::rc::Rc::new(Vec::new());
            self.set_diff_files(Vec::new(), cx);
            return;
        }
        if let Selection::WorkFile { path, rev } = &selection {
            self.diff_model = std::rc::Rc::new(Vec::new());
            self.set_diff_files(Vec::new(), cx);
            self.load_viewer(path.clone(), rev.clone(), cx);
            return;
        }
        self.viewer = None;
        self.diff_loading = true;
        self.diff_error = None;
        self.diff_too_large = None;
        let limit = if self.diff_force_large {
            u64::MAX
        } else {
            self.settings.diff.large_file_bytes
        };
        self.diff_task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { load_patch(&repo, &selection, context, limit) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.diff_revision != revision {
                    return;
                }
                this.diff_loading = false;
                match result {
                    Ok((files, model)) => {
                        this.diff_model = std::rc::Rc::new(model);
                        this.set_diff_files(files, cx);
                    }
                    Err(LoadError::TooLarge(size)) => {
                        this.diff_too_large = Some(size);
                        this.diff_model = std::rc::Rc::new(Vec::new());
                        this.set_diff_files(Vec::new(), cx);
                    }
                    Err(LoadError::Git(error)) => {
                        this.diff_error = Some(error);
                        this.diff_model = std::rc::Rc::new(Vec::new());
                        this.set_diff_files(Vec::new(), cx);
                    }
                }
                cx.notify();
            });
        });
        cx.notify();
    }

    fn set_diff_files(&mut self, files: Vec<DiffFile>, cx: &mut Context<Self>) {
        self.diff_files = files.len();
        self.diff.update(cx, |state, cx| state.set_files(files, cx));
    }

    /// "Load anyway" for a file over the size limit.
    pub(crate) fn force_large_diff(&mut self, cx: &mut Context<Self>) {
        self.diff_force_large = true;
        self.reload_selection(cx);
    }

    /// Move the selection in the visible list (Changes or Log) by `delta`.
    pub(crate) fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(project) = self.active_project() else {
            return;
        };
        let Some(snapshot) = &project.snapshot else {
            return;
        };
        match self.tab {
            Tab::Changes => {
                let items = crate::project::ordered_changes(&snapshot.status);
                if items.is_empty() {
                    return;
                }
                let current = match &project.selection {
                    Selection::File { path, area } => {
                        items.iter().position(|(p, a)| p == path && a == area)
                    }
                    _ => None,
                };
                let next = match current {
                    Some(i) => i.saturating_add_signed(delta).min(items.len() - 1),
                    None => 0,
                };
                let (path, area) = items[next].clone();
                self.select_file(path, area, cx);
            }
            Tab::Log => {
                if snapshot.commits.is_empty() {
                    return;
                }
                let next = match project.selected_commit {
                    Some(i) => i
                        .saturating_add_signed(delta)
                        .min(snapshot.commits.len() - 1),
                    None => 0,
                };
                self.select_commit(next, cx);
            }
            Tab::Branches | Tab::Stashes | Tab::Files | Tab::Patches => {}
        }
    }

    pub(crate) fn on_pane_resize(&mut self, sizes: &[Pixels]) {
        if let Some(first) = sizes.first() {
            self.state.layout.center_width = Some(f32::from(*first));
            self.state_dirty = true;
        }
    }

    pub(crate) fn toggle_soft_wrap(&mut self, cx: &mut Context<Self>) {
        self.settings.diff.soft_wrap = !self.settings.diff.soft_wrap;
        self.save_settings();
        cx.notify();
    }

    pub(crate) fn set_diff_mode(&mut self, mode: DiffMode, cx: &mut Context<Self>) {
        self.diff.update(cx, |state, cx| state.set_mode(mode, cx));
        self.settings.diff.mode = match mode {
            DiffMode::Split => osier_config::settings::DiffMode::Split,
            _ => osier_config::settings::DiffMode::Unified,
        };
        self.save_settings();
        cx.notify();
    }

    // ----- layout -----

    pub(crate) fn set_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.tab = tab;
        cx.notify();
    }

    pub(crate) fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_visible = !self.sidebar_visible;
        self.save_state();
        cx.notify();
    }

    pub(crate) fn toggle_command_log(&mut self, cx: &mut Context<Self>) {
        self.command_log_visible = !self.command_log_visible;
        self.save_state();
        cx.notify();
    }
}

/// Conflicts first, then staged, unstaged, untracked.
fn first_change(status: &osier_git::status::RepoStatus) -> Option<(String, Area)> {
    status
        .conflicted()
        .map(|e| (e.path.clone(), Area::Conflicted))
        .chain(status.staged().map(|e| (e.path.clone(), Area::Staged)))
        .chain(status.unstaged().map(|e| (e.path.clone(), Area::Unstaged)))
        .chain(
            status
                .untracked()
                .map(|e| (e.path.clone(), Area::Untracked)),
        )
        .next()
}

pub(crate) enum LoadError {
    TooLarge(u64),
    Git(String),
}

impl From<osier_git::GitError> for LoadError {
    fn from(e: osier_git::GitError) -> Self {
        LoadError::Git(e.to_string())
    }
}

/// Blocking: raw patch for the selection, parsed by gpui-kit's diff parser.
fn load_patch(
    repo: &Repository,
    selection: &Selection,
    context: u32,
    size_limit: u64,
) -> Result<(Vec<DiffFile>, Vec<osier_git::diff::DiffFile>), LoadError> {
    if let Selection::File { path, .. } = selection
        && let Ok(meta) = std::fs::metadata(repo.workdir().join(path))
        && meta.len() > size_limit
    {
        return Err(LoadError::TooLarge(meta.len()));
    }
    if let Selection::File {
        path,
        area: Area::Conflicted,
    } = selection
    {
        // The working file with its markers: the Diff component renders each conflict with
        // Current / Incoming / Both controls.
        let text = std::fs::read_to_string(repo.workdir().join(path))
            .map_err(|e| LoadError::Git(format!("{path}: {e}")))?;
        let file = DiffFile::parse_conflicts(path.clone(), &text)
            .map_err(|e| LoadError::Git(e.to_string()))?;
        return Ok((vec![file], Vec::new()));
    }
    let patch = match selection {
        Selection::None => return Ok((Vec::new(), Vec::new())),
        Selection::File { path, area } => match area {
            Area::Untracked => osier_git::diff::untracked_patch_text(repo, path, context)?,
            Area::Staged => {
                let mut o = DiffOptions::new(DiffTarget::Staged).path(path.clone());
                o.context_lines = context;
                osier_git::diff::patch_text(repo, &o)?
            }
            Area::Unstaged | Area::Conflicted => {
                let mut o = DiffOptions::new(DiffTarget::Unstaged).path(path.clone());
                o.context_lines = context;
                osier_git::diff::patch_text(repo, &o)?
            }
        },
        Selection::Commit { hash } => {
            let mut o = DiffOptions::new(DiffTarget::Commit(hash.clone()));
            o.context_lines = context;
            osier_git::diff::patch_text(repo, &o)?
        }
        Selection::WorkFile { .. } => return Ok((Vec::new(), Vec::new())),
        Selection::Patch { file, .. } => osier_git::ops::patches::normalize(
            &std::fs::read_to_string(file)
                .map_err(|e| LoadError::Git(format!("{}: {e}", file.display())))?,
        ),
        Selection::Compare {
            from,
            to,
            since_merge_base,
        } => {
            let target = if *since_merge_base {
                DiffTarget::SinceMergeBase {
                    from: from.clone(),
                    to: to.clone(),
                }
            } else {
                DiffTarget::Range {
                    from: from.clone(),
                    to: to.clone(),
                }
            };
            let mut o = DiffOptions::new(target);
            o.context_lines = context;
            osier_git::diff::patch_text(repo, &o)?
        }
    };
    if patch.trim().is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let files = DiffFile::parse(&patch).map_err(|e| LoadError::Git(e.to_string()))?;
    // Our parser is stricter; a failure here only disables hunk actions.
    let model = osier_git::diff::parse(&patch).unwrap_or_else(|e| {
        tracing::debug!(error = %e, "hunk model unavailable for this patch");
        Vec::new()
    });
    Ok((files, model))
}

impl Render for Osier {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let has_project = self.active.is_some();
        v_flex()
            .track_focus(&self.focus_handle)
            .key_context("Osier")
            .on_action(|_: &Quit, _, cx| cx.quit())
            .on_action(cx.listener(|this, _: &Refresh, _, cx| this.refresh_active(cx)))
            .on_action(cx.listener(|this, _: &OpenRepository, window, cx| {
                this.prompt_open_repository(window, cx)
            }))
            .on_action(cx.listener(|this, _: &CloseProject, window, cx| {
                if let Some(ix) = this.active {
                    this.close_project(ix, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| this.toggle_sidebar(cx)))
            .on_action(cx.listener(|this, _: &ToggleCommandLog, _, cx| this.toggle_command_log(cx)))
            .on_action(cx.listener(|this, _: &ToggleTheme, _, cx| this.toggle_theme(cx)))
            .on_action(
                cx.listener(|this, _: &OpenSettings, window, cx| this.open_settings(window, cx)),
            )
            .on_action(cx.listener(|this, _: &OpenCommandPalette, window, cx| {
                this.open_command_palette(window, cx)
            }))
            .on_action(cx.listener(|this, _: &TabChanges, _, cx| this.set_tab(Tab::Changes, cx)))
            .on_action(cx.listener(|this, _: &TabLog, _, cx| this.set_tab(Tab::Log, cx)))
            .on_action(cx.listener(|this, _: &TabBranches, _, cx| this.set_tab(Tab::Branches, cx)))
            .on_action(cx.listener(|this, _: &TabStashes, _, cx| this.set_tab(Tab::Stashes, cx)))
            .on_action(cx.listener(|this, _: &TabFiles, _, cx| this.set_tab(Tab::Files, cx)))
            .on_action(cx.listener(|this, _: &TabPatches, _, cx| this.set_tab(Tab::Patches, cx)))
            .on_action(cx.listener(|this, _: &CommitStaged, window, cx| this.commit(window, cx)))
            .on_action(cx.listener(|this, _: &Fetch, window, cx| {
                this.start_remote(crate::remote_ops::RemoteOp::fetch_all(), window, cx)
            }))
            .on_action(cx.listener(|this, _: &Pull, window, cx| {
                this.start_remote(crate::remote_ops::RemoteOp::pull_default(), window, cx)
            }))
            .on_action(cx.listener(|this, _: &Push, window, cx| {
                this.start_remote(crate::remote_ops::RemoteOp::push_default(), window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleStageSelected, window, cx| {
                this.toggle_stage_selected(window, cx)
            }))
            .on_action(cx.listener(|this, _: &DiscardSelected, window, cx| {
                this.discard_selected(window, cx)
            }))
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.move_selection(1, cx)))
            .on_action(cx.listener(|this, _: &SelectPrev, _, cx| this.move_selection(-1, cx)))
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.render_title_bar(cx))
            .child(
                div().flex_1().min_h_0().child(
                    h_flex()
                        .size_full()
                        .when(self.sidebar_visible, |this| {
                            this.child(self.render_sidebar(cx))
                        })
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .size_full()
                                .child(
                                    div().flex_1().min_h_0().child(
                                        h_resizable("panes")
                                            .on_resize(cx.listener(
                                                |this, state: &Entity<ResizableState>, _, cx| {
                                                    let sizes = state.read(cx).sizes().clone();
                                                    this.on_pane_resize(&sizes);
                                                },
                                            ))
                                            .child(
                                                resizable_panel()
                                                    .size(px(self
                                                        .state
                                                        .layout
                                                        .center_width
                                                        .unwrap_or(440.)))
                                                    .size_range(px(260.)..px(1400.))
                                                    .child(self.render_center(window, cx)),
                                            )
                                            .child(
                                                resizable_panel()
                                                    .size_range(px(240.)..px(4000.))
                                                    .child(self.render_diff_pane(window, cx)),
                                            ),
                                    ),
                                )
                                .when(self.command_log_visible, |this| {
                                    this.child(self.render_command_log(cx))
                                }),
                        ),
                ),
            )
            .child(self.render_status_bar(has_project, cx))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    // `gpui-pre` publishes its library as `gpui`, the same crate gpui-kit re-exports, so the
    // `#[gpui::test]` macro's `gpui::` paths resolve. No glob imports here: `gpui_kit::*`
    // also exports a `gpui` module, which would make the name ambiguous.
    use gpui::TestAppContext;
    use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, WindowOptions};
    use osier_config::{Paths, Settings, State};
    use osier_git::GitRunner;
    use osier_git::testing::TempRepo;

    use super::{Osier, init};
    use crate::command_log::UiCommandSink;
    use crate::project::{Area, Selection};

    fn isolated(sink: Arc<UiCommandSink>) -> GitRunner {
        GitRunner::new(sink)
            .with_env("GIT_CONFIG_GLOBAL", "/dev/null")
            .with_env("GIT_CONFIG_NOSYSTEM", "1")
            .with_env("HOME", std::env::temp_dir())
    }

    struct Harness {
        _dirs: tempfile::TempDir,
        window: AnyWindowHandle,
        view: Entity<Osier>,
    }

    impl Harness {
        fn open(cx: &mut TestAppContext, repo: &TempRepo) -> Self {
            cx.update(gpui_kit::init);
            cx.update(init);
            let dirs = tempfile::tempdir().expect("tempdir");
            let paths = Paths::under(dirs.path());
            let sink = Arc::new(UiCommandSink::default());
            let runner = isolated(sink.clone());
            let open = Some(repo.path().to_path_buf());
            let (handle, view) = cx.update(|cx| {
                gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                    cx.new(|cx| {
                        Osier::new(
                            paths,
                            Settings::default(),
                            State::default(),
                            runner,
                            sink,
                            open,
                            None,
                            window,
                            cx,
                        )
                    })
                })
                .expect("window")
            });
            cx.run_until_parked();
            Self {
                _dirs: dirs,
                window: handle,
                view,
            }
        }

        /// Run `f` with the window, like a click handler would.
        fn act(
            &self,
            cx: &mut TestAppContext,
            f: impl FnOnce(&mut Osier, &mut gpui::Window, &mut gpui::Context<Osier>),
        ) {
            let view = self.view.clone();
            cx.update_window(self.window, |_, window, cx| {
                view.update(cx, |osier, cx| f(osier, window, cx));
            })
            .expect("window alive");
            cx.run_until_parked();
        }

        fn read<R>(&self, cx: &mut TestAppContext, f: impl FnOnce(&Osier, &gpui::App) -> R) -> R {
            cx.update(|cx| f(self.view.read(cx), cx))
        }
    }

    #[gpui::test]
    fn shelve_and_unshelve_through_the_patch_clipboard(cx: &mut TestAppContext) {
        use osier_git::ops::patches::ApplyOptions;
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "one\n", "first");
        repo.write("a.txt", "two\n");
        repo.write("new.txt", "fresh\n");
        let h = Harness::open(cx, &repo);

        h.act(cx, |osier, window, cx| {
            osier.shelve_named("my shelf".into(), Vec::new(), window, cx)
        });
        cx.run_until_parked();
        let (id, text) = h.read(cx, |osier, _| {
            assert_eq!(osier.patches.len(), 1, "one stored patch");
            let p = &osier.patches[0];
            assert_eq!(p.meta.name, "my shelf");
            assert_eq!(p.meta.files, ["a.txt", "new.txt"]);
            assert_eq!(osier.tab, super::Tab::Patches);
            (p.id.clone(), p.text().expect("patch text"))
        });
        assert_eq!(repo.read("a.txt"), "one\n", "tracked change discarded");
        assert!(!repo.path().join("new.txt").exists(), "untracked file gone");

        // Unshelve: apply the stored text and delete the entry afterwards.
        h.act(cx, |osier, window, cx| {
            let id = id.clone();
            osier.run_apply(
                text.clone(),
                ApplyOptions::default(),
                "\"my shelf\"".into(),
                Some(std::rc::Rc::new(
                    move |this: &mut Osier,
                          window: &mut gpui::Window,
                          cx: &mut gpui::Context<Osier>| {
                        this.delete_stored(id.clone(), window, cx)
                    },
                )),
                window,
                cx,
            );
        });
        cx.run_until_parked();
        assert_eq!(repo.read("a.txt"), "two\n");
        assert_eq!(repo.read("new.txt"), "fresh\n");
        h.read(cx, |osier, _| {
            assert!(osier.patches.is_empty(), "unshelved patch deleted");
            assert_eq!(osier.tab, super::Tab::Changes);
        });
    }

    #[gpui::test]
    fn settings_reload_from_disk_and_account_overrides(cx: &mut TestAppContext) {
        use gpui_kit::component::ActiveTheme as _;
        use osier_config::settings::ThemeMode;
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "one\n", "first");
        let h = Harness::open(cx, &repo);

        // An external edit to config.toml is picked up and applied.
        let file = h.read(cx, |osier, _| osier.paths.settings_file());
        std::fs::create_dir_all(file.parent().expect("dir")).expect("mkdir");
        std::fs::write(
            &file,
            "[appearance]\nmode = \"light\"\n[diff]\ncontext_lines = 7\n",
        )
        .expect("write config");
        h.act(cx, |osier, window, cx| {
            assert!(osier.reload_settings_from_disk(window, cx));
            assert!(
                !osier.reload_settings_from_disk(window, cx),
                "unchanged second time"
            );
        });
        h.read(cx, |osier, cx| {
            assert_eq!(osier.settings.appearance.mode, ThemeMode::Light);
            assert_eq!(osier.settings.diff.context_lines, 7);
            assert!(!cx.theme().mode.is_dark());
        });
        // A broken file keeps the current settings.
        std::fs::write(&file, "[appearance\nmode = ").expect("write config");
        h.act(cx, |osier, window, cx| {
            assert!(!osier.reload_settings_from_disk(window, cx));
        });
        h.read(cx, |osier, _| {
            assert_eq!(osier.settings.diff.context_lines, 7)
        });

        // Per-project account overrides can be reset from the panel.
        h.act(cx, |osier, _, _| {
            osier.set_project_account("/r/one", "alice", true);
            osier.set_project_account("/r/two", "bob", false);
        });
        h.read(cx, |osier, _| {
            assert_eq!(osier.state.accounts.len(), 2);
            assert!(osier.session_only_accounts.contains("/r/two"));
        });
        h.act(cx, |osier, _, cx| {
            osier.clear_account_override("/r/two".into(), cx)
        });
        h.read(cx, |osier, _| {
            assert_eq!(osier.state.accounts.len(), 1);
            assert!(osier.session_only_accounts.is_empty());
        });
        h.act(cx, |osier, _, cx| osier.clear_all_account_overrides(cx));
        h.read(cx, |osier, _| assert!(osier.state.accounts.is_empty()));
    }

    #[gpui::test]
    fn blame_and_file_history(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "one\n", "first");
        repo.commit_file("b.txt", "bee\n", "second");
        repo.commit_file("a.txt", "one\ntwo\n", "third");
        let h = Harness::open(cx, &repo);

        h.act(cx, |osier, _, cx| {
            osier.show_blame("a.txt".into(), None, cx)
        });
        cx.run_until_parked();
        h.read(cx, |osier, _| {
            let view = osier.blame.as_ref().expect("blame view");
            let lines = view.lines.as_ref().expect("blame loaded");
            assert_eq!(lines.len(), 2);
            assert_eq!(lines[0].summary, "first");
            assert_eq!(lines[1].summary, "third");
            assert_ne!(lines[0].hash, lines[1].hash);
        });
        // Selecting something else closes the blame.
        h.act(cx, |osier, _, cx| osier.select_commit(0, cx));
        h.read(cx, |osier, _| assert!(osier.blame.is_none()));

        h.act(cx, |osier, _, cx| {
            osier.show_file_history("b.txt".into(), cx)
        });
        cx.run_until_parked();
        h.read(cx, |osier, _| {
            assert_eq!(osier.tab, super::Tab::Log);
            let project = osier.active_project().expect("project");
            assert_eq!(project.log_path.as_deref(), Some("b.txt"));
            let commits = &project.snapshot.as_ref().expect("snapshot").commits;
            assert_eq!(commits.len(), 1);
            assert_eq!(commits[0].subject, "second");
        });
        h.act(cx, |osier, _, cx| osier.clear_file_history(cx));
        cx.run_until_parked();
        h.read(cx, |osier, _| {
            let project = osier.active_project().expect("project");
            assert!(project.log_path.is_none());
            assert_eq!(
                project.snapshot.as_ref().expect("snapshot").commits.len(),
                3
            );
        });
    }

    #[gpui::test]
    fn a_repository_deleted_while_open_shows_an_error(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "one\n", "first");
        let h = Harness::open(cx, &repo);
        h.read(cx, |osier, _| {
            assert!(osier.active_project().expect("project").error.is_none());
        });
        std::fs::remove_dir_all(repo.path()).expect("remove repo");
        h.act(cx, |osier, _, cx| osier.refresh_active(cx));
        cx.run_until_parked();
        h.read(cx, |osier, _| {
            let project = osier.active_project().expect("project still listed");
            assert!(
                project.error.is_some(),
                "refresh of a missing repo reports an error"
            );
            assert!(!project.loading);
        });
    }

    #[gpui::test]
    fn settings_sheet_closes_on_cancel(cx: &mut TestAppContext) {
        use gpui_kit::component::WindowExt as _;
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "one\n", "first");
        let h = Harness::open(cx, &repo);
        h.act(cx, |osier, window, cx| osier.open_settings(window, cx));
        cx.run_until_parked();
        let open = cx
            .update_window(h.window, |_, window, cx| window.has_active_sheet(cx))
            .expect("window");
        assert!(open, "sheet opened");
        // Focus must sit inside the sheet, otherwise Cancel never reaches it.
        let focused = cx
            .update_window(h.window, |_, window, cx| {
                window
                    .focused(cx)
                    .map(|f| f == h.view.read(cx).settings_focus)
            })
            .expect("window");
        assert_eq!(focused, Some(true), "settings content is focused");
        cx.dispatch_action(h.window, gpui_kit::component::dialog::Cancel);
        cx.run_until_parked();
        let open = cx
            .update_window(h.window, |_, window, cx| window.has_active_sheet(cx))
            .expect("window");
        assert!(!open, "Cancel (close button or Escape) closed the sheet");
    }

    #[gpui::test]
    fn phpstorm_patches_apply_and_render(cx: &mut TestAppContext) {
        use gpui_kit::component::diff::DiffFile;
        use osier_git::ops::patches::{ApplyOptions, normalize};
        use osier_git::testing::PHPSTORM_PATCH;
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "one\ntwo\nthree\n", "first");
        let h = Harness::open(cx, &repo);
        // The diff view gets the normalised text; gpui-kit's parser must accept it.
        for text in [
            PHPSTORM_PATCH.to_string(),
            PHPSTORM_PATCH.replace("\t(revision", "    (revision"),
        ] {
            let files = DiffFile::parse(&normalize(&text)).expect("gpui-kit parses it");
            assert_eq!(files.len(), 2);
        }
        let spaced = PHPSTORM_PATCH.replace("\t(revision", "    (revision");
        h.act(cx, |osier, window, cx| {
            osier.run_apply(
                spaced.clone(),
                ApplyOptions::default(),
                "clipboard patch".into(),
                None,
                window,
                cx,
            )
        });
        cx.run_until_parked();
        assert_eq!(repo.read("a.txt"), "one\nTWO\nthree\n");
        assert_eq!(repo.read("notes/new.md"), "# Notes\n-- not a signature\n");
        h.read(cx, |osier, _| assert_eq!(osier.tab, super::Tab::Changes));
    }

    #[gpui::test]
    fn commit_message_history_and_keyboard_staging(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "one\n", "first");
        repo.write("a.txt", "two\n");
        let h = Harness::open(cx, &repo);

        // Space on the selected file stages it, a second Space unstages it.
        h.act(cx, |osier, _, cx| {
            osier.select_file("a.txt".into(), Area::Unstaged, cx)
        });
        h.act(cx, |osier, window, cx| {
            osier.toggle_stage_selected(window, cx)
        });
        cx.run_until_parked();
        assert_eq!(
            repo.git(["diff", "--cached", "--name-only"]).trim(),
            "a.txt"
        );
        h.act(cx, |osier, _, cx| {
            osier.select_file("a.txt".into(), Area::Staged, cx)
        });
        h.act(cx, |osier, window, cx| {
            osier.toggle_stage_selected(window, cx)
        });
        cx.run_until_parked();
        assert_eq!(repo.git(["diff", "--cached", "--name-only"]).trim(), "");

        // Delete asks before discarding.
        h.act(cx, |osier, _, cx| {
            osier.select_file("a.txt".into(), Area::Unstaged, cx)
        });
        h.act(cx, |osier, window, cx| osier.discard_selected(window, cx));
        let asked = cx
            .update_window(h.window, |_, window, cx| {
                use gpui_kit::component::WindowExt as _;
                window.has_active_dialog(cx)
            })
            .expect("window");
        assert!(asked, "discard opens a confirmation");
        assert_eq!(repo.read("a.txt"), "two\n", "nothing discarded yet");

        // Committed messages come back in the history, newest first, and fill the box.
        h.act(cx, |osier, window, cx| {
            use gpui_kit::component::WindowExt as _;
            window.close_dialog(cx);
            osier.stage_paths(vec!["a.txt".into()], window, cx);
        });
        cx.run_until_parked();
        h.act(cx, |osier, window, cx| {
            osier.set_message("Second commit message", window, cx);
            osier.commit(window, cx);
        });
        cx.run_until_parked();
        let history = h.read(cx, |osier, _| osier.recent_commit_messages());
        assert_eq!(
            history.first().map(String::as_str),
            Some("Second commit message")
        );
        h.act(cx, |osier, window, cx| {
            osier.set_message(&history[0], window, cx)
        });
        h.read(cx, |osier, cx| {
            assert_eq!(osier.commit_message_text(cx), "Second commit message")
        });
    }

    #[gpui::test]
    fn tags_compare_picks_and_view_at_head(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "one\n", "first");
        repo.commit_file("a.txt", "two\n", "second");
        repo.git(["tag", "-a", "-m", "release", "v1"]);
        repo.write("a.txt", "three\n");
        let h = Harness::open(cx, &repo);

        h.read(cx, |osier, _| {
            let tags = &osier
                .active_project()
                .unwrap()
                .snapshot
                .as_ref()
                .unwrap()
                .tags;
            assert_eq!(tags.len(), 1);
            assert_eq!(tags[0].name, "v1");
            assert!(tags[0].annotated);
        });

        // Select the newest commit, then compare with the older one: older is the base.
        h.act(cx, |osier, _, cx| osier.select_commit(0, cx));
        h.act(cx, |osier, _, cx| osier.compare_with_selected(1, cx));
        h.read(cx, |osier, _| {
            let p = osier.active_project().unwrap();
            let commits = &p.snapshot.as_ref().unwrap().commits;
            assert_eq!(p.compare_commit, Some(1));
            assert_eq!(
                p.selection,
                Selection::Compare {
                    from: commits[1].hash.clone(),
                    to: commits[0].hash.clone(),
                    since_merge_base: false,
                }
            );
        });
        // A plain click clears the compare partner.
        h.act(cx, |osier, _, cx| osier.select_commit(1, cx));
        h.read(cx, |osier, _| {
            assert_eq!(osier.active_project().unwrap().compare_commit, None)
        });

        // View at HEAD shows the committed text, not the working tree.
        h.act(cx, |osier, _, cx| {
            osier.select_work_file("a.txt".into(), Some("HEAD".into()), cx)
        });
        h.read(cx, |osier, _| match &osier.viewer {
            Some(crate::viewer::Viewer::Code { text, .. }) => assert_eq!(text.as_ref(), "two\n"),
            other => panic!("unexpected viewer {other:?}"),
        });
    }

    #[gpui::test]
    fn pins_and_drag_reorder(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "one\n", "first");
        let other = TempRepo::new();
        other.commit_file("b.txt", "b\n", "first");
        let third = TempRepo::new();
        third.commit_file("c.txt", "c\n", "first");
        let h = Harness::open(cx, &repo);
        h.act(cx, |osier, window, cx| {
            osier.open_project_notify(other.path(), window, cx);
            osier.open_project_notify(third.path(), window, cx);
        });
        cx.run_until_parked();
        let names = |h: &Harness, cx: &mut TestAppContext| {
            h.read(cx, |osier, _| {
                osier
                    .projects
                    .iter()
                    .map(|p| p.path.clone())
                    .collect::<Vec<_>>()
            })
        };
        let before = names(&h, cx);
        assert_eq!(before.len(), 3);
        // Drag the last project to the top.
        h.act(cx, |osier, _, cx| osier.move_project_to(2, 0, cx));
        let after = names(&h, cx);
        assert_eq!(after[0], before[2]);
        assert_eq!(after[1], before[0]);
        h.read(cx, |osier, _| {
            assert_eq!(osier.state.open_projects, after, "order is persisted");
            assert_eq!(
                osier.active_project().unwrap().path,
                before[2],
                "the active project follows its row"
            );
        });

        // Pin the second repository, close it: it moves to Pinned, not Recent.
        let pinned_path = before[1].clone();
        h.act(cx, |osier, _, cx| osier.toggle_pin(&pinned_path, cx));
        h.act(cx, |osier, window, cx| {
            let ix = osier
                .projects
                .iter()
                .position(|p| p.path == pinned_path)
                .unwrap();
            osier.close_project(ix, window, cx);
        });
        h.read(cx, |osier, cx| {
            assert_eq!(osier.pinned_entries(cx), vec![pinned_path.clone()]);
            assert!(
                !osier
                    .recent_entries(cx)
                    .iter()
                    .any(|(p, _)| p == &pinned_path)
            );
        });
        h.act(cx, |osier, _, cx| osier.toggle_pin(&pinned_path, cx));
        h.read(cx, |osier, cx| {
            assert!(osier.pinned_entries(cx).is_empty());
            assert!(
                osier
                    .recent_entries(cx)
                    .iter()
                    .any(|(p, _)| p == &pinned_path)
            );
        });
    }

    #[gpui::test]
    fn markdown_mode_is_remembered_per_file_type(cx: &mut TestAppContext) {
        use osier_config::settings::MarkdownMode;
        let repo = TempRepo::new();
        repo.commit_file("README.md", "# Title\n", "first");
        repo.commit_file("notes.md", "# Notes\n", "second");
        repo.commit_file("page.mdx", "# Page\n", "third");
        let h = Harness::open(cx, &repo);
        h.act(cx, |osier, _, cx| {
            osier.select_work_file("README.md".into(), None, cx)
        });
        h.read(cx, |osier, _| {
            assert!(!osier.markdown_as_code, "rendered by default")
        });
        h.act(cx, |osier, _, cx| osier.toggle_markdown_code(cx));
        h.read(cx, |osier, _| {
            assert!(osier.markdown_as_code);
            assert_eq!(
                osier.settings.viewer.markdown_modes.get("md"),
                Some(&MarkdownMode::Code)
            );
        });
        // Another .md opens as code; .mdx keeps the default.
        h.act(cx, |osier, _, cx| {
            osier.select_work_file("notes.md".into(), None, cx)
        });
        h.read(cx, |osier, _| assert!(osier.markdown_as_code));
        h.act(cx, |osier, _, cx| {
            osier.select_work_file("page.mdx".into(), None, cx)
        });
        h.read(cx, |osier, _| assert!(!osier.markdown_as_code));
        // Toggling back to the default drops the override instead of storing it.
        h.act(cx, |osier, _, cx| {
            osier.select_work_file("notes.md".into(), None, cx)
        });
        h.act(cx, |osier, _, cx| osier.toggle_markdown_code(cx));
        h.read(cx, |osier, _| {
            assert!(osier.settings.viewer.markdown_modes.is_empty())
        });
    }

    #[gpui::test]
    fn theme_menu_switches_mode_and_theme(cx: &mut TestAppContext) {
        use gpui_kit::component::ActiveTheme as _;
        use osier_config::settings::ThemeMode;
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "one\n", "first");
        let h = Harness::open(cx, &repo);

        h.read(cx, |_, cx| {
            assert!(cx.theme().mode.is_dark());
            assert_eq!(cx.theme().theme_name().as_ref(), "Catppuccin Mocha");
        });
        h.act(cx, |osier, _, cx| {
            osier.set_theme_mode(ThemeMode::Light, cx)
        });
        h.read(cx, |osier, cx| {
            assert!(!cx.theme().mode.is_dark());
            assert_eq!(cx.theme().theme_name().as_ref(), "Catppuccin Latte");
            assert_eq!(osier.settings.appearance.mode, ThemeMode::Light);
        });
        // Picking a dark theme while in light mode switches to dark and remembers it.
        h.act(cx, |osier, _, cx| osier.set_theme("Catppuccin Frappe", cx));
        h.read(cx, |osier, cx| {
            assert!(cx.theme().mode.is_dark());
            assert_eq!(cx.theme().theme_name().as_ref(), "Catppuccin Frappe");
            assert_eq!(osier.settings.appearance.dark_theme, "Catppuccin Frappe");
            assert_eq!(osier.settings.appearance.light_theme, "Catppuccin Latte");
        });
        // Unknown names are ignored.
        h.act(cx, |osier, _, cx| osier.set_theme("Nope", cx));
        h.read(cx, |_, cx| {
            assert_eq!(cx.theme().theme_name().as_ref(), "Catppuccin Frappe")
        });
    }

    #[gpui::test]
    fn dux_shell_sync_applies_the_generated_theme(cx: &mut TestAppContext) {
        use crate::themes::{DuxSource, ThemeStore};
        use gpui_kit::component::ActiveTheme as _;
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "one\n", "first");
        let h = Harness::open(cx, &repo);
        let dms = tempfile::tempdir().expect("tempdir");
        let colors = dms.path().join("dms-colors.json");
        std::fs::write(
            &colors,
            r##"{"mode":"light","colors":{"dark":{"background":"#101418","primary":"#9ecbff"},
                "light":{"background":"#f7f9fe","primary":"#1f5f98"}},"dank16":{}}"##,
        )
        .expect("write dms file");

        h.read(cx, |osier, cx| {
            assert!(!ThemeStore::global(cx).dux_available());
            assert!(!osier.settings.appearance.dux_shell_sync);
        });
        h.act(cx, |osier, _, cx| {
            ThemeStore::global_mut(cx).set_dux_source(DuxSource::File(colors.clone()));
            osier.set_dux_sync(true, cx);
        });
        h.read(cx, |osier, cx| {
            assert!(ThemeStore::global(cx).dux_available());
            // Mode is still the user's (dark), only the colours come from Dux Shell.
            assert_eq!(cx.theme().theme_name().as_ref(), "Dux Shell Dark");
            let rgb = cx.theme().background.to_rgb();
            assert_eq!((rgb.r * 255.0).round() as u8, 0x10);
            assert_eq!((rgb.b * 255.0).round() as u8, 0x18);
            assert!(osier.settings.appearance.dux_shell_sync);
            let names = ThemeStore::global(cx).names(true);
            assert!(!names.iter().any(|n| n.as_ref() == "Dux Shell Dark"));
        });
        // System mode follows the Dux Shell mode while sync is on.
        h.act(cx, |osier, _, cx| {
            osier.set_theme_mode(osier_config::settings::ThemeMode::System, cx)
        });
        h.read(cx, |_, cx| {
            assert_eq!(cx.theme().theme_name().as_ref(), "Dux Shell Light")
        });
        // Picking an explicit theme turns sync off.
        h.act(cx, |osier, _, cx| osier.set_theme("Catppuccin Mocha", cx));
        h.read(cx, |osier, cx| {
            assert_eq!(cx.theme().theme_name().as_ref(), "Catppuccin Mocha");
            assert!(!osier.settings.appearance.dux_shell_sync);
        });
    }

    #[gpui::test]
    fn opens_a_repository_and_selects_the_first_change(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "one\n", "first");
        repo.write("a.txt", "two\n");
        repo.write("new.txt", "new\n");
        let h = Harness::open(cx, &repo);

        h.read(cx, |osier, _| {
            let project = osier.active_project().expect("active project");
            let snapshot = project.snapshot.as_ref().expect("snapshot loaded");
            assert_eq!(snapshot.status.unstaged().count(), 1);
            assert_eq!(snapshot.status.untracked().count(), 1);
            assert_eq!(snapshot.commits.len(), 1);
            assert_eq!(
                project.selection,
                Selection::File {
                    path: "a.txt".into(),
                    area: Area::Unstaged
                }
            );
            assert_eq!(osier.diff_files, 1, "diff pane shows the selected file");
            assert_eq!(osier.diff_model.len(), 1, "hunk model parsed");
            assert!(osier.state.open_projects.contains(&project.path));
        });

        // Keyboard navigation moves to the next change and reloads the diff.
        h.act(cx, |osier, _, cx| osier.move_selection(1, cx));
        h.read(cx, |osier, _| {
            assert_eq!(
                osier.active_project().expect("project").selection,
                Selection::File {
                    path: "new.txt".into(),
                    area: Area::Untracked
                }
            );
            assert_eq!(osier.diff_files, 1);
        });
    }

    #[gpui::test]
    fn staging_and_committing_through_the_app(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "one\n", "first");
        repo.write("a.txt", "two\n");
        let h = Harness::open(cx, &repo);

        h.act(cx, |osier, window, cx| {
            osier.stage_paths(vec!["a.txt".into()], window, cx)
        });
        h.read(cx, |osier, _| {
            let snapshot = osier.active_project().unwrap().snapshot.as_ref().unwrap();
            assert_eq!(
                snapshot.status.staged().count(),
                1,
                "file staged via the app"
            );
            assert_eq!(snapshot.status.unstaged().count(), 0);
        });

        h.act(cx, |osier, window, cx| {
            osier.set_message("From the headless test", window, cx);
            assert!(osier.can_commit(cx));
            osier.commit(window, cx);
        });
        h.read(cx, |osier, cx| {
            let snapshot = osier.active_project().unwrap().snapshot.as_ref().unwrap();
            assert_eq!(snapshot.commits.len(), 2);
            assert_eq!(snapshot.commits[0].subject, "From the headless test");
            assert!(snapshot.status.is_clean());
            assert!(
                osier.commit_message_text(cx).is_empty(),
                "message cleared after commit"
            );
        });
        assert_eq!(repo.git(["log", "--oneline"]).lines().count(), 2);
    }

    #[gpui::test]
    fn hunk_staging_and_discard_from_the_diff_pane(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        let base = "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n";
        repo.commit_file("f.txt", base, "base");
        repo.write(
            "f.txt",
            "ONE\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nTEN\n",
        );
        let h = Harness::open(cx, &repo);
        h.read(cx, |osier, _| {
            assert_eq!(osier.diff_model[0].hunks.len(), 2)
        });

        h.act(cx, |osier, window, cx| {
            osier.hunk_action(
                "f.txt".into(),
                1,
                crate::hunk_ops::HunkAction::Stage,
                window,
                cx,
            )
        });
        h.read(cx, |osier, _| {
            let snapshot = osier.active_project().unwrap().snapshot.as_ref().unwrap();
            let f = snapshot
                .status
                .entries
                .iter()
                .find(|e| e.path == "f.txt")
                .unwrap();
            assert!(f.is_staged() && f.is_unstaged(), "partially staged");
        });
        // Staged side shows exactly the second hunk.
        h.act(cx, |osier, _, cx| {
            osier.select_file("f.txt".into(), Area::Staged, cx)
        });
        h.read(cx, |osier, _| {
            let hunks = &osier.diff_model[0].hunks;
            assert_eq!(hunks.len(), 1);
            let added: Vec<&str> = hunks[0]
                .lines
                .iter()
                .filter(|l| l.kind == osier_git::diff::LineKind::Added)
                .map(|l| l.text.as_str())
                .collect();
            assert_eq!(added, ["TEN"], "only the second hunk is staged");
        });
    }

    #[gpui::test]
    fn closing_projects_and_recents(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "one\n", "first");
        let other = TempRepo::new();
        other.commit_file("b.txt", "two\n", "first");
        let h = Harness::open(cx, &repo);

        // Open a second project, close the first: the second stays active, the first is recent.
        let other_path = other.path().to_path_buf();
        h.act(cx, |osier, window, cx| {
            osier.open_project_notify(&other_path, window, cx)
        });
        h.read(cx, |osier, _| assert_eq!(osier.projects.len(), 2));
        h.act(cx, |osier, window, cx| osier.close_project(0, window, cx));
        h.read(cx, |osier, _| {
            assert_eq!(osier.projects.len(), 1);
            assert_eq!(osier.active_project().unwrap().path, other.path());
            assert_eq!(osier.state.recent_not_open().count(), 1);
        });

        // Forget the recent entry.
        let gone = repo.path().to_path_buf();
        h.act(cx, |osier, window, cx| {
            osier.forget_recent(&gone, window, cx)
        });
        h.read(cx, |osier, _| {
            assert_eq!(osier.state.recent_not_open().count(), 0)
        });

        // A path that is not a repository fails cleanly and is not kept anywhere.
        let not_repo = tempfile::tempdir().unwrap();
        let p = not_repo.path().to_path_buf();
        h.act(cx, |osier, _, cx| {
            assert!(osier.open_project(&p, true, cx).is_err());
        });
        h.read(cx, |osier, _| {
            assert_eq!(osier.projects.len(), 1);
            assert!(!osier.state.recent.contains(&p));
        });
    }

    #[gpui::test]
    fn push_fetch_and_pull_through_the_app(cx: &mut TestAppContext) {
        use crate::remote_ops::RemoteOp;
        use osier_git::ops::remote::PushOptions;

        let repo = TempRepo::new();
        repo.commit_file("a.txt", "one\n", "first");
        let bare = repo.add_bare_origin();
        let h = Harness::open(cx, &repo);

        // Push and set upstream.
        h.act(cx, |osier, window, cx| {
            osier.start_remote(
                RemoteOp::Push(PushOptions {
                    remote: Some("origin".into()),
                    set_upstream: true,
                    ..Default::default()
                }),
                window,
                cx,
            )
        });
        h.read(cx, |osier, _| {
            assert!(osier.remote_job.is_none(), "job finished");
            let snapshot = osier.active_project().unwrap().snapshot.as_ref().unwrap();
            assert_eq!(
                snapshot.status.branch.upstream.as_deref(),
                Some("origin/main")
            );
            assert_eq!(snapshot.remotes.len(), 1);
        });

        // Someone else pushes; fetch shows us behind; pull fast-forwards.
        let other = TempRepo::new();
        other.git(["remote", "add", "origin", bare.to_string_lossy().as_ref()]);
        other.git(["fetch", "-q", "origin"]);
        other.git(["checkout", "-q", "-b", "main", "origin/main"]);
        other.commit_file("b.txt", "two\n", "second");
        other.git(["push", "-q", "origin", "main"]);

        h.act(cx, |osier, window, cx| {
            osier.start_remote(RemoteOp::fetch_all(), window, cx)
        });
        h.read(cx, |osier, _| {
            let snapshot = osier.active_project().unwrap().snapshot.as_ref().unwrap();
            assert_eq!(snapshot.status.branch.behind, 1);
        });
        h.act(cx, |osier, window, cx| {
            osier.start_remote(RemoteOp::pull_default(), window, cx)
        });
        h.read(cx, |osier, _| {
            let snapshot = osier.active_project().unwrap().snapshot.as_ref().unwrap();
            assert_eq!(snapshot.status.branch.behind, 0);
            assert_eq!(snapshot.commits.len(), 2);
        });
        assert!(repo.path().join("b.txt").exists());

        // Session-only account choices never reach state.toml.
        h.act(cx, |osier, _, _| {
            osier.set_project_account("/tmp/x", "alice", false);
            osier.save_state();
        });
        h.read(cx, |osier, _| {
            let saved = osier_config::State::load(&osier.paths).unwrap();
            assert!(!saved.accounts.contains_key("/tmp/x"));
            assert_eq!(
                osier.state.accounts.get("/tmp/x").map(String::as_str),
                Some("alice")
            );
        });
    }

    #[gpui::test]
    fn merge_conflict_resolve_and_continue(cx: &mut TestAppContext) {
        use osier_git::ops::integrate::{ConflictSide, MergeMode};
        use osier_git::repo::InProgress;

        let repo = TempRepo::new();
        repo.commit_file("c.txt", "base\n", "base");
        repo.checkout_new("feature");
        repo.commit_file("c.txt", "theirs\n", "theirs");
        repo.checkout("main");
        repo.commit_file("c.txt", "ours\n", "ours");
        let h = Harness::open(cx, &repo);

        h.act(cx, |osier, window, cx| {
            osier.merge_branch("feature".into(), MergeMode::Default, window, cx)
        });
        h.read(cx, |osier, _| {
            assert_eq!(osier.in_progress(), Some(InProgress::Merge));
            let snapshot = osier.active_project().unwrap().snapshot.as_ref().unwrap();
            assert_eq!(snapshot.status.conflicted().count(), 1);
            assert_eq!(
                osier.active_project().unwrap().selection,
                Selection::File {
                    path: "c.txt".into(),
                    area: Area::Conflicted
                }
            );
            assert_eq!(
                osier.diff_files, 1,
                "conflict view loaded from the working file"
            );
        });

        h.act(cx, |osier, window, cx| {
            osier.resolve_conflict_with("c.txt".into(), ConflictSide::Theirs, window, cx)
        });
        h.read(cx, |osier, _| {
            let snapshot = osier.active_project().unwrap().snapshot.as_ref().unwrap();
            assert_eq!(snapshot.status.conflicted().count(), 0);
            assert_eq!(osier.in_progress(), Some(InProgress::Merge));
        });
        h.act(cx, |osier, window, cx| {
            osier.continue_in_progress(window, cx)
        });
        h.read(cx, |osier, _| {
            assert_eq!(osier.in_progress(), None);
            let snapshot = osier.active_project().unwrap().snapshot.as_ref().unwrap();
            assert!(snapshot.status.is_clean());
            assert!(snapshot.commits[0].is_merge());
        });
        assert_eq!(repo.read("c.txt"), "theirs\n");

        // Compare the merged branch with its parent: one file.
        h.act(cx, |osier, _, cx| {
            osier.select_compare("main~1".into(), "main".into(), false, cx)
        });
        h.read(cx, |osier, _| assert_eq!(osier.diff_files, 1));
        h.read(cx, |osier, _| {
            assert!(
                osier
                    .active_project()
                    .unwrap()
                    .snapshot
                    .as_ref()
                    .unwrap()
                    .reflog
                    .len()
                    >= 3
            )
        });
    }

    #[gpui::test]
    fn scan_dirs_feed_recents_and_filter_and_reorder(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a\n", "first");
        let second = TempRepo::new();
        second.commit_file("b.txt", "b\n", "first");
        // A scan directory holding two repositories and one plain folder.
        let scan_root = tempfile::tempdir().unwrap();
        for name in ["alpha", "beta"] {
            let r = scan_root.path().join(name);
            std::fs::create_dir_all(&r).unwrap();
            std::process::Command::new("git")
                .args(["init", "-q"])
                .current_dir(&r)
                .status()
                .unwrap();
        }
        std::fs::create_dir_all(scan_root.path().join("plain")).unwrap();

        let h = Harness::open(cx, &repo);
        h.act(cx, |osier, _, cx| {
            osier.settings.projects.scan_dirs = vec![scan_root.path().to_path_buf()];
            osier.rescan_projects(cx);
        });
        h.read(cx, |osier, cx| {
            assert_eq!(
                osier.scanned.len(),
                2,
                "alpha and beta found, plain skipped"
            );
            let recents = osier.recent_entries(cx);
            assert_eq!(recents.len(), 2);
            assert!(recents.iter().all(|(_, provider)| provider.is_some()));
        });

        // Filtering narrows recents; Enter opens the first match.
        h.act(cx, |osier, window, cx| {
            osier
                .sidebar_filter
                .update(cx, |s, cx| s.set_value("beta", window, cx))
        });
        h.read(cx, |osier, cx| {
            assert_eq!(osier.recent_entries(cx).len(), 1)
        });
        h.act(cx, |osier, window, cx| {
            osier.open_first_filtered(window, cx)
        });
        h.read(cx, |osier, _| {
            assert_eq!(osier.projects.len(), 2);
            assert!(osier.active_project().unwrap().path.ends_with("beta"));
        });

        // A scanned entry cannot be forgotten: recents are unchanged after the attempt.
        h.act(cx, |osier, window, cx| {
            osier
                .sidebar_filter
                .update(cx, |s, cx| s.set_value("", window, cx))
        });
        let alpha = scan_root.path().join("alpha");
        h.act(cx, |osier, window, cx| {
            osier.forget_recent(&alpha, window, cx)
        });
        h.read(cx, |osier, cx| {
            assert!(
                osier
                    .recent_entries(cx)
                    .iter()
                    .any(|(p, _)| p.ends_with("alpha"))
            )
        });

        // Reorder open projects.
        let second_path = second.path().to_path_buf();
        h.act(cx, |osier, window, cx| {
            osier.open_project_notify(&second_path, window, cx)
        });
        h.read(cx, |osier, _| assert_eq!(osier.projects.len(), 3));
        h.act(cx, |osier, _, cx| osier.move_project(2, -2, cx));
        h.read(cx, |osier, _| {
            assert_eq!(osier.projects[0].path, second.path());
            assert_eq!(osier.state.open_projects[0], second.path());
            assert_eq!(
                osier.active_project().unwrap().path,
                second.path(),
                "active follows the moved project"
            );
        });
    }

    #[gpui::test]
    fn files_tab_viewer_picks_markdown_code_and_binary(cx: &mut TestAppContext) {
        use crate::viewer::Viewer;
        let repo = TempRepo::new();
        repo.commit_file("README.md", "# Title\n\nhello\n", "docs");
        repo.commit_file("src/main.rs", "fn main() {}\n", "code");
        repo.write("blob.bin", "ab\0cd");
        let h = Harness::open(cx, &repo);
        h.read(cx, |osier, _| {
            let files = &osier
                .active_project()
                .unwrap()
                .snapshot
                .as_ref()
                .unwrap()
                .files;
            assert_eq!(files, &["README.md", "blob.bin", "src/main.rs"]);
        });
        h.act(cx, |osier, _, cx| {
            osier.select_work_file("README.md".into(), None, cx)
        });
        h.read(cx, |osier, _| {
            assert!(matches!(osier.viewer, Some(Viewer::Markdown { .. })))
        });
        h.act(cx, |osier, _, cx| {
            osier.select_work_file("src/main.rs".into(), None, cx)
        });
        h.read(cx, |osier, _| {
            assert!(matches!(
                &osier.viewer,
                Some(Viewer::Code {
                    language: "rust",
                    ..
                })
            ));
        });
        h.act(cx, |osier, _, cx| {
            osier.select_work_file("blob.bin".into(), None, cx)
        });
        h.read(cx, |osier, _| {
            assert!(matches!(osier.viewer, Some(Viewer::Binary { .. })))
        });
        // At a revision: the committed README is served from the object store.
        h.act(cx, |osier, _, cx| {
            osier.select_work_file("README.md".into(), Some("HEAD".into()), cx)
        });
        h.read(cx, |osier, _| {
            assert!(matches!(osier.viewer, Some(Viewer::Markdown { .. })))
        });
        h.act(cx, |osier, _, cx| {
            osier.select_work_file("nope.txt".into(), Some("HEAD".into()), cx)
        });
        h.read(cx, |osier, _| {
            assert!(matches!(osier.viewer, Some(Viewer::Missing)))
        });
    }
}
