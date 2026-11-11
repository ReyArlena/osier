//! The selection in the centre lists and the diff it loads into the diff pane.

use gpui_kit::component::diff::{DiffFile, DiffMode};
use gpui_kit::*;
use osier_git::Repository;
use osier_git::diff::{DiffOptions, DiffTarget};

use super::{Osier, Tab};
use crate::project::{Area, Project, Selection};

impl Osier {
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
