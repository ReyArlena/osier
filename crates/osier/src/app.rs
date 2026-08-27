//! Root view: owns projects, settings, state and the layout.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::diff::{DiffFile, DiffMode, DiffState};
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
    pub(crate) sidebar_visible: bool,
    pub(crate) command_log_visible: bool,
    pub(crate) expanded_command: Option<u64>,
    pub(crate) diff: Entity<DiffState>,
    /// Files in the diff pane, for the header counts.
    pub(crate) diff_files: usize,
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
    /// State changed by frequent events (bounds, pane drags); saved by the tick loop.
    state_dirty: bool,
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
        let mut this = Self {
            focus_handle: cx.focus_handle(),
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
            diff_task: Task::ready(()),
            _subscriptions: Vec::new(),
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

        // Restore open projects, then the CLI argument on top.
        let restore: Vec<PathBuf> = this.state.open_projects.clone();
        for p in restore {
            this.open_project(&p, false, cx);
        }
        if let Some(active) = this.state.active_project.clone()
            && let Some(ix) = this.projects.iter().position(|p| p.path == active)
        {
            this.activate(ix, window, cx);
        }
        if let Some(p) = open_arg {
            let p = std::fs::canonicalize(&p).unwrap_or(p);
            this.open_project(&p, true, cx);
            if let Some(ix) = this.projects.iter().position(|p2| p2.path == p) {
                this.activate(ix, window, cx);
            }
        }
        if this.active.is_none() && !this.projects.is_empty() {
            this.activate(0, window, cx);
        }
        this.update_title(window);
        this.start_watcher_loop(cx);
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
        self.state.layout.sidebar_visible = self.sidebar_visible;
        if !self.command_log_from_env {
            self.state.layout.command_log_visible = self.command_log_visible;
        }
        if let Err(error) = self.state.save(&self.paths) {
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

    /// Open (or re-activate) a repository at `path`. Returns its index.
    pub(crate) fn open_project(
        &mut self,
        path: &Path,
        make_active: bool,
        cx: &mut Context<Self>,
    ) -> Option<usize> {
        if let Some(ix) = self.projects.iter().position(|p| p.path == path) {
            if make_active {
                self.active = Some(ix);
                self.state.open(&self.projects[ix].path);
                self.save_state();
            }
            return Some(ix);
        }
        match Project::open(self.runner.clone(), path, self.settings.git.log_page_size) {
            Ok(project) => {
                let workdir = project.path.clone();
                self.projects.push(project);
                let ix = self.projects.len() - 1;
                self.state.open(&workdir);
                if !make_active && self.state.active_project.as_deref() != Some(&workdir) {
                    // `open` made it active; keep whatever was active before.
                }
                self.save_state();
                self.refresh_project(ix, cx);
                if make_active {
                    self.active = Some(ix);
                }
                Some(ix)
            }
            Err(error) => {
                tracing::warn!(%error, path = %path.display(), "cannot open repository");
                self.global_error = Some(format!("{}: {error}", path.display()));
                self.state.forget_recent(path);
                self.state.close(path);
                self.save_state();
                cx.notify();
                None
            }
        }
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

    pub(crate) fn prompt_open_repository(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open repository".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                if let Some(ix) = this.open_project(&path, true, cx) {
                    this.active = Some(ix);
                    this.reload_selection(cx);
                    cx.notify();
                }
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
            self.set_diff_files(Vec::new(), cx);
            return;
        };
        let repo = project.repo.clone();
        let selection = project.selection.clone();
        if selection == Selection::None {
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
                    Ok(files) => this.set_diff_files(files, cx),
                    Err(LoadError::TooLarge(size)) => {
                        this.diff_too_large = Some(size);
                        this.set_diff_files(Vec::new(), cx);
                    }
                    Err(LoadError::Git(error)) => {
                        this.diff_error = Some(error);
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
) -> Result<Vec<DiffFile>, LoadError> {
    if let Selection::File { path, .. } = selection
        && let Ok(meta) = std::fs::metadata(repo.workdir().join(path))
        && meta.len() > size_limit
    {
        return Err(LoadError::TooLarge(meta.len()));
    }
    let patch = match selection {
        Selection::None => return Ok(Vec::new()),
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
    };
    if patch.trim().is_empty() {
        return Ok(Vec::new());
    }
    DiffFile::parse(&patch).map_err(|e| LoadError::Git(e.to_string()))
}

impl Render for Osier {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let has_project = self.active.is_some();
        v_flex()
            .track_focus(&self.focus_handle)
            .key_context("Osier")
            .on_action(|_: &Quit, _, cx| cx.quit())
            .on_action(cx.listener(|this, _: &Refresh, _, cx| this.refresh_active(cx)))
            .on_action(
                cx.listener(|this, _: &OpenRepository, _, cx| this.prompt_open_repository(cx)),
            )
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
