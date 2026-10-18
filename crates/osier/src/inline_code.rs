//! A command or path shown inline, like HTML `<kbd>`/`<code>`: monospace on a muted chip.
//! Styled after gpui-kit's `Kbd`, which only renders keystrokes.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::*;

pub(crate) fn inline_code(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .flex_none()
        .px_1()
        .py_0p5()
        .rounded(cx.theme().radius.half())
        .bg(cx.theme().muted)
        .text_color(cx.theme().foreground)
        .font_family(cx.theme().mono_font_family.clone())
        .text_sm()
        .line_height(relative(1.2))
        .child(text.into())
}
