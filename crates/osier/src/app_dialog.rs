//! Every Osier dialog opens through `open_app_dialog`: it fades in where it will sit instead
//! of gpui-kit's default slide down from the top. The fade runs for the theme's motion
//! duration (180ms by default) instead of the slide's fixed 250ms, so an opening dialog
//! costs about a third fewer frames (each frame costs the same). With animations off
//! (`appearance.animations`), dialogs appear at once either way.

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::dialog::{Dialog, DialogEntrance};
use gpui_kit::{App, Window};

pub(crate) trait AppDialogExt {
    /// `open_dialog`, with Osier's entrance.
    fn open_app_dialog<F>(&mut self, cx: &mut App, build: F)
    where
        F: Fn(Dialog, &mut Window, &mut App) -> Dialog + 'static;
}

impl AppDialogExt for Window {
    fn open_app_dialog<F>(&mut self, cx: &mut App, build: F)
    where
        F: Fn(Dialog, &mut Window, &mut App) -> Dialog + 'static,
    {
        self.open_dialog(cx, move |dialog, window, cx| {
            build(dialog.entrance(DialogEntrance::Fade), window, cx)
        });
    }
}
