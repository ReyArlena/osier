//! About page: the Updates group, runtime info, configuration paths and the Osier card.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::setting::{SettingField, SettingGroup, SettingItem, SettingPage};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app::Osier;
use crate::test_ids::TestId as _;

use super::{SettingsTarget, get, highlightable, kv, set};

/// About > Updates: where Osier stands against the latest release, and what can be done.
fn updates_group(view: &Entity<Osier>, cx: &App) -> SettingGroup {
    use crate::app::update_ui::{CheckState, InstallState};
    use crate::install_route::InstallRoute;
    use crate::update::Decision;
    let (v_check, v_install, v_restart, v_notes) =
        (view.clone(), view.clone(), view.clone(), view.clone());
    let group = SettingGroup::new()
        .title("Updates")
        .item(SettingItem::render(move |options, _, cx| {
            let o = v_check.read(cx);
            let u = &o.updates;
            let muted = cx.theme().muted_foreground;
            let route = u.route.clone();
            let scriptable = matches!(route, Some(InstallRoute::Script { .. }));
            let packaged = matches!(
                route,
                Some(InstallRoute::Package { .. } | InstallRoute::Flatpak)
            );
            let (status, latest) = match &u.check {
                CheckState::NotChecked => ("Not checked yet.".to_string(), None),
                CheckState::Checking => ("Checking…".to_string(), None),
                CheckState::Failed(e) => (format!("Could not check: {e}"), None),
                CheckState::Done { latest, decision } => (
                    match decision {
                        Decision::Update => {
                            format!("{latest} is available (you have v{}).", u.current)
                        }
                        Decision::UpToDate => format!("Up to date ({latest})."),
                        Decision::DevBuild => format!(
                            "Development build (v{}); the latest release is {latest}.",
                            u.current
                        ),
                        Decision::Newer => format!(
                            "v{} is newer than the latest release ({latest}).",
                            u.current
                        ),
                    },
                    Some((latest.clone(), *decision)),
                ),
            };
            let checked = o
                .state
                .update_check
                .checked_at
                .map(crate::format::relative_time);
            let available = matches!(latest, Some((_, Decision::Update)));
            let installing = matches!(u.install, InstallState::Installing(_));
            let install_line = match &u.install {
                InstallState::Idle => None,
                InstallState::Installing(t) => Some(format!("Installing {t}…")),
                InstallState::Installed(t) => {
                    Some(format!("{t} is installed. Restart Osier to use it."))
                }
                InstallState::Failed(e) => Some(format!("Update failed: {e}")),
            };
            let installed = matches!(u.install, InstallState::Installed(_));
            let refusal = route.as_ref().and_then(|r| r.update_refusal());
            let tag = latest.as_ref().map(|(t, _)| t.clone());
            let (vi, vr, vn) = (v_install.clone(), v_restart.clone(), v_notes.clone());
            v_flex()
                .test_id("updates-section")
                .gap_2()
                .w_full()
                .child(div().test_id("update-status").text_sm().child(status))
                .when_some(checked, |this, c| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(format!("Last checked {c}.")),
                    )
                })
                .when_some(install_line, |this, l| {
                    this.child(div().test_id("update-install-status").text_sm().child(l))
                })
                .when(available && !scriptable, |this| {
                    this.when_some(refusal.clone(), |this, r| {
                        this.child(div().text_sm().text_color(muted).child(r))
                    })
                })
                .when(packaged, |this| {
                    this.child(div().text_xs().text_color(muted).child(
                        "Packages (AUR, Flathub, distribution repositories) can take a little \
                         while to publish a new release, so an update may show here before \
                         your package manager has it.",
                    ))
                })
                .child(
                    h_flex()
                        .gap_2()
                        .flex_wrap()
                        .child(
                            Button::new("update-check-now")
                                .test_id("update-check-now")
                                .outline()
                                .with_size(options.size())
                                .icon(Lucide::RefreshCw)
                                .label("Check now")
                                .disabled(u.check == CheckState::Checking || installing)
                                .on_click({
                                    let v = v_check.clone();
                                    move |_, _, cx| v.update(cx, |o, cx| o.check_for_updates(cx))
                                }),
                        )
                        .child(
                            Button::new("update-install")
                                .test_id("update-install")
                                .primary()
                                .with_size(options.size())
                                .icon(Lucide::Download)
                                .label(match (&tag, available) {
                                    (Some(t), true) => format!("Update to {t}"),
                                    _ => "Update".to_string(),
                                })
                                .disabled(!(available && scriptable) || installing || installed)
                                .on_click(move |_, _, cx| {
                                    vi.update(cx, |o, cx| o.install_update(cx))
                                }),
                        )
                        .when(installed, |this| {
                            this.child(
                                Button::new("update-restart")
                                    .test_id("update-restart")
                                    .primary()
                                    .with_size(options.size())
                                    .icon(Lucide::RotateCw)
                                    .label("Restart now")
                                    .on_click(move |_, window, cx| {
                                        vr.update(cx, |o, cx| o.restart_after_update(window, cx))
                                    }),
                            )
                        })
                        .when_some(tag.clone(), |this, t| {
                            this.child(
                                Button::new("update-release-notes")
                                    .test_id("update-release-notes")
                                    .ghost()
                                    .with_size(options.size())
                                    .icon(Lucide::ExternalLink)
                                    .label(format!("What's new in {t}"))
                                    .on_click(move |_, _, cx| {
                                        let _ = &vn;
                                        cx.open_url(&crate::update::release_url(&t))
                                    }),
                            )
                        }),
                )
        }))
        .item(
            SettingItem::new(
                "Check for updates on startup",
                SettingField::switch(
                    get(view, |o| o.settings.updates.check_on_startup),
                    set(view, |o, v: bool, _| {
                        o.settings.updates.check_on_startup = v
                    }),
                )
                .default_value(true),
            )
            .description(
                "At most once a day. Shows an \"Update available\" badge in the status bar.",
            ),
        );
    highlightable(group, SettingsTarget::UPDATES, view, cx)
}

pub(super) fn about_page(view: &Entity<Osier>, cx: &App) -> SettingPage {
    let info = view.read(cx).runtime_info();
    let text = info.to_string();
    let config_file = view.read(cx).paths.settings_file();
    let config_dir = view.read(cx).paths.config_dir().to_path_buf();
    let v1 = view.clone();
    let v2 = view.clone();
    SettingPage::new("About")
        .description("Version, updates and where Osier keeps its files.")
        .icon(Icon::new(Lucide::Info))
        .group(updates_group(view, cx))
        .group(
            SettingGroup::new()
                .title("Runtime")
                .description("Include this block in bug reports (`osier --version --verbose` prints the same).")
                .item(SettingItem::render(move |options, _, cx| {
                    let text = text.clone();
                    let mut col = v_flex().gap_1().w_full();
                    for (label, value) in info.rows() {
                        col = col.child(kv(label, value, cx));
                    }
                    col.child(
                        h_flex().pt_1().child(
                            Button::new("copy-runtime").test_id("copy-runtime")
                                .outline()
                                .with_size(options.size())
                                .icon(Lucide::Copy)
                                .label("Copy")
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                                }),
                        ),
                    )
                })),
        )
        .group(
            SettingGroup::new()
                .title("Configuration")
                .description("config.toml is re-read when it changes on disk, so it is safe to manage with dotfiles.")
                .item(SettingItem::render(move |options, _, _| {
                    let (f, d) = (config_file.clone(), config_dir.clone());
                    let (va, vb) = (v1.clone(), v1.clone());
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("edit-config").test_id("edit-config")
                                .outline()
                                .with_size(options.size())
                                .icon(Lucide::SquarePen)
                                .label("Edit config.toml")
                                .on_click(move |_, window, cx| {
                                    let f = f.clone();
                                    va.update(cx, |o, cx| o.open_externally(&f, window, cx));
                                }),
                        )
                        .child(
                            Button::new("open-config-dir").test_id("open-config-dir")
                                .outline()
                                .with_size(options.size())
                                .icon(Lucide::FolderOpen)
                                .label("Open folder")
                                .on_click(move |_, window, cx| {
                                    let d = d.clone();
                                    vb.update(cx, |o, cx| o.open_externally(&d, window, cx));
                                }),
                        )
                })),
        )
        .group(
            SettingGroup::new().title("Osier").item(SettingItem::render(move |options, _, cx| {
                let _ = &v2;
                v_flex()
                    .gap_2()
                    .w_full()
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("A native git GUI for Linux. MIT licensed."),
                    )
                    .child(
                        h_flex().gap_2().child(
                            Button::new("repo-link").test_id("repo-link")
                                .outline()
                                .with_size(options.size())
                                .icon(Lucide::Github)
                                .label("github.com/ReyArlena/osier")
                                .on_click(|_, _, cx| cx.open_url("https://github.com/ReyArlena/osier")),
                        ),
                    )
            })),
        )
}
