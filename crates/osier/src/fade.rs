//! Soft right edge for text that is clipped (no ellipsis possible, e.g. a row of badges).
//!
//! gpui has no alpha masks, so the fade is an overlay: a short horizontal gradient from
//! transparent to the colour behind the text. The parent must be `relative()` and clip
//! (`overflow_hidden`). When the background changes with state (hover, selection), pass the
//! matching colour, or the overlay shows as a band.

use gpui_kit::*;

/// Width of the fade.
pub(crate) const FADE_WIDTH: f32 = 28.0;

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
