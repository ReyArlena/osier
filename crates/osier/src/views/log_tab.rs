use std::sync::Arc;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::list::ListItem;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::*;
use osier_git::graph::Edge;

use crate::app::Osier;
use crate::format::relative_time;
use crate::graph::{graph_cell, lane_colors};

impl Osier {
    pub(crate) fn render_log(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
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
        let commits = Arc::new(snapshot.commits.clone());
        let graph = Arc::new(snapshot.graph.clone());
        let selected = project.selected_commit;
        let colors = lane_colors(cx);
        let max_lanes = graph.iter().map(|r| r.width).max().unwrap_or(1).min(12);
        let owner = cx.entity().downgrade();
        let mono = cx.theme().mono_font_family.clone();
        let muted = cx.theme().muted_foreground;
        let count = commits.len();
        let page = project.log_page_size;

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
                    .child(div().text_xs().text_color(muted).child(if count >= page {
                        format!("latest {count} commits, all branches")
                    } else {
                        format!("{count} commits, all branches")
                    }))
                    .child(div().flex_1())
                    .child(
                        Button::new("refresh-log")
                            .ghost()
                            .xsmall()
                            .icon(Lucide::RefreshCw)
                            .tooltip("Refresh")
                            .on_click(cx.listener(|this, _, _, cx| this.refresh_active(cx))),
                    ),
            )
            .child(
                div().id("log-list").flex_1().min_h_0().child(
                    uniform_list("commits", count, move |range, _, _cx| {
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
                                ListItem::new(("commit", ix))
                                    .selected(selected == Some(ix))
                                    .h(px(crate::graph::ROW_HEIGHT))
                                    .px_2()
                                    .w_full()
                                    .on_click(move |_, _, cx| {
                                        let _ =
                                            owner.update(cx, |this, cx| this.select_commit(ix, cx));
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
                                            .child(
                                                h_flex()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .gap_2()
                                                    .items_center()
                                                    .children(refs.into_iter().take(3).map(|r| {
                                                        let is_head = r.starts_with("HEAD");
                                                        let is_tag = r.starts_with("tag: ");
                                                        let label = r
                                                            .trim_start_matches("HEAD -> ")
                                                            .trim_start_matches("tag: ")
                                                            .to_string();
                                                        let text = div()
                                                            .max_w(px(150.))
                                                            .truncate()
                                                            .child(label);
                                                        div().flex_none().child(if is_head {
                                                            Tag::primary().xsmall().child(text)
                                                        } else if is_tag {
                                                            Tag::warning().xsmall().child(text)
                                                        } else {
                                                            Tag::secondary().xsmall().child(text)
                                                        })
                                                    }))
                                                    .child(
                                                        div()
                                                            .text_sm()
                                                            .truncate()
                                                            .min_w_0()
                                                            .child(subject),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(muted)
                                                    .flex_none()
                                                    .child(commit.author_name.clone()),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(muted)
                                                    .flex_none()
                                                    .w(px(78.))
                                                    .text_right()
                                                    .child(relative_time(commit.committed_at)),
                                            )
                                            .child(
                                                div()
                                                    .font_family(mono.clone())
                                                    .text_xs()
                                                    .text_color(muted)
                                                    .flex_none()
                                                    .child(commit.short_hash().to_string()),
                                            ),
                                    )
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
}
