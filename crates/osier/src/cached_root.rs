//! Mount the main view as a cached gpui view.
//!
//! gpui re-renders, lays out and paints every uncached view on every frame. While a dialog
//! animates in (settings, the command palette) that meant the whole main window was rebuilt
//! about 15 times in 250ms although nothing under the dialog changed. A cached view reuses
//! its last frame until it, or an entity it read while rendering, is notified, or the window
//! is refreshed.
//!
//! The rule that follows: change what the main view shows through `cx.notify()` on the view
//! (or an entity it reads), and when a *global* it reads changes (keymap, tooltip delay),
//! call `cx.refresh_windows()`. gpui does not track globals, so without it the main view
//! would keep its old frame until something else redraws it. gpui-kit's `Theme::update`
//! already refreshes, so theme changes are covered.
//!
//! The cache is off while assistive technology is connected (see `render`): a cached view
//! drops out of gpui's accessibility tree on every frame that reuses it.

use gpui_kit::*;

/// The window's content: the real view, cached. Cheap to render on its own.
struct CachedRoot<V: Render> {
    inner: Entity<V>,
}

impl<V: Render> Render for CachedRoot<V> {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        // gpui builds the accessibility tree while it prepaints, and a view served from the
        // cache is not prepainted again: on every frame that reuses it, all of its nodes are
        // missing from the tree sent to assistive technology (the whole window goes blank for
        // a screen reader while a toast fades). So no cache while accessibility is active;
        // gpui redraws when that changes, which brings us back here.
        if window.is_a11y_active() {
            return self.inner.clone().into_any_element();
        }
        let mut style = StyleRefinement::default();
        style = style.size_full();
        self.inner.clone().cached(style).into_any_element()
    }
}

/// `gpui_kit::open_window`, with the built view mounted cached.
pub(crate) fn open_window<V: Render>(
    options: WindowOptions,
    cx: &mut App,
    build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
) -> Result<(AnyWindowHandle, Entity<V>)> {
    let mut built = None;
    let window = cx.open_window(options, |window, cx| {
        let view = build(window, cx);
        built = Some(view.clone());
        let shell = cx.new(|_| CachedRoot { inner: view });
        cx.new(|cx| gpui_kit::base::Root::new(shell, window, cx))
    })?;
    let view = built.ok_or_else(|| anyhow::anyhow!("open_window did not build the view"))?;
    Ok((window.into(), view))
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use gpui::TestAppContext;
    use gpui_kit::{AppContext as _, Context, IntoElement, Render, Window, div};

    struct Counted(Rc<Cell<usize>>);

    impl Render for Counted {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            self.0.set(self.0.get() + 1);
            div()
        }
    }

    #[gpui::test]
    fn the_view_renders_again_only_when_notified_or_refreshed(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let renders = Rc::new(Cell::new(0));
        let r = renders.clone();
        let (handle, view) = cx
            .update(|cx| {
                super::open_window(gpui_kit::WindowOptions::default(), cx, |_, cx| {
                    cx.new(|_| Counted(r))
                })
            })
            .expect("window");
        cx.run_until_parked();
        let first = renders.get();
        assert!(first >= 1);
        let _ = cx.update_window(handle, |_, window, _| window.refresh());
        cx.run_until_parked();
        let after_refresh = renders.get();
        assert!(after_refresh > first, "window.refresh() ignores the cache");
        let _ = cx.update_window(handle, |_, window, cx| {
            gpui_kit::base::Root::update(window, cx, |_, _, cx| cx.notify())
        });
        cx.run_until_parked();
        assert_eq!(
            renders.get(),
            after_refresh,
            "a redraw of the root alone reuses the view"
        );
        cx.update(|cx| view.update(cx, |_, cx| cx.notify()));
        cx.run_until_parked();
        assert!(
            renders.get() > after_refresh,
            "notifying the view renders it"
        );
        let before = renders.get();
        cx.update(|cx| cx.refresh_windows());
        cx.run_until_parked();
        assert!(
            renders.get() > before,
            "refresh_windows redraws past the cache"
        );
    }
}
