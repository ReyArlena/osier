//! Diff page: how the diff pane lays out and what it shows.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::Icon;
use gpui_kit::component::setting::{
    NumberFieldOptions, SettingField, SettingGroup, SettingItem, SettingPage,
};
use gpui_kit::*;
use osier_config::Settings;

use crate::app::Osier;

use super::{get, set};

pub(super) fn diff_page(view: &Entity<Osier>) -> SettingPage {
    let d = Settings::default().diff;
    SettingPage::new("Diff")
        .icon(Icon::new(Lucide::FileDiff))
        .resettable(true)
        .group(SettingGroup::new().title("Diff pane").items(vec![
            SettingItem::new(
                "Layout",
                SettingField::dropdown(
                    vec![
                        ("unified".into(), "Unified".into()),
                        ("split".into(), "Split".into()),
                    ],
                    get(view, |o| match o.settings.diff.mode {
                        osier_config::settings::DiffMode::Split => "split".into(),
                        osier_config::settings::DiffMode::Unified => "unified".into(),
                    }),
                    set(view, |o, v: SharedString, _| {
                        o.settings.diff.mode = if v.as_ref() == "split" {
                            osier_config::settings::DiffMode::Split
                        } else {
                            osier_config::settings::DiffMode::Unified
                        }
                    }),
                )
                .default_value("unified".to_string()),
            ),
            SettingItem::new(
                "Conflicted files",
                SettingField::dropdown(
                    vec![
                        ("three-way".into(), "3-way merge tool".into()),
                        ("inline".into(), "Inline conflict blocks".into()),
                    ],
                    get(view, |o| match o.settings.diff.conflicts {
                        osier_config::settings::ConflictView::ThreeWay => "three-way".into(),
                        osier_config::settings::ConflictView::Inline => "inline".into(),
                    }),
                    set(view, |o, v: SharedString, _| {
                        o.settings.diff.conflicts = if v.as_ref() == "inline" {
                            osier_config::settings::ConflictView::Inline
                        } else {
                            osier_config::settings::ConflictView::ThreeWay
                        }
                    }),
                )
                .default_value("three-way".to_string()),
            ),
            SettingItem::new(
                "Context lines",
                SettingField::number_input(
                    NumberFieldOptions {
                        min: 0.0,
                        max: 30.0,
                        step: 1.0,
                    },
                    get(view, |o| f64::from(o.settings.diff.context_lines)),
                    set(view, |o, v: f64, cx| {
                        o.settings.diff.context_lines = v.max(0.0) as u32;
                        o.reload_selection(cx);
                    }),
                )
                .default_value(f64::from(d.context_lines)),
            ),
            SettingItem::new(
                "Line numbers",
                SettingField::switch(
                    get(view, |o| o.settings.diff.line_numbers),
                    set(view, |o, v: bool, _| o.settings.diff.line_numbers = v),
                )
                .default_value(d.line_numbers),
            ),
            SettingItem::new(
                "Soft wrap",
                SettingField::switch(
                    get(view, |o| o.settings.diff.soft_wrap),
                    set(view, |o, v: bool, _| o.settings.diff.soft_wrap = v),
                )
                .default_value(d.soft_wrap),
            )
            .description("Wrap long lines instead of scrolling sideways."),
            SettingItem::new(
                "Large file limit (MB)",
                SettingField::number_input(
                    NumberFieldOptions {
                        min: 1.0,
                        max: 500.0,
                        step: 1.0,
                    },
                    get(view, |o| {
                        o.settings.diff.large_file_bytes as f64 / (1024.0 * 1024.0)
                    }),
                    set(view, |o, v: f64, _| {
                        o.settings.diff.large_file_bytes =
                            (v.max(1.0) * 1024.0 * 1024.0) as u64
                    }),
                )
                .default_value(d.large_file_bytes as f64 / (1024.0 * 1024.0)),
            )
            .description("Files above this size ask before rendering."),
        ]))
}
