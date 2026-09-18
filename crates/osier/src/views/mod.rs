//! Each view is an `impl Osier` block with one `render_*` method.

mod branches;
mod changes;
mod command_log;
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
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, StyledExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use osier_git::repo::InProgress;

use crate::app::{Osier, Tab};

impl Osier {
    pub(crate) fn render_center(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let active_ix = Tab::ALL.iter().position(|t| *t == self.tab).unwrap_or(0);
        v_flex()
            .id("center")
            .size_full()
            .min_w_0()
            .key_context("OsierList")
            .track_focus(&self.list_focus)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.list_focus.focus(window, cx)),
            )
            .when_some(self.in_progress(), |this, op| {
                this.child(self.render_in_progress_bar(op, cx))
            })
            .child(
                TabBar::new("main-tabs")
                    .w_full()
                    .small()
                    .menu(true)
                    .selected_index(active_ix)
                    .on_click(cx.listener(|this, ix: &usize, _, cx| {
                        if let Some(tab) = Tab::ALL.get(*ix) {
                            this.set_tab(*tab, cx);
                        }
                    }))
                    .children(Tab::ALL.iter().map(|t| KitTab::new().label(t.label()))),
            )
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
