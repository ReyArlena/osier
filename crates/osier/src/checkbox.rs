//! Osier's checkbox: gpui-kit's look, plus a third state (a dash: "some of it", for partly
//! staged files and hunks) and a subtle colour change under the pointer, so a box shows it can
//! be clicked. Use it instead of `gpui_kit::component::checkbox::Checkbox` everywhere.
//!
//! Built on gpui-kit's unstyled `base::Checkbox`, which owns toggling, focus, keyboard and the
//! accessibility state (including "mixed" for the dash).

use std::rc::Rc;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::{Checkbox as BaseCheckbox, CheckboxState};
use gpui_kit::component::{ActiveTheme as _, Disableable, Icon, Sizable, Size, StyledExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

type ClickHandler = Rc<dyn Fn(&bool, &mut Window, &mut App)>;

/// A checkbox with an optional label. `on_click` gets the state the click asks for: `true`
/// to tick (from empty or from the dash), `false` to clear.
#[derive(IntoElement)]
pub(crate) struct Checkbox {
    base: BaseCheckbox,
    state: CheckboxState,
    label: Option<SharedString>,
    accessibility_label: Option<SharedString>,
    disabled: bool,
    size: Size,
    on_click: Option<ClickHandler>,
    /// Shown after the label (a hint icon, for example).
    children: Vec<AnyElement>,
}

impl Checkbox {
    pub(crate) fn new(id: impl Into<ElementId>) -> Self {
        Self {
            // a11y: the name comes from `label` or `accessibility_label` at render.
            base: BaseCheckbox::new(id),
            state: CheckboxState::Unchecked,
            label: None,
            accessibility_label: None,
            disabled: false,
            size: Size::Medium,
            on_click: None,
            children: Vec::new(),
        }
    }

    pub(crate) fn checked(mut self, checked: bool) -> Self {
        self.state = if checked {
            CheckboxState::Checked
        } else {
            CheckboxState::Unchecked
        };
        self
    }

    /// The dash: neither ticked nor clear. A click ticks it.
    pub(crate) fn indeterminate(mut self, indeterminate: bool) -> Self {
        if indeterminate {
            self.state = CheckboxState::Indeterminate;
        }
        self
    }

    pub(crate) fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub(crate) fn accessibility_label(mut self, label: impl Into<SharedString>) -> Self {
        self.accessibility_label = Some(label.into());
        self
    }

    pub(crate) fn on_click(
        mut self,
        handler: impl Fn(&bool, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }
}

impl Disableable for Checkbox {
    fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl Sizable for Checkbox {
    fn with_size(mut self, size: impl Into<Size>) -> Self {
        self.size = size.into();
        self
    }
}

impl ParentElement for Checkbox {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl InteractiveElement for Checkbox {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl StatefulInteractiveElement for Checkbox {}

/// The hover group of a checkbox: the box changes colour while the pointer is anywhere over
/// the checkbox, label included.
const GROUP: &str = "osier-checkbox";

impl RenderOnce for Checkbox {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let (state, disabled) = (self.state, self.disabled);
        let marked = state != CheckboxState::Unchecked;
        let indicator = rems(match self.size {
            Size::XSmall => 0.75,
            Size::Small => 0.875,
            Size::Large => 1.125,
            _ => 1.,
        });
        let radius = theme.radius.min(px(4.));
        let primary = theme.primary;
        let border = theme.input;
        let empty_bg = theme.input_background();
        let mark_color = theme.primary_foreground;
        let accessibility_label = self.accessibility_label.or_else(|| self.label.clone());
        let icon = match state {
            CheckboxState::Checked => Some(Lucide::Check),
            CheckboxState::Indeterminate => Some(Lucide::Minus),
            CheckboxState::Unchecked => None,
        };
        let on_click = self.on_click;
        self.base
            .state(state)
            .disabled(disabled)
            .when_some(accessibility_label, |this, label| {
                this.accessibility_label(label)
            })
            .when_some(on_click, |this, on_click| {
                this.on_change(move |next, _, window, cx| {
                    window.prevent_default();
                    on_click(&(next == CheckboxState::Checked), window, cx);
                })
            })
            .group(GROUP)
            .h_flex()
            .gap_2()
            .items_start()
            .line_height(relative(1.))
            .text_color(theme.foreground)
            .map(|this| match self.size {
                Size::XSmall | Size::Small => this.text_sm(),
                _ => this.text_base(),
            })
            .when(disabled, |this| this.text_color(theme.muted_foreground))
            .when(!disabled, |this| this.cursor_pointer())
            .child(
                div()
                    .relative()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(indicator)
                    .when(self.label.is_some(), |this| this.mt(indicator * 0.125))
                    .flex_shrink_0()
                    .border_1()
                    .rounded(radius)
                    .map(|this| {
                        if marked {
                            this.border_color(primary).bg(primary)
                        } else {
                            this.border_color(border).bg(empty_bg)
                        }
                    })
                    .when(disabled, |this| {
                        if marked {
                            this.border_color(primary.opacity(0.5))
                                .bg(primary.opacity(0.5))
                        } else {
                            this.border_color(border.opacity(0.5))
                        }
                    })
                    // Under the pointer: an empty box takes a tint and an accent edge, a
                    // ticked one gets a little lighter.
                    .when(!disabled, |this| {
                        this.group_hover(GROUP, move |s| {
                            if marked {
                                s.bg(primary.opacity(0.85))
                            } else {
                                s.border_color(primary.opacity(0.7))
                                    .bg(primary.opacity(0.12))
                            }
                        })
                    })
                    .when_some(icon, |this, icon| {
                        this.child(
                            Icon::new(icon)
                                .size(indicator * 0.75)
                                .text_color(if disabled {
                                    mark_color.opacity(0.5)
                                } else {
                                    mark_color
                                }),
                        )
                    }),
            )
            .when_some(self.label, |this, label| this.child(div().child(label)))
            .children(self.children)
    }
}
