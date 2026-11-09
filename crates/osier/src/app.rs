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
        ToggleDiffExpanded,
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
        UpdateProject,
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
    /// Shown only when Settings > Git > Git config tab is on.
    Config,
}

impl Tab {
    pub const ALL: [Tab; 7] = [
        Tab::Changes,
        Tab::Log,
        Tab::Branches,
        Tab::Stashes,
        Tab::Files,
        Tab::Patches,
        Tab::Config,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Changes => "Changes",
            Tab::Log => "Log",
            Tab::Branches => "Branches",
            Tab::Stashes => "Stashes",
            Tab::Files => "Files",
            Tab::Patches => "Patches",
            Tab::Config => "Git config",
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
    /// Three-way merge tool views for conflicted files (`merge_ops.rs`).
    pub(crate) merges: crate::merge_ops::Merges,
    /// The diff pane fills the window: sidebar and centre column hidden.
    pub(crate) diff_expanded: bool,
    /// The selected conflicted file has no conflict markers left (inline view).
    pub(crate) diff_no_markers: bool,
    /// Merged branches with a deleted upstream already offered for deletion this session
    /// (project, branch), so a dismissed offer does not come back after every fetch.
    pub(crate) gone_offered: std::collections::HashSet<(PathBuf, String)>,
    /// File of a multi-file diff the toolbar's file arrows last moved to.
    pub(crate) diff_file_ix: usize,
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
    /// The Files tab's rows, rebuilt only when the listing, the ignored list or the filter
    /// changes (with thousands of ignored files, rebuilding on every frame is slow).
    pub(crate) files_listing: Option<crate::views::files_tab::FilesListing>,
    /// Repository the tree was last built for, so its cursor is only carried over within
    /// one project.
    pub(crate) files_tree_root: Option<PathBuf>,
    /// Which folders were open (true) or closed in the Files tree, by id, from the last
    /// unfiltered tree: kept when the tree is rebuilt, and brought back after filtering.
    pub(crate) files_tree_folders: std::collections::HashMap<String, bool>,
    /// Whether the tree was last built for a filter (every folder open).
    pub(crate) files_tree_filtering: bool,
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
    /// The Git config panel's repository and its config (dialog or tab).
    pub(crate) repo_config: Option<crate::repo_config::RepoConfigView>,
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
        let mode = kit_mode(settings.diff.mode);
        let diff = cx.new(|cx| {
            DiffState::new([], cx)
                .with_mode(mode)
                .with_context_lines(Some(settings.diff.context_lines as usize))
                .with_inline_unit(
                    settings
                        .diff
                        .word_highlights
                        .then_some(gpui_kit::component::diff::DiffInlineUnit::Word),
                )
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
            merges: crate::merge_ops::Merges::default(),
            diff_expanded: false,
            diff_no_markers: false,
            diff_file_ix: 0,
            gone_offered: std::collections::HashSet::new(),
            viewer: None,
            markdown_as_code: false,
            files_filter,
            files_tree: cx.new(|cx| gpui_kit::component::tree::TreeState::new(cx)),
            show_ignored: false,
            files_tree_key: 0,
            files_listing: None,
            files_tree_root: None,
            files_tree_folders: std::collections::HashMap::new(),
            files_tree_filtering: false,
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
            // Dev aid for screenshots: OSIER_START_TAB=log|branches|stashes|files|patches|config.
            tab: match std::env::var("OSIER_START_TAB").as_deref() {
                Ok("log") => Tab::Log,
                Ok("branches") => Tab::Branches,
                Ok("stashes") => Tab::Stashes,
                Ok("files") => Tab::Files,
                Ok("patches") => Tab::Patches,
                Ok("config") => Tab::Config,
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
            repo_config: None,
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
        // Arrow keys move the Files tree's cursor: the viewer follows it.
        let files_tree = this.files_tree.clone();
        this._subscriptions
            .push(cx.observe(&files_tree, |this, _, cx| this.follow_files_cursor(cx)));
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
            return Ok(self.reopen_project(ix, make_active));
        }
        // `path` may be a file or folder inside a repository: compare its root too, so an
        // open repository is not added a second time.
        let opened = match Project::discover(self.runner.clone(), path) {
            Ok(repo) => {
                if let Some(ix) = self.projects.iter().position(|p| p.path == repo.workdir()) {
                    return Ok(self.reopen_project(ix, make_active));
                }
                Ok(Project::from_prepared(
                    Project::prepare_repo(repo, self.watch_runner()),
                    self.settings.git.log_page_size,
                ))
            }
            Err(error) => Err(error),
        };
        match opened {
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

    /// Open-project path for a repository that is already open at `ix`.
    fn reopen_project(&mut self, ix: usize, make_active: bool) -> usize {
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
        ix
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
        let before = self.projects.len();
        match self.open_project(path, true, cx) {
            Ok(ix) => {
                if self.projects.len() > before {
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
        let with_ignored = self.show_ignored && self.tab == Tab::Files && self.active == Some(ix);
        let project = &mut self.projects[ix];
        project.task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    load_snapshot(&repo, page, log_path.as_deref(), with_ignored)
                })
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
                    let mut due: Vec<usize> = Vec::new();
                    for (ix, p) in this.projects.iter_mut().enumerate() {
                        // Just switched on: the ignored list may predate changes nobody
                        // watched, so read it again.
                        if p.set_watch_all(watch_all && active == Some(ix)) {
                            due.push(ix);
                        }
                    }
                    let polled: Vec<usize> = this
                        .projects
                        .iter_mut()
                        .enumerate()
                        .filter_map(|(ix, p)| p.poll_watcher().then_some(ix))
                        .collect();
                    for ix in polled {
                        if !due.contains(&ix) {
                            due.push(ix);
                        }
                    }
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
            p.picked = crate::project::range_between(&s.commits, anchor, ix);
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
        let whitespace = self.settings.diff.whitespace;
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
        self.diff_no_markers = false;
        self.diff_file_ix = 0;
        let limit = if self.diff_force_large {
            u64::MAX
        } else {
            self.settings.diff.large_file_bytes
        };
        self.diff_task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    load_patch(&repo, &selection, context, limit, whitespace)
                })
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
                    Err(LoadError::NoMarkers) => {
                        this.diff_no_markers = true;
                        this.diff_model = std::rc::Rc::new(Vec::new());
                        this.set_diff_files(Vec::new(), cx);
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
            Tab::Files | Tab::Patches | Tab::Config => {}
        }
    }

    pub(crate) fn on_pane_resize(&mut self, sizes: &[Pixels]) {
        if let Some(first) = sizes.first() {
            self.state.layout.center_width = Some(f32::from(*first));
            self.state_dirty = true;
        }
    }

    pub(crate) fn toggle_soft_wrap(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut o = self.diff_options();
        o.soft_wrap = !o.soft_wrap;
        self.set_diff_options(o, window, cx);
    }

    /// Unified or split, for files without conflicts.
    pub(crate) fn set_diff_mode(&mut self, mode: DiffMode, cx: &mut Context<Self>) {
        self.diff.update(cx, |state, cx| state.set_mode(mode, cx));
        self.settings.diff.mode = match mode {
            DiffMode::Split => osier_config::settings::DiffMode::Split,
            _ => osier_config::settings::DiffMode::Unified,
        };
        self.save_settings();
        cx.notify();
    }

    /// How conflicted files show: the 3-way merge tool or inline conflict blocks.
    pub(crate) fn set_conflict_view(
        &mut self,
        view: osier_config::settings::ConflictView,
        cx: &mut Context<Self>,
    ) {
        self.settings.diff.conflicts = view;
        self.save_settings();
        cx.notify();
    }

    /// Expand the diff pane over the sidebar and the centre column, or back.
    pub(crate) fn toggle_diff_expanded(&mut self, cx: &mut Context<Self>) {
        self.diff_expanded = !self.diff_expanded;
        cx.notify();
    }

    // ----- layout -----

    pub(crate) fn set_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        let tab = if tab == Tab::Config && !self.config_tab_enabled() {
            Tab::Log
        } else {
            tab
        };
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

/// gpui-kit's layout for a diff mode setting.
pub(crate) fn kit_mode(mode: osier_config::settings::DiffMode) -> DiffMode {
    match mode {
        osier_config::settings::DiffMode::Split => DiffMode::Split,
        osier_config::settings::DiffMode::Unified => DiffMode::Unified,
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
    /// A conflicted file whose working copy has no conflict markers left.
    NoMarkers,
}

impl From<osier_git::GitError> for LoadError {
    fn from(e: osier_git::GitError) -> Self {
        LoadError::Git(e.to_string())
    }
}

/// Whether a selection's diff can hide whitespace changes: read-only ones only, since hunks
/// of a diff that hides them cannot be staged or discarded.
pub(crate) fn whitespace_applies(selection: &Selection) -> bool {
    matches!(
        selection,
        Selection::Commit { .. } | Selection::Compare { .. }
    )
}

/// Blocking: raw patch for the selection, parsed by gpui-kit's diff parser.
fn load_patch(
    repo: &Repository,
    selection: &Selection,
    context: u32,
    size_limit: u64,
    whitespace: osier_config::settings::DiffWhitespace,
) -> Result<(Vec<DiffFile>, Vec<osier_git::diff::DiffFile>), LoadError> {
    let hide = |o: &mut DiffOptions| {
        use osier_config::settings::DiffWhitespace as W;
        o.ignore_whitespace = whitespace == W::All;
        o.ignore_space_change = whitespace == W::Trim;
    };
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
        if !osier_git::ops::integrate::has_conflict_markers(&text) {
            return Err(LoadError::NoMarkers);
        }
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
            hide(&mut o);
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
            hide(&mut o);
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
        use gpui_kit::component::WindowExt as _;
        // A dialog that closes can leave nothing focused (it gives focus back to a handle that
        // is gone), and app shortcuts need a focused element to dispatch from. Put focus back
        // on the app once no dialog or sheet is open.
        if window.focused(cx).is_none()
            && !window.has_active_dialog(cx)
            && !window.has_active_sheet(cx)
        {
            let root = self.focus_handle.clone();
            window.defer(cx, move |window, cx| {
                if window.focused(cx).is_none() {
                    root.focus(window, cx);
                }
            });
        }
        let has_project = self.active.is_some();
        let merge_fills = self.diff_expanded;
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
            .on_action(
                cx.listener(|this, _: &ToggleDiffExpanded, _, cx| this.toggle_diff_expanded(cx)),
            )
            // Escape leaves the expanded diff pane (from an input that has nothing to do
            // with it, or from anywhere else).
            .on_action(
                cx.listener(|this, _: &gpui_kit::component::input::Escape, _, cx| {
                    if this.diff_expanded {
                        this.toggle_diff_expanded(cx);
                    } else {
                        cx.propagate();
                    }
                }),
            )
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                use gpui_kit::component::WindowExt as _;
                let m = &e.keystroke.modifiers;
                // F7 / shift-F7: next / previous change (the merge tool takes them first).
                if e.keystroke.key == "f7" && !m.control && !m.alt && this.diff_files > 0 {
                    this.diff.update(cx, |s, cx| {
                        if m.shift {
                            s.previous_change(cx)
                        } else {
                            s.next_change(cx)
                        }
                    });
                    cx.stop_propagation();
                    return;
                }
                if e.keystroke.key == "escape"
                    && this.diff_expanded
                    && !window.has_active_dialog(cx)
                    && !window.has_active_sheet(cx)
                {
                    this.toggle_diff_expanded(cx);
                    cx.stop_propagation();
                }
            }))
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
            .on_action(
                cx.listener(|this, _: &UpdateProject, window, cx| this.update_project(window, cx)),
            )
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
                        .when(self.sidebar_visible && !merge_fills, |this| {
                            this.child(self.render_sidebar(window, cx))
                        })
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .size_full()
                                .when(merge_fills, |this| {
                                    this.child(
                                        div()
                                            .flex_1()
                                            .min_h_0()
                                            .child(self.render_diff_pane(window, cx)),
                                    )
                                })
                                .when(!merge_fills, |this| {
                                    this.child(
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
                                })
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
mod tests;
