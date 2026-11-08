//! Helper text under a dialog option.
//!
//! Option labels stay short and name the action. What the option does, what it leaves alone
//! and what comes next goes underneath in muted text, never in brackets on the label.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::radio::Radio;
use gpui_kit::*;

/// Muted, smaller text for a `Checkbox` child, or for a line in a dialog body.
pub(crate) fn hint(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

/// A radio with a short label and a hint underneath. gpui-kit gives a radio's text column the
/// full width of the radio on top of the indicator (1rem) and gap (0.5rem), so wrapped text
/// would run that far past the dialog's padding. Right padding on the radio takes it back.
pub(crate) fn hinted_radio(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    text: impl Into<SharedString>,
    cx: &App,
) -> Radio {
    Radio::new(id)
        .label(label.into())
        .pr(rems(1.5))
        .child(hint(text, cx))
}
