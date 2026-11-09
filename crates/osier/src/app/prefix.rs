//! The prefix (leader) key. Pressing it puts Osier in a waiting state, shown as a badge in the
//! status bar: the next key runs the action bound to it "after the prefix". Escape, a click,
//! any key with no prefixed binding, or the timeout (Settings > Keymap) cancel.
//!
//! The waiting state uses a keystroke interceptor rather than gpui's multi-key bindings: those
//! time out silently after about a second and show nothing, and an interceptor sees the next
//! key wherever focus is, text fields included, before it can type.

use gpui_kit::component::WindowExt as _;
use gpui_kit::*;
use std::time::Duration;

use crate::app::Osier;
use crate::keymap::{Keymap, actions, keystroke_string, pretty_key};

impl Osier {
    /// The prefix key was pressed: wait for the next key.
    pub(crate) fn start_prefix(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_prefix(cx);
        let generation = self.prefix_generation;
        let weak = cx.weak_entity();
        let sub = cx.intercept_keystrokes(move |event, window, cx| {
            // A modifier on its own: keep waiting for the real key.
            let Some(keys) = keystroke_string(&event.keystroke) else {
                return;
            };
            cx.stop_propagation();
            let _ = weak.update(cx, |this, cx| this.finish_prefix(keys, window, cx));
        });
        self.prefix_pending = Some(sub);
        let secs = self.settings.keys.prefix_timeout_secs;
        if secs > 0 {
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_secs(u64::from(secs)))
                    .await;
                let _ = this.update(cx, |this, cx| {
                    if this.prefix_generation == generation {
                        this.cancel_prefix(cx);
                    }
                });
            })
            .detach();
        }
        cx.notify();
    }

    /// The key after the prefix: run what is bound to it.
    fn finish_prefix(&mut self, keys: String, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_prefix(cx);
        if keys == "escape" {
            return;
        }
        let wanted = keys.to_ascii_lowercase();
        let ids: Vec<&'static str> = Keymap::global(cx)
            .prefixed
            .iter()
            .filter(|(k, _)| *k == wanted)
            .map(|(_, id)| *id)
            .collect();
        if ids.is_empty() {
            window.push_notification(
                format!("Nothing is bound to Prefix {}", pretty_key(&keys)),
                cx,
            );
            return;
        }
        let specs = actions();
        for id in ids {
            if let Some(spec) = specs.iter().find(|s| s.id == id) {
                window.dispatch_action((spec.make)(), cx);
            }
        }
    }

    /// Leave the waiting state (Escape, a click, the timeout, or the key arrived).
    pub(crate) fn cancel_prefix(&mut self, cx: &mut Context<Self>) {
        // Every start and cancel moves the generation on, so an old timer cannot cancel a
        // newer press.
        self.prefix_generation += 1;
        if self.prefix_pending.take().is_some() {
            cx.notify();
        }
    }

    pub(crate) fn prefix_waiting(&self) -> bool {
        self.prefix_pending.is_some()
    }
}
