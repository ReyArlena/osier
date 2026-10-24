//! The desktop's light/dark preference for System mode.
//!
//! gpui reports "prefer light", "no preference" and "no settings portal" all as light, so a
//! dark-preferring user on a desktop that states nothing would get a light theme. On Linux we
//! ask the XDG settings portal ourselves (the same `ashpd` crate gpui uses) and fall back to
//! the user's choice (`appearance.system_fallback`, dark by default) when there is no answer.

use osier_config::settings::SystemFallback;

/// What the desktop said, if anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum SystemScheme {
    /// Not asked yet (startup, or not Linux): use gpui's view on other platforms.
    #[default]
    Unknown,
    Dark,
    Light,
    /// The portal answered "no preference".
    NoPreference,
    /// No settings portal, or it failed.
    Unavailable,
}

/// Whether System mode shows the dark theme.
pub(crate) fn system_is_dark(
    scheme: SystemScheme,
    gpui_dark: bool,
    fallback: SystemFallback,
) -> bool {
    match scheme {
        SystemScheme::Dark => true,
        SystemScheme::Light => false,
        SystemScheme::NoPreference | SystemScheme::Unavailable => fallback == SystemFallback::Dark,
        // Until the portal answers on Linux, the fallback avoids a light flash (gpui starts
        // out light); elsewhere gpui's answer is the platform's real one.
        SystemScheme::Unknown if cfg!(target_os = "linux") => fallback == SystemFallback::Dark,
        SystemScheme::Unknown => gpui_dark,
    }
}

#[cfg(all(target_os = "linux", not(test)))]
impl crate::app::Osier {
    /// Read the portal's colour scheme and follow its changes, re-applying the theme.
    pub(crate) fn watch_system_scheme(&mut self, cx: &mut gpui_kit::Context<Self>) {
        use ashpd::desktop::settings::{ColorScheme, Settings};
        use futures_util::StreamExt as _;
        fn map(s: ColorScheme) -> SystemScheme {
            match s {
                ColorScheme::PreferDark => SystemScheme::Dark,
                ColorScheme::PreferLight => SystemScheme::Light,
                ColorScheme::NoPreference => SystemScheme::NoPreference,
            }
        }
        cx.spawn(async move |this, cx| {
            let set = |scheme: SystemScheme, cx: &mut gpui_kit::AsyncApp| {
                let _ = this.update(cx, |this, cx| {
                    if this.system_scheme != scheme {
                        this.system_scheme = scheme;
                        this.apply_appearance(cx);
                        cx.notify();
                    }
                });
            };
            let settings = match Settings::new().await {
                Ok(s) => s,
                Err(e) => {
                    tracing::debug!(error = %e, "no settings portal");
                    set(SystemScheme::Unavailable, cx);
                    return;
                }
            };
            match settings.color_scheme().await {
                Ok(s) => set(map(s), cx),
                Err(e) => {
                    tracing::debug!(error = %e, "portal has no colour scheme");
                    set(SystemScheme::Unavailable, cx);
                }
            }
            if let Ok(mut changes) = settings.receive_color_scheme_changed().await {
                while let Some(s) = changes.next().await {
                    set(map(s), cx);
                }
            }
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::{SystemScheme, system_is_dark};
    use osier_config::settings::SystemFallback::{Dark, Light};

    #[test]
    fn stated_preferences_win_and_silence_uses_the_fallback() {
        assert!(system_is_dark(SystemScheme::Dark, false, Light));
        assert!(!system_is_dark(SystemScheme::Light, true, Dark));
        assert!(system_is_dark(SystemScheme::NoPreference, false, Dark));
        assert!(!system_is_dark(SystemScheme::NoPreference, true, Light));
        assert!(system_is_dark(SystemScheme::Unavailable, false, Dark));
        assert!(!system_is_dark(SystemScheme::Unavailable, false, Light));
    }
}
