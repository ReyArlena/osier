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

/// Shortest and longest animation length the settings slider offers, in milliseconds.
/// Off is the Animations setting's job, so the slider starts above zero.
pub(crate) const ANIMATION_MS_MIN: u32 = 60;
pub(crate) const ANIMATION_MS_MAX: u32 = 400;

/// gpui-kit's motion timings with the dialog fade at `ms`, everything else scaled to match.
/// The springs are rebuilt from gpui-kit's defaults (`MotionTokens::default`, 0.9.0): `Spring`
/// has no getters to scale an existing one.
pub(crate) fn motion_tokens(ms: u32) -> gpui_kit::component::theme::MotionTokens {
    use gpui_kit::base::Spring;
    use std::time::Duration;
    let mut tokens = gpui_kit::component::theme::MotionTokens::default();
    let ms = ms.clamp(ANIMATION_MS_MIN, ANIMATION_MS_MAX);
    let factor = ms as f32 / tokens.duration_normal.as_millis().max(1) as f32;
    let scale = |d: Duration| d.mul_f32(factor);
    tokens.duration_fast = scale(tokens.duration_fast);
    tokens.duration_normal = Duration::from_millis(u64::from(ms));
    tokens.duration_slow = scale(tokens.duration_slow);
    tokens.spring_control = Spring::new(scale(Duration::from_millis(180)));
    tokens.spring_move = Spring::new(scale(Duration::from_millis(280)))
        .with_damping(0.85)
        .with_epsilon(0.1);
    tokens
}

impl crate::app::Osier {
    /// Turn gpui's reduced motion on or off for the current setting (gpui redraws every
    /// window when it changes), and set gpui-kit's motion timings from the animation length.
    pub(crate) fn apply_motion(&self, cx: &mut gpui_kit::App) {
        cx.set_reduce_motion(reduce_motion(
            self.settings.appearance.animations,
            self.system_reduced_motion,
        ));
        let tokens = motion_tokens(self.settings.appearance.animation_ms);
        gpui_kit::component::theme::Theme::update(cx, |theme| theme.motion = tokens);
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
    use super::{ANIMATION_MS_MAX, ANIMATION_MS_MIN, motion_tokens, reduce_motion};
    use std::time::Duration;

    #[test]
    fn animation_length_sets_the_fade_and_scales_the_rest() {
        let d = motion_tokens(180);
        assert_eq!(d.duration_normal, Duration::from_millis(180));
        assert_eq!(d.duration_fast, Duration::from_millis(120));
        assert_eq!(d.duration_slow, Duration::from_millis(280));
        let half = motion_tokens(90);
        assert_eq!(half.duration_normal, Duration::from_millis(90));
        assert_eq!(half.duration_fast, Duration::from_millis(60));
        assert_eq!(half.duration_slow, Duration::from_millis(140));
        assert_eq!(
            motion_tokens(0).duration_normal,
            Duration::from_millis(u64::from(ANIMATION_MS_MIN))
        );
        assert_eq!(
            motion_tokens(10_000).duration_normal,
            Duration::from_millis(u64::from(ANIMATION_MS_MAX))
        );
    }
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
