//! Appearance page: theme, sidebar, tooltips, lists and fonts.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::setting::{
    NumberFieldOptions, SettingField, SettingGroup, SettingItem, SettingPage,
};
use gpui_kit::component::{Icon, h_flex};
use gpui_kit::*;
use osier_config::settings::{Animations, ThemeMode};

use crate::app::Osier;
use crate::themes::ThemeStore;

use super::{get, opt_string, set};

/// What Animations does, and what "Follow the desktop" currently means.
fn motion_description(system_reduced_motion: Option<bool>) -> String {
    let desktop = match system_reduced_motion {
        Some(true) => "Your desktop asks for reduced motion, so following it means off.",
        Some(false) => "Your desktop allows animations, so following it means on.",
        None => "Your desktop does not say, so following it means on.",
    };
    format!(
        "Dialogs, menus and toasts animate in, and spinners turn. Off shows them at \
         once and keeps spinners still, which also helps on a slow machine. {desktop}"
    )
}

pub(super) fn appearance_page(view: &Entity<Osier>, cx: &App) -> SettingPage {
    let store = ThemeStore::global(cx);
    let dux_available = store.dux_available();
    SettingPage::new("Appearance")
        .description("Theme, colours, fonts and how lists and tooltips look.")
        .icon(Icon::new(Lucide::Palette))
        .default_open(true)
        .resettable(true)
        .group(
            SettingGroup::new()
                .title("Theme")
                .items(vec![
                SettingItem::new(
                    "Follow Dux Shell colours",
                    SettingField::switch(
                        {
                            let v = view.clone();
                            move |cx: &App| v.read(cx).dux_sync_on(cx)
                        },
                        {
                            let v = view.clone();
                            move |on: bool, cx: &mut App| {
                                v.update(cx, |o, cx| o.set_dux_sync(on, cx))
                            }
                        },
                    )
                    .default_value(dux_available),
                )
                .description(if dux_available {
                    "Take the theme from DankMaterialShell's colours and follow them live. \
                     Turning it on sets the mode to System; the theme pickers below are off \
                     while it is on."
                } else {
                    "Not detected: no dms-colors.json in the cache directory."
                })
                .disabled(!dux_available),
                SettingItem::new(
                    "Mode",
                    SettingField::dropdown(
                        vec![
                            ("dark".into(), "Dark".into()),
                            ("light".into(), "Light".into()),
                            ("system".into(), "System".into()),
                        ],
                        get(view, |o| {
                            match o.settings.appearance.mode {
                                ThemeMode::Dark => "dark",
                                ThemeMode::Light => "light",
                                ThemeMode::System => "system",
                            }
                            .into()
                        }),
                        set(view, |o, v: SharedString, _| {
                            o.settings.appearance.mode = match v.as_ref() {
                                "light" => ThemeMode::Light,
                                "system" => ThemeMode::System,
                                _ => ThemeMode::Dark,
                            }
                        }),
                    )
                    .default_value("dark".to_string()),
                )
                .description("System follows the desktop, or Dux Shell when its sync is on."),
                SettingItem::new(
                    "When the desktop has no preference",
                    SettingField::dropdown(
                        vec![("dark".into(), "Dark".into()), ("light".into(), "Light".into())],
                        get(view, |o| {
                            match o.settings.appearance.system_fallback {
                                osier_config::settings::SystemFallback::Dark => "dark",
                                osier_config::settings::SystemFallback::Light => "light",
                            }
                            .into()
                        }),
                        set(view, |o, v: SharedString, _| {
                            o.settings.appearance.system_fallback = if v.as_ref() == "light" {
                                osier_config::settings::SystemFallback::Light
                            } else {
                                osier_config::settings::SystemFallback::Dark
                            }
                        }),
                    )
                    .default_value("dark".to_string()),
                )
                .description(
                    "System mode uses this when the desktop states no light or dark preference, \
                     or has no settings portal.",
                ),
                ]),
        )
        .group(
            SettingGroup::new()
                .title("Dark theme")
                .description("Used while the mode is dark.")
                .item(SettingItem::render({
                    let v = view.clone();
                    move |_, window, cx| {
                        crate::theme_cards::theme_picker(&v, true, window, cx)
                    }
                })),
        )
        .group(
            SettingGroup::new()
                .title("Light theme")
                .description("Used while the mode is light.")
                .item(SettingItem::render({
                    let v = view.clone();
                    move |_, window, cx| {
                        crate::theme_cards::theme_picker(&v, false, window, cx)
                    }
                })),
        )
        .group(
            SettingGroup::new()
                .title("Contrast")
                .items(vec![
                SettingItem::new(
                    "Higher contrast",
                    SettingField::switch(
                        get(view, |o| o.settings.appearance.high_contrast),
                        set(view, |o, v: bool, _| o.settings.appearance.high_contrast = v),
                    )
                    .default_value(false),
                )
                .description(
                    "Selections, hovers and the sidebar highlight in a stronger tint of the \
                     theme's main colour, like the switches.",
                ),
                SettingItem::new(
                    "Readable syntax colours",
                    SettingField::switch(
                        get(view, |o| o.settings.appearance.syntax_contrast),
                        set(view, |o, v: bool, _| o.settings.appearance.syntax_contrast = v),
                    )
                    .default_value(true),
                )
                .description(
                    "Lift syntax colours that sit too close to the editor background (comments \
                     above all) until they read, whatever the theme. Hue stays, so the theme \
                     still looks like itself. Off: the theme's colours as they are.",
                ),
                ]),
        )
        .group(
            SettingGroup::new().title("Motion").item(
                SettingItem::new(
                    "Animations",
                    SettingField::dropdown(
                        vec![
                            ("system".into(), "Follow the desktop".into()),
                            ("on".into(), "On".into()),
                            ("off".into(), "Off".into()),
                        ],
                        get(view, |o| {
                            match o.settings.appearance.animations {
                                Animations::System => "system",
                                Animations::On => "on",
                                Animations::Off => "off",
                            }
                            .into()
                        }),
                        set(view, |o, v: SharedString, _| {
                            o.settings.appearance.animations = match v.as_ref() {
                                "on" => Animations::On,
                                "off" => Animations::Off,
                                _ => Animations::System,
                            }
                        }),
                    )
                    .default_value("system".to_string()),
                )
                .description(motion_description(view.read(cx).system_reduced_motion)),
            ),
        )
        .group(
            SettingGroup::new()
                .title("Status bar")
                .items(vec![
                SettingItem::new(
                    "Show background tasks",
                    SettingField::switch(
                        get(view, |o| o.settings.appearance.show_background_tasks),
                        set(view, |o, v: bool, _| {
                            o.settings.appearance.show_background_tasks = v
                        }),
                    )
                    .default_value(false),
                )
                .description(
                    "A spinner in the status bar while Osier works quietly: re-reading a project after \
                     a change, checking project status, loading GitHub lists, diffs and branch \
                     lookups. Fetch, pull, push, clone and create always show.",
                ),
                ]),
        )
        .group(
            SettingGroup::new().title("Sidebar").item(
                SettingItem::new(
                    "Uncommitted changes",
                    SettingField::dropdown(
                        vec![
                            ("icon".into(), "Icon colour".into()),
                            ("dot".into(), "Dot".into()),
                        ],
                        get(view, |o| {
                            match o.settings.appearance.changes_indicator {
                                osier_config::settings::ChangesIndicator::Icon => "icon",
                                osier_config::settings::ChangesIndicator::Dot => "dot",
                            }
                            .into()
                        }),
                        set(view, |o, v: SharedString, _| {
                            o.settings.appearance.changes_indicator = if v.as_ref() == "dot" {
                                osier_config::settings::ChangesIndicator::Dot
                            } else {
                                osier_config::settings::ChangesIndicator::Icon
                            }
                        }),
                    )
                    .default_value("icon".to_string()),
                )
                .description("How a project with uncommitted changes is marked in the sidebar."),
            ),
        )
        .group(
            SettingGroup::new()
                .title("Tooltips")
                .item(SettingItem::render({
                    let v = view.clone();
                    move |options, _, cx| {
                        let o = v.read(cx);
                        let ms = o
                            .settings
                            .appearance
                            .tooltip_delay_ms
                            .min(crate::app::TOOLTIP_DELAY_MAX_MS);
                        let slider = o.tooltip_delay.clone();
                        h_flex()
                            .w_full()
                            .gap_3()
                            .items_center()
                            .child(div().text_sm().flex_none().child("Show after"))
                            .child(
                                div().flex_1().child(
                                    gpui_kit::component::slider::Slider::new(&slider)
                                        .disabled(options.is_disabled()),
                                ),
                            )
                            .child(div().text_sm().w(px(80.)).flex_none().text_right().whitespace_nowrap().child(if ms == 0 {
                                "at once".to_string()
                            } else {
                                format!("{ms} ms")
                            }))
                    }
                }))
                .description(
                    "How long the pointer rests on a button or control before its tooltip \
                     appears. Default: at once.",
                ),
        )
        .group(
            SettingGroup::new().title("Lists").item(
                SettingItem::new(
                    "Show menu buttons on rows",
                    SettingField::switch(
                        get(view, |o| o.settings.appearance.row_menu_buttons),
                        set(view, |o, v: bool, _| {
                            o.settings.appearance.row_menu_buttons = v
                        }),
                    )
                    .default_value(false),
                )
                .description(
                    "Add a ... button to the end of commit, branch, tag, remote, worktree and \
                     patch rows. Right-clicking a row opens the same menu whether this is on or \
                     off.",
                ),
            ),
        )
        .group(
            SettingGroup::new()
                .title("Fonts")
                .description("Leave empty to use the theme's fonts.")
                .items(vec![
                    SettingItem::new(
                        "UI font",
                        SettingField::input(
                            get(view, |o| {
                                o.settings
                                    .appearance
                                    .ui_font
                                    .clone()
                                    .unwrap_or_default()
                                    .into()
                            }),
                            set(view, |o, v: SharedString, _| {
                                o.settings.appearance.ui_font = opt_string(v)
                            }),
                        )
                        .default_value(""),
                    ),
                    SettingItem::new(
                        "Monospace font",
                        SettingField::input(
                            get(view, |o| {
                                o.settings
                                    .appearance
                                    .mono_font
                                    .clone()
                                    .unwrap_or_default()
                                    .into()
                            }),
                            set(view, |o, v: SharedString, _| {
                                o.settings.appearance.mono_font = opt_string(v)
                            }),
                        )
                        .default_value(""),
                    ),
                    SettingItem::new(
                        "Font size",
                        SettingField::number_input(
                            NumberFieldOptions {
                                min: 0.0,
                                max: 40.0,
                                step: 1.0,
                            },
                            get(view, |o| {
                                f64::from(o.settings.appearance.font_size.unwrap_or(0.0))
                            }),
                            set(view, |o, v: f64, _| {
                                o.settings.appearance.font_size = (v > 0.0).then_some(v as f32)
                            }),
                        )
                        .default_value(0.0),
                    )
                    .description("0 uses the theme's size."),
                ]),
        )
}
