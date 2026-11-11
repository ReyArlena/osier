//! Hunk and line level staging from the diff pane.

use gpui_kit::component::diff::DiffSide;
use gpui_kit::*;
use osier_git::diff::{DiffFile, Line};
use osier_git::ops::patch::{hunk_patch, lines_patch};
use osier_git::ops::stage::{self, ApplyTarget, apply_patch};
use osier_git::staging;

use crate::app::Osier;
use crate::project::{Area, Selection};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HunkAction {
    Stage,
    Unstage,
    Discard,
}

impl HunkAction {
    pub fn label(self) -> &'static str {
        match self {
            HunkAction::Stage => "Stage",
            HunkAction::Unstage => "Unstage",
            HunkAction::Discard => "Discard",
        }
    }

    /// Actions that make sense for a file in `area`. For a changed file, Discard rolls the
    /// lines back to HEAD in the index and the working tree.
    pub fn for_area(area: Area) -> &'static [HunkAction] {
        match area {
            Area::Changed => &[HunkAction::Stage, HunkAction::Unstage, HunkAction::Discard],
            Area::Untracked => &[HunkAction::Stage],
            Area::Conflicted => &[],
        }
    }
}

impl Osier {
    /// Area of the file shown in the diff pane, when it is a working tree file.
    pub(crate) fn diff_area(&self) -> Option<(String, Area)> {
        match &self.active_project()?.selection {
            Selection::File { path, area } => Some((path.clone(), *area)),
            _ => None,
        }
    }

    pub(crate) fn diff_file_model(&self, path: &str) -> Option<&DiffFile> {
        self.diff_model.iter().find(|f| f.path == path)
    }

    pub(crate) fn hunk_action(
        &mut self,
        path: String,
        hunk_ix: usize,
        action: HunkAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.diff_area().is_some_and(|(_, a)| a == Area::Changed) {
            self.staging_action(
                path,
                action,
                "hunk",
                move |hix, _| hix == hunk_ix,
                window,
                cx,
            );
            return;
        }
        let Some(file) = self.diff_file_model(&path) else {
            return;
        };
        let Some(patch) = hunk_patch(file, hunk_ix) else {
            return;
        };
        let Some((_, area)) = self.diff_area() else {
            return;
        };
        self.apply_hunk_patch(path, patch, area, action, "hunk", window, cx);
    }

    /// Apply `action` to the lines selected in the diff view, or to the ones its button was
    /// pressed with (`line_pick`): the selection is already gone by the time a click arrives.
    pub(crate) fn lines_action(
        &mut self,
        action: HunkAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let picked = self.line_pick.take();
        let Some(range) = self.diff.read(cx).selected_lines().or(picked) else {
            return;
        };
        let path = range.path().to_string();
        let (start, end) = (
            range.start().min(range.end()),
            range.start().max(range.end()),
        );
        let sides = [range.side(), range.end_side()];
        let on_new = sides.contains(&DiffSide::Modified);
        let on_old = sides.contains(&DiffSide::Original);
        let selected = move |l: &Line| {
            let n = l
                .new_no
                .map(|n| n as usize)
                .is_some_and(|n| (start..=end).contains(&n));
            let o = l
                .old_no
                .map(|n| n as usize)
                .is_some_and(|n| (start..=end).contains(&n));
            (on_new && n) || (on_old && o)
        };
        if self.diff_area().is_some_and(|(_, a)| a == Area::Changed) {
            self.staging_action(path, action, "lines", move |_, l| selected(l), window, cx);
            return;
        }
        let Some(file) = self.diff_file_model(&path) else {
            return;
        };
        // Unstage and discard apply the patch in reverse, which changes what the lines left
        // out of it have to look like.
        let reverse = action != HunkAction::Stage;
        // The hunk that holds the selection: the first with a selected changed line.
        let Some((hunk_ix, patch)) = file.hunks.iter().enumerate().find_map(|(ix, h)| {
            let hit = h
                .lines
                .iter()
                .any(|l| l.kind != osier_git::diff::LineKind::Context && selected(l));
            hit.then(|| lines_patch(file, ix, reverse, selected))
                .flatten()
                .map(|p| (ix, p))
        }) else {
            return;
        };
        let _ = hunk_ix;
        let Some((_, area)) = self.diff_area() else {
            return;
        };
        self.apply_hunk_patch(path, patch, area, action, "lines", window, cx);
    }

    /// Stage, unstage or roll back the lines of the shown changed file that `pick` chooses
    /// (hunk index, line), through its staging (`osier_git::staging`).
    #[allow(clippy::too_many_arguments)]
    fn staging_action(
        &mut self,
        path: String,
        action: HunkAction,
        what: &'static str,
        pick: impl Fn(usize, &Line) -> bool + Send + Clone + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(staging) = self.diff_staging.as_deref().cloned() else {
            return;
        };
        let context = self.settings.diff.context_lines;
        let orig = self.change_entry(&path).and_then(|e| e.orig_path);
        match action {
            HunkAction::Stage => self.run_op(
                format!("Staged {what} in {path}"),
                move |repo| staging::stage_lines(repo, &staging, pick).map(|_| ()),
                window,
                cx,
            ),
            HunkAction::Unstage => self.run_op(
                format!("Unstaged {what} in {path}"),
                move |repo| staging::unstage_lines(repo, &staging, pick).map(|_| ()),
                window,
                cx,
            ),
            HunkAction::Discard => self.confirm(
                if what == "hunk" {
                    "Roll back this change?".to_string()
                } else {
                    "Roll back the selected lines?".to_string()
                },
                if what == "hunk" {
                    format!(
                        "This change in {path} goes back to how it is in HEAD. Its staged and \
                         unstaged parts are lost."
                    )
                } else {
                    format!(
                        "The selected lines in {path} go back to how they are in HEAD. Their \
                         staged and unstaged changes are lost."
                    )
                },
                "Roll back",
                true,
                move |this, window, cx| {
                    let (staging, path, orig, pick) =
                        (staging.clone(), path.clone(), orig.clone(), pick.clone());
                    this.run_op(
                        format!("Rolled back {what} in {path}"),
                        move |repo| {
                            staging::revert_lines(
                                repo,
                                &staging,
                                &path,
                                orig.as_deref(),
                                context,
                                pick,
                            )
                            .map(|_| ())
                        },
                        window,
                        cx,
                    )
                },
                window,
                cx,
            ),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_hunk_patch(
        &mut self,
        path: String,
        patch: String,
        area: Area,
        action: HunkAction,
        what: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !HunkAction::for_area(area).contains(&action) {
            return;
        }
        let summary = format!("{} {what} in {path}", action.label());
        match action {
            HunkAction::Stage => {
                let untracked = area == Area::Untracked;
                let p = path.clone();
                self.run_op(
                    summary,
                    move |repo| {
                        if untracked {
                            stage::intent_to_add(repo, std::slice::from_ref(&p))?;
                        }
                        apply_patch(repo, &patch, ApplyTarget::Index, false)
                    },
                    window,
                    cx,
                );
            }
            HunkAction::Unstage => {
                self.run_op(
                    summary,
                    move |repo| apply_patch(repo, &patch, ApplyTarget::Index, true),
                    window,
                    cx,
                );
            }
            HunkAction::Discard => {
                self.confirm(
                    format!("Discard {what}?"),
                    format!("The selected changes in {path} are removed from the working tree."),
                    "Discard",
                    true,
                    move |this, window, cx| {
                        let patch = patch.clone();
                        this.run_op(
                            summary.clone(),
                            move |repo| apply_patch(repo, &patch, ApplyTarget::Worktree, true),
                            window,
                            cx,
                        );
                    },
                    window,
                    cx,
                );
            }
        }
    }
}
