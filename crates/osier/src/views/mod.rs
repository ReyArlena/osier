//! Each view is an `impl Osier` block with one `render_*` method.

mod branches;
mod changes;
mod command_log;
mod diff_pane;
mod log_tab;
mod sidebar;
mod stashes;
mod status_bar;
mod title_bar;

use gpui_kit::component::tab::{Tab as KitTab, TabBar};
use gpui_kit::component::{Sizable as _, StyledExt as _, v_flex};
use gpui_kit::*;

use crate::app::{Osier, Tab};

impl Osier {
    pub(crate) fn render_center(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let active_ix = Tab::ALL.iter().position(|t| *t == self.tab).unwrap_or(0);
        v_flex()
            .size_full()
            .min_w_0()
            .child(
                TabBar::new("main-tabs")
                    .w_full()
                    .small()
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
            }))
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
