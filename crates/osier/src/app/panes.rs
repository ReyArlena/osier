//! Layout: tabs, the right pane kept per tab, diff views, the sidebar and the command log
//! panel.

use gpui_kit::component::diff::{DiffEvent, DiffState};
use gpui_kit::*;

use super::{BottomTab, Osier, Tab};
use crate::project::{Project, Selection};

impl Osier {
    pub(crate) fn on_pane_resize(&mut self, sizes: &[Pixels]) {
        if let Some(first) = sizes.first() {
            self.state.layout.center_width = Some(f32::from(*first));
            self.state_dirty = true;
        }
    }

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
    pub(super) fn park_pane(&mut self) {
        let (editor, blame) = (self.editor.take(), self.blame.take());
        if let Some(p) = self.active_project_mut() {
            p.parked_editor = editor;
            p.parked_blame = blame;
            for pane in p.panes.values_mut() {
                pane.diff = None;
            }
        }
    }

    pub(super) fn subscribe_diff(diff: &Entity<DiffState>, cx: &mut Context<Self>) -> Subscription {
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

    pub(crate) fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_visible = !self.sidebar_visible;
        self.save_state();
        cx.notify();
    }

    /// Show the bottom panel on `tab`, or hide it when it already shows that tab.
    fn toggle_bottom_panel(&mut self, tab: BottomTab, cx: &mut Context<Self>) {
        if self.command_log_visible && self.bottom_tab == tab {
            self.command_log_visible = false;
        } else {
            self.show_bottom_tab(tab, cx);
        }
        self.save_state();
        cx.notify();
    }

    /// Open the bottom panel on `tab`. Showing the notifications marks them all as seen.
    pub(crate) fn show_bottom_tab(&mut self, tab: BottomTab, cx: &mut Context<Self>) {
        self.command_log_visible = true;
        self.bottom_tab = tab;
        if tab == BottomTab::Notifications {
            self.notices_seen = crate::notices::NoticeLog::latest(cx);
        }
        cx.notify();
    }

    pub(crate) fn toggle_command_log(&mut self, cx: &mut Context<Self>) {
        self.toggle_bottom_panel(BottomTab::Commands, cx);
    }

    pub(crate) fn toggle_notifications(&mut self, cx: &mut Context<Self>) {
        self.toggle_bottom_panel(BottomTab::Notifications, cx);
    }

    /// Notices raised after the panel was last open on them.
    pub(crate) fn unread_notices(&self, cx: &App) -> usize {
        if self.command_log_visible && self.bottom_tab == BottomTab::Notifications {
            return 0;
        }
        crate::notices::NoticeLog::unread(self.notices_seen, cx)
    }

    /// From an error notification: open the command log on failures only, with the command
    /// that failed (when known) expanded. Failures are few, so it is at or near the top.
    pub(crate) fn show_failed_command(&mut self, id: Option<u64>, cx: &mut Context<Self>) {
        if !self.command_log_visible {
            self.save_state();
        }
        self.show_bottom_tab(BottomTab::Commands, cx);
        self.command_log_failures_only = true;
        self.expanded_command = id;
        cx.notify();
    }
}
