//! Key bindings as the user sees them: loading `keymap.toml`, rebinding, recording and
//! resetting keys. The registry is `crate::keymap`.

use gpui_kit::*;

use super::Osier;

impl Osier {
    /// Apply `keymap.toml` (startup, or after it changed). Problems are logged and returned.
    pub(crate) fn load_keymap(&mut self, cx: &mut App) -> Vec<String> {
        let file = match osier_config::keymap::KeymapFile::load(&self.paths) {
            Ok(f) => f,
            Err(error) => {
                tracing::warn!(%error, "keymap.toml is invalid, keeping the current keys");
                return vec![format!("keymap.toml: {error}")];
            }
        };
        self.apply_keymap_file(file, cx)
    }

    fn apply_keymap_file(
        &mut self,
        file: osier_config::keymap::KeymapFile,
        cx: &mut App,
    ) -> Vec<String> {
        crate::keymap::apply(&file, cx);
        self.keymap_applied = file;
        if let Some(dms) = &self.dms_dir {
            match crate::dms_cheatsheet::write(dms, crate::keymap::Keymap::global(cx)) {
                Ok(true) => tracing::debug!(dir = %dms.display(), "wrote the DMS cheatsheet"),
                Ok(false) => {}
                Err(error) => tracing::warn!(%error, "could not write the DMS cheatsheet"),
            }
        }
        let problems = crate::keymap::Keymap::global(cx).problems.clone();
        for p in &problems {
            tracing::warn!(problem = %p, "keymap.toml");
        }
        problems
    }

    /// `keymap.toml` changed on disk. Our own saves land here too; they change nothing.
    pub(crate) fn reload_keymap(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use gpui_kit::component::WindowExt as _;
        use gpui_kit::component::notification::Notification;
        if osier_config::keymap::KeymapFile::load(&self.paths)
            .ok()
            .as_ref()
            == Some(&self.keymap_applied)
        {
            return;
        }
        let problems = self.load_keymap(cx);
        if problems.is_empty() {
            window.push_notification("Key bindings reloaded from keymap.toml", cx);
        } else {
            window.push_notification(
                Notification::warning(problems.join("\n")).title("keymap.toml has problems"),
                cx,
            );
        }
        cx.notify();
    }

    /// Set an action's keys (Settings > Keymap). Matching the defaults removes the override.
    /// A `keymap.toml` that does not parse is never overwritten.
    pub(crate) fn set_action_keys(
        &mut self,
        id: &'static str,
        keys: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use gpui_kit::component::WindowExt as _;
        use gpui_kit::component::notification::Notification;
        let mut file = match osier_config::keymap::KeymapFile::load(&self.paths) {
            Ok(f) => f,
            Err(error) => {
                window.push_notification(
                    Notification::error(error.to_string())
                        .title("keymap.toml has an error; fix it first"),
                    cx,
                );
                return;
            }
        };
        let defaults: Vec<String> = crate::keymap::actions()
            .iter()
            .find(|a| a.id == id)
            .map(|a| a.defaults.iter().map(|k| k.to_string()).collect())
            .unwrap_or_default();
        if keys == defaults {
            file.bindings.remove(id);
        } else {
            file.bindings.insert(id.to_string(), keys);
        }
        if let Err(error) = file.save(&self.paths) {
            window.push_notification(
                Notification::error(error.to_string()).title("Could not save keymap.toml"),
                cx,
            );
            return;
        }
        self.apply_keymap_file(file, cx);
        cx.notify();
    }

    /// Open `keymap.toml` in the editor, creating it with an explanatory header if missing.
    pub(crate) fn edit_keymap_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let path = self.paths.keymap_file();
        if !path.exists() {
            let header = "# Osier key bindings: only your changes to the defaults.\n\
                # Example:\n# [bindings]\n# refresh = [\"f5\"]\n# open_settings = []   # unbound\n\n[bindings]\n";
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(error) = std::fs::write(&path, header) {
                tracing::warn!(%error, "could not create keymap.toml");
            }
        }
        self.open_externally(&path, window, cx);
    }

    /// Reset every binding to the defaults (removes `keymap.toml`).
    pub(crate) fn reset_keymap(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use gpui_kit::component::WindowExt as _;
        let file = osier_config::keymap::KeymapFile::default();
        if let Err(error) = file.save(&self.paths) {
            window.push_notification(
                gpui_kit::component::notification::Notification::error(error.to_string())
                    .title("Could not reset keymap"),
                cx,
            );
            return;
        }
        self.apply_keymap_file(file, cx);
        cx.notify();
    }

    /// Record the next key combination for `id` (the "+" on the Keymap page). The interceptor
    /// runs before key bindings, so the keys are captured instead of triggering anything.
    /// Escape cancels.
    pub(crate) fn start_key_recording(&mut self, id: &'static str, cx: &mut Context<Self>) {
        let weak = cx.weak_entity();
        let sub = cx.intercept_keystrokes(move |event, window, cx| {
            let Some(keys) = crate::keymap::keystroke_string(&event.keystroke) else {
                return; // a modifier on its own: keep waiting
            };
            cx.stop_propagation();
            let _ = weak.update(cx, |this, cx| this.finish_key_recording(keys, window, cx));
        });
        self.keymap_recording = Some((id, sub));
        cx.notify();
    }

    fn finish_key_recording(&mut self, keys: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some((id, _sub)) = self.keymap_recording.take() else {
            return;
        };
        cx.notify();
        if keys == "escape" {
            return;
        }
        let keymap = crate::keymap::Keymap::global(cx);
        let refused = crate::keymap::refused(keymap, id, &keys);
        if !refused.is_empty() {
            self.refuse_key(&keys, &refused, window, cx);
            return;
        }
        let mut current = keymap.keys_for(id).to_vec();
        if !current.contains(&keys) {
            current.push(keys);
            self.set_action_keys(id, current, window, cx);
        }
    }

    /// A key that would clash with another binding where Osier does not allow it.
    fn refuse_key(
        &mut self,
        keys: &str,
        others: &[&str],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use gpui_kit::component::WindowExt as _;
        use gpui_kit::component::notification::Notification;
        window.push_notification(
            Notification::warning(
                "Context-menu keys and the prefix key cannot share a key with another binding.",
            )
            .title(format!(
                "{} is already used by {}",
                crate::keymap::pretty(keys),
                others.join(", ")
            )),
            cx,
        );
    }

    /// Switch one key of an action between no prefix, prefix only and either.
    pub(crate) fn set_key_mode(
        &mut self,
        id: &'static str,
        key: &str,
        mode: crate::keymap::KeyMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::keymap::{Keymap, refused, split_mode, with_mode};
        let keymap = Keymap::global(cx);
        let new = with_mode(mode, split_mode(key).1);
        if new == key {
            return;
        }
        let clash = refused(keymap, id, &new);
        if !clash.is_empty() {
            self.refuse_key(&new, &clash, window, cx);
            return;
        }
        let keys: Vec<String> = keymap
            .keys_for(id)
            .iter()
            .map(|k| if k == key { new.clone() } else { k.clone() })
            .collect();
        self.set_action_keys(id, keys, window, cx);
    }

    pub(crate) fn remove_action_key(
        &mut self,
        id: &'static str,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let keys: Vec<String> = crate::keymap::Keymap::global(cx)
            .keys_for(id)
            .iter()
            .filter(|k| k.as_str() != key)
            .cloned()
            .collect();
        self.set_action_keys(id, keys, window, cx);
    }
}
