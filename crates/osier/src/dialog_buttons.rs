//! OK and Cancel buttons for a gpui-kit `Dialog`.
//!
//! A plain `Dialog` never draws buttons: `button_props` only installs what the `Confirm`
//! (Enter) and `Cancel` (Escape) actions do. This footer draws the buttons and dispatches
//! those same actions, so a click behaves exactly like the key. Like gpui-kit's own
//! `AlertDialog` buttons, each dispatches from a hidden focus node inside itself, so the action
//! reaches the dialog the button sits in whatever holds focus.

use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::dialog::{Cancel, Confirm, DialogFooter};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::test_ids::TestId as _;

#[derive(IntoElement)]
pub(crate) struct DialogButtons {
    ok: SharedString,
    variant: ButtonVariant,
    cancel: Option<SharedString>,
}

impl DialogButtons {
    pub(crate) fn new(ok: impl Into<SharedString>) -> Self {
        Self {
            ok: ok.into(),
            variant: ButtonVariant::Primary,
            cancel: Some("Cancel".into()),
        }
    }

    pub(crate) fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }
}

impl RenderOnce for DialogButtons {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let anchor = |key: &'static str, window: &mut Window, cx: &mut App| {
            window
                .use_keyed_state(key, cx, |_, cx| cx.focus_handle())
                .read(cx)
                .clone()
        };
        let ok_anchor = anchor("osier-dialog-ok", window, cx);
        let cancel_anchor = anchor("osier-dialog-cancel", window, cx);
        DialogFooter::new()
            .when_some(self.cancel, |this, label| {
                let handle = cancel_anchor.clone();
                this.child(
                    Button::new("dialog-cancel")
                        .test_id("dialog-cancel")
                        .label(label)
                        .child(div().absolute().size_0().track_focus(&cancel_anchor))
                        .on_click(move |_, window, cx| handle.dispatch_action(&Cancel, window, cx)),
                )
            })
            .child({
                let handle = ok_anchor.clone();
                Button::new("dialog-ok")
                    .test_id("dialog-ok")
                    .label(self.ok)
                    .with_variant(self.variant)
                    .child(div().absolute().size_0().track_focus(&ok_anchor))
                    .on_click(move |_, window, cx| {
                        handle.dispatch_action(&Confirm { secondary: false }, window, cx)
                    })
            })
    }
}
