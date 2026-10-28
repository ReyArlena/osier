//! Each view is an `impl Osier` block with one `render_*` method.

mod branches;
mod changes;
pub(crate) mod command_log;
mod diff_pane;
mod files_tab;
mod log_tab;
mod patches_tab;
mod sidebar;
mod stashes;
mod status_bar;
mod title_bar;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::tab::{Tab as KitTab, TabBar};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, StyledExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use osier_git::repo::InProgress;

use crate::app::{Osier, Tab};
use crate::test_ids::TestId as _;

/// A time as lists show it (`text`, e.g. "12h ago" or a date) with the full local date and
/// time in a tooltip. gpui's own element tooltips have a fixed delay, so the configured one
/// (`appearance.tooltip_delay_ms`, installed as gpui-kit's tooltip default) is passed on.
pub(crate) fn time_label(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    unix: i64,
    cx: &App,
) -> Stateful<Div> {
    let full = SharedString::from(crate::format::full_time(unix));
    let delay = gpui_kit::base::TooltipDefaults::global(cx).show_delay();
    div()
        .id(id)
        .child(text.into())
        .tooltip_show_delay(delay)
        .tooltip(move |window, cx| {
            let full = full.clone();
            gpui_kit::component::tooltip::Tooltip::element(move |_, _| {
                div().test_id("time-tooltip").child(full.clone())
            })
            .build(window, cx)
        })
}

/// [`time_label`] with the relative time ("5 min ago", "12h ago", "yesterday").
pub(crate) fn relative_time_label(id: impl Into<ElementId>, unix: i64, cx: &App) -> Stateful<Div> {
    time_label(id, crate::format::relative_time(unix), unix, cx)
}

impl Osier {
    pub(crate) fn render_center(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let active_ix = Tab::ALL.iter().position(|t| *t == self.tab).unwrap_or(0);
        // Measured on the previous frame; the canvas below re-renders when it changes.
        let tab_edges = tab_strip_edges(&self.tab_scroll);
        let tabs_overflow = tab_edges.overflow;
        let changes = self
            .active_project()
            .and_then(|p| p.snapshot.as_ref())
            .map_or(0, |s| s.status.entries.len());
        v_flex()
            .id("center")
            .size_full()
            .min_w_0()
            .key_context("OsierList")
            .track_focus(&self.list_focus)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    // Take focus for the list keys (arrows, space, delete) unless something
                    // inside the panel already has it: the commit box or a filter input
                    // focuses itself on the same click and must keep it.
                    let inner_has_focus = this.list_focus.contains_focused(window, cx)
                        && !this.list_focus.is_focused(window);
                    if !inner_has_focus {
                        this.list_focus.focus(window, cx);
                    }
                }),
            )
            .when_some(self.in_progress(), |this, op| {
                this.child(self.render_in_progress_bar(op, cx))
            })
            .child(
                TabBar::new("main-tabs")
                    .w_full()
                    // The tab list menu only when the tabs do not all fit.
                    .menu(tabs_overflow)
                    .track_scroll(&self.tab_scroll)
                    .selected_index(active_ix)
                    .on_click(cx.listener(|this, ix: &usize, _, cx| {
                        if let Some(tab) = Tab::ALL.get(*ix) {
                            this.set_tab(*tab, cx);
                        }
                    }))
                    .children(Tab::ALL.iter().map(|t| {
                        KitTab::new()
                            .label(t.label())
                            .test_id(format!("tab-{}", t.label().to_lowercase()))
                            .when(*t == Tab::Changes && changes > 0, |tab| {
                                tab.suffix(
                                    div()
                                        .test_id("tab-changes-count")
                                        // The label keeps its own 12px padding (plus the
                                        // tab's 4px gap): pull the badge in to sit 6px from
                                        // the text, and match text-only tabs at the edge.
                                        .ml(px(-10.))
                                        .pr(px(14.))
                                        .child(
                                            Tag::secondary().xsmall().child(changes.to_string()),
                                        ),
                                )
                            })
                    })),
            )
            .child({
                let handle = self.tab_scroll.clone();
                let view = cx.entity().downgrade();
                canvas(
                    move |_, window, cx| {
                        if tab_strip_edges(&handle) != tab_edges {
                            let view = view.clone();
                            window.defer(cx, move |_, cx| {
                                let _ = view.update(cx, |_, cx| cx.notify());
                            });
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_0()
            })
            // Tabs cut off by the strip's edges fade out instead of running into the menu
            // button. Anchored in window coordinates at the visible strip's edges; they take
            // no clicks.
            .when(tab_edges.more_left || tab_edges.more_right, |this| {
                let strip = self.tab_scroll.bounds();
                let bg = cx.theme().tab_bar;
                let w = px(crate::fade::FADE_WIDTH);
                // Stop short of the bar's bottom border.
                let h = (strip.size.height - px(1.)).max(px(0.));
                this.when(tab_edges.more_right, |this| {
                    this.child(
                        anchored()
                            .position(point(strip.right() - w, strip.top()))
                            .child(div().w(w).h(h).bg(crate::fade::fade_fill(bg))),
                    )
                })
                .when(tab_edges.more_left, |this| {
                    this.child(anchored().position(strip.origin).child(div().w(w).h(h).bg(
                        linear_gradient(
                            90.,
                            linear_color_stop(bg, 0.),
                            linear_color_stop(bg.opacity(0.), 1.),
                        ),
                    )))
                })
            })
            .child(div().flex_1().min_h_0().child(match self.tab {
                Tab::Changes => self.render_changes(cx).into_any_element(),
                Tab::Log => self.render_log(window, cx).into_any_element(),
                Tab::Branches => self.render_branches(cx).into_any_element(),
                Tab::Stashes => self.render_stashes(cx).into_any_element(),
                Tab::Files => self.render_files(cx).into_any_element(),
                Tab::Patches => self.render_patches(cx).into_any_element(),
            }))
    }

    /// Bar shown while a merge, rebase, cherry-pick or revert waits for the user.
    fn render_in_progress_bar(&self, op: InProgress, cx: &mut Context<Self>) -> impl IntoElement {
        let conflicts = self
            .active_project()
            .and_then(|p| p.snapshot.as_ref())
            .map(|s| s.status.conflicted().count())
            .unwrap_or(0);
        let (name, skippable) = match op {
            InProgress::Merge => ("Merge", false),
            InProgress::Rebase => ("Rebase", true),
            InProgress::CherryPick => ("Cherry-pick", true),
            InProgress::Revert => ("Revert", true),
            InProgress::Bisect => ("Bisect", false),
        };
        let text = if conflicts > 0 {
            format!(
                "{name} in progress: {conflicts} conflicted {}.",
                if conflicts == 1 { "file" } else { "files" }
            )
        } else {
            format!("{name} in progress, nothing left to resolve.")
        };
        h_flex()
            .px_3()
            .py_2()
            .gap_2()
            .items_center()
            .bg(cx.theme().warning.opacity(0.15))
            .border_b_1()
            .border_color(cx.theme().warning.opacity(0.4))
            .child(div().text_sm().flex_1().min_w_0().child(text))
            .child(
                Button::new("ip-continue")
                    .test_id("ip-continue")
                    .primary()
                    .xsmall()
                    .label("Continue")
                    .disabled(conflicts > 0 || op == InProgress::Bisect)
                    .on_click(
                        cx.listener(|this, _, window, cx| this.continue_in_progress(window, cx)),
                    ),
            )
            .when(skippable, |this| {
                this.child(
                    Button::new("ip-skip")
                        .test_id("ip-skip")
                        .outline()
                        .xsmall()
                        .label("Skip commit")
                        .on_click(
                            cx.listener(|this, _, window, cx| this.skip_in_progress(window, cx)),
                        ),
                )
            })
            .child(
                Button::new("ip-abort")
                    .test_id("ip-abort")
                    .danger()
                    .xsmall()
                    .label("Abort")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.confirm_abort_in_progress(window, cx)
                    })),
            )
    }

    /// Centered placeholder used by tabs when nothing is loaded.
    pub(crate) fn empty_state(
        &self,
        title: &str,
        hint: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        use gpui_kit::component::ActiveTheme as _;
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_2()
            .p_6()
            .child(div().font_medium().child(title.to_string()))
            .child(
                div()
                    .text_sm()
                    .text_center()
                    .text_color(cx.theme().muted_foreground)
                    .child(hint.to_string()),
            )
    }
}

/// What the tab strip looked like on the last frame: whether its tabs overflow, and whether
/// some are hidden past either edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TabEdges {
    overflow: bool,
    more_left: bool,
    more_right: bool,
}

fn tab_strip_edges(handle: &ScrollHandle) -> TabEdges {
    let max = handle.max_offset().x;
    // Scroll offsets are negative: -max is fully scrolled to the end.
    let offset = handle.offset().x;
    let overflow = max > px(0.5);
    TabEdges {
        overflow,
        more_left: overflow && offset < px(-0.5),
        more_right: overflow && offset + max > px(0.5),
    }
}
