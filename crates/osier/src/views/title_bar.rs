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
use crate::test_ids::TestId as _;

type MenuHandler = Box<dyn Fn(&mut Osier, &mut Window, &mut Context<Osier>)>;

impl Osier {
    /// The theme button: a menu with mode, dark theme, light theme and Dux Shell sync.
    /// The keyboard shortcut (ctrl-shift-t) still toggles dark/light directly.
    fn render_theme_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        use crate::themes::ThemeStore;
        use osier_config::settings::ThemeMode;

        let w = cx.weak_entity();
        let a = self.settings.appearance.clone();
        let store = ThemeStore::global(cx);
        let dark_names = store.names(true);
        let light_names = store.names(false);
        let dux_available = store.dux_available();
        let dux_on = a.dux_shell_sync.unwrap_or(true) && dux_available;
        let active = cx.theme().theme_name().clone();
        Button::new("theme")
            .test_id("theme")
            .ghost()
            .small()
            .icon(if cx.theme().mode.is_dark() {
                Lucide::Sun
            } else {
                Lucide::Moon
            })
            .tooltip(format!("Theme: {active} (ctrl-shift-t toggles dark/light)"))
            .dropdown_menu(move |mut menu, _, _| {
                let act = |f: MenuHandler| {
                    let w = w.clone();
                    move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                        let _ = w.update(cx, |this, cx| f(this, window, cx));
                    }
                };
                menu = menu.label("Mode");
                for (label, mode) in [
                    ("Dark", ThemeMode::Dark),
                    ("Light", ThemeMode::Light),
                    ("System", ThemeMode::System),
                ] {
                    menu = menu.item(PopupMenuItem::new(label).checked(a.mode == mode).on_click(
                        act(Box::new(move |this, _, cx| this.set_theme_mode(mode, cx))),
                    ));
                }
                menu = menu.separator().label("Dark theme");
                for name in &dark_names {
                    let n = name.clone();
                    menu = menu.item(
                        PopupMenuItem::new(name.clone())
                            .checked(!dux_on && a.dark_theme == name.as_ref())
                            .on_click(act(Box::new(move |this, _, cx| this.set_theme(&n, cx)))),
                    );
                }
                menu = menu.separator().label("Light theme");
                for name in &light_names {
                    let n = name.clone();
                    menu = menu.item(
                        PopupMenuItem::new(name.clone())
                            .checked(!dux_on && a.light_theme == name.as_ref())
                            .on_click(act(Box::new(move |this, _, cx| this.set_theme(&n, cx)))),
                    );
                }
                menu.separator()
                    .item(
                        PopupMenuItem::new(if dux_available {
                            "Follow Dux Shell colours"
                        } else {
                            "Follow Dux Shell colours (not detected)"
                        })
                        .disabled(!dux_available)
                        .checked(dux_on)
                        .on_click(act(Box::new(move |this, _, cx| {
                            this.set_dux_sync(!dux_on, cx)
                        }))),
                    )
                    .item(
                        PopupMenuItem::new("Open themes folder")
                            .icon(Lucide::FolderOpen)
                            .on_click(act(Box::new(|this, window, cx| {
                                this.open_themes_dir(window, cx)
                            }))),
                    )
            })
    }

    /// Patch actions for the whole working tree, plus applying from clipboard or file.
    fn render_patch_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        use crate::app::Tab;
        let w = cx.weak_entity();
        let all = self.all_changes_source();
        let has_project = self.active_project().is_some();
        Button::new("patch")
            .test_id("patch")
            .ghost()
            .small()
            .icon(Lucide::ClipboardCopy)
            .disabled(!has_project)
            .tooltip("Patches")
            .dropdown_menu(move |menu, _, _| {
                let act = |f: MenuHandler| {
                    let w = w.clone();
                    move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                        let _ = w.update(cx, |this, cx| f(this, window, cx));
                    }
                };
                let (a1, a2, a3) = (all.clone(), all.clone(), all.clone());
                menu.item(
                    PopupMenuItem::new("Apply patch from clipboard…")
                        .icon(Lucide::ClipboardPaste)
                        .on_click(act(Box::new(|this, window, cx| {
                            this.apply_patch_from_clipboard(window, cx)
                        }))),
                )
                .item(
                    PopupMenuItem::new("Apply patch from file…")
                        .icon(Lucide::FileInput)
                        .on_click(act(Box::new(|this, window, cx| {
                            this.apply_patch_from_file(window, cx)
                        }))),
                )
                .separator()
                .item(
                    PopupMenuItem::new("Copy all changes as patch")
                        .disabled(a1.is_none())
                        .on_click(act(Box::new(move |this, window, cx| {
                            if let Some(s) = a1.clone() {
                                this.copy_patch(s, window, cx)
                            }
                        }))),
                )
                .item(
                    PopupMenuItem::new("Save all changes to Patches…")
                        .disabled(a2.is_none())
                        .on_click(act(Box::new(move |this, window, cx| {
                            if let Some(s) = a2.clone() {
                                this.save_patch_prompt(s, "Working changes".into(), window, cx)
                            }
                        }))),
                )
                .item(
                    PopupMenuItem::new("Shelve all changes…")
                        .icon(Lucide::Package)
                        .disabled(a3.is_none())
                        .on_click(act(Box::new(|this, window, cx| {
                            this.shelve_prompt(Vec::new(), window, cx)
                        }))),
                )
                .separator()
                .item(
                    PopupMenuItem::new("Patches tab")
                        .on_click(act(Box::new(|this, _, cx| this.set_tab(Tab::Patches, cx)))),
                )
            })
    }

    /// The branch button: switch, merge into current or rebase onto any local branch, plus
    /// new branch and a jump to the Branches tab.
    fn render_branch_menu(&self, current: String, cx: &mut Context<Self>) -> impl IntoElement {
        use crate::app::Tab;
        use osier_git::ops::integrate::MergeMode;
        let others: Vec<String> = self
            .active_project()
            .and_then(|p| p.snapshot.as_ref())
            .map(|s| {
                s.branches
                    .iter()
                    .filter(|b| !b.is_remote && b.name != current)
                    .map(|b| b.name.clone())
                    .take(30)
                    .collect()
            })
            .unwrap_or_default();
        let w = cx.weak_entity();
        Button::new("branch")
            .test_id("branch")
            .ghost()
            .small()
            .icon(Lucide::GitBranch)
            .when(!self.compact, |b| b.label(current.clone()))
            .tooltip(format!("Branch {current}"))
            .dropdown_menu(move |menu, window, cx| {
                let act = |f: MenuHandler| {
                    let w = w.clone();
                    move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                        let _ = w.update(cx, |this, cx| f(this, window, cx));
                    }
                };
                let mut menu = menu
                    .item(
                        PopupMenuItem::new("New branch…")
                            .icon(Lucide::Plus)
                            .on_click(act(Box::new(|this, window, cx| {
                                this.prompt_new_branch(None, window, cx)
                            }))),
                    )
                    .item(
                        PopupMenuItem::new("Branches tab")
                            .on_click(act(Box::new(|this, _, cx| this.set_tab(Tab::Branches, cx)))),
                    );
                if !others.is_empty() {
                    menu = menu.separator().label("Local branches");
                }
                for name in &others {
                    let (n1, n2, n3) = (name.clone(), name.clone(), name.clone());
                    let cur = current.clone();
                    let w = w.clone();
                    menu = menu.submenu(name.clone(), window, cx, move |sub, _, _| {
                        let act = |f: MenuHandler| {
                            let w = w.clone();
                            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                let _ = w.update(cx, |this, cx| f(this, window, cx));
                            }
                        };
                        let (n1, n2, n3) = (n1.clone(), n2.clone(), n3.clone());
                        sub.item(PopupMenuItem::new("Check out").on_click(act(Box::new(
                            move |this, window, cx| {
                                this.checkout_branch(n1.clone(), false, window, cx)
                            },
                        ))))
                        .item(
                            PopupMenuItem::new(format!("Merge into {cur}")).on_click(act(
                                Box::new(move |this, window, cx| {
                                    this.merge_branch(n2.clone(), MergeMode::Default, window, cx)
                                }),
                            )),
                        )
                        .item(
                            PopupMenuItem::new(format!("Rebase {cur} onto it…")).on_click(act(
                                Box::new(move |this, window, cx| {
                                    this.confirm_rebase_onto(n3.clone(), window, cx)
                                }),
                            )),
                        )
                    });
                }
                menu
            })
    }

    pub(crate) fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let project = self.active_project();
        let branch = project
            .and_then(|p| p.snapshot.as_ref())
            .and_then(|s| s.status.branch.head.clone())
            .unwrap_or_else(|| "detached".into());
        let has_project = project.is_some();
        let busy = !has_project || self.remote_job.is_some();

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
                            // Long paths shrink with an ellipsis instead of being cut by the
                            // window edge. (A fade overlay would also dim the end of a path
                            // that fits, since this element is only as wide as its text.)
                            div()
                                .min_w_0()
                                .flex_shrink(1.)
                                .truncate()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(tilde(&p.path)),
                        )
                        .child(Separator::vertical().h_4().mx_1())
                        .child(self.render_branch_menu(branch.clone(), cx))
                })
                .when(!has_project, |this| {
                    this.child(div().text_sm().font_medium().child("Osier"))
                })
                .child(div().flex_1())
                .child(
                    Button::new("open-editor")
                        .test_id("open-editor")
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
                        .test_id("refresh")
                        .ghost()
                        .small()
                        .icon(Lucide::RefreshCw)
                        .when(!self.compact, |b| b.label("Refresh"))
                        .tooltip("Refresh (ctrl-r)")
                        .disabled(!has_project)
                        .on_click(cx.listener(|this, _, _, cx| this.refresh_active(cx))),
                )
                .child(Separator::vertical().h_4().mx_1())
                .child(
                    Button::new("update-project")
                        .test_id("update-project")
                        .ghost()
                        .small()
                        .icon(Lucide::FolderSync)
                        .when(!self.compact, |b| b.label("Update project"))
                        .tooltip("Update project: fetch, then merge or rebase (ctrl-t)")
                        .disabled(busy)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.update_project(window, cx)),
                        ),
                )
                .child(self.remote_menu_button("fetch", Lucide::CloudDownload, "Fetch", busy, cx))
                .child(self.remote_menu_button("pull", Lucide::ArrowDown, "Pull", busy, cx))
                .child(self.remote_menu_button("push", Lucide::ArrowUp, "Push", busy, cx))
                .child(Separator::vertical().h_4().mx_1())
                .child(
                    Button::new("stash")
                        .test_id("stash")
                        .ghost()
                        .small()
                        .icon(Lucide::Archive)
                        .tooltip("Stash changes")
                        .disabled(!has_project)
                        .on_click(cx.listener(|this, _, window, cx| this.prompt_stash(window, cx))),
                )
                .child(self.render_patch_menu(cx))
                .child(
                    Button::new("command-log")
                        .test_id("command-log")
                        .ghost()
                        .small()
                        .icon(Lucide::SquareTerminal)
                        .toggled(self.command_log_visible)
                        .tooltip("Git command log (ctrl-`)")
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_command_log(cx))),
                )
                .child(self.render_theme_menu(cx))
                .child(
                    Button::new("settings")
                        .test_id("settings")
                        .ghost()
                        .small()
                        .icon(Lucide::Settings)
                        .tooltip("Settings (ctrl-,)")
                        .on_click(
                            cx.listener(|this, _, window, cx| this.open_settings(window, cx)),
                        ),
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
        let mut button = Button::new(id)
            .test_id(id)
            .ghost()
            .small()
            .icon(icon)
            .disabled(disabled);
        if self.compact {
            button = button.tooltip(label);
        } else {
            button = button.label(label);
        }
        button.dropdown_menu(move |menu, _, _| match id {
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
                    m =
                        m.separator()
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
