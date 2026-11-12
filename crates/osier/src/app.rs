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
pub(crate) mod external;
pub(crate) mod github;
pub(crate) mod hunk_ops;
pub(crate) mod ignore_ops;
mod keymap;
pub(crate) mod merge_ops;
pub(crate) mod ops;
pub(crate) mod panes;
mod patch_ops;
mod prefix;
mod projects;
mod rebase_dialog;
pub(crate) mod recent_status;
mod refresh;
pub(crate) mod remote_ops;
pub(crate) mod row_actions;
mod selection;
mod sidebar;
mod squash_ui;
pub(crate) mod switch_project;
mod update_branch;
pub(crate) mod update_ui;

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
        ToggleNotifications,
        ToggleCommitDetails,
        ToggleDiffExpanded,
        ToggleTheme,
        OpenSettings,
        OpenCommandPalette,
        SwitchProject,
        SwitchBranch,
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
        UpdateBranch,
        Fetch,
        Pull,
        Push,
        UpdateBranchNow,
        UpdateBranchMerge,
        UpdateBranchRebase,
        ForgetUpdateMethod,
        FetchAll,
        FetchPrune,
        FetchOrigin,
        PullFfOnly,
        PullRebase,
        PullMerge,
        PushBranch,
        PushSetUpstream,
        ForcePush,
        PrefixKey,
    ]
);

pub fn init(cx: &mut App) {
    crate::keymap::init(cx);
    // Dialogs and sheets (the palette, settings, prompts) are drawn outside the root view, so
    // actions from inside them never reach its handlers. Quit must work from anywhere.
    cx.on_action(|_: &Quit, cx| cx.quit());
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

/// What the bottom panel shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BottomTab {
    Commands,
    Notifications,
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
    /// The bottom panel (git commands or notifications) is shown.
    pub(crate) command_log_visible: bool,
    pub(crate) bottom_tab: BottomTab,
    /// Id of the newest notice the user has seen in the panel; the title bar counts the rest.
    pub(crate) notices_seen: u64,
    pub(crate) expanded_command: Option<u64>,
    /// The command log lists only failed commands.
    pub(crate) command_log_failures_only: bool,
    pub(crate) diff: Entity<DiffState>,
    /// The diff's line selection as it was when a "Stage lines" style button was pressed.
    /// Any mouse press clears the selection in the diff itself, before the button's click
    /// arrives, so the button keeps its own copy from press to release.
    pub(crate) line_pick: Option<gpui_kit::component::diff::DiffLineRange>,
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
    /// For a changed file in the Changes tab: which of its lines are staged
    /// (`osier_git::staging`), behind the hunk checkboxes and the staged-line tint.
    pub(crate) diff_staging: Option<std::rc::Rc<osier_git::staging::FileStaging>>,
    pub(crate) diff_loading: bool,
    /// The selection whose diff is on screen. Reloading the same one (after a tick, a roll
    /// back or a file change) keeps it shown instead of the "Reading diff" placeholder, so
    /// the diff does not flicker.
    pub(crate) diff_shown: Option<crate::project::Selection>,
    /// A reload of `diff_shown` is running (the old diff stays up meanwhile).
    pub(crate) diff_refreshing: bool,
    pub(crate) diff_error: Option<String>,
    pub(crate) diff_revision: u64,
    pub(crate) log_scroll: UniformListScrollHandle,
    /// The Changes list (virtualised).
    pub(crate) changes_scroll: UniformListScrollHandle,
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
    /// Files tab badges and tracked set, built once per snapshot.
    pub(crate) files_badges: Option<crate::views::files_tab::FilesBadges>,
    /// Counts viewer loads, so a slow read for an old selection does not replace a newer one.
    pub(crate) viewer_load: u64,
    /// Counts patch list reloads, so only the newest one is installed.
    pub(crate) patches_load: u64,
    /// Listed projects whose folder was gone at the last check (`check_missing_projects`).
    pub(crate) missing_projects: std::collections::HashSet<std::path::PathBuf>,
    pub(crate) missing_checked_at: Option<std::time::Instant>,
    pub(crate) missing_check_running: bool,
    /// Settings, state and cache writes, off the UI thread and in order (`save_queue.rs`).
    pub(crate) saves: crate::save_queue::SaveQueue,
    /// Known editors found on PATH (name, command), once settings asked (`detect_editors`).
    pub(crate) detected_editors: Option<Vec<(String, String)>>,
    pub(crate) detecting_editors: bool,
    /// The snapshot lists `files_badges` is being built from in the background.
    #[allow(clippy::type_complexity)]
    pub(crate) files_badges_pending: Option<(
        std::sync::Arc<osier_git::status::RepoStatus>,
        std::sync::Arc<Vec<String>>,
    )>,
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
    /// GitHub lists kept ready for the clone and create flows (`cached_list.rs`).
    pub(crate) gh_lists: crate::new_repo::GhLists,
    /// When Osier started (timed background work waits a little after launch).
    pub(crate) started: std::time::Instant,
    /// Counted background work for the status bar (`background.rs`).
    pub(crate) background: crate::background::Background,
    /// `background` version last drawn.
    background_seen: u64,
    /// Scan results per set of scan directories, kept on disk (`scanned()` reads it).
    pub(crate) scan_cache: crate::cached_list::CachedLists<Vec<PathBuf>>,
    /// The current scan directories' key in `scan_cache`; `None` with no scan directories.
    pub(crate) scan_key: Option<String>,
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
    pub(crate) updates: crate::app::update_ui::Updates,
    /// Focus target inside the settings sheet so Escape and its close button reach it.
    pub(crate) settings_focus: FocusHandle,
    /// Read by the settings dialog and notified when something only settings shows has
    /// changed (ssh status, install route), so the dialog redraws while the cached main view
    /// keeps its frame (`redraw_settings`).
    pub(crate) settings_redraw: Entity<()>,
    /// The install route is being looked up in the background (`detect_install_route`).
    pub(crate) install_route_pending: bool,
    /// Waiting for the key after the prefix key (see `prefix.rs`).
    pub(crate) prefix_pending: Option<Subscription>,
    /// The desktop's light/dark preference, for System mode (`system_scheme.rs`).
    pub(crate) system_scheme: crate::system_scheme::SystemScheme,
    /// The desktop's reduced-motion preference, `None` until (or unless) it says
    /// (`system_motion.rs`).
    pub(crate) system_reduced_motion: Option<bool>,
    /// Selected row in the Branches tab (branch, tag or remote).
    pub(crate) ref_selected: Option<crate::app::row_actions::RefRow>,
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
    pub(crate) recent_status: crate::app::recent_status::RecentStatus,
    /// Interval slider on the settings page (`projects.recent_status_minutes`).
    pub(crate) recent_interval: Entity<gpui_kit::component::slider::SliderState>,
    pub(crate) tooltip_delay: Entity<gpui_kit::component::slider::SliderState>,
    /// Toast duration slider on the settings page (`appearance.toast_seconds`).
    pub(crate) toast_time: Entity<gpui_kit::component::slider::SliderState>,
    /// Animation length slider on the settings page (`appearance.animation_ms`).
    pub(crate) animation_length: Entity<gpui_kit::component::slider::SliderState>,
    pub(crate) prefix_timeout: Entity<gpui_kit::component::slider::SliderState>,
    /// Narrow window: title-bar buttons drop their labels.
    pub(crate) compact: bool,
    /// Command log opened by the dev env var: do not persist that as a preference.
    command_log_from_env: bool,
    diff_task: Task<()>,
    /// The parts of the window that redraw on their own (`crate::pane`).
    pub(crate) panes: panes::Panes,
    /// The Branches tab's list (`crate::lazy_list`).
    pub(crate) branches_list: crate::lazy_list::LazyList,
    /// The Log tab's reflog list.
    pub(crate) reflog_list: crate::lazy_list::LazyList,
    /// The bottom panel's git command list.
    pub(crate) command_log_list: crate::lazy_list::LazyList,
    /// The bottom panel's notification list.
    pub(crate) notices_list: crate::lazy_list::LazyList,
    /// The Stashes tab's list.
    pub(crate) stashes_list: crate::lazy_list::LazyList,
    /// The Patches tab's list.
    pub(crate) patches_list: crate::lazy_list::LazyList,
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
        let recent_status = crate::app::recent_status::RecentStatus::new(
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
        let toast_time = cx.new(|_| {
            use crate::notices::{TOAST_SLIDER_MAX, ToastTime};
            gpui_kit::component::slider::SliderState::new()
                .min(1.)
                .max(TOAST_SLIDER_MAX as f32)
                .step(1.)
                .default_value(ToastTime::slider_stop(settings.appearance.toast_seconds) as f32)
        });
        let animation_length = cx.new(|_| {
            use crate::system_motion::{ANIMATION_MS_MAX, ANIMATION_MS_MIN};
            gpui_kit::component::slider::SliderState::new()
                .min(ANIMATION_MS_MIN as f32)
                .max(ANIMATION_MS_MAX as f32)
                .step(10.)
                .default_value(
                    settings
                        .appearance
                        .animation_ms
                        .clamp(ANIMATION_MS_MIN, ANIMATION_MS_MAX) as f32,
                )
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
            files_badges: None,
            viewer_load: 0,
            patches_load: 0,
            missing_projects: Default::default(),
            missing_checked_at: None,
            missing_check_running: false,
            saves: crate::save_queue::SaveQueue::new(),
            detected_editors: None,
            detecting_editors: false,
            files_badges_pending: None,
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
            bottom_tab: BottomTab::Commands,
            notices_seen: 0,
            expanded_command: None,
            command_log_failures_only: false,
            line_pick: None,
            diff,
            diff_subscription: None,
            diff_key: None,
            commit_branches: Default::default(),
            commit_branches_loading: None,
            diff_files: 0,
            diff_model: std::rc::Rc::new(Vec::new()),
            diff_staging: None,
            diff_loading: false,
            diff_shown: None,
            diff_refreshing: false,
            diff_error: None,
            diff_revision: 0,
            log_scroll: UniformListScrollHandle::new(),
            changes_scroll: UniformListScrollHandle::new(),
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
            gh_lists: Default::default(),
            started: std::time::Instant::now(),
            background: Default::default(),
            background_seen: 0,
            scan_cache: crate::cached_list::CachedLists::new(
                "scanned-projects",
                crate::background::SCAN_DIRS,
            ),
            scan_key: None,
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
            settings_redraw: cx.new(|_| ()),
            install_route_pending: false,
            prefix_pending: None,
            system_scheme: Default::default(),
            system_reduced_motion: None,
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
            toast_time,
            animation_length,
            prefix_timeout,
            compact: window.bounds().size.width < px(1500.),
            diff_task: Task::ready(()),
            panes: panes::Panes::new(&cx.entity(), cx),
            branches_list: Default::default(),
            reflog_list: Default::default(),
            command_log_list: Default::default(),
            notices_list: Default::default(),
            stashes_list: Default::default(),
            patches_list: Default::default(),
            _subscriptions: vec![
                message_subscription,
                filter_subscription,
                files_filter_subscription,
            ],
        };
        this.apply_appearance(cx);
        crate::notices::NoticeLog::attach(cx.weak_entity(), cx);
        let diff_entity = this.diff.clone();
        this.diff_subscription = Some(Self::subscribe_diff(&diff_entity, cx));
        // Arrow keys move the Files tree's cursor: the viewer follows it.
        let files_tree = this.files_tree.clone();
        this._subscriptions
            .push(cx.observe(&files_tree, |this, _, cx| this.follow_files_cursor(cx)));
        // Queued writes finish before Osier exits.
        this._subscriptions.push(cx.on_app_quit(|this, _| {
            this.saves.flush();
            async {}
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
        // The focused element can leave the tree: a filter box whose panel was hidden
        // (ctrl-b over the sidebar filter, a tab switch away from the Files filter) or a
        // handle a closed dialog gave focus back to. Shortcuts dispatch from the focused
        // element, so without this every key is dead until the user clicks.
        this._subscriptions
            .push(cx.on_focus_lost(window, |this, window, cx| {
                this.focus_handle.focus(window, cx);
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
        let slider = this.toast_time.clone();
        this._subscriptions.push(cx.subscribe(
            &slider,
            |this, _, ev: &gpui_kit::component::slider::SliderEvent, cx| {
                use gpui_kit::component::slider::SliderEvent;
                let (v, save) = match ev {
                    SliderEvent::Change(v) => (v, false),
                    SliderEvent::Release(v) => (v, true),
                };
                this.settings.appearance.toast_seconds =
                    crate::notices::ToastTime::from_slider_stop(v.start().round() as u32);
                this.apply_toast_time(cx);
                if save {
                    this.save_settings();
                }
                cx.notify();
            },
        ));
        let slider = this.animation_length.clone();
        this._subscriptions.push(cx.subscribe(
            &slider,
            |this, _, ev: &gpui_kit::component::slider::SliderEvent, cx| {
                use gpui_kit::component::slider::SliderEvent;
                let (v, save) = match ev {
                    SliderEvent::Change(v) => (v, false),
                    SliderEvent::Release(v) => (v, true),
                };
                this.settings.appearance.animation_ms = v.start().round() as u32;
                this.apply_motion(cx);
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
        #[cfg(all(target_os = "linux", not(test)))]
        this.watch_system_motion(cx);
        this.load_keymap(cx);
        // Without a focused element the app's key bindings have no dispatch path, so
        // shortcuts would only work after the first click into a list.
        let root_focus = this.focus_handle.clone();
        window.defer(cx, move |window, cx| root_focus.focus(window, cx));
        // Dev aid for screenshots: OSIER_OPEN_SETTINGS=1 opens the settings sheet at startup.
        // The sheet needs gpui-kit's root, which exists only after this view is installed.
        // OSIER_OPEN_PALETTE=root|switch_project|switch_branch|update_branch|fetch|pull|push|
        // clone|create opens the command palette on that mode.
        let palette = match std::env::var("OSIER_OPEN_PALETTE").as_deref() {
            Ok("root") => Some(crate::palette::PaletteMode::Root),
            Ok("switch_project") => Some(crate::palette::PaletteMode::SwitchProject),
            Ok("switch_branch") => Some(crate::palette::PaletteMode::SwitchBranch),
            Ok("update_branch") => Some(crate::palette::PaletteMode::UpdateBranch),
            Ok("fetch") => Some(crate::palette::PaletteMode::Fetch),
            Ok("pull") => Some(crate::palette::PaletteMode::Pull),
            Ok("push") => Some(crate::palette::PaletteMode::Push),
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

    /// Write `config.toml`, on the save queue (off the UI thread, in order).
    pub(crate) fn save_settings(&self) {
        let (settings, paths) = (self.settings.clone(), self.paths.clone());
        self.saves.push(move || {
            if let Err(error) = settings.save(&paths) {
                tracing::warn!(%error, "could not save settings");
            }
        });
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
        // On the save queue: dragging the window saves every 400ms.
        let paths = self.paths.clone();
        self.saves.push(move || {
            if let Err(error) = snapshot.save(&paths) {
                tracing::warn!(%error, "could not save state");
            }
        });
    }
}

impl Render for Osier {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use gpui_kit::component::WindowExt as _;
        // Before anything is built: rows only make their accessible names while this is on.
        crate::a11y::sync(window);
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
        let merge_fills = self.diff_expanded;
        let mut root = v_flex()
            .track_focus(&self.focus_handle)
            .key_context("Osier");
        // Every shortcut first closes the popup menu that has the keyboard, if any (see
        // `dismiss_popup_menus`). Registered before the handlers below so it runs first.
        for spec in crate::keymap::actions() {
            root = root.on_boxed_action(&*(spec.make)(), |_, window, cx| {
                Osier::dismiss_popup_menus(window, cx);
                cx.propagate();
            });
        }
        root.on_action(|_: &Quit, _, cx| cx.quit())
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
                cx.listener(|this, _: &ToggleNotifications, _, cx| this.toggle_notifications(cx)),
            )
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
            .on_action(cx.listener(|this, _: &SwitchBranch, window, cx| {
                this.open_palette_in(crate::palette::PaletteMode::SwitchBranch, window, cx)
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
            .on_action(cx.listener(|this, _: &UpdateBranch, window, cx| {
                this.open_palette_in(crate::palette::PaletteMode::UpdateBranch, window, cx)
            }))
            .on_action(cx.listener(|this, _: &Fetch, window, cx| {
                this.open_palette_in(crate::palette::PaletteMode::Fetch, window, cx)
            }))
            .on_action(cx.listener(|this, _: &Pull, window, cx| {
                this.open_palette_in(crate::palette::PaletteMode::Pull, window, cx)
            }))
            .on_action(cx.listener(|this, _: &Push, window, cx| {
                this.open_palette_in(crate::palette::PaletteMode::Push, window, cx)
            }))
            .on_action(cx.listener(|this, _: &UpdateBranchNow, w, cx| {
                this.run_menu_action("update_branch_now", w, cx)
            }))
            .on_action(cx.listener(|this, _: &UpdateBranchMerge, w, cx| {
                this.run_menu_action("update_branch_merge", w, cx)
            }))
            .on_action(cx.listener(|this, _: &UpdateBranchRebase, w, cx| {
                this.run_menu_action("update_branch_rebase", w, cx)
            }))
            .on_action(cx.listener(|this, _: &ForgetUpdateMethod, w, cx| {
                this.run_menu_action("forget_update_method", w, cx)
            }))
            .on_action(
                cx.listener(|this, _: &FetchAll, w, cx| this.run_menu_action("fetch_all", w, cx)),
            )
            .on_action(
                cx.listener(|this, _: &FetchPrune, w, cx| {
                    this.run_menu_action("fetch_prune", w, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &FetchOrigin, w, cx| {
                this.run_menu_action("fetch_origin", w, cx)
            }))
            .on_action(cx.listener(|this, _: &PullFfOnly, w, cx| {
                this.run_menu_action("pull_ff_only", w, cx)
            }))
            .on_action(
                cx.listener(|this, _: &PullRebase, w, cx| {
                    this.run_menu_action("pull_rebase", w, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &PullMerge, w, cx| this.run_menu_action("pull_merge", w, cx)),
            )
            .on_action(
                cx.listener(|this, _: &PushBranch, w, cx| {
                    this.run_menu_action("push_branch", w, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &PushSetUpstream, w, cx| {
                this.run_menu_action("push_set_upstream", w, cx)
            }))
            .on_action(
                cx.listener(|this, _: &ForcePush, w, cx| this.run_menu_action("force_push", w, cx)),
            )
            .on_action(cx.listener(|this, _: &ToggleStageSelected, window, cx| {
                this.toggle_stage_selected(window, cx)
            }))
            .on_action(cx.listener(|this, _: &DiscardSelected, window, cx| {
                this.discard_selected(window, cx)
            }))
            .on_action(cx.listener(|this, _: &PrefixKey, window, cx| this.start_prefix(window, cx)))
            .on_action(
                cx.listener(|this, a: &crate::app::row_actions::RowAction, window, cx| {
                    this.run_row_action(a.kind, window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, a: &crate::palette::PaletteAction, window, cx| {
                    this.run_palette_command(a.id, window, cx)
                }),
            )
            // Any click cancels a waiting prefix key.
            .capture_any_mouse_down(cx.listener(|this, _, _, cx| this.cancel_prefix(cx)))
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.move_selection(1, cx)))
            .on_action(cx.listener(|this, _: &SelectPrev, _, cx| this.move_selection(-1, cx)))
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(crate::pane::live(&self.panes.title_bar))
            .child(
                div().flex_1().min_h_0().child(
                    h_flex()
                        .size_full()
                        .when(self.sidebar_visible && !merge_fills, |this| {
                            this.child(self.render_sidebar_frame(window, cx))
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
                                            .child(crate::pane::cached(&self.panes.diff, window)),
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
                                                    .child(crate::pane::cached(
                                                        &self.panes.diff,
                                                        window,
                                                    )),
                                            ),
                                    ),
                                )
                                })
                                .when(self.command_log_visible, |this| {
                                    this.child(self.render_bottom_panel_frame(window, cx))
                                }),
                        ),
                ),
            )
            .child(crate::pane::live(&self.panes.status_bar))
    }
}

#[cfg(test)]
mod tests;
