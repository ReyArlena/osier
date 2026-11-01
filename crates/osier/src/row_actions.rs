//! Context-menu actions as key bindings. Each fires on the selected row of the current tab
//! (Changes, Files, Log, Patches), doing what the same entry in that row's right-click menu
//! does. One binding covers every row that offers the action; rows that do not offer it ignore
//! the key. Destructive actions still open their confirmation dialog.

use gpui_kit::*;
use osier_git::ops::ignore::IgnoreFile;
use osier_git::ops::integrate::MergeMode;
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
    Squash,
    UndoLastCommit,
    CopyHash,
    OpenOnGithub,
    Apply,
    Unshelve,
    Pop,
    Rename,
    Delete,
    MergeIntoCurrent,
    RebaseCurrentOnto,
    CompareWithCurrent,
    ShowCommit,
    CopyName,
    EditRemoteUrl,
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
        "Log commits and tags (detached), branches",
        RowKind::Checkout,
    ),
    (
        "row_new_branch_here",
        "New branch here",
        "Log commits and local branches",
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
        "row_squash",
        "Squash selected commits",
        "Log: a shift-click range of commits",
        RowKind::Squash,
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
    (
        "row_apply",
        "Apply",
        "Patches and stashes rows",
        RowKind::Apply,
    ),
    (
        "row_unshelve",
        "Unshelve",
        "Patches rows",
        RowKind::Unshelve,
    ),
    ("row_pop", "Pop", "Stashes rows", RowKind::Pop),
    (
        "row_rename",
        "Rename",
        "Local branches, remotes and patches",
        RowKind::Rename,
    ),
    (
        "row_delete",
        "Delete / Drop",
        "Local branches, tags, remotes (remove), stashes (drop) and patches",
        RowKind::Delete,
    ),
    (
        "row_merge_into_current",
        "Merge into current branch",
        "Branches",
        RowKind::MergeIntoCurrent,
    ),
    (
        "row_rebase_current_onto",
        "Rebase current branch onto this",
        "Local branches",
        RowKind::RebaseCurrentOnto,
    ),
    (
        "row_compare_with_current",
        "Compare with current branch",
        "Branches",
        RowKind::CompareWithCurrent,
    ),
    (
        "row_show_commit",
        "Show commit",
        "Tags",
        RowKind::ShowCommit,
    ),
    (
        "row_copy_name",
        "Copy name",
        "Branches, tags and remotes",
        RowKind::CopyName,
    ),
    (
        "row_edit_remote_url",
        "Edit remote URL",
        "Remotes",
        RowKind::EditRemoteUrl,
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
        Squash,
        UndoLastCommit,
        CopyHash,
        OpenOnGithub,
        Apply,
        Unshelve,
        Pop,
        Rename,
        Delete,
        MergeIntoCurrent,
        RebaseCurrentOnto,
        CompareWithCurrent,
        ShowCommit,
        CopyName,
        EditRemoteUrl
    )
}

/// A selected row in the Branches tab, kept by name so it survives refreshes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RefRow {
    Branch { name: String, remote: bool },
    Tag(String),
    Remote(String),
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
    Branch {
        name: String,
        remote: bool,
        is_head: bool,
    },
    Tag {
        name: String,
        target: String,
    },
    Remote {
        name: String,
        url: String,
    },
    Stash {
        index: usize,
        message: String,
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
            Tab::Branches => {
                let snapshot = self.active_project()?.snapshot.as_ref()?;
                match self.ref_selected.as_ref()? {
                    RefRow::Branch { name, remote } => snapshot
                        .branches
                        .iter()
                        .find(|b| &b.name == name && b.is_remote == *remote)
                        .map(|b| Row::Branch {
                            name: b.name.clone(),
                            remote: b.is_remote,
                            is_head: b.is_head,
                        }),
                    RefRow::Tag(name) => {
                        snapshot
                            .tags
                            .iter()
                            .find(|t| &t.name == name)
                            .map(|t| Row::Tag {
                                name: t.name.clone(),
                                target: t.target.clone(),
                            })
                    }
                    RefRow::Remote(name) => {
                        snapshot
                            .remotes
                            .iter()
                            .find(|r| &r.name == name)
                            .map(|r| Row::Remote {
                                name: r.name.clone(),
                                url: r.fetch_url.clone(),
                            })
                    }
                }
            }
            Tab::Stashes => {
                let snapshot = self.active_project()?.snapshot.as_ref()?;
                let oid = self.stash_selected.as_ref()?;
                snapshot
                    .stashes
                    .iter()
                    .find(|s| &s.oid == oid)
                    .map(|s| Row::Stash {
                        index: s.index,
                        message: s.message.clone(),
                    })
            }
        }
    }

    /// The Branches tab's selectable rows in screen order: local branches, remote branches,
    /// tags, remotes.
    pub(crate) fn ref_rows(&self) -> Vec<RefRow> {
        let Some(snapshot) = self.active_project().and_then(|p| p.snapshot.as_ref()) else {
            return Vec::new();
        };
        let branches = |remote: bool| {
            snapshot
                .branches
                .iter()
                .filter(move |b| b.is_remote == remote)
                .map(move |b| RefRow::Branch {
                    name: b.name.clone(),
                    remote,
                })
        };
        branches(false)
            .chain(branches(true))
            .chain(snapshot.tags.iter().map(|t| RefRow::Tag(t.name.clone())))
            .chain(
                snapshot
                    .remotes
                    .iter()
                    .map(|r| RefRow::Remote(r.name.clone())),
            )
            .collect()
    }

    /// Select a Branches-tab row (click).
    pub(crate) fn select_ref(&mut self, row: RefRow, cx: &mut Context<Self>) {
        self.ref_selected = Some(row);
        cx.notify();
    }

    /// Select a stash row by its commit id (click).
    pub(crate) fn select_stash(&mut self, oid: String, cx: &mut Context<Self>) {
        self.stash_selected = Some(oid);
        cx.notify();
    }

    /// Up/down in the Branches and Stashes tabs.
    pub(crate) fn move_ref_or_stash_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        fn step<T: PartialEq + Clone>(rows: &[T], current: Option<&T>, delta: isize) -> Option<T> {
            if rows.is_empty() {
                return None;
            }
            let next = match current.and_then(|c| rows.iter().position(|r| r == c)) {
                Some(i) => i.saturating_add_signed(delta).min(rows.len() - 1),
                None => 0,
            };
            rows.get(next).cloned()
        }
        match self.tab {
            Tab::Branches => {
                let rows = self.ref_rows();
                self.ref_selected = step(&rows, self.ref_selected.as_ref(), delta);
            }
            Tab::Stashes => {
                let rows: Vec<String> = self
                    .active_project()
                    .and_then(|p| p.snapshot.as_ref())
                    .map(|s| s.stashes.iter().map(|s| s.oid.clone()).collect())
                    .unwrap_or_default();
                self.stash_selected = step(&rows, self.stash_selected.as_ref(), delta);
            }
            _ => return,
        }
        cx.notify();
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
                K::Squash => self.confirm_squash(window, cx),
                K::UndoLastCommit if is_head => self.confirm_undo_commit(window, cx),
                K::CopyHash => self.copy_to_clipboard(hash, window, cx),
                K::OpenOnGithub => self.open_commit_on_github(hash, window, cx),
                _ => {}
            },
            Row::Branch {
                name,
                remote,
                is_head,
            } => match kind {
                K::Checkout if !is_head => self.checkout_branch(name, remote, window, cx),
                K::NewBranchHere if !remote => self.prompt_new_branch(Some(name), window, cx),
                K::Rename if !remote => self.prompt_rename_branch(name, window, cx),
                K::Delete if !remote && !is_head => {
                    self.confirm_delete_branch(name, false, window, cx)
                }
                K::MergeIntoCurrent if !is_head => {
                    self.merge_branch(name, MergeMode::Default, window, cx)
                }
                K::RebaseCurrentOnto if !remote && !is_head => {
                    self.confirm_rebase_onto(name, window, cx)
                }
                K::CompareWithCurrent if !is_head => {
                    let current = self
                        .active_project()
                        .and_then(|p| p.snapshot.as_ref())
                        .and_then(|s| s.status.branch.head.clone())
                        .unwrap_or_else(|| "HEAD".into());
                    self.select_compare(current, name, true, cx)
                }
                K::CopyName => self.copy_to_clipboard(name, window, cx),
                _ => {}
            },
            Row::Tag { name, target } => match kind {
                K::ShowCommit => self.select_commit_hash(target, cx),
                K::Checkout => self.checkout_commit(target, window, cx),
                K::CopyName => self.copy_to_clipboard(name, window, cx),
                K::Delete => self.confirm_delete_tag(name, window, cx),
                _ => {}
            },
            Row::Remote { name, url } => match kind {
                K::EditRemoteUrl => self.prompt_remote_url(name, url, window, cx),
                K::Rename => self.prompt_rename_remote(name, window, cx),
                K::Delete => self.confirm_remove_remote(name, window, cx),
                K::CopyName => self.copy_to_clipboard(name, window, cx),
                _ => {}
            },
            Row::Stash { index, message } => match kind {
                K::Apply => self.stash_apply(index, false, window, cx),
                K::Pop => self.stash_apply(index, true, window, cx),
                K::Delete => self.confirm_stash_drop(index, message, window, cx),
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

/// A context-menu entry for `kind`, labelled `label`, with its key (if bound) on the right,
/// prefix included, read from the live keymap.
pub(crate) fn menu_item(
    label: impl Into<SharedString>,
    kind: RowKind,
) -> gpui_kit::component::menu::PopupMenuItem {
    use gpui_kit::component::{ActiveTheme as _, h_flex};
    use gpui_kit::prelude::FluentBuilder as _;
    let label = label.into();
    gpui_kit::component::menu::PopupMenuItem::element(move |_, cx| {
        let hint = key_hint(kind, cx);
        h_flex()
            .w_full()
            .gap_6()
            .justify_between()
            .child(label.clone())
            .when_some(hint, |this, hint| {
                this.child(
                    div()
                        .flex_none()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(hint),
                )
            })
    })
}

/// "I", "Prefix I", or `None` when `kind` has no key.
pub(crate) fn key_hint(kind: RowKind, cx: &App) -> Option<String> {
    use crate::keymap::{KeyMode, Keymap, pretty_key, split_mode};
    let id = ROWS.iter().find(|r| r.3 == kind)?.0;
    let first = Keymap::global(cx).keys_for(id).first()?;
    let (mode, key) = split_mode(first);
    Some(match mode {
        KeyMode::Prefix => format!("Prefix {}", pretty_key(key)),
        KeyMode::Plain | KeyMode::Either => pretty_key(key),
    })
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
