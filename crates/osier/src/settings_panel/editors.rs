//! Editors page: opening files in an external editor.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::Icon;
use gpui_kit::component::setting::{SettingField, SettingGroup, SettingItem, SettingPage};
use gpui_kit::*;

use crate::app::Osier;

use super::{get, opt_string, set};

pub(super) fn editors_page(view: &Entity<Osier>, cx: &App) -> SettingPage {
    let o = view.read(cx);
    let mut options: Vec<(SharedString, SharedString)> =
        vec![("".into(), "System default (xdg-open)".into())];
    // Found on PATH in the background when settings opens (`detect_editors`). Until then the
    // configured editor stays listed, so the dropdown does not jump to "Custom".
    let detected: Vec<(String, String)> = match &o.detected_editors {
        Some(found) => found.clone(),
        None => {
            // Named after the known editor whose arguments it ends with.
            let current = o.settings.editor.command.clone().unwrap_or_default();
            crate::app::external::KNOWN_EDITORS
                .iter()
                .find(|e| !current.is_empty() && current.ends_with(e.args))
                .map(|e| vec![(e.name.to_string(), current.clone())])
                .unwrap_or_default()
        }
    };
    for (name, template) in detected {
        options.push((template.into(), name.into()));
    }
    options.push(("custom".into(), "Custom command (below)".into()));
    let found_note = if o.detected_editors.is_some() {
        "Editors found on this computer."
    } else {
        "Looking for editors…"
    };
    SettingPage::new("Editors")
        .description("The editor that opens files and repositories from Osier.")
        .icon(Icon::new(Lucide::SquarePen))
        .resettable(true)
        .group(
            SettingGroup::new()
                .title("Open in editor")
                .description(
                    "Placeholders: {path} is the repository, {file} the file and {line} the line.",
                )
                .items(vec![
                    SettingItem::render(|_, _, _| {
                        crate::dialog_notice::DialogNotice::info(
                            "Only editors Osier can start from the command line are listed: \
                             commands on PATH, JetBrains Toolbox scripts and Flatpak apps. For \
                             another editor, choose Custom command and enter how to start it.",
                        )
                        .test_id("editors-notice")
                    }),
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
                    .description(found_note),
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
