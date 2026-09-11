use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use osier_git::ops::remote::{FetchOptions, PullOptions, PullStrategy, PushOptions};

use crate::remote_ops::RemoteOp;
use gpui_kit::component::separator::Separator;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, StyledExt as _, TitleBar, h_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app::Osier;
use crate::format::tilde;

impl Osier {
    pub(crate) fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let project = self.active_project();
        let branch = project
            .and_then(|p| p.snapshot.as_ref())
            .and_then(|s| s.status.branch.head.clone())
            .unwrap_or_else(|| "detached".into());
        let has_project = project.is_some();
        let busy = !has_project || self.remote_job.is_some();
        let later = "Available in a later phase";

        TitleBar::new().child(
            h_flex()
                .size_full()
                .px_2()
                .pr_3()
                .gap_2()
                .child(Icon::new(Lucide::FolderGit2))
                .when_some(project, |this, p| {
                    this.child(div().text_sm().font_medium().child(p.name.clone()))
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(tilde(&p.path)),
                        )
                        .child(Separator::vertical().h_4().mx_1())
                        .child(
                            Button::new("branch")
                                .ghost()
                                .small()
                                .icon(Lucide::GitBranch)
                                .label(branch)
                                .tooltip("Branches")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.set_tab(crate::app::Tab::Branches, cx)
                                })),
                        )
                })
                .when(!has_project, |this| {
                    this.child(div().text_sm().font_medium().child("Osier"))
                })
                .child(div().flex_1())
                .child(
                    Button::new("open-editor")
                        .ghost()
                        .small()
                        .icon(Lucide::SquareArrowOutUpRight)
                        .tooltip("Open project in editor")
                        .disabled(!has_project)
                        .on_click(
                            cx.listener(|this, _, window, cx| {
                                this.open_in_editor(None, window, cx)
                            }),
                        ),
                )
                .child(
                    Button::new("refresh")
                        .ghost()
                        .small()
                        .icon(Lucide::RefreshCw)
                        .label("Refresh")
                        .tooltip("Refresh (ctrl-r)")
                        .disabled(!has_project)
                        .on_click(cx.listener(|this, _, _, cx| this.refresh_active(cx))),
                )
                .child(Separator::vertical().h_4().mx_1())
                .child(self.remote_menu_button("fetch", Lucide::CloudDownload, "Fetch", busy, cx))
                .child(self.remote_menu_button("pull", Lucide::ArrowDown, "Pull", busy, cx))
                .child(self.remote_menu_button("push", Lucide::ArrowUp, "Push", busy, cx))
                .child(Separator::vertical().h_4().mx_1())
                .child(
                    Button::new("stash")
                        .ghost()
                        .small()
                        .icon(Lucide::Archive)
                        .tooltip("Stash changes")
                        .disabled(!has_project)
                        .on_click(cx.listener(|this, _, window, cx| this.prompt_stash(window, cx))),
                )
                .child(
                    Button::new("patch")
                        .ghost()
                        .small()
                        .icon(Lucide::ClipboardCopy)
                        .disabled(true)
                        .tooltip(later),
                )
                .child(
                    Button::new("command-log")
                        .ghost()
                        .small()
                        .icon(Lucide::SquareTerminal)
                        .toggled(self.command_log_visible)
                        .tooltip("Git command log (ctrl-`)")
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_command_log(cx))),
                )
                .child(
                    Button::new("theme")
                        .ghost()
                        .small()
                        .icon(if cx.theme().mode.is_dark() {
                            Lucide::Sun
                        } else {
                            Lucide::Moon
                        })
                        .tooltip("Toggle light/dark (ctrl-shift-t)")
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_theme(cx))),
                )
                .child(
                    Button::new("settings")
                        .ghost()
                        .small()
                        .icon(Lucide::Settings)
                        .disabled(true)
                        .tooltip(later),
                ),
        )
    }

    /// Fetch / Pull / Push button opening a menu of variants.
    fn remote_menu_button(
        &self,
        id: &'static str,
        icon: Lucide,
        label: &'static str,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let weak = cx.entity().downgrade();
        let item = move |label: &str, op: RemoteOp| {
            let weak = weak.clone();
            PopupMenuItem::new(label.to_string()).on_click(move |_, window, cx| {
                let op = op.clone();
                let _ = weak.update(cx, |this, cx| this.start_remote(op, window, cx));
            })
        };
        let weak2 = cx.entity().downgrade();
        let weak3 = cx.entity().downgrade();
        let has_gh = !self.gh_accounts.is_empty();
        Button::new(id)
            .ghost()
            .small()
            .icon(icon)
            .label(label)
            .disabled(disabled)
            .dropdown_menu(move |menu, _, _| match id {
                "fetch" => menu
                    .item(item(
                        "Fetch all remotes",
                        RemoteOp::Fetch(FetchOptions::default()),
                    ))
                    .item(item(
                        "Fetch all and prune deleted branches",
                        RemoteOp::Fetch(FetchOptions {
                            prune: true,
                            ..Default::default()
                        }),
                    ))
                    .item(item(
                        "Fetch origin",
                        RemoteOp::Fetch(FetchOptions {
                            remote: Some("origin".into()),
                            ..Default::default()
                        }),
                    )),
                "pull" => menu
                    .item(item(
                        "Pull (fast-forward only)",
                        RemoteOp::Pull(PullOptions::default()),
                    ))
                    .item(item(
                        "Pull with rebase",
                        RemoteOp::Pull(PullOptions {
                            strategy: PullStrategy::Rebase,
                            ..Default::default()
                        }),
                    ))
                    .item(item(
                        "Pull with merge",
                        RemoteOp::Pull(PullOptions {
                            strategy: PullStrategy::Merge,
                            ..Default::default()
                        }),
                    )),
                _ => {
                    let w_force = weak2.clone();
                    let w_acc = weak3.clone();
                    let w_clear = weak3.clone();
                    let mut m = menu
                        .item(item("Push", RemoteOp::Push(PushOptions::default())))
                        .item(item(
                            "Push and set upstream (origin)",
                            RemoteOp::Push(PushOptions {
                                remote: Some("origin".into()),
                                set_upstream: true,
                                ..Default::default()
                            }),
                        ))
                        .item(PopupMenuItem::new("Force push with lease…").on_click(
                            move |_, window, cx| {
                                let _ = w_force.update(cx, |this, cx| {
                                    this.confirm_force_push(PushOptions::default(), window, cx)
                                });
                            },
                        ));
                    if has_gh {
                        m = m
                            .separator()
                            .item(PopupMenuItem::new("Choose GitHub account…").on_click(
                                move |_, window, cx| {
                                    let _ = w_acc
                                        .update(cx, |this, cx| this.choose_account(window, cx));
                                },
                            ))
                            .item(
                                PopupMenuItem::new("Forget account choice for this project")
                                    .on_click(move |_, window, cx| {
                                        let _ = w_clear.update(cx, |this, cx| {
                                            this.clear_project_account(window, cx)
                                        });
                                    }),
                            );
                    }
                    m
                }
            })
    }
}
