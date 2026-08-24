use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app::Osier;

impl Osier {
    pub(crate) fn render_command_log(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let mono = cx.theme().mono_font_family.clone();
        let records = self.sink.records();
        let shown = records.len().min(200);
        let expanded = self.expanded_command;

        let mut list = v_flex()
            .flex_1()
            .min_h_0()
            .id("command-log-list")
            .overflow_y_scrollbar()
            .font_family(mono.clone())
            .text_xs();
        for rec in records.iter().rev().take(shown) {
            let ok = rec.status == Some(0);
            let id = rec.id;
            let is_open = expanded == Some(id);
            let shell = rec.as_shell();
            // Hide the boilerplate prefix in the one-line view.
            let short = shell
                .replace(" --no-pager -c color.ui=never -c core.quotePath=false", "")
                .replace("--no-pager -c color.ui=never -c core.quotePath=false ", "");
            let ms = rec.duration.as_millis();
            list = list.child(
                v_flex()
                    .id(("cmd", id as usize))
                    .cursor_pointer()
                    .hover(|this| this.bg(cx.theme().list_hover))
                    .when(is_open, |this| this.bg(cx.theme().list_active))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.expanded_command = if this.expanded_command == Some(id) {
                            None
                        } else {
                            Some(id)
                        };
                        cx.notify();
                    }))
                    .child(
                        h_flex()
                            .px_3()
                            .py_0p5()
                            .gap_2()
                            .items_center()
                            .child(if ok {
                                Tag::success().xsmall().child("0")
                            } else {
                                Tag::danger().xsmall().child(
                                    rec.status
                                        .map(|s| s.to_string())
                                        .unwrap_or_else(|| "killed".into()),
                                )
                            })
                            .child(div().flex_1().min_w_0().truncate().child(short))
                            .child(
                                div()
                                    .text_color(muted)
                                    .flex_none()
                                    .child(format!("{ms} ms")),
                            ),
                    )
                    .when(is_open, |this| {
                        this.child(
                            v_flex()
                                .px_3()
                                .pb_2()
                                .gap_1()
                                .child(div().text_color(muted).child(shell.clone()))
                                .when_some(rec.cwd.as_ref(), |this, cwd| {
                                    this.child(
                                        div()
                                            .text_color(muted)
                                            .child(format!("cwd: {}", cwd.display())),
                                    )
                                })
                                .child(
                                    div()
                                        .text_color(muted)
                                        .child(format!("stdout: {} bytes", rec.stdout_len)),
                                )
                                .when(!rec.stderr.trim().is_empty(), |this| {
                                    this.child(
                                        div()
                                            .p_2()
                                            .rounded(cx.theme().radius)
                                            .bg(cx.theme().muted.opacity(0.4))
                                            .whitespace_normal()
                                            .child(rec.stderr.trim().to_string()),
                                    )
                                }),
                        )
                    }),
            );
        }
        if records.is_empty() {
            list = list.child(
                div()
                    .p_3()
                    .text_color(muted)
                    .child("No git commands run yet."),
            );
        }

        v_flex()
            .h(px(220.))
            .flex_none()
            .border_t_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().sidebar)
            .child(
                h_flex()
                    .px_3()
                    .py_1()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(div().text_xs().font_medium().child("Git commands"))
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(format!("{} recorded", records.len())),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("clear-log")
                            .ghost()
                            .xsmall()
                            .label("Clear")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.sink.clear();
                                this.expanded_command = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("close-log")
                            .ghost()
                            .xsmall()
                            .icon(Lucide::X)
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_command_log(cx))),
                    ),
            )
            .child(list)
    }
}
