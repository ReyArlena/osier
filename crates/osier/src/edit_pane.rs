//! Editing in the diff pane: the merge result of a conflicted file, or a working-tree file.

use gpui_kit::component::input::{EditorState, InputEvent};
use gpui_kit::*;
use osier_git::ops::integrate;

use crate::app::Osier;
use crate::notices::{Notice, WindowNoticeExt as _};
use crate::project::{Area, Selection};

/// What the editor is editing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditKind {
    /// A conflicted file; saving without markers left marks it resolved.
    MergeResult,
    /// Any other working-tree file; saving just writes it.
    File,
}

pub struct FileEditor {
    pub path: String,
    pub kind: EditKind,
    pub state: Entity<EditorState>,
    /// Re-renders on every edit, so the Save button's label follows the text.
    _changes: Subscription,
}

impl std::fmt::Debug for FileEditor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileEditor")
            .field("path", &self.path)
            .field("kind", &self.kind)
            .finish()
    }
}

pub(crate) fn language_for(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "rs" => "rust",
        "toml" => "toml",
        "md" => "markdown",
        "js" | "mjs" | "cjs" => "javascript",
        "ts" => "typescript",
        "tsx" => "tsx",
        "php" => "php",
        "css" => "css",
        "html" | "htm" => "html",
        "py" => "python",
        "go" => "go",
        "sh" | "bash" => "bash",
        "yml" | "yaml" => "yaml",
        _ => "text",
    }
}

impl Osier {
    /// Open the editor for the selected file (merge result for conflicts, plain otherwise).
    /// A merge whose conflicts all have a choice starts from the resolved text; otherwise the
    /// file is read in the background (no size limit here, so it can be large) and the editor
    /// opens once it is in.
    pub(crate) fn open_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((path, area)) = self.diff_area() else {
            return;
        };
        let kind = match area {
            Area::Conflicted => EditKind::MergeResult,
            _ => EditKind::File,
        };
        if kind == EditKind::MergeResult
            && let Some(text) = self.diff.read(cx).resolved_text(&path)
        {
            self.install_editor(path, kind, text, window, cx);
            return;
        }
        let Some(full) = self.active_project().map(|p| p.repo.workdir().join(&path)) else {
            return;
        };
        self.in_background(
            crate::background::LOADING_FILE,
            move || std::fs::read_to_string(full).ok(),
            move |this, text, window, cx| match text {
                Some(text) => this.install_editor(path, kind, text, window, cx),
                None => window.push_notice(Notice::warning("Cannot read the file as text"), cx),
            },
            window,
            cx,
        );
    }

    fn install_editor(
        &mut self,
        path: String,
        kind: EditKind,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let language = language_for(&path);
        let state = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(language)
                .line_number(true)
                .default_value(text)
        });
        let changes = cx.subscribe(&state, |_, _, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::Change) {
                cx.notify();
            }
        });
        self.editor = Some(FileEditor {
            path,
            kind,
            state,
            _changes: changes,
        });
        cx.notify();
    }

    /// The editor's Save button: "Save and resolve" for a conflicted file with no markers
    /// left (saving marks it resolved), else "Save". With its tooltip.
    pub(crate) fn save_button_text(&self, cx: &App) -> (&'static str, &'static str) {
        match &self.editor {
            Some(e) if e.kind == EditKind::MergeResult => {
                if integrate::has_conflict_markers(&e.state.read(cx).value()) {
                    (
                        "Save",
                        "Saves the file. It stays conflicted until no conflict markers are left.",
                    )
                } else {
                    (
                        "Save and resolve",
                        "Saves the file and marks the conflict resolved",
                    )
                }
            }
            _ => ("Save", "Save the file"),
        }
    }

    pub(crate) fn close_editor(&mut self, cx: &mut Context<Self>) {
        self.editor = None;
        self.reload_selection(cx);
        cx.notify();
    }

    /// Write the editor's text to the working tree. For a merge result without markers left,
    /// also mark the file resolved.
    pub(crate) fn save_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = &self.editor else {
            return;
        };
        let path = editor.path.clone();
        let kind = editor.kind;
        let text = editor.state.read(cx).value().to_string();
        let markers_left = integrate::has_conflict_markers(&text);
        let p = path.clone();
        let summary = match kind {
            EditKind::MergeResult if markers_left => {
                format!("Saved {path} (conflict markers remain)")
            }
            EditKind::MergeResult => format!("Resolved {path}"),
            EditKind::File => format!("Saved {path}"),
        };
        self.editor = None;
        self.run_op(
            summary,
            move |repo| {
                integrate::write_resolution(repo, &p, &text)?;
                if kind == EditKind::MergeResult && !markers_left {
                    integrate::mark_resolved(repo, std::slice::from_ref(&p))?;
                }
                Ok(())
            },
            window,
            cx,
        );
    }

    /// Apply the choices made with the Diff component's Accept buttons: write the resolved
    /// file and mark it resolved.
    pub(crate) fn apply_conflict_resolution(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((path, Area::Conflicted)) = self.diff_area() else {
            return;
        };
        let Some(text) = self.diff.read(cx).resolved_text(&path) else {
            window.push_notice(
                Notice::warning("Choose a side for every conflict first"),
                cx,
            );
            return;
        };
        let p = path.clone();
        self.run_op(
            format!("Resolved {path}"),
            move |repo| {
                integrate::write_resolution(repo, &p, &text)?;
                integrate::mark_resolved(repo, std::slice::from_ref(&p))
            },
            window,
            cx,
        );
    }

    pub(crate) fn can_apply_resolution(&self, cx: &App) -> bool {
        match self.active_project().map(|p| &p.selection) {
            Some(Selection::File {
                path,
                area: Area::Conflicted,
            }) => self.diff.read(cx).resolved_text(path).is_some(),
            _ => false,
        }
    }
}
