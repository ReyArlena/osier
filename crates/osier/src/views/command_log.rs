use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app::Osier;
use crate::notices::WindowNoticeExt as _;
use crate::test_ids::TestId as _;
use osier_git::runner::CommandRecord;

impl Osier {
    /// The bottom panel: a tab row (git commands, notifications), then the chosen list.
    pub(crate) fn render_bottom_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        use crate::app::BottomTab;
        let muted = cx.theme().muted_foreground;
        let tab = self.bottom_tab;
        let commands = self.sink.records().len();
        let failed = self.sink.records().iter().filter(|r| !r.expected).count();
        let failures_only = self.command_log_failures_only;
        let notices = crate::notices::NoticeLog::notices(cx).len();
        let tab_button = |id: &'static str, label: &'static str, which: BottomTab| {
            Button::new(id)
                .test_id(id)
                .xsmall()
                .map(|b| if tab == which { b.primary() } else { b.ghost() })
                .label(label)
                .on_click(cx.listener(move |this, _, _, cx| this.show_bottom_tab(which, cx)))
        };
        let header = h_flex()
            .px_3()
            .py_1()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(tab_button(
                "panel-commands",
                "Git commands",
                BottomTab::Commands,
            ))
            .child(tab_button(
                "panel-notifications",
                "Notifications",
                BottomTab::Notifications,
            ))
            .child(div().text_xs().text_color(muted).child(match tab {
                BottomTab::Commands => format!("{commands} recorded"),
                BottomTab::Notifications => crate::format::count(notices, "notification"),
            }))
            .child(div().flex_1())
            .when(tab == BottomTab::Commands, |this| {
                this.child(
                    Button::new("log-failures-only")
                        .test_id("log-failures-only")
                        .xsmall()
                        .map(|b| {
                            if failures_only {
                                b.primary()
                            } else {
                                b.ghost()
                            }
                        })
                        .label(match failed {
                            0 => "Failures only".to_string(),
                            1 => "Failures only (1)".to_string(),
                            n => format!("Failures only ({n})"),
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.command_log_failures_only = !this.command_log_failures_only;
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("clear-log")
                        .test_id("clear-log")
                        .ghost()
                        .xsmall()
                        .label("Clear")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.sink.clear();
                            this.expanded_command = None;
                            cx.notify();
                        })),
                )
            })
            .when(tab == BottomTab::Notifications, |this| {
                this.child(
                    Button::new("clear-notices")
                        .test_id("clear-notices")
                        .ghost()
                        .xsmall()
                        .label("Clear")
                        .on_click(cx.listener(|this, _, _, cx| {
                            crate::notices::NoticeLog::clear(cx);
                            this.notices_seen = crate::notices::NoticeLog::latest(cx);
                            cx.notify();
                        })),
                )
            })
            .child(
                Button::new("close-log")
                    .test_id("close-log")
                    .ghost()
                    .xsmall()
                    .icon(Lucide::X)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.command_log_visible = false;
                        this.save_state();
                        cx.notify();
                    })),
            );
        v_flex()
            .h(px(220.))
            .flex_none()
            .border_t_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().sidebar)
            .child(header)
            .child(match tab {
                BottomTab::Commands => self.render_command_log(cx).into_any_element(),
                BottomTab::Notifications => self.render_notices(cx).into_any_element(),
            })
    }

    /// The git command list of the bottom panel.
    fn render_command_log(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let mono = cx.theme().mono_font_family.clone();
        let all = self.sink.records();
        let failures_only = self.command_log_failures_only;
        // Failures are easy to miss: a refresh right after one runs a dozen routine commands.
        let records: Vec<CommandRecord> = if failures_only {
            all.iter().filter(|r| !r.expected).cloned().collect()
        } else {
            all.clone()
        };
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
            let ok = rec.expected;
            let id = rec.id;
            let is_open = expanded == Some(id);
            let shell = rec.as_shell();
            // Hide the boilerplate prefix in the one-line view.
            let short = shell
                .replace(" --no-pager -c color.ui=never -c core.quotePath=false", "")
                .replace("--no-pager -c color.ui=never -c core.quotePath=false ", "");
            let ms = rec.duration.as_millis();
            let started = crate::format::unix_secs(rec.started);
            list = list.child(
                v_flex()
                    .id(("cmd", id as usize))
                    .test_id(|| format!("command-row:{id}"))
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
                    .context_menu({
                        let weak = cx.entity().downgrade();
                        move |menu, _, _| {
                            let (w1, w2) = (weak.clone(), weak.clone());
                            menu.item(PopupMenuItem::new("Copy command").on_click(
                                move |_, window, cx| {
                                    let _ = w1.update(cx, |this, cx| {
                                        this.copy_command_record(id, false, window, cx)
                                    });
                                },
                            ))
                            .item(
                                PopupMenuItem::new("Copy details").on_click(
                                    move |_, window, cx| {
                                        let _ = w2.update(cx, |this, cx| {
                                            this.copy_command_record(id, true, window, cx)
                                        });
                                    },
                                ),
                            )
                        }
                    })
                    .child(
                        h_flex()
                            .px_3()
                            .py_0p5()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .text_color(muted)
                                    .flex_none()
                                    .child(crate::format::clock_time(started)),
                            )
                            .child(if ok {
                                Tag::success()
                                    .xsmall()
                                    .child(rec.status.unwrap_or(0).to_string())
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
                                .child(div().text_color(muted).child(format!(
                                    "started: {}",
                                    crate::format::full_time(started)
                                )))
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
            list = list.child(div().p_3().text_color(muted).child(if failures_only {
                "No failed commands."
            } else {
                "No git commands run yet."
            }));
        }
        list
    }

    /// Copy one recorded command: the shell line, or everything the expanded row shows.
    pub(crate) fn copy_command_record(
        &mut self,
        id: u64,
        details: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(rec) = self.sink.records().into_iter().find(|r| r.id == id) else {
            return;
        };
        let (text, what) = if details {
            (command_details(&rec), "command details")
        } else {
            (rec.as_shell(), "command")
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        window.push_notice(format!("Copied {what}"), cx);
    }
}

/// Plain-text form of an expanded log row, for pasting into an issue or a chat.
pub(crate) fn command_details(rec: &CommandRecord) -> String {
    let status = match rec.status {
        Some(code) => code.to_string(),
        None => "killed".into(),
    };
    let mut out = format!("$ {}\n", rec.as_shell());
    out.push_str(&format!(
        "started: {}\n",
        crate::format::full_time(crate::format::unix_secs(rec.started))
    ));
    if let Some(cwd) = &rec.cwd {
        out.push_str(&format!("cwd: {}\n", cwd.display()));
    }
    out.push_str(&format!(
        "exit: {status}\nduration: {} ms\nstdout: {} bytes\n",
        rec.duration.as_millis(),
        rec.stdout_len
    ));
    let stderr = rec.stderr.trim();
    if !stderr.is_empty() {
        out.push_str("stderr:\n");
        out.push_str(stderr);
        out.push('\n');
    }
    out
}
