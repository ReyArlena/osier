//! Opening, restoring, activating and closing projects; the recent list and pins.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::*;
use osier_git::GitRunner;

use super::{Osier, reopen_error};
use crate::notices::{Notice, WindowNoticeExt as _};
use crate::project::Project;

impl Osier {
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
            Ok(project) => Ok(self.install_opened(project, make_active, cx)),
            Err(error) => Err(self.forget_unopenable(path, &error, cx)),
        }
    }

    /// Add a newly opened project (not open yet) and start its first refresh.
    fn install_opened(
        &mut self,
        project: Project,
        make_active: bool,
        cx: &mut Context<Self>,
    ) -> usize {
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
        ix
    }

    /// A path that is not (or no longer) a repository: drop it from recents and the open list.
    /// Returns the message to show.
    fn forget_unopenable(&mut self, path: &Path, error: &str, cx: &mut Context<Self>) -> String {
        tracing::warn!(%error, path = %path.display(), "cannot open repository");
        self.state.forget_recent(path);
        self.state.close(path);
        self.save_state();
        cx.notify();
        format!("{}: {error}", crate::format::tilde(path))
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
    pub(super) fn restore_in_background(
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
        self.open_project_then(path, |_, _, _, _| {}, window, cx);
    }

    /// [`Osier::open_project_notify`], then `then` with the project's index once it is open
    /// and active (not called when the path cannot be opened). The open finishes in the
    /// background, so follow-up work that needs the project goes here.
    pub(crate) fn open_project_then(
        &mut self,
        path: &Path,
        then: impl FnOnce(&mut Osier, usize, &mut Window, &mut Context<Osier>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(ix) = self.projects.iter().position(|p| p.path == path) {
            let ix = self.reopen_project(ix, true);
            self.activate(ix, window, cx);
            then(self, ix, window, cx);
            return;
        }
        // Finding the repository runs git and starting its watcher walks folders: both in the
        // background, then the project is added here.
        let (runner, watch_runner) = (self.runner.clone(), self.watch_runner());
        let path = path.to_path_buf();
        let target = path.clone();
        self.in_background(
            crate::background::OPENING_PROJECT,
            move || Project::prepare(runner, watch_runner, &target),
            move |this, prepared, window, cx| match prepared {
                Ok(prepared) => {
                    let project = Project::from_prepared(prepared, this.settings.git.log_page_size);
                    // `path` may have been a folder inside a repository that is open already,
                    // or the same repository opened twice quickly.
                    let ix = match this.projects.iter().position(|p| p.path == project.path) {
                        Some(ix) => this.reopen_project(ix, true),
                        None => {
                            let ix = this.install_opened(project, true, cx);
                            this.fresh_open = Some(this.projects[ix].path.clone());
                            ix
                        }
                    };
                    this.activate(ix, window, cx);
                    then(this, ix, window, cx);
                }
                Err(error) => {
                    let message = this.forget_unopenable(&path, &error, cx);
                    window.push_notice(
                        Notice::warning(format!("{message}. It was removed from the recent list."))
                            .title("Could not open repository"),
                        cx,
                    )
                }
            },
            window,
            cx,
        );
    }

    /// Forget a recent entry. Entries provided by a scan directory cannot be removed here.
    pub(crate) fn forget_recent(
        &mut self,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        window.push_notice(
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

    pub(super) fn update_title(&self, window: &mut Window) {
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
}
