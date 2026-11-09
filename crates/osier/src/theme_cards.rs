//! The theme picker on the settings Appearance page: a card per theme with a small preview
//! drawn in that theme's own colours (title bar, sidebar, text lines, accents).

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::theme::ThemeConfig;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app::Osier;
use crate::test_ids::TestId as _;
use crate::themes::ThemeStore;

/// Cards for every dark (or light) theme. Clicking one makes it the dark (light) theme.
/// Disabled while Dux Shell colours are followed.
pub(crate) fn theme_picker(
    view: &Entity<Osier>,
    dark: bool,
    window: &Window,
    cx: &App,
) -> AnyElement {
    let o = view.read(cx);
    let store = ThemeStore::global(cx);
    let current = if dark {
        o.settings.appearance.dark_theme.clone()
    } else {
        o.settings.appearance.light_theme.clone()
    };
    let disabled = o.dux_sync_on(cx);
    let mut cards = div().grid().grid_cols(columns(window)).gap_3().w_full();
    for name in store.names(dark) {
        let Some(config) = store.get(&name) else {
            continue;
        };
        let selected = name.as_ref() == current;
        let v = view.clone();
        let pick = name.to_string();
        cards = cards.child(
            v_flex()
                .id(SharedString::from(format!("theme-card-{name}")))
                .test_id(|| format!("theme-card:{name}"))
                .min_w_0()
                .p_1p5()
                .gap_1p5()
                .rounded(cx.theme().radius_lg)
                .border_2()
                .border_color(if selected {
                    cx.theme().primary
                } else {
                    cx.theme().transparent
                })
                .when(selected, |this| this.bg(cx.theme().list_active))
                .when(disabled, |this| this.opacity(0.45))
                .when(!disabled, |this| {
                    this.cursor_pointer()
                        .hover(|this| this.bg(cx.theme().list_hover))
                        .on_click(move |_, _, cx| {
                            v.update(cx, |o, cx| {
                                let a = &mut o.settings.appearance;
                                if dark {
                                    a.dark_theme = pick.clone();
                                } else {
                                    a.light_theme = pick.clone();
                                }
                                o.settings_changed(cx);
                            })
                        })
                })
                .child(preview(&config, cx))
                .child(
                    h_flex()
                        .gap_1()
                        .items_center()
                        .justify_between()
                        .child(div().text_sm().truncate().child(name.clone()))
                        .when(selected, |this| {
                            this.child(
                                Icon::new(Lucide::Check)
                                    .small()
                                    .text_color(cx.theme().primary),
                            )
                        }),
                ),
        );
    }
    cards.into_any_element()
}

/// Cards per row, so they fill the settings card edge to edge. The page width is not known
/// while building, so it is worked out from the window the way the settings dialog sizes
/// itself (90% of the window, at most 1100px), less its sidebar and paddings.
fn columns(window: &Window) -> u16 {
    const MIN_CARD: f32 = 160.;
    const GAP: f32 = 12.;
    let dialog = (f32::from(window.viewport_size().width) * 0.9).min(1100.);
    let available = dialog - 250. - 2. * 16. - 2. * 16.;
    ((available + GAP) / (MIN_CARD + GAP)).floor().clamp(2., 5.) as u16
}

/// A tiny window in the theme's colours.
fn preview(config: &ThemeConfig, cx: &App) -> impl IntoElement {
    let c = &config.colors;
    let fallback = cx.theme();
    let bg = color(&c.background, fallback.background);
    let fg = color(&c.foreground, fallback.foreground);
    let sidebar = color(&c.sidebar, bg);
    let title = color(&c.title_bar, sidebar);
    let primary = color(&c.primary, fallback.primary);
    let muted = color(&c.muted_foreground, fg.opacity(0.6));
    let border = color(&c.border, fallback.border);
    let success = color(&c.success, fallback.success);
    let danger = color(&c.danger, fallback.danger);
    let bar = |color: Hsla, width: f32| {
        div()
            .h(px(3.))
            .w(relative(width))
            .rounded(px(1.5))
            .bg(color)
    };
    v_flex()
        .h(px(84.))
        .w_full()
        .overflow_hidden()
        .rounded(cx.theme().radius)
        .border_1()
        .border_color(border)
        .bg(bg)
        .child(
            h_flex()
                .h(px(10.))
                .px_1()
                .gap(px(3.))
                .items_center()
                .bg(title)
                .border_b_1()
                .border_color(border)
                .child(div().size(px(4.)).rounded_full().bg(primary))
                .child(bar(muted, 0.3)),
        )
        .child(
            h_flex()
                .flex_1()
                .child(
                    v_flex()
                        .w(px(40.))
                        .h_full()
                        .p_1()
                        .gap(px(4.))
                        .bg(sidebar)
                        .border_r_1()
                        .border_color(border)
                        .child(bar(primary, 0.8))
                        .child(bar(muted, 0.6))
                        .child(bar(muted, 0.75))
                        .child(bar(muted, 0.5)),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .p(px(5.))
                        .gap(px(4.))
                        .child(bar(fg, 0.45))
                        .child(bar(fg.opacity(0.55), 0.9))
                        .child(bar(fg.opacity(0.55), 0.7))
                        .child(
                            h_flex()
                                .gap(px(3.))
                                .child(bar(success, 0.3))
                                .child(bar(danger, 0.2)),
                        )
                        .child(bar(fg.opacity(0.55), 0.8))
                        .child(bar(primary, 0.35)),
                ),
        )
}

/// A theme colour (`#rrggbb` or `#rrggbbaa`), or `fallback` when unset or unreadable.
fn color(value: &Option<SharedString>, fallback: Hsla) -> Hsla {
    value
        .as_ref()
        .and_then(|v| Rgba::try_from(v.as_ref()).ok())
        .map(Hsla::from)
        .unwrap_or(fallback)
}
