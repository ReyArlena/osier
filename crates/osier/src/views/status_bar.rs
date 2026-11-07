use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, StyledExt as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use osier_git::repo::InProgress;

use crate::app::Osier;
use crate::test_ids::TestId as _;

impl Osier {
    pub(crate) fn render_status_bar(
        &self,
        has_project: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // Main text (branch, running jobs) in the foreground; secondary text in the bar's
        // grey, a touch lighter.
        let fg = cx.theme().foreground;
        let muted = cx.theme().muted_foreground.blend(fg.opacity(0.12));
        let primary = cx.theme().primary;
        let busy = |text: String| {
            h_flex()
                .gap_1p5()
                .items_center()
                .child(
                    Spinner::new()
                        .icon(Lucide::LoaderCircle)
                        .small()
                        .color(primary),
                )
                .child(div().truncate().max_w(px(420.)).child(text))
        };
        let background = self.background_labels();
        let project = self.active_project();
        let snapshot = project.and_then(|p| p.snapshot.as_ref());
        let branch = snapshot.map(|s| &s.status.branch);
        let error = project
            .and_then(|p| p.error.clone())
            .or_else(|| self.global_error.clone());
        let in_progress = snapshot.and_then(|s| s.in_progress).map(|op| match op {
            InProgress::Merge => "merge in progress",
            InProgress::Rebase => "rebase in progress",
            InProgress::CherryPick => "cherry-pick in progress",
            InProgress::Revert => "revert in progress",
            InProgress::Bisect => "bisect in progress",
        });
        let changes = snapshot.map(|s| s.status.entries.len()).unwrap_or(0);

        StatusBar::new()
            .left(
                h_flex()
                    .gap_1()
                    .text_xs()
                    .text_color(fg)
                    .items_center()
                    .when(has_project, |this| {
                        this.child(Icon::new(Lucide::GitBranch).xsmall())
                    })
                    .when_some(branch, |this, b| {
                        this.child(b.head.clone().unwrap_or_else(|| "detached".into()))
                            .when(b.upstream.is_some(), |this| {
                                this.child(
                                    div()
                                        .text_color(muted)
                                        .child(format!("↑{} ↓{}", b.ahead, b.behind)),
                                )
                            })
                            .when(b.upstream.is_none() && b.oid.is_some(), |this| {
                                this.child(div().text_color(muted).child("no upstream"))
                            })
                            .when(b.oid.is_none(), |this| {
                                this.child(div().text_color(muted).child("no commits"))
                            })
                    }),
            )
            .left(
                h_flex()
                    .gap_3()
                    .text_xs()
                    .text_color(fg)
                    .items_center()
                    .when_some(in_progress, |this, op| {
                        this.child(Tag::warning().xsmall().child(op))
                    })
                    .when_some(self.remote_job.as_ref(), |this, job| {
                        let text = match job.percent {
                            Some(p) => format!("{}: {} {p}%", job.label, job.phase),
                            None => format!("{}: {}", job.label, job.phase),
                        };
                        this.child(
                            busy(text).test_id("remote-job").child(
                                Button::new("cancel-remote")
                                    .test_id("cancel-remote")
                                    .ghost()
                                    .xsmall()
                                    .label("Cancel")
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.cancel_remote_job(cx)),
                                    ),
                            ),
                        )
                    })
                    // Quiet work, when `appearance.show_background_tasks` asks for it.
                    .when(!background.is_empty(), |this| {
                        this.child(busy(background.join(" · ")).test_id("background-tasks"))
                    })
                    .when_some(error, |this, e| {
                        this.child(
                            h_flex()
                                .gap_1()
                                .items_center()
                                .text_color(cx.theme().danger)
                                .child(Icon::new(Lucide::CircleX).xsmall())
                                .child(div().truncate().max_w(px(480.)).child(e)),
                        )
                    }),
            )
            .right(
                h_flex()
                    .gap_3()
                    .text_xs()
                    .text_color(muted)
                    .items_center()
                    // Waiting for the key after the prefix key: orange (the theme's peach).
                    .when(self.prefix_waiting(), |this| {
                        this.child(
                            h_flex()
                                .test_id("prefix-badge")
                                .gap_1()
                                .items_center()
                                .child("Esc cancels")
                                .child(
                                    div()
                                        .px_1p5()
                                        .py_0p5()
                                        .rounded(cx.theme().radius)
                                        .bg(cx.theme().chart_4)
                                        .text_color(cx.theme().background)
                                        .font_medium()
                                        .child("Prefix key pressed"),
                                ),
                        )
                    })
                    .when(has_project, |this| {
                        this.child(crate::format::count(changes, "change"))
                    })
                    .child(
                        h_flex()
                            .gap_1()
                            .items_center()
                            .children(Keystroke::parse("ctrl-q").ok().map(Kbd::new))
                            .child("Quit"),
                    ),
            )
    }
}
