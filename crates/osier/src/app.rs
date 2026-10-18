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
use crate::sidebar_query::{self, Query, RowStatus};
use osier_config::settings::RecentStatusMode;

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
        FocusProjectSearch,
        FocusFileSearch,
        FocusSettingsSearch,
        CommitStaged,
        Fetch,
        Pull,
        Push,
    ]
);

pub fn init(cx: &mut App) {
    crate::keymap::init(cx);
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

/// Longest tooltip delay the settings slider offers, in milliseconds.
pub(crate) const TOOLTIP_DELAY_MAX_MS: u32 = 1000;

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
    /// Where the filter box was painted, to anchor the dropdown.
    pub(crate) sidebar_filter_bounds: std::rc::Rc<std::cell::Cell<Bounds<Pixels>>>,
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
    /// Scroll state of the main tab strip, to tell when the tabs overflow.
    pub(crate) tab_scroll: ScrollHandle,
    /// Settings group being flashed after `open_settings_at`, and since when.
    pub(crate) settings_highlight:
        Option<(crate::settings_panel::SettingsTarget, std::time::Instant)>,
    /// Where the open settings dialog was asked to start, if anywhere.
    pub(crate) settings_target: Option<crate::settings_panel::SettingsTarget>,
    /// First frame of `open_settings_at`: see `settings_panel::pages`.
    pub(crate) settings_scroll_nudge: bool,
    pub(crate) config_watch: Option<crate::config_watch::ConfigWatch>,
    keymap_watch: Option<crate::config_watch::ConfigWatch>,
    /// The `keymap.toml` contents last applied, to tell our own writes from outside edits.
    pub(crate) keymap_applied: osier_config::keymap::KeymapFile,
    /// The action whose new key is being recorded, and the interceptor doing it.
    pub(crate) keymap_recording: Option<(&'static str, Subscription)>,
    /// Where the DMS cheatsheet goes (`dms_cheatsheet.rs`). `None` in tests unless set.
    pub(crate) dms_dir: Option<PathBuf>,
    /// Badges for projects that are not open (`recent_status.rs`).
    pub(crate) recent_status: crate::recent_status::RecentStatus,
    /// Interval slider on the settings page (`projects.recent_status_minutes`).
    pub(crate) recent_interval: Entity<gpui_kit::component::slider::SliderState>,
    pub(crate) tooltip_delay: Entity<gpui_kit::component::slider::SliderState>,
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
        let recent_status = crate::recent_status::RecentStatus::new(
            settings.projects.recent_status,
            &paths,
            sink.clone(),
        );
        let minutes = settings.projects.recent_status_minutes.clamp(1, 60) as f32;
        let recent_interval = cx.new(|_| {
            gpui_kit::component::slider::SliderState::new()
                .min(1.)
                .max(60.)
                .step(1.)
                .default_value(minutes)
        });
        let tooltip_ms = settings
            .appearance
            .tooltip_delay_ms
            .min(TOOLTIP_DELAY_MAX_MS) as f32;
        let tooltip_delay = cx.new(|_| {
            gpui_kit::component::slider::SliderState::new()
                .min(0.)
                .max(TOOLTIP_DELAY_MAX_MS as f32)
                .step(50.)
                .default_value(tooltip_ms)
        });
        let keymap_watch = if cfg!(test) {
            None
        } else {
            crate::config_watch::ConfigWatch::new(&paths.keymap_file())
        };
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
        let message_subscription = cx.subscribe_in(
            &commit_message,
            window,
            |this, _, ev: &InputEvent, window, cx| match ev {
                InputEvent::Change => cx.notify(),
                // ctrl-enter inside the box: the textarea turns it into a secondary Enter,
                // so the window-level CommitStaged binding never sees it.
                InputEvent::PressEnter {
                    secondary: true, ..
                } => this.commit(window, cx),
                _ => {}
            },
        );
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
            sidebar_filter_bounds: Default::default(),
            session_only_accounts: Default::default(),
            patches,
            patch_selected: None,
            blame: None,
            ssh_status: None,
            git_version: None,
            // A tab stop, so moving to the next stop from here lands on the settings search.
            settings_focus: cx.focus_handle().tab_stop(true),
            tab_scroll: ScrollHandle::new(),
            settings_highlight: None,
            settings_target: None,
            settings_scroll_nudge: false,
            config_watch,
            keymap_watch,
            keymap_applied: Default::default(),
            keymap_recording: None,
            dms_dir: if cfg!(test) {
                None
            } else {
                crate::dms_cheatsheet::dms_dir()
            },
            recent_status,
            recent_interval,
            tooltip_delay,
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
            .push(cx.observe_window_activation(window, |this, window, _| {
                if window.is_window_active() {
                    this.recent_status_on_focus();
                }
            }));
        let slider = this.tooltip_delay.clone();
        this._subscriptions.push(cx.subscribe(
            &slider,
            |this, _, ev: &gpui_kit::component::slider::SliderEvent, cx| {
                use gpui_kit::component::slider::SliderEvent;
                let (v, save) = match ev {
                    SliderEvent::Change(v) => (v, false),
                    SliderEvent::Release(v) => (v, true),
                };
                this.settings.appearance.tooltip_delay_ms = v.start().round() as u32;
                this.apply_tooltip_delay(cx);
                if save {
                    this.save_settings();
                }
                cx.notify();
            },
        ));
        let slider = this.recent_interval.clone();
        this._subscriptions.push(cx.subscribe(
            &slider,
            |this, _, ev: &gpui_kit::component::slider::SliderEvent, cx| {
                use gpui_kit::component::slider::SliderEvent;
                match ev {
                    SliderEvent::Change(v) => {
                        this.settings.projects.recent_status_minutes = v.start().round() as u32;
                        cx.notify();
                    }
                    SliderEvent::Release(v) => {
                        this.settings.projects.recent_status_minutes = v.start().round() as u32;
                        this.save_settings();
                        cx.notify();
                    }
                }
            },
        ));
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
        this.load_keymap(cx);
        // Without a focused element the app's key bindings have no dispatch path, so
        // shortcuts would only work after the first click into a list.
        let root_focus = this.focus_handle.clone();
        window.defer(cx, move |window, cx| root_focus.focus(window, cx));
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

    pub(crate) fn sidebar_query(&self, cx: &App) -> Query {
        Query::parse(&self.sidebar_filter_text(cx))
    }

    /// State of an open project for `is:` filters; `None` until its first snapshot.
    pub(crate) fn open_row_status(project: &Project) -> Option<RowStatus> {
        project.snapshot.as_ref().map(|s| RowStatus {
            dirty: Some(!s.status.entries.is_empty()),
            ahead: s.status.branch.ahead,
            behind: s.status.branch.behind,
        })
    }

    /// State of a project that is not open, from the recent-status badges. Never known when
    /// recent status is off, so `is:` filters only cover open projects then.
    pub(crate) fn other_row_status(&self, path: &Path) -> Option<RowStatus> {
        if self.settings.projects.recent_status == RecentStatusMode::Never {
            return None;
        }
        self.recent_badge(path).map(|b| RowStatus {
            dirty: b.dirty,
            ahead: b.ahead,
            behind: b.behind,
        })
    }

    /// Recents and scanned repositories that are not open, filtered, with their origin.
    /// Pinned repositories that are not open, filtered like the rest of the sidebar.
    pub(crate) fn pinned_entries(&self, cx: &App) -> Vec<PathBuf> {
        let query = self.sidebar_query(cx);
        self.state
            .pinned
            .iter()
            .filter(|p| !self.projects.iter().any(|o| &o.path == *p))
            .filter(|p| query.matches(p, self.other_row_status(p)))
            .cloned()
            .collect()
    }

    pub(crate) fn recent_entries(&self, cx: &App) -> Vec<(PathBuf, Option<PathBuf>)> {
        let query = self.sidebar_query(cx);
        let mut out: Vec<(PathBuf, Option<PathBuf>)> = Vec::new();
        for p in self.state.recent_not_open() {
            if self.state.is_pinned(p) {
                continue;
            }
            if query.matches(p, self.other_row_status(p)) {
                let provider =
                    osier_config::scan::provider(&self.settings.projects.scan_dirs, p).cloned();
                out.push((p.clone(), provider));
            }
        }
        for p in &self.scanned {
            if !self.state.open_projects.contains(p)
                && !self.state.is_pinned(p)
                && !out.iter().any(|(q, _)| q == p)
                && query.matches(p, self.other_row_status(p))
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
        let text = self.sidebar_filter.read(cx).value().to_string();
        // A half-typed qualifier (`is:a`): Enter completes it instead of opening something.
        if !text.ends_with(char::is_whitespace)
            && let Some((term, _)) = sidebar_query::suggestions(&text).first()
            && !text.split_whitespace().last().unwrap_or("").is_empty()
        {
            self.pick_filter_suggestion(term, window, cx);
            return;
        }
        let query = Query::parse(&text);
        if query.is_empty() {
            return;
        }
        if let Some(ix) = self
            .projects
            .iter()
            .position(|p| query.matches(&p.path, Self::open_row_status(p)))
        {
            self.activate(ix, window, cx);
            return;
        }
        if let Some((path, _)) = self.recent_entries(cx).into_iter().next() {
            self.open_project_notify(&path, window, cx);
        }
    }

    /// Put a suggested qualifier into the sidebar filter in place of the word being typed.
    pub(crate) fn pick_filter_suggestion(
        &mut self,
        term: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = self.sidebar_filter.read(cx).value().to_string();
        let next = sidebar_query::insert(&text, term);
        self.sidebar_filter.update(cx, |input, cx| {
            input.set_value(next, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
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

    /// Install the tooltip delay for every tooltip in the app (takes effect at once).
    pub(crate) fn apply_tooltip_delay(&self, cx: &mut App) {
        let ms = self
            .settings
            .appearance
            .tooltip_delay_ms
            .min(TOOLTIP_DELAY_MAX_MS);
        gpui_kit::base::TooltipDefaults::global(cx)
            .with_show_delay(Duration::from_millis(u64::from(ms)))
            .install(cx);
    }

    pub(crate) fn apply_appearance(&mut self, cx: &mut App) {
        self.apply_tooltip_delay(cx);
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
        let watch_runner =
            self.runner
                .clone()
                .with_sink(Arc::new(crate::command_log::FailuresOnly(
                    self.sink.clone(),
                )));
        match Project::open(
            self.runner.clone(),
            watch_runner,
            path,
            self.settings.git.log_page_size,
        ) {
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
                let b = &snapshot.status.branch;
                self.recent_status.record(
                    path,
                    !snapshot.status.entries.is_empty(),
                    b.ahead,
                    b.behind,
                );
                let project = &mut self.projects[ix];
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
                    // Live updates for ignored files only while the Files tab lists them.
                    let watch_all = this.tab == Tab::Files && this.show_ignored;
                    let active = this.active;
                    for (ix, p) in this.projects.iter_mut().enumerate() {
                        p.set_watch_all(watch_all && active == Some(ix));
                    }
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
                    this.pump_recent_status(cx);
                    if this.keymap_watch.as_ref().is_some_and(|w| w.take_dirty()) {
                        this.reload_keymap(window, cx);
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

    /// Apply `keymap.toml` (startup, or after it changed). Problems are logged and returned.
    pub(crate) fn load_keymap(&mut self, cx: &mut App) -> Vec<String> {
        let file = match osier_config::keymap::KeymapFile::load(&self.paths) {
            Ok(f) => f,
            Err(error) => {
                tracing::warn!(%error, "keymap.toml is invalid, keeping the current keys");
                return vec![format!("keymap.toml: {error}")];
            }
        };
        self.apply_keymap_file(file, cx)
    }

    fn apply_keymap_file(
        &mut self,
        file: osier_config::keymap::KeymapFile,
        cx: &mut App,
    ) -> Vec<String> {
        crate::keymap::apply(&file, cx);
        self.keymap_applied = file;
        if let Some(dms) = &self.dms_dir {
            match crate::dms_cheatsheet::write(dms, crate::keymap::Keymap::global(cx)) {
                Ok(true) => tracing::debug!(dir = %dms.display(), "wrote the DMS cheatsheet"),
                Ok(false) => {}
                Err(error) => tracing::warn!(%error, "could not write the DMS cheatsheet"),
            }
        }
        let problems = crate::keymap::Keymap::global(cx).problems.clone();
        for p in &problems {
            tracing::warn!(problem = %p, "keymap.toml");
        }
        problems
    }

    /// `keymap.toml` changed on disk. Our own saves land here too; they change nothing.
    pub(crate) fn reload_keymap(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use gpui_kit::component::WindowExt as _;
        use gpui_kit::component::notification::Notification;
        if osier_config::keymap::KeymapFile::load(&self.paths)
            .ok()
            .as_ref()
            == Some(&self.keymap_applied)
        {
            return;
        }
        let problems = self.load_keymap(cx);
        if problems.is_empty() {
            window.push_notification("Key bindings reloaded from keymap.toml", cx);
        } else {
            window.push_notification(
                Notification::warning("keymap.toml has problems").message(problems.join("\n")),
                cx,
            );
        }
        cx.notify();
    }

    /// Set an action's keys (Settings > Keymap). Matching the defaults removes the override.
    /// A `keymap.toml` that does not parse is never overwritten.
    pub(crate) fn set_action_keys(
        &mut self,
        id: &'static str,
        keys: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use gpui_kit::component::WindowExt as _;
        use gpui_kit::component::notification::Notification;
        let mut file = match osier_config::keymap::KeymapFile::load(&self.paths) {
            Ok(f) => f,
            Err(error) => {
                window.push_notification(
                    Notification::error("keymap.toml has an error; fix it first")
                        .message(error.to_string()),
                    cx,
                );
                return;
            }
        };
        let defaults: Vec<String> = crate::keymap::actions()
            .iter()
            .find(|a| a.id == id)
            .map(|a| a.defaults.iter().map(|k| k.to_string()).collect())
            .unwrap_or_default();
        if keys == defaults {
            file.bindings.remove(id);
        } else {
            file.bindings.insert(id.to_string(), keys);
        }
        if let Err(error) = file.save(&self.paths) {
            window.push_notification(
                Notification::error("Could not save keymap.toml").message(error.to_string()),
                cx,
            );
            return;
        }
        self.apply_keymap_file(file, cx);
        cx.notify();
    }

    /// Open `keymap.toml` in the editor, creating it with an explanatory header if missing.
    pub(crate) fn edit_keymap_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let path = self.paths.keymap_file();
        if !path.exists() {
            let header = "# Osier key bindings: only your changes to the defaults.\n\
                # Example:\n# [bindings]\n# refresh = [\"f5\"]\n# open_settings = []   # unbound\n\n[bindings]\n";
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(error) = std::fs::write(&path, header) {
                tracing::warn!(%error, "could not create keymap.toml");
            }
        }
        self.open_externally(&path, window, cx);
    }

    /// Reset every binding to the defaults (removes `keymap.toml`).
    pub(crate) fn reset_keymap(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use gpui_kit::component::WindowExt as _;
        let file = osier_config::keymap::KeymapFile::default();
        if let Err(error) = file.save(&self.paths) {
            window.push_notification(
                gpui_kit::component::notification::Notification::error("Could not reset keymap")
                    .message(error.to_string()),
                cx,
            );
            return;
        }
        self.apply_keymap_file(file, cx);
        cx.notify();
    }

    /// Record the next key combination for `id` (the "+" on the Keymap page). The interceptor
    /// runs before key bindings, so the keys are captured instead of triggering anything.
    /// Escape cancels.
    pub(crate) fn start_key_recording(&mut self, id: &'static str, cx: &mut Context<Self>) {
        let weak = cx.weak_entity();
        let sub = cx.intercept_keystrokes(move |event, window, cx| {
            let Some(keys) = crate::keymap::keystroke_string(&event.keystroke) else {
                return; // a modifier on its own: keep waiting
            };
            cx.stop_propagation();
            let _ = weak.update(cx, |this, cx| this.finish_key_recording(keys, window, cx));
        });
        self.keymap_recording = Some((id, sub));
        cx.notify();
    }

    fn finish_key_recording(&mut self, keys: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some((id, _sub)) = self.keymap_recording.take() else {
            return;
        };
        cx.notify();
        if keys == "escape" {
            return;
        }
        let mut current = crate::keymap::Keymap::global(cx).keys_for(id).to_vec();
        if !current.contains(&keys) {
            current.push(keys);
            self.set_action_keys(id, current, window, cx);
        }
    }

    pub(crate) fn remove_action_key(
        &mut self,
        id: &'static str,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let keys: Vec<String> = crate::keymap::Keymap::global(cx)
            .keys_for(id)
            .iter()
            .filter(|k| k.as_str() != key)
            .cloned()
            .collect();
        self.set_action_keys(id, keys, window, cx);
    }

    /// `/`: the sidebar's project filter (showing the sidebar if hidden).
    pub(crate) fn focus_project_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.sidebar_visible {
            self.toggle_sidebar(cx);
        }
        self.sidebar_filter
            .update(cx, |input, cx| input.focus(window, cx));
    }

    /// `?`: the Files tab and its filter.
    pub(crate) fn focus_file_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.active_project().is_none() {
            return;
        }
        self.set_tab(Tab::Files, cx);
        self.files_filter
            .update(cx, |input, cx| input.focus(window, cx));
    }

    /// Escape in a filter box: clear it, or when it is already empty, leave it so the app's
    /// keys work again.
    pub(crate) fn escape_filter(
        &mut self,
        input: Entity<gpui_kit::component::input::InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if input.read(cx).value().is_empty() {
            self.focus_handle.focus(window, cx);
        } else {
            input.update(cx, |input, cx| input.clean(window, cx));
        }
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
            .on_action(cx.listener(|this, _: &FocusProjectSearch, window, cx| {
                this.focus_project_search(window, cx)
            }))
            .on_action(cx.listener(|this, _: &FocusFileSearch, window, cx| {
                this.focus_file_search(window, cx)
            }))
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
                            this.child(self.render_sidebar(window, cx))
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

        // ----- simulated input (see `test_ids.rs` for the id convention) -----

        fn visual(&self, cx: &TestAppContext) -> gpui::VisualTestContext {
            gpui::VisualTestContext::from_window(self.window, cx)
        }

        /// Bounds of the element with test id `id` in the last rendered frame.
        fn bounds(&self, cx: &mut TestAppContext, id: &str) -> Option<gpui::Bounds<gpui::Pixels>> {
            // gpui keys the map by `&'static str`; leaking a few test strings is fine.
            let id: &'static str = Box::leak(id.to_string().into_boxed_str());
            self.visual(cx).debug_bounds(id)
        }

        /// Click the centre of the element with test id `id`, as a user would.
        fn click(&self, cx: &mut TestAppContext, id: &str) {
            self.click_with(cx, id, gpui::Modifiers::none());
        }

        fn click_with(&self, cx: &mut TestAppContext, id: &str, modifiers: gpui::Modifiers) {
            let bounds = self
                .bounds(cx, id)
                .unwrap_or_else(|| panic!("no element with test id {id:?} in the last frame"));
            self.visual(cx).simulate_click(bounds.center(), modifiers);
            cx.run_until_parked();
        }

        /// Right-click the centre of the element with test id `id` (opens context menus).
        fn right_click(&self, cx: &mut TestAppContext, id: &str) {
            let bounds = self
                .bounds(cx, id)
                .unwrap_or_else(|| panic!("no element with test id {id:?} in the last frame"));
            let mut v = self.visual(cx);
            let at = bounds.center();
            v.simulate_mouse_move(at, None, gpui::Modifiers::none());
            v.simulate_mouse_down(at, gpui::MouseButton::Right, gpui::Modifiers::none());
            v.simulate_mouse_up(at, gpui::MouseButton::Right, gpui::Modifiers::none());
            cx.run_until_parked();
        }

        /// Keystrokes in gpui notation, space separated: `"ctrl-, escape"`.
        fn keys(&self, cx: &mut TestAppContext, keys: &str) {
            cx.simulate_keystrokes(self.window, keys);
            cx.run_until_parked();
        }

        /// Text typed into whatever has focus.
        fn type_text(&self, cx: &mut TestAppContext, text: &str) {
            cx.simulate_input(self.window, text);
            cx.run_until_parked();
        }

        fn has_settings(&self, cx: &mut TestAppContext) -> bool {
            use gpui_kit::component::WindowExt as _;
            cx.update_window(self.window, |_, window, cx| window.has_active_dialog(cx))
                .expect("window")
        }
    }

    // ----- status of projects that are not open -----

    /// A repository with an upstream, `ahead` local commits and optionally a dirty tree.
    fn repo_with_upstream(ahead: usize, dirty: bool) -> TempRepo {
        let t = TempRepo::new();
        t.commit_file("a.txt", "a\n", "first");
        t.add_bare_origin();
        t.git(["push", "-q", "-u", "origin", "HEAD"]);
        for n in 0..ahead {
            t.commit_file("a.txt", &format!("{n}\n"), &format!("local {n}"));
        }
        if dirty {
            t.write("new.txt", "x\n");
        }
        t
    }

    fn set_recent_mode(
        h: &Harness,
        cx: &mut TestAppContext,
        mode: osier_config::settings::RecentStatusMode,
    ) {
        h.act(cx, |osier, _, cx| {
            osier.settings.projects.recent_status = mode;
            osier.apply_recent_status_mode(cx);
        });
    }

    /// Run the tick-loop step until the queue is empty (the tick loop's timer does not run in
    /// tests).
    fn drain_recent(h: &Harness, cx: &mut TestAppContext) {
        for _ in 0..100 {
            h.act(cx, |osier, _, cx| osier.pump_recent_status(cx));
            cx.run_until_parked();
            let idle = h.read(cx, |o, _| {
                o.recent_status_queue_len() == 0 && !o.recent_status_busy()
            });
            if idle {
                h.act(cx, |osier, _, cx| osier.pump_recent_status(cx));
                return;
            }
        }
        panic!("recent status queue did not drain");
    }

    #[gpui::test]
    fn recent_status_never_does_nothing(cx: &mut TestAppContext) {
        let repo = repo_with_upstream(2, true);
        let other = repo_with_upstream(1, false);
        let h = Harness::open(cx, &repo);
        h.act(cx, |osier, _, _| {
            osier.state.recent.push(other.path().to_path_buf())
        });
        drain_recent(&h, cx);
        h.read(cx, |osier, _| {
            assert!(osier.recent_badge(other.path()).is_none());
            assert!(osier.recent_badge(repo.path()).is_none());
            let cache = osier_config::status_cache::StatusCache::file(&osier.paths);
            assert!(!cache.exists(), "nothing written in never mode");
        });
    }

    #[gpui::test]
    fn recent_status_cached_keeps_values_from_the_last_open(cx: &mut TestAppContext) {
        use osier_config::settings::RecentStatusMode;
        let repo = repo_with_upstream(2, true);
        let h = Harness::open(cx, &repo);
        set_recent_mode(&h, cx, RecentStatusMode::Cached);
        h.act(cx, |osier, window, cx| osier.close_project(0, window, cx));
        drain_recent(&h, cx);
        h.read(cx, |osier, _| {
            let b = osier.recent_badge(repo.path()).expect("cached badge");
            assert_eq!((b.dirty, b.ahead, b.behind), (Some(true), 2, 0));
            let cache = osier_config::status_cache::StatusCache::load(&osier.paths);
            assert_eq!(
                cache.get(repo.path()).map(|b| b.ahead),
                Some(2),
                "written to disk"
            );
            assert_eq!(
                osier.recent_status_queue_len(),
                0,
                "cached mode runs no git"
            );
        });
        // Switching to never hides everything again.
        set_recent_mode(&h, cx, RecentStatusMode::Never);
        h.read(cx, |osier, _| {
            assert!(osier.recent_badge(repo.path()).is_none())
        });
    }

    #[gpui::test]
    fn recent_status_check_mode_reads_projects_that_were_never_opened(cx: &mut TestAppContext) {
        use osier_config::settings::RecentStatusMode;
        let repo = repo_with_upstream(0, false);
        let other = repo_with_upstream(3, true);
        let h = Harness::open(cx, &repo);
        h.act(cx, |osier, _, _| {
            osier.state.recent.push(other.path().to_path_buf())
        });
        set_recent_mode(&h, cx, RecentStatusMode::Check);
        drain_recent(&h, cx);
        h.read(cx, |osier, _| {
            let b = osier.recent_badge(other.path()).expect("checked");
            assert_eq!((b.dirty, b.ahead, b.behind), (Some(true), 3, 0));
        });
        // Focus regained: everything is queued again.
        h.act(cx, |osier, _, _| osier.recent_status_on_focus());
        h.read(cx, |osier, _| assert!(osier.recent_status_queue_len() >= 1));
    }

    #[gpui::test]
    fn scan_now_follows_the_mode(cx: &mut TestAppContext) {
        use osier_config::settings::RecentStatusMode;
        let repo = repo_with_upstream(0, false);
        let a = repo_with_upstream(1, true);
        let b = repo_with_upstream(0, false);
        let h = Harness::open(cx, &repo);
        h.act(cx, |osier, _, _| {
            osier.state.recent.push(a.path().to_path_buf());
            osier.state.recent.push(b.path().to_path_buf());
        });
        // Never: nothing queued, nothing shown.
        h.act(cx, |osier, window, cx| osier.scan_recent_now(window, cx));
        h.read(cx, |o, _| assert_eq!(o.recent_status_queue_len(), 0));
        // Cached: a one-off full check, so the change marker becomes known.
        set_recent_mode(&h, cx, RecentStatusMode::Cached);
        h.act(cx, |osier, window, cx| osier.scan_recent_now(window, cx));
        drain_recent(&h, cx);
        h.read(cx, |o, _| {
            let badge = o.recent_badge(a.path()).expect("scanned");
            assert_eq!((badge.dirty, badge.ahead), (Some(true), 1));
            assert!(o.recent_badge(b.path()).is_some());
        });
        // Watch: refs only; the change marker from the earlier full check is kept.
        set_recent_mode(&h, cx, RecentStatusMode::Watch);
        drain_recent(&h, cx);
        a.git(["commit", "-q", "--allow-empty", "-m", "more"]);
        h.act(cx, |osier, window, cx| osier.scan_recent_now(window, cx));
        drain_recent(&h, cx);
        h.read(cx, |o, _| {
            let badge = o.recent_badge(a.path()).unwrap();
            assert_eq!(badge.ahead, 2);
            assert_eq!(
                badge.dirty,
                Some(true),
                "refs-only scan keeps the last marker"
            );
        });
    }

    #[gpui::test]
    fn failed_background_checks_reach_the_command_log(cx: &mut TestAppContext) {
        use osier_config::settings::RecentStatusMode;
        let repo = repo_with_upstream(0, false);
        let healthy = repo_with_upstream(1, false);
        // A repository whose .git points nowhere: git fails on it.
        let broken = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            broken.path().join(".git"),
            "gitdir: /nonexistent/osier-test\n",
        )
        .expect("write .git");
        let h = Harness::open(cx, &repo);
        let before = h.read(cx, |o, _| o.sink.records().len());
        h.act(cx, |osier, _, _| {
            osier.state.recent.push(healthy.path().to_path_buf());
            osier.state.recent.push(broken.path().to_path_buf());
        });
        set_recent_mode(&h, cx, RecentStatusMode::Check);
        drain_recent(&h, cx);
        h.read(cx, |o, _| {
            let new: Vec<_> = o.sink.records().into_iter().skip(before).collect();
            assert!(!new.is_empty(), "the broken check is logged");
            assert!(new.iter().all(|r| !r.expected), "only failures are logged");
            assert!(
                new.iter().all(|r| r.cwd.as_deref() == Some(broken.path())),
                "the healthy repository's checks stay out of the log"
            );
            assert!(o.recent_badge(healthy.path()).is_some());
        });
    }

    #[gpui::test]
    fn recent_status_watch_mode_follows_commits_without_a_change_marker(cx: &mut TestAppContext) {
        use osier_config::settings::RecentStatusMode;
        let repo = repo_with_upstream(0, false);
        let other = repo_with_upstream(1, true);
        let h = Harness::open(cx, &repo);
        h.act(cx, |osier, _, _| {
            osier.state.recent.push(other.path().to_path_buf())
        });
        set_recent_mode(&h, cx, RecentStatusMode::Watch);
        drain_recent(&h, cx);
        h.read(cx, |osier, _| {
            let b = osier.recent_badge(other.path()).expect("checked");
            assert_eq!((b.ahead, b.dirty), (1, None), "refs only in watch mode");
            assert!(!osier.recent_status.shows_changes());
        });
        // A commit made by another tool reaches us through the .git watcher.
        other.git(["add", "new.txt"]);
        other.git(["commit", "-qm", "outside"]);
        let mut ahead = 1;
        for _ in 0..50 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            drain_recent(&h, cx);
            ahead = h.read(cx, |o, _| {
                o.recent_badge(other.path()).map(|b| b.ahead).unwrap_or(0)
            });
            if ahead == 2 {
                break;
            }
        }
        assert_eq!(ahead, 2, "the watcher saw the commit");
    }

    // ----- tests driven only by clicks, keystrokes and typing -----

    #[gpui::test]
    fn ui_settings_open_by_click_and_close_by_escape(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "one\n", "first");
        let h = Harness::open(cx, &repo);
        h.click(cx, "settings");
        assert!(h.has_settings(cx), "the gear opens settings");
        assert!(
            h.bounds(cx, "settings-dialog").is_some(),
            "dialog content is drawn"
        );
        h.keys(cx, "escape");
        assert!(
            !h.has_settings(cx),
            "escape reaches the dialog and closes it"
        );
        h.keys(cx, "ctrl-,");
        assert!(h.has_settings(cx), "the keyboard shortcut opens it too");
        h.keys(cx, "escape");
        assert!(!h.has_settings(cx));
    }

    #[gpui::test]
    fn ui_stage_with_the_checkbox_and_commit_from_the_keyboard(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "one\n", "first");
        repo.write("a.txt", "two\n");
        let h = Harness::open(cx, &repo);
        h.click(cx, "change-stage:unstaged:a.txt");
        assert_eq!(
            repo.git(["diff", "--cached", "--name-only"]).trim(),
            "a.txt"
        );
        assert!(
            h.bounds(cx, "change-row:staged:a.txt").is_some(),
            "the row moved to Staged"
        );
        h.click(cx, "commit-message");
        h.type_text(cx, "Typed in the commit box");
        h.keys(cx, "ctrl-enter");
        assert_eq!(
            repo.git(["log", "-1", "--format=%s"]).trim(),
            "Typed in the commit box"
        );
        assert!(repo.git(["status", "--porcelain"]).trim().is_empty());
    }

    #[gpui::test]
    fn ui_list_keys_select_and_stage(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a\n", "first");
        repo.commit_file("b.txt", "b\n", "second");
        repo.write("a.txt", "A\n");
        repo.write("b.txt", "B\n");
        let h = Harness::open(cx, &repo);
        h.click(cx, "change-row:unstaged:a.txt");
        h.keys(cx, "down");
        h.read(cx, |osier, _| {
            assert_eq!(
                osier.active_project().unwrap().selection,
                Selection::File {
                    path: "b.txt".into(),
                    area: Area::Unstaged
                }
            )
        });
        h.keys(cx, "space");
        assert_eq!(
            repo.git(["diff", "--cached", "--name-only"]).trim(),
            "b.txt"
        );
        h.keys(cx, "j");
        h.keys(cx, "delete");
        assert!(
            cx.update_window(h.window, |_, window, cx| {
                use gpui_kit::component::WindowExt as _;
                window.has_active_dialog(cx)
            })
            .unwrap(),
            "delete asks first"
        );
        h.keys(cx, "escape");
        assert_eq!(repo.read("a.txt"), "A\n");
    }

    fn input_text(
        h: &Harness,
        cx: &mut TestAppContext,
        pick: fn(&Osier) -> &Entity<gpui_kit::component::input::InputState>,
    ) -> String {
        h.read(cx, |o, cx| pick(o).read(cx).value().to_string())
    }

    fn focused_is(
        h: &Harness,
        cx: &mut TestAppContext,
        pick: fn(&Osier, &gpui::App) -> gpui::FocusHandle,
    ) -> bool {
        let view = h.view.clone();
        cx.update_window(h.window, |_, window, cx| {
            let handle = pick(view.read(cx), cx);
            handle.is_focused(window)
        })
        .expect("window")
    }

    #[gpui::test]
    fn ui_slash_searches_projects_and_escape_clears(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a\n", "first");
        let h = Harness::open(cx, &repo);
        h.keys(cx, "/");
        h.type_text(cx, "lan");
        assert_eq!(
            input_text(&h, cx, |o| &o.sidebar_filter),
            "lan",
            "no stray slash"
        );
        h.keys(cx, "escape");
        assert_eq!(
            input_text(&h, cx, |o| &o.sidebar_filter),
            "",
            "escape clears"
        );
        assert!(focused_is(&h, cx, |o, cx| {
            gpui_kit::Focusable::focus_handle(o.sidebar_filter.read(cx), cx)
        }));
        h.keys(cx, "escape");
        assert!(
            focused_is(&h, cx, |o, _| o.focus_handle.clone()),
            "second escape leaves the box"
        );
        // The shortcut works again afterwards.
        h.keys(cx, "/");
        h.type_text(cx, "x");
        assert_eq!(input_text(&h, cx, |o| &o.sidebar_filter), "x");
    }

    #[gpui::test]
    fn ui_filter_suggestions_insert_is_terms_and_filter_by_state(cx: &mut TestAppContext) {
        use osier_config::settings::RecentStatusMode;
        let name = |t: &TempRepo| {
            t.path()
                .file_name()
                .expect("name")
                .to_string_lossy()
                .into_owned()
        };
        let ahead = repo_with_upstream(1, false);
        let clean = repo_with_upstream(0, false);
        let recent_ahead = repo_with_upstream(2, false);
        let h = Harness::open(cx, &ahead);
        h.act(cx, |osier, _, cx| {
            osier.open_project(clean.path(), false, cx).expect("open");
            osier.state.recent.push(recent_ahead.path().to_path_buf());
        });
        let shown = |h: &Harness, cx: &mut TestAppContext, id: String| h.bounds(cx, &id).is_some();

        // Focus shows every qualifier, plus the note while recent status is off.
        h.keys(cx, "/");
        assert!(shown(&h, cx, "filter-suggestions".into()));
        for term in ["is:changed", "is:ahead", "is:behind"] {
            assert!(shown(&h, cx, format!("filter-suggestion:{term}")), "{term}");
        }
        assert!(shown(&h, cx, "filter-open-settings".into()));

        // Clicking a suggestion inserts it, keeps focus, and filters open projects by state.
        h.click(cx, "filter-suggestion:is:ahead");
        assert_eq!(input_text(&h, cx, |o| &o.sidebar_filter), "is:ahead ");
        assert!(focused_is(&h, cx, |o, cx| {
            gpui_kit::Focusable::focus_handle(o.sidebar_filter.read(cx), cx)
        }));
        assert!(shown(&h, cx, format!("sidebar-open:{}", name(&ahead))));
        assert!(!shown(&h, cx, format!("sidebar-open:{}", name(&clean))));
        assert!(
            !shown(&h, cx, format!("sidebar-recent:{}", name(&recent_ahead))),
            "recent projects are unknown while recent status is off"
        );
        assert!(
            !shown(&h, cx, "filter-suggestion:is:ahead".into()),
            "used terms drop out"
        );

        // The note's link opens settings on the right page, even after a normal open.
        h.keys(cx, "escape escape ctrl-,");
        assert!(h.has_settings(cx));
        assert!(
            !shown(&h, cx, "scan-recent-now".into()),
            "normal open starts elsewhere"
        );
        h.keys(cx, "escape /");
        h.click(cx, "filter-open-settings");
        assert!(h.has_settings(cx));
        assert!(
            shown(&h, cx, "scan-recent-now".into()),
            "settings open on the Projects page"
        );
        assert_eq!(
            h.read(cx, |o, _| o.settings_highlight.map(|(t, _)| t)),
            Some(crate::settings_panel::SettingsTarget::RECENT_STATUS)
        );
        h.keys(cx, "escape");
        h.keys(cx, "/");
        h.type_text(cx, "is:ahead ");

        // With status checks on, recent projects match by their badges and the note goes.
        set_recent_mode(&h, cx, RecentStatusMode::Check);
        drain_recent(&h, cx);
        assert!(shown(
            &h,
            cx,
            format!("sidebar-recent:{}", name(&recent_ahead))
        ));
        assert!(!shown(&h, cx, "filter-open-settings".into()));

        // Enter completes a half-typed qualifier; is:behind matches nothing here.
        h.type_text(cx, "is:b");
        h.keys(cx, "enter");
        assert_eq!(
            input_text(&h, cx, |o| &o.sidebar_filter),
            "is:ahead is:behind "
        );
        assert!(!shown(&h, cx, format!("sidebar-open:{}", name(&ahead))));

        // Leaving the box hides the dropdown.
        h.keys(cx, "escape escape");
        assert!(!shown(&h, cx, "filter-suggestions".into()));
    }

    #[gpui::test]
    fn ui_changes_tab_shows_the_number_of_changes(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a\n", "first");
        let clean = Harness::open(cx, &repo);
        assert!(
            clean.bounds(cx, "tab-changes-count").is_none(),
            "no badge when clean"
        );

        let dirty = TempRepo::new();
        dirty.commit_file("a.txt", "a\n", "first");
        dirty.write("a.txt", "b\n");
        dirty.write("new.txt", "x\n");
        let h = Harness::open(cx, &dirty);
        assert!(h.bounds(cx, "tab-changes-count").is_some());
        h.read(cx, |o, _| {
            let n = o
                .active_project()
                .and_then(|p| p.snapshot.as_ref())
                .map(|s| s.status.entries.len());
            assert_eq!(n, Some(2));
        });
    }

    #[gpui::test]
    fn open_on_github_uses_the_github_remote(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a\n", "first");
        repo.git(["remote", "add", "origin", "git@github.com:octo/repo.git"]);
        let other = TempRepo::new();
        other.commit_file("b.txt", "b\n", "first");
        other.git([
            "remote",
            "add",
            "origin",
            "https://github.com/octo/other.git",
        ]);
        let h = Harness::open(cx, &repo);

        let hash = h.read(cx, |o, _| {
            o.active_project()
                .and_then(|p| p.snapshot.as_ref())
                .expect("snapshot")
                .commits[0]
                .hash
                .clone()
        });
        h.act(cx, |o, window, cx| {
            o.open_commit_on_github(hash.clone(), window, cx)
        });
        assert_eq!(
            cx.opened_url(),
            Some(format!("https://github.com/octo/repo/commit/{hash}"))
        );

        // A project that is not open reads its remotes first.
        let path = other.path().to_path_buf();
        h.act(cx, |o, window, cx| {
            o.open_project_on_github(&path, window, cx)
        });
        assert_eq!(
            cx.opened_url().as_deref(),
            Some("https://github.com/octo/other")
        );
    }

    #[gpui::test]
    fn ui_question_mark_searches_files(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a\n", "first");
        for keys in ["?", "shift-/"] {
            let h = Harness::open(cx, &repo);
            h.keys(cx, keys);
            h.read(cx, |o, _| assert_eq!(o.tab, super::Tab::Files, "{keys}"));
            h.type_text(cx, "a.t");
            assert_eq!(input_text(&h, cx, |o| &o.files_filter), "a.t", "{keys}");
            h.keys(cx, "escape");
            assert_eq!(input_text(&h, cx, |o| &o.files_filter), "");
        }
    }

    #[gpui::test]
    fn ui_plain_keys_type_into_text_fields(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a\n", "first");
        repo.commit_file("b.txt", "b\n", "second");
        repo.write("a.txt", "A\n");
        repo.write("b.txt", "B\n");
        let h = Harness::open(cx, &repo);
        let selection = h.read(cx, |o, _| o.active_project().unwrap().selection.clone());
        h.click(cx, "commit-message");
        h.keys(cx, "j k space / ?");
        let text = h.read(cx, |o, cx| o.commit_message_text(cx));
        assert_eq!(text, "jk /?", "every key typed as text");
        h.read(cx, |o, _| {
            assert_eq!(
                o.active_project().unwrap().selection,
                selection,
                "j/k did not move"
            );
            assert_eq!(o.tab, super::Tab::Changes, "? did not switch tabs");
        });
        assert_eq!(
            repo.git(["diff", "--cached", "--name-only"]).trim(),
            "",
            "space did not stage"
        );
        assert_eq!(input_text(&h, cx, |o| &o.sidebar_filter), "");
    }

    #[gpui::test]
    fn ui_slash_in_settings_searches_settings(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a\n", "first");
        let h = Harness::open(cx, &repo);
        h.keys(cx, "ctrl-,");
        assert!(h.has_settings(cx));
        h.keys(cx, "/");
        let (inside, on_container) = {
            let view = h.view.clone();
            cx.update_window(h.window, |_, window, cx| {
                let f = view.read(cx).settings_focus.clone();
                (f.contains_focused(window, cx), f.is_focused(window))
            })
            .expect("window")
        };
        assert!(
            inside && !on_container,
            "focus moved into the settings search box"
        );
        assert!(!focused_is(&h, cx, |o, cx| {
            gpui_kit::Focusable::focus_handle(o.sidebar_filter.read(cx), cx)
        }));
        h.type_text(cx, "diff");
        assert!(
            h.has_settings(cx),
            "typing in the search box keeps settings open"
        );
        assert_eq!(
            input_text(&h, cx, |o| &o.sidebar_filter),
            "",
            "nothing reached the sidebar"
        );
    }

    #[gpui::test]
    fn keymap_overrides_apply_live_and_keep_input_keys(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a\n", "first");
        let h = Harness::open(cx, &repo);
        let file = h.read(cx, |o, _| o.paths.keymap_file());
        std::fs::create_dir_all(file.parent().expect("dir")).expect("mkdir");
        std::fs::write(
            &file,
            "[bindings]\nfocus_project_search = [\"ctrl-f\"]\ntab_log = []\nrefresh = [\"ctrl-??\"]\nnope = [\"x\"]\n",
        )
        .expect("write keymap");
        let mut reported = Vec::new();
        h.act(cx, |osier, _, cx| reported = osier.load_keymap(cx));
        assert!(
            reported
                .iter()
                .any(|p| p.contains("unknown action \"nope\"")),
            "{reported:?}"
        );
        assert!(
            reported.iter().any(|p| p.contains("ctrl-??")),
            "{reported:?}"
        );
        h.read(cx, |_, cx| {
            let k = crate::keymap::Keymap::global(cx);
            assert_eq!(k.keys_for("focus_project_search"), ["ctrl-f"]);
            assert!(k.keys_for("tab_log").is_empty(), "unbound");
            assert!(k.keys_for("refresh").is_empty(), "invalid key dropped");
            assert_eq!(k.keys_for("tab_files"), ["ctrl-5"], "defaults untouched");
        });
        // The old default no longer fires; the new key does.
        h.keys(cx, "/");
        assert_eq!(input_text(&h, cx, |o| &o.sidebar_filter), "");
        assert!(!focused_is(&h, cx, |o, cx| {
            gpui_kit::Focusable::focus_handle(o.sidebar_filter.read(cx), cx)
        }));
        h.keys(cx, "ctrl-f");
        h.type_text(cx, "abc");
        assert_eq!(input_text(&h, cx, |o| &o.sidebar_filter), "abc");
        // gpui-kit's own bindings survived the reload: backspace edits, escape clears.
        h.keys(cx, "backspace");
        assert_eq!(input_text(&h, cx, |o| &o.sidebar_filter), "ab");
        h.keys(cx, "escape");
        assert_eq!(input_text(&h, cx, |o| &o.sidebar_filter), "");
        h.keys(cx, "ctrl-2");
        h.read(cx, |o, _| {
            assert_eq!(o.tab, super::Tab::Changes, "unbound action stays unbound")
        });
    }

    #[gpui::test]
    fn ui_keymap_page_records_removes_and_resets(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a\n", "first");
        let h = Harness::open(cx, &repo);
        let keys_for = |h: &Harness, cx: &mut TestAppContext, id: &str| {
            let id = id.to_string();
            h.read(cx, move |_, cx| {
                crate::keymap::Keymap::global(cx).keys_for(&id).to_vec()
            })
        };
        // Find Refresh through the settings search, then record a new key with the +.
        h.keys(cx, "ctrl-,");
        h.keys(cx, "/");
        h.type_text(cx, "Refresh");
        h.click(cx, "keymap-add:refresh");
        h.read(cx, |o, _| assert!(o.keymap_recording.is_some()));
        h.keys(cx, "ctrl-shift-r");
        assert_eq!(
            keys_for(&h, cx, "refresh"),
            ["ctrl-r", "f5", "ctrl-shift-r"]
        );
        let file = h.read(cx, |o, _| o.paths.keymap_file());
        let text = std::fs::read_to_string(&file).expect("keymap.toml written");
        assert!(text.contains("refresh = ["), "{text}");
        // Remove one key with its x.
        h.click(cx, "keymap-remove:refresh:f5");
        assert_eq!(keys_for(&h, cx, "refresh"), ["ctrl-r", "ctrl-shift-r"]);
        // Reset brings back the defaults and the file goes away (nothing overridden).
        h.click(cx, "keymap-reset:refresh");
        assert_eq!(keys_for(&h, cx, "refresh"), ["ctrl-r", "f5"]);
        assert!(!file.exists());
    }

    #[gpui::test]
    fn recording_captures_keys_instead_of_running_them(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a\n", "first");
        let h = Harness::open(cx, &repo);
        h.act(cx, |o, _, cx| o.start_key_recording("tab_log", cx));
        h.keys(cx, "ctrl-3");
        h.read(cx, |o, cx| {
            assert_eq!(o.tab, super::Tab::Changes, "ctrl-3 was captured, not run");
            let k = crate::keymap::Keymap::global(cx);
            assert_eq!(k.keys_for("tab_log"), ["ctrl-2", "ctrl-3"]);
            assert_eq!(
                crate::keymap::conflicts(k, "tab_log", "ctrl-3"),
                ["Branches tab"]
            );
            assert!(o.keymap_recording.is_none());
        });
        // A shifted symbol is stored as the symbol; escape cancels without changes.
        h.act(cx, |o, _, cx| {
            o.start_key_recording("focus_project_search", cx)
        });
        h.keys(cx, "?");
        h.act(cx, |o, _, cx| {
            o.start_key_recording("focus_project_search", cx)
        });
        h.keys(cx, "escape");
        h.read(cx, |o, cx| {
            assert_eq!(
                crate::keymap::Keymap::global(cx).keys_for("focus_project_search"),
                ["/", "?"]
            );
            assert!(o.keymap_recording.is_none());
        });
    }

    #[gpui::test]
    fn keymap_changes_update_the_dms_cheatsheet(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a\n", "first");
        let h = Harness::open(cx, &repo);
        let dir = tempfile::tempdir().expect("tempdir");
        let dms = dir.path().join("dms");
        std::fs::create_dir(&dms).expect("dms dir");
        h.act(cx, |o, window, cx| {
            o.dms_dir = Some(dms.clone());
            o.set_action_keys("quit", vec!["ctrl-shift-q".into()], window, cx);
        });
        let sheet = std::fs::read_to_string(dms.join("cheatsheets/osier.json")).expect("written");
        assert!(sheet.contains("\"Ctrl+Shift+Q\""), "{sheet}");
        // An outside edit of keymap.toml is picked up and reaches DMS too.
        let file = h.read(cx, |o, _| o.paths.keymap_file());
        std::fs::write(&file, "[bindings]\nquit = [\"ctrl-alt-q\"]\n").expect("edit");
        h.act(cx, |o, window, cx| o.reload_keymap(window, cx));
        let sheet = std::fs::read_to_string(dms.join("cheatsheets/osier.json")).expect("written");
        assert!(sheet.contains("\"Ctrl+Alt+Q\""), "{sheet}");
        h.read(cx, |_, cx| {
            assert_eq!(
                crate::keymap::Keymap::global(cx).keys_for("quit"),
                ["ctrl-alt-q"]
            )
        });
    }

    #[gpui::test]
    fn ui_command_log_rows_copy_from_the_context_menu(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a\n", "first");
        let h = Harness::open(cx, &repo);
        h.act(cx, |osier, _, cx| {
            if !osier.command_log_visible {
                osier.toggle_command_log(cx);
            }
        });
        let rec = h.read(cx, |o, _| {
            o.sink.records().last().cloned().expect("a record")
        });
        let row = format!("command-row:{}", rec.id);

        h.right_click(cx, &row);
        h.keys(cx, "down enter");
        let copied = cx.read_from_clipboard().and_then(|c| c.text());
        assert_eq!(copied.as_deref(), Some(rec.as_shell().as_str()));

        h.right_click(cx, &row);
        h.keys(cx, "down down enter");
        let copied = cx
            .read_from_clipboard()
            .and_then(|c| c.text())
            .unwrap_or_default();
        assert_eq!(copied, crate::views::command_log::command_details(&rec));
        assert!(copied.starts_with("$ git "), "{copied}");
        assert!(copied.contains("\nexit: "), "{copied}");
        assert!(copied.contains("\nstdout: "), "{copied}");
    }

    #[gpui::test]
    fn ui_tabs_and_command_palette(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "one\n", "first");
        let h = Harness::open(cx, &repo);
        h.click(cx, "tab-log");
        h.read(cx, |osier, _| assert_eq!(osier.tab, super::Tab::Log));
        h.keys(cx, "f1");
        h.type_text(cx, "Patches tab");
        h.keys(cx, "enter");
        h.read(cx, |osier, _| assert_eq!(osier.tab, super::Tab::Patches));
        h.keys(cx, "ctrl-1");
        h.read(cx, |osier, _| assert_eq!(osier.tab, super::Tab::Changes));
        h.keys(cx, "ctrl-p");
        h.type_text(cx, "Log tab");
        h.keys(cx, "enter");
        h.read(cx, |osier, _| assert_eq!(osier.tab, super::Tab::Log));
    }

    fn status_has(h: &Harness, cx: &mut TestAppContext, line: &str) -> bool {
        h.read(cx, |o, _| {
            o.active_project()
                .and_then(|p| p.snapshot.as_ref())
                .is_some_and(|s| s.status.entries.iter().any(|e| e.path == line))
        })
    }

    #[gpui::test]
    fn ignoring_an_untracked_file_takes_effect_at_once_and_twice_is_a_no_op(
        cx: &mut TestAppContext,
    ) {
        use crate::ignore_ops::{PatternKind, RowState};
        use osier_git::ops::ignore::IgnoreFile;
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a\n", "first");
        repo.write("debug.log", "x\n");
        let h = Harness::open(cx, &repo);
        assert!(status_has(&h, cx, "debug.log"));
        let row = RowState {
            is_dir: false,
            tracked: false,
        };
        h.act(cx, |o, w, cx| {
            o.ignore_path(
                "debug.log".into(),
                row,
                PatternKind::Exact,
                IgnoreFile::Gitignore,
                w,
                cx,
            )
        });
        let gitignore =
            || std::fs::read_to_string(repo.path().join(".gitignore")).unwrap_or_default();
        assert_eq!(gitignore(), "/debug.log\n");
        assert!(
            !status_has(&h, cx, "debug.log"),
            "gone from Changes straight away"
        );
        assert!(!h.has_settings(cx), "no dialog for an untracked file");

        // Again: already ignored, nothing written, no dialog.
        h.act(cx, |o, w, cx| {
            o.ignore_path(
                "debug.log".into(),
                row,
                PatternKind::Exact,
                IgnoreFile::Gitignore,
                w,
                cx,
            )
        });
        assert_eq!(gitignore(), "/debug.log\n");
        assert!(!h.has_settings(cx));
    }

    #[gpui::test]
    fn ignoring_places_the_pattern_with_related_lines(cx: &mut TestAppContext) {
        use crate::ignore_ops::{PatternKind, RowState};
        use osier_git::ops::ignore::IgnoreFile;
        let repo = TempRepo::new();
        repo.commit_file(
            ".gitignore",
            "# local binaries\nlocal-bin/.local/bin/zed\nlocal-bin/.local/bin/btop\n\n# other\n*.log\n",
            "ignore",
        );
        repo.write("local-bin/.local/bin/new-app", "x\n");
        let h = Harness::open(cx, &repo);
        let row = RowState {
            is_dir: false,
            tracked: false,
        };
        h.act(cx, |o, w, cx| {
            o.ignore_path(
                "local-bin/.local/bin/new-app".into(),
                row,
                PatternKind::Exact,
                IgnoreFile::Gitignore,
                w,
                cx,
            )
        });
        let text = std::fs::read_to_string(repo.path().join(".gitignore")).unwrap();
        assert_eq!(
            text,
            "# local binaries\nlocal-bin/.local/bin/zed\nlocal-bin/.local/bin/btop\n\
             local-bin/.local/bin/new-app\n\n# other\n*.log\n"
        );
        assert!(!status_has(&h, cx, "local-bin/.local/bin/new-app"));
    }

    #[gpui::test]
    fn ignoring_a_tracked_file_asks_and_untracks_by_default(cx: &mut TestAppContext) {
        use crate::ignore_ops::{PatternKind, RowState};
        use osier_git::ops::ignore::IgnoreFile;
        let repo = TempRepo::new();
        repo.commit_file("config/local.php", "secret\n", "tracked");
        repo.commit_file("keep.txt", "k\n", "tracked too");
        let h = Harness::open(cx, &repo);
        let row = RowState {
            is_dir: false,
            tracked: true,
        };
        let tracked = |path: &str| !repo.git(["ls-files", "--", path]).trim().is_empty();

        // Enter accepts with the checkbox on: ignored and untracked, the file stays.
        h.act(cx, |o, w, cx| {
            o.ignore_path(
                "config/local.php".into(),
                row,
                PatternKind::Exact,
                IgnoreFile::Gitignore,
                w,
                cx,
            )
        });
        assert!(h.has_settings(cx), "a dialog for a tracked file");
        assert!(h.bounds(cx, "tracked-dialog-check").is_some());
        h.keys(cx, "enter");
        assert!(!h.has_settings(cx));
        assert!(!tracked("config/local.php"));
        assert!(repo.path().join("config/local.php").exists());
        let gitignore = std::fs::read_to_string(repo.path().join(".gitignore")).unwrap();
        assert!(gitignore.contains("/config/local.php"));

        // Escape cancels.
        h.act(cx, |o, w, cx| {
            o.ignore_path(
                "keep.txt".into(),
                row,
                PatternKind::Exact,
                IgnoreFile::Gitignore,
                w,
                cx,
            )
        });
        assert!(h.has_settings(cx));
        h.keys(cx, "escape");
        assert!(!h.has_settings(cx));
        // The Cancel button does the same.
        h.act(cx, |o, w, cx| {
            o.ignore_path(
                "keep.txt".into(),
                row,
                PatternKind::Exact,
                IgnoreFile::Gitignore,
                w,
                cx,
            )
        });
        h.click(cx, "dialog-cancel");
        assert!(!h.has_settings(cx), "Cancel closes the dialog");
        assert!(tracked("keep.txt"));
        assert!(
            !std::fs::read_to_string(repo.path().join(".gitignore"))
                .unwrap()
                .contains("keep.txt")
        );

        // Unchecking the box only adds the pattern.
        h.act(cx, |o, w, cx| {
            o.ignore_path(
                "keep.txt".into(),
                row,
                PatternKind::Exact,
                IgnoreFile::Gitignore,
                w,
                cx,
            )
        });
        h.click(cx, "tracked-dialog-check");
        h.click(cx, "dialog-ok");
        assert!(!h.has_settings(cx), "the OK button confirms");
        assert!(tracked("keep.txt"), "still tracked");
        assert!(
            std::fs::read_to_string(repo.path().join(".gitignore"))
                .unwrap()
                .contains("/keep.txt")
        );
    }

    #[gpui::test]
    fn ui_add_to_gitignore_from_the_changes_menu(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a\n", "first");
        repo.write("notes.tmp", "x\n");
        let h = Harness::open(cx, &repo);
        h.right_click(cx, "change-row:untracked:notes.tmp");
        // Open in editor, Open in file manager, Copy path, Blame, History, Copy as patch,
        // Save to Patches, Shelve, then Add to .gitignore.
        h.keys(cx, "down down down down down down down down down enter");
        let gitignore = std::fs::read_to_string(repo.path().join(".gitignore")).unwrap_or_default();
        assert_eq!(gitignore, "/notes.tmp\n");
        assert!(!status_has(&h, cx, "notes.tmp"));
    }

    #[gpui::test]
    fn ui_confirm_dialogs_have_working_buttons(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a\n", "first");
        let h = Harness::open(cx, &repo);
        let hits = std::rc::Rc::new(std::cell::Cell::new(0));
        let open =
            |h: &Harness, cx: &mut TestAppContext, hits: std::rc::Rc<std::cell::Cell<i32>>| {
                h.act(cx, move |o, w, cx| {
                    o.confirm(
                        "Title",
                        "Body",
                        "Do it",
                        false,
                        move |_, _, _| hits.set(hits.get() + 1),
                        w,
                        cx,
                    )
                });
            };
        open(&h, cx, hits.clone());
        assert!(h.bounds(cx, "dialog-ok").is_some() && h.bounds(cx, "dialog-cancel").is_some());
        h.click(cx, "dialog-cancel");
        assert!(!h.has_settings(cx));
        assert_eq!(hits.get(), 0);
        open(&h, cx, hits.clone());
        h.click(cx, "dialog-ok");
        assert!(!h.has_settings(cx));
        assert_eq!(hits.get(), 1);
    }

    #[gpui::test]
    fn ui_clicking_the_option_words_toggles_the_checkbox(cx: &mut TestAppContext) {
        use crate::ignore_ops::RowState;
        let repo = TempRepo::new();
        repo.commit_file("notes.txt", "n\n", "tracked");
        let h = Harness::open(cx, &repo);
        let row = RowState {
            is_dir: false,
            tracked: true,
        };
        h.act(cx, |o, w, cx| {
            o.untrack_prompt("notes.txt".into(), row, w, cx)
        });
        h.click(cx, "tracked-dialog-option");
        h.click(cx, "dialog-ok");
        assert!(
            repo.git(["ls-files", "--", "notes.txt"]).trim().is_empty(),
            "untracked"
        );
        assert!(
            !repo.path().join(".gitignore").exists(),
            "the unticked option did not add a pattern"
        );
    }

    #[gpui::test]
    fn untrack_offers_to_ignore_too(cx: &mut TestAppContext) {
        use crate::ignore_ops::RowState;
        let repo = TempRepo::new();
        repo.commit_file("build/out.bin", "b\n", "tracked build output");
        let h = Harness::open(cx, &repo);
        let row = RowState {
            is_dir: true,
            tracked: true,
        };
        h.act(cx, |o, w, cx| o.untrack_prompt("build".into(), row, w, cx));
        assert!(h.has_settings(cx));
        h.keys(cx, "enter");
        assert!(
            repo.git(["ls-files", "--", "build"]).trim().is_empty(),
            "folder untracked"
        );
        assert!(repo.path().join("build/out.bin").exists());
        let gitignore = std::fs::read_to_string(repo.path().join(".gitignore")).unwrap();
        assert_eq!(gitignore, "/build/\n");
        assert!(
            status_has(&h, cx, "build/out.bin"),
            "the staged removal shows at once"
        );
    }

    #[gpui::test]
    fn tooltip_delay_defaults_to_at_once_and_applies_live(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "one\n", "first");
        let h = Harness::open(cx, &repo);
        let delay = |cx: &mut TestAppContext| {
            cx.update(|cx| gpui_kit::base::TooltipDefaults::global(cx).show_delay())
        };
        assert_eq!(delay(cx), std::time::Duration::from_millis(0));
        h.act(cx, |o, _, cx| {
            o.settings.appearance.tooltip_delay_ms = 600;
            o.apply_tooltip_delay(cx);
        });
        assert_eq!(delay(cx), std::time::Duration::from_millis(600));
        h.act(cx, |o, _, cx| {
            o.settings.appearance.tooltip_delay_ms = 99_999;
            o.apply_tooltip_delay(cx);
        });
        assert_eq!(
            delay(cx),
            std::time::Duration::from_millis(u64::from(super::TOOLTIP_DELAY_MAX_MS)),
            "hand-edited config is clamped"
        );
    }

    #[gpui::test]
    fn tab_overflow_is_measured_and_settles(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "one\n", "first");
        let h = Harness::open(cx, &repo);
        // The test layout's centre panel is about 440px: six tabs do not fit, so the strip
        // scrolls and the menu shows. Redrawing settles (a render loop would never park).
        h.act(cx, |_, _, cx| cx.notify());
        h.act(cx, |_, _, cx| cx.notify());
        h.read(cx, |o, _| {
            assert!(o.tab_scroll.bounds().size.width > gpui::px(0.));
            assert!(o.tab_scroll.max_offset().x > gpui::px(0.5));
        });
    }

    #[gpui::test]
    fn ui_right_click_opens_the_row_menu_and_buttons_follow_the_setting(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "one\n", "first");
        repo.git(["branch", "feature"]);
        let h = Harness::open(cx, &repo);
        h.click(cx, "tab-branches");
        let button = "branch-menu:feature";
        // The buttons live in the row's hover group, so hover the row before looking.
        let visible = |h: &Harness, cx: &mut TestAppContext| {
            let row = h.bounds(cx, "branch-row:feature").expect("row");
            h.visual(cx)
                .simulate_mouse_move(row.center(), None, gpui::Modifiers::none());
            cx.run_until_parked();
            h.bounds(cx, button)
                .is_some_and(|b| b.size.width > gpui::px(0.))
        };
        assert!(!visible(&h, cx), "menu buttons are off by default");

        // Right-click, first entry: Checkout (the hover button's action).
        h.right_click(cx, "branch-row:feature");
        h.keys(cx, "down enter");
        h.read(cx, |o, _| {
            let head = o
                .active_project()
                .and_then(|p| p.snapshot.as_ref())
                .and_then(|s| s.status.branch.head.clone());
            assert_eq!(head.as_deref(), Some("feature"));
        });

        h.act(cx, |o, _, cx| {
            o.settings.appearance.row_menu_buttons = true;
            cx.notify();
        });
        assert!(visible(&h, cx), "the setting shows them");
    }

    #[gpui::test]
    fn ui_shift_click_compares_two_commits(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "one\n", "first");
        repo.commit_file("a.txt", "two\n", "second");
        let h = Harness::open(cx, &repo);
        h.click(cx, "tab-log");
        h.click(cx, "log-row:0");
        h.click_with(
            cx,
            "log-row:1",
            gpui::Modifiers {
                shift: true,
                ..Default::default()
            },
        );
        h.read(cx, |osier, _| {
            assert!(matches!(
                osier.active_project().unwrap().selection,
                Selection::Compare { .. }
            ));
        });
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
    fn settings_dialog_closes_on_cancel(cx: &mut TestAppContext) {
        use gpui_kit::component::WindowExt as _;
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "one\n", "first");
        let h = Harness::open(cx, &repo);
        h.act(cx, |osier, window, cx| osier.open_settings(window, cx));
        cx.run_until_parked();
        let open = cx
            .update_window(h.window, |_, window, cx| window.has_active_dialog(cx))
            .expect("window");
        assert!(open, "dialog opened");
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
            .update_window(h.window, |_, window, cx| window.has_active_dialog(cx))
            .expect("window");
        assert!(!open, "Cancel (close button or Escape) closed the dialog");
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
