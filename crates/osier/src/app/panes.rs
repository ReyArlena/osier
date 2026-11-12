//! Layout: tabs, the right pane kept per tab, diff views, the sidebar and the command log
//! panel.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::diff::{DiffEvent, DiffState};
use gpui_kit::*;

use super::{BottomTab, Osier, Tab};
use crate::pane::{Pane, RenderFn};
use crate::project::{Project, Selection};

/// Width of the project sidebar until its edge is dragged, and how far a drag can take it.
pub(crate) const SIDEBAR_WIDTH: f32 = 240.;
const SIDEBAR_MIN: f32 = 160.;
const SIDEBAR_MAX: f32 = 600.;
/// Narrowest the center column (tabs) and the diff pane can be dragged.
pub(crate) const CENTER_MIN: f32 = 260.;
pub(crate) const DIFF_MIN: f32 = 240.;
/// Room the diff pane keeps when the window is too narrow for the dragged widths: the center
/// column gives way first (down to `CENTER_MIN`), then the sidebar, so the diff stays on
/// screen and readable. Widening the window brings the dragged widths back.
pub(crate) const DIFF_ROOM: f32 = 320.;
/// The same for the height of the bottom panel.
pub(crate) const BOTTOM_PANEL_HEIGHT: f32 = 220.;
const BOTTOM_PANEL_MIN: f32 = 100.;
const BOTTOM_PANEL_MAX: f32 = 900.;

/// What a drag of the sidebar's edge carries. gpui tells drags apart by the type of their
/// payload; it draws nothing under the pointer.
#[derive(Clone)]
pub(crate) struct ResizeSidebar;

impl Render for ResizeSidebar {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

/// What a drag of the bottom panel's top edge carries.
#[derive(Clone)]
pub(crate) struct ResizeBottomPanel;

impl Render for ResizeBottomPanel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

/// The parts of the main window that redraw on their own (`crate::pane`). `Osier::render`
/// is the shell that lays them out.
pub(crate) struct Panes {
    pub(crate) title_bar: Entity<Pane<Osier>>,
    pub(crate) sidebar: Entity<Pane<Osier>>,
    pub(crate) tab_strip: Entity<Pane<Osier>>,
    pub(crate) tab: Entity<Pane<Osier>>,
    pub(crate) commit_details: Entity<Pane<Osier>>,
    pub(crate) diff: Entity<Pane<Osier>>,
    pub(crate) bottom_panel: Entity<Pane<Osier>>,
    pub(crate) status_bar: Entity<Pane<Osier>>,
}

impl Panes {
    pub(crate) fn new(owner: &Entity<Osier>, cx: &mut App) -> Self {
        let mut pane = |render: RenderFn<Osier>| Pane::new(owner, render, cx);
        Self {
            title_bar: pane(|o, _, cx| o.render_title_bar(cx).into_any_element()),
            sidebar: pane(|o, window, cx| o.render_sidebar(window, cx)),
            tab_strip: pane(|o, _, cx| o.render_tab_strip(cx)),
            tab: pane(|o, window, cx| o.render_tab(window, cx)),
            commit_details: pane(|o, _, cx| {
                let border = cx.theme().border;
                div()
                    .size_full()
                    .border_t_1()
                    .border_color(border)
                    .child(o.render_commit_details(cx))
                    .into_any_element()
            }),
            diff: pane(|o, window, cx| o.render_diff_pane(window, cx).into_any_element()),
            bottom_panel: pane(|o, _, cx| o.render_bottom_panel(cx)),
            status_bar: pane(|o, _, cx| {
                let has_project = o.active.is_some();
                o.render_status_bar(has_project, cx).into_any_element()
            }),
        }
    }
}

impl Osier {
    /// Width of the project sidebar: the dragged one, kept to a usable size, to half the
    /// window at most, and to what leaves the center column and the diff pane their
    /// narrowest.
    pub(crate) fn sidebar_width(&self, window: &Window) -> f32 {
        let window_w = f32::from(window.viewport_size().width);
        let most = (window_w / 2.)
            .min(window_w - CENTER_MIN - DIFF_ROOM)
            .clamp(SIDEBAR_MIN, SIDEBAR_MAX);
        self.state
            .layout
            .sidebar_width
            .unwrap_or(SIDEBAR_WIDTH)
            .clamp(SIDEBAR_MIN, most)
    }

    /// The sidebar's edge was dragged to `width`.
    pub(crate) fn resize_sidebar(
        &mut self,
        width: Pixels,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        // Stored as dragged, then as `sidebar_width` limits it.
        self.state.layout.sidebar_width = Some(f32::from(width));
        self.state.layout.sidebar_width = Some(self.sidebar_width(window));
        self.state_dirty = true;
        cx.notify();
    }

    /// Height of the bottom panel (git commands, notifications): the dragged one, kept to a
    /// usable size and to 60% of the window at most.
    pub(crate) fn bottom_panel_height(&self, window: &Window) -> f32 {
        let most = (f32::from(window.viewport_size().height) * 0.6)
            .clamp(BOTTOM_PANEL_MIN, BOTTOM_PANEL_MAX);
        self.state
            .layout
            .command_log_height
            .unwrap_or(BOTTOM_PANEL_HEIGHT)
            .clamp(BOTTOM_PANEL_MIN, most)
    }

    /// The bottom panel's top edge was dragged so that the panel is `height` tall.
    pub(crate) fn resize_bottom_panel(
        &mut self,
        height: Pixels,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        // Stored as dragged, then as `bottom_panel_height` limits it.
        self.state.layout.command_log_height = Some(f32::from(height));
        self.state.layout.command_log_height = Some(self.bottom_panel_height(window));
        self.state_dirty = true;
        cx.notify();
    }

    /// Widest the center column can be in this window: what is left beside the sidebar once
    /// the diff pane has `DIFF_ROOM`. The dragged width is kept as it is and only limited
    /// here, so it comes back when the window grows again.
    pub(crate) fn center_width_most(&self, window: &Window) -> f32 {
        let sidebar = if self.sidebar_visible {
            self.sidebar_width(window)
        } else {
            0.
        };
        (f32::from(window.viewport_size().width) - sidebar - DIFF_ROOM).max(CENTER_MIN)
    }

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
        let leaving_diff = (switching && !self.diff_loading && !self.diff_refreshing).then(|| {
            let (fresh, fresh_sub) = self.new_diff_view(cx);
            crate::project::DiffCache {
                state: std::mem::replace(&mut self.diff, fresh),
                subscription: self
                    .diff_subscription
                    .replace(fresh_sub)
                    .unwrap_or_else(|| Self::subscribe_diff(&self.diff, cx)),
                model: std::mem::take(&mut self.diff_model),
                staging: self.diff_staging.take(),
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
            self.diff_staging = cache.staging;
            self.diff_files = cache.files;
            self.diff_error = cache.error;
            self.diff_too_large = cache.too_large;
            self.viewer = cache.viewer;
            self.diff_loading = false;
            self.diff_refreshing = false;
            self.diff_shown = self.active_project().map(|p| p.selection.clone());
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
