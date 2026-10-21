//! Context-menu actions as key bindings. Each fires on the selected row of the current tab
//! (Changes, Files, Log, Patches), doing what the same entry in that row's right-click menu
//! does. One binding covers every row that offers the action; rows that do not offer it ignore
//! the key. Destructive actions still open their confirmation dialog.

use gpui_kit::*;
use osier_git::ops::ignore::IgnoreFile;
use osier_git::ops::patches::PatchSource;
use osier_git::ops::reset::ResetMode;

use crate::app::{Osier, Tab};
use crate::ignore_ops::{PatternKind, RowState};
use crate::keymap::{ActionSpec, Scope};
use crate::project::{Area, Selection};

/// Which context-menu entry a key runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    OpenInEditor,
    OpenInFileManager,
    CopyPath,
    Blame,
    FileHistory,
    ViewAtHead,
    AddToGitignore,
    AddExtensionToGitignore,
    IgnoreLocally,
    Untrack,
    CopyAsPatch,
    SaveToPatches,
    Shelve,
    ExportPatch,
    Checkout,
    NewBranchHere,
    TagHere,
    CompareWithWorkingTree,
    ResetSoft,
    ResetMixed,
    ResetHard,
    Revert,
    CherryPick,
    InteractiveRebase,
    UndoLastCommit,
    CopyHash,
    OpenOnGithub,
    Apply,
    Unshelve,
    Rename,
    Delete,
}

/// The key-bindable form of a context-menu entry.
#[derive(Debug, Clone, PartialEq, Action)]
#[action(namespace = osier, no_json)]
pub struct RowAction {
    pub kind: RowKind,
}

/// (id, settings name, where it applies, kind). Names read as "Context menus > name".
const ROWS: &[(&str, &str, &str, RowKind)] = &[
    (
        "row_open_in_editor",
        "Open in editor",
        "Changes and Files rows",
        RowKind::OpenInEditor,
    ),
    (
        "row_open_in_file_manager",
        "Open in file manager",
        "Changes and Files rows",
        RowKind::OpenInFileManager,
    ),
    (
        "row_copy_path",
        "Copy path",
        "Changes and Files rows",
        RowKind::CopyPath,
    ),
    (
        "row_blame",
        "Blame",
        "Changes and Files rows",
        RowKind::Blame,
    ),
    (
        "row_file_history",
        "File history",
        "Changes and Files rows",
        RowKind::FileHistory,
    ),
    (
        "row_view_at_head",
        "View at HEAD",
        "Files rows",
        RowKind::ViewAtHead,
    ),
    (
        "row_add_to_gitignore",
        "Add to .gitignore",
        "Changes and Files rows",
        RowKind::AddToGitignore,
    ),
    (
        "row_add_extension_to_gitignore",
        "Add all *.ext files to .gitignore",
        "Changes and Files rows",
        RowKind::AddExtensionToGitignore,
    ),
    (
        "row_ignore_locally",
        "Ignore only on this machine",
        "Changes and Files rows",
        RowKind::IgnoreLocally,
    ),
    (
        "row_untrack",
        "Untrack",
        "Tracked rows in Changes and Files",
        RowKind::Untrack,
    ),
    (
        "row_copy_as_patch",
        "Copy as patch",
        "Changes rows and Log commits",
        RowKind::CopyAsPatch,
    ),
    (
        "row_save_to_patches",
        "Save to Patches",
        "Changes rows and Log commits",
        RowKind::SaveToPatches,
    ),
    ("row_shelve", "Shelve", "Changes rows", RowKind::Shelve),
    (
        "row_export_patch",
        "Export patch file",
        "Log commits and Patches rows",
        RowKind::ExportPatch,
    ),
    (
        "row_checkout",
        "Check out",
        "Log commits (detached)",
        RowKind::Checkout,
    ),
    (
        "row_new_branch_here",
        "New branch here",
        "Log commits",
        RowKind::NewBranchHere,
    ),
    ("row_tag_here", "Tag here", "Log commits", RowKind::TagHere),
    (
        "row_compare_with_working_tree",
        "Compare with working tree",
        "Log commits",
        RowKind::CompareWithWorkingTree,
    ),
    (
        "row_reset_soft",
        "Reset current branch here (soft)",
        "Log commits",
        RowKind::ResetSoft,
    ),
    (
        "row_reset_mixed",
        "Reset current branch here (mixed)",
        "Log commits",
        RowKind::ResetMixed,
    ),
    (
        "row_reset_hard",
        "Reset current branch here (hard)",
        "Log commits",
        RowKind::ResetHard,
    ),
    (
        "row_revert",
        "Revert commit",
        "Log commits",
        RowKind::Revert,
    ),
    (
        "row_cherry_pick",
        "Cherry-pick onto current branch",
        "Log commits",
        RowKind::CherryPick,
    ),
    (
        "row_interactive_rebase",
        "Interactive rebase from here",
        "Log commits",
        RowKind::InteractiveRebase,
    ),
    (
        "row_undo_last_commit",
        "Undo last commit",
        "The HEAD commit in Log",
        RowKind::UndoLastCommit,
    ),
    (
        "row_copy_hash",
        "Copy hash",
        "Log commits",
        RowKind::CopyHash,
    ),
    (
        "row_open_on_github",
        "Open on GitHub",
        "Log commits",
        RowKind::OpenOnGithub,
    ),
    ("row_apply", "Apply", "Patches rows", RowKind::Apply),
    (
        "row_unshelve",
        "Unshelve",
        "Patches rows",
        RowKind::Unshelve,
    ),
    ("row_rename", "Rename", "Patches rows", RowKind::Rename),
    (
        "row_delete",
        "Delete / Drop",
        "Patches rows",
        RowKind::Delete,
    ),
];

/// Registry entries: category "Context menus", unbound by default.
pub(crate) fn specs() -> Vec<ActionSpec> {
    ROWS.iter()
        .map(|&(id, name, detail, kind)| ActionSpec {
            id,
            name,
            category: "Context menus",
            defaults: &[],
            scope: Scope::Row,
            detail,
            make: row_maker(kind),
        })
        .collect()
}

/// `ActionSpec::make` is a plain fn pointer, so each kind needs its own.
fn row_maker(kind: RowKind) -> fn() -> Box<dyn Action> {
    macro_rules! makers {
        ($($k:ident),*) => {
            match kind {
                $(RowKind::$k => || Box::new(RowAction { kind: RowKind::$k }),)*
            }
        };
    }
    makers!(
        OpenInEditor,
        OpenInFileManager,
        CopyPath,
        Blame,
        FileHistory,
        ViewAtHead,
        AddToGitignore,
        AddExtensionToGitignore,
        IgnoreLocally,
        Untrack,
        CopyAsPatch,
        SaveToPatches,
        Shelve,
        ExportPatch,
        Checkout,
        NewBranchHere,
        TagHere,
        CompareWithWorkingTree,
        ResetSoft,
        ResetMixed,
        ResetHard,
        Revert,
        CherryPick,
        InteractiveRebase,
        UndoLastCommit,
        CopyHash,
        OpenOnGithub,
        Apply,
        Unshelve,
        Rename,
        Delete
    )
}

/// What is selected in the current tab, as far as row actions care.
enum Row {
    /// A Changes or Files row. `tracked` is false for untracked rows.
    File {
        path: String,
        area: Option<Area>,
        tracked: bool,
        is_dir: bool,
    },
    Commit {
        hash: String,
        is_head: bool,
    },
    Patch {
        id: String,
        name: String,
    },
}

impl Osier {
    fn selected_row(&self) -> Option<Row> {
        match self.tab {
            Tab::Changes | Tab::Files => {
                let project = self.active_project()?;
                let (path, area) = match &project.selection {
                    Selection::File { path, area } => (path.clone(), Some(*area)),
                    Selection::WorkFile { path, .. } => (path.clone(), None),
                    _ => return None,
                };
                let untracked = project.snapshot.as_ref().is_some_and(|s| {
                    s.status
                        .entries
                        .iter()
                        .any(|e| e.is_untracked() && (e.path == path || path.starts_with(&e.path)))
                });
                let tracked = match area {
                    Some(a) => a != Area::Untracked,
                    None => !untracked,
                };
                Some(Row::File {
                    is_dir: path.ends_with('/'),
                    path: path.trim_end_matches('/').to_string(),
                    area,
                    tracked,
                })
            }
            Tab::Log => {
                let project = self.active_project()?;
                let commit = project.commit_at(project.selected_commit?)?;
                Some(Row::Commit {
                    hash: commit.hash.clone(),
                    is_head: commit.refs.iter().any(|r| r.starts_with("HEAD")),
                })
            }
            Tab::Patches => {
                let id = self.patch_selected.clone()?;
                let name = self
                    .patches
                    .iter()
                    .find(|p| p.id == id)
                    .map(|p| p.meta.name.clone())?;
                Some(Row::Patch { id, name })
            }
            Tab::Branches | Tab::Stashes => None,
        }
    }

    /// Run `kind` on the selected row, when the row offers it.
    pub(crate) fn run_row_action(
        &mut self,
        kind: RowKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use RowKind as K;
        let Some(row) = self.selected_row() else {
            return;
        };
        match row {
            Row::File {
                path,
                area,
                tracked,
                is_dir,
            } => {
                let state = RowState { is_dir, tracked };
                match kind {
                    K::OpenInEditor if !is_dir => {
                        self.open_in_editor(Some((path, None)), window, cx)
                    }
                    K::OpenInFileManager => self.reveal_in_file_manager(Some(path), window, cx),
                    K::CopyPath => self.copy_to_clipboard(path, window, cx),
                    K::Blame if !is_dir => self.show_blame(path, None, cx),
                    K::FileHistory if !is_dir => self.show_file_history(path, cx),
                    K::ViewAtHead if area.is_none() => {
                        self.select_work_file(path, Some("HEAD".into()), cx)
                    }
                    K::AddToGitignore => self.ignore_path(
                        path,
                        state,
                        PatternKind::Exact,
                        IgnoreFile::Gitignore,
                        window,
                        cx,
                    ),
                    K::AddExtensionToGitignore if !is_dir => self.ignore_path(
                        path,
                        state,
                        PatternKind::Extension,
                        IgnoreFile::Gitignore,
                        window,
                        cx,
                    ),
                    K::IgnoreLocally => self.ignore_path(
                        path,
                        state,
                        PatternKind::Exact,
                        IgnoreFile::LocalExclude,
                        window,
                        cx,
                    ),
                    K::Untrack if tracked => self.untrack_prompt(path, state, window, cx),
                    K::CopyAsPatch => {
                        if let Some(a) = area {
                            self.copy_patch(Osier::file_source(&path, a), window, cx)
                        }
                    }
                    K::SaveToPatches => {
                        if let Some(a) = area {
                            let source = Osier::file_source(&path, a);
                            self.save_patch_prompt(source, path, window, cx)
                        }
                    }
                    K::Shelve => {
                        if let Some(a) = area {
                            self.shelve_prompt(vec![(path, a)], window, cx)
                        }
                    }
                    _ => {}
                }
            }
            Row::Commit { hash, is_head } => match kind {
                K::CopyAsPatch => {
                    self.copy_patch(PatchSource::Commits { hashes: vec![hash] }, window, cx)
                }
                K::SaveToPatches => {
                    let name = self.commit_subject(&hash).unwrap_or_else(|| hash.clone());
                    self.save_patch_prompt(
                        PatchSource::Commits { hashes: vec![hash] },
                        name,
                        window,
                        cx,
                    )
                }
                K::ExportPatch => {
                    let name = format!("{}.patch", &hash[..hash.len().min(7)]);
                    self.export_patch(
                        PatchSource::Commits { hashes: vec![hash] },
                        name,
                        window,
                        cx,
                    )
                }
                K::Checkout => self.checkout_commit(hash, window, cx),
                K::NewBranchHere => self.prompt_new_branch(Some(hash), window, cx),
                K::TagHere => self.prompt_tag(hash, window, cx),
                K::CompareWithWorkingTree => self.select_compare(hash, "HEAD".into(), false, cx),
                K::ResetSoft => self.confirm_reset(hash, ResetMode::Soft, window, cx),
                K::ResetMixed => self.confirm_reset(hash, ResetMode::Mixed, window, cx),
                K::ResetHard => self.confirm_reset(hash, ResetMode::Hard, window, cx),
                K::Revert => self.confirm_revert(hash, window, cx),
                K::CherryPick => self.confirm_cherry_pick(hash, window, cx),
                K::InteractiveRebase => self.open_interactive_rebase(hash, window, cx),
                K::UndoLastCommit if is_head => self.confirm_undo_commit(window, cx),
                K::CopyHash => self.copy_to_clipboard(hash, window, cx),
                K::OpenOnGithub => self.open_commit_on_github(hash, window, cx),
                _ => {}
            },
            Row::Patch { id, name } => match kind {
                K::Apply => self.apply_stored(id, false, window, cx),
                K::Unshelve => self.apply_stored(id, true, window, cx),
                K::CopyAsPatch => self.copy_stored(id, window, cx),
                K::ExportPatch => self.export_stored(id, window, cx),
                K::Rename => self.rename_stored_prompt(id, name, window, cx),
                K::Delete => self.confirm_delete_stored(id, name, window, cx),
                _ => {}
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ROWS;

    #[test]
    fn every_kind_is_listed_once() {
        let mut ids: Vec<&str> = ROWS.iter().map(|r| r.0).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), ROWS.len());
        let mut kinds: Vec<String> = ROWS.iter().map(|r| format!("{:?}", r.3)).collect();
        kinds.sort();
        kinds.dedup();
        assert_eq!(kinds.len(), ROWS.len());
    }
}
