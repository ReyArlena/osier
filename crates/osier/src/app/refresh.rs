//! Refreshing a project: snapshots, the file watcher loop and what a new snapshot changes in
//! the UI.

use std::path::Path;
use std::time::Duration;

use gpui_kit::*;

use super::{Osier, Tab};
use crate::project::{Area, Project, Selection, load_snapshot};

impl Osier {
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
        let first_load = project.snapshot.is_none();
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
        // A notify on `Osier` redraws every pane, and the watcher refreshes often. Once a
        // project has a snapshot, the start of a refresh shows only as the status bar's
        // "Refreshing" (quiet work, "Show background tasks" on, active project); the result
        // notifies in `install_snapshot`.
        if first_load || (self.settings.appearance.show_background_tasks && self.active == Some(ix))
        {
            cx.notify();
        }
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

    pub(super) fn start_watcher_loop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
                    this.check_missing_projects(cx);
                    this.poll_scan_watch(cx);
                    this.refresh_lists_on_timer(cx);
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
}
