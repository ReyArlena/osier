//! Root view: owns projects, settings, state and the layout.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::diff::{DiffFile, DiffMode, DiffState};
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
        TabChanges,
        TabLog,
        TabBranches,
        TabStashes,
        SelectNext,
        SelectPrev,
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
        KeyBinding::new("ctrl-1", TabChanges, None),
        KeyBinding::new("ctrl-2", TabLog, None),
        KeyBinding::new("ctrl-3", TabBranches, None),
        KeyBinding::new("ctrl-4", TabStashes, None),
        KeyBinding::new("ctrl-enter", CommitStaged, None),
        KeyBinding::new("ctrl-shift-f", Fetch, None),
        KeyBinding::new("ctrl-shift-l", Pull, None),
        KeyBinding::new("ctrl-shift-p", Push, None),
        KeyBinding::new("down", SelectNext, Some("OsierList")),
        KeyBinding::new("up", SelectPrev, Some("OsierList")),
        KeyBinding::new("j", SelectNext, Some("OsierList")),
        KeyBinding::new("k", SelectPrev, Some("OsierList")),
    ]);
}

pub(crate) fn apply_theme_by_name(name: &str, cx: &mut App) {
    crate::apply_theme(name, cx);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Changes,
    Log,
    Branches,
    Stashes,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Changes, Tab::Log, Tab::Branches, Tab::Stashes];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Changes => "Changes",
            Tab::Log => "Log",
            Tab::Branches => "Branches",
            Tab::Stashes => "Stashes",
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
    diff_force_large: bool,
    /// Focus for keyboard navigation in the center lists.
    pub(crate) list_focus: FocusHandle,
    pub(crate) commit_message: Entity<TextareaState>,
    pub(crate) amend: bool,
    pub(crate) signoff: bool,
    /// State changed by frequent events (bounds, pane drags); saved by the tick loop.
    state_dirty: bool,
    /// Running fetch/pull/push, if any.
    pub(crate) remote_job: Option<crate::remote_ops::RemoteJob>,
    /// gh accounts discovered at startup (no tokens).
    pub(crate) gh_accounts: Vec<osier_auth::GhAccount>,
    /// Projects whose account choice was "this time only" (not written to state on save).
    pub(crate) session_only_accounts: std::collections::BTreeSet<String>,
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
        let message_subscription =
            cx.subscribe_in(&commit_message, window, |_, _, ev: &InputEvent, _, cx| {
                if matches!(ev, InputEvent::Change) {
                    cx.notify();
                }
            });
        let mut this = Self {
            focus_handle: cx.focus_handle(),
            commit_message,
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
            // Dev aid for screenshots: OSIER_START_TAB=log|branches|stashes.
            tab: match std::env::var("OSIER_START_TAB").as_deref() {
                Ok("log") => Tab::Log,
                Ok("branches") => Tab::Branches,
                Ok("stashes") => Tab::Stashes,
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
            session_only_accounts: Default::default(),
            diff_task: Task::ready(()),
            _subscriptions: vec![message_subscription],
        };
        this.apply_appearance(cx);
        this._subscriptions
            .push(cx.observe_window_appearance(window, |this, _, cx| {
                if this.settings.appearance.mode == ThemeMode::System {
                    this.apply_appearance(cx);
                    cx.notify();
                }
            }));
        this._subscriptions
            .push(cx.observe_window_bounds(window, |this, window, _| {
                let b = window.bounds();
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
        this.update_title(window);
        this.start_watcher_loop(cx);
        this.load_gh_accounts(cx);
        this
    }

    // ----- appearance -----

    pub(crate) fn apply_appearance(&mut self, cx: &mut App) {
        let a = &self.settings.appearance;
        let dark = match a.mode {
            ThemeMode::Dark => true,
            ThemeMode::Light => false,
            ThemeMode::System => matches!(
                cx.window_appearance(),
                WindowAppearance::Dark | WindowAppearance::VibrantDark
            ),
        };
        let name = if dark { &a.dark_theme } else { &a.light_theme };
        if !crate::apply_theme(name, cx) {
            let fallback = if dark {
                "Catppuccin Mocha"
            } else {
                "Catppuccin Latte"
            };
            crate::apply_theme(fallback, cx);
        }
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

    /// Forget a recent entry. Scan-directory entries cannot be removed here.
    pub(crate) fn forget_recent(
        &mut self,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use gpui_kit::component::WindowExt as _;
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
        project.task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { load_snapshot(&repo, page) })
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
                    Selection::None => false,
                    Selection::File { .. } => status_changed,
                    Selection::Commit { .. } => false,
                    Selection::Compare { .. } => status_changed,
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

    fn start_watcher_loop(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(400))
                    .await;
                let alive = this.update(cx, |this, cx| {
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
                    if this.poll_remote_job() {
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
        if let Some(p) = self.active_project_mut() {
            p.selection = Selection::File { path, area };
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
        self.log_scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
        self.reload_selection(cx);
        cx.notify();
    }

    /// Load the diff for the active project's selection into the diff pane.
    pub(crate) fn reload_selection(&mut self, cx: &mut Context<Self>) {
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
            Tab::Branches | Tab::Stashes => {}
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
            .on_action(cx.listener(|this, _: &TabChanges, _, cx| this.set_tab(Tab::Changes, cx)))
            .on_action(cx.listener(|this, _: &TabLog, _, cx| this.set_tab(Tab::Log, cx)))
            .on_action(cx.listener(|this, _: &TabBranches, _, cx| this.set_tab(Tab::Branches, cx)))
            .on_action(cx.listener(|this, _: &TabStashes, _, cx| this.set_tab(Tab::Stashes, cx)))
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
                                                    .child(self.render_diff_pane(cx)),
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
}
