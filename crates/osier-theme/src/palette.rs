//! A semantic palette and its expansion into gpui-kit theme tokens.

use serde_json::{Map, Value};

use crate::with_alpha;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Dark,
    Light,
}

/// The handful of colours a theme is built from. Every gpui-kit token derives from these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Palette {
    pub name: String,
    pub mode: Mode,
    /// Main editor/content background.
    pub base: String,
    /// Sidebar, title bar, status bar, tab bar.
    pub mantle: String,
    /// Deepest background (window border, tab bar in dark themes).
    pub crust: String,
    /// Panels, popovers, inputs.
    pub surface0: String,
    /// Borders, hover backgrounds.
    pub surface1: String,
    /// Active/pressed surfaces, scrollbar thumb.
    pub surface2: String,
    pub text: String,
    pub subtext: String,
    pub overlay: String,
    pub primary: String,
    /// Text drawn on top of `primary`.
    pub on_primary: String,
    pub link: String,
    pub red: String,
    pub maroon: String,
    pub green: String,
    pub yellow: String,
    pub peach: String,
    pub blue: String,
    pub cyan: String,
    pub magenta: String,
    /// Optional font overrides.
    pub font_family: Option<String>,
    pub mono_font_family: Option<String>,
}

impl Palette {
    /// The `themes[]` entry for gpui-kit.
    pub fn to_theme_value(&self) -> Value {
        let p = self;
        let is_dark = p.mode == Mode::Dark;
        let mut colors = Map::new();
        let mut set = |k: &str, v: String| {
            colors.insert(k.to_string(), Value::String(v));
        };
        set("background", p.base.clone());
        set("foreground", p.text.clone());
        set("border", p.surface1.clone());
        set("ring", p.primary.clone());
        set("accent.background", p.surface1.clone());
        set("accent.foreground", p.text.clone());
        set("muted.background", p.surface0.clone());
        set("muted.foreground", p.subtext.clone());
        set("panel.background", p.surface0.clone());
        set("popover.background", p.surface0.clone());
        set("popover.foreground", p.text.clone());
        set("input.border", p.overlay.clone());
        set("primary.background", p.primary.clone());
        set("primary.foreground", p.on_primary.clone());
        set("primary.hover.background", with_alpha(&p.primary, 0xe0));
        set("primary.active.background", with_alpha(&p.primary, 0xc0));
        set("secondary.background", p.surface0.clone());
        set("secondary.foreground", p.text.clone());
        set("secondary.hover.background", p.surface1.clone());
        set("secondary.active.background", p.surface2.clone());
        set("danger.background", p.red.clone());
        set("danger.foreground", p.on_primary.clone());
        set("danger.hover.background", with_alpha(&p.red, 0xe0));
        set("danger.active.background", with_alpha(&p.red, 0xc0));
        set("success.background", p.green.clone());
        set("success.foreground", p.on_primary.clone());
        set("warning.background", p.yellow.clone());
        set("warning.foreground", p.on_primary.clone());
        set("info.background", p.cyan.clone());
        set("info.foreground", p.on_primary.clone());
        set("link.foreground", p.link.clone());
        set("link.hover.foreground", p.primary.clone());
        set("link.active.foreground", p.primary.clone());
        set("list.background", p.base.clone());
        set(
            "list.even.background",
            with_alpha(&p.mantle, if is_dark { 0x80 } else { 0x40 }),
        );
        set("list.head.background", p.mantle.clone());
        set("list.hover.background", with_alpha(&p.surface1, 0x80));
        set("list.active.background", with_alpha(&p.primary, 0x22));
        set("list.active.border", with_alpha(&p.primary, 0x77));
        set("scrollbar.background", with_alpha(&p.base, 0x00));
        set("scrollbar.thumb.background", p.surface2.clone());
        set("scrollbar.thumb.hover.background", p.overlay.clone());
        set("sidebar.background", p.mantle.clone());
        set("sidebar.foreground", p.text.clone());
        set("sidebar.border", p.surface1.clone());
        set("sidebar.accent.background", with_alpha(&p.primary, 0x22));
        set("sidebar.accent.foreground", p.text.clone());
        set("sidebar.primary.background", p.primary.clone());
        set("sidebar.primary.foreground", p.on_primary.clone());
        set("tab_bar.background", p.mantle.clone());
        set("tab.background", with_alpha(&p.mantle, 0x00));
        set("tab.foreground", p.subtext.clone());
        set("tab.active.background", p.base.clone());
        set("tab.active.foreground", p.text.clone());
        set("title_bar.background", p.mantle.clone());
        set("title_bar.border", p.surface1.clone());
        set("status_bar.background", p.mantle.clone());
        set("status_bar.border", p.surface1.clone());
        set("window.border", p.crust.clone());
        set("selection.background", with_alpha(&p.primary, 0x40));
        set("caret", p.text.clone());
        set("base.red", p.red.clone());
        set("base.red.light", with_alpha(&p.red, 0x66));
        set("base.green", p.green.clone());
        set("base.green.light", with_alpha(&p.green, 0x66));
        set("base.yellow", p.yellow.clone());
        set("base.yellow.light", with_alpha(&p.yellow, 0x66));
        set("base.blue", p.blue.clone());
        set("base.blue.light", with_alpha(&p.blue, 0x66));
        set("base.cyan", p.cyan.clone());
        set("base.cyan.light", with_alpha(&p.cyan, 0x66));
        set("base.magenta", p.magenta.clone());
        set("base.magenta.light", with_alpha(&p.magenta, 0x66));
        set("chart.1", p.blue.clone());
        set("chart.2", p.magenta.clone());
        set("chart.3", p.green.clone());
        set("chart.4", p.peach.clone());
        set("chart.5", p.cyan.clone());

        let syntax_entries: [(&str, &str, Option<&str>); 28] = [
            ("attribute", &p.yellow, None),
            ("boolean", &p.peach, None),
            ("comment", &p.overlay, Some("italic")),
            ("comment.doc", &p.overlay, Some("italic")),
            ("constant", &p.peach, None),
            ("constructor", &p.blue, None),
            ("embedded", &p.text, None),
            ("emphasis", &p.text, Some("italic")),
            ("emphasis.strong", &p.text, Some("bold")),
            ("function", &p.blue, None),
            ("keyword", &p.magenta, None),
            ("label", &p.cyan, None),
            ("link_text", &p.link, None),
            ("link_uri", &p.overlay, Some("italic")),
            ("number", &p.peach, None),
            ("operator", &p.cyan, None),
            ("property", &p.blue, None),
            ("punctuation", &p.subtext, None),
            ("string", &p.green, None),
            ("string.escape", &p.magenta, None),
            ("string.regex", &p.peach, None),
            ("string.special", &p.yellow, None),
            ("tag", &p.blue, None),
            ("text.literal", &p.green, None),
            ("title", &p.text, Some("bold")),
            ("type", &p.yellow, None),
            ("variable", &p.text, None),
            ("variable.special", &p.red, None),
        ];
        let mut syntax = Map::new();
        for (key, color, style) in syntax_entries {
            let mut entry = Map::new();
            entry.insert("color".into(), Value::String(color.to_string()));
            match style {
                Some("bold") => {
                    entry.insert("font_weight".into(), Value::from(700));
                }
                Some(style) => {
                    entry.insert("font_style".into(), Value::String(style.to_string()));
                }
                None => {}
            }
            syntax.insert(key.to_string(), Value::Object(entry));
        }
        let status_entries: [(&str, String); 19] = [
            ("editor.foreground", p.text.clone()),
            ("editor.background", p.base.clone()),
            (
                "editor.active_line.background",
                with_alpha(&p.surface0, 0xaa),
            ),
            ("editor.line_number", p.overlay.clone()),
            ("editor.active_line_number", p.text.clone()),
            ("editor.invisible", with_alpha(&p.overlay, 0x66)),
            ("conflict", p.maroon.clone()),
            ("created", p.green.clone()),
            ("deleted", p.red.clone()),
            ("modified", p.yellow.clone()),
            ("renamed", p.magenta.clone()),
            ("error", p.red.clone()),
            ("warning", p.yellow.clone()),
            ("info", p.blue.clone()),
            ("hint", p.cyan.clone()),
            ("success", p.green.clone()),
            ("hidden", p.overlay.clone()),
            ("ignored", p.overlay.clone()),
            ("predictive", p.overlay.clone()),
        ];
        let mut highlight = Map::new();
        for (key, color) in status_entries {
            highlight.insert(key.to_string(), Value::String(color));
        }
        highlight.insert("syntax".into(), Value::Object(syntax));
        let highlight = Value::Object(highlight);
        let mut theme = Map::new();
        theme.insert("name".into(), Value::String(p.name.clone()));
        theme.insert(
            "mode".into(),
            Value::String(if is_dark { "dark" } else { "light" }.into()),
        );
        if let Some(f) = &p.font_family {
            theme.insert("font.family".into(), Value::String(f.clone()));
        }
        if let Some(f) = &p.mono_font_family {
            theme.insert("mono_font.family".into(), Value::String(f.clone()));
        }
        theme.insert("colors".into(), Value::Object(colors));
        theme.insert("highlight".into(), highlight);
        Value::Object(theme)
    }
}
