//! One menu per list row, opened by right-click and, when the setting is on, by a `...`
//! button at the end of the row. Both use the same builder so they never drift apart.

use std::rc::Rc;

use gpui_kit::component::menu::PopupMenu;
use gpui_kit::*;

type Builder = dyn Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu;

#[derive(Clone)]
pub(crate) struct RowMenu(Rc<Builder>);

impl RowMenu {
    pub(crate) fn new(
        f: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> Self {
        Self(Rc::new(f))
    }

    /// The builder in the shape `context_menu` and `dropdown_menu` take.
    pub(crate) fn build(
        &self,
    ) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
        let f = self.0.clone();
        move |menu, window, cx| f(menu, window, cx)
    }
}

impl std::fmt::Debug for RowMenu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RowMenu")
    }
}
