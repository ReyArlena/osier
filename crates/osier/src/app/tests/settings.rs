//! The settings dialog, themes, Dux Shell sync and appearance settings.

use super::*;

#[gpui::test]
fn ui_settings_open_by_click_and_close_by_escape(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "one\n", "first");
    let h = Harness::open(cx, &repo);
    h.click(cx, "settings");
    assert!(h.has_settings(cx), "the gear opens settings");
    assert!(
        h.bounds(cx, "settings-dialog").is_some(),
        "dialog content is drawn"
    );
    h.keys(cx, "escape");
    assert!(
        !h.has_settings(cx),
        "escape reaches the dialog and closes it"
    );
    h.keys(cx, "ctrl-,");
    assert!(h.has_settings(cx), "the keyboard shortcut opens it too");
    h.keys(cx, "escape");
    assert!(!h.has_settings(cx));
}

#[gpui::test]
fn ui_settings_close_button_closes_and_shortcuts_keep_working(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "one\n", "first");
    let h = Harness::open(cx, &repo);
    h.keys(cx, "ctrl-,");
    assert!(h.has_settings(cx));
    h.click(cx, "settings-close");
    assert!(
        !h.has_settings(cx),
        "the header's close button closes settings"
    );
    h.keys(cx, "ctrl-,");
    assert!(
        h.has_settings(cx),
        "focus went back to the app, so its shortcuts still work"
    );
}

#[gpui::test]
fn higher_contrast_strengthens_selections_and_switches_back_cleanly(cx: &mut TestAppContext) {
    use gpui_kit::component::ActiveTheme as _;
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    let h = Harness::open(cx, &repo);
    // The colours widgets actually draw with (`tokens`), not only the plain fields.
    let colors = |h: &Harness, cx: &mut TestAppContext| {
        h.read(cx, |_, cx| {
            let t = cx.theme();
            (
                t.tokens.list_active.color,
                t.tokens.sidebar_accent.color,
                t.tokens.list_hover.color,
            )
        })
    };
    let normal = colors(&h, cx);
    // Applied twice, as startup does: the second must not undo the first.
    h.act(cx, |o, _, cx| {
        o.settings.appearance.high_contrast = true;
        o.apply_appearance(cx);
        o.apply_appearance(cx);
    });
    let strong = colors(&h, cx);
    assert_ne!(strong.0, normal.0);
    assert_ne!(strong.1, normal.1);
    assert_eq!((strong.0.a, strong.1.a, strong.2.a), (1., 1., 1.), "opaque");
    h.act(cx, |o, _, cx| {
        o.settings.appearance.high_contrast = false;
        o.apply_appearance(cx);
    });
    assert_eq!(colors(&h, cx), normal, "off is the theme as it was");
}

#[gpui::test]
fn tooltip_delay_defaults_to_at_once_and_applies_live(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "one\n", "first");
    let h = Harness::open(cx, &repo);
    let delay = |cx: &mut TestAppContext| {
        cx.update(|cx| gpui_kit::base::TooltipDefaults::global(cx).show_delay())
    };
    assert_eq!(delay(cx), std::time::Duration::from_millis(0));
    h.act(cx, |o, _, cx| {
        o.settings.appearance.tooltip_delay_ms = 600;
        o.apply_tooltip_delay(cx);
    });
    assert_eq!(delay(cx), std::time::Duration::from_millis(600));
    h.act(cx, |o, _, cx| {
        o.settings.appearance.tooltip_delay_ms = 99_999;
        o.apply_tooltip_delay(cx);
    });
    assert_eq!(
        delay(cx),
        std::time::Duration::from_millis(u64::from(TOOLTIP_DELAY_MAX_MS)),
        "hand-edited config is clamped"
    );
}

#[gpui::test]
fn animations_setting_turns_motion_on_and_off(cx: &mut TestAppContext) {
    use osier_config::settings::Animations;
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "one\n", "first");
    let h = Harness::open(cx, &repo);
    let reduced = |h: &Harness, cx: &mut TestAppContext| h.read(cx, |_, cx| cx.reduce_motion());
    assert!(
        !reduced(&h, cx),
        "follows the desktop, which says nothing in tests"
    );
    let set = |h: &Harness, cx: &mut TestAppContext, a: Animations, desktop: Option<bool>| {
        h.act(cx, move |o, _, cx| {
            o.settings.appearance.animations = a;
            o.system_reduced_motion = desktop;
            o.settings_changed(cx);
        });
    };
    set(&h, cx, Animations::Off, None);
    assert!(reduced(&h, cx));
    set(&h, cx, Animations::On, Some(true));
    assert!(!reduced(&h, cx), "On wins over the desktop");
    set(&h, cx, Animations::System, Some(true));
    assert!(reduced(&h, cx), "the desktop asks for reduced motion");
    set(&h, cx, Animations::System, Some(false));
    assert!(!reduced(&h, cx));
    // Settings opens and closes with motion off: the dialog has no animation to wait for.
    set(&h, cx, Animations::Off, None);
    h.keys(cx, "ctrl-,");
    assert!(h.has_settings(cx));
    h.keys(cx, "escape");
    assert!(!h.has_settings(cx));
    // An edit to config.toml applies too.
    let file = h.read(cx, |o, _| o.paths.settings_file());
    std::fs::write(&file, "[appearance]\nanimations = \"on\"\n").expect("write config");
    h.act(cx, |o, window, cx| {
        assert!(o.reload_settings_from_disk(window, cx));
    });
    assert!(!reduced(&h, cx));
}

#[gpui::test]
fn animation_length_times_the_dialog_fade(cx: &mut TestAppContext) {
    use gpui_kit::component::ActiveTheme as _;
    use osier_config::settings::ThemeMode;
    use std::time::Duration;
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "one\n", "first");
    let h = Harness::open(cx, &repo);
    let fade = |h: &Harness, cx: &mut TestAppContext| {
        h.read(cx, |_, cx| cx.theme().motion.duration_normal)
    };
    assert_eq!(
        fade(&h, cx),
        Duration::from_millis(180),
        "gpui-kit's default"
    );
    h.act(cx, |o, _, cx| {
        o.settings.appearance.animation_ms = 100;
        o.settings_changed(cx);
    });
    assert_eq!(fade(&h, cx), Duration::from_millis(100));
    h.act(cx, |o, _, cx| o.set_theme_mode(ThemeMode::Light, cx));
    assert_eq!(
        fade(&h, cx),
        Duration::from_millis(100),
        "kept across a theme change"
    );
    // An edit to config.toml moves the slider too.
    let file = h.read(cx, |o, _| o.paths.settings_file());
    std::fs::write(&file, "[appearance]\nanimation_ms = 250\n").expect("write config");
    h.act(cx, |o, window, cx| {
        assert!(o.reload_settings_from_disk(window, cx));
    });
    assert_eq!(fade(&h, cx), Duration::from_millis(250));
    let slider = h.read(cx, |o, cx| o.animation_length.read(cx).value().start());
    assert_eq!(slider, 250.);
}

#[gpui::test]
fn settings_reload_from_disk_and_account_overrides(cx: &mut TestAppContext) {
    use gpui_kit::component::ActiveTheme as _;
    use osier_config::settings::ThemeMode;
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "one\n", "first");
    let h = Harness::open(cx, &repo);

    // An external edit to config.toml is picked up and applied.
    let file = h.read(cx, |osier, _| osier.paths.settings_file());
    std::fs::create_dir_all(file.parent().expect("dir")).expect("mkdir");
    std::fs::write(
        &file,
        "[appearance]\nmode = \"light\"\n[diff]\ncontext_lines = 7\n",
    )
    .expect("write config");
    h.act(cx, |osier, window, cx| {
        assert!(osier.reload_settings_from_disk(window, cx));
        assert!(
            !osier.reload_settings_from_disk(window, cx),
            "unchanged second time"
        );
    });
    h.read(cx, |osier, cx| {
        assert_eq!(osier.settings.appearance.mode, ThemeMode::Light);
        assert_eq!(osier.settings.diff.context_lines, 7);
        assert!(!cx.theme().mode.is_dark());
    });
    // A broken file keeps the current settings.
    std::fs::write(&file, "[appearance\nmode = ").expect("write config");
    h.act(cx, |osier, window, cx| {
        assert!(!osier.reload_settings_from_disk(window, cx));
    });
    h.read(cx, |osier, _| {
        assert_eq!(osier.settings.diff.context_lines, 7)
    });

    // Per-project account overrides can be reset from the panel.
    h.act(cx, |osier, _, _| {
        osier.set_project_account("/r/one", "alice", true);
        osier.set_project_account("/r/two", "bob", false);
    });
    h.read(cx, |osier, _| {
        assert_eq!(osier.state.accounts.len(), 2);
        assert!(osier.session_only_accounts.contains("/r/two"));
    });
    h.act(cx, |osier, _, cx| {
        osier.clear_account_override("/r/two".into(), cx)
    });
    h.read(cx, |osier, _| {
        assert_eq!(osier.state.accounts.len(), 1);
        assert!(osier.session_only_accounts.is_empty());
    });
    h.act(cx, |osier, _, cx| osier.clear_all_account_overrides(cx));
    h.read(cx, |osier, _| assert!(osier.state.accounts.is_empty()));
}

#[gpui::test]
fn settings_dialog_closes_on_cancel(cx: &mut TestAppContext) {
    use gpui_kit::component::WindowExt as _;
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "one\n", "first");
    let h = Harness::open(cx, &repo);
    h.act(cx, |osier, window, cx| osier.open_settings(window, cx));
    cx.run_until_parked();
    let open = cx
        .update_window(h.window, |_, window, cx| window.has_active_dialog(cx))
        .expect("window");
    assert!(open, "dialog opened");
    // Focus must sit inside the sheet, otherwise Cancel never reaches it.
    let focused = cx
        .update_window(h.window, |_, window, cx| {
            window
                .focused(cx)
                .map(|f| f == h.view.read(cx).settings_focus)
        })
        .expect("window");
    assert_eq!(focused, Some(true), "settings content is focused");
    cx.dispatch_action(h.window, gpui_kit::component::dialog::Cancel);
    cx.run_until_parked();
    let open = cx
        .update_window(h.window, |_, window, cx| window.has_active_dialog(cx))
        .expect("window");
    assert!(!open, "Cancel (close button or Escape) closed the dialog");
}

#[gpui::test]
fn theme_menu_switches_mode_and_theme(cx: &mut TestAppContext) {
    use gpui_kit::component::ActiveTheme as _;
    use osier_config::settings::ThemeMode;
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "one\n", "first");
    let h = Harness::open(cx, &repo);

    h.read(cx, |_, cx| {
        assert!(cx.theme().mode.is_dark());
        assert_eq!(cx.theme().theme_name().as_ref(), "Catppuccin Mocha");
    });
    h.act(cx, |osier, _, cx| {
        osier.set_theme_mode(ThemeMode::Light, cx)
    });
    h.read(cx, |osier, cx| {
        assert!(!cx.theme().mode.is_dark());
        assert_eq!(cx.theme().theme_name().as_ref(), "Catppuccin Latte");
        assert_eq!(osier.settings.appearance.mode, ThemeMode::Light);
    });
    // Picking a dark theme while in light mode switches to dark and remembers it.
    h.act(cx, |osier, _, cx| osier.set_theme("Catppuccin Frappe", cx));
    h.read(cx, |osier, cx| {
        assert!(cx.theme().mode.is_dark());
        assert_eq!(cx.theme().theme_name().as_ref(), "Catppuccin Frappe");
        assert_eq!(osier.settings.appearance.dark_theme, "Catppuccin Frappe");
        assert_eq!(osier.settings.appearance.light_theme, "Catppuccin Latte");
    });
    // Unknown names are ignored.
    h.act(cx, |osier, _, cx| osier.set_theme("Nope", cx));
    h.read(cx, |_, cx| {
        assert_eq!(cx.theme().theme_name().as_ref(), "Catppuccin Frappe")
    });
}

#[gpui::test]
fn dux_shell_sync_applies_the_generated_theme(cx: &mut TestAppContext) {
    use crate::themes::{DuxSource, ThemeStore};
    use gpui_kit::component::ActiveTheme as _;
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "one\n", "first");
    let h = Harness::open(cx, &repo);
    let dms = tempfile::tempdir().expect("tempdir");
    let colors = dms.path().join("dms-colors.json");
    std::fs::write(
        &colors,
        r##"{"mode":"light","colors":{"dark":{"background":"#101418","primary":"#9ecbff"},
            "light":{"background":"#f7f9fe","primary":"#1f5f98"}},"dank16":{}}"##,
    )
    .expect("write dms file");

    h.read(cx, |osier, cx| {
        assert!(!ThemeStore::global(cx).dux_available());
        assert_eq!(
            osier.settings.appearance.dux_shell_sync, None,
            "auto by default"
        );
        assert!(!osier.dux_sync_on(cx), "not detected: off");
    });
    // Detected and never chosen: on by default.
    h.act(cx, |osier, _, cx| {
        ThemeStore::global_mut(cx).set_dux_source(DuxSource::File(colors.clone()));
        osier.apply_appearance(cx);
    });
    h.read(cx, |osier, cx| {
        assert!(osier.dux_sync_on(cx));
        // Fresh installs start in System mode, so the shell's mode (light here) decides.
        assert_eq!(
            osier.settings.appearance.mode,
            osier_config::settings::ThemeMode::System
        );
        assert_eq!(cx.theme().theme_name().as_ref(), "Dux Shell Light");
    });
    // Turning it on sets System, so the shell's mode (light here) decides.
    h.act(cx, |osier, _, cx| osier.set_dux_sync(true, cx));
    h.read(cx, |osier, cx| {
        assert_eq!(
            osier.settings.appearance.mode,
            osier_config::settings::ThemeMode::System
        );
        assert_eq!(osier.settings.appearance.dux_shell_sync, Some(true));
        assert_eq!(cx.theme().theme_name().as_ref(), "Dux Shell Light");
    });
    // The user can still pick Dark by hand: the Dux dark colours.
    h.act(cx, |osier, _, cx| {
        osier.set_theme_mode(osier_config::settings::ThemeMode::Dark, cx)
    });
    h.read(cx, |_, cx| {
        assert_eq!(cx.theme().theme_name().as_ref(), "Dux Shell Dark");
        let rgb = cx.theme().background.to_rgb();
        assert_eq!((rgb.r * 255.0).round() as u8, 0x10);
        assert_eq!((rgb.b * 255.0).round() as u8, 0x18);
        let names = ThemeStore::global(cx).names(true);
        assert!(!names.iter().any(|n| n.as_ref() == "Dux Shell Dark"));
    });
    // Turning it off leaves the mode alone.
    h.act(cx, |osier, _, cx| osier.set_dux_sync(false, cx));
    h.read(cx, |osier, _| {
        assert_eq!(
            osier.settings.appearance.mode,
            osier_config::settings::ThemeMode::Dark
        )
    });
    // Picking an explicit theme turns sync off.
    h.act(cx, |osier, _, cx| {
        osier.set_dux_sync(true, cx);
        osier.set_theme("Catppuccin Mocha", cx)
    });
    h.read(cx, |osier, cx| {
        assert_eq!(cx.theme().theme_name().as_ref(), "Catppuccin Mocha");
        assert_eq!(osier.settings.appearance.dux_shell_sync, Some(false));
    });
}

#[gpui::test]
fn fresh_installs_follow_the_system_and_default_to_mocha_without_an_answer(
    cx: &mut TestAppContext,
) {
    use crate::system_scheme::SystemScheme;
    use gpui_kit::component::ActiveTheme as _;
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "one\n", "first");
    let h = Harness::open(cx, &repo);
    let theme = |h: &Harness, cx: &mut TestAppContext| {
        h.read(cx, |_, cx| cx.theme().theme_name().to_string())
    };
    h.read(cx, |o, _| {
        assert_eq!(
            o.settings.appearance.mode,
            osier_config::settings::ThemeMode::System
        )
    });
    // No Dux Shell, no answer from the desktop: Catppuccin Mocha.
    assert_eq!(theme(&h, cx), "Catppuccin Mocha");
    let set = |h: &Harness, cx: &mut TestAppContext, s: SystemScheme| {
        h.act(cx, move |o, _, cx| {
            o.system_scheme = s;
            o.apply_appearance(cx);
        })
    };
    set(&h, cx, SystemScheme::Light);
    assert_eq!(theme(&h, cx), "Catppuccin Latte");
    set(&h, cx, SystemScheme::Dark);
    assert_eq!(theme(&h, cx), "Catppuccin Mocha");
    set(&h, cx, SystemScheme::NoPreference);
    assert_eq!(theme(&h, cx), "Catppuccin Mocha", "fallback is dark");
    h.act(cx, |o, _, cx| {
        o.settings.appearance.system_fallback = osier_config::settings::SystemFallback::Light;
        o.apply_appearance(cx);
    });
    assert_eq!(theme(&h, cx), "Catppuccin Latte");
}

#[gpui::test]
fn ui_theme_cards_pick_a_theme_unless_dux_sync_is_on(cx: &mut TestAppContext) {
    use crate::themes::{DuxSource, ThemeStore};
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "one\n", "first");
    let h = Harness::open(cx, &repo);
    let other = h.read(cx, |o, cx| {
        ThemeStore::global(cx)
            .names(true)
            .into_iter()
            .find(|n| n.as_ref() != o.settings.appearance.dark_theme)
            .expect("a second dark theme")
            .to_string()
    });
    h.keys(cx, "ctrl-,");
    h.click(cx, &format!("theme-card:{other}"));
    h.read(cx, |o, _| {
        assert_eq!(o.settings.appearance.dark_theme, other)
    });

    // With Dux Shell colours followed, the cards do nothing.
    let dms = tempfile::tempdir().expect("tempdir");
    let colors = dms.path().join("dms-colors.json");
    std::fs::write(
        &colors,
        r##"{"mode":"dark","colors":{"dark":{"background":"#101418"},"light":{}},"dank16":{}}"##,
    )
    .expect("write");
    h.act(cx, |o, _, cx| {
        ThemeStore::global_mut(cx).set_dux_source(DuxSource::File(colors.clone()));
        o.set_dux_sync(true, cx);
        cx.notify();
    });
    h.click(cx, "theme-card:Catppuccin Mocha");
    h.read(cx, |o, _| {
        assert_eq!(o.settings.appearance.dark_theme, other, "disabled")
    });
}

#[gpui::test]
fn higher_contrast_makes_ref_badges_opaque(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    let _h = Harness::open(cx, &repo);
    cx.update(|cx| {
        cx.set_global(crate::themes::HighContrast(false));
        let (bg, _) = crate::themes::badge_tint(cx);
        assert!(bg.a < 0.5, "faint as the theme intends");
        cx.set_global(crate::themes::HighContrast(true));
        let (bg, border) = crate::themes::badge_tint(cx);
        assert!(
            bg.a > 0.99 && border.a > 0.99,
            "opaque with higher contrast"
        );
    });
}

#[gpui::test]
fn readable_syntax_colours_lift_dim_comments_and_can_be_turned_off(cx: &mut TestAppContext) {
    use gpui_kit::HighlightStyle;
    use gpui_kit::component::ActiveTheme as _;
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    let h = Harness::open(cx, &repo);
    let comment = |h: &Harness, cx: &mut TestAppContext| {
        h.read(cx, |_, cx| {
            let theme = cx.theme();
            let style = theme
                .highlight_theme
                .style
                .syntax
                .comment
                .expect("comment style");
            let color = HighlightStyle::from(style).color.expect("comment colour");
            (color, crate::themes::contrast(color, theme.background))
        })
    };
    // The default (on) lifts Catppuccin Mocha's comment grey past the floor.
    let (lifted, ratio) = comment(&h, cx);
    assert!(ratio >= 4.5, "comment contrast {ratio} with the floor on");
    h.act(cx, |o, _, cx| {
        o.settings.appearance.syntax_contrast = false;
        o.apply_appearance(cx);
    });
    let (original, ratio) = comment(&h, cx);
    assert!(
        ratio < 4.5,
        "the theme's own grey is below the floor ({ratio})"
    );
    assert_ne!(original, lifted);
    // Hex rounding moves the hue of a near-grey a little; the lift itself keeps it.
    assert!(
        (original.h - lifted.h).abs() < 0.05,
        "same hue, only lighter: {original:?} vs {lifted:?}"
    );
    // Back on, applied twice as startup does: the lift is idempotent.
    h.act(cx, |o, _, cx| {
        o.settings.appearance.syntax_contrast = true;
        o.apply_appearance(cx);
        o.apply_appearance(cx);
    });
    assert_eq!(comment(&h, cx).0, lifted);
}
