//! Accounts page: GitHub CLI accounts, the default account per host and ssh.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::Button;
use gpui_kit::component::setting::{SettingField, SettingGroup, SettingItem, SettingPage};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app::Osier;
use crate::test_ids::TestId as _;

use super::{get, kv, set};

pub(super) fn accounts_page(view: &Entity<Osier>, cx: &App) -> SettingPage {
    let osier = view.read(cx);
    let mut hosts: Vec<String> = osier.gh_accounts.iter().map(|a| a.host.clone()).collect();
    hosts.sort();
    hosts.dedup();
    let mut host_items = Vec::new();
    for host in hosts {
        let mut options: Vec<(SharedString, SharedString)> =
            vec![("".into(), "Ask when needed".into())];
        for a in osier.gh_accounts.iter().filter(|a| a.host == host) {
            options.push((a.login.clone().into(), a.login.clone().into()));
        }
        let (h1, h2) = (host.clone(), host.clone());
        host_items.push(
            SettingItem::new(
                host.clone(),
                SettingField::dropdown(
                    options,
                    get(view, move |o| {
                        o.settings
                            .auth
                            .default_accounts
                            .get(&h1)
                            .cloned()
                            .unwrap_or_default()
                            .into()
                    }),
                    set(view, move |o, v: SharedString, _| {
                        if v.is_empty() {
                            o.settings.auth.default_accounts.remove(&h2);
                        } else {
                            o.settings
                                .auth
                                .default_accounts
                                .insert(h2.clone(), v.to_string());
                        }
                    }),
                )
                .default_value(""),
            )
            .description("Used for https remotes on this host when the project has no choice."),
        );
    }
    let v1 = view.clone();
    let v3 = view.clone();
    let mut page = SettingPage::new("Accounts")
        .description("The GitHub accounts and ssh keys used to reach remotes.")
        .icon(Icon::new(Lucide::KeyRound))
        .group(
            SettingGroup::new()
                .title("GitHub CLI accounts")
                .description("Detected with `gh auth status`. Tokens are read only inside Osier's credential helper, never shown.")
                .item(SettingItem::render(move |options, _, cx| {
                    let o = v1.read(cx);
                    let muted = cx.theme().muted_foreground;
                    let mut col = v_flex().gap_1().w_full();
                    if o.gh_accounts.is_empty() {
                        col = col.child(
                            div().text_sm().text_color(muted).child(
                                "No accounts. Run `gh auth login` in a terminal, then refresh.",
                            ),
                        );
                    }
                    for a in &o.gh_accounts {
                        col = col.child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(Icon::new(Lucide::Github).xsmall())
                                .child(div().text_sm().child(a.login.clone()))
                                .child(div().text_xs().text_color(muted).child(a.host.clone()))
                                .child(div().text_xs().text_color(muted).child(a.git_protocol.clone()))
                                .when(a.active, |this| {
                                    this.child(Tag::info().xsmall().child("gh default"))
                                })
                                .when(a.state != "ok" && !a.state.is_empty(), |this| {
                                    this.child(Tag::warning().xsmall().child(a.state.clone()))
                                }),
                        );
                    }
                    let v = v1.clone();
                    col.child(
                        h_flex().pt_1().child(
                            Button::new("refresh-accounts").test_id("refresh-accounts")
                                .outline()
                                .with_size(options.size())
                                .icon(Lucide::RefreshCw)
                                .label("Refresh")
                                .on_click(move |_, _, cx| {
                                    v.update(cx, |o, cx| o.load_gh_accounts(cx));
                                }),
                        ),
                    )
                })),
        );
    if !host_items.is_empty() {
        page = page.group(
            SettingGroup::new()
                .title("Default account per host")
                .items(host_items),
        );
    }
    page.group(crate::project_choices::accounts_pointer(view))
        .group(
            SettingGroup::new()
                .title("SSH")
                .description("Key names only; private keys are never read.")
                .item(SettingItem::render(move |_, _, cx| {
                    let o = v3.read(cx);
                    let muted = cx.theme().muted_foreground;
                    // Read in the background when settings opens (`open_settings_with`).
                    let Some(status) = o.ssh_status.clone() else {
                        return v_flex()
                            .w_full()
                            .child(kv("Agent", "checking…", cx))
                            .into_any_element();
                    };
                    let agent = if status.agent_socket {
                        format!(
                            "running, {} {} loaded",
                            status.loaded_identities.len(),
                            if status.loaded_identities.len() == 1 { "identity" } else { "identities" }
                        )
                    } else {
                        "no agent socket (SSH_AUTH_SOCK unset)".to_string()
                    };
                    let keys = if status.key_files.is_empty() {
                        "none found in ~/.ssh".to_string()
                    } else {
                        status
                            .key_files
                            .iter()
                            .filter_map(|p| p.file_name())
                            .map(|n| n.to_string_lossy().into_owned())
                            .collect::<Vec<_>>()
                            .join(", ")
                    };
                    v_flex()
                        .gap_1()
                        .w_full()
                        .child(kv("Agent", agent, cx))
                        .when(!status.loaded_identities.is_empty(), |this| {
                            this.child(kv("Loaded", status.loaded_identities.join(", "), cx))
                        })
                        .child(kv("Keys", keys, cx))
                        .child(
                            div().text_xs().text_color(muted).child(
                                "ssh remotes use these directly; the gh accounts above only apply to https remotes.",
                            ),
                        )
                        .into_any_element()
                })),
        )
        .group(github_lists_group(view))
}

/// Accounts > GitHub lists: how often the cached GitHub lists are refreshed.
fn github_lists_group(view: &Entity<Osier>) -> SettingGroup {
    let v4 = view.clone();
    SettingGroup::new()
        .title("GitHub lists")
        .description(
            "Repositories to clone, organisations, templates, .gitignore and licence lists \
                 are kept on disk so the clone and create screens show them at once, and are \
                 checked for new ones in the background.",
        )
        .item(
            SettingItem::new(
                "Update in the background",
                SettingField::dropdown(
                    vec![
                        ("0".into(), "Off".into()),
                        ("15".into(), "Every 15 minutes".into()),
                        ("30".into(), "Every 30 minutes".into()),
                        ("60".into(), "Every hour".into()),
                        ("120".into(), "Every 2 hours".into()),
                    ],
                    get(view, |o| {
                        o.settings.github.list_refresh_minutes.to_string().into()
                    }),
                    set(view, |o, v: SharedString, _| {
                        o.settings.github.list_refresh_minutes = v.parse().unwrap_or(30)
                    }),
                )
                .default_value("30".to_string()),
            )
            .description(
                "Off: only when a clone or create screen opens (they always check for new \
                     ones then).",
            ),
        )
        .item(SettingItem::render(move |options, _, cx| {
            let v = v4.clone();
            let refreshing = v.read(cx).gh_lists_refreshing();
            h_flex().w_full().justify_end().child(
                Button::new("refresh-gh-lists")
                    .test_id("refresh-gh-lists")
                    .outline()
                    .with_size(options.size())
                    .icon(Lucide::RefreshCw)
                    .label(if refreshing {
                        "Refreshing…"
                    } else {
                        "Refresh now"
                    })
                    .disabled(refreshing)
                    .on_click(move |_, _, cx| {
                        v.update(cx, |o, cx| o.refresh_gh_lists_now(cx));
                    }),
            )
        }))
}
