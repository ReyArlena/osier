//! Long lists that build, lay out and paint only the rows on screen.
//!
//! A list built as one `v_flex` of every row lays out and paints all of them on every redraw
//! of its pane, including each hover. `uniform_list` avoids that for rows of one height; this
//! is the same for lists that mix row heights (section headers, two-line rows): gpui's `list`
//! measures rows as they come into view.
//!
//! Use: describe the rows as a small `Hash` value each (a section header, or an index into
//! the snapshot), keep a [`LazyList`] on the owner, and render with
//! `rows(self.thing_list.sync(&rows, None), cx.processor(move |this, ix, window, cx| ...))`.
//! `cx.processor` hands the row builder the owner and its `Context`, so row code keeps using
//! `cx.listener` and the owner's fields as before.

use std::cell::Cell;
use std::hash::{Hash, Hasher};

use gpui_kit::*;

/// gpui's `list` with each row stretched to the list's width (gpui lays every row out on its
/// own, so rows would otherwise shrink to their content like in a row of a flex container).
pub(crate) fn rows(
    state: ListState,
    mut render_row: impl FnMut(usize, &mut Window, &mut App) -> AnyElement + 'static,
) -> List {
    list(state, move |ix, window, cx| {
        div()
            .flex()
            .flex_col()
            .w_full()
            .child(render_row(ix, window, cx))
            .into_any_element()
    })
}

/// Scroll position and measured row heights of one lazy list.
pub(crate) struct LazyList {
    state: ListState,
    shape: Cell<Option<u64>>,
    reveal: Cell<bool>,
}

impl Default for LazyList {
    fn default() -> Self {
        Self {
            state: ListState::new(0, ListAlignment::Top, px(200.)),
            shape: Cell::new(None),
            reveal: Cell::new(false),
        }
    }
}

impl LazyList {
    /// Scroll the selected row into view on the next [`Self::sync`] (after a keyboard move).
    pub(crate) fn reveal_selection(&self) {
        self.reveal.set(true);
    }

    /// Bring the list up to date with this frame's `rows` and return its state for `list`.
    /// When the rows change, their heights are measured again and the scroll position is kept
    /// in proportion. `selected` is the selected row's index, for [`Self::reveal_selection`].
    pub(crate) fn sync<T: Hash>(&self, rows: &[T], selected: Option<usize>) -> ListState {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        rows.hash(&mut hasher);
        let shape = hasher.finish();
        let (old, new) = (self.state.item_count(), rows.len());
        if new > old {
            self.state.splice(old..old, new - old);
        } else if new < old {
            self.state.splice(new..old, 0);
        }
        if self.shape.replace(Some(shape)) != Some(shape) {
            self.state.remeasure();
        }
        if self.reveal.take()
            && let Some(ix) = selected
        {
            self.state.scroll_to_reveal_item(ix);
        }
        self.state.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::LazyList;

    #[test]
    fn sync_follows_the_row_count() {
        let list = LazyList::default();
        assert_eq!(list.sync(&[1, 2, 3], None).item_count(), 3);
        assert_eq!(list.sync(&[1, 2, 3, 4, 5], None).item_count(), 5);
        assert_eq!(list.sync(&[1], None).item_count(), 1);
        assert_eq!(list.sync::<u8>(&[], None).item_count(), 0);
    }
}
