use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::*;

use crate::app::Osier;
use crate::format::relative_time;

impl Osier {
    pub(crate) fn render_stashes(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(project) = self.active_project() else {
            return self
                .empty_state(
                    "No repository open",
                    "Open a repository to see its stashes.",
                    cx,
                )
                .into_any_element();
        };
        let Some(snapshot) = &project.snapshot else {
            return self
                .empty_state(&project.name, "Reading stashes…", cx)
                .into_any_element();
        };
        if snapshot.stashes.is_empty() {
            return self
                .empty_state("No stashes", "Stashed changes will show up here.", cx)
                .into_any_element();
        }
        let muted = cx.theme().muted_foreground;
        let mut list = v_flex()
            .flex_1()
            .min_h_0()
            .id("stash-list")
            .overflow_y_scrollbar();
        for s in &snapshot.stashes {
            list = list.child(
                h_flex()
                    .id(("stash", s.index))
                    .px_3()
                    .py_1p5()
                    .gap_2()
                    .items_center()
                    .hover(|this| this.bg(cx.theme().list_hover))
                    .child(Icon::new(Lucide::Archive).xsmall())
                    .child(div().text_xs().text_color(muted).child(s.selector.clone()))
                    .child(
                        div()
                            .text_sm()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(s.message.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(relative_time(s.created_at)),
                    ),
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
                    .child(div().text_sm().font_medium().child("Stashes"))
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(snapshot.stashes.len().to_string()),
                    ),
            )
            .child(list)
            .into_any_element()
    }
}
