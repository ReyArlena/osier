//! Editors page: opening files in an external editor.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::Icon;
use gpui_kit::component::setting::{SettingField, SettingGroup, SettingItem, SettingPage};
use gpui_kit::*;

use crate::app::Osier;

use super::{get, opt_string, set};

pub(super) fn editors_page(view: &Entity<Osier>) -> SettingPage {
    let mut options: Vec<(SharedString, SharedString)> =
        vec![("".into(), "System default (xdg-open)".into())];
    for (name, template) in Osier::detected_editors() {
        options.push((template.into(), name.into()));
    }
    options.push(("custom".into(), "Custom command (below)".into()));
    SettingPage::new("Editors")
        .icon(Icon::new(Lucide::SquarePen))
        .resettable(true)
        .group(
            SettingGroup::new()
                .title("Open in editor")
                .description(
                    "Placeholders: {path} is the repository, {file} the file and {line} the line.",
                )
                .items(vec![
                    SettingItem::new(
                        "Editor",
                        SettingField::dropdown(
                            options.clone(),
                            {
                                let options = options.clone();
                                get(view, move |o| {
                                    let current =
                                        o.settings.editor.command.clone().unwrap_or_default();
                                    if options.iter().any(|(v, _)| v.as_ref() == current) {
                                        current.into()
                                    } else {
                                        "custom".into()
                                    }
                                })
                            },
                            set(view, |o, v: SharedString, _| {
                                if v.as_ref() != "custom" {
                                    o.settings.editor.command = opt_string(v);
                                }
                            }),
                        )
                        .default_value(""),
                    )
                    .description("Editors found on PATH."),
                    SettingItem::new(
                        "Editor command",
                        SettingField::input(
                            get(view, |o| {
                                o.settings.editor.command.clone().unwrap_or_default().into()
                            }),
                            set(view, |o, v: SharedString, _| {
                                o.settings.editor.command = opt_string(v)
                            }),
                        )
                        .default_value(""),
                    )
                    .layout(Axis::Vertical)
                    .description("For example: zed {file}:{line}"),
                    SettingItem::new(
                        "Terminal command",
                        SettingField::input(
                            get(view, |o| {
                                o.settings
                                    .editor
                                    .terminal
                                    .clone()
                                    .unwrap_or_default()
                                    .into()
                            }),
                            set(view, |o, v: SharedString, _| {
                                o.settings.editor.terminal = opt_string(v)
                            }),
                        )
                        .default_value(""),
                    )
                    .layout(Axis::Vertical)
                    .description("Empty tries common terminals. {path} is the repository."),
                ]),
        )
}
