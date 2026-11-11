//! The line under the pointer, highlighted in every code editor (the file viewer, the edit
//! pane, the three panes of the merge tool).
//!
//! gpui-kit's editor only highlights the cursor's line. This wraps an [`Editor`]: the wrapper
//! paints the editor's background and the highlight, and the editor (without its own
//! background) draws its text on top, so the highlight sits under the text like the active
//! line does. The gutter keeps its own background, so the highlight covers the text area.
//! A soft-wrapped line is highlighted as a whole.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::input::{Editor, EditorState};
use gpui_kit::*;

/// `editor` (built on `state`) with the line under the pointer highlighted.
pub(crate) fn hover_lines(editor: Editor, state: &Entity<EditorState>, cx: &App) -> AnyElement {
    let color = cx.theme().list_hover;
    let state = state.clone();
    div()
        .relative()
        .size_full()
        .bg(editor_background(cx))
        .rounded(cx.theme().radius)
        .child(
            canvas(
                |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
                move |bounds, hitbox, window, cx| paint(bounds, &hitbox, &state, color, window, cx),
            )
            .absolute()
            .size_full(),
        )
        .child(editor.appearance(false))
        .into_any_element()
}

/// The background gpui-kit's editor paints itself (`Theme::editor_background`, which is
/// private to gpui-kit): the theme's editor colour, else the input background.
fn editor_background(cx: &App) -> Hsla {
    let theme = cx.theme();
    theme
        .highlight_theme
        .style
        .editor_background
        .unwrap_or_else(|| theme.input_background())
}

/// The buffer line's rows, in window coordinates, under `y`.
fn line_at(state: &EditorState, y: Pixels) -> Option<Bounds<Pixels>> {
    let (text, line_height) = (state.text_bounds()?, state.line_height()?);
    if line_height <= px(0.) || y < text.origin.y {
        return None;
    }
    let map = state.display_map();
    let row = ((y - text.origin.y) / line_height).floor() as usize;
    if row >= map.display_row_count() {
        return None;
    }
    let line = map.display_row_to_buffer_line(row);
    let rows = map.buffer_line_to_display_row_range(line)?;
    let top = text.origin.y + line_height * rows.start as f32;
    Some(Bounds::new(
        point(text.origin.x, top),
        size(text.size.width, line_height * rows.len() as f32),
    ))
}

fn paint(
    bounds: Bounds<Pixels>,
    hitbox: &Hitbox,
    state: &Entity<EditorState>,
    color: Hsla,
    window: &mut Window,
    cx: &mut App,
) {
    let under = |position: Point<Pixels>, window: &Window, cx: &App| {
        if !hitbox.is_hovered(window) || !bounds.contains(&position) {
            return None;
        }
        line_at(state.read(cx), position.y)
    };
    let shown = under(window.mouse_position(), window, cx);
    if let Some(line) = shown {
        let line = Bounds::new(
            point(bounds.left(), line.top()),
            size(bounds.size.width, line.size.height),
        );
        window.paint_quad(fill(line, color));
    }
    // Redraw when the pointer moves to another line (or off the editor).
    let shown = Rc::new(Cell::new(shown.map(|b| b.top())));
    let (view, hitbox, state) = (window.current_view(), hitbox.clone(), state.clone());
    window.on_mouse_event(move |e: &MouseMoveEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble {
            return;
        }
        let now = (hitbox.is_hovered(window) && bounds.contains(&e.position))
            .then(|| line_at(state.read(cx), e.position.y))
            .flatten()
            .map(|b| b.top());
        if now != shown.get() {
            shown.set(now);
            cx.notify(view);
        }
    });
}
