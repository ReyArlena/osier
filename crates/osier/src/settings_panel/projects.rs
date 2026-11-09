//! Projects page: scan directories, switching, status of closed projects, recents and
//! per-project choices.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::setting::{
    NumberFieldOptions, SettingField, SettingGroup, SettingItem, SettingPage,
};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app::Osier;
use crate::test_ids::TestId as _;

use super::{SettingsTarget, get, highlightable, set};

pub(super) fn projects_page(view: &Entity<Osier>, cx: &App) -> SettingPage {
    let v1 = view.clone();
    let v2 = view.clone();
    SettingPage::new("Projects")
        .description("Where repositories are found and what Osier remembers about each one.")
        .icon(Icon::new(Lucide::FolderGit2))
        .group(
            SettingGroup::new()
                .title("Scan directories")
                .description("Repositories found directly inside these directories are listed under Recents and cannot be removed individually.")
                .item(SettingItem::render(move |options, _, cx| {
                    let o = v1.read(cx);
                    let muted = cx.theme().muted_foreground;
                    let mut col = v_flex().gap_1().w_full();
                    if o.settings.projects.scan_dirs.is_empty() {
                        col = col.child(div().text_sm().text_color(muted).child("None."));
                    }
                    for (ix, dir) in o.settings.projects.scan_dirs.iter().enumerate() {
                        let v = v1.clone();
                        let d = dir.clone();
                        col = col.child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(Icon::new(Lucide::Folder).xsmall())
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_sm()
                                        .child(crate::format::tilde(dir)),
                                )
                                .child(
                                    Button::new(("remove-scan-dir", ix))
                                        .ghost()
                                        .with_size(options.size())
                                        .icon(Lucide::X)
                                        // Icon flush with the controls below it.
                                        .mr(px(-8.))
                                        .tooltip("Remove")
                                        .on_click(move |_, _, cx| {
                                            v.update(cx, |o, cx| o.remove_scan_dir(d.clone(), cx));
                                        }),
                                ),
                        );
                    }
                    let v = v1.clone();
                    col.child(
                        h_flex().pt_1().child(
                            Button::new("add-scan-dir").test_id("add-scan-dir")
                                .outline()
                                .with_size(options.size())
                                .icon(Lucide::FolderPlus)
                                .label("Add directory…")
                                .on_click(move |_, window, cx| {
                                    v.update(cx, |o, cx| o.add_scan_dir_prompt(window, cx));
                                }),
                        ),
                    )
                }))
                .item(
                    SettingItem::new(
                        "Scan depth",
                        SettingField::number_input(
                            NumberFieldOptions {
                                min: 1.0,
                                max: 3.0,
                                step: 1.0,
                            },
                            get(view, |o| f64::from(o.settings.projects.scan_depth.unwrap_or(1))),
                            set(view, |o, v: f64, _| {
                                o.settings.projects.scan_depth = Some(v.clamp(1.0, 3.0) as u8)
                            }),
                        )
                        .default_value(1.0),
                    )
                    .description("1 looks at direct children only."),
                ),
        )
        .group(
            SettingGroup::new().title("Switching projects").item(
                SettingItem::new(
                    "When Changes is empty",
                    SettingField::dropdown(
                        vec![
                            ("stay".into(), "Don't switch".into()),
                            ("log".into(), "Log".into()),
                            ("branches".into(), "Branches".into()),
                            ("stashes".into(), "Stashes".into()),
                            ("files".into(), "Files".into()),
                            ("patches".into(), "Patches".into()),
                        ],
                        get(view, |o| {
                            use osier_config::settings::EmptyChangesTab as E;
                            match o.settings.projects.empty_changes_tab {
                                E::Stay => "stay",
                                E::Log => "log",
                                E::Branches => "branches",
                                E::Stashes => "stashes",
                                E::Files => "files",
                                E::Patches => "patches",
                            }
                            .into()
                        }),
                        set(view, |o, v: SharedString, _| {
                            use osier_config::settings::EmptyChangesTab as E;
                            o.settings.projects.empty_changes_tab = match v.as_ref() {
                                "stay" => E::Stay,
                                "branches" => E::Branches,
                                "stashes" => E::Stashes,
                                "files" => E::Files,
                                "patches" => E::Patches,
                                _ => E::Log,
                            };
                        }),
                    )
                    .default_value("log".to_string()),
                )
                .description(
                    "When you switch to or open a project while on the Changes tab and it has \
                     nothing to commit, show this tab instead. Switching to a project with \
                     changes brings Changes back.",
                ),
            ),
        )
        .group(
            highlightable(SettingGroup::new(), SettingsTarget::RECENT_STATUS, view, cx)
                .title("Status of projects that are not open")
                .description("Ahead/behind badges and the changes marker on pinned, recent and scanned projects.")
                .item(
                    SettingItem::new(
                        "Show status",
                        SettingField::dropdown(
                            vec![
                                ("never".into(), "Never".into()),
                                ("cached".into(), "From when it was last open".into()),
                                ("watch".into(), "Watch git (ahead/behind only)".into()),
                                ("check".into(), "Watch git and check every N minutes".into()),
                            ],
                            get(view, |o| {
                                use osier_config::settings::RecentStatusMode as M;
                                match o.settings.projects.recent_status {
                                    M::Never => "never",
                                    M::Cached => "cached",
                                    M::Watch => "watch",
                                    M::Check => "check",
                                }
                                .into()
                            }),
                            set(view, |o, v: SharedString, _| {
                                use osier_config::settings::RecentStatusMode as M;
                                o.settings.projects.recent_status = match v.as_ref() {
                                    "cached" => M::Cached,
                                    "watch" => M::Watch,
                                    "check" => M::Check,
                                    _ => M::Never,
                                };
                            }),
                        )
                        .default_value("never".to_string()),
                    )
                    .description("Never runs nothing in the background. Watch modes use a few file watches per repository and one git process at a time."),
                )
                .item(SettingItem::render({
                    let v = view.clone();
                    move |options, _, cx| {
                        let o = v.read(cx);
                        let checking = o.settings.projects.recent_status
                            == osier_config::settings::RecentStatusMode::Check;
                        let minutes = o.settings.projects.recent_status_minutes.clamp(1, 60);
                        let slider = o.recent_interval.clone();
                        h_flex()
                            .w_full()
                            .gap_3()
                            .items_center()
                            .when(!checking, |this| this.opacity(0.5))
                            .child(div().text_sm().flex_none().child("Check every"))
                            .child(
                                div().flex_1().child(
                                    gpui_kit::component::slider::Slider::new(&slider)
                                        .disabled(!checking || options.is_disabled()),
                                ),
                            )
                            .child(div().text_sm().w(px(80.)).flex_none().text_right().whitespace_nowrap().child(if minutes == 1 {
                                "1 minute".to_string()
                            } else {
                                format!("{minutes} minutes")
                            }))
                    }
                }))
                .item(SettingItem::render({
                    let v = view.clone();
                    move |options, _, cx| {
                        use osier_config::settings::RecentStatusMode as M;
                        let mode = v.read(cx).settings.projects.recent_status;
                        let hint = match mode {
                            M::Never => "Turn the status on to scan.",
                            M::Cached => "One full check now; the results are cached.",
                            M::Watch => "Re-read ahead/behind of every project now.",
                            M::Check => "Full check of every project now; the timer restarts.",
                        };
                        let v = v.clone();
                        h_flex()
                            .w_full()
                            .gap_3()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(hint),
                            )
                            .child(
                                Button::new("scan-recent-now")
                                    .test_id("scan-recent-now")
                                    .outline()
                                    .with_size(options.size())
                                    .icon(Lucide::RefreshCw)
                                    .label("Scan now")
                                    .disabled(mode == M::Never || options.is_disabled())
                                    .on_click(move |_, window, cx| {
                                        v.update(cx, |o, cx| o.scan_recent_now(window, cx));
                                    }),
                            )
                    }
                })),
        )
        .group(
            SettingGroup::new()
                .title("Recents")
                .item(SettingItem::render(move |options, _, cx| {
                    let o = v2.read(cx);
                    let n = o.state.recent_not_open().count();
                    let v = v2.clone();
                    h_flex()
                        .w_full()
                        .justify_between()
                        .items_center()
                        .gap_3()
                        .child(div().text_sm().child(format!(
                            "{n} recent {} not currently open",
                            if n == 1 { "repository" } else { "repositories" }
                        )))
                        .child(
                            Button::new("clear-recents").test_id("clear-recents")
                                .outline()
                                .with_size(options.size())
                                .label("Clear all recents")
                                .disabled(n == 0)
                                .on_click(move |_, window, cx| {
                                    v.update(cx, |o, cx| o.confirm_clear_recents(window, cx));
                                }),
                        )
                })),
        )
        .group(crate::project_choices::group(view, cx))
}
