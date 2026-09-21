use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, StyledExt as _, h_flex, v_flex,
};
use gpui_kit::*;

use crate::app::Osier;
use crate::format::relative_time;
use crate::test_ids::TestId as _;

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
        let muted = cx.theme().muted_foreground;
        let has_changes = !snapshot.status.is_clean();

        let header = h_flex()
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
            )
            .child(div().flex_1())
            .child(
                Button::new("stash-push")
                    .test_id("stash-push")
                    .outline()
                    .xsmall()
                    .icon(Lucide::Archive)
                    .label("Stash changes")
                    .disabled(!has_changes)
                    .tooltip(if has_changes {
                        "Stash working tree changes"
                    } else {
                        "Nothing to stash"
                    })
                    .on_click(cx.listener(|this, _, window, cx| this.prompt_stash(window, cx))),
            );

        if snapshot.stashes.is_empty() {
            return v_flex()
                .size_full()
                .child(header)
                .child(self.empty_state("No stashes", "Stashed changes will show up here.", cx))
                .into_any_element();
        }
        let mut list = v_flex()
            .flex_1()
            .min_h_0()
            .id("stash-list")
            .overflow_y_scrollbar();
        for s in &snapshot.stashes {
            let index = s.index;
            let message = s.message.clone();
            list = list.child(
                h_flex()
                    .id(("stash", s.index))
                    .test_id(format!("stash-row:{}", s.index))
                    .group("row")
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
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .invisible()
                            .group_hover("row", |this| this.visible())
                            .child(
                                Button::new(("stash-apply", index))
                                    .ghost()
                                    .xsmall()
                                    .label("Apply")
                                    .tooltip("Apply and keep the stash")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.stash_apply(index, false, window, cx)
                                    })),
                            )
                            .child(
                                Button::new(("stash-pop", index))
                                    .ghost()
                                    .xsmall()
                                    .label("Pop")
                                    .tooltip("Apply and drop the stash")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.stash_apply(index, true, window, cx)
                                    })),
                            )
                            .child(
                                Button::new(("stash-drop", index))
                                    .ghost()
                                    .xsmall()
                                    .icon(Lucide::Trash)
                                    .tooltip("Drop")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.confirm_stash_drop(index, message.clone(), window, cx)
                                    })),
                            ),
                    ),
            );
        }
        v_flex()
            .size_full()
            .child(header)
            .child(list)
            .into_any_element()
    }
}
