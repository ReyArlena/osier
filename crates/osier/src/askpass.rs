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

pub fn run(prompt: String) -> ExitCode {
    // Secrets are typed here; the prompt may say "passphrase" or "password". Mask unless the
    // prompt is clearly a yes/no question (host key confirmation).
    let masked = !prompt.to_lowercase().contains("(yes/no");
    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            // Bundled themes are not loaded in this mode; apply the dark one directly.
            if let Err(error) = gpui_kit::component::ThemeRegistry::global_mut(cx)
                .load_themes_from_str(crate::BUNDLED_THEMES[0])
            {
                tracing::warn!(%error, "bundled theme");
            }
            crate::apply_theme("Catppuccin Mocha", cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(460.), px(200.)), cx)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Osier".into()),
                    ..Default::default()
                }),
                app_id: Some("com.reyarlena.osier.askpass".into()),
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
