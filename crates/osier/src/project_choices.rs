//! Settings > Projects > Per-project choices: what Osier remembers per repository (the GitHub
//! account, the diverged-pull method and the Update project method), as pills with an X that
//! forgets one, so Osier asks again next time.

use std::collections::BTreeSet;
use std::path::Path;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::setting::{SettingGroup, SettingItem};
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _, StyledExt as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app::Osier;
use crate::settings_panel::{SettingsTarget, highlightable};
use crate::test_ids::TestId as _;

/// Which remembered choice a pill shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Choice {
    Account,
    Pull,
    Update,
}

impl Choice {
    fn id(self) -> &'static str {
        match self {
            Choice::Account => "account",
            Choice::Pull => "pull",
            Choice::Update => "update",
        }
    }
}

/// One pill's text: what the choice is, its value, and a note (`this session`, `remembered`).
pub(crate) struct Pill {
    pub kind: Choice,
    pub label: &'static str,
    pub value: String,
    pub note: Option<&'static str>,
}

/// Every repository with a remembered choice, and its pills, in path order.
pub(crate) fn choices(o: &Osier) -> Vec<(String, Vec<Pill>)> {
    let s = &o.state;
    let repos: BTreeSet<&String> = s
        .accounts
        .keys()
        .chain(s.pull_strategies.keys())
        .chain(s.update_strategies.keys())
        .collect();
    let method = |m: &str| if m == "rebase" { "Rebase" } else { "Merge" }.to_string();
    repos
        .into_iter()
        .map(|repo| {
            let mut pills = Vec::new();
            if let Some(login) = s.accounts.get(repo) {
                pills.push(Pill {
                    kind: Choice::Account,
                    label: "GitHub account",
                    value: login.clone(),
                    note: o
                        .session_only_accounts
                        .contains(repo)
                        .then_some("this session"),
                });
            }
            if let Some(m) = s.pull_strategies.get(repo) {
                pills.push(Pill {
                    kind: Choice::Pull,
                    label: "Diverged pull",
                    value: method(m),
                    note: Some("last used"),
                });
            }
            if let Some(c) = s.update_strategies.get(repo) {
                pills.push(Pill {
                    kind: Choice::Update,
                    label: "Update project",
                    value: method(&c.strategy),
                    note: Some(if c.remember {
                        "remembered"
                    } else {
                        "last used"
                    }),
                });
            }
            (repo.clone(), pills)
        })
        .collect()
}

impl Osier {
    fn forget_choice(&mut self, repo: &str, kind: Choice, cx: &mut Context<Self>) {
        match kind {
            Choice::Account => self.clear_account_override(repo.to_string(), cx),
            Choice::Pull => self.clear_pull_strategy(repo, cx),
            Choice::Update => self.clear_update_strategy(repo, cx),
        }
    }
}

fn pill(repo_ix: usize, repo: &str, p: &Pill, view: &Entity<Osier>, cx: &App) -> Div {
    let theme = cx.theme();
    let icon = match p.kind {
        Choice::Account => Lucide::Github,
        Choice::Pull => Lucide::ArrowDown,
        Choice::Update => Lucide::FolderSync,
    };
    let (v, repo, kind) = (view.clone(), repo.to_string(), p.kind);
    let id = format!("choice:{}:{repo_ix}", p.kind.id());
    h_flex()
        .test_id(id.clone())
        .flex_none()
        .items_center()
        .gap_1p5()
        .pl_2p5()
        .pr_0p5()
        .py_0p5()
        .rounded_full()
        .border_1()
        .border_color(theme.border)
        .bg(theme.secondary)
        .text_xs()
        .child(Icon::new(icon).xsmall().text_color(theme.muted_foreground))
        .child(div().text_color(theme.muted_foreground).child(p.label))
        .child(div().font_medium().child(p.value.clone()))
        .when_some(p.note, |this, note| {
            this.child(
                div()
                    .text_color(theme.muted_foreground)
                    .child(format!("· {note}")),
            )
        })
        .child(
            Button::new(SharedString::from(format!("forget-{id}")))
                .test_id(format!("choice-forget:{}:{repo_ix}", p.kind.id()))
                .ghost()
                .xsmall()
                .rounded_full()
                .icon(Lucide::X)
                .tooltip("Forget this choice: Osier asks again next time")
                .on_click(move |_, _, cx| {
                    v.update(cx, |o, cx| o.forget_choice(&repo, kind, cx));
                }),
        )
}

/// The Projects page group.
pub(crate) fn group(view: &Entity<Osier>, cx: &App) -> SettingGroup {
    let v = view.clone();
    highlightable(
        SettingGroup::new(),
        SettingsTarget::PER_PROJECT_CHOICES,
        view,
        cx,
    )
    .title("Per-project choices")
    .description(
        "What Osier remembers for each project: the GitHub account to use, how to pull when \
         the branch has diverged, and how Update project joins commits. Forget one with its X \
         and Osier asks again next time.",
    )
    .item(SettingItem::render(move |options, _, cx| {
        let o = v.read(cx);
        let rows = choices(o);
        let theme = cx.theme();
        let mut col = v_flex().gap_2().w_full().test_id("project-choices");
        if rows.is_empty() {
            return col.child(div().text_sm().text_color(theme.muted_foreground).child(
                "Nothing remembered yet. Choices appear here when you pick an account, a \
                     pull method or an Update project method for a project.",
            ));
        }
        for (ix, (repo, pills)) in rows.iter().enumerate() {
            let path = Path::new(repo);
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| repo.clone());
            col = col.child(
                v_flex()
                    .test_id(format!("project-choice-row:{ix}"))
                    .gap_2()
                    .p_3()
                    .rounded(theme.radius)
                    .border_1()
                    .border_color(theme.border)
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(Icon::new(Lucide::FolderGit2).small())
                            .child(div().text_sm().font_medium().child(name))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(crate::format::tilde(path)),
                            ),
                    )
                    .child(
                        h_flex()
                            .flex_wrap()
                            .gap_2()
                            .children(pills.iter().map(|p| pill(ix, repo, p, &v, cx))),
                    ),
            );
        }
        let v2 = v.clone();
        col.child(
            h_flex().pt_1().child(
                Button::new("reset-all-choices")
                    .test_id("reset-all-choices")
                    .outline()
                    .with_size(options.size())
                    .label("Forget all")
                    .on_click(move |_, _, cx| {
                        v2.update(cx, |o, cx| o.clear_all_project_choices(cx));
                    }),
            ),
        )
    }))
}

/// Accounts page: where the per-project account choices went, as a card that opens them.
pub(crate) fn accounts_pointer(view: &Entity<Osier>) -> SettingGroup {
    let v = view.clone();
    SettingGroup::new()
        .title("Per-project choices")
        .item(SettingItem::render(move |_, _, cx| {
            let theme = cx.theme();
            let v = v.clone();
            h_flex()
                .id("accounts-choices-link")
                .test_id("accounts-choices-link")
                .w_full()
                .gap_3()
                .items_center()
                .p_3()
                .rounded(theme.radius)
                .border_1()
                .border_color(theme.info.opacity(0.4))
                .bg(theme.info.opacity(0.08))
                .cursor_pointer()
                .hover(|s| s.bg(theme.info.opacity(0.14)))
                .child(Icon::new(Lucide::Info).small().text_color(theme.info))
                .child(div().flex_1().min_w_0().text_sm().child(
                    "Per-project account choices can be found in Projects > Per-project choices.",
                ))
                .child(
                    Icon::new(Lucide::ArrowRight)
                        .small()
                        .text_color(theme.muted_foreground),
                )
                .on_click(move |_, window, cx| {
                    // Settings is a dialog: close it, then reopen it on the Projects group.
                    window.close_dialog(cx);
                    let v = v.clone();
                    window.defer(cx, move |window, cx| {
                        v.update(cx, |o, cx| {
                            o.open_settings_at(SettingsTarget::PER_PROJECT_CHOICES, window, cx)
                        });
                    });
                })
        }))
}
