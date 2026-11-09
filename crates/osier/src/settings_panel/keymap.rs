//! Keymap page: every action's keys, the prefix mode and the recorder.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::setting::{SettingField, SettingGroup, SettingItem, SettingPage};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme as _, Icon, Selectable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app::Osier;
use crate::test_ids::TestId as _;

pub(super) fn keymap_page(view: &Entity<Osier>) -> SettingPage {
    use crate::keymap::{Scope, actions};
    let specs = actions();
    let mut categories: Vec<&'static str> = Vec::new();
    for s in &specs {
        if !categories.contains(&s.category) {
            categories.push(s.category);
        }
    }
    let v_top = view.clone();
    let mut page = SettingPage::new("Keymap")
        .description("Keyboard shortcuts for every action.")
        .icon(Icon::new(Lucide::Keyboard))
        .group(
            SettingGroup::new()
                .title("Key bindings")
                .description(
                    "Click + and press the keys to add a binding, x to remove one. Plain keys (letters, /, space) never fire while typing in a text field.",
                )
                .item(SettingItem::render(move |options, _, _| {
                    let (va, vb) = (v_top.clone(), v_top.clone());
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("keymap-edit-file")
                                .test_id("keymap-edit-file")
                                .outline()
                                .with_size(options.size())
                                .icon(Lucide::SquarePen)
                                .label("Edit keymap.toml")
                                .on_click(move |_, window, cx| {
                                    va.update(cx, |o, cx| o.edit_keymap_file(window, cx));
                                }),
                        )
                        .child(
                            Button::new("keymap-reset-all")
                                .test_id("keymap-reset-all")
                                .outline()
                                .with_size(options.size())
                                .label("Reset all to defaults")
                                .on_click(move |_, window, cx| {
                                    vb.update(cx, |o, cx| o.reset_keymap(window, cx));
                                }),
                        )
                })),
        );
    for category in categories {
        let mut group = SettingGroup::new().title(category);
        if category == "Prefix" {
            group = group.description(
                "Press the prefix key, then a binding's key. While it waits, the status bar shows \
                 \"Prefix key pressed\"; Escape, a click, an unbound key or the timeout cancel \
                 it. Each key below picks how it fires with its three buttons: No prefix (the \
                 key alone), Prefix only (the prefix key, then the key) or Either (both work). \
                 Keys after the prefix also work while typing in a text field.",
            );
        }
        if category == "Context menus" {
            group = group.description(
                "Run a right-click menu entry on the selected row with a key. One key covers \
                 every row that offers the entry; others ignore it. These keys cannot share a \
                 key with any other binding.",
            );
        }
        for spec in specs.iter().filter(|s| s.category == category) {
            let (id, scope) = (spec.id, spec.scope);
            let v = view.clone();
            let mut item = SettingItem::new(
                spec.name,
                SettingField::render(move |options, _, cx| key_row(&v, id, options, cx)),
            );
            match scope {
                Scope::List => item = item.description("In the Changes, Log and other lists"),
                Scope::Row => {
                    item = item.description(format!("On the selected row: {}", spec.detail))
                }
                Scope::Settings => item = item.description("While settings is open"),
                Scope::App => {}
            }
            group = group.item(item);
        }
        if category == "Prefix" {
            let v = view.clone();
            group = group.item(SettingItem::render(move |options, _, cx| {
                let o = v.read(cx);
                let secs = o
                    .settings
                    .keys
                    .prefix_timeout_secs
                    .min(crate::app::PREFIX_TIMEOUT_MAX_SECS);
                let slider = o.prefix_timeout.clone();
                h_flex()
                    .w_full()
                    .gap_3()
                    .items_center()
                    .child(div().text_sm().flex_none().child("Stop waiting after"))
                    .child(
                        div().flex_1().child(
                            gpui_kit::component::slider::Slider::new(&slider)
                                .disabled(options.is_disabled()),
                        ),
                    )
                    .child(
                        div()
                            .text_sm()
                            .w(px(140.))
                            .flex_none()
                            .text_right()
                            .whitespace_nowrap()
                            .child(match secs {
                                0 => "never (Esc or a key)".to_string(),
                                1 => "1 second".to_string(),
                                n => format!("{n} seconds"),
                            }),
                    )
            }));
        }
        page = page.group(group);
    }
    page
}

/// No prefix / Prefix only / Either for one key, as three joined icon buttons.
fn mode_buttons(
    view: &Entity<Osier>,
    id: &'static str,
    stored: &str,
    current: crate::keymap::KeyMode,
) -> AnyElement {
    use crate::keymap::{KeyMode, pretty_key, split_mode};
    use gpui_kit::component::button::ButtonGroup;
    let bare = pretty_key(split_mode(stored).1);
    let info = |mode: KeyMode| match mode {
        KeyMode::Plain => ("plain", Lucide::Key, format!("No prefix: {bare} alone")),
        KeyMode::Prefix => (
            "prefix",
            Lucide::ChevronsRight,
            format!("Prefix only: the prefix key, then {bare}"),
        ),
        KeyMode::Either => (
            "either",
            Lucide::Split,
            format!("Either: {bare} alone, or the prefix key then {bare}"),
        ),
    };
    let mut group = ButtonGroup::new(SharedString::from(format!("keymode-{id}-{stored}")))
        .compact()
        .xsmall();
    for mode in KeyMode::ALL {
        let (name, icon, tip) = info(mode);
        group = group.child(
            Button::new(SharedString::from(format!("keymode-{id}-{stored}-{name}")))
                .test_id(|| format!("keymode:{id}:{stored}:{name}"))
                .icon(icon)
                .tooltip(tip)
                // The chosen mode is filled so it reads at a glance; the others are outlined.
                .map(|b| {
                    if mode == current {
                        b.primary()
                    } else {
                        b.outline()
                    }
                })
                .selected(mode == current),
        );
    }
    let v = view.clone();
    let stored = stored.to_string();
    group
        .on_click(move |picked: &Vec<usize>, window, cx| {
            if let Some(&ix) = picked.first()
                && let Some(&mode) = KeyMode::ALL.get(ix)
            {
                v.update(cx, |o, cx| o.set_key_mode(id, &stored, mode, window, cx));
            }
        })
        .into_any_element()
}

/// One action's keys: removable chips, a recorder, Reset when changed, and conflicts.
fn key_row(
    view: &Entity<Osier>,
    id: &'static str,
    options: &gpui_kit::component::setting::RenderOptions,
    cx: &App,
) -> AnyElement {
    use crate::keymap::{Keymap, conflicts};
    let o = view.read(cx);
    let keymap = Keymap::global(cx);
    let keys = keymap.keys_for(id).to_vec();
    let recording = o.keymap_recording.as_ref().is_some_and(|(r, _)| *r == id);
    let overridden = o.keymap_applied.bindings.contains_key(id);
    let clashes: Vec<&'static str> = keys.iter().flat_map(|k| conflicts(keymap, id, k)).collect();
    let muted = cx.theme().muted_foreground;
    let mut row = h_flex().gap_1().items_center().flex_wrap().justify_end();
    if keys.is_empty() && !recording {
        row = row.child(div().text_sm().text_color(muted).child("Unbound"));
    }
    for k in keys {
        let v = view.clone();
        let key = k.clone();
        let (mode, bare) = crate::keymap::split_mode(&k);
        let modes = (id != crate::keymap::PREFIX_KEY).then(|| mode_buttons(view, id, &k, mode));
        row = row.child(
            h_flex()
                .gap_0p5()
                .items_center()
                .child(
                    Tag::secondary()
                        .xsmall()
                        .child(crate::keymap::pretty_key(bare)),
                )
                .when_some(modes, |this, m| this.child(m))
                .child(
                    Button::new(SharedString::from(format!("keymap-remove-{id}-{k}")))
                        .test_id(|| format!("keymap-remove:{id}:{k}"))
                        .ghost()
                        .xsmall()
                        .icon(Lucide::X)
                        .tooltip("Remove")
                        .on_click(move |_, window, cx| {
                            v.update(cx, |o, cx| o.remove_action_key(id, &key, window, cx));
                        }),
                ),
        );
    }
    let v = view.clone();
    row = if recording {
        row.child(Tag::warning().xsmall().child("Press keys… (Esc cancels)"))
    } else {
        row.child(
            Button::new(SharedString::from(format!("keymap-add-{id}")))
                .test_id(|| format!("keymap-add:{id}"))
                .ghost()
                .with_size(options.size())
                .icon(Lucide::Plus)
                // A ghost button's icon sits inside its padding; pull it out so the icon
                // ends where the other rows' controls end.
                .when(!overridden, |b| b.mr(px(-8.)))
                .tooltip("Add a key binding")
                .on_click(move |_, _, cx| {
                    v.update(cx, |o, cx| o.start_key_recording(id, cx));
                }),
        )
    };
    let v = view.clone();
    v_flex()
        .items_end()
        .gap_1()
        .child(row.when(overridden, |row| {
            row.child(
                Button::new(SharedString::from(format!("keymap-reset-{id}")))
                    .test_id(|| format!("keymap-reset:{id}"))
                    .ghost()
                    .with_size(options.size())
                    .label("Reset")
                    .mr(px(-10.))
                    .on_click(move |_, window, cx| {
                        let defaults: Vec<String> = crate::keymap::actions()
                            .iter()
                            .find(|a| a.id == id)
                            .map(|a| a.defaults.iter().map(|k| k.to_string()).collect())
                            .unwrap_or_default();
                        v.update(cx, |o, cx| o.set_action_keys(id, defaults, window, cx));
                    }),
            )
        }))
        .when(!clashes.is_empty(), |this| {
            this.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().warning)
                    .child(format!("Also bound to: {}", clashes.join(", "))),
            )
        })
        .into_any_element()
}
