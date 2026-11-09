//! Root view: owns projects, settings, state and the layout.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::diff::DiffState;
use gpui_kit::component::input::{InputEvent, TextareaState};
use gpui_kit::component::resizable::{ResizableState, h_resizable, resizable_panel};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use osier_config::settings::ThemeMode;
use osier_config::{Paths, Settings, State};
use osier_git::GitRunner;

use crate::command_log::UiCommandSink;
use crate::project::Project;
use crate::test_ids::TestId as _;

pub(crate) mod actions;
mod appearance;
mod changes_ops;
mod external;
pub(crate) mod github;
pub(crate) mod hunk_ops;
pub(crate) mod ignore_ops;
mod keymap;
pub(crate) mod merge_ops;
pub(crate) mod ops;
mod panes;
mod patch_ops;
mod prefix;
mod projects;
mod rebase_dialog;
mod refresh;
pub(crate) mod remote_ops;
mod selection;
mod sidebar;
mod squash_ui;
mod update_project;

pub(crate) use selection::{first_change, kit_mode, whitespace_applies};

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
    pub(crate) merges: crate::app::merge_ops::Merges,
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
    pub(crate) remote_job: Option<crate::app::remote_ops::RemoteJob>,
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
            merges: crate::app::merge_ops::Merges::default(),
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
                this.start_remote(crate::app::remote_ops::RemoteOp::fetch_all(), window, cx)
            }))
            .on_action(cx.listener(|this, _: &Pull, window, cx| {
                this.start_remote(crate::app::remote_ops::RemoteOp::pull_default(), window, cx)
            }))
            .on_action(cx.listener(|this, _: &Push, window, cx| {
                this.start_remote(crate::app::remote_ops::RemoteOp::push_default(), window, cx)
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
