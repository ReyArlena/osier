//! Viewing a file from the Files tab: code, markdown, image or binary.

use gpui_kit::component::input::EditorState;
use gpui_kit::*;
use osier_git::files;

use crate::app::Osier;
use crate::edit_pane::language_for;

pub enum Viewer {
    /// `state` is created on first render (editors need a `Window`).
    Code {
        text: SharedString,
        language: &'static str,
        state: Option<Entity<EditorState>>,
    },
    Markdown {
        text: SharedString,
        language: &'static str,
        state: Option<Entity<EditorState>>,
    },
    Image {
        full_path: std::path::PathBuf,
        bytes: u64,
    },
    Binary {
        bytes: u64,
    },
    /// A Git LFS pointer: the content is not in the repository.
    Lfs {
        oid: String,
        bytes: u64,
    },
    Missing,
    Error(String),
}

impl std::fmt::Debug for Viewer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Viewer::Code { .. } => "Code",
            Viewer::Markdown { .. } => "Markdown",
            Viewer::Image { .. } => "Image",
            Viewer::Binary { .. } => "Binary",
            Viewer::Lfs { .. } => "Lfs",
            Viewer::Missing => "Missing",
            Viewer::Error(_) => "Error",
        };
        f.write_str(name)
    }
}

pub fn is_image(path: &str) -> bool {
    matches!(
        path.rsplit('.')
            .next()
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "svg")
    )
}

pub fn is_markdown(path: &str) -> bool {
    matches!(
        path.rsplit('.')
            .next()
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("md" | "markdown" | "mdx")
    )
}

impl Osier {
    /// Read the file (working tree or at `rev`) and pick a viewer. Synchronous: files are
    /// capped by the large-file setting, so reads are quick.
    pub(crate) fn load_viewer(
        &mut self,
        path: String,
        rev: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.active_project() else {
            return;
        };
        let full = project.repo.workdir().join(&path);
        let limit = self.settings.diff.large_file_bytes;
        let bytes: Option<Vec<u8>> = match &rev {
            None => match std::fs::metadata(&full) {
                Ok(meta) if meta.len() > limit && !is_image(&path) => {
                    self.viewer = Some(Viewer::Binary { bytes: meta.len() });
                    cx.notify();
                    return;
                }
                Ok(_) => std::fs::read(&full).ok(),
                Err(_) => None,
            },
            Some(rev) => match files::show_at(&project.repo, rev, &path) {
                Ok(b) => b,
                Err(e) => {
                    self.viewer = Some(Viewer::Error(e.to_string()));
                    cx.notify();
                    return;
                }
            },
        };
        let Some(bytes) = bytes else {
            self.viewer = Some(Viewer::Missing);
            cx.notify();
            return;
        };
        let size = bytes.len() as u64;
        self.viewer = Some(if is_image(&path) && rev.is_none() {
            Viewer::Image {
                full_path: full,
                bytes: size,
            }
        } else if files::is_lfs_pointer(&bytes) {
            let text = String::from_utf8_lossy(&bytes);
            let (oid, bytes) = files::lfs_pointer_info(&text).unwrap_or_default();
            Viewer::Lfs { oid, bytes }
        } else if files::looks_binary(&bytes) {
            Viewer::Binary { bytes: size }
        } else {
            let text: SharedString = String::from_utf8_lossy(&bytes).into_owned().into();
            let language = language_for(&path);
            if is_markdown(&path) {
                self.markdown_as_code = self.settings.viewer.markdown_mode_for(&path)
                    == osier_config::settings::MarkdownMode::Code;
                Viewer::Markdown {
                    text,
                    language,
                    state: None,
                }
            } else {
                Viewer::Code {
                    text,
                    language,
                    state: None,
                }
            }
        });
        cx.notify();
    }

    /// The read-only editor for the current viewer, created on first use.
    pub(crate) fn viewer_editor(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<EditorState>> {
        let (text, language, slot) = match self.viewer.as_mut()? {
            Viewer::Code {
                text,
                language,
                state,
            }
            | Viewer::Markdown {
                text,
                language,
                state,
            } => (text.clone(), *language, state),
            _ => return None,
        };
        if slot.is_none() {
            let state = cx.new(|cx| {
                EditorState::new(window, cx)
                    .language(language)
                    .line_number(true)
                    .default_value(text)
            });
            *slot = Some(state);
        }
        slot.clone()
    }

    /// Switch the markdown viewer between rendered and source, and remember the choice for
    /// this file type (`viewer.markdown_modes` in config.toml).
    pub(crate) fn toggle_markdown_code(&mut self, cx: &mut Context<Self>) {
        use osier_config::settings::{MarkdownMode, extension};
        self.markdown_as_code = !self.markdown_as_code;
        let path = match self.active_project().map(|p| &p.selection) {
            Some(crate::project::Selection::WorkFile { path, .. }) => Some(path.clone()),
            _ => None,
        };
        if let Some(ext) = path.as_deref().and_then(extension) {
            let mode = if self.markdown_as_code {
                MarkdownMode::Code
            } else {
                MarkdownMode::Rendered
            };
            if mode == self.settings.viewer.markdown_default {
                self.settings.viewer.markdown_modes.remove(&ext);
            } else {
                self.settings.viewer.markdown_modes.insert(ext, mode);
            }
            self.save_settings();
        }
        cx.notify();
    }
}
