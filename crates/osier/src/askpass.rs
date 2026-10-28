//! Minimal password window used as SSH_ASKPASS / GIT_ASKPASS. Prints the answer to stdout
//! and exits 0, or exits 1 when cancelled.

use std::io::Write;
use std::process::ExitCode;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::*;

struct Askpass {
    prompt: SharedString,
    input: Entity<InputState>,
    _subscription: Subscription,
}

impl Askpass {
    fn submit(&self, cx: &mut Context<Self>) {
        let value = self.input.read(cx).value().to_string();
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{value}");
        let _ = out.flush();
        cx.quit();
    }
}

impl Render for Askpass {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .p_5()
            .gap_4()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(div().font_medium().child("Osier"))
            .child(div().text_sm().child(self.prompt.clone()))
            .child(Input::new(&self.input))
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("cancel")
                            .outline()
                            .small()
                            .label("Cancel")
                            .on_click(|_, _, _| std::process::exit(1)),
                    )
                    .child(
                        Button::new("ok")
                            .primary()
                            .small()
                            .label("OK")
                            .on_click(cx.listener(|this, _, _, cx| this.submit(cx))),
                    ),
            )
    }
}

/// The askpass window's own Wayland app id, so a compositor rule can target it (float it,
/// say). `packaging/com.reyarlena.osier.askpass.desktop` matches it and points at the main
/// icon by name, which is how the window gets Osier's icon.
pub(crate) const ASKPASS_APP_ID: &str = "com.reyarlena.osier.askpass";

pub fn run(prompt: String) -> ExitCode {
    // Secrets are typed here; the prompt may say "passphrase" or "password". Mask unless the
    // prompt is clearly a yes/no question (host key confirmation).
    let masked = !prompt.to_lowercase().contains("(yes/no");
    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            // Follow the user's theme choice; no file watching for a short-lived prompt.
            if let Ok(paths) = osier_config::Paths::discover() {
                let settings = osier_config::Settings::load(&paths).unwrap_or_default();
                crate::themes::ThemeStore::ensure(paths.themes_dir(), false, cx);
                let a = &settings.appearance;
                let dark = a.mode != osier_config::settings::ThemeMode::Light;
                let name = if dark { &a.dark_theme } else { &a.light_theme };
                if !crate::themes::apply(name, a, cx) {
                    let fallback = if dark {
                        "Catppuccin Mocha"
                    } else {
                        "Catppuccin Latte"
                    };
                    crate::themes::apply(fallback, a, cx);
                }
            }
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(460.), px(200.)), cx)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Osier".into()),
                    ..Default::default()
                }),
                app_id: Some(ASKPASS_APP_ID.into()),
                ..Default::default()
            };
            let prompt = prompt.clone();
            let opened = gpui_kit::open_window(options, cx, move |window, cx| {
                cx.new(|cx| {
                    let input = cx.new(|cx| InputState::new(window, cx).masked(masked));
                    input.read(cx).focus_handle(cx).focus(window, cx);
                    let subscription = cx.subscribe_in(
                        &input,
                        window,
                        |this: &mut Askpass, _, ev: &InputEvent, _, cx| {
                            if matches!(ev, InputEvent::PressEnter { .. }) {
                                this.submit(cx);
                            }
                        },
                    );
                    Askpass {
                        prompt: prompt.clone().into(),
                        input,
                        _subscription: subscription,
                    }
                })
            });
            if opened.is_err() {
                std::process::exit(1);
            }
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    std::process::exit(1);
                }
            })
            .detach();
            cx.activate(true);
        });
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    /// Each window's app id needs a desktop entry of the same name for its icon, and both
    /// entries name the one themed icon, so a new icon reaches every window.
    #[test]
    fn every_app_id_has_a_desktop_entry_with_the_main_icon() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packaging");
        for id in [crate::APP_ID, super::ASKPASS_APP_ID] {
            let entry = std::fs::read_to_string(dir.join(format!("{id}.desktop")))
                .unwrap_or_else(|e| panic!("no desktop entry for {id}: {e}"));
            assert!(
                entry.lines().any(|l| l == format!("StartupWMClass={id}")),
                "{id}"
            );
            assert!(
                entry
                    .lines()
                    .any(|l| l == format!("Icon={}", crate::APP_ID)),
                "{id} must use the main icon"
            );
        }
    }
}
