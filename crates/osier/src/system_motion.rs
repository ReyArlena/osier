//! Whether the interface animates: `appearance.animations`, and for `system` the desktop's
//! reduced-motion preference.
//!
//! gpui-base follows the portal's own key (`org.freedesktop.appearance` `reduced-motion`) by
//! itself, and stops once the app sets the value, which Osier always does: the setting can
//! override the desktop, and portals that lack that fairly new key are asked for GNOME's
//! `enable-animations` instead (the GTK and GNOME portals pass it through). With neither,
//! the interface animates.

use osier_config::settings::Animations;

/// Whether animations are turned off. `system` is what the desktop said, `None` when it said
/// nothing (or has not answered yet).
pub(crate) fn reduce_motion(setting: Animations, system: Option<bool>) -> bool {
    match setting {
        Animations::On => false,
        Animations::Off => true,
        Animations::System => system.unwrap_or(false),
    }
}

impl crate::app::Osier {
    /// Turn gpui's reduced motion on or off for the current setting. gpui redraws every
    /// window when it changes.
    pub(crate) fn apply_motion(&self, cx: &mut gpui_kit::App) {
        cx.set_reduce_motion(reduce_motion(
            self.settings.appearance.animations,
            self.system_reduced_motion,
        ));
    }
}

#[cfg(all(target_os = "linux", not(test)))]
impl crate::app::Osier {
    /// Read the desktop's reduced-motion preference and follow its changes.
    pub(crate) fn watch_system_motion(&mut self, cx: &mut gpui_kit::Context<Self>) {
        use ashpd::desktop::settings::{ReducedMotion, Settings};
        use futures_util::StreamExt as _;
        const APPEARANCE: &str = "org.freedesktop.appearance";
        const REDUCED_MOTION: &str = "reduced-motion";
        const GNOME: &str = "org.gnome.desktop.interface";
        const ENABLE_ANIMATIONS: &str = "enable-animations";
        cx.spawn(async move |this, cx| {
            let set = |reduce: Option<bool>, cx: &mut gpui_kit::AsyncApp| {
                let _ = this.update(cx, |this, cx| {
                    if this.system_reduced_motion != reduce {
                        this.system_reduced_motion = reduce;
                        this.apply_motion(cx);
                    }
                });
            };
            let settings = match Settings::new().await {
                Ok(s) => s,
                Err(e) => {
                    tracing::debug!(error = %e, "no settings portal");
                    return;
                }
            };
            // Once the portal has its own key, GNOME's is not consulted.
            let standard = settings.reduced_motion().await.ok();
            let initial = match standard {
                Some(m) => Some(m == ReducedMotion::ReducedMotion),
                None => settings
                    .read::<bool>(GNOME, ENABLE_ANIMATIONS)
                    .await
                    .ok()
                    .map(|on| !on),
            };
            set(initial, cx);
            let Ok(mut changes) = settings.receive_setting_changed().await else {
                return;
            };
            while let Some(change) = changes.next().await {
                let reduce = match (change.namespace(), change.key()) {
                    (APPEARANCE, REDUCED_MOTION) => change
                        .value()
                        .try_clone()
                        .ok()
                        .and_then(|v| ReducedMotion::try_from(v).ok())
                        .map(|m| m == ReducedMotion::ReducedMotion),
                    (GNOME, ENABLE_ANIMATIONS) if standard.is_none() => {
                        change.value().downcast_ref::<bool>().ok().map(|on| !on)
                    }
                    _ => continue,
                };
                if reduce.is_some() {
                    set(reduce, cx);
                }
            }
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::reduce_motion;
    use osier_config::settings::Animations;

    #[test]
    fn explicit_choices_win_and_system_follows_the_desktop() {
        assert!(!reduce_motion(Animations::On, Some(true)));
        assert!(reduce_motion(Animations::Off, Some(false)));
        assert!(reduce_motion(Animations::Off, None));
        assert!(reduce_motion(Animations::System, Some(true)));
        assert!(!reduce_motion(Animations::System, Some(false)));
        assert!(
            !reduce_motion(Animations::System, None),
            "silence means animate"
        );
    }
}
