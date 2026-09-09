use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::diff::{Diff, DiffGutterMarker, DiffLinePosition, DiffMode, DiffSide};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
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
use crate::hunk_ops::HunkAction;
use crate::project::{Area, Selection};

impl Osier {
    pub(crate) fn render_diff_pane(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
                    Area::Staged => Tag::success().xsmall().child("staged"),
                    Area::Unstaged => Tag::warning().xsmall().child("unstaged"),
                    Area::Untracked => Tag::secondary().xsmall().child("untracked"),
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

        let area = self.diff_area();
        let has_selection = self.diff.read(cx).selected_lines().is_some();
        let line_actions: &[HunkAction] = match &area {
            Some((_, a)) if has_selection => HunkAction::for_area(*a),
            _ => &[],
        };

        let body: AnyElement = if self.diff_loading {
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
                    Button::new("load-anyway")
                        .outline()
                        .small()
                        .label("Load anyway")
                        .on_click(cx.listener(|this, _, _, cx| this.force_large_diff(cx))),
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
            self.empty_state(
                "Nothing selected",
                "Pick a file in Changes or a commit in Log to see its diff here.",
                cx,
            )
            .into_any_element()
        } else if self.diff_files == 0 {
            self.empty_state("No changes", "This selection has no textual changes.", cx)
                .into_any_element()
        } else {
            let mut view = Diff::new(&self.diff)
                .line_number(self.settings.diff.line_numbers)
                .soft_wrap(self.settings.diff.soft_wrap);
            if let Some((path, a)) = &area
                && !HunkAction::for_area(*a).is_empty()
                && let Some(file) = self.diff_file_model(path)
            {
                // One marker per hunk, on its first line, opening a small menu.
                let markers: Vec<DiffGutterMarker> = file
                    .hunks
                    .iter()
                    .enumerate()
                    .map(|(ix, h)| {
                        let position = if h.new_lines > 0 {
                            DiffLinePosition::new(
                                path.clone(),
                                DiffSide::Modified,
                                h.new_start as usize,
                            )
                        } else {
                            DiffLinePosition::new(
                                path.clone(),
                                DiffSide::Original,
                                h.old_start as usize,
                            )
                        };
                        DiffGutterMarker::new(("hunk", ix), position).with_label(match a {
                            Area::Staged => "−",
                            _ => "+",
                        })
                    })
                    .collect();
                let model = self.diff_model.clone();
                let weak = cx.entity().downgrade();
                let actions = HunkAction::for_area(*a);
                let path = path.clone();
                view = view
                    .gutter_markers(markers)
                    .render_gutter_marker(move |marker, _, _| {
                        let line = marker.position().line() as u32;
                        let side = marker.position().side();
                        let hunk_ix = model
                            .iter()
                            .find(|f| f.path == *marker.position().path())
                            .and_then(|f| {
                                f.hunks.iter().position(|h| {
                                    (h.new_lines > 0
                                        && side == DiffSide::Modified
                                        && h.new_start == line)
                                        || (h.new_lines == 0
                                            && side == DiffSide::Original
                                            && h.old_start == line)
                                })
                            });
                        let button = Button::new(marker.id().clone())
                            .ghost()
                            .xsmall()
                            .label(marker.label().clone())
                            .tooltip("Hunk actions");
                        let Some(hunk_ix) = hunk_ix else {
                            return button.into_any_element();
                        };
                        let weak = weak.clone();
                        let path = path.clone();
                        button
                            .dropdown_menu(move |mut menu, _, _| {
                                for action in actions {
                                    let weak = weak.clone();
                                    let path = path.clone();
                                    let action = *action;
                                    menu = menu.item(
                                        PopupMenuItem::new(format!("{} hunk", action.label()))
                                            .on_click(move |_, window, cx| {
                                                let _ = weak.update(cx, |this, cx| {
                                                    this.hunk_action(
                                                        path.clone(),
                                                        hunk_ix,
                                                        action,
                                                        window,
                                                        cx,
                                                    )
                                                });
                                            }),
                                    );
                                }
                                menu
                            })
                            .into_any_element()
                    });
            }
            view.into_any_element()
        };

        v_flex()
            .size_full()
            .min_w_0()
            .overflow_hidden()
            .child(
                h_flex()
                    .px_3()
                    .py_2()
                    .gap_2()
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
                    .when(self.diff_files > 1, |this| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(muted)
                                .child(format!("{} files", self.diff_files)),
                        )
                    })
                    .child(div().flex_1())
                    .children(line_actions.iter().map(|action| {
                        let action = *action;
                        Button::new(SharedString::from(format!("lines-{}", action.label())))
                            .outline()
                            .xsmall()
                            .label(format!("{} lines", action.label()))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.lines_action(action, window, cx)
                            }))
                    }))
                    .child(
                        Button::new("unified")
                            .ghost()
                            .xsmall()
                            .label("Unified")
                            .selected(mode == DiffMode::Unified)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_diff_mode(DiffMode::Unified, cx)
                            })),
                    )
                    .child(
                        Button::new("split")
                            .ghost()
                            .xsmall()
                            .label("Split")
                            .selected(mode == DiffMode::Split)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_diff_mode(DiffMode::Split, cx)
                            })),
                    )
                    .child(
                        Button::new("wrap")
                            .ghost()
                            .xsmall()
                            .icon(Lucide::TextWrap)
                            .selected(self.settings.diff.soft_wrap)
                            .tooltip("Wrap long lines")
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_soft_wrap(cx))),
                    )
                    .child(Separator::vertical().h_4())
                    .child(
                        Button::new("revert")
                            .ghost()
                            .xsmall()
                            .icon(Lucide::Undo2)
                            .disabled(true)
                            .tooltip("Revert file"),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .overflow_hidden()
                    .child(body),
            )
    }
}
