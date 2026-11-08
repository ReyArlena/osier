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
use crate::test_ids::TestId as _;
use osier_config::settings::RecentStatusMode;

gpui_kit::actions!(
    osier,
    [
        Quit,
        Refresh,
        OpenRepository,
        CloneRepository,
        CreateRepository,
        CloseProject,
        ToggleSidebar,
        ToggleCommandLog,
        ToggleCommitDetails,
        ToggleTheme,
        OpenSettings,
        OpenCommandPalette,
        SwitchProject,
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
        PrefixKey,
    ]
);

pub fn init(cx: &mut App) {
    crate::keymap::init(cx);
}

/// The status bar message for projects that could not be reopened at startup.
fn reopen_error(failures: &[String]) -> String {
    format!(
        "Could not reopen {}: {}",
        if failures.len() == 1 {
            "a project".to_string()
        } else {
            format!("{} projects", failures.len())
        },
        failures.join("; ")
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

/// Longest prefix key timeout the settings slider offers, in seconds.
pub(crate) const PREFIX_TIMEOUT_MAX_SECS: u32 = 30;

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
    /// The tab a project switch moved to from an empty Changes tab
    /// (`projects.empty_changes_tab`). Switching to a project with changes returns to Changes
    /// while it is still showing; choosing a tab clears it.
    pub(crate) auto_tab: Option<Tab>,
    /// The active project changed before its first snapshot: apply the empty-Changes rule
    /// when the snapshot arrives.
    tab_check_pending: bool,
    /// A project the user just opened (not one switched back to): if it has changes when its
    /// first snapshot arrives, show the Changes tab.
    fresh_open: Option<PathBuf>,
    /// Log tab shows the reflog instead of the commit list.
    pub(crate) show_reflog: bool,
    pub(crate) sidebar_visible: bool,
    pub(crate) command_log_visible: bool,
    pub(crate) expanded_command: Option<u64>,
    /// The command log lists only failed commands.
    pub(crate) command_log_failures_only: bool,
    pub(crate) diff: Entity<DiffState>,
    /// Branches containing a commit, for the commit details panel, by hash. Cleared when
    /// the refs move.
    pub(crate) commit_branches: std::collections::HashMap<String, Vec<String>>,
    pub(crate) commit_branches_loading: Option<String>,
    /// `Project::content_key` of what `diff` was loaded for.
    diff_key: Option<String>,
    /// Redraws on conflict choices and line selection in `diff`; swapped with it per tab.
    diff_subscription: Option<Subscription>,
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
    pub(crate) state_dirty: bool,
    /// Running fetch/pull/push, if any.
    pub(crate) remote_job: Option<crate::remote_ops::RemoteJob>,
    /// gh accounts discovered at startup (no tokens).
    pub(crate) gh_accounts: Vec<osier_auth::GhAccount>,
    /// `gh_accounts` has been asked for (whatever the answer); the clone and create
    /// submenus wait for it.
    pub(crate) gh_accounts_loaded: bool,
    /// GitHub API access for clone and create (`new_repo.rs`); a fake in tests.
    pub(crate) hub: std::sync::Arc<dyn crate::new_repo::Hub>,
    /// Repository lists per account (`login@host`), for the clone submenu and templates.
    pub(crate) hub_repos: std::collections::HashMap<String, Vec<osier_auth::hub::GhRepo>>,
    /// Counted background work for the status bar (`background.rs`).
    pub(crate) background: crate::background::Background,
    /// `background` version last drawn.
    background_seen: u64,
    /// Accounts whose list `prefetch_hub_repos` is fetching.
    pub(crate) hub_repos_loading: std::collections::HashSet<String>,
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
    /// Update check, install and the install route (`update_ui.rs`).
    pub(crate) updates: crate::update_ui::Updates,
    /// Focus target inside the settings sheet so Escape and its close button reach it.
    pub(crate) settings_focus: FocusHandle,
    /// Waiting for the key after the prefix key (see `prefix.rs`).
    pub(crate) prefix_pending: Option<Subscription>,
    /// The desktop's light/dark preference, for System mode (`system_scheme.rs`).
    pub(crate) system_scheme: crate::system_scheme::SystemScheme,
    /// Selected row in the Branches tab (branch, tag or remote).
    pub(crate) ref_selected: Option<crate::row_actions::RefRow>,
    /// Selected stash, by commit id.
    pub(crate) stash_selected: Option<String>,
    pub(crate) prefix_generation: u64,
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
    pub(crate) prefix_timeout: Entity<gpui_kit::component::slider::SliderState>,
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
        let prefix_secs = settings
            .keys
            .prefix_timeout_secs
            .min(PREFIX_TIMEOUT_MAX_SECS) as f32;
        let prefix_timeout = cx.new(|_| {
            gpui_kit::component::slider::SliderState::new()
                .min(0.)
                .max(PREFIX_TIMEOUT_MAX_SECS as f32)
                .step(1.)
                .default_value(prefix_secs)
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
            sink: sink.clone(),
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
            auto_tab: None,
            tab_check_pending: false,
            fresh_open: None,
            expanded_command: None,
            command_log_failures_only: false,
            diff,
            diff_subscription: None,
            diff_key: None,
            commit_branches: Default::default(),
            commit_branches_loading: None,
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
            gh_accounts_loaded: false,
            hub: crate::new_repo::default_hub(sink.clone()),
            hub_repos: std::collections::HashMap::new(),
            hub_repos_loading: std::collections::HashSet::new(),
            background: Default::default(),
            background_seen: 0,
            scanned: Vec::new(),
            sidebar_filter,
            sidebar_filter_bounds: Default::default(),
            session_only_accounts: Default::default(),
            patches,
            patch_selected: None,
            blame: None,
            ssh_status: None,
            git_version: None,
            updates: Default::default(),
            // A tab stop, so moving to the next stop from here lands on the settings search.
            settings_focus: cx.focus_handle().tab_stop(true),
            prefix_pending: None,
            system_scheme: Default::default(),
            ref_selected: None,
            stash_selected: None,
            prefix_generation: 0,
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
            prefix_timeout,
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
        this.diff_subscription = Some(Self::subscribe_diff(&diff_entity, cx));
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
        let slider = this.prefix_timeout.clone();
        this._subscriptions.push(cx.subscribe(
            &slider,
            |this, _, ev: &gpui_kit::component::slider::SliderEvent, cx| {
                use gpui_kit::component::slider::SliderEvent;
                let (v, save) = match ev {
                    SliderEvent::Change(v) => (v, false),
                    SliderEvent::Release(v) => (v, true),
                };
                this.settings.keys.prefix_timeout_secs = v.start().round() as u32;
                if save {
                    this.save_settings();
                }
                cx.notify();
            },
        ));
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

        // Restore open projects, then the CLI argument on top. Only the project that was
        // active opens here, so the window shows it at once however many are open; the rest
        // open in the background (`restore_in_background`). Failures go to the status bar:
        // toasts need the window root, which does not exist yet during construction.
        let restore: Vec<PathBuf> = this.state.open_projects.clone();
        // Opening marks each project active in the state; keep the one saved last session.
        let saved_active = this.state.active_project.clone();
        let first = saved_active
            .clone()
            .filter(|p| restore.contains(p))
            .or_else(|| restore.first().cloned());
        if let Some(p) = &first
            && let Err(e) = this.open_project(p, false, cx)
        {
            this.global_error = Some(reopen_error(&[e]));
        }
        let rest: Vec<PathBuf> = restore
            .into_iter()
            .filter(|p| Some(p) != first.as_ref())
            .collect();
        this.restore_in_background(rest, window, cx);
        if let Some(active) = saved_active
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
            this.set_tab(Tab::Files, cx);
            this.select_work_file(file, None, cx);
        }
        this.update_title(window);
        this.start_watcher_loop(window, cx);
        #[cfg(all(target_os = "linux", not(test)))]
        this.watch_system_scheme(cx);
        this.load_keymap(cx);
        // Without a focused element the app's key bindings have no dispatch path, so
        // shortcuts would only work after the first click into a list.
        let root_focus = this.focus_handle.clone();
        window.defer(cx, move |window, cx| root_focus.focus(window, cx));
        // Dev aid for screenshots: OSIER_OPEN_SETTINGS=1 opens the settings sheet at startup.
        // The sheet needs gpui-kit's root, which exists only after this view is installed.
        // OSIER_OPEN_PALETTE=root|switch_project|clone|create opens the command palette on
        // that mode.
        let palette = match std::env::var("OSIER_OPEN_PALETTE").as_deref() {
            Ok("root") => Some(crate::palette::PaletteMode::Root),
            Ok("switch_project") => Some(crate::palette::PaletteMode::SwitchProject),
            Ok("clone") => Some(crate::palette::PaletteMode::CloneAccount),
            Ok("create") => Some(crate::palette::PaletteMode::CreateAccount),
            _ => None,
        };
        if let Some(mode) = palette {
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(300))
                    .await;
                let _ = this.update_in(cx, |this, window, cx| {
                    this.open_palette_in(mode, window, cx)
                });
            })
            .detach();
        }
        // OSIER_OPEN_NEW_REPO=clone|local|github opens the clone ("Other…") or create dialog;
        // github uses the first gh account once they have loaded.
        if let Ok(which) = std::env::var("OSIER_OPEN_NEW_REPO") {
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(1500))
                    .await;
                let _ = this.update_in(cx, |this, window, cx| match which.as_str() {
                    "clone" => this.prompt_clone(None, window, cx),
                    "local" => this.prompt_create(None, window, cx),
                    "github" => {
                        let account = this.gh_accounts.first().cloned();
                        this.prompt_create(account, window, cx)
                    }
                    _ => {}
                });
            })
            .detach();
        }
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
        this.startup_update_check(cx);
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
        let busy = self.background.begin(crate::background::SCAN_DIRS);
        cx.spawn(async move |this, cx| {
            let _busy = busy;
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

    /// Pinned repositories that are not open, filtered like the rest of the sidebar.
    pub(crate) fn pinned_entries(&self, cx: &App) -> Vec<PathBuf> {
        self.pinned_matching(&self.sidebar_query(cx))
    }

    pub(crate) fn pinned_matching(&self, query: &Query) -> Vec<PathBuf> {
        self.state
            .pinned
            .iter()
            .filter(|p| !self.projects.iter().any(|o| &o.path == *p))
            .filter(|p| query.matches(p, self.other_row_status(p)))
            .cloned()
            .collect()
    }

    /// Recents and scanned repositories that are not open, filtered, with their origin.
    pub(crate) fn recent_entries(&self, cx: &App) -> Vec<(PathBuf, Option<PathBuf>)> {
        self.recent_matching(&self.sidebar_query(cx))
    }

    /// Recents and scanned repositories that are not open or pinned, with their scan
    /// directory when one provides them.
    pub(crate) fn recent_matching(&self, query: &Query) -> Vec<(PathBuf, Option<PathBuf>)> {
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
        let dux = a.dux_shell_sync.unwrap_or(true) && store.dux_available();
        let dark = match a.mode {
            ThemeMode::Dark => true,
            ThemeMode::Light => false,
            ThemeMode::System => match store.dux_mode() {
                Some(mode) if dux => mode == osier_theme::Mode::Dark,
                _ => crate::system_scheme::system_is_dark(
                    self.system_scheme,
                    matches!(
                        cx.window_appearance(),
                        WindowAppearance::Dark | WindowAppearance::VibrantDark
                    ),
                    a.system_fallback,
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
        a.dux_shell_sync = Some(false);
        self.apply_appearance(cx);
        self.save_settings();
        cx.notify();
    }

    /// Whether Dux Shell colours are followed: the user's choice, else on when detected.
    pub(crate) fn dux_sync_on(&self, cx: &App) -> bool {
        crate::themes::ThemeStore::global(cx).dux_available()
            && self.settings.appearance.dux_shell_sync.unwrap_or(true)
    }

    /// Turn Dux Shell sync on or off. Turning it on also sets the mode to System, so the
    /// shell's light/dark decides; the user can still pick Dark or Light afterwards.
    pub(crate) fn set_dux_sync(&mut self, on: bool, cx: &mut Context<Self>) {
        self.settings.appearance.dux_shell_sync = Some(on);
        if on {
            self.settings.appearance.mode = ThemeMode::System;
        }
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
                let switched = self.active != Some(ix);
                if switched {
                    self.park_pane();
                }
                self.active = Some(ix);
                if switched {
                    self.switch_tab_for_project();
                }
                self.state.open(&self.projects[ix].path);
                self.save_state();
            }
            return Ok(ix);
        }
        match Project::open(
            self.runner.clone(),
            self.watch_runner(),
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
                    self.park_pane();
                    self.active = Some(ix);
                    // A new project has no snapshot yet: decided when it arrives.
                    self.switch_tab_for_project();
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

    /// The runner for file watchers' own git calls: only failures reach the command log.
    fn watch_runner(&self) -> GitRunner {
        self.runner
            .clone()
            .with_sink(Arc::new(crate::command_log::FailuresOnly(
                self.sink.clone(),
            )))
    }

    /// Open the saved projects other than the active one without blocking the window: git
    /// discovery and watcher start run on the background executor in small chunks, and each
    /// chunk joins the sidebar in its saved order as it lands. Projects closed or opened by
    /// hand in the meantime are left alone.
    fn restore_in_background(
        &mut self,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        const CHUNK: usize = 16;
        if paths.is_empty() {
            return;
        }
        let (runner, watch_runner) = (self.runner.clone(), self.watch_runner());
        cx.spawn_in(window, async move |this, cx| {
            let mut failures = Vec::new();
            for chunk in paths.chunks(CHUNK) {
                let chunk = chunk.to_vec();
                let (runner, watch_runner) = (runner.clone(), watch_runner.clone());
                let prepared = cx
                    .background_spawn(async move {
                        chunk
                            .into_iter()
                            .map(|p| {
                                let r = Project::prepare(runner.clone(), watch_runner.clone(), &p);
                                (p, r)
                            })
                            .collect::<Vec<_>>()
                    })
                    .await;
                let Ok(failed) = this.update_in(cx, |this, window, cx| {
                    this.install_restored(prepared, window, cx)
                }) else {
                    return;
                };
                failures.extend(failed);
            }
            if !failures.is_empty() {
                let _ = this.update(cx, |this, cx| {
                    this.global_error = Some(reopen_error(&failures));
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Add projects prepared by `restore_in_background`. Returns the failures, for the
    /// status bar.
    fn install_restored(
        &mut self,
        prepared: Vec<(PathBuf, Result<crate::project::Prepared, String>)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<String> {
        let mut failures = Vec::new();
        let mut changed_state = false;
        for (path, result) in prepared {
            // Closed while it was loading, or opened by hand.
            if !self.state.open_projects.contains(&path)
                || self.projects.iter().any(|p| p.path == path)
            {
                continue;
            }
            let prepared = match result {
                Ok(p) => p,
                Err(error) => {
                    tracing::warn!(%error, path = %path.display(), "cannot reopen repository");
                    self.state.forget_recent(&path);
                    self.state.close(&path);
                    changed_state = true;
                    failures.push(format!("{}: {error}", crate::format::tilde(&path)));
                    continue;
                }
            };
            let project = Project::from_prepared(prepared, self.settings.git.log_page_size);
            if self.projects.iter().any(|p| p.path == project.path) {
                continue;
            }
            // Saved order: before the first open project that comes later in the list.
            let order = |p: &Path| {
                self.state
                    .open_projects
                    .iter()
                    .position(|q| q == p)
                    .unwrap_or(usize::MAX)
            };
            let rank = order(&path);
            let ix = self
                .projects
                .iter()
                .position(|p| order(&p.path) > rank)
                .unwrap_or(self.projects.len());
            self.projects.insert(ix, project);
            if let Some(active) = self.active
                && ix <= active
            {
                self.active = Some(active + 1);
            }
            self.refresh_project(ix, cx);
        }
        if changed_state {
            self.save_state();
        }
        if self.active.is_none() && !self.projects.is_empty() {
            self.activate(0, window, cx);
        }
        cx.notify();
        failures
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
        let already_open = self.projects.iter().any(|p| p.path == path);
        match self.open_project(path, true, cx) {
            Ok(ix) => {
                if !already_open {
                    self.fresh_open = Some(self.projects[ix].path.clone());
                    if self.projects[ix].snapshot.is_some() {
                        self.switch_tab_for_project();
                    }
                }
                self.activate(ix, window, cx)
            }
            Err(error) => window.push_notification(
                Notification::warning(format!("{error}. It was removed from the recent list."))
                    .title("Could not open repository"),
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
        let switched = self.active != Some(ix);
        if switched {
            self.park_pane();
        }
        self.active = Some(ix);
        let path = self.projects[ix].path.clone();
        self.state.open(&path);
        self.save_state();
        if switched {
            self.switch_tab_for_project();
        }
        self.sync_pane(true, cx);
        self.update_title(window);
        cx.notify();
    }

    pub(crate) fn close_project(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.projects.len() {
            return;
        }
        let closed_active = self.active == Some(ix);
        if closed_active {
            // Its editor and blame go with it.
            self.editor = None;
            self.blame = None;
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
            if closed_active {
                self.switch_tab_for_project();
            }
        }
        self.save_state();
        self.sync_pane(true, cx);
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
        let had_conflicts = project
            .snapshot
            .as_ref()
            .is_some_and(|s| s.status.conflicted().count() > 0);
        if project.revision != revision {
            return;
        }
        project.loading = false;
        project.mark_refreshed();
        let mut reload = false;
        match result {
            Ok(snapshot) => {
                // Refs moved: part of the content key of compares by name (see the reload
                // decision at the end).
                let refs_changed = project.snapshot.as_ref().is_none_or(|old| {
                    old.branches != snapshot.branches
                        || old.commits.first().map(|c| &c.hash)
                            != snapshot.commits.first().map(|c| &c.hash)
                });
                if refs_changed {
                    project.content_rev += 1;
                    if is_active {
                        self.commit_branches.clear();
                    }
                }
                // Keep the selected commit by hash across refreshes.
                if let Selection::Commit { hash } = &project.selection {
                    project.selected_commit = snapshot.commits.iter().position(|c| &c.hash == hash);
                }
                // Picked commits are hashes; drop the ones that are gone (squashed, reset away).
                project
                    .picked
                    .retain(|h| snapshot.commits.iter().any(|c| &c.hash == h));
                // Keep a shift-click pair on the same commits too: the rows are indices, and
                // a new commit at the top moves every row down by one.
                if project.compare_commit.is_some() {
                    let old_commits = project.snapshot.as_ref().map(|s| &s.commits);
                    let remap = |ix: Option<usize>| {
                        let hash = &old_commits?.get(ix?)?.hash;
                        snapshot.commits.iter().position(|c| &c.hash == hash)
                    };
                    let (anchor, other) = (
                        remap(project.selected_commit),
                        remap(project.compare_commit),
                    );
                    project.selected_commit = anchor;
                    project.compare_commit = other;
                }
                // Move every selection off what the refresh took away (a file that changed
                // area, a dropped stash), here and on the tabs that are away, so the pane
                // never names something the lists no longer show. An open editor keeps its
                // file selected.
                let editing = |editor: Option<&crate::edit_pane::FileEditor>, s: &Selection| matches!((editor, s), (Some(e), Selection::File { path, .. }) if &e.path == path);
                let editor = if is_active {
                    self.editor.as_ref()
                } else {
                    project.parked_editor.as_ref()
                };
                let keep = editing(editor, &project.selection);
                let moved = project.reconcile_selection(&project.selection, &snapshot, keep);
                if moved != project.selection {
                    project.selection = moved;
                    if is_active {
                        self.diff_force_large = false;
                    }
                }
                let panes: Vec<(Tab, Selection)> = project
                    .panes
                    .iter()
                    .map(|(tab, pane)| {
                        let keep = editing(pane.editor.as_ref(), &pane.selection);
                        (
                            *tab,
                            project.reconcile_selection(&pane.selection, &snapshot, keep),
                        )
                    })
                    .collect();
                for (tab, selection) in panes {
                    if let Some(pane) = project.panes.get_mut(&tab) {
                        pane.selection = selection;
                    }
                }
                if is_active && let Some(oid) = &self.stash_selected {
                    let was: Vec<String> = project
                        .snapshot
                        .as_ref()
                        .map(|s| s.stashes.iter().map(|s| s.oid.clone()).collect())
                        .unwrap_or_default();
                    let still = |o: &String| snapshot.stashes.iter().any(|s| &s.oid == o);
                    if !still(oid) {
                        self.stash_selected = crate::project::neighbour(&was, oid, still).cloned();
                    }
                }
                // Nothing picked on this tab yet: its default (the first change on Changes,
                // the latest commit on Log), so the pane is not empty.
                if project.selection == Selection::None
                    && let Some(tab) = project.pane_tab
                {
                    let pane = Project::default_pane(tab, Some(&snapshot));
                    if pane.selection != Selection::None {
                        project.selection = pane.selection;
                        project.picked = pane.picked;
                        project.selected_commit = pane
                            .selected_hash
                            .and_then(|h| snapshot.commits.iter().position(|c| c.hash == h));
                        reload = true;
                    }
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
        if is_active && self.tab_check_pending {
            self.switch_tab_for_project();
            self.sync_pane(false, cx);
        }
        // A merge, rebase or cherry-pick just stopped on conflicts: they are resolved on the
        // Changes tab, so go there and show the first one.
        let first_conflict = self.projects[ix]
            .snapshot
            .as_ref()
            .and_then(|s| s.status.conflicted().next().map(|e| e.path.clone()));
        if is_active
            && !had_conflicts
            && let Some(path) = first_conflict
        {
            self.set_tab(Tab::Changes, cx);
            let shown = matches!(
                &self.projects[ix].selection,
                Selection::File {
                    area: Area::Conflicted,
                    ..
                }
            );
            if !shown {
                self.select_file(path, Area::Conflicted, cx);
            }
            cx.notify();
            return;
        }
        // Reload the visible diff only when what it shows changed: the file it shows was
        // edited (even when git status stays the same), the index or the refs moved, or the
        // selection itself changed. Other files changing leave it, and its scroll, alone.
        if is_active {
            let p = &self.projects[ix];
            reload |= self.diff_key.as_deref() != Some(p.content_key(&p.selection).as_str());
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
                    // Background work started or finished: redraw the status bar.
                    let bg = this.background.version();
                    if bg != this.background_seen {
                        this.background_seen = bg;
                        if this.settings.appearance.show_background_tasks {
                            cx.notify();
                        }
                    }
                    // Another `osier <path>` handed its path over.
                    for request in crate::instance::take_requests() {
                        if let crate::instance::Request::Open(path) = request {
                            this.open_project_notify(&path, window, cx);
                        }
                        window.activate_window();
                    }
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
            p.picked.clear();
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
            p.picked.clear();
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
            p.picked.clear();
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
        let Some(hash) = p.commit_at(ix).map(|c| c.hash.clone()) else {
            return;
        };
        p.selection = Selection::Commit { hash: hash.clone() };
        p.selected_commit = Some(ix);
        p.compare_commit = None;
        p.picked = vec![hash];
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

    /// Shift-click: pick every row from the selected commit to `ix`, and compare the two ends.
    pub(crate) fn select_range(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(anchor) = self.active_project().and_then(|p| p.selected_commit) else {
            self.select_commit(ix, cx);
            return;
        };
        self.compare_with_selected(ix, cx);
        if let Some(p) = self.active_project_mut()
            && let Some(s) = &p.snapshot
        {
            let rows = anchor.min(ix)..=anchor.max(ix);
            p.picked = s.commits[rows].iter().map(|c| c.hash.clone()).collect();
        }
        cx.notify();
    }

    /// Ctrl-click: add or remove one commit. Two picked compare like shift-click; with more,
    /// the diff shows the clicked commit (or the last one left after removing).
    pub(crate) fn toggle_pick(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(p) = self.active_project_mut() else {
            return;
        };
        let Some(hash) = p.commit_at(ix).map(|c| c.hash.clone()) else {
            return;
        };
        if p.picked.is_empty() {
            self.select_commit(ix, cx);
            return;
        }
        let mut picked = p.picked.clone();
        if let Some(pos) = picked.iter().position(|h| *h == hash) {
            if picked.len() == 1 {
                return;
            }
            picked.remove(pos);
        } else {
            picked.push(hash.clone());
        }
        let row_of = |p: &Project, h: &str| {
            p.snapshot
                .as_ref()
                .and_then(|s| s.commits.iter().position(|c| c.hash == h))
        };
        match picked.len() {
            1 => {
                if let Some(row) = row_of(p, &picked[0]) {
                    self.select_commit(row, cx);
                }
            }
            2 => {
                let (Some(a), Some(b)) = (row_of(p, &picked[0]), row_of(p, &picked[1])) else {
                    return;
                };
                p.selected_commit = Some(a);
                self.compare_with_selected(b, cx);
                if let Some(p) = self.active_project_mut() {
                    p.picked = picked;
                }
            }
            _ => {
                let show = if picked.contains(&hash) {
                    hash
                } else {
                    picked[picked.len() - 1].clone()
                };
                let row = row_of(p, &show);
                p.picked = picked;
                p.selected_commit = row;
                p.compare_commit = None;
                p.selection = Selection::Commit { hash: show };
                self.diff_force_large = false;
                self.editor = None;
                self.reload_selection(cx);
            }
        }
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
        self.diff_key = self.active_project().map(|p| p.content_key(&p.selection));
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
            Tab::Branches | Tab::Stashes => self.move_ref_or_stash_selection(delta, cx),
            Tab::Files | Tab::Patches => {}
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
        self.auto_tab = None;
        self.sync_pane(false, cx);
        cx.notify();
    }

    /// Keep the active project's open editor and blame with it while another is active.
    /// Its tabs' cached diff views are dropped (selections stay): only the active project
    /// keeps diff views, so memory stays bounded however many projects are open.
    fn park_pane(&mut self) {
        let (editor, blame) = (self.editor.take(), self.blame.take());
        if let Some(p) = self.active_project_mut() {
            p.parked_editor = editor;
            p.parked_blame = blame;
            for pane in p.panes.values_mut() {
                pane.diff = None;
            }
        }
    }

    fn subscribe_diff(diff: &Entity<DiffState>, cx: &mut Context<Self>) -> Subscription {
        cx.subscribe(diff, |_, _, ev: &DiffEvent, cx| {
            if matches!(
                ev,
                DiffEvent::ConflictResolved(..) | DiffEvent::SelectionChanged(_)
            ) {
                cx.notify();
            }
        })
    }

    /// An empty diff view with the current layout and context settings.
    fn new_diff_view(&self, cx: &mut Context<Self>) -> (Entity<DiffState>, Subscription) {
        let mode = self.diff.read(cx).mode();
        let context = self.settings.diff.context_lines as usize;
        let state = cx.new(|cx| {
            DiffState::new([], cx)
                .with_mode(mode)
                .with_context_lines(Some(context))
        });
        let subscription = Self::subscribe_diff(&state, cx);
        (state, subscription)
    }

    /// Make the right pane match the current tab: each tab keeps its own selection (and
    /// editor and blame), restored when it comes back, or starts from its default (the
    /// first change on Changes, the latest commit on Log). `reload` also reloads when the
    /// tab did not change, for a project switch.
    pub(crate) fn sync_pane(&mut self, reload: bool, cx: &mut Context<Self>) {
        let tab = self.tab;
        let Some(ix) = self.active.filter(|ix| *ix < self.projects.len()) else {
            self.editor = None;
            self.blame = None;
            if reload {
                self.reload_selection(cx);
            }
            return;
        };
        let switching = self.projects[ix].pane_tab != Some(tab);
        // The outgoing tab's diff view, kept unless it was still loading (never reuse a
        // half-loaded view).
        let leaving_diff = (switching && !self.diff_loading).then(|| {
            let (fresh, fresh_sub) = self.new_diff_view(cx);
            crate::project::DiffCache {
                state: std::mem::replace(&mut self.diff, fresh),
                subscription: self
                    .diff_subscription
                    .replace(fresh_sub)
                    .unwrap_or_else(|| Self::subscribe_diff(&self.diff, cx)),
                model: std::mem::take(&mut self.diff_model),
                files: self.diff_files,
                error: self.diff_error.take(),
                too_large: self.diff_too_large.take(),
                viewer: self.viewer.take(),
                key: self.diff_key.clone().unwrap_or_default(),
            }
        });
        let mut restored_diff = None;
        let p = &mut self.projects[ix];
        let mut editor = self.editor.take().or_else(|| p.parked_editor.take());
        let mut blame = self.blame.take().or_else(|| p.parked_blame.take());
        if switching {
            if let Some(old) = p.pane_tab {
                let hash_at = |ix: Option<usize>| {
                    let s = p.snapshot.as_ref()?;
                    Some(s.commits.get(ix?)?.hash.clone())
                };
                let pane = crate::project::TabPane {
                    selected_hash: hash_at(p.selected_commit),
                    compare_hash: hash_at(p.compare_commit),
                    selection: std::mem::take(&mut p.selection),
                    picked: std::mem::take(&mut p.picked),
                    editor: editor.take(),
                    blame: blame.take(),
                    diff: leaving_diff,
                };
                p.panes.insert(old, pane);
            }
            // A tab left with nothing picked starts from its default again (changes may have
            // appeared on Changes since).
            let restored = match p.panes.remove(&tab) {
                Some(pane) if pane.selection != Selection::None => pane,
                _ => Project::default_pane(tab, p.snapshot.as_ref()),
            };
            p.selected_commit = restored.selected_hash.as_deref().and_then(|h| p.row_of(h));
            p.compare_commit = restored.compare_hash.as_deref().and_then(|h| p.row_of(h));
            p.selection = restored.selection;
            p.picked = restored.picked;
            p.pane_tab = Some(tab);
            editor = restored.editor;
            blame = restored.blame;
            // Reuse the tab's diff view (scroll and all) unless its content has moved.
            let key = p.content_key(&p.selection);
            restored_diff = restored.diff.filter(|d| d.key == key);
            self.diff_force_large = false;
        }
        let reused = if let Some(cache) = restored_diff {
            let mode = self.diff.read(cx).mode();
            self.diff_key = Some(cache.key);
            self.diff = cache.state;
            self.diff_subscription = Some(cache.subscription);
            self.diff_model = cache.model;
            self.diff_files = cache.files;
            self.diff_error = cache.error;
            self.diff_too_large = cache.too_large;
            self.viewer = cache.viewer;
            self.diff_loading = false;
            // A load still running for the tab that was left must not land here.
            self.diff_revision += 1;
            if self.diff.read(cx).mode() != mode {
                self.diff.update(cx, |d, cx| d.set_mode(mode, cx));
            }
            true
        } else {
            false
        };
        if (switching && !reused) || reload {
            self.reload_selection(cx);
        }
        self.editor = editor;
        self.blame = blame;
    }

    /// After the active project changed: leave an empty Changes tab for the tab set in
    /// `projects.empty_changes_tab`, or return to Changes when an earlier switch left it and
    /// this project has something to show; a project just opened with changes always shows
    /// Changes (`fresh_open`). Waits for the first snapshot of a new project.
    pub(crate) fn switch_tab_for_project(&mut self) {
        use osier_config::settings::EmptyChangesTab as E;
        let Some(empty) = self
            .active_project()
            .and_then(|p| p.snapshot.as_ref())
            .map(|s| s.status.entries.is_empty() && s.in_progress.is_none())
        else {
            self.tab_check_pending = true;
            return;
        };
        self.tab_check_pending = false;
        // Just opened (fresh or from recents) with changes: show them. Clean ones, and every
        // switch between open projects, follow the rules below.
        if let Some(fresh) = self.fresh_open.take()
            && !empty
            && self.active_project().is_some_and(|p| p.path == fresh)
        {
            self.tab = Tab::Changes;
            self.auto_tab = None;
            return;
        }
        if let Some(auto) = self.auto_tab.take()
            && self.tab == auto
        {
            if empty {
                self.auto_tab = Some(auto);
            } else {
                self.tab = Tab::Changes;
            }
            return;
        }
        if self.tab != Tab::Changes || !empty {
            return;
        }
        let target = match self.settings.projects.empty_changes_tab {
            E::Stay => return,
            E::Log => Tab::Log,
            E::Branches => Tab::Branches,
            E::Stashes => Tab::Stashes,
            E::Files => Tab::Files,
            E::Patches => Tab::Patches,
        };
        self.tab = target;
        self.auto_tab = Some(target);
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
                Notification::warning(problems.join("\n")).title("keymap.toml has problems"),
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
                    Notification::error(error.to_string())
                        .title("keymap.toml has an error; fix it first"),
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
                Notification::error(error.to_string()).title("Could not save keymap.toml"),
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
                gpui_kit::component::notification::Notification::error(error.to_string())
                    .title("Could not reset keymap"),
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
        let keymap = crate::keymap::Keymap::global(cx);
        let refused = crate::keymap::refused(keymap, id, &keys);
        if !refused.is_empty() {
            self.refuse_key(&keys, &refused, window, cx);
            return;
        }
        let mut current = keymap.keys_for(id).to_vec();
        if !current.contains(&keys) {
            current.push(keys);
            self.set_action_keys(id, current, window, cx);
        }
    }

    /// A key that would clash with another binding where Osier does not allow it.
    fn refuse_key(
        &mut self,
        keys: &str,
        others: &[&str],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use gpui_kit::component::WindowExt as _;
        use gpui_kit::component::notification::Notification;
        window.push_notification(
            Notification::warning(
                "Context-menu keys and the prefix key cannot share a key with another binding.",
            )
            .title(format!(
                "{} is already used by {}",
                crate::keymap::pretty(keys),
                others.join(", ")
            )),
            cx,
        );
    }

    /// Switch one key of an action between no prefix, prefix only and either.
    pub(crate) fn set_key_mode(
        &mut self,
        id: &'static str,
        key: &str,
        mode: crate::keymap::KeyMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::keymap::{Keymap, refused, split_mode, with_mode};
        let keymap = Keymap::global(cx);
        let new = with_mode(mode, split_mode(key).1);
        if new == key {
            return;
        }
        let clash = refused(keymap, id, &new);
        if !clash.is_empty() {
            self.refuse_key(&new, &clash, window, cx);
            return;
        }
        let keys: Vec<String> = keymap
            .keys_for(id)
            .iter()
            .map(|k| if k == key { new.clone() } else { k.clone() })
            .collect();
        self.set_action_keys(id, keys, window, cx);
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

    /// From an error notification: open the command log on failures only, with the command
    /// that failed (when known) expanded. Failures are few, so it is at or near the top.
    pub(crate) fn show_failed_command(&mut self, id: Option<u64>, cx: &mut Context<Self>) {
        if !self.command_log_visible {
            self.command_log_visible = true;
            self.save_state();
        }
        self.command_log_failures_only = true;
        self.expanded_command = id;
        cx.notify();
    }
}

/// Conflicts first, then staged, unstaged, untracked.
pub(crate) fn first_change(status: &osier_git::status::RepoStatus) -> Option<(String, Area)> {
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
            .on_action(cx.listener(|this, _: &CloneRepository, window, cx| {
                this.open_palette_in(crate::palette::PaletteMode::CloneAccount, window, cx)
            }))
            .on_action(cx.listener(|this, _: &CreateRepository, window, cx| {
                this.open_palette_in(crate::palette::PaletteMode::CreateAccount, window, cx)
            }))
            .on_action(cx.listener(|this, _: &CloseProject, window, cx| {
                if let Some(ix) = this.active {
                    this.close_project(ix, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| this.toggle_sidebar(cx)))
            .on_action(cx.listener(|this, _: &ToggleCommandLog, _, cx| this.toggle_command_log(cx)))
            .on_action(
                cx.listener(|this, _: &ToggleCommitDetails, _, cx| this.toggle_commit_details(cx)),
            )
            .on_action(cx.listener(|this, _: &ToggleTheme, _, cx| this.toggle_theme(cx)))
            .on_action(
                cx.listener(|this, _: &OpenSettings, window, cx| this.open_settings(window, cx)),
            )
            .on_action(cx.listener(|this, _: &OpenCommandPalette, window, cx| {
                this.open_command_palette(window, cx)
            }))
            .on_action(cx.listener(|this, _: &SwitchProject, window, cx| {
                this.open_palette_in(crate::palette::PaletteMode::SwitchProject, window, cx)
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
            .on_action(cx.listener(|this, _: &PrefixKey, window, cx| this.start_prefix(window, cx)))
            .on_action(
                cx.listener(|this, a: &crate::row_actions::RowAction, window, cx| {
                    this.run_row_action(a.kind, window, cx)
                }),
            )
            // Any click cancels a waiting prefix key.
            .capture_any_mouse_down(cx.listener(|this, _, _, cx| this.cancel_prefix(cx)))
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
                                                    // Keep the saved width when the window
                                                    // resizes (a tiling compositor resizes it
                                                    // right after it opens); the diff pane
                                                    // takes up the difference.
                                                    .fixed()
                                                    .child(
                                                        div()
                                                            .test_id("center-pane")
                                                            .size_full()
                                                            .child(self.render_center(window, cx)),
                                                    ),
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
            Self::open_with(cx, State::default(), Some(repo.path().to_path_buf()))
        }

        /// Start like a later session: `state` as saved, `open` as the command-line path.
        fn open_with(
            cx: &mut TestAppContext,
            state: State,
            open: Option<std::path::PathBuf>,
        ) -> Self {
            cx.update(gpui_kit::init);
            cx.update(init);
            let dirs = tempfile::tempdir().expect("tempdir");
            let paths = Paths::under(dirs.path());
            let sink = Arc::new(UiCommandSink::default());
            let runner = isolated(sink.clone());
            let (handle, view) = cx.update(|cx| {
                gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                    cx.new(|cx| {
                        Osier::new(
                            paths,
                            Settings::default(),
                            state,
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

    fn selected(h: &Harness, cx: &mut TestAppContext) -> Selection {
        h.read(cx, |o, _| o.active_project().unwrap().selection.clone())
    }

    fn file(path: &str, area: Area) -> Selection {
        Selection::File {
            path: path.into(),
            area,
        }
    }

    /// Three changed files `a`, `b`, `c` in the Unstaged list.
    fn three_unstaged() -> TempRepo {
        let repo = TempRepo::new();
        for f in ["a.txt", "b.txt", "c.txt"] {
            repo.commit_file(f, "x\n", f);
        }
        for f in ["a.txt", "b.txt", "c.txt"] {
            repo.write(f, "changed\n");
        }
        repo
    }

    #[gpui::test]
    fn ui_staging_the_selected_file_moves_to_the_next(cx: &mut TestAppContext) {
        let repo = three_unstaged();
        let h = Harness::open(cx, &repo);
        h.click(cx, "change-row:unstaged:b.txt");
        h.click(cx, "change-stage:unstaged:b.txt");
        assert_eq!(selected(&h, cx), file("c.txt", Area::Unstaged));
        // The pane shows the new selection's diff, not an empty one.
        assert_eq!(h.read(cx, |o, _| o.diff_files), 1);
        // Last in the list: the one above.
        h.click(cx, "change-stage:unstaged:c.txt");
        assert_eq!(selected(&h, cx), file("a.txt", Area::Unstaged));
        // Only one left: follow it into Staged.
        h.click(cx, "change-stage:unstaged:a.txt");
        assert_eq!(selected(&h, cx), file("a.txt", Area::Staged));
        assert_eq!(h.read(cx, |o, _| o.diff_files), 1);
    }

    #[gpui::test]
    fn ui_staging_another_file_keeps_the_selection(cx: &mut TestAppContext) {
        let repo = three_unstaged();
        let h = Harness::open(cx, &repo);
        h.click(cx, "change-row:unstaged:a.txt");
        h.click(cx, "change-stage:unstaged:c.txt");
        assert_eq!(selected(&h, cx), file("a.txt", Area::Unstaged));
        // Stage all follows the selected file; unstaging moves on in the Staged list.
        h.act(cx, |o, w, cx| o.stage_all(w, cx));
        assert_eq!(selected(&h, cx), file("a.txt", Area::Staged));
        h.click(cx, "change-stage:staged:a.txt");
        assert_eq!(selected(&h, cx), file("b.txt", Area::Staged));
    }

    #[gpui::test]
    fn ui_staging_a_file_open_in_the_editor_keeps_it(cx: &mut TestAppContext) {
        let repo = three_unstaged();
        let h = Harness::open(cx, &repo);
        h.click(cx, "change-row:unstaged:b.txt");
        h.act(cx, |o, w, cx| o.open_editor(w, cx));
        h.click(cx, "change-stage:unstaged:b.txt");
        assert_eq!(selected(&h, cx), file("b.txt", Area::Staged));
        assert!(h.read(cx, |o, _| o.editor.is_some()));
    }

    #[gpui::test]
    fn ui_selection_on_an_away_tab_follows_too(cx: &mut TestAppContext) {
        let repo = three_unstaged();
        let h = Harness::open(cx, &repo);
        h.click(cx, "change-row:unstaged:b.txt");
        h.act(cx, |o, _, cx| o.set_tab(super::Tab::Log, cx));
        repo.git(["add", "b.txt"]);
        h.act(cx, |o, _, cx| o.refresh_active(cx));
        h.act(cx, |o, _, cx| o.set_tab(super::Tab::Changes, cx));
        assert_eq!(selected(&h, cx), file("c.txt", Area::Unstaged));
    }

    #[gpui::test]
    fn ui_dropping_the_selected_stash_moves_to_the_next(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a\n", "first");
        for n in 0..3 {
            repo.write("a.txt", &format!("{n}\n"));
            repo.git(["stash", "-q"]);
        }
        let h = Harness::open(cx, &repo);
        h.act(cx, |o, _, cx| o.set_tab(super::Tab::Stashes, cx));
        let oids: Vec<String> = h.read(cx, |o, _| {
            let s = o.active_project().unwrap().snapshot.as_ref().unwrap();
            s.stashes.iter().map(|s| s.oid.clone()).collect()
        });
        h.act(cx, |o, _, cx| o.select_stash(oids[1].clone(), cx));
        repo.git(["stash", "drop", "-q", "stash@{1}"]);
        h.act(cx, |o, _, cx| o.refresh_active(cx));
        let to = |oid: &str| Selection::Compare {
            from: format!("{oid}^1"),
            to: oid.into(),
            since_merge_base: false,
        };
        assert_eq!(selected(&h, cx), to(&oids[2]));
        assert_eq!(
            h.read(cx, |o, _| o.stash_selected.clone()),
            Some(oids[2].clone())
        );
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
        // A clean repository opens on Log (`projects.empty_changes_tab`); start from Changes.
        h.act(cx, |o, _, cx| o.set_tab(super::Tab::Changes, cx));
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
    fn ui_keymap_mode_buttons_and_refused_clashes(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a\n", "first");
        let h = Harness::open(cx, &repo);
        let keys_for = |h: &Harness, cx: &mut TestAppContext, id: &str| {
            let id = id.to_string();
            h.read(cx, move |_, cx| {
                crate::keymap::Keymap::global(cx).keys_for(&id).to_vec()
            })
        };
        h.keys(cx, "ctrl-,");
        h.keys(cx, "/");
        h.type_text(cx, "Search projects");
        // The three buttons switch the key's mode.
        h.click(cx, "keymode:focus_project_search:/:prefix");
        assert_eq!(keys_for(&h, cx, "focus_project_search"), ["prefix /"]);
        h.click(cx, "keymode:focus_project_search:prefix /:either");
        assert_eq!(keys_for(&h, cx, "focus_project_search"), ["prefix? /"]);
        h.click(cx, "keymode:focus_project_search:prefix? /:plain");
        assert_eq!(keys_for(&h, cx, "focus_project_search"), ["/"]);

        // A context-menu key may not take a key another binding uses.
        h.act(cx, |o, _, cx| {
            o.start_key_recording("row_add_to_gitignore", cx)
        });
        h.keys(cx, "/");
        assert!(
            keys_for(&h, cx, "row_add_to_gitignore").is_empty(),
            "refused"
        );
        h.act(cx, |o, _, cx| {
            o.start_key_recording("row_add_to_gitignore", cx)
        });
        h.keys(cx, "i");
        assert_eq!(keys_for(&h, cx, "row_add_to_gitignore"), ["i"]);
        // ...and a global cannot take it afterwards either.
        h.act(cx, |o, _, cx| o.start_key_recording("refresh", cx));
        h.keys(cx, "i");
        assert_eq!(keys_for(&h, cx, "refresh"), ["ctrl-r", "f5"]);
        // Prefixed keys live in their own namespace: `prefix i` is fine for a global.
        h.act(cx, |o, w, cx| {
            o.set_key_mode(
                "row_add_to_gitignore",
                "i",
                crate::keymap::KeyMode::Prefix,
                w,
                cx,
            )
        });
        h.act(cx, |o, _, cx| o.start_key_recording("refresh", cx));
        h.keys(cx, "i");
        assert_eq!(keys_for(&h, cx, "refresh"), ["ctrl-r", "f5", "i"]);
        // Switching the menu key back to plain would clash now: refused.
        h.act(cx, |o, w, cx| {
            o.set_key_mode(
                "row_add_to_gitignore",
                "prefix i",
                crate::keymap::KeyMode::Plain,
                w,
                cx,
            )
        });
        assert_eq!(keys_for(&h, cx, "row_add_to_gitignore"), ["prefix i"]);
    }

    #[gpui::test]
    fn recording_captures_keys_instead_of_running_them(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a\n", "first");
        let h = Harness::open(cx, &repo);
        // A clean repository opens on Log (`projects.empty_changes_tab`); start from Changes.
        h.act(cx, |o, _, cx| o.set_tab(super::Tab::Changes, cx));
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

    /// Two open repositories, the second one active. Returns the harness and both names.
    fn two_projects(cx: &mut TestAppContext) -> (Harness, TempRepo, TempRepo, String, String) {
        let a = TempRepo::new();
        a.commit_file("a.txt", "a\n", "first");
        let b = TempRepo::new();
        b.commit_file("b.txt", "b\n", "first");
        let h = Harness::open(cx, &a);
        let path = b.path().to_path_buf();
        h.act(cx, |o, w, cx| o.open_project_notify(&path, w, cx));
        let names = h.read(cx, |o, _| {
            assert_eq!(o.active, Some(1));
            (o.projects[0].name.clone(), o.projects[1].name.clone())
        });
        (h, a, b, names.0, names.1)
    }

    fn in_palette(h: &Harness, cx: &mut TestAppContext) -> bool {
        h.has_settings(cx)
    }

    fn active_name(h: &Harness, cx: &mut TestAppContext) -> String {
        h.read(cx, |o, _| {
            o.active_project()
                .map(|p| p.name.clone())
                .unwrap_or_default()
        })
    }

    #[gpui::test]
    fn ui_switch_project_shortcut_flips_to_the_other_project(cx: &mut TestAppContext) {
        let (h, _a, _b, first, second) = two_projects(cx);
        h.keys(cx, "ctrl-o");
        assert!(h.bounds(cx, "palette-badge").is_some(), "submenu badge");
        assert!(h.bounds(cx, &format!("palette-project:{first}")).is_some());
        assert!(h.bounds(cx, &format!("palette-project:{second}")).is_some());
        // The current project is skipped by the initial highlight.
        h.keys(cx, "enter");
        assert!(!in_palette(&h, cx));
        assert_eq!(active_name(&h, cx), first);
        // Now the first is current: the highlight starts on the second.
        h.keys(cx, "ctrl-o enter");
        assert_eq!(active_name(&h, cx), second);
    }

    #[gpui::test]
    fn ui_switch_project_highlights_the_first_row_when_it_is_not_current(cx: &mut TestAppContext) {
        let (h, _a, _b, first, second) = two_projects(cx);
        // A recent project below the open ones must not steal the highlight.
        h.act(cx, |o, w, cx| o.activate(0, w, cx));
        let c = TempRepo::new();
        c.commit_file("c.txt", "c\n", "first");
        let path = c.path().to_path_buf();
        h.act(cx, |o, w, cx| {
            o.open_project_notify(&path, w, cx);
            o.close_project(2, w, cx);
            o.activate(1, w, cx);
        });
        assert_eq!(active_name(&h, cx), second);
        h.keys(cx, "ctrl-o enter");
        assert_eq!(active_name(&h, cx), first);
    }

    #[gpui::test]
    fn ui_switch_project_filters_and_moves_with_arrows(cx: &mut TestAppContext) {
        let (h, _a, _b, first, second) = two_projects(cx);
        h.keys(cx, "ctrl-o");
        h.type_text(cx, &second);
        assert!(h.bounds(cx, &format!("palette-project:{first}")).is_none());
        h.keys(cx, "enter");
        assert_eq!(active_name(&h, cx), second);
        // Arrows reach the list through the custom query field.
        h.keys(cx, "ctrl-o down up enter");
        assert_eq!(active_name(&h, cx), first);
    }

    #[gpui::test]
    fn ui_switch_project_opens_a_recent_project(cx: &mut TestAppContext) {
        let (h, _a, b, first, second) = two_projects(cx);
        h.act(cx, |o, w, cx| o.close_project(1, w, cx));
        assert_eq!(active_name(&h, cx), first);
        h.keys(cx, "ctrl-o");
        h.type_text(cx, &second);
        assert!(h.bounds(cx, &format!("palette-project:{second}")).is_some());
        h.keys(cx, "enter");
        assert_eq!(active_name(&h, cx), second);
        h.read(cx, |o, _| {
            assert!(o.projects.iter().any(|p| p.path == b.path()));
        });
    }

    #[gpui::test]
    fn ui_palette_submenu_back_and_escape(cx: &mut TestAppContext) {
        let (h, _a, _b, first, _second) = two_projects(cx);
        // Into the submenu from the commands, by typing and by clicking.
        h.keys(cx, "ctrl-p");
        assert!(h.bounds(cx, "palette-badge").is_none());
        h.type_text(cx, "switch proj");
        h.keys(cx, "enter");
        assert!(h.bounds(cx, "palette-badge").is_some());
        // Backspace with an empty query goes back to the commands; the palette stays.
        h.keys(cx, "backspace");
        assert!(h.bounds(cx, "palette-badge").is_none());
        assert!(in_palette(&h, cx));
        h.click(cx, "palette-command:Switch project…");
        assert!(h.bounds(cx, "palette-badge").is_some());
        // Backspace that deletes text stays in the submenu.
        h.type_text(cx, "x");
        h.keys(cx, "backspace");
        assert!(h.bounds(cx, "palette-badge").is_some());
        // Escape goes back too, then closes the commands as before.
        h.keys(cx, "escape");
        assert!(h.bounds(cx, "palette-badge").is_none());
        assert!(in_palette(&h, cx));
        h.keys(cx, "escape");
        assert!(!in_palette(&h, cx));
        // Opened straight into the submenu, Escape still lands on the commands.
        h.keys(cx, "ctrl-o escape");
        assert!(in_palette(&h, cx));
        assert!(h.bounds(cx, "palette-command:Switch project…").is_some());
        h.keys(cx, "escape");
        assert!(!in_palette(&h, cx));
        assert_ne!(active_name(&h, cx), first);
    }

    #[gpui::test]
    fn switch_project_rows_carry_the_sidebar_markers(cx: &mut TestAppContext) {
        use crate::switch_project::Section;
        use osier_config::settings::ChangesIndicator;
        let repo = repo_with_upstream(2, true);
        let h = Harness::open(cx, &repo);
        let path = repo.path().to_path_buf();
        let row = |h: &Harness, cx: &mut TestAppContext| {
            h.read(cx, |o, _| o.switch_project_entries().remove(0))
        };
        let e = row(&h, cx);
        assert_eq!(e.section, Section::Open);
        assert!(e.active && e.dirty && !e.dot && !e.pinned);
        assert_eq!(e.arrows.as_deref(), Some("↑2"));
        assert!(e.dir.ends_with(&e.name));
        h.act(cx, |o, _, cx| {
            o.settings.appearance.changes_indicator = ChangesIndicator::Dot;
            o.toggle_pin(&path, cx);
        });
        let e = row(&h, cx);
        assert!(e.dirty && e.dot && e.pinned);
        // Closed and pinned: listed under Pinned with the pin as its icon.
        h.act(cx, |o, w, cx| o.close_project(0, w, cx));
        let e = row(&h, cx);
        assert_eq!(e.section, Section::Pinned);
        assert!(!e.active);
    }

    fn tab(h: &Harness, cx: &mut TestAppContext) -> super::Tab {
        h.read(cx, |o, _| o.tab)
    }

    #[gpui::test]
    fn empty_changes_tab_on_startup_and_when_switching(cx: &mut TestAppContext) {
        use super::Tab;
        // Startup on a clean project: Changes would be empty, so Log shows.
        let (h, a, _b, first, second) = two_projects(cx);
        assert_eq!(tab(&h, cx), Tab::Log);
        // Changes appear in the first project; going back there returns to Changes.
        a.write("new.txt", "x\n");
        h.act(cx, |o, _, cx| o.refresh_project(0, cx));
        h.keys(cx, "ctrl-o enter");
        assert_eq!(active_name(&h, cx), first);
        assert_eq!(tab(&h, cx), Tab::Changes);
        // And to the clean one: Log again.
        h.keys(cx, "ctrl-o enter");
        assert_eq!(active_name(&h, cx), second);
        assert_eq!(tab(&h, cx), Tab::Log);
    }

    #[gpui::test]
    fn ui_other_tabs_and_manual_choices_are_kept(cx: &mut TestAppContext) {
        use super::Tab;
        let (h, a, _b, first, _second) = two_projects(cx);
        a.write("new.txt", "x\n");
        h.act(cx, |o, _, cx| o.refresh_project(0, cx));
        // On Branches, switching never moves the tab.
        h.click(cx, "tab-branches");
        h.keys(cx, "ctrl-o enter");
        assert_eq!(active_name(&h, cx), first);
        assert_eq!(tab(&h, cx), Tab::Branches);
        h.keys(cx, "ctrl-o enter");
        assert_eq!(tab(&h, cx), Tab::Branches);
        // Moved to Log automatically, then picked Log by hand: it stays Log on a project
        // with changes.
        h.click(cx, "tab-changes");
        h.keys(cx, "ctrl-o enter");
        assert_eq!(tab(&h, cx), Tab::Changes, "first has changes");
        h.keys(cx, "ctrl-o enter");
        assert_eq!(tab(&h, cx), Tab::Log);
        h.click(cx, "tab-log");
        h.keys(cx, "ctrl-o enter");
        assert_eq!(active_name(&h, cx), first);
        assert_eq!(tab(&h, cx), Tab::Log);
    }

    #[gpui::test]
    fn empty_changes_tab_setting_picks_the_tab_or_stays(cx: &mut TestAppContext) {
        use super::Tab;
        use osier_config::settings::EmptyChangesTab;
        let (h, a, _b, _first, _second) = two_projects(cx);
        a.write("new.txt", "x\n");
        h.act(cx, |o, _, cx| o.refresh_project(0, cx));
        h.act(cx, |o, w, cx| {
            o.settings.projects.empty_changes_tab = EmptyChangesTab::Stashes;
            o.activate(0, w, cx);
        });
        assert_eq!(tab(&h, cx), Tab::Changes);
        h.act(cx, |o, w, cx| o.activate(1, w, cx));
        assert_eq!(tab(&h, cx), Tab::Stashes);
        h.act(cx, |o, w, cx| {
            o.activate(0, w, cx);
            o.settings.projects.empty_changes_tab = EmptyChangesTab::Stay;
            o.activate(1, w, cx);
        });
        assert_eq!(tab(&h, cx), Tab::Changes);
        // A project opened fresh is judged once its first status arrives.
        let c = TempRepo::new();
        c.commit_file("c.txt", "c\n", "first");
        let path = c.path().to_path_buf();
        h.act(cx, |o, w, cx| {
            o.settings.projects.empty_changes_tab = EmptyChangesTab::Files;
            o.activate(0, w, cx);
            o.open_project_notify(&path, w, cx);
        });
        assert_eq!(tab(&h, cx), Tab::Files);
    }

    #[gpui::test]
    fn ui_log_time_shows_the_full_date_on_hover(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a\n", "first");
        let h = Harness::open(cx, &repo);
        h.act(cx, |o, _, cx| o.set_tab(super::Tab::Log, cx));
        let at = h
            .bounds(cx, "commit-time:0")
            .expect("time in the log row")
            .center();
        assert!(h.bounds(cx, "time-tooltip").is_none());
        // The configured delay applies, not gpui's fixed 500 ms.
        h.act(cx, |o, _, cx| {
            o.settings.appearance.tooltip_delay_ms = 1000;
            o.apply_tooltip_delay(cx);
            cx.notify();
        });
        h.visual(cx)
            .simulate_mouse_move(at, None, gpui::Modifiers::none());
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(700));
        cx.run_until_parked();
        assert!(
            h.bounds(cx, "time-tooltip").is_none(),
            "still waiting at 700 ms"
        );
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(400));
        cx.run_until_parked();
        assert!(
            h.bounds(cx, "time-tooltip").is_some(),
            "tooltip after the delay"
        );
        let committed = h.read(cx, |o, _| {
            o.active_project()
                .and_then(|p| p.snapshot.as_ref())
                .map(|s| s.commits[0].committed_at)
                .unwrap_or_default()
        });
        assert!(crate::format::full_time(committed).contains(", "));
    }

    #[gpui::test]
    fn log_badges_sit_before_the_subject_or_at_the_end(cx: &mut TestAppContext) {
        use osier_config::settings::LogBadges;
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a\n", "first");
        repo.git(["tag", "v1"]);
        let h = Harness::open(cx, &repo);
        h.act(cx, |o, _, cx| o.set_tab(super::Tab::Log, cx));
        let badge = |h: &Harness, cx: &mut TestAppContext| {
            h.bounds(cx, "log-ref:0:v1").expect("tag badge").origin.x
        };
        let subject = |h: &Harness, cx: &mut TestAppContext| {
            h.bounds(cx, "log-subject:0").expect("subject").origin.x
        };
        assert!(badge(&h, cx) < subject(&h, cx), "left by default");
        h.act(cx, |o, _, cx| {
            o.settings.git.log_badges = LogBadges::Right;
            cx.notify();
        });
        assert!(badge(&h, cx) > subject(&h, cx), "right when set");
    }

    #[gpui::test]
    fn startup_restores_the_project_that_was_active(cx: &mut TestAppContext) {
        let (a, b, c) = (TempRepo::new(), TempRepo::new(), TempRepo::new());
        for r in [&a, &b, &c] {
            r.commit_file("a.txt", "a\n", "first");
        }
        let state = State {
            open_projects: vec![a.path().into(), b.path().into(), c.path().into()],
            active_project: Some(b.path().into()),
            ..State::default()
        };
        let h = Harness::open_with(cx, state, None);
        h.read(cx, |o, _| {
            assert_eq!(o.projects.len(), 3);
            assert_eq!(
                o.active,
                Some(1),
                "the saved active project, not the last opened"
            );
            let order: Vec<&std::path::Path> =
                o.projects.iter().map(|p| p.path.as_path()).collect();
            assert_eq!(order, [a.path(), b.path(), c.path()], "saved order kept");
        });
    }

    #[gpui::test]
    fn startup_opens_the_active_project_first_and_the_rest_in_the_background(
        cx: &mut TestAppContext,
    ) {
        let repos: Vec<TempRepo> = (0..20)
            .map(|n| {
                let r = TempRepo::new();
                r.commit_file("a.txt", &format!("{n}\n"), "first");
                r
            })
            .collect();
        let gone = std::env::temp_dir().join("osier-test-no-such-repo");
        let mut open_projects: Vec<std::path::PathBuf> =
            repos.iter().map(|r| r.path().to_path_buf()).collect();
        open_projects.insert(5, gone.clone());
        let state = State {
            open_projects,
            active_project: Some(repos[12].path().into()),
            ..State::default()
        };
        let h = Harness::open_with(cx, state, None);
        h.read(cx, |o, _| {
            let paths: Vec<&std::path::Path> =
                o.projects.iter().map(|p| p.path.as_path()).collect();
            let want: Vec<&std::path::Path> = repos.iter().map(|r| r.path()).collect();
            assert_eq!(
                paths, want,
                "all twenty, in saved order, missing one dropped"
            );
            assert_eq!(o.active, Some(12));
            assert!(
                !o.state.open_projects.contains(&gone),
                "missing repo forgotten"
            );
            let error = o.global_error.clone().unwrap_or_default();
            assert!(error.starts_with("Could not reopen a project"), "{error}");
            assert!(
                o.projects.iter().all(|p| p.snapshot.is_some()),
                "all loaded"
            );
        });
    }

    #[gpui::test]
    fn center_pane_keeps_its_saved_width_when_the_window_resizes(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a\n", "first");
        let mut state = State::default();
        state.layout.sidebar_visible = true;
        state.layout.center_width = Some(320.);
        let h = Harness::open_with(cx, state, Some(repo.path().to_path_buf()));
        let width = |h: &Harness, cx: &mut TestAppContext| {
            f32::from(h.bounds(cx, "center-pane").expect("center pane").size.width)
        };
        assert!(
            (width(&h, cx) - 320.).abs() < 2.,
            "restored: {}",
            width(&h, cx)
        );
        for size in [
            gpui::size(gpui::px(1600.), gpui::px(900.)),
            gpui::size(gpui::px(1100.), gpui::px(700.)),
        ] {
            h.visual(cx).simulate_resize(size);
            cx.run_until_parked();
            assert!(
                (width(&h, cx) - 320.).abs() < 2.,
                "after resize: {}",
                width(&h, cx)
            );
        }
    }

    fn log_subjects(h: &Harness, cx: &mut TestAppContext) -> Vec<String> {
        h.read(cx, |o, _| {
            o.active_project()
                .and_then(|p| p.snapshot.as_ref())
                .map(|s| s.commits.iter().map(|c| c.subject.clone()).collect())
                .unwrap_or_default()
        })
    }

    #[gpui::test]
    fn shift_click_pair_stays_on_its_commits_when_new_ones_arrive(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        for s in ["a", "b", "c", "d"] {
            repo.commit_file(&format!("{s}.txt"), "x\n", s);
        }
        let h = Harness::open(cx, &repo);
        h.act(cx, |o, _, cx| o.set_tab(super::Tab::Log, cx));
        h.click(cx, "log-row:1");
        h.click_with(cx, "log-row:2", gpui::Modifiers::shift());
        let picked = |h: &Harness, cx: &mut TestAppContext| {
            h.read(cx, |o, _| {
                let p = o.active_project().unwrap();
                let hash = |ix: Option<usize>| p.commit_at(ix.unwrap()).unwrap().subject.clone();
                (hash(p.selected_commit), hash(p.compare_commit))
            })
        };
        assert_eq!(picked(&h, cx), ("c".to_string(), "b".to_string()));
        // A commit made outside Osier lands on top.
        repo.commit_file("e.txt", "x\n", "e");
        h.act(cx, |o, _, cx| o.refresh_project(0, cx));
        assert_eq!(log_subjects(&h, cx)[0], "e");
        assert_eq!(
            picked(&h, cx),
            ("c".to_string(), "b".to_string()),
            "same commits"
        );
    }

    #[gpui::test]
    fn shift_click_range_of_several_stays_on_its_commits(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        for s in ["a", "b", "c", "d", "e"] {
            repo.commit_file(&format!("{s}.txt"), "x\n", s);
        }
        let h = Harness::open(cx, &repo);
        h.act(cx, |o, _, cx| o.set_tab(super::Tab::Log, cx));
        // Rows 1 to 3: d, c, b.
        h.click(cx, "log-row:1");
        h.click_with(cx, "log-row:3", gpui::Modifiers::shift());
        let run = |h: &Harness, cx: &mut TestAppContext| {
            h.read(cx, |o, _| {
                let p = o.active_project().unwrap();
                let mut rows: Vec<usize> = p.picked_rows().into_iter().collect();
                rows.sort_unstable();
                rows.into_iter()
                    .map(|ix| p.commit_at(ix).unwrap().subject.clone())
                    .collect::<Vec<_>>()
            })
        };
        assert_eq!(run(&h, cx), ["d", "c", "b"]);
        for s in ["f", "g"] {
            repo.commit_file(&format!("{s}.txt"), "x\n", s);
        }
        h.act(cx, |o, _, cx| o.refresh_project(0, cx));
        assert_eq!(log_subjects(&h, cx)[..2], ["g", "f"]);
        assert_eq!(
            run(&h, cx),
            ["d", "c", "b"],
            "the same three commits, two rows lower"
        );
    }

    #[gpui::test]
    fn ui_squash_a_shift_click_range(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        for s in ["a", "b", "c", "d"] {
            repo.commit_file(&format!("{s}.txt"), "x\n", s);
        }
        let h = Harness::open(cx, &repo);
        h.act(cx, |o, _, cx| o.set_tab(super::Tab::Log, cx));
        // Newest to oldest: d, c, b.
        h.click(cx, "log-row:0");
        h.click_with(cx, "log-row:2", gpui::Modifiers::shift());
        h.read(cx, |o, _| {
            assert_eq!(o.active_project().unwrap().picked.len(), 3)
        });
        h.act(cx, |o, w, cx| o.confirm_squash(w, cx));
        assert!(h.has_settings(cx), "squash dialog open");
        assert!(h.bounds(cx, "squash-message").is_some());
        h.click(cx, "dialog-ok");
        assert_eq!(
            log_subjects(&h, cx),
            ["b\n\nc\n\nd".lines().next().unwrap(), "a"]
        );
        h.read(cx, |o, _| {
            let p = o.active_project().unwrap();
            let head = &p.snapshot.as_ref().unwrap().commits[0];
            assert_eq!(head.body.trim(), "c\n\nd", "messages kept, oldest first");
            assert_eq!(
                p.selected_commit,
                Some(0),
                "the squashed commit is selected"
            );
            assert_eq!(p.compare_commit, None);
        });
    }

    #[gpui::test]
    fn squash_of_commits_on_different_lines_is_refused(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "x\n", "a");
        repo.git(["checkout", "-qb", "side"]);
        repo.commit_file("s.txt", "x\n", "side");
        repo.git(["checkout", "-q", "main"]);
        repo.commit_file("b.txt", "x\n", "b");
        let h = Harness::open(cx, &repo);
        h.act(cx, |o, _, cx| o.set_tab(super::Tab::Log, cx));
        // b and side: rows 0 and 1, picked with ctrl-click; side is on another branch.
        h.click(cx, "log-row:0");
        h.click_with(cx, "log-row:1", gpui::Modifiers::control());
        h.read(cx, |o, _| {
            assert_eq!(o.active_project().unwrap().picked.len(), 2)
        });
        h.act(cx, |o, w, cx| o.confirm_squash(w, cx));
        assert!(!h.has_settings(cx), "no dialog");
        assert_eq!(log_subjects(&h, cx).len(), 3, "nothing changed");
    }

    #[gpui::test]
    fn ui_ctrl_click_picks_separate_commits_and_squashes_them(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        for s in ["a", "b", "c", "d", "e"] {
            repo.commit_file(&format!("{s}.txt"), "x\n", s);
        }
        let h = Harness::open(cx, &repo);
        h.act(cx, |o, _, cx| o.set_tab(super::Tab::Log, cx));
        // Rows: e d c b a. Pick d and b (c between), then toggle a in and out again.
        h.click(cx, "log-row:1");
        h.click_with(cx, "log-row:3", gpui::Modifiers::control());
        h.click_with(cx, "log-row:4", gpui::Modifiers::control());
        h.click_with(cx, "log-row:4", gpui::Modifiers::control());
        h.read(cx, |o, _| {
            let p = o.active_project().unwrap();
            let mut rows: Vec<usize> = p.picked_rows().into_iter().collect();
            rows.sort_unstable();
            assert_eq!(rows, [1, 3], "d and b, not c between them");
        });
        // New commits do not move the picks.
        repo.commit_file("f.txt", "x\n", "f");
        h.act(cx, |o, _, cx| o.refresh_project(0, cx));
        h.read(cx, |o, _| {
            let p = o.active_project().unwrap();
            let mut rows: Vec<usize> = p.picked_rows().into_iter().collect();
            rows.sort_unstable();
            assert_eq!(rows, [2, 4]);
        });
        h.act(cx, |o, w, cx| o.confirm_squash(w, cx));
        assert!(h.has_settings(cx), "squash dialog open");
        h.click(cx, "dialog-ok");
        assert_eq!(log_subjects(&h, cx), ["f", "e", "c", "b", "a"]);
        h.read(cx, |o, _| {
            let s = o.active_project().unwrap().snapshot.as_ref().unwrap();
            assert_eq!(
                s.commits[3].body.trim(),
                "d",
                "b's subject, d's message below"
            );
        });
    }

    #[gpui::test]
    fn snapshot_knows_which_commits_are_not_pushed(cx: &mut TestAppContext) {
        let repo = repo_with_upstream(2, false);
        let h = Harness::open(cx, &repo);
        h.read(cx, |o, _| {
            let s = o.active_project().unwrap().snapshot.as_ref().unwrap();
            let marked: Vec<&str> = s
                .commits
                .iter()
                .filter(|c| s.unpushed.contains(&c.hash))
                .map(|c| c.subject.as_str())
                .collect();
            assert_eq!(marked, ["local 1", "local 0"]);
        });
    }

    fn selection(h: &Harness, cx: &mut TestAppContext) -> Selection {
        h.read(cx, |o, _| o.active_project().unwrap().selection.clone())
    }

    #[gpui::test]
    fn ui_each_tab_keeps_its_own_right_pane(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a\n", "first");
        repo.commit_file("b.txt", "b\n", "second");
        repo.write("a.txt", "changed\n");
        repo.write("b.txt", "changed too\n");
        let h = Harness::open(cx, &repo);
        assert_eq!(h.read(cx, |o, _| o.tab), super::Tab::Changes);
        // Changes starts on its first change; pick the second.
        h.click(cx, "change-row:unstaged:b.txt");
        let b = Selection::File {
            path: "b.txt".into(),
            area: Area::Unstaged,
        };
        assert_eq!(selection(&h, cx), b);
        // Log starts on the latest commit, not on Changes' file.
        h.click(cx, "tab-log");
        let latest = h.read(cx, |o, _| {
            o.active_project()
                .unwrap()
                .snapshot
                .as_ref()
                .unwrap()
                .commits[0]
                .hash
                .clone()
        });
        assert_eq!(selection(&h, cx), Selection::Commit { hash: latest });
        h.click(cx, "log-row:1");
        let older = h.read(cx, |o, _| {
            o.active_project()
                .unwrap()
                .commit_at(1)
                .unwrap()
                .hash
                .clone()
        });
        // Tabs with nothing picked show nothing.
        h.click(cx, "tab-stashes");
        assert_eq!(selection(&h, cx), Selection::None);
        // Each tab comes back as it was left.
        h.click(cx, "tab-changes");
        assert_eq!(selection(&h, cx), b);
        h.click(cx, "tab-log");
        assert_eq!(
            selection(&h, cx),
            Selection::Commit {
                hash: older.clone()
            }
        );
        h.read(cx, |o, _| {
            let p = o.active_project().unwrap();
            assert_eq!(p.selected_commit, Some(1));
            assert_eq!(p.picked, [older]);
        });
    }

    #[gpui::test]
    fn an_open_editor_survives_switching_tabs(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a\n", "first");
        repo.write("a.txt", "changed\n");
        let h = Harness::open(cx, &repo);
        h.act(cx, |o, w, cx| o.open_editor(w, cx));
        h.act(cx, |o, w, cx| {
            let editor = o.editor.as_ref().expect("editor open");
            editor
                .state
                .update(cx, |s, cx| s.set_value("typed but not saved\n", w, cx));
        });
        h.click(cx, "tab-log");
        h.read(cx, |o, _| {
            assert!(o.editor.is_none(), "Log shows its commit")
        });
        h.click(cx, "tab-changes");
        h.read(cx, |o, cx| {
            let editor = o.editor.as_ref().expect("editor back");
            assert_eq!(
                editor.state.read(cx).value().to_string(),
                "typed but not saved\n"
            );
        });
    }

    #[gpui::test]
    fn an_open_editor_comes_back_with_its_project(cx: &mut TestAppContext) {
        let (h, a, b, _first, _second) = two_projects(cx);
        a.write("a.txt", "a changed\n");
        b.write("b.txt", "b changed\n");
        h.act(cx, |o, _, cx| {
            o.refresh_project(0, cx);
            o.refresh_project(1, cx);
        });
        h.act(cx, |o, w, cx| {
            o.activate(0, w, cx);
            o.set_tab(super::Tab::Changes, cx);
        });
        h.act(cx, |o, w, cx| o.open_editor(w, cx));
        h.read(cx, |o, _| {
            assert_eq!(o.editor.as_ref().unwrap().path, "a.txt")
        });
        h.act(cx, |o, w, cx| o.activate(1, w, cx));
        h.read(cx, |o, _| {
            assert!(o.editor.is_none(), "not on the other project")
        });
        h.act(cx, |o, w, cx| o.activate(0, w, cx));
        h.read(cx, |o, _| {
            assert_eq!(o.editor.as_ref().map(|e| e.path.as_str()), Some("a.txt"))
        });
    }

    #[gpui::test]
    fn a_tab_gets_its_own_diff_view_back_unless_the_content_moved(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a\n", "first");
        repo.commit_file("b.txt", "b\n", "second");
        repo.write("a.txt", "changed\n");
        let h = Harness::open(cx, &repo);
        let view = |h: &Harness, cx: &mut TestAppContext| h.read(cx, |o, _| o.diff.entity_id());
        let changes_view = view(&h, cx);
        h.click(cx, "tab-log");
        let log_view = view(&h, cx);
        assert_ne!(log_view, changes_view, "each tab has its own view");
        // Back and forth with nothing changed: the same views, scroll and all.
        h.click(cx, "tab-changes");
        assert_eq!(view(&h, cx), changes_view);
        assert_eq!(h.read(cx, |o, _| o.diff_files), 1);
        h.click(cx, "tab-log");
        assert_eq!(view(&h, cx), log_view);
        // The working tree moves while Changes is away: its diff is loaded fresh.
        repo.write("a.txt", "changed again\n");
        h.act(cx, |o, _, cx| o.refresh_project(0, cx));
        assert_eq!(view(&h, cx), log_view, "a fixed commit stays as it was");
        h.click(cx, "tab-changes");
        assert_ne!(view(&h, cx), changes_view, "reloaded");
        assert_eq!(h.read(cx, |o, _| o.diff_files), 1);
    }

    #[gpui::test]
    fn the_shown_diff_follows_edits_to_its_file_and_ignores_other_files(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a\n", "first");
        repo.commit_file("b.txt", "b\n", "second");
        repo.write("a.txt", "one\n");
        repo.write("b.txt", "other\n");
        let h = Harness::open(cx, &repo);
        h.click(cx, "change-row:unstaged:a.txt");
        let loads = |h: &Harness, cx: &mut TestAppContext| h.read(cx, |o, _| o.diff_revision);
        let added = |h: &Harness, cx: &mut TestAppContext| {
            h.read(cx, |o, _| {
                o.diff_model.iter().map(|f| f.added()).sum::<usize>()
            })
        };
        assert_eq!(added(&h, cx), 1);
        // Same status (a.txt stays modified), new content: the diff follows.
        let before = loads(&h, cx);
        repo.write("a.txt", "one\ntwo\nthree\n");
        h.act(cx, |o, _, cx| o.refresh_project(0, cx));
        assert!(loads(&h, cx) > before, "reloaded");
        assert_eq!(added(&h, cx), 3);
        // Another file changes: the shown diff (and its scroll) is left alone.
        let before = loads(&h, cx);
        repo.write("b.txt", "other, edited\n");
        h.act(cx, |o, _, cx| o.refresh_project(0, cx));
        assert_eq!(loads(&h, cx), before, "not reloaded");
    }

    #[gpui::test]
    fn switching_project_drops_the_cached_diff_views(cx: &mut TestAppContext) {
        let (h, _a, _b, _first, _second) = two_projects(cx);
        h.act(cx, |o, w, cx| o.activate(0, w, cx));
        h.click(cx, "tab-log");
        h.click(cx, "tab-branches");
        h.read(cx, |o, _| {
            let p = o.active_project().unwrap();
            assert!(p.panes.values().any(|pane| pane.diff.is_some()));
        });
        h.act(cx, |o, w, cx| o.activate(1, w, cx));
        h.read(cx, |o, _| {
            assert!(o.projects[0].panes.values().all(|pane| pane.diff.is_none()));
        });
    }

    #[gpui::test]
    fn clicking_a_stash_shows_its_changes(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a\n", "first");
        repo.write("a.txt", "stashed\n");
        repo.git(["stash", "push", "-q", "-m", "wip"]);
        let h = Harness::open(cx, &repo);
        h.click(cx, "tab-stashes");
        assert_eq!(selection(&h, cx), Selection::None);
        let oid = h.read(cx, |o, _| {
            o.active_project()
                .unwrap()
                .snapshot
                .as_ref()
                .unwrap()
                .stashes[0]
                .oid
                .clone()
        });
        h.act(cx, |o, _, cx| o.select_stash(oid.clone(), cx));
        assert_eq!(
            selection(&h, cx),
            Selection::Compare {
                from: format!("{oid}^1"),
                to: oid,
                since_merge_base: false
            }
        );
        h.read(cx, |o, _| assert_eq!(o.diff_files, 1, "the stashed file"));
    }

    /// Hover the element with test id `id` until its tooltip shows; the tooltip's text.
    fn hover_text(h: &Harness, cx: &mut TestAppContext, id: &str, tooltip: &'static str) -> bool {
        let at = h
            .bounds(cx, id)
            .unwrap_or_else(|| panic!("no {id}"))
            .center();
        h.visual(cx)
            .simulate_mouse_move(at, None, gpui::Modifiers::none());
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(2));
        cx.run_until_parked();
        h.bounds(cx, tooltip).is_some()
    }

    #[gpui::test]
    fn ui_files_folder_marker_follows_the_sidebar_style_and_explains_itself(
        cx: &mut TestAppContext,
    ) {
        use osier_config::settings::ChangesIndicator;
        let repo = TempRepo::new();
        repo.commit_file("dir/sub/a.txt", "a\n", "first");
        repo.commit_file("other/c.txt", "c\n", "second");
        repo.write("dir/sub/a.txt", "changed\n");
        repo.write("dir/b.txt", "new\n");
        let h = Harness::open(cx, &repo);
        h.click(cx, "tab-files");
        // Icon style (the default): the folder icon carries the marker.
        assert!(h.bounds(cx, "files-dirty:dir").is_some());
        assert!(
            h.bounds(cx, "files-dirty:other").is_none(),
            "clean folder unmarked"
        );
        assert!(hover_text(
            &h,
            cx,
            "files-dirty:dir",
            "files-change-tooltip"
        ));
        h.read(cx, |o, _| {
            assert_eq!(
                o.settings.appearance.changes_indicator,
                ChangesIndicator::Icon
            )
        });
        // Dot style: a dot in the sidebar's colour, same tooltip.
        h.act(cx, |o, _, cx| {
            o.settings.appearance.changes_indicator = ChangesIndicator::Dot;
            cx.notify();
        });
        h.visual(cx).simulate_mouse_move(
            gpui::point(gpui::px(1.), gpui::px(1.)),
            None,
            gpui::Modifiers::none(),
        );
        cx.run_until_parked();
        assert!(hover_text(
            &h,
            cx,
            "files-dirty:dir",
            "files-change-tooltip"
        ));
        let count = h.read(cx, |o, _| {
            let s = o.active_project().unwrap().snapshot.as_ref().unwrap();
            s.status
                .entries
                .iter()
                .filter(|e| e.path.starts_with("dir/"))
                .count()
        });
        assert_eq!(count, 2, "two changed files inside dir");
    }

    #[gpui::test]
    fn higher_contrast_strengthens_selections_and_switches_back_cleanly(cx: &mut TestAppContext) {
        use gpui_kit::component::ActiveTheme as _;
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a\n", "first");
        let h = Harness::open(cx, &repo);
        // The colours widgets actually draw with (`tokens`), not only the plain fields.
        let colors = |h: &Harness, cx: &mut TestAppContext| {
            h.read(cx, |_, cx| {
                let t = cx.theme();
                (
                    t.tokens.list_active.color,
                    t.tokens.sidebar_accent.color,
                    t.tokens.list_hover.color,
                )
            })
        };
        let normal = colors(&h, cx);
        // Applied twice, as startup does: the second must not undo the first.
        h.act(cx, |o, _, cx| {
            o.settings.appearance.high_contrast = true;
            o.apply_appearance(cx);
            o.apply_appearance(cx);
        });
        let strong = colors(&h, cx);
        assert_ne!(strong.0, normal.0);
        assert_ne!(strong.1, normal.1);
        assert_eq!((strong.0.a, strong.1.a, strong.2.a), (1., 1., 1.), "opaque");
        h.act(cx, |o, _, cx| {
            o.settings.appearance.high_contrast = false;
            o.apply_appearance(cx);
        });
        assert_eq!(colors(&h, cx), normal, "off is the theme as it was");
    }

    #[gpui::test]
    fn ui_commit_details_panel_shows_toggles_and_remembers(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a\n", "first");
        repo.commit_file("b.txt", "b\n", "second");
        let h = Harness::open(cx, &repo);
        h.click(cx, "tab-log");
        assert!(h.bounds(cx, "commit-details").is_some(), "shown by default");
        let latest = h.read(cx, |o, _| {
            o.active_project()
                .unwrap()
                .snapshot
                .as_ref()
                .unwrap()
                .commits[0]
                .hash
                .clone()
        });
        h.read(cx, |o, _| {
            assert_eq!(
                o.commit_branches.get(&latest).map(Vec::as_slice),
                Some(&["main".to_string()][..])
            )
        });
        // The header button hides it, the key brings it back; the choice is saved.
        h.click(cx, "toggle-commit-details");
        assert!(h.bounds(cx, "commit-details").is_none());
        h.read(cx, |o, _| assert!(o.state.layout.log_details_hidden));
        h.keys(cx, "ctrl-shift-d");
        assert!(h.bounds(cx, "commit-details").is_some());
        h.read(cx, |o, _| assert!(!o.state.layout.log_details_hidden));
    }

    #[gpui::test]
    fn ui_open_repository_moved_to_ctrl_shift_o(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "one\n", "first");
        let h = Harness::open(cx, &repo);
        h.read(cx, |_, cx| {
            let keymap = crate::keymap::Keymap::global(cx);
            assert_eq!(
                keymap.keys_for("open_repository"),
                vec!["ctrl-shift-o".to_string()]
            );
            assert_eq!(
                keymap.keys_for("switch_project"),
                vec!["ctrl-o".to_string()]
            );
        });
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
    fn squash_over_uncommitted_changes_stashes_them_while_it_runs(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        for f in ["a", "b", "c", "d"] {
            repo.commit_file(&format!("{f}.txt"), &format!("{f}\n"), f);
        }
        repo.write("a.txt", "edited\n");
        let h = Harness::open(cx, &repo);
        // Squashing HEAD~2 and HEAD~1 replays HEAD over the uncommitted edit.
        let picked: Vec<String> = ["HEAD~1", "HEAD~2"]
            .iter()
            .map(|r| repo.git(["rev-parse", r]).trim().to_string())
            .collect();
        h.act(cx, move |o, w, cx| {
            o.active_project_mut().expect("project").picked = picked;
            o.confirm_squash(w, cx);
        });
        h.click(cx, "dialog-ok");
        assert_eq!(repo.git(["rev-list", "--count", "HEAD"]).trim(), "3");
        assert_eq!(repo.read("a.txt"), "edited\n", "the edit is put back");
        assert!(repo.git(["stash", "list"]).trim().is_empty());
    }

    #[gpui::test]
    fn switching_over_conflicting_changes_offers_to_save_them(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a\n", "base");
        repo.checkout_new("other");
        repo.commit_file("a.txt", "other\n", "other");
        repo.checkout("main");
        repo.write("a.txt", "mine\n");
        let h = Harness::open(cx, &repo);
        h.act(cx, |o, w, cx| {
            o.checkout_branch("other".into(), false, w, cx)
        });
        assert_eq!(repo.git(["branch", "--show-current"]).trim(), "main");
        h.click(cx, "dialog-ok");
        assert_eq!(repo.git(["branch", "--show-current"]).trim(), "other");
        assert!(
            repo.git(["stash", "list"])
                .contains("Saved before switching to other"),
            "the edit is saved, not brought along"
        );
        assert_eq!(repo.read("a.txt"), "other\n");
    }

    fn bind(cx: &mut TestAppContext, pairs: &[(&str, &[&str])]) {
        let mut file = osier_config::keymap::KeymapFile::default();
        for (id, keys) in pairs {
            file.bindings
                .insert(id.to_string(), keys.iter().map(|k| k.to_string()).collect());
        }
        cx.update(|cx| crate::keymap::apply(&file, cx));
    }

    fn filter_focused(h: &Harness, cx: &mut TestAppContext) -> bool {
        focused_is(h, cx, |o, cx| {
            gpui_kit::Focusable::focus_handle(o.sidebar_filter.read(cx), cx)
        })
    }

    #[gpui::test]
    fn ui_prefix_key_waits_shows_a_badge_and_runs_the_next_key(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a\n", "first");
        let h = Harness::open(cx, &repo);
        bind(
            cx,
            &[
                ("prefix_key", &["ctrl-h"]),
                ("focus_project_search", &["prefix /"]),
                ("focus_file_search", &["prefix? ?"]),
            ],
        );
        let badge = |h: &Harness, cx: &mut TestAppContext| h.bounds(cx, "prefix-badge").is_some();

        // Prefix only: `/` alone does nothing, prefix then `/` searches.
        h.keys(cx, "/");
        assert!(
            !filter_focused(&h, cx),
            "prefix-only key ignored on its own"
        );
        h.keys(cx, "ctrl-h");
        assert!(badge(&h, cx), "badge while waiting");
        h.keys(cx, "/");
        assert!(!badge(&h, cx));
        assert!(filter_focused(&h, cx));

        // While typing in a text field: the prefixed key runs instead of typing.
        h.keys(cx, "ctrl-h ?");
        assert_eq!(
            input_text(&h, cx, |o| &o.sidebar_filter),
            "",
            "nothing typed"
        );
        h.read(cx, |o, _| assert_eq!(o.tab, super::Tab::Files));

        // Either: works alone too (from outside a text field).
        h.act(cx, |o, w, cx| {
            o.set_tab(super::Tab::Changes, cx);
            o.focus_handle.focus(w, cx);
        });
        h.keys(cx, "?");
        h.read(cx, |o, _| assert_eq!(o.tab, super::Tab::Files));
    }

    #[gpui::test]
    fn ui_prefix_cancels_on_escape_click_unknown_key_and_timeout(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a\n", "first");
        let h = Harness::open(cx, &repo);
        bind(
            cx,
            &[
                ("prefix_key", &["ctrl-h"]),
                ("focus_project_search", &["prefix /"]),
            ],
        );
        let waiting = |h: &Harness, cx: &mut TestAppContext| h.read(cx, |o, _| o.prefix_waiting());

        h.keys(cx, "ctrl-h escape");
        assert!(!waiting(&h, cx), "escape");
        h.keys(cx, "/");
        assert!(!filter_focused(&h, cx), "the prefix was really cancelled");

        h.keys(cx, "ctrl-h");
        h.click(cx, "tab-log");
        assert!(!waiting(&h, cx), "click");

        h.keys(cx, "ctrl-h z");
        assert!(!waiting(&h, cx), "unbound key");

        h.keys(cx, "ctrl-h");
        assert!(waiting(&h, cx));
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(4));
        cx.run_until_parked();
        assert!(waiting(&h, cx), "still waiting at 4 s");
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(2));
        cx.run_until_parked();
        assert!(!waiting(&h, cx), "timed out after 5 s");
    }

    #[gpui::test]
    fn ui_context_menu_keys_fire_on_the_selected_row(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a\n", "first");
        repo.write("notes.tmp", "x\n");
        let h = Harness::open(cx, &repo);
        bind(cx, &[("row_add_to_gitignore", &["i"])]);
        h.click(cx, "change-row:untracked:notes.tmp");
        h.keys(cx, "i");
        let gitignore = std::fs::read_to_string(repo.path().join(".gitignore")).unwrap_or_default();
        assert_eq!(gitignore, "/notes.tmp\n");
    }

    #[gpui::test]
    fn ui_branch_tag_and_stash_rows_select_and_take_menu_keys(cx: &mut TestAppContext) {
        use crate::row_actions::RefRow;
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a\n", "first");
        repo.git(["branch", "feature"]);
        repo.git(["tag", "v1"]);
        repo.write("a.txt", "changed\n");
        repo.git(["stash", "-q"]);
        let h = Harness::open(cx, &repo);
        bind(
            cx,
            &[
                ("row_checkout", &["c"]),
                ("row_delete", &["x"]),
                ("row_copy_name", &["n"]),
            ],
        );
        let head = |h: &Harness, cx: &mut TestAppContext| {
            h.read(cx, |o, _| {
                o.active_project()
                    .and_then(|p| p.snapshot.as_ref())
                    .and_then(|s| s.status.branch.head.clone())
            })
        };

        // Click selects; the key runs the entry on that row.
        h.click(cx, "tab-branches");
        h.click(cx, "branch-row:feature");
        h.read(cx, |o, _| {
            assert_eq!(
                o.ref_selected,
                Some(RefRow::Branch {
                    name: "feature".into(),
                    remote: false
                })
            )
        });
        h.keys(cx, "c");
        assert_eq!(head(&h, cx).as_deref(), Some("feature"));

        // Arrow keys walk branches (feature, main), then tags.
        h.keys(cx, "down");
        h.read(cx, |o, _| {
            assert!(matches!(
                &o.ref_selected,
                Some(RefRow::Branch { remote: false, .. })
            ))
        });
        h.keys(cx, "down");
        h.read(cx, |o, _| {
            assert_eq!(o.ref_selected, Some(RefRow::Tag("v1".into())))
        });
        h.keys(cx, "n");
        assert_eq!(
            cx.read_from_clipboard().and_then(|c| c.text()).as_deref(),
            Some("v1")
        );

        // Stashes: select, then Delete / Drop asks first.
        h.click(cx, "tab-stashes");
        h.click(cx, "stash-row:0");
        h.keys(cx, "x");
        assert!(h.has_settings(cx), "drop asks for confirmation");
        h.click(cx, "dialog-ok");
        assert!(
            repo.git(["stash", "list"]).trim().is_empty(),
            "stash dropped"
        );
    }

    #[gpui::test]
    fn menu_entries_show_their_key(cx: &mut TestAppContext) {
        use crate::row_actions::{RowKind, key_hint};
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a\n", "first");
        let _h = Harness::open(cx, &repo);
        assert_eq!(cx.update(|cx| key_hint(RowKind::AddToGitignore, cx)), None);
        bind(
            cx,
            &[
                ("row_add_to_gitignore", &["i"]),
                ("row_untrack", &["prefix u"]),
            ],
        );
        assert_eq!(
            cx.update(|cx| key_hint(RowKind::AddToGitignore, cx))
                .as_deref(),
            Some("I")
        );
        assert_eq!(
            cx.update(|cx| key_hint(RowKind::Untrack, cx)).as_deref(),
            Some("Prefix U")
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
            assert_eq!(
                osier.settings.appearance.dux_shell_sync, None,
                "auto by default"
            );
            assert!(!osier.dux_sync_on(cx), "not detected: off");
        });
        // Detected and never chosen: on by default.
        h.act(cx, |osier, _, cx| {
            ThemeStore::global_mut(cx).set_dux_source(DuxSource::File(colors.clone()));
            osier.apply_appearance(cx);
        });
        h.read(cx, |osier, cx| {
            assert!(osier.dux_sync_on(cx));
            // Fresh installs start in System mode, so the shell's mode (light here) decides.
            assert_eq!(
                osier.settings.appearance.mode,
                osier_config::settings::ThemeMode::System
            );
            assert_eq!(cx.theme().theme_name().as_ref(), "Dux Shell Light");
        });
        // Turning it on sets System, so the shell's mode (light here) decides.
        h.act(cx, |osier, _, cx| osier.set_dux_sync(true, cx));
        h.read(cx, |osier, cx| {
            assert_eq!(
                osier.settings.appearance.mode,
                osier_config::settings::ThemeMode::System
            );
            assert_eq!(osier.settings.appearance.dux_shell_sync, Some(true));
            assert_eq!(cx.theme().theme_name().as_ref(), "Dux Shell Light");
        });
        // The user can still pick Dark by hand: the Dux dark colours.
        h.act(cx, |osier, _, cx| {
            osier.set_theme_mode(osier_config::settings::ThemeMode::Dark, cx)
        });
        h.read(cx, |_, cx| {
            assert_eq!(cx.theme().theme_name().as_ref(), "Dux Shell Dark");
            let rgb = cx.theme().background.to_rgb();
            assert_eq!((rgb.r * 255.0).round() as u8, 0x10);
            assert_eq!((rgb.b * 255.0).round() as u8, 0x18);
            let names = ThemeStore::global(cx).names(true);
            assert!(!names.iter().any(|n| n.as_ref() == "Dux Shell Dark"));
        });
        // Turning it off leaves the mode alone.
        h.act(cx, |osier, _, cx| osier.set_dux_sync(false, cx));
        h.read(cx, |osier, _| {
            assert_eq!(
                osier.settings.appearance.mode,
                osier_config::settings::ThemeMode::Dark
            )
        });
        // Picking an explicit theme turns sync off.
        h.act(cx, |osier, _, cx| {
            osier.set_dux_sync(true, cx);
            osier.set_theme("Catppuccin Mocha", cx)
        });
        h.read(cx, |osier, cx| {
            assert_eq!(cx.theme().theme_name().as_ref(), "Catppuccin Mocha");
            assert_eq!(osier.settings.appearance.dux_shell_sync, Some(false));
        });
    }

    #[gpui::test]
    fn fresh_installs_follow_the_system_and_default_to_mocha_without_an_answer(
        cx: &mut TestAppContext,
    ) {
        use crate::system_scheme::SystemScheme;
        use gpui_kit::component::ActiveTheme as _;
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "one\n", "first");
        let h = Harness::open(cx, &repo);
        let theme = |h: &Harness, cx: &mut TestAppContext| {
            h.read(cx, |_, cx| cx.theme().theme_name().to_string())
        };
        h.read(cx, |o, _| {
            assert_eq!(
                o.settings.appearance.mode,
                osier_config::settings::ThemeMode::System
            )
        });
        // No Dux Shell, no answer from the desktop: Catppuccin Mocha.
        assert_eq!(theme(&h, cx), "Catppuccin Mocha");
        let set = |h: &Harness, cx: &mut TestAppContext, s: SystemScheme| {
            h.act(cx, move |o, _, cx| {
                o.system_scheme = s;
                o.apply_appearance(cx);
            })
        };
        set(&h, cx, SystemScheme::Light);
        assert_eq!(theme(&h, cx), "Catppuccin Latte");
        set(&h, cx, SystemScheme::Dark);
        assert_eq!(theme(&h, cx), "Catppuccin Mocha");
        set(&h, cx, SystemScheme::NoPreference);
        assert_eq!(theme(&h, cx), "Catppuccin Mocha", "fallback is dark");
        h.act(cx, |o, _, cx| {
            o.settings.appearance.system_fallback = osier_config::settings::SystemFallback::Light;
            o.apply_appearance(cx);
        });
        assert_eq!(theme(&h, cx), "Catppuccin Latte");
    }

    #[gpui::test]
    fn ui_theme_cards_pick_a_theme_unless_dux_sync_is_on(cx: &mut TestAppContext) {
        use crate::themes::{DuxSource, ThemeStore};
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "one\n", "first");
        let h = Harness::open(cx, &repo);
        let other = h.read(cx, |o, cx| {
            ThemeStore::global(cx)
                .names(true)
                .into_iter()
                .find(|n| n.as_ref() != o.settings.appearance.dark_theme)
                .expect("a second dark theme")
                .to_string()
        });
        h.keys(cx, "ctrl-,");
        h.click(cx, &format!("theme-card:{other}"));
        h.read(cx, |o, _| {
            assert_eq!(o.settings.appearance.dark_theme, other)
        });

        // With Dux Shell colours followed, the cards do nothing.
        let dms = tempfile::tempdir().expect("tempdir");
        let colors = dms.path().join("dms-colors.json");
        std::fs::write(
            &colors,
            r##"{"mode":"dark","colors":{"dark":{"background":"#101418"},"light":{}},"dank16":{}}"##,
        )
        .expect("write");
        h.act(cx, |o, _, cx| {
            ThemeStore::global_mut(cx).set_dux_source(DuxSource::File(colors.clone()));
            o.set_dux_sync(true, cx);
            cx.notify();
        });
        h.click(cx, "theme-card:Catppuccin Mocha");
        h.read(cx, |o, _| {
            assert_eq!(o.settings.appearance.dark_theme, other, "disabled")
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

    // ----- clone and create (`new_repo.rs`) -----

    /// GitHub as the clone and create flows see it, with nothing on the network.
    #[derive(Default)]
    struct FakeHub {
        repos: Vec<osier_auth::hub::GhRepo>,
        /// Where a created repository can be cloned from (a local bare repo).
        created_url: String,
        created: std::sync::Mutex<Option<osier_auth::hub::NewRepo>>,
    }

    impl crate::new_repo::Hub for FakeHub {
        fn repos(&self, _: &osier_auth::GhAccount) -> Result<Vec<osier_auth::hub::GhRepo>, String> {
            Ok(self.repos.clone())
        }
        fn orgs(&self, _: &osier_auth::GhAccount) -> Result<Vec<String>, String> {
            Ok(vec!["acme".into()])
        }
        fn templates(
            &self,
            _: &osier_auth::GhAccount,
            owners: &[String],
        ) -> Result<Vec<String>, String> {
            Ok(owners.iter().map(|o| format!("{o}/starter")).collect())
        }
        fn gitignore_templates(&self, _: &osier_auth::GhAccount) -> Result<Vec<String>, String> {
            Ok(vec!["Rust".into(), "Node".into()])
        }
        fn licenses(
            &self,
            _: &osier_auth::GhAccount,
        ) -> Result<Vec<osier_auth::hub::License>, String> {
            Ok(vec![osier_auth::hub::License {
                key: "mit".into(),
                name: "MIT License".into(),
            }])
        }
        fn gitignore_template(
            &self,
            _: &osier_auth::GhAccount,
            name: &str,
        ) -> Result<String, String> {
            Ok(format!("# {name}\n/target\n"))
        }
        fn license_text(
            &self,
            _: &osier_auth::GhAccount,
            _: &str,
            year: i32,
            holder: &str,
        ) -> Result<String, String> {
            Ok(format!("MIT {year} {holder}\n"))
        }
        fn create(
            &self,
            _: &osier_auth::GhAccount,
            new: &osier_auth::hub::NewRepo,
        ) -> Result<osier_auth::hub::CreatedRepo, String> {
            *self.created.lock().expect("lock") = Some(new.clone());
            Ok(osier_auth::hub::CreatedRepo {
                full_name: format!("{}/{}", new.owner, new.name),
                clone_url: self.created_url.clone(),
                html_url: String::new(),
            })
        }
    }

    fn gh_account(login: &str) -> osier_auth::GhAccount {
        osier_auth::GhAccount {
            host: "github.com".into(),
            login: login.into(),
            active: true,
            git_protocol: "https".into(),
            state: "success".into(),
        }
    }

    fn with_hub(h: &Harness, cx: &mut TestAppContext, hub: Arc<FakeHub>) {
        h.act(cx, move |o, _, _| {
            o.gh_accounts = vec![gh_account("alice")];
            o.gh_accounts_loaded = true;
            o.hub = hub;
        });
    }

    /// Replace the text of the input with test id `id`.
    fn retype(h: &Harness, cx: &mut TestAppContext, id: &str, text: &str) {
        h.click(cx, id);
        h.keys(cx, "ctrl-a backspace");
        h.type_text(cx, text);
    }

    fn active_path(h: &Harness, cx: &mut TestAppContext) -> Option<std::path::PathBuf> {
        h.read(cx, |o, _| o.active_project().map(|p| p.path.clone()))
    }

    fn same_dir(a: &std::path::Path, b: &std::path::Path) -> bool {
        std::fs::canonicalize(a).ok() == std::fs::canonicalize(b).ok()
    }

    #[gpui::test]
    fn ui_clone_a_repository_of_an_account(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a\n", "first");
        let source = TempRepo::new();
        source.commit_file("hello.txt", "hi\n", "hello");
        let bare = source.bare_clone();
        let mut gh_repo = crate::new_repo::tests::repo("alice", "hello");
        // A local address: no credentials are involved.
        gh_repo.clone_url = bare.to_string_lossy().into_owned();
        let hub = Arc::new(FakeHub {
            repos: vec![gh_repo, crate::new_repo::tests::repo("acme", "other")],
            ..Default::default()
        });
        let h = Harness::open(cx, &repo);
        with_hub(&h, cx, hub);
        // The palette entry and its account submenu.
        h.keys(cx, "ctrl-p");
        h.type_text(cx, "clone");
        h.click(cx, "palette-command:Clone repository…");
        assert!(h.bounds(cx, "palette-choice:alice").is_some());
        assert!(h.bounds(cx, "palette-choice:Other…").is_some());
        h.click(cx, "palette-choice:alice");
        assert!(h.bounds(cx, "palette-choice:hello").is_some());
        assert!(h.bounds(cx, "palette-choice:other").is_some());
        // Backspace goes back to the accounts, not the commands.
        h.keys(cx, "backspace");
        assert!(h.bounds(cx, "palette-choice:alice").is_some());
        h.click(cx, "palette-choice:alice");
        h.type_text(cx, "hel");
        assert!(h.bounds(cx, "palette-choice:other").is_none());
        h.keys(cx, "enter");
        // The directory dialog starts next to the open project.
        assert!(h.bounds(cx, "clone-form").is_some());
        assert!(h.bounds(cx, "clone-url").is_none());
        assert!(h.bounds(cx, "clone-choose-dir").is_some());
        let dest = tempfile::tempdir().expect("tempdir");
        retype(&h, cx, "clone-dir", &dest.path().to_string_lossy());
        h.click(cx, "dialog-ok");
        let target = dest.path().join("hello");
        assert!(target.join("hello.txt").exists(), "cloned");
        let path = active_path(&h, cx).expect("opened");
        assert!(same_dir(&path, &target));
        let key = path.to_string_lossy().into_owned();
        h.read(cx, |o, _| {
            assert_eq!(
                o.state.accounts.get(&key).map(String::as_str),
                Some("alice")
            );
            assert!(o.remote_job.is_none());
        });
    }

    #[gpui::test]
    fn ui_clone_any_address_and_refuse_a_full_directory(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a\n", "first");
        let source = TempRepo::new();
        source.commit_file("x.txt", "x\n", "x");
        let bare = source.bare_clone();
        let h = Harness::open(cx, &repo);
        with_hub(&h, cx, Arc::new(FakeHub::default()));
        h.act(cx, |o, w, cx| {
            o.open_palette_in(crate::palette::PaletteMode::CloneAccount, w, cx)
        });
        h.click(cx, "palette-choice:Other…");
        assert!(h.bounds(cx, "clone-url").is_some());
        // The address field has focus.
        h.type_text(cx, &bare.to_string_lossy());
        let dest = tempfile::tempdir().expect("tempdir");
        let name = bare
            .file_name()
            .map(|n| n.to_string_lossy().trim_end_matches(".git").to_string())
            .expect("name");
        std::fs::create_dir(dest.path().join(&name)).expect("mkdir");
        std::fs::write(dest.path().join(&name).join("keep.txt"), "mine").expect("write");
        retype(&h, cx, "clone-dir", &dest.path().to_string_lossy());
        assert!(
            h.bounds(cx, "new-repo-error").is_some(),
            "full target flagged"
        );
        h.click(cx, "dialog-ok");
        assert!(h.has_settings(cx), "the dialog stays open");
        assert_eq!(
            std::fs::read_to_string(dest.path().join(&name).join("keep.txt")).expect("kept"),
            "mine"
        );
        // Into a fresh directory, without submodules.
        let fresh = dest.path().join("fresh");
        std::fs::create_dir(&fresh).expect("mkdir");
        retype(&h, cx, "clone-dir", &fresh.to_string_lossy());
        h.click(cx, "clone-recurse");
        h.click(cx, "dialog-ok");
        assert!(!h.has_settings(cx));
        let target = fresh.join(&name);
        assert!(target.join("x.txt").exists());
        assert!(same_dir(&active_path(&h, cx).expect("opened"), &target));
        // No account was picked, so none is remembered.
        h.read(cx, |o, _| assert!(o.state.accounts.is_empty()));
    }

    #[gpui::test]
    fn ui_create_a_local_repository(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a\n", "first");
        let h = Harness::open(cx, &repo);
        with_hub(&h, cx, Arc::new(FakeHub::default()));
        h.keys(cx, "ctrl-p");
        h.type_text(cx, "create new");
        h.keys(cx, "enter");
        assert!(h.bounds(cx, "palette-choice:alice").is_some());
        h.click(cx, "palette-choice:No remote");
        assert!(h.bounds(cx, "create-form").is_some());
        // GitHub-only fields are hidden.
        assert!(h.bounds(cx, "create-owner").is_none());
        assert!(h.bounds(cx, "create-template").is_none());
        assert!(h.bounds(cx, "create-gitignore").is_some());
        // The name field has focus; a bad name is refused.
        h.type_text(cx, "bad/name");
        h.click(cx, "dialog-ok");
        assert!(h.has_settings(cx));
        assert!(h.bounds(cx, "new-repo-error").is_some());
        retype(&h, cx, "create-name", "fresh");
        retype(&h, cx, "create-description", "A new thing");
        let dest = tempfile::tempdir().expect("tempdir");
        retype(&h, cx, "create-dir", &dest.path().to_string_lossy());
        h.click(cx, "dialog-ok");
        assert!(!h.has_settings(cx));
        let target = dest.path().join("fresh");
        assert!(target.join(".git").is_dir());
        assert_eq!(
            std::fs::read_to_string(target.join("README.md")).expect("readme"),
            "# fresh\n\nA new thing\n"
        );
        assert!(!target.join(".gitignore").exists(), "None was picked");
        assert!(same_dir(&active_path(&h, cx).expect("opened"), &target));
    }

    #[gpui::test]
    fn ui_create_on_github_then_clone(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a\n", "first");
        // What GitHub would have made: a repository with a README.
        let made = TempRepo::new();
        made.commit_file("README.md", "# shiny\n", "Initial commit");
        let bare = made.bare_clone();
        let hub = Arc::new(FakeHub {
            created_url: bare.to_string_lossy().into_owned(),
            ..Default::default()
        });
        let h = Harness::open(cx, &repo);
        with_hub(&h, cx, hub.clone());
        h.act(cx, |o, w, cx| {
            o.open_palette_in(crate::palette::PaletteMode::CreateAccount, w, cx)
        });
        h.click(cx, "palette-choice:alice");
        assert!(h.bounds(cx, "create-owner").is_some());
        assert!(h.bounds(cx, "create-template").is_some());
        assert!(h.bounds(cx, "create-status").is_none(), "lists loaded");
        h.type_text(cx, "shiny");
        let dest = tempfile::tempdir().expect("tempdir");
        retype(&h, cx, "create-dir", &dest.path().to_string_lossy());
        h.click(cx, "create-public");
        h.click(cx, "dialog-ok");
        let sent = hub.created.lock().expect("lock").clone().expect("created");
        assert_eq!(sent.owner, "alice");
        assert_eq!(sent.name, "shiny");
        assert!(!sent.private && sent.readme && sent.template.is_none());
        let target = dest.path().join("shiny");
        assert!(target.join("README.md").exists(), "cloned after creating");
        let path = active_path(&h, cx).expect("opened");
        assert!(same_dir(&path, &target));
        let key = path.to_string_lossy().into_owned();
        h.read(cx, |o, _| {
            assert_eq!(
                o.state.accounts.get(&key).map(String::as_str),
                Some("alice")
            )
        });
    }

    #[gpui::test]
    fn a_local_repository_gets_the_picked_gitignore_and_licence(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a\n", "first");
        let h = Harness::open(cx, &repo);
        with_hub(&h, cx, Arc::new(FakeHub::default()));
        let dest = tempfile::tempdir().expect("tempdir");
        let target = dest.path().join("tidy");
        let plan = crate::new_repo::CreatePlan {
            account: None,
            lookup: Some(gh_account("alice")),
            new: osier_auth::hub::NewRepo {
                name: "tidy".into(),
                readme: false,
                gitignore: Some("Rust".into()),
                license: Some("mit".into()),
                ..Default::default()
            },
            target: target.clone(),
        };
        h.act(cx, move |o, w, cx| o.start_create(plan, w, cx));
        assert_eq!(
            std::fs::read_to_string(target.join(".gitignore")).expect("gitignore"),
            "# Rust\n/target\n"
        );
        let licence = std::fs::read_to_string(target.join("LICENSE")).expect("licence");
        assert!(licence.starts_with("MIT 20"), "{licence}");
        assert!(!target.join("README.md").exists());
        // Left uncommitted, for review in Changes.
        h.read(cx, |o, _| {
            let p = o.active_project().expect("opened");
            assert!(p.path.ends_with("tidy"));
            assert!(
                p.snapshot
                    .as_ref()
                    .is_some_and(|s| s.status.entries.len() == 2)
            );
        });
    }

    #[gpui::test]
    fn opening_a_project_with_changes_shows_them(cx: &mut TestAppContext) {
        use super::Tab;
        let (h, a, _b, first, _second) = two_projects(cx);
        h.click(cx, "tab-branches");
        // Opened fresh with changes: Changes, whatever tab was showing.
        let dirty = TempRepo::new();
        dirty.commit_file("d.txt", "d\n", "first");
        dirty.write("d.txt", "changed\n");
        let dirty_path = dirty.path().to_path_buf();
        h.act(cx, move |o, w, cx| {
            o.open_project_notify(&dirty_path, w, cx)
        });
        assert_eq!(tab(&h, cx), Tab::Changes);
        // Opened fresh and clean: the tab stays (Branches is never moved).
        h.click(cx, "tab-branches");
        let clean = TempRepo::new();
        clean.commit_file("c.txt", "c\n", "first");
        let clean_path = clean.path().to_path_buf();
        h.act(cx, move |o, w, cx| {
            o.open_project_notify(&clean_path, w, cx)
        });
        assert_eq!(tab(&h, cx), Tab::Branches);
        // Switching back to an open project with changes keeps the old behaviour: Branches.
        a.write("new.txt", "x\n");
        h.act(cx, |o, _, cx| o.refresh_project(0, cx));
        let a_path = a.path().to_path_buf();
        h.act(cx, move |o, w, cx| o.switch_to_project(&a_path, w, cx));
        assert_eq!(active_name(&h, cx), first);
        assert_eq!(tab(&h, cx), Tab::Branches);
        // From the recent list (closed, then reopened) it counts as opening again.
        h.click(cx, "tab-log");
        h.act(cx, |o, w, cx| o.close_project(0, w, cx));
        let a_path = a.path().to_path_buf();
        h.act(cx, move |o, w, cx| o.switch_to_project(&a_path, w, cx));
        assert_eq!(active_name(&h, cx), first);
        assert_eq!(tab(&h, cx), Tab::Changes);
    }

    #[gpui::test]
    fn background_tasks_show_only_when_asked(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a\n", "first");
        let h = Harness::open(cx, &repo);
        let guard = h.read(cx, |o, _| o.background.begin(crate::background::GH_LISTS));
        h.act(cx, |_, _, cx| cx.notify());
        assert!(h.bounds(cx, "background-tasks").is_none(), "off by default");
        h.act(cx, |o, _, cx| {
            o.settings.appearance.show_background_tasks = true;
            cx.notify();
        });
        assert!(h.bounds(cx, "background-tasks").is_some());
        h.read(cx, |o, _| {
            assert!(
                o.background_labels()
                    .contains(&crate::background::GH_LISTS.to_string())
            )
        });
        // Dropping the guard (the work finished or its dialog closed) counts it out.
        drop(guard);
        h.act(cx, |_, _, cx| cx.notify());
        assert!(h.bounds(cx, "background-tasks").is_none());
    }

    #[gpui::test]
    fn the_clone_submenu_waits_for_the_gh_accounts(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a\n", "first");
        let h = Harness::open(cx, &repo);
        h.act(cx, |o, w, cx| {
            o.gh_accounts.clear();
            o.gh_accounts_loaded = false;
            o.hub = Arc::new(FakeHub::default());
            o.open_palette_in(crate::palette::PaletteMode::CloneAccount, w, cx)
        });
        assert!(h.bounds(cx, "palette-status").is_some());
        assert!(h.bounds(cx, "palette-choice:alice").is_none());
        h.act(cx, |o, _, _| {
            o.gh_accounts = vec![gh_account("alice")];
            o.gh_accounts_loaded = true;
        });
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(400));
        cx.run_until_parked();
        assert!(h.bounds(cx, "palette-status").is_none());
        assert!(h.bounds(cx, "palette-choice:alice").is_some());
    }

    #[gpui::test]
    fn higher_contrast_makes_ref_badges_opaque(cx: &mut TestAppContext) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a\n", "first");
        let _h = Harness::open(cx, &repo);
        cx.update(|cx| {
            cx.set_global(crate::themes::HighContrast(false));
            let (bg, _) = crate::themes::badge_tint(cx);
            assert!(bg.a < 0.5, "faint as the theme intends");
            cx.set_global(crate::themes::HighContrast(true));
            let (bg, border) = crate::themes::badge_tint(cx);
            assert!(
                bg.a > 0.99 && border.a > 0.99,
                "opaque with higher contrast"
            );
        });
    }

    // ----- updates (`update_ui.rs`) -----

    /// GitHub releases and the installer as the update flow sees them, offline.
    #[derive(Default)]
    struct FakeUpdates {
        latest: std::sync::Mutex<Option<String>>,
        checks: std::sync::atomic::AtomicUsize,
        installs: std::sync::Mutex<Vec<(std::path::PathBuf, String)>>,
    }

    impl crate::update_ui::UpdateSource for FakeUpdates {
        fn latest(&self) -> Result<String, String> {
            self.checks
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.latest
                .lock()
                .expect("lock")
                .clone()
                .ok_or_else(|| "offline".to_string())
        }
        fn install(&self, bin_dir: &std::path::Path, tag: &str) -> Result<String, String> {
            self.installs
                .lock()
                .expect("lock")
                .push((bin_dir.to_path_buf(), tag.to_string()));
            Ok("Installed osier".into())
        }
        fn restart(&self, _: &std::path::Path) -> Result<(), String> {
            Err("not in tests".into())
        }
    }

    /// A harness on release v0.1.0, installed by the script, with `latest` published.
    fn updates_harness(
        cx: &mut TestAppContext,
        latest: Option<&str>,
        route: crate::install_route::InstallRoute,
    ) -> (Harness, TempRepo, Arc<FakeUpdates>) {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a\n", "first");
        let h = Harness::open(cx, &repo);
        let fake = Arc::new(FakeUpdates::default());
        *fake.latest.lock().expect("lock") = latest.map(String::from);
        let source = fake.clone();
        h.act(cx, move |o, _, _| {
            o.updates.source = source;
            o.updates.current = "0.1.0".into();
            o.updates.route = Some(route);
            o.updates.check = crate::update_ui::CheckState::NotChecked;
            o.state.update_check = Default::default();
        });
        (h, repo, fake)
    }

    fn script_route() -> crate::install_route::InstallRoute {
        crate::install_route::InstallRoute::Script {
            bin_dir: "/home/x/.local/bin".into(),
        }
    }

    #[gpui::test]
    fn ui_update_badge_opens_the_updates_section(cx: &mut TestAppContext) {
        let (h, _repo, fake) = updates_harness(cx, Some("v0.2.0"), script_route());
        assert!(
            h.bounds(cx, "update-badge").is_none(),
            "nothing checked yet"
        );
        h.act(cx, |o, _, cx| o.startup_update_check(cx));
        assert_eq!(fake.checks.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(h.bounds(cx, "update-badge").is_some());
        h.read(cx, |o, _| {
            assert_eq!(o.state.update_check.latest.as_deref(), Some("v0.2.0"));
            assert!(o.state.update_check.checked_at.is_some());
        });
        h.click(cx, "update-badge");
        assert!(h.has_settings(cx));
        assert_eq!(
            h.read(cx, |o, _| o.settings_highlight.map(|(t, _)| t)),
            Some(crate::settings_panel::SettingsTarget::UPDATES)
        );
        assert!(
            h.bounds(cx, "updates-section").is_some(),
            "About > Updates shown"
        );
        assert!(h.bounds(cx, "update-release-notes").is_some());
    }

    #[gpui::test]
    fn startup_reuses_a_recent_check_and_respects_the_setting(cx: &mut TestAppContext) {
        let (h, _repo, fake) = updates_harness(cx, Some("v0.3.0"), script_route());
        let calls = |fake: &FakeUpdates| fake.checks.load(std::sync::atomic::Ordering::SeqCst);
        // Checked an hour ago: no request, the cached result shows.
        h.act(cx, |o, _, cx| {
            o.state.update_check.checked_at = Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("time")
                    .as_secs() as i64
                    - 3600,
            );
            o.state.update_check.latest = Some("v0.2.0".into());
            o.startup_update_check(cx);
        });
        assert_eq!(calls(&fake), 0);
        assert_eq!(
            h.read(cx, |o, _| o.update_badge().map(String::from))
                .as_deref(),
            Some("v0.2.0")
        );
        // Two days old: asks again.
        h.act(cx, |o, _, cx| {
            o.state.update_check.checked_at = Some(0);
            o.startup_update_check(cx);
        });
        assert_eq!(calls(&fake), 1);
        assert_eq!(
            h.read(cx, |o, _| o.update_badge().map(String::from))
                .as_deref(),
            Some("v0.3.0")
        );
        // Turned off: never asks.
        h.act(cx, |o, _, cx| {
            o.settings.updates.check_on_startup = false;
            o.state.update_check.checked_at = Some(0);
            o.startup_update_check(cx);
        });
        assert_eq!(calls(&fake), 1);
        // Up to date or a development build: no badge.
        h.act(cx, |o, _, cx| {
            *fake.latest.lock().expect("lock") = Some("v0.1.0".into());
            o.check_for_updates(cx);
        });
        assert!(h.read(cx, |o, _| o.update_badge().is_none()));
        h.act(cx, |o, _, cx| {
            o.updates.current = "0.1.0+3.gabcdef0".into();
            *fake.latest.lock().expect("lock") = Some("v0.2.0".into());
            o.check_for_updates(cx);
        });
        assert!(h.read(cx, |o, _| o.update_badge().is_none()));
    }

    #[gpui::test]
    fn ui_update_installs_into_the_script_prefix_and_offers_a_restart(cx: &mut TestAppContext) {
        let (h, _repo, fake) = updates_harness(cx, Some("v0.2.0"), script_route());
        h.act(cx, |o, _, cx| o.check_for_updates(cx));
        h.act(cx, |o, w, cx| {
            o.open_settings_at(crate::settings_panel::SettingsTarget::UPDATES, w, cx)
        });
        assert!(h.bounds(cx, "update-restart").is_none());
        h.click(cx, "update-install");
        assert_eq!(
            fake.installs.lock().expect("lock").clone(),
            [("/home/x/.local/bin".into(), "v0.2.0".to_string())]
        );
        assert!(h.bounds(cx, "update-restart").is_some());
        assert!(
            h.bounds(cx, "update-badge").is_none(),
            "installed: no more badge"
        );
    }

    #[gpui::test]
    fn ui_package_installs_get_the_badge_but_no_update_button(cx: &mut TestAppContext) {
        let route = crate::install_route::InstallRoute::Package {
            manager: "pacman",
            name: "osier-bin".into(),
        };
        let (h, _repo, fake) = updates_harness(cx, Some("v0.2.0"), route);
        h.act(cx, |o, _, cx| o.check_for_updates(cx));
        assert!(
            h.bounds(cx, "update-badge").is_some(),
            "the news is shown anyway"
        );
        h.click(cx, "update-badge");
        h.click(cx, "update-install");
        assert!(
            fake.installs.lock().expect("lock").is_empty(),
            "the button is disabled"
        );
        h.read(cx, |o, _| {
            assert_eq!(o.updates.install, crate::update_ui::InstallState::Idle)
        });
    }
}
