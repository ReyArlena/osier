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

/// What [`read_for_viewer`] found, made into a [`Viewer`] on the UI thread.
enum Loaded {
    Missing,
    Error(String),
    Binary(u64),
    Image {
        full_path: std::path::PathBuf,
        bytes: u64,
    },
    Lfs {
        oid: String,
        bytes: u64,
    },
    Text(String),
}

/// Read a file for the viewer: the working tree copy (up to `limit` bytes, images whole) or
/// the one at `rev`. Blocking: runs in the background.
fn read_for_viewer(
    repo: &osier_git::Repository,
    path: &str,
    rev: Option<&str>,
    limit: u64,
) -> Loaded {
    let full = repo.workdir().join(path);
    let bytes: Option<Vec<u8>> = match rev {
        None => match std::fs::metadata(&full) {
            Ok(meta) if meta.len() > limit && !is_image(path) => {
                return Loaded::Binary(meta.len());
            }
            Ok(_) => std::fs::read(&full).ok(),
            Err(_) => None,
        },
        Some(rev) => match files::show_at(repo, rev, path) {
            Ok(b) => b,
            Err(e) => return Loaded::Error(e.to_string()),
        },
    };
    let Some(bytes) = bytes else {
        return Loaded::Missing;
    };
    let size = bytes.len() as u64;
    if is_image(path) && rev.is_none() {
        Loaded::Image {
            full_path: full,
            bytes: size,
        }
    } else if files::is_lfs_pointer(&bytes) {
        let text = String::from_utf8_lossy(&bytes);
        let (oid, bytes) = files::lfs_pointer_info(&text).unwrap_or_default();
        Loaded::Lfs { oid, bytes }
    } else if files::looks_binary(&bytes) {
        Loaded::Binary(size)
    } else {
        Loaded::Text(
            String::from_utf8(bytes)
                .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()),
        )
    }
}

impl Osier {
    /// Read the file (working tree or at `rev`) and pick a viewer. The read (and the
    /// `git show` for a revision) runs in the background; the previous viewer stays until the
    /// result is in, and a result for a selection that has moved on is dropped.
    pub(crate) fn load_viewer(
        &mut self,
        path: String,
        rev: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.active_project() else {
            return;
        };
        let repo = project.repo.clone();
        let limit = self.settings.diff.large_file_bytes;
        self.viewer_load += 1;
        let load = self.viewer_load;
        let read_path = path.clone();
        let wanted = crate::project::Selection::WorkFile {
            path: path.clone(),
            rev: rev.clone(),
        };
        self.in_background_user_cx(
            crate::background::LOADING_FILE,
            move || read_for_viewer(&repo, &read_path, rev.as_deref(), limit),
            move |this, loaded, cx| {
                // Another file was picked meanwhile, or the selection moved to a diff.
                let still_wanted = this.active_project().is_some_and(|p| p.selection == wanted);
                if this.viewer_load != load || !still_wanted {
                    return;
                }
                this.viewer = Some(match loaded {
                    Loaded::Missing => Viewer::Missing,
                    Loaded::Error(e) => Viewer::Error(e),
                    Loaded::Binary(bytes) => Viewer::Binary { bytes },
                    Loaded::Image { full_path, bytes } => Viewer::Image { full_path, bytes },
                    Loaded::Lfs { oid, bytes } => Viewer::Lfs { oid, bytes },
                    Loaded::Text(text) => {
                        let text: SharedString = text.into();
                        let language = language_for(&path);
                        if is_markdown(&path) {
                            this.markdown_as_code = this.settings.viewer.markdown_mode_for(&path)
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
                    }
                });
                cx.notify();
            },
            cx,
        );
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
