use std::sync::Arc;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::list::ListItem;
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{
    ActiveTheme as _, Selectable as _, Sizable as _, StyledExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use osier_git::graph::Edge;
use osier_git::ops::reset::ResetMode;

use osier_git::ops::patches::PatchSource;

use crate::app::Osier;
use crate::test_ids::TestId as _;

/// Menu handler receiving the commit hash.
type CommitHandler = std::rc::Rc<dyn Fn(&mut Osier, String, &mut Window, &mut Context<Osier>)>;
use crate::graph::{graph_cell, lane_colors};

impl Osier {
    pub(crate) fn render_log(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let menu_buttons = self.settings.appearance.row_menu_buttons;
        let Some(project) = self.active_project() else {
            return self
                .empty_state(
                    "No repository open",
                    "Open a repository to see its history.",
                    cx,
                )
                .into_any_element();
        };
        let Some(snapshot) = &project.snapshot else {
            return self
                .empty_state(&project.name, "Reading history…", cx)
                .into_any_element();
        };
        if snapshot.commits.is_empty() {
            return self
                .empty_state(
                    "No commits yet",
                    "Make the first commit to start the history.",
                    cx,
                )
                .into_any_element();
        }
        if self.show_reflog {
            return self.render_reflog(cx).into_any_element();
        }
        let commits = Arc::new(snapshot.commits.clone());
        let graph = Arc::new(snapshot.graph.clone());
        let selected = project.selected_commit;
        let compare = project.compare_commit;
        // A shift-click range on one line of history: highlighted whole, squashable.
        let run: Arc<Vec<usize>> = Arc::new(project.squash_run().unwrap_or_default());
        // Colours behind a row, for the fade over clipped badges (see `fade.rs`).
        let row_bg = cx.theme().background;
        let row_hover = row_bg.blend(cx.theme().list_hover);
        let row_selected = row_bg.blend(cx.theme().list_active);
        let colors = lane_colors(cx);
        let max_lanes = graph.iter().map(|r| r.width).max().unwrap_or(1).min(12);
        let owner = cx.entity().downgrade();
        let on_github = self.active_github_url().is_some();
        let mono = cx.theme().mono_font_family.clone();
        let muted = cx.theme().muted_foreground;
        let count = commits.len();
        let page = project.log_page_size;
        let badges_right = self.settings.git.log_badges == osier_config::settings::LogBadges::Right;

        v_flex()
            .size_full()
            .child(
                h_flex()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(div().text_sm().font_medium().child("Log"))
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(match &project.log_path {
                                Some(path) => format!("{count} commits touching {path}"),
                                None if count >= page => {
                                    format!("latest {count} commits, all branches")
                                }
                                None => format!("{count} commits, all branches"),
                            }),
                    )
                    .when(project.log_path.is_some(), |this| {
                        this.child(
                            Button::new("clear-history")
                                .test_id("clear-history")
                                .ghost()
                                .xsmall()
                                .icon(Lucide::X)
                                .label("All commits")
                                .tooltip("Leave the file history")
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.clear_file_history(cx)),
                                ),
                        )
                    })
                    .child(div().flex_1())
                    .child(
                        Button::new("toggle-reflog")
                            .test_id("toggle-reflog")
                            .ghost()
                            .xsmall()
                            .label("Reflog")
                            .selected(self.show_reflog)
                            .tooltip("Show where HEAD has been (recovery)")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_reflog = !this.show_reflog;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("refresh-log")
                            .test_id("refresh-log")
                            .ghost()
                            .xsmall()
                            .icon(Lucide::RefreshCw)
                            .tooltip("Refresh")
                            .on_click(cx.listener(|this, _, _, cx| this.refresh_active(cx))),
                    ),
            )
            .child(
                div().id("log-list").flex_1().min_h_0().child(
                    uniform_list("commits", count, move |range, _, cx| {
                        range
                            .map(|ix| {
                                let commit = &commits[ix];
                                let row = graph[ix].clone();
                                let incoming: Vec<Edge> = if ix > 0 {
                                    graph[ix - 1].edges.clone()
                                } else {
                                    Vec::new()
                                };
                                let owner = owner.clone();
                                let subject = if commit.subject.is_empty() {
                                    "(no subject)".to_string()
                                } else {
                                    commit.subject.clone()
                                };
                                let refs: Vec<String> = commit.refs.clone();
                                let hash = commit.hash.clone();
                                let is_head = commit.refs.iter().any(|r| r.starts_with("HEAD"));
                                let menu_owner = owner.clone();
                                let in_run = run.contains(&ix);
                                let run_len = run.len();
                                let is_selected =
                                    selected == Some(ix) || compare == Some(ix) || in_run;
                                let (fade_base, fade_hover) = if is_selected {
                                    (row_selected, row_selected)
                                } else {
                                    (row_bg, row_hover)
                                };
                                let row_menu = crate::row_menu::RowMenu::new(move |menu, _, _| {
                                    let h = hash.clone();
                                    let o = menu_owner.clone();
                                    let item = |label: &str, f: CommitHandler| {
                                        let h = h.clone();
                                        let o = o.clone();
                                        match log_menu_kind(label) {
                                            Some(kind) => crate::row_actions::menu_item(
                                                label.to_string(),
                                                kind,
                                            ),
                                            None => PopupMenuItem::new(label.to_string()),
                                        }
                                        .on_click(
                                            move |_, window, cx| {
                                                let h = h.clone();
                                                let f = f.clone();
                                                let _ =
                                                    o.update(cx, |this, cx| f(this, h, window, cx));
                                            },
                                        )
                                    };
                                    let mut menu = menu
                                        .item(item(
                                            "Checkout (detached)",
                                            std::rc::Rc::new(|t, h, w, cx| {
                                                t.checkout_commit(h, w, cx)
                                            }),
                                        ))
                                        .item(item(
                                            "New branch here…",
                                            std::rc::Rc::new(|t, h, w, cx| {
                                                t.prompt_new_branch(Some(h), w, cx)
                                            }),
                                        ))
                                        .item(item(
                                            "Tag here…",
                                            std::rc::Rc::new(|t, h, w, cx| t.prompt_tag(h, w, cx)),
                                        ))
                                        .item(item(
                                            "Copy as patch",
                                            std::rc::Rc::new(|t, h, w, cx| {
                                                t.copy_patch(
                                                    PatchSource::Commits { hashes: vec![h] },
                                                    w,
                                                    cx,
                                                )
                                            }),
                                        ))
                                        .item(item(
                                            "Save to Patches…",
                                            std::rc::Rc::new(|t, h, w, cx| {
                                                let name = t
                                                    .commit_subject(&h)
                                                    .unwrap_or_else(|| h.clone());
                                                t.save_patch_prompt(
                                                    PatchSource::Commits { hashes: vec![h] },
                                                    name,
                                                    w,
                                                    cx,
                                                )
                                            }),
                                        ))
                                        .item(item(
                                            "Export patch file…",
                                            std::rc::Rc::new(|t, h, w, cx| {
                                                let name =
                                                    format!("{}.patch", &h[..h.len().min(7)]);
                                                t.export_patch(
                                                    PatchSource::Commits { hashes: vec![h] },
                                                    name,
                                                    w,
                                                    cx,
                                                )
                                            }),
                                        ))
                                        .item(item(
                                            "Compare with selected commit",
                                            std::rc::Rc::new(|t, h, _, cx| {
                                                t.compare_with_hash(h, cx)
                                            }),
                                        ))
                                        .separator()
                                        .item(item(
                                            "Reset current branch here (soft)",
                                            std::rc::Rc::new(|t, h, w, cx| {
                                                t.confirm_reset(h, ResetMode::Soft, w, cx)
                                            }),
                                        ))
                                        .item(item(
                                            "Reset current branch here (mixed)",
                                            std::rc::Rc::new(|t, h, w, cx| {
                                                t.confirm_reset(h, ResetMode::Mixed, w, cx)
                                            }),
                                        ))
                                        .item(item(
                                            "Reset current branch here (hard)",
                                            std::rc::Rc::new(|t, h, w, cx| {
                                                t.confirm_reset(h, ResetMode::Hard, w, cx)
                                            }),
                                        ))
                                        .item(item(
                                            "Revert commit…",
                                            std::rc::Rc::new(|t, h, w, cx| {
                                                t.confirm_revert(h, w, cx)
                                            }),
                                        ))
                                        .item(item(
                                            "Cherry-pick onto current branch…",
                                            std::rc::Rc::new(|t, h, w, cx| {
                                                t.confirm_cherry_pick(h, w, cx)
                                            }),
                                        ))
                                        .separator()
                                        .item(item(
                                            "Compare with working tree",
                                            std::rc::Rc::new(|t, h, _, cx| {
                                                t.select_compare(h, "HEAD".into(), false, cx)
                                            }),
                                        ))
                                        .item(item(
                                            "Interactive rebase from here…",
                                            std::rc::Rc::new(|t, h, w, cx| {
                                                t.open_interactive_rebase(h, w, cx)
                                            }),
                                        ));
                                    if in_run && run_len >= 2 {
                                        menu = menu.item(item(
                                            "Squash selected commits…",
                                            std::rc::Rc::new(|t, _, w, cx| t.confirm_squash(w, cx)),
                                        ));
                                    }
                                    if is_head {
                                        menu = menu.item(item(
                                            "Undo last commit…",
                                            std::rc::Rc::new(|t, _, w, cx| {
                                                t.confirm_undo_commit(w, cx)
                                            }),
                                        ));
                                    }
                                    menu.separator()
                                        .item(item(
                                            "Copy hash",
                                            std::rc::Rc::new(|t, h, w, cx| {
                                                t.copy_to_clipboard(h, w, cx)
                                            }),
                                        ))
                                        .when(on_github, |menu| {
                                            menu.item(item(
                                                "Open on GitHub",
                                                std::rc::Rc::new(|t, h, w, cx| {
                                                    t.open_commit_on_github(h, w, cx)
                                                }),
                                            ))
                                        })
                                });
                                ListItem::new(("commit", ix))
                                    .test_id(format!("log-row:{ix}"))
                                    .group("log-row")
                                    .selected(is_selected)
                                    .h(px(crate::graph::ROW_HEIGHT))
                                    .px_2()
                                    .w_full()
                                    .on_click(move |event, _, cx| {
                                        // Shift or ctrl-click: compare with the selected commit.
                                        let m = event.modifiers();
                                        let _ = owner.update(cx, |this, cx| {
                                            if m.shift || m.control {
                                                this.compare_with_selected(ix, cx)
                                            } else {
                                                this.select_commit(ix, cx)
                                            }
                                        });
                                    })
                                    .child(
                                        h_flex()
                                            .size_full()
                                            .gap_2()
                                            .items_center()
                                            .child(graph_cell(
                                                row,
                                                incoming,
                                                max_lanes,
                                                colors,
                                                commit.is_merge(),
                                            ))
                                            .child(subject_with_refs(
                                                ix,
                                                subject,
                                                refs,
                                                badges_right,
                                                (fade_base, fade_hover),
                                                cx,
                                            ))
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(muted)
                                                    .flex_none()
                                                    .max_w(px(110.))
                                                    .truncate()
                                                    .child(commit.author_name.clone()),
                                            )
                                            .child(
                                                super::relative_time_label(
                                                    ("commit-time", ix),
                                                    commit.committed_at,
                                                    cx,
                                                )
                                                .test_id(format!("commit-time:{ix}"))
                                                .text_xs()
                                                .text_color(muted)
                                                .flex_none()
                                                .w(px(78.))
                                                .text_right(),
                                            )
                                            .child(
                                                div()
                                                    .font_family(mono.clone())
                                                    .text_xs()
                                                    .text_color(muted)
                                                    .flex_none()
                                                    .child(commit.short_hash().to_string()),
                                            )
                                            .child(
                                                Button::new(("commit-menu", ix))
                                                    .when(!menu_buttons, |b| b.hidden())
                                                    .ghost()
                                                    .xsmall()
                                                    .icon(Lucide::Ellipsis)
                                                    .dropdown_menu(row_menu.build()),
                                            ),
                                    )
                                    .context_menu(row_menu.build())
                                    .into_any_element()
                            })
                            .collect()
                    })
                    .track_scroll(&self.log_scroll)
                    .size_full(),
                ),
            )
            .into_any_element()
    }

    fn render_reflog(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let menu_buttons = self.settings.appearance.row_menu_buttons;
        use gpui_kit::component::scroll::ScrollableElement as _;
        let Some(snapshot) = self.active_project().and_then(|p| p.snapshot.as_ref()) else {
            return div().into_any_element();
        };
        let muted = cx.theme().muted_foreground;
        let mono = cx.theme().mono_font_family.clone();
        let owner = cx.entity().downgrade();
        let mut list = v_flex()
            .flex_1()
            .min_h_0()
            .id("reflog-list")
            .overflow_y_scrollbar();
        for (ix, e) in snapshot.reflog.iter().enumerate() {
            let hash = e.hash.clone();
            let o = owner.clone();
            let row_menu = crate::row_menu::RowMenu::new(move |menu, _, _| {
                let (h1, h2, h3, h4) = (hash.clone(), hash.clone(), hash.clone(), hash.clone());
                let (o1, o2, o3, o4) = (o.clone(), o.clone(), o.clone(), o.clone());
                menu.item(PopupMenuItem::new("Check out (detached)").on_click(
                    move |_, window, cx| {
                        let _ =
                            o1.update(cx, |this, cx| this.checkout_commit(h1.clone(), window, cx));
                    },
                ))
                .item(
                    PopupMenuItem::new("New branch here…").on_click(move |_, window, cx| {
                        let _ = o2.update(cx, |this, cx| {
                            this.prompt_new_branch(Some(h2.clone()), window, cx)
                        });
                    }),
                )
                .item(
                    PopupMenuItem::new("Reset current branch here (hard)…").on_click(
                        move |_, window, cx| {
                            let _ = o3.update(cx, |this, cx| {
                                this.confirm_reset(h3.clone(), ResetMode::Hard, window, cx)
                            });
                        },
                    ),
                )
                .separator()
                .item(
                    PopupMenuItem::new("Copy hash").on_click(move |_, window, cx| {
                        let _ = o4.update(cx, |this, cx| {
                            this.copy_to_clipboard(h4.clone(), window, cx)
                        });
                    }),
                )
            });
            list = list.child(
                h_flex()
                    .id(("reflog", ix))
                    .px_3()
                    .py_1p5()
                    .gap_3()
                    .items_center()
                    .hover(|this| this.bg(cx.theme().list_hover))
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .w(px(80.))
                            .flex_none()
                            .child(e.selector.clone()),
                    )
                    .child(
                        div()
                            .text_sm()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(e.message.clone()),
                    )
                    .child(
                        super::relative_time_label(("reflog-time", ix), e.at, cx)
                            .text_xs()
                            .text_color(muted)
                            .flex_none(),
                    )
                    .child(
                        div()
                            .font_family(mono.clone())
                            .text_xs()
                            .text_color(muted)
                            .flex_none()
                            .child(e.hash[..7].to_string()),
                    )
                    .child(
                        Button::new(("reflog-menu", ix))
                            .when(!menu_buttons, |b| b.hidden())
                            .ghost()
                            .xsmall()
                            .icon(Lucide::Ellipsis)
                            .dropdown_menu(row_menu.build()),
                    )
                    .context_menu(row_menu.build()),
            );
        }
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(div().text_sm().font_medium().child("Reflog"))
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child("where HEAD has been; use it to recover"),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("close-reflog")
                            .test_id("close-reflog")
                            .ghost()
                            .xsmall()
                            .label("Back to log")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_reflog = false;
                                cx.notify();
                            })),
                    ),
            )
            .child(list)
            .into_any_element()
    }
}

/// Gap between a commit subject and its ref badges.
const REF_GAP: f32 = 8.0;
/// Fade over the end of a subject that runs into the badges, like sidebar rows, and over
/// badges that do not fit.
const SUBJECT_FADE: f32 = 14.0;

/// The badge for one ref: HEAD's branch solid, tags in the warning colour, other branches
/// tinted with the primary colour like the sidebar's selected row.
pub(crate) fn ref_tag(label: String, is_head: bool, is_tag: bool, cx: &App) -> Tag {
    let text = div().max_w(px(150.)).truncate().child(label);
    let tag = if is_head {
        Tag::primary()
    } else if is_tag {
        Tag::warning()
    } else {
        let primary = cx.theme().primary;
        Tag::custom(
            primary.opacity(0.18),
            cx.theme().foreground,
            primary.opacity(0.32),
        )
    };
    tag.xsmall().child(text)
}

/// A commit subject with up to three ref badges (`git.log_badges`). Without badges the
/// subject ends in an ellipsis.
///
/// Left: badges first, then the subject; the row's end fades out over badges that do not
/// fit. Right: the subject runs up to the badges at the end of the line and fades out
/// under their left edge, as sidebar names do before their ahead/behind badges. Badges that
/// do not fit run into the box's right padding and fade out there; badges that fit end
/// before it, so the fade covers nothing. `fade` is the row background (normal, hover).
fn subject_with_refs(
    ix: usize,
    subject: String,
    refs: Vec<String>,
    right: bool,
    fade: (Hsla, Hsla),
    cx: &App,
) -> AnyElement {
    let (base, hover) = fade;
    if refs.is_empty() {
        return div()
            .test_id(format!("log-subject:{ix}"))
            .flex_1()
            .min_w_0()
            .text_sm()
            .truncate()
            .child(subject)
            .into_any_element();
    }
    let badges = refs.into_iter().take(3).map(|r| {
        let is_head = r.starts_with("HEAD");
        let is_tag = r.starts_with("tag: ");
        let label = r
            .trim_start_matches("HEAD -> ")
            .trim_start_matches("tag: ")
            .to_string();
        div()
            .flex_none()
            .test_id(format!("log-ref:{ix}:{label}"))
            .child(ref_tag(label, is_head, is_tag, cx))
    });
    if !right {
        return h_flex()
            .relative()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .gap(px(REF_GAP))
            .items_center()
            .children(badges)
            .child(
                div()
                    .test_id(format!("log-subject:{ix}"))
                    .text_sm()
                    .truncate()
                    .min_w_0()
                    .child(subject),
            )
            .child(
                crate::fade::fade_right(base)
                    .group_hover("log-row", move |s| s.bg(crate::fade::fade_fill(hover))),
            )
            .into_any_element();
    }
    h_flex()
        .flex_1()
        .min_w_0()
        .gap(px(REF_GAP))
        .items_center()
        .child(
            // Runs under the gap so the fade, not the clip, ends the text.
            div()
                .test_id(format!("log-subject:{ix}"))
                .flex_1()
                .min_w_0()
                .mr(px(-REF_GAP))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_sm()
                .child(subject),
        )
        .child(
            h_flex()
                .relative()
                .flex_none()
                .max_w(relative(0.45))
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(px(-SUBJECT_FADE))
                        .w(px(SUBJECT_FADE))
                        .bg(crate::fade::fade_fill(base))
                        .group_hover("log-row", move |s| s.bg(crate::fade::fade_fill(hover))),
                )
                .child(
                    h_flex()
                        .relative()
                        .min_w_0()
                        .overflow_hidden()
                        .pr(px(SUBJECT_FADE))
                        .mr(px(-SUBJECT_FADE / 2.))
                        .gap_1()
                        .items_center()
                        .children(badges)
                        .child(
                            crate::fade::fade_right(base)
                                .w(px(SUBJECT_FADE))
                                .group_hover("log-row", move |s| {
                                    s.bg(crate::fade::fade_fill(hover))
                                }),
                        ),
                ),
        )
        .into_any_element()
}

/// The bindable action behind a Log commit menu entry, for its key hint.
fn log_menu_kind(label: &str) -> Option<crate::row_actions::RowKind> {
    use crate::row_actions::RowKind as K;
    Some(match label {
        "Checkout (detached)" => K::Checkout,
        "New branch here…" => K::NewBranchHere,
        "Tag here…" => K::TagHere,
        "Copy as patch" => K::CopyAsPatch,
        "Save to Patches…" => K::SaveToPatches,
        "Export patch file…" => K::ExportPatch,
        "Reset current branch here (soft)" => K::ResetSoft,
        "Reset current branch here (mixed)" => K::ResetMixed,
        "Reset current branch here (hard)" => K::ResetHard,
        "Revert commit…" => K::Revert,
        "Cherry-pick onto current branch…" => K::CherryPick,
        "Compare with working tree" => K::CompareWithWorkingTree,
        "Interactive rebase from here…" => K::InteractiveRebase,
        "Squash selected commits…" => K::Squash,
        "Undo last commit…" => K::UndoLastCommit,
        "Copy hash" => K::CopyHash,
        "Open on GitHub" => K::OpenOnGithub,
        _ => return None,
    })
}
