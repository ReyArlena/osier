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

/// A tiny window in the theme's colours: title bar, sidebar, text lines and accents.
///
/// One canvas painting about 20 rectangles, instead of a tree of styled divs: the settings
/// page shows 16 of these, and each was ~25 elements to lay out on every frame while the
/// dialog animated in.
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
    let radius = fallback.radius;
    canvas(
        |_, _, _| (),
        move |b, _, window, _| {
            const TITLE_H: f32 = 10.;
            const SIDEBAR_W: f32 = 40.;
            const BAR_H: f32 = 3.;
            const BAR_GAP: f32 = 4.;
            let inner_r = (radius - px(1.)).max(px(0.));
            let (x0, y0) = (b.origin.x + px(1.), b.origin.y + px(1.));
            let (w, h) = (b.size.width - px(2.), b.size.height - px(2.));
            let rect = |x: Pixels, y: Pixels, w: Pixels, h: Pixels| {
                Bounds::new(point(x, y), size(w.max(px(0.)), h.max(px(0.))))
            };
            let bar = |window: &mut Window, x: Pixels, y: Pixels, width: Pixels, color: Hsla| {
                window.paint_quad(fill(rect(x, y, width, px(BAR_H)), color).corner_radii(px(1.5)));
            };

            window.paint_quad(fill(b, bg).corner_radii(radius));
            // Title bar: a dot and a short line.
            let title_rect = rect(x0, y0, w, px(TITLE_H - 1.));
            window.paint_quad(fill(title_rect, title).corner_radii(Corners {
                top_left: inner_r,
                top_right: inner_r,
                ..Corners::default()
            }));
            window.paint_quad(fill(rect(x0, y0 + px(TITLE_H - 1.), w, px(1.)), border));
            let dot_y = y0 + px((TITLE_H - 1. - 4.) / 2.);
            window.paint_quad(
                fill(rect(x0 + px(4.), dot_y, px(4.), px(4.)), primary).corner_radii(px(2.)),
            );
            bar(window, x0 + px(11.), y0 + px(3.), (w - px(8.)) * 0.3, muted);

            // Sidebar: four lines, the first one selected.
            let body_y = y0 + px(TITLE_H);
            let body_h = h - px(TITLE_H);
            window.paint_quad(
                fill(rect(x0, body_y, px(SIDEBAR_W - 1.), body_h), sidebar).corner_radii(Corners {
                    bottom_left: inner_r,
                    ..Corners::default()
                }),
            );
            window.paint_quad(fill(
                rect(x0 + px(SIDEBAR_W - 1.), body_y, px(1.), body_h),
                border,
            ));
            let side_w = px(SIDEBAR_W - 9.);
            for (k, (part, color)) in [(0.8, primary), (0.6, muted), (0.75, muted), (0.5, muted)]
                .into_iter()
                .enumerate()
            {
                let y = body_y + px(4. + k as f32 * (BAR_H + BAR_GAP));
                bar(window, x0 + px(4.), y, side_w * part, color);
            }

            // Content: text lines, a green and a red change, a link.
            let cx0 = x0 + px(SIDEBAR_W + 5.);
            let cw = w - px(SIDEBAR_W + 10.);
            // Six lines with their padding, centred in the body as the old flex layout did.
            let block_h = px(5. * 2. + 6. * BAR_H + 5. * BAR_GAP);
            let top = body_y + ((body_h - block_h) / 2.).max(px(0.)) + px(5.);
            let line = |k: usize| top + px(k as f32 * (BAR_H + BAR_GAP));
            let text = fg.opacity(0.55);
            bar(window, cx0, line(0), cw * 0.45, fg);
            bar(window, cx0, line(1), cw * 0.9, text);
            bar(window, cx0, line(2), cw * 0.7, text);
            bar(window, cx0, line(3), cw * 0.3, success);
            bar(window, cx0 + cw * 0.3 + px(3.), line(3), cw * 0.2, danger);
            bar(window, cx0, line(4), cw * 0.8, text);
            bar(window, cx0, line(5), cw * 0.35, primary);

            // The border last, over the fills' edges.
            window.paint_quad(quad(
                b,
                radius,
                gpui_kit::transparent_black(),
                px(1.),
                border,
                BorderStyle::Solid,
            ));
        },
    )
    .h(px(84.))
    .w_full()
}

/// A theme colour (`#rrggbb` or `#rrggbbaa`), or `fallback` when unset or unreadable.
fn color(value: &Option<SharedString>, fallback: Hsla) -> Hsla {
    value
        .as_ref()
        .and_then(|v| Rgba::try_from(v.as_ref()).ok())
        .map(Hsla::from)
        .unwrap_or(fallback)
}
