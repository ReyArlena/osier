//! The project sidebar: scanning, the filter box and its suggestions, the pinned and recent
//! lists, reordering.

use std::path::{Path, PathBuf};

use gpui_kit::*;
use osier_config::settings::RecentStatusMode;

use super::{Osier, Tab};
use crate::project::Project;
use crate::sidebar_query::{self, Query, RowStatus};

impl Osier {
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
}
