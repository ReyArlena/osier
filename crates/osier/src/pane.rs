//! Panes: the parts of a window that redraw on their own.
//!
//! gpui redraws a view when it, or a view inside it, is notified. A hover that starts or ends
//! notifies the view the hovered element was built in, so with everything built in one view
//! every row hover rebuilt, laid out and painted the whole window. A [`Pane`] is a small view
//! that renders one part of its owner (the sidebar, a tab, the diff) by calling one of the
//! owner's `render_*` methods. Elements built there belong to the pane, so a hover redraws
//! only that pane and every other pane reuses its last frame.
//!
//! The rules that follow (they hold for every window built this way):
//! - A window's root view (`Osier`) is a thin shell: layout containers, actions, resize
//!   handles. It is not cached and rebuilds on every frame, so keep it cheap. Anything with
//!   hover styles, tooltips or many elements belongs in a pane, not the shell.
//! - Panes are leaves. Never mount a cached pane inside another cached pane: a cached view that
//!   renders again also renders everything inside it, so the inner cache never hits.
//! - [`cached`] panes take their size from the container the shell puts them in (they cannot
//!   size to their content): give that container a size, or flex it. Bars that size to their
//!   content (title bar, status bar, tab strip) are [`live`] panes instead: rebuilt with every
//!   frame like the shell, but their hovers still only notify the pane.
//! - Every pane observes its owner, so `cx.notify()` on the owner redraws all of them. State
//!   changes keep working as before; only hovers and other view-local redraws stay local.
//! - Element state (scroll handles, focus handles, inputs) lives on the owner as before;
//!   listeners from the owner's `Context` (`cx.listener`, `cx.entity()`) work unchanged.
//! - While assistive technology is connected nothing is cached: a cached view drops out of
//!   gpui's accessibility tree on every frame that reuses it.
//!
//! A new part of the window: add a `render_*` method on the owner, a `Pane` for it in the
//! owner's `Panes`, and mount it in the shell with [`cached`] (or [`live`] for content-sized
//! bars).

use gpui_kit::*;

/// Renders one part of the owner.
pub(crate) type RenderFn<V> = fn(&mut V, &mut Window, &mut Context<V>) -> AnyElement;

/// A view that renders one part of its owner (see the module docs).
pub(crate) struct Pane<V: 'static> {
    owner: WeakEntity<V>,
    render: RenderFn<V>,
    _observe: Subscription,
}

impl<V: 'static> Pane<V> {
    pub(crate) fn new(owner: &Entity<V>, render: RenderFn<V>, cx: &mut App) -> Entity<Self> {
        let weak = owner.downgrade();
        cx.new(|cx| Self {
            owner: weak,
            render,
            _observe: cx.observe(owner, |_, _, cx| cx.notify()),
        })
    }
}

impl<V: 'static> Render for Pane<V> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let render = self.render;
        self.owner
            .update(cx, |owner, cx| render(owner, window, cx))
            .unwrap_or_else(|_| div().into_any_element())
    }
}

/// Mount `pane` so it reuses its last frame until it, or its owner, is notified. It fills
/// the container it is put in.
pub(crate) fn cached<V: 'static>(pane: &Entity<Pane<V>>, window: &Window) -> AnyElement {
    if window.is_a11y_active() {
        return pane.clone().into_any_element();
    }
    let mut style = StyleRefinement::default();
    style = style.size_full();
    pane.clone().cached(style).into_any_element()
}

/// Mount `pane` uncached: it sizes to its content and rebuilds with every frame.
pub(crate) fn live<V: 'static>(pane: &Entity<Pane<V>>) -> AnyElement {
    pane.clone().into_any_element()
}

/// `gpui_kit::open_window` for a window built from panes: the root view is mounted as is
/// (not cached, see the module docs), inside gpui-kit's `Root`.
pub(crate) fn open_window<V: Render>(
    options: WindowOptions,
    cx: &mut App,
    build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
) -> Result<(AnyWindowHandle, Entity<V>)> {
    let mut built = None;
    let window = cx.open_window(options, |window, cx| {
        let view = build(window, cx);
        built = Some(view.clone());
        cx.new(|cx| gpui_kit::base::Root::new(view, window, cx))
    })?;
    let view = built.ok_or_else(|| anyhow::anyhow!("open_window did not build the view"))?;
    Ok((window.into(), view))
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use gpui::{Modifiers, TestAppContext, VisualTestContext, point};
    use gpui_kit::{
        AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
        ParentElement as _, Render, Styled as _, Window, div, px, rgb,
    };

    use super::Pane;

    type ShellPane = Entity<Pane<Shell>>;

    /// A shell with two cached panes side by side, counting each pane's renders.
    struct Shell {
        left: Rc<Cell<usize>>,
        right: Rc<Cell<usize>>,
        panes: Option<(ShellPane, ShellPane)>,
    }

    impl Shell {
        fn render_left(&mut self, _: &mut Window, _: &mut Context<Self>) -> AnyElement {
            self.left.set(self.left.get() + 1);
            div()
                .id("left-row")
                .size_full()
                .hover(|s| s.bg(rgb(0x334455)))
                .into_any_element()
        }

        fn render_right(&mut self, _: &mut Window, _: &mut Context<Self>) -> AnyElement {
            self.right.set(self.right.get() + 1);
            div()
                .id("right-row")
                .size_full()
                .hover(|s| s.bg(rgb(0x334455)))
                .into_any_element()
        }
    }

    impl Render for Shell {
        fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let Some((left, right)) = &self.panes else {
                return div().into_any_element();
            };
            div()
                .flex()
                .size_full()
                .child(
                    div()
                        .w(px(100.))
                        .h(px(100.))
                        .child(super::cached(left, window)),
                )
                .child(
                    div()
                        .w(px(100.))
                        .h(px(100.))
                        .child(super::cached(right, window)),
                )
                .into_any_element()
        }
    }

    #[gpui::test]
    fn a_pane_redraws_alone_and_with_its_owner(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (left, right) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
        let (l, r) = (left.clone(), right.clone());
        let (handle, shell) = cx
            .update(|cx| {
                super::open_window(gpui_kit::WindowOptions::default(), cx, |_, cx| {
                    let shell = cx.new(|_| Shell {
                        left: l,
                        right: r,
                        panes: None,
                    });
                    let panes = (
                        Pane::new(&shell, Shell::render_left, cx),
                        Pane::new(&shell, Shell::render_right, cx),
                    );
                    shell.update(cx, |s, _| s.panes = Some(panes));
                    shell
                })
            })
            .expect("window");
        let window = handle
            .downcast::<gpui_kit::base::Root>()
            .expect("root window");
        let cx = &mut VisualTestContext::from_window(*window, cx);
        cx.run_until_parked();
        let (l0, r0) = (left.get(), right.get());
        assert!(l0 >= 1 && r0 >= 1, "both panes drawn");

        // A hover starting in the left pane redraws that pane only.
        cx.simulate_mouse_move(point(px(50.), px(50.)), None, Modifiers::none());
        cx.run_until_parked();
        assert!(left.get() > l0, "the hovered pane redraws");
        assert_eq!(right.get(), r0, "the other pane reuses its frame");

        // A redraw of the shell alone (a dialog animating over it) reuses both.
        let (l1, r1) = (left.get(), right.get());
        cx.update(|window, cx| gpui_kit::base::Root::update(window, cx, |_, _, cx| cx.notify()));
        cx.run_until_parked();
        assert_eq!(
            (left.get(), right.get()),
            (l1, r1),
            "panes reuse their frames"
        );

        // A change on the owner redraws every pane.
        cx.update(|_, cx| shell.update(cx, |_, cx| cx.notify()));
        cx.run_until_parked();
        assert!(
            left.get() > l1 && right.get() > r1,
            "owner changes reach all panes"
        );
    }
}
