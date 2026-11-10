//! Appearance: themes and theme mode, Dux Shell sync, the tooltip delay.

use std::time::Duration;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::*;
use osier_config::settings::ThemeMode;

use super::{Osier, TOOLTIP_DELAY_MAX_MS};

impl Osier {
    /// Install the tooltip delay for every tooltip in the app (takes effect at once).
    pub(crate) fn apply_tooltip_delay(&self, cx: &mut App) {
        let ms = self
            .settings
            .appearance
            .tooltip_delay_ms
            .min(TOOLTIP_DELAY_MAX_MS);
        gpui_kit::base::TooltipDefaults::global(cx)
            .with_show_delay(Duration::from_millis(u64::from(ms)))
            .install(cx);
        // A global the views read while rendering: redraw past the main view's cache.
        cx.refresh_windows();
    }

    pub(crate) fn apply_appearance(&mut self, cx: &mut App) {
        self.apply_tooltip_delay(cx);
        self.apply_motion(cx);
        use crate::themes::{DUX_DARK, DUX_LIGHT, ThemeStore};
        let a = &self.settings.appearance;
        let store = ThemeStore::global(cx);
        let dux = a.dux_shell_sync.unwrap_or(true) && store.dux_available();
        let dark = match a.mode {
            ThemeMode::Dark => true,
            ThemeMode::Light => false,
            ThemeMode::System => match store.dux_mode() {
                Some(mode) if dux => mode == osier_theme::Mode::Dark,
                _ => crate::system_scheme::system_is_dark(
                    self.system_scheme,
                    matches!(
                        cx.window_appearance(),
                        WindowAppearance::Dark | WindowAppearance::VibrantDark
                    ),
                    a.system_fallback,
                ),
            },
        };
        let name = match (dux, dark) {
            (true, true) => DUX_DARK,
            (true, false) => DUX_LIGHT,
            (false, true) => a.dark_theme.as_str(),
            (false, false) => a.light_theme.as_str(),
        };
        if !crate::themes::apply(name, a, cx) {
            tracing::warn!(name, "theme not found, using the bundled default");
            let fallback = if dark {
                "Catppuccin Mocha"
            } else {
                "Catppuccin Latte"
            };
            crate::themes::apply(fallback, a, cx);
        }
    }

    pub(crate) fn set_theme_mode(&mut self, mode: ThemeMode, cx: &mut Context<Self>) {
        self.settings.appearance.mode = mode;
        self.apply_appearance(cx);
        self.save_settings();
        cx.notify();
    }

    /// Pick a theme for its own mode (dark themes replace the dark choice, light the light).
    /// Picking an explicit theme turns Dux Shell sync off.
    pub(crate) fn set_theme(&mut self, name: &str, cx: &mut Context<Self>) {
        let Some(config) = crate::themes::ThemeStore::global(cx).get(name) else {
            return;
        };
        // Picking a theme of the other mode switches to that mode explicitly (System mode
        // would otherwise keep showing the old one).
        let showing_dark = cx.theme().mode.is_dark();
        let a = &mut self.settings.appearance;
        if config.mode.is_dark() {
            a.dark_theme = name.to_string();
            if !showing_dark {
                a.mode = ThemeMode::Dark;
            }
        } else {
            a.light_theme = name.to_string();
            if showing_dark {
                a.mode = ThemeMode::Light;
            }
        }
        a.dux_shell_sync = Some(false);
        self.apply_appearance(cx);
        self.save_settings();
        cx.notify();
    }

    /// Whether Dux Shell colours are followed: the user's choice, else on when detected.
    pub(crate) fn dux_sync_on(&self, cx: &App) -> bool {
        crate::themes::ThemeStore::global(cx).dux_available()
            && self.settings.appearance.dux_shell_sync.unwrap_or(true)
    }

    /// Turn Dux Shell sync on or off. Turning it on also sets the mode to System, so the
    /// shell's light/dark decides; the user can still pick Dark or Light afterwards.
    pub(crate) fn set_dux_sync(&mut self, on: bool, cx: &mut Context<Self>) {
        self.settings.appearance.dux_shell_sync = Some(on);
        if on {
            self.settings.appearance.mode = ThemeMode::System;
        }
        self.apply_appearance(cx);
        self.save_settings();
        cx.notify();
    }

    /// Reload themes when a theme file (or the Dux Shell colours) changed on disk: read and
    /// parsed in the background, then applied. Returns false: the redraw comes with the result.
    pub(super) fn poll_themes(&mut self, cx: &mut Context<Self>) -> bool {
        if !crate::themes::ThemeStore::global(cx).take_dirty() {
            return false;
        }
        let (user_dir, dux) = crate::themes::ThemeStore::global(cx).sources();
        self.in_background_cx(
            crate::background::READING_THEMES,
            move || crate::themes::read_themes(&user_dir, dux.as_deref()),
            |this, loaded, cx| {
                crate::themes::ThemeStore::global_mut(cx).install(loaded);
                this.apply_appearance(cx);
                cx.notify();
            },
            cx,
        );
        false
    }

    pub(crate) fn open_themes_dir(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dir = self.paths.themes_dir();
        let create = dir.clone();
        self.in_background_user(
            crate::background::OPENING_FOLDER,
            move || {
                if let Err(error) = std::fs::create_dir_all(&create) {
                    tracing::warn!(%error, "could not create the themes directory");
                }
            },
            move |this, (), window, cx| this.open_externally(&dir, window, cx),
            window,
            cx,
        );
    }

    pub(crate) fn toggle_theme(&mut self, cx: &mut Context<Self>) {
        let dark = cx.theme().mode.is_dark();
        self.settings.appearance.mode = if dark {
            ThemeMode::Light
        } else {
            ThemeMode::Dark
        };
        self.apply_appearance(cx);
        self.save_settings();
        cx.notify();
    }
}
