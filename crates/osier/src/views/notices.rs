//! The Notifications tab of the bottom panel: every toast of this session, newest first.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::a11y::A11yRow as _;
use crate::app::Osier;
use crate::notices::{NoticeKind, NoticeLog};
use crate::test_ids::TestId as _;

impl Osier {
    pub(crate) fn render_notices(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let mono = theme.mono_font_family.clone();
        let colors = [
            (NoticeKind::Info, theme.info),
            (NoticeKind::Success, theme.success),
            (NoticeKind::Warning, theme.warning),
            (NoticeKind::Error, theme.danger),
        ];
        let notices: Vec<_> = NoticeLog::notices(cx).iter().rev().cloned().collect();
        let mut list = v_flex()
            .flex_1()
            .min_h_0()
            .id("notices-list")
            .overflow_y_scrollbar()
            .text_xs();
        for notice in notices {
            let id = notice.id;
            let color = colors
                .iter()
                .find(|(k, _)| *k == notice.kind)
                .map_or(muted, |(_, c)| *c);
            let command = notice.command;
            list = list.child(
                h_flex()
                    .id(("notice", id as usize))
                    .test_id(|| format!("notice-row:{id}"))
                    .a11y_item(|| notice.message.to_string())
                    .px_3()
                    .py_1p5()
                    .gap_2()
                    .items_start()
                    .hover(|this| this.bg(cx.theme().list_hover))
                    .child(
                        div()
                            .flex_none()
                            .pt_0p5()
                            .text_color(color)
                            .child(Icon::new(notice.kind.icon()).xsmall()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .font_family(mono.clone())
                            .text_color(muted)
                            .child(crate::format::clock_time(notice.at)),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .when_some(notice.title.clone(), |this, title| {
                                this.child(div().font_medium().child(title))
                            })
                            .child(div().whitespace_normal().child(notice.message.clone())),
                    )
                    .when(notice.command_log, |this| {
                        this.child(
                            Button::new(("notice-command-log", id as usize))
                                .test_id(|| format!("notice-command-log:{id}"))
                                .ghost()
                                .xsmall()
                                .label("View command log")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.show_failed_command(command, cx)
                                })),
                        )
                    }),
            );
        }
        if NoticeLog::notices(cx).is_empty() {
            list = list.child(
                div()
                    .p_3()
                    .text_color(muted)
                    .child("No notifications yet. Toasts the app shows are kept here."),
            );
        }
        list
    }
}
