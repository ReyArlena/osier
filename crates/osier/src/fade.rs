//! Soft right edge for text that is clipped, and how a line of text that does not fit ends
//! (`appearance.text_overflow`: a fade, an ellipsis, or an ellipsis that fades out).
//!
//! gpui has no alpha masks, so the fade is an overlay: a short horizontal gradient from
//! transparent to the colour behind the text. The parent must be `relative()` and clip
//! (`overflow_hidden`). When the background changes with state (hover, selection), pass the
//! matching colour, or the overlay shows as a band.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::*;
use osier_config::settings::TextOverflow;

/// Width of the fade.
pub(crate) const FADE_WIDTH: f32 = 28.0;

/// Width of the fade over an ellipsis (`TextOverflow::Both`): about the "…" itself, so the
/// dots fade out and the text before them stays as it is.
const ELLIPSIS_FADE_WIDTH: f32 = 20.0;

/// The gradient itself, for `bg()` and hover refinements.
pub(crate) fn fade_fill(color: Hsla) -> Background {
    linear_gradient(
        90.,
        linear_color_stop(color.opacity(0.), 0.),
        linear_color_stop(color, 1.),
    )
}

/// The overlay element, pinned to the right edge of its (relative, clipping) parent.
pub(crate) fn fade_right(color: Hsla) -> Div {
    div()
        .absolute()
        .top_0()
        .right_0()
        .h_full()
        .w(px(FADE_WIDTH))
        .bg(fade_fill(color))
}

/// The colours behind a line of text, for its fade: plain, and under the pointer anywhere in
/// the hover `group` (the row), when the row has a hover colour.
#[derive(Clone, Copy)]
pub(crate) struct Behind {
    base: Hsla,
    hover: Option<(&'static str, Hsla)>,
}

impl Behind {
    /// Text on the pane's own background (a header line), which never changes colour.
    pub(crate) fn plain(cx: &App) -> Self {
        Self {
            base: cx.theme().background,
            hover: None,
        }
    }

    /// Text in a list row of a center tab: the pane background, `list_active` while the
    /// row is selected, `list_hover` while the pointer is over the row (`group`).
    pub(crate) fn row(group: &'static str, selected: bool, cx: &App) -> Self {
        let bg = cx.theme().background;
        Self {
            base: if selected {
                bg.blend(cx.theme().list_active)
            } else {
                bg
            },
            hover: Some((group, bg.blend(cx.theme().list_hover))),
        }
    }

    /// Explicit colours (`hover` while the pointer is over `group`).
    pub(crate) fn colors(base: Hsla, hover: Hsla, group: &'static str) -> Self {
        Self {
            base,
            hover: Some((group, hover)),
        }
    }

    /// A fade overlay of `width` in these colours.
    fn overlay(self, width: f32) -> Div {
        let overlay = fade_right(self.base).w(px(width));
        match self.hover {
            Some((group, hover)) => overlay.group_hover(group, move |s| s.bg(fade_fill(hover))),
            None => overlay,
        }
    }
}

/// How a one-line text box ends when its text does not fit (`appearance.text_overflow`).
pub(crate) trait EndText: Sized {
    /// Add `content` as the box's text and end it as `style` says: fade it out into
    /// `behind`, cut it with an ellipsis, or both (the ellipsis fades out). The box must be
    /// able to shrink (`min_w_0`, or a `max_w`).
    fn end_text(self, style: TextOverflow, behind: Behind, content: impl IntoElement) -> Self;
}

impl EndText for Div {
    fn end_text(self, style: TextOverflow, behind: Behind, content: impl IntoElement) -> Self {
        // The overlay comes after the text, so it paints over it.
        match style {
            TextOverflow::Ellipsis => self.truncate().child(content),
            TextOverflow::Fade => self
                .relative()
                .overflow_hidden()
                .whitespace_nowrap()
                .child(content)
                .child(behind.overlay(FADE_WIDTH)),
            TextOverflow::Both => self
                .relative()
                .truncate()
                .child(content)
                .child(behind.overlay(ELLIPSIS_FADE_WIDTH)),
        }
    }
}

/// The overlay alone, for a box that clips several things (text and badges) in one row:
/// none for an ellipsis, a short one over the ellipsis for both, the full fade otherwise.
pub(crate) fn end_overlay(style: TextOverflow, behind: Behind) -> Option<Div> {
    match style {
        TextOverflow::Ellipsis => None,
        TextOverflow::Fade => Some(behind.overlay(FADE_WIDTH)),
        TextOverflow::Both => Some(behind.overlay(ELLIPSIS_FADE_WIDTH)),
    }
}
