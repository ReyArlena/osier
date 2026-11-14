use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::diff::{
    Diff, DiffDecoration, DiffGutterMarker, DiffLinePosition, DiffLineRange, DiffMode, DiffSide,
};
use gpui_kit::component::input::Editor;
use gpui_kit::component::text::TextView;

use crate::viewer::Viewer;
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::separator::Separator;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Selectable as _, Sizable as _, StyledExt as _,
    h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app::Osier;
use crate::app::hunk_ops::HunkAction;
use crate::project::{Area, Selection};
use crate::test_ids::TestId as _;

/// Title and explanation for a changed file with no diff to show: why it is in the list.
pub(crate) fn unchanged_text(reason: &osier_git::staging::Unchanged) -> (&'static str, String) {
    use osier_git::staging::Unchanged as U;
    const SAME: &str = "Its text is the same as in the last commit.";
    match reason {
        U::Mode { old, new } => {
            let what = match (old.as_str(), new.as_str()) {
                ("100644", "100755") => "This file was made executable.".to_string(),
                ("100755", "100644") => "This file is no longer executable.".to_string(),
                _ => format!("The file mode changed from {old} to {new}."),
            };
            ("Only the permissions changed", format!("{what} {SAME}"))
        }
        U::Renamed { from } => (
            "Moved without changes",
            format!("This file came from {from}. {SAME}"),
        ),
        U::EmptyAdded => (
            "New empty file",
            "This file is new and has nothing in it.".to_string(),
        ),
        U::EmptyDeleted => (
            "Empty file deleted",
            "This file was deleted and had nothing in it.".to_string(),
        ),
        U::StagedOnly => (
            "Changed on disk since it was staged",
            "This file was changed on disk after it was staged, and now matches the last \
             commit again. The staged version is out of date and would still be committed. \
             Click its checkbox to drop it."
                .to_string(),
        ),
    }
}

impl Osier {
    pub(crate) fn render_diff_pane(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let project = self.active_project();
        let selection = project
            .map(|p| p.selection.clone())
            .unwrap_or(Selection::None);
        let mode = self.diff.read(cx).mode();
        let muted = cx.theme().muted_foreground;

        // Header: what is being shown.
        let (icon, title, tag): (Lucide, String, Option<Tag>) = match &selection {
            Selection::None => (Lucide::FileDiff, "No selection".into(), None),
            Selection::File { path, area } => (
                Lucide::FileDiff,
                path.clone(),
                Some(match area {
                    Area::Changed => Tag::warning().xsmall().child("changed"),
                    Area::Untracked => Tag::secondary().xsmall().child("unversioned"),
                    Area::Conflicted => Tag::danger().xsmall().child("conflict"),
                }),
            ),
            Selection::Compare {
                from,
                to,
                since_merge_base,
            } => (
                Lucide::GitCompare,
                if *since_merge_base {
                    format!("{to} since it left {from}")
                } else {
                    format!("{from} → {to}")
                },
                Some(Tag::info().xsmall().child("compare")),
            ),
            Selection::WorkFile { path, rev } => (
                Lucide::File,
                path.clone(),
                Some(match rev {
                    Some(r) => Tag::secondary()
                        .xsmall()
                        .child(r[..r.len().min(7)].to_string()),
                    None => Tag::secondary().xsmall().child("working tree"),
                }),
            ),
            Selection::Patch { name, .. } => (
                Lucide::Package,
                name.clone(),
                Some(Tag::info().xsmall().child("patch")),
            ),
            Selection::Commit { hash } => {
                let commit = project
                    .and_then(|p| p.snapshot.as_ref())
                    .and_then(|s| s.commits.iter().find(|c| &c.hash == hash));
                (
                    Lucide::GitCommitHorizontal,
                    commit
                        .map(|c| c.subject.clone())
                        .unwrap_or_else(|| hash.clone()),
                    Some(
                        Tag::secondary()
                            .xsmall()
                            .child(hash[..hash.len().min(7)].to_string()),
                    ),
                )
            }
        };

        let blame = self.blame.clone();
        let (icon, title, tag) = match &blame {
            Some(b) => (
                Lucide::UserRoundSearch,
                format!("Blame: {}", b.path),
                Some(Tag::info().xsmall().child(match &b.rev {
                    Some(r) => r[..r.len().min(7)].to_string(),
                    None => "working tree".to_string(),
                })),
            ),
            None => (icon, title, tag),
        };
        let lfs = self.diff_model.iter().any(|f| {
            f.hunks.iter().any(|h| {
                h.lines.iter().any(|l| {
                    l.text
                        .starts_with("version https://git-lfs.github.com/spec/v1")
                })
            })
        });
        let area = self.diff_area();
        // The selected lines, or the ones a line button was pressed with (see `line_pick`).
        let line_range = self.diff.read(cx).selected_lines().or_else(|| {
            let pick = self.line_pick.as_ref()?;
            let (path, _) = area.as_ref()?;
            (pick.path().as_ref() == path.as_str()).then(|| pick.clone())
        });
        let line_actions: &[HunkAction] = match &area {
            Some((_, a)) if line_range.is_some() => HunkAction::for_area(*a),
            _ => &[],
        };

        let editing = self.editor.is_some();
        let viewing = matches!(&selection, Selection::WorkFile { .. });
        let three_way = self.three_way_target().is_some();
        let three_way_mode =
            self.settings.diff.conflicts == osier_config::settings::ConflictView::ThreeWay;
        let conflicted = matches!(&area, Some((_, Area::Conflicted)));
        let body: AnyElement = if let Some(view) = &blame {
            self.render_blame(view, cx)
        } else if let Some(editor) = &self.editor {
            // No border of its own: the header's bottom border already separates it, and a
            // second line under it reads as one thick rule.
            crate::editor_hover::hover_lines(
                Editor::new(&editor.state).bordered(false).size_full(),
                &editor.state,
                cx,
            )
        } else if three_way {
            match self.merge_view(window, cx) {
                Ok(Some(view)) => view.into_any_element(),
                Ok(None) => v_flex()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .child(Spinner::new())
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .child("Reading the versions…"),
                    )
                    .into_any_element(),
                Err(error) => self
                    .empty_state("Cannot show the merge tool", &error, cx)
                    .into_any_element(),
            }
        } else if viewing {
            self.render_viewer(window, cx)
        } else if self.diff_loading {
            v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_3()
                .child(Spinner::new())
                .child(div().text_sm().text_color(muted).child("Reading diff…"))
                .into_any_element()
        } else if let Some(size) = self.diff_too_large {
            let mb = size as f64 / (1024.0 * 1024.0);
            v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_3()
                .p_6()
                .child(div().font_medium().child("Large file"))
                .child(div().text_sm().text_center().text_color(muted).child(format!(
                    "This file is {mb:.1} MB, above the {:.1} MB limit (diff.large_file_bytes).",
                    self.settings.diff.large_file_bytes as f64 / (1024.0 * 1024.0)
                )))
                .child(
                    Button::new("load-anyway").test_id("load-anyway")
                        .outline()
                        .small()
                        .label("Load anyway")
                        .on_click(cx.listener(|this, _, _, cx| this.force_large_diff(cx))),
                )
                .into_any_element()
        } else if self.diff_no_markers
            && let Some((path, Area::Conflicted)) = &area
        {
            let path = path.clone();
            v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_3()
                .p_6()
                .child(div().font_medium().child("No conflict markers left"))
                .child(div().text_sm().text_center().text_color(muted).child(
                    "The file was edited or saved after the merge stopped, so there are no \
                     conflict blocks to show. Mark it resolved if it is right, or merge it again \
                     from the original versions with 3-way.",
                ))
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("no-markers-three-way")
                                .test_id("no-markers-three-way")
                                .outline()
                                .small()
                                .label("Open in 3-way")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.set_conflict_view(
                                        osier_config::settings::ConflictView::ThreeWay,
                                        cx,
                                    )
                                })),
                        )
                        .child(
                            Button::new("no-markers-resolve")
                                .test_id("no-markers-resolve")
                                .primary()
                                .small()
                                .label("Mark resolved")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.mark_resolved(path.clone(), window, cx)
                                })),
                        ),
                )
                .into_any_element()
        } else if let Some(error) = &self.diff_error {
            v_flex()
                .size_full()
                .p_6()
                .gap_2()
                .child(div().font_medium().child("Could not read the diff"))
                .child(div().text_sm().text_color(muted).child(error.clone()))
                .into_any_element()
        } else if selection == Selection::None {
            // Each tab keeps its own selection here; say what this tab's would be.
            let (title, hint) = match self.tab {
                crate::app::Tab::Changes => (
                    "Nothing to show",
                    "Pick a changed file to see its diff here.",
                ),
                crate::app::Tab::Log => {
                    ("No commit picked", "Pick a commit to see what it changed.")
                }
                crate::app::Tab::Branches => (
                    "Nothing to compare",
                    "Use Compare on a branch to see how it differs from the current one.",
                ),
                crate::app::Tab::Stashes => ("No stash picked", "Pick a stash to see its changes."),
                crate::app::Tab::Files => ("No file open", "Pick a file to view it here."),
                crate::app::Tab::Patches => ("No patch picked", "Pick a patch to see it here."),
                crate::app::Tab::Config => (
                    "Read only",
                    "Change a setting in the file it comes from: Open, under Other sources.",
                ),
            };
            self.empty_state(title, hint, cx).into_any_element()
        } else if self.diff_files == 0 {
            match &self.diff_unchanged {
                Some(reason) => {
                    let (title, hint) = unchanged_text(reason);
                    self.empty_state(title, &hint, cx).into_any_element()
                }
                None => self
                    .empty_state("No changes", "This selection has no textual changes.", cx)
                    .into_any_element(),
            }
        } else {
            let mut view = Diff::new(&self.diff)
                .line_number(self.settings.diff.line_numbers)
                .soft_wrap(self.settings.diff.soft_wrap);
            if let Some((path, Area::Changed)) = &area
                && let Some(staging) = self.diff_staging.clone()
            {
                view = self.staging_gutter(view, path.clone(), staging, cx);
            }
            // An unversioned file has no hunk controls: its diff is one all-added change,
            // and the file's checkbox adds it (selected lines can still be staged).
            view.into_any_element()
        };

        let showing_diff = blame.is_none()
            && !editing
            && !three_way
            && !viewing
            && !self.diff_loading
            && self.diff_too_large.is_none()
            && !self.diff_no_markers
            && self.diff_error.is_none()
            && selection != Selection::None
            && self.diff_files > 0;
        let toolbar = showing_diff.then(|| self.render_diff_toolbar(&selection, cx));

        v_flex()
            .size_full()
            .min_w_0()
            .overflow_hidden()
            .child(
                h_flex()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .gap_y_1()
                    .flex_wrap()
                    .items_center()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(Icon::new(icon))
                    .child(
                        div()
                            .text_sm()
                            .font_medium()
                            .truncate()
                            .min_w_0()
                            .child(title),
                    )
                    .when_some(tag, |this, t| this.child(t))
                    .when(lfs, |this| {
                        this.child(Tag::warning().xsmall().child("LFS pointer"))
                    })
                    .when(self.diff_files > 1, |this| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(muted)
                                .child(format!("{} files", self.diff_files)),
                        )
                    })
                    .child(div().flex_1())
                    .when(
                        matches!(&self.viewer, Some(Viewer::Markdown { .. }))
                            && viewing
                            && !editing,
                        |this| {
                            this.child(
                                Button::new("md-rendered")
                                    .test_id("md-rendered")
                                    .ghost()
                                    .xsmall()
                                    .label("Rendered")
                                    .selected(!self.markdown_as_code)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if this.markdown_as_code {
                                            this.toggle_markdown_code(cx)
                                        }
                                    })),
                            )
                            .child(
                                Button::new("md-code")
                                    .test_id("md-code")
                                    .ghost()
                                    .xsmall()
                                    .label("Code")
                                    .selected(self.markdown_as_code)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if !this.markdown_as_code {
                                            this.toggle_markdown_code(cx)
                                        }
                                    })),
                            )
                        },
                    )
                    .when(blame.is_some(), |this| {
                        this.child(
                            Button::new("close-blame")
                                .test_id("close-blame")
                                .ghost()
                                .xsmall()
                                .icon(Lucide::X)
                                .tooltip("Close blame")
                                .on_click(cx.listener(|this, _, _, cx| this.close_blame(cx))),
                        )
                    })
                    .when(blame.is_none() && !editing, |this| {
                        let target = match &selection {
                            Selection::File { path, .. } => Some((path.clone(), None)),
                            Selection::WorkFile { path, rev } => Some((path.clone(), rev.clone())),
                            _ => None,
                        };
                        this.when_some(target, |this, (path, rev)| {
                            this.child(self.blame_history_buttons(path, rev, cx))
                        })
                    })
                    .when(
                        viewing && !editing && self.viewer_is_editable(&selection),
                        |this| {
                            this.child(
                                Button::new("view-edit")
                                    .test_id("view-edit")
                                    .ghost()
                                    .xsmall()
                                    .icon(Lucide::Pencil)
                                    .tooltip("Edit this file in place")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_editor(window, cx)
                                    })),
                            )
                        },
                    )
                    .when(editing, |this| {
                        let (save_label, save_tip) = self.save_button_text(cx);
                        this.child(
                            Button::new("editor-save")
                                .test_id("editor-save")
                                .primary()
                                .xsmall()
                                .label(save_label)
                                .tooltip(save_tip)
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.save_editor(window, cx)),
                                ),
                        )
                        .child(
                            Button::new("editor-cancel")
                                .test_id("editor-cancel")
                                .ghost()
                                .xsmall()
                                .label("Cancel")
                                .on_click(cx.listener(|this, _, _, cx| this.close_editor(cx))),
                        )
                    })
                    .when(!editing && !three_way && conflicted, |this| {
                        let ready = self.can_apply_resolution(cx);
                        this.child(
                            Button::new("apply-resolution")
                                .test_id("apply-resolution")
                                .primary()
                                .xsmall()
                                .label("Apply resolution")
                                .disabled(!ready)
                                .tooltip(if ready {
                                    "Write the chosen sides to the file and mark it resolved"
                                } else {
                                    "Pick Accept Current / Incoming / Both on every conflict first"
                                })
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.apply_conflict_resolution(window, cx)
                                })),
                        )
                        .child(
                            Button::new("edit-result")
                                .test_id("edit-result")
                                .outline()
                                .xsmall()
                                .label("Edit result")
                                .tooltip("Edit the merged file by hand")
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.open_editor(window, cx)),
                                ),
                        )
                    })
                    .when(
                        !editing && matches!(&area, Some((_, a)) if *a != Area::Conflicted),
                        |this| {
                            this.child(
                                Button::new("edit-file")
                                    .test_id("edit-file")
                                    .ghost()
                                    .xsmall()
                                    .icon(Lucide::Pencil)
                                    .tooltip("Edit this file in place")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_editor(window, cx)
                                    })),
                            )
                        },
                    )
                    .children(line_actions.iter().map(|action| {
                        let action = *action;
                        let pressed = line_range.clone();
                        Button::new(SharedString::from(format!("lines-{}", action.label())))
                            .test_id(|| format!("lines-{}", action.label().to_lowercase()))
                            .outline()
                            .xsmall()
                            .label(format!("{} lines", action.label()))
                            // The press itself clears the diff's selection, so hold on to
                            // the lines until the click (or a release somewhere else).
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    this.line_pick = pressed.clone();
                                    cx.notify();
                                }),
                            )
                            // Released somewhere else: let go, but only once the release
                            // has reached every button, since the one next to this may be
                            // the one that was clicked.
                            .on_mouse_up_out(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    if this.line_pick.is_some() {
                                        let view = cx.entity();
                                        cx.defer(move |cx| {
                                            view.update(cx, |this, cx| {
                                                if this.line_pick.take().is_some() {
                                                    cx.notify();
                                                }
                                            })
                                        });
                                    }
                                }),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.lines_action(action, window, cx)
                            }))
                    }))
                    .when(!viewing && !editing, |this| {
                        this.child(
                            Button::new("unified")
                                .test_id("unified")
                                .ghost()
                                .xsmall()
                                .label("Unified")
                                .selected(if conflicted {
                                    !three_way_mode
                                } else {
                                    mode == DiffMode::Unified
                                })
                                .tooltip(if conflicted {
                                    "Conflict blocks inline, with Accept buttons"
                                } else {
                                    "Changes in one column"
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if conflicted {
                                        this.set_conflict_view(
                                            osier_config::settings::ConflictView::Inline,
                                            cx,
                                        )
                                    } else {
                                        this.set_diff_mode(DiffMode::Unified, cx)
                                    }
                                })),
                        )
                        .child(
                            Button::new("split")
                                .test_id("split")
                                .ghost()
                                .xsmall()
                                .label("Split")
                                .selected(!conflicted && mode == DiffMode::Split)
                                // Conflict blocks only come in one column.
                                .disabled(conflicted)
                                .tooltip(if conflicted {
                                    "Not for conflicts: 3-way puts the two sides next to each other"
                                } else {
                                    "Before and after side by side"
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.set_diff_mode(DiffMode::Split, cx)
                                })),
                        )
                        .child(
                            Button::new("three-way")
                                .test_id("three-way")
                                .ghost()
                                .xsmall()
                                .label("3-way")
                                .selected(conflicted && three_way_mode)
                                .disabled(!conflicted)
                                .tooltip(if conflicted {
                                    "Merge tool: yours, the result and theirs side by side"
                                } else {
                                    "Merge tool, for files with conflicts"
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.set_conflict_view(
                                        osier_config::settings::ConflictView::ThreeWay,
                                        cx,
                                    )
                                })),
                        )
                        .child(
                            Button::new("wrap")
                                .test_id("wrap")
                                .ghost()
                                .xsmall()
                                .icon(Lucide::TextWrap)
                                .selected(self.settings.diff.soft_wrap)
                                .tooltip("Wrap long lines")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.toggle_soft_wrap(window, cx)
                                })),
                        )
                        .child(Separator::vertical().h_4())
                        .child(
                            Button::new("revert")
                                .test_id("revert")
                                .ghost()
                                .xsmall()
                                .icon(Lucide::Undo2)
                                .disabled(true)
                                .tooltip("Revert file"),
                        )
                    })
                    .child(
                        Button::new("diff-expand")
                            .test_id("diff-expand")
                            .ghost()
                            .xsmall()
                            .icon(if self.diff_expanded {
                                Lucide::Minimize2
                            } else {
                                Lucide::Maximize2
                            })
                            .label(if self.diff_expanded {
                                "Collapse"
                            } else {
                                "Expand"
                            })
                            .selected(self.diff_expanded)
                            .tooltip(if self.diff_expanded {
                                "Back to the normal layout (Esc)"
                            } else {
                                "Hide the sidebar and file list"
                            })
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_diff_expanded(cx))),
                    ),
            )
            .children(toolbar)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .overflow_hidden()
                    .child(body),
            )
    }

    /// Which file of the list the expanded diff pane is on, and how many there are: the
    /// Changes list for a changed file, the diff's own files for a commit or comparison.
    fn diff_file_position(&self, selection: &Selection) -> Option<(usize, usize)> {
        match selection {
            Selection::File { path, area } => {
                let snapshot = self.active_project()?.snapshot.as_ref()?;
                let items = crate::project::ordered_changes(&snapshot.status);
                let ix = items.iter().position(|(p, a)| p == path && a == area)?;
                Some((ix + 1, items.len()))
            }
            _ if self.diff_files > 1 => Some((self.diff_file_ix + 1, self.diff_files)),
            _ => None,
        }
    }

    /// The expanded diff pane's file arrows.
    pub(crate) fn step_diff_file(&mut self, delta: isize, cx: &mut Context<Self>) {
        let selection = match self.active_project() {
            Some(p) => p.selection.clone(),
            None => return,
        };
        if let Selection::File { path, area } = &selection {
            let Some(items) = self
                .active_project()
                .and_then(|p| p.snapshot.as_ref())
                .map(|s| crate::project::ordered_changes(&s.status))
            else {
                return;
            };
            if items.is_empty() {
                return;
            }
            let ix = items
                .iter()
                .position(|(p, a)| p == path && a == area)
                .unwrap_or(0);
            let next = (ix as isize + delta).rem_euclid(items.len() as isize) as usize;
            let (path, area) = items[next].clone();
            self.select_file(path, area, cx);
            return;
        }
        let paths: Vec<String> = self
            .diff
            .read(cx)
            .files()
            .iter()
            .map(|f| f.path().to_string())
            .collect();
        if paths.is_empty() {
            return;
        }
        let next = (self.diff_file_ix as isize + delta).rem_euclid(paths.len() as isize) as usize;
        self.diff_file_ix = next;
        let path = paths[next].clone();
        self.diff.update(cx, |s, cx| s.scroll_to_file(&path, cx));
        cx.notify();
    }

    /// The bar under the header for unified and split: change and file navigation, a count
    /// and the shared view settings, laid out like the merge tool's.
    fn render_diff_toolbar(&self, selection: &Selection, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let split = self.diff.read(cx).mode() == DiffMode::Split;
        let options = self.diff_options();
        let whitespace_ok = crate::app::whitespace_applies(selection);
        let hunks: usize = self.diff_model.iter().map(|f| f.hunks.len()).sum();
        let position = if self.diff_expanded {
            self.diff_file_position(selection)
        } else {
            None
        };
        let weak = cx.entity().downgrade();
        h_flex()
            .px_2()
            .py_1()
            .gap_1()
            .items_center()
            .flex_wrap()
            .border_b_1()
            .border_color(theme.border)
            .child(
                Button::new("diff-prev")
                    .test_id("diff-prev")
                    .ghost()
                    .xsmall()
                    .icon(Lucide::ArrowUp)
                    .tooltip("Previous change (shift-F7)")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.diff.update(cx, |s, cx| s.previous_change(cx))
                    })),
            )
            .child(
                Button::new("diff-next")
                    .test_id("diff-next")
                    .ghost()
                    .xsmall()
                    .icon(Lucide::ArrowDown)
                    .tooltip("Next change (F7)")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.diff.update(cx, |s, cx| s.next_change(cx))
                    })),
            )
            .child(div().flex_1())
            .when_some(position, |this, (n, total)| {
                this.child(
                    Button::new("diff-prev-file")
                        .test_id("diff-prev-file")
                        .ghost()
                        .xsmall()
                        .icon(Lucide::ChevronLeft)
                        .disabled(total < 2)
                        .tooltip("Previous file")
                        .on_click(cx.listener(|this, _, _, cx| this.step_diff_file(-1, cx))),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(format!("File {n} of {total}")),
                )
                .child(
                    Button::new("diff-next-file")
                        .test_id("diff-next-file")
                        .ghost()
                        .xsmall()
                        .icon(Lucide::ChevronRight)
                        .disabled(total < 2)
                        .tooltip("Next file")
                        .on_click(cx.listener(|this, _, _, cx| this.step_diff_file(1, cx))),
                )
                .child(Separator::vertical().h_4())
            })
            .when(!self.diff_model.is_empty(), |this| {
                this.child(
                    div()
                        .test_id("diff-counter")
                        .text_xs()
                        .child(crate::format::count(hunks, "change")),
                )
            })
            .child(
                Button::new("diff-settings")
                    .test_id("diff-settings")
                    .accessibility_label("Diff settings")
                    .ghost()
                    .xsmall()
                    .icon(Lucide::Settings)
                    .dropdown_menu(move |mut menu, _, _| {
                        use osier_git::merge3::Whitespace;
                        menu = menu.item(
                            PopupMenuItem::new("Synchronize scrolling")
                                .checked(split)
                                .disabled(true),
                        );
                        menu = menu.separator().label("Ignore differences");
                        for (label, value) in [
                            ("None", Whitespace::Exact),
                            ("Trim whitespace", Whitespace::Trim),
                            ("Ignore whitespace", Whitespace::Ignore),
                        ] {
                            let w = weak.clone();
                            let shown = if whitespace_ok {
                                options.whitespace
                            } else {
                                Whitespace::Exact
                            };
                            menu = menu.item(
                                PopupMenuItem::new(label)
                                    .checked(shown == value)
                                    .disabled(!whitespace_ok)
                                    .on_click(move |_, window, cx| {
                                        let _ = w.update(cx, |this, cx| {
                                            let mut o = this.diff_options();
                                            o.whitespace = value;
                                            this.set_diff_options(o, window, cx)
                                        });
                                    }),
                            );
                        }
                        let w = weak.clone();
                        menu = menu.separator().item(
                            PopupMenuItem::new("Highlight changed words")
                                .checked(options.highlight_words)
                                .on_click(move |_, window, cx| {
                                    let _ = w.update(cx, |this, cx| {
                                        let mut o = this.diff_options();
                                        o.highlight_words = !o.highlight_words;
                                        this.set_diff_options(o, window, cx)
                                    });
                                }),
                        );
                        let w = weak.clone();
                        menu.item(
                            PopupMenuItem::new("Wrap long lines")
                                .checked(options.soft_wrap)
                                .on_click(move |_, window, cx| {
                                    let _ =
                                        w.update(cx, |this, cx| this.toggle_soft_wrap(window, cx));
                                }),
                        )
                    }),
            )
            .into_any_element()
    }

    /// Whether the Files tab viewer shows something the editor can open: text in the working
    /// tree, not a file at a revision, an image or a binary.
    fn viewer_is_editable(&self, selection: &Selection) -> bool {
        matches!(selection, Selection::WorkFile { rev: None, .. })
            && matches!(
                &self.viewer,
                Some(Viewer::Code { .. } | Viewer::Markdown { .. })
            )
    }

    fn render_viewer(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let repo_root = self
            .active_project()
            .map(|p| p.repo.workdir().to_path_buf());
        let editor = self.viewer_editor(window, cx);
        match &self.viewer {
            None => self.empty_state("Loading…", "", cx).into_any_element(),
            Some(Viewer::Missing) => self
                .empty_state("File not found", "It may have been deleted or renamed.", cx)
                .into_any_element(),
            Some(Viewer::Error(e)) => self
                .empty_state("Cannot show file", e, cx)
                .into_any_element(),
            Some(Viewer::Lfs { oid, bytes }) => self
                .empty_state(
                    "Stored in Git LFS",
                    &format!(
                        "This file is a pointer to {:.1} KB of LFS content (oid {}…). Run git lfs pull to fetch it.",
                        *bytes as f64 / 1024.0,
                        &oid[..oid.len().min(12)]
                    ),
                    cx,
                )
                .into_any_element(),
            Some(Viewer::Binary { bytes }) => self
                .empty_state(
                    "Binary or very large file",
                    &format!(
                        "{:.1} KB. Open it in an external program instead.",
                        *bytes as f64 / 1024.0
                    ),
                    cx,
                )
                .into_any_element(),
            Some(Viewer::Image { full_path, bytes }) => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .p_4()
                .child(
                    img(full_path.clone())
                        .max_w_full()
                        .max_h_full()
                        .object_fit(ObjectFit::Contain),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(format!("{:.1} KB", *bytes as f64 / 1024.0)),
                )
                .into_any_element(),
            Some(Viewer::Code { .. }) => match &editor {
                Some(state) => crate::editor_hover::hover_lines(
                    Editor::new(state).readonly(true).bordered(false).size_full(),
                    state,
                    cx,
                ),
                None => div().into_any_element(),
            },
            Some(Viewer::Markdown { text, .. }) => {
                if self.markdown_as_code {
                    match &editor {
                        Some(state) => crate::editor_hover::hover_lines(
                            Editor::new(state).readonly(true).bordered(false).size_full(),
                            state,
                            cx,
                        ),
                        None => div().into_any_element(),
                    }
                } else {
                    let root = repo_root.clone();
                    let remote_ok = self.settings.viewer.remote_images;
                    div()
                        .id("markdown-view")
                        .size_full()
                        .p_4()
                        .overflow_y_scroll()
                        .child(
                            TextView::markdown("markdown-body", text.clone())
                                .selectable(true)
                                .image_source(move |uri| {
                                    let s = uri.to_string();
                                    if s.starts_with("http://") || s.starts_with("https://") {
                                        if remote_ok {
                                            ImageSource::from(uri.clone())
                                        } else {
                                            // Setting off: a path that does not exist renders as a
                                            // missing image instead of fetching anything.
                                            ImageSource::from(std::path::PathBuf::from(
                                                "/nonexistent/osier-remote-images-off",
                                            ))
                                        }
                                    } else if s.starts_with("data:") {
                                        ImageSource::from(uri.clone())
                                    } else {
                                        let p = root
                                            .as_ref()
                                            .map(|r| r.join(&s))
                                            .unwrap_or_else(|| std::path::PathBuf::from(&s));
                                        ImageSource::from(p)
                                    }
                                }),
                        )
                        .into_any_element()
                }
            }
        }
    }
}

impl Osier {
    /// A changed file's diff (HEAD against the working tree): per hunk a checkbox (ticked,
    /// empty or a dash, from `osier_git::staging`) on its first added line and a roll-back
    /// arrow on its first removed line, and every staged line in a stronger shade of its change
    /// colour.
    fn staging_gutter(
        &self,
        view: Diff,
        path: String,
        staging: std::rc::Rc<osier_git::staging::FileStaging>,
        cx: &mut Context<Self>,
    ) -> Diff {
        use osier_git::diff::LineKind;
        use osier_git::staging::Check;

        // Staged lines: their own change colour, stronger (like PhpStorm's included changes).
        let (added_tint, removed_tint) = (
            cx.theme().success.opacity(0.14),
            cx.theme().danger.opacity(0.14),
        );
        let line_range = |kind: LineKind, n: u32| {
            let side = if kind == LineKind::Removed {
                DiffSide::Original
            } else {
                DiffSide::Modified
            };
            DiffLineRange::new(path.clone(), side, n as usize, n as usize)
        };
        let mut decorations = Vec::new();
        let mut markers = Vec::new();
        for (hix, h) in staging.shown.hunks.iter().enumerate() {
            for (lix, l) in h.lines.iter().enumerate() {
                if staging.line_staged(hix, lix) == Some(true)
                    && let Some(n) = if l.kind == LineKind::Removed {
                        l.old_no
                    } else {
                        l.new_no
                    }
                {
                    let tint = if l.kind == LineKind::Removed {
                        removed_tint
                    } else {
                        added_tint
                    };
                    decorations
                        .push(DiffDecoration::new(line_range(l.kind, n)).with_background(tint));
                }
            }
            // Like PhpStorm: the roll-back arrow on the change's first removed (red) line,
            // the checkbox on its first added (green) line, so the arrow is always above the
            // checkbox. One control fits a row, so a change with only one kind of line uses
            // that kind's first two lines; a one-line change borrows the context line above
            // (additions) or below (removals), or the other side when the file ends there.
            let at = |l: &osier_git::diff::Line| match l.kind {
                LineKind::Removed => l
                    .old_no
                    .map(|n| DiffLinePosition::new(path.clone(), DiffSide::Original, n as usize)),
                _ => l
                    .new_no
                    .map(|n| DiffLinePosition::new(path.clone(), DiffSide::Modified, n as usize)),
            };
            let of_kind = |kind: LineKind| h.lines.iter().filter(move |l| l.kind == kind);
            let changed = |l: &&osier_git::diff::Line| l.kind != LineKind::Context;
            let above = h
                .lines
                .iter()
                .position(|l| changed(&l))
                .and_then(|ix| ix.checked_sub(1))
                .and_then(|ix| at(&h.lines[ix]));
            let below = h
                .lines
                .iter()
                .rposition(|l| changed(&l))
                .and_then(|ix| h.lines.get(ix + 1))
                .and_then(at);
            let added: Vec<_> = of_kind(LineKind::Added).filter_map(&at).take(2).collect();
            let removed: Vec<_> = of_kind(LineKind::Removed).filter_map(&at).take(2).collect();
            // (arrow, checkbox), top to bottom.
            let one_kind = |lines: &[DiffLinePosition], borrowed_above: bool| match lines {
                [first, second, ..] => (Some(first.clone()), Some(second.clone())),
                [only] => match (borrowed_above, &above, &below) {
                    (true, Some(a), _) => (Some(a.clone()), Some(only.clone())),
                    (false, _, Some(b)) => (Some(only.clone()), Some(b.clone())),
                    (_, _, Some(b)) => (Some(only.clone()), Some(b.clone())),
                    (_, Some(a), _) => (Some(a.clone()), Some(only.clone())),
                    // A one-line file: the checkbox alone; the file's own roll back remains.
                    _ => (None, Some(only.clone())),
                },
                [] => (None, None),
            };
            let (revert_at, check_at) = match (added.first(), removed.first()) {
                (Some(a), Some(r)) => (Some(r.clone()), Some(a.clone())),
                (Some(_), None) => one_kind(&added, true),
                (None, Some(_)) => one_kind(&removed, false),
                (None, None) => (None, None),
            };
            let check = match staging.hunk_check(hix) {
                Check::Staged => "staged",
                Check::Unstaged => "unstaged",
                Check::Partial => "partial",
            };
            if let Some(position) = check_at {
                markers
                    .push(DiffGutterMarker::new(("hunk-check", hix), position).with_label(check));
            }
            if let Some(position) = revert_at {
                markers.push(
                    DiffGutterMarker::new(("hunk-revert", hix), position).with_label("revert"),
                );
            }
        }
        let weak = cx.entity().downgrade();
        view.decorations(decorations)
            .gutter_markers(markers)
            .render_gutter_marker(move |marker, _, _| {
                let ElementId::NamedInteger(kind, hix) = marker.id().clone() else {
                    return div().into_any_element();
                };
                let hix = hix as usize;
                let act = {
                    let (weak, path) = (weak.clone(), path.clone());
                    move |action: HunkAction| {
                        let (weak, path) = (weak.clone(), path.clone());
                        move |window: &mut Window, cx: &mut App| {
                            let _ = weak.update(cx, |this, cx| {
                                this.hunk_action(path.clone(), hix, action, window, cx)
                            });
                        }
                    }
                };
                if kind.as_ref() == "hunk-revert" {
                    let revert = act(HunkAction::Discard);
                    return Button::new(marker.id().clone())
                        .test_id(|| format!("hunk-revert:{hix}"))
                        .ghost()
                        .xsmall()
                        .icon(Lucide::Undo2)
                        .tooltip("Roll back this change")
                        .on_click(move |_, window, cx| revert(window, cx))
                        .into_any_element();
                }
                let label = marker.label().clone();
                let staged = label.as_ref() == "staged";
                let toggle = act(if staged {
                    HunkAction::Unstage
                } else {
                    HunkAction::Stage
                });
                let (stage, unstage, revert) = (
                    act(HunkAction::Stage),
                    act(HunkAction::Unstage),
                    act(HunkAction::Discard),
                );
                div()
                    .id(marker.id().clone())
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        crate::checkbox::Checkbox::new(("hunk-checkbox", hix))
                            .test_id(|| format!("hunk-check:{hix}"))
                            .accessibility_label(if staged {
                                "Unstage this change"
                            } else {
                                "Stage this change"
                            })
                            .small()
                            .checked(staged)
                            .indeterminate(label.as_ref() == "partial")
                            .on_click(move |_, window, cx| {
                                cx.stop_propagation();
                                toggle(window, cx)
                            }),
                    )
                    .context_menu(move |menu, _, _| {
                        let (stage, unstage, revert) =
                            (stage.clone(), unstage.clone(), revert.clone());
                        menu.item(
                            PopupMenuItem::new("Stage this change")
                                .disabled(staged)
                                .on_click(move |_, window, cx| stage(window, cx)),
                        )
                        .item(
                            PopupMenuItem::new("Unstage this change")
                                .disabled(label.as_ref() == "unstaged")
                                .on_click(move |_, window, cx| unstage(window, cx)),
                        )
                        .separator()
                        .item(
                            PopupMenuItem::new("Roll back this change…")
                                .on_click(move |_, window, cx| revert(window, cx)),
                        )
                    })
                    .into_any_element()
            })
    }
}
