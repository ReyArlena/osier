use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use osier_git::refs::Branch;

use crate::app::Osier;
use crate::row_actions::RefRow;
use crate::test_ids::TestId as _;
use osier_git::ops::integrate::MergeMode;

impl Osier {
    pub(crate) fn render_branches(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let menu_buttons = self.settings.appearance.row_menu_buttons;
        let selected_ref = self.ref_selected.clone();
        let Some(project) = self.active_project() else {
            return self
                .empty_state(
                    "No repository open",
                    "Open a repository to see its branches.",
                    cx,
                )
                .into_any_element();
        };
        let Some(snapshot) = &project.snapshot else {
            return self
                .empty_state(&project.name, "Reading branches…", cx)
                .into_any_element();
        };
        let local: Vec<&Branch> = snapshot.branches.iter().filter(|b| !b.is_remote).collect();
        let remote: Vec<&Branch> = snapshot.branches.iter().filter(|b| b.is_remote).collect();
        let muted = cx.theme().muted_foreground;
        let weak = cx.entity().downgrade();

        let mut list = v_flex()
            .flex_1()
            .min_h_0()
            .id("branches-list")
            .overflow_y_scrollbar();
        for (title, entries, icon) in [
            ("Local", &local, Lucide::GitBranch),
            ("Remote", &remote, Lucide::Cloud),
        ] {
            if entries.is_empty() {
                continue;
            }
            list = list.child(
                h_flex()
                    .px_3()
                    .py_1()
                    .gap_2()
                    .text_xs()
                    .font_medium()
                    .text_color(muted)
                    .child(title)
                    .child(entries.len().to_string()),
            );
            for (ix, b) in entries.iter().enumerate() {
                let name = b.name.clone();
                let is_remote = b.is_remote;
                let is_head = b.is_head;
                let checkout_name = name.clone();
                let menu_name = name.clone();
                let weak = weak.clone();
                let checkout = (!is_head).then(|| (weak.clone(), checkout_name.clone()));
                let inner = if is_remote {
                    let (wm, wc) = (weak.clone(), weak.clone());
                    let (m1, c1) = (menu_name.clone(), menu_name.clone());
                    crate::row_menu::RowMenu::new(move |menu, _, _| {
                        let (wm, wc) = (wm.clone(), wc.clone());
                        let (m1, c1) = (m1.clone(), c1.clone());
                        menu.item(
                            crate::row_actions::menu_item(
                                "Merge into current branch",
                                crate::row_actions::RowKind::MergeIntoCurrent,
                            )
                            .on_click(move |_, window, cx| {
                                let _ = wm.update(cx, |this, cx| {
                                    this.merge_branch(m1.clone(), MergeMode::Default, window, cx)
                                });
                            }),
                        )
                        .item(
                            crate::row_actions::menu_item(
                                "Compare with current branch",
                                crate::row_actions::RowKind::CompareWithCurrent,
                            )
                            .on_click(move |_, _, cx| {
                                let _ = wc.update(cx, |this, cx| {
                                    let current = this
                                        .active_project()
                                        .and_then(|p| p.snapshot.as_ref())
                                        .and_then(|s| s.status.branch.head.clone())
                                        .unwrap_or_else(|| "HEAD".into());
                                    this.select_compare(current, c1.clone(), true, cx);
                                });
                            }),
                        )
                    })
                } else {
                    crate::row_menu::RowMenu::new(move |menu, _, _| {
                        let n1 = menu_name.clone();
                        let n2 = menu_name.clone();
                        let n3 = menu_name.clone();
                        let n4 = menu_name.clone();
                        let (w1, w2, w3, w4) =
                            (weak.clone(), weak.clone(), weak.clone(), weak.clone());
                        let (wm1, wm2, wm3, wr, wc) = (
                            weak.clone(),
                            weak.clone(),
                            weak.clone(),
                            weak.clone(),
                            weak.clone(),
                        );
                        let (m1, m2, m3, r1, c1) = (
                            menu_name.clone(),
                            menu_name.clone(),
                            menu_name.clone(),
                            menu_name.clone(),
                            menu_name.clone(),
                        );
                        let menu = menu
                            .item(
                                crate::row_actions::menu_item(
                                    "Merge into current branch",
                                    crate::row_actions::RowKind::MergeIntoCurrent,
                                )
                                .disabled(is_head)
                                .on_click(move |_, window, cx| {
                                    let _ = wm1.update(cx, |this, cx| {
                                        this.merge_branch(
                                            m1.clone(),
                                            MergeMode::Default,
                                            window,
                                            cx,
                                        )
                                    });
                                }),
                            )
                            .item(
                                PopupMenuItem::new("Merge into current (always a merge commit)")
                                    .disabled(is_head)
                                    .on_click(move |_, window, cx| {
                                        let _ = wm2.update(cx, |this, cx| {
                                            this.merge_branch(
                                                m2.clone(),
                                                MergeMode::NoFastForward,
                                                window,
                                                cx,
                                            )
                                        });
                                    }),
                            )
                            .item(
                                PopupMenuItem::new("Squash merge into current (no commit)")
                                    .disabled(is_head)
                                    .on_click(move |_, window, cx| {
                                        let _ = wm3.update(cx, |this, cx| {
                                            this.merge_branch(
                                                m3.clone(),
                                                MergeMode::Squash,
                                                window,
                                                cx,
                                            )
                                        });
                                    }),
                            )
                            .item(
                                crate::row_actions::menu_item(
                                    "Rebase current branch onto this…",
                                    crate::row_actions::RowKind::RebaseCurrentOnto,
                                )
                                .disabled(is_head)
                                .on_click(move |_, window, cx| {
                                    let _ = wr.update(cx, |this, cx| {
                                        this.confirm_rebase_onto(r1.clone(), window, cx)
                                    });
                                }),
                            )
                            .item(
                                crate::row_actions::menu_item(
                                    "Compare with current branch",
                                    crate::row_actions::RowKind::CompareWithCurrent,
                                )
                                .disabled(is_head)
                                .on_click(move |_, _, cx| {
                                    let _ = wc.update(cx, |this, cx| {
                                        let current = this
                                            .active_project()
                                            .and_then(|p| p.snapshot.as_ref())
                                            .and_then(|s| s.status.branch.head.clone())
                                            .unwrap_or_else(|| "HEAD".into());
                                        this.select_compare(current, c1.clone(), true, cx);
                                    });
                                }),
                            )
                            .separator();
                        menu.item(
                            crate::row_actions::menu_item(
                                "New branch from here…",
                                crate::row_actions::RowKind::NewBranchHere,
                            )
                            .on_click(move |_, window, cx| {
                                let _ = w1.update(cx, |this, cx| {
                                    this.prompt_new_branch(Some(n1.clone()), window, cx)
                                });
                            }),
                        )
                        .item(
                            crate::row_actions::menu_item(
                                "Rename…",
                                crate::row_actions::RowKind::Rename,
                            )
                            .on_click(move |_, window, cx| {
                                let _ = w2.update(cx, |this, cx| {
                                    this.prompt_rename_branch(n2.clone(), window, cx)
                                });
                            }),
                        )
                        .separator()
                        .item(
                            crate::row_actions::menu_item(
                                "Delete",
                                crate::row_actions::RowKind::Delete,
                            )
                            .disabled(is_head)
                            .on_click(move |_, window, cx| {
                                let _ = w3.update(cx, |this, cx| {
                                    this.confirm_delete_branch(n3.clone(), false, window, cx)
                                });
                            }),
                        )
                        .item(
                            PopupMenuItem::new("Force delete")
                                .disabled(is_head)
                                .on_click(move |_, window, cx| {
                                    let _ = w4.update(cx, |this, cx| {
                                        this.confirm_delete_branch(n4.clone(), true, window, cx)
                                    });
                                }),
                        )
                    })
                };
                // Right-click adds Checkout, which the row otherwise shows as a hover button.
                let row_menu = crate::row_menu::RowMenu::new(move |menu, window, cx| {
                    let menu = match &checkout {
                        Some((w, n)) => {
                            let (w, n) = (w.clone(), n.clone());
                            menu.item(
                                crate::row_actions::menu_item(
                                    "Checkout",
                                    crate::row_actions::RowKind::Checkout,
                                )
                                .on_click(move |_, window, cx| {
                                    let _ = w.update(cx, |this, cx| {
                                        this.checkout_branch(n.clone(), is_remote, window, cx)
                                    });
                                }),
                            )
                            .separator()
                        }
                        None => menu,
                    };
                    (inner.build())(menu, window, cx)
                });
                list =
                    list.child(
                        h_flex()
                            .id((title, ix))
                            .cursor_pointer()
                            .when(
                                selected_ref.as_ref()
                                    == Some(&RefRow::Branch {
                                        name: b.name.clone(),
                                        remote: b.is_remote,
                                    }),
                                |this| this.bg(cx.theme().list_active),
                            )
                            .on_click(cx.listener({
                                let row = RefRow::Branch {
                                    name: b.name.clone(),
                                    remote: b.is_remote,
                                };
                                move |this, _, _, cx| this.select_ref(row.clone(), cx)
                            }))
                            .test_id(format!("branch-row:{}", b.name))
                            .group("row")
                            .px_3()
                            .py_1p5()
                            .gap_2()
                            .items_center()
                            .hover(|this| this.bg(cx.theme().list_hover))
                            .child(Icon::new(icon).xsmall())
                            .child(
                                div()
                                    .text_sm()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .when(b.is_head, |this| this.font_medium())
                                    .child(b.name.clone()),
                            )
                            .when(b.is_head, |this| {
                                this.child(Tag::primary().xsmall().child("current"))
                            })
                            .when(b.upstream_gone, |this| {
                                this.child(Tag::danger().xsmall().child("upstream gone"))
                            })
                            .when(b.ahead > 0, |this| {
                                this.child(
                                    div()
                                        .text_xs()
                                        .text_color(muted)
                                        .child(format!("↑{}", b.ahead)),
                                )
                            })
                            .when(b.behind > 0, |this| {
                                this.child(
                                    div()
                                        .text_xs()
                                        .text_color(muted)
                                        .child(format!("↓{}", b.behind)),
                                )
                            })
                            .child(
                                super::relative_time_label(
                                    SharedString::from(format!("branch-time:{}", b.name)),
                                    b.committed_at,
                                    cx,
                                )
                                .text_xs()
                                .text_color(muted)
                                .flex_none()
                                .w(px(78.))
                                .text_right(),
                            )
                            .child(
                                h_flex()
                                    .gap_1()
                                    .invisible()
                                    .group_hover("row", |this| this.visible())
                                    .when(!is_head, |this| {
                                        this.child(
                                            Button::new(SharedString::from(format!(
                                                "checkout-{title}-{ix}"
                                            )))
                                            .ghost()
                                            .xsmall()
                                            .label("Checkout")
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.checkout_branch(
                                                    checkout_name.clone(),
                                                    is_remote,
                                                    window,
                                                    cx,
                                                )
                                            })),
                                        )
                                    })
                                    .child(
                                        div()
                                            .test_id(format!("branch-menu:{}", b.name))
                                            .when(!menu_buttons, |d| d.hidden())
                                            .child(
                                                Button::new(SharedString::from(format!(
                                                    "branch-menu-{title}-{ix}"
                                                )))
                                                .ghost()
                                                .xsmall()
                                                .icon(Lucide::Ellipsis)
                                                .dropdown_menu(row_menu.build()),
                                            ),
                                    ),
                            )
                            .context_menu(row_menu.build()),
                    );
            }
        }

        // Tags, newest first.
        if !snapshot.tags.is_empty() {
            list = list.child(
                h_flex()
                    .px_3()
                    .py_1()
                    .gap_2()
                    .text_xs()
                    .font_medium()
                    .text_color(muted)
                    .child("Tags")
                    .child(snapshot.tags.len().to_string()),
            );
            let remotes: Vec<String> = snapshot.remotes.iter().map(|r| r.name.clone()).collect();
            for (ix, t) in snapshot.tags.iter().enumerate() {
                let name = t.name.clone();
                let target = t.target.clone();
                let weak = weak.clone();
                let remotes = remotes.clone();
                let row_menu = crate::row_menu::RowMenu::new(move |menu, _, _| {
                    let (n1, n2, n3) = (name.clone(), name.clone(), name.clone());
                    let (t1, t2) = (target.clone(), target.clone());
                    let (w1, w2, w3, w4) = (weak.clone(), weak.clone(), weak.clone(), weak.clone());
                    let mut menu = menu
                        .item(
                            crate::row_actions::menu_item(
                                "Show commit",
                                crate::row_actions::RowKind::ShowCommit,
                            )
                            .on_click(move |_, _, cx| {
                                let _ = w1
                                    .update(cx, |this, cx| this.select_commit_hash(t1.clone(), cx));
                            }),
                        )
                        .item(
                            crate::row_actions::menu_item(
                                "Check out (detached)",
                                crate::row_actions::RowKind::Checkout,
                            )
                            .on_click(move |_, window, cx| {
                                let _ = w2.update(cx, |this, cx| {
                                    this.checkout_commit(t2.clone(), window, cx)
                                });
                            }),
                        )
                        .item(
                            crate::row_actions::menu_item(
                                "Copy name",
                                crate::row_actions::RowKind::CopyName,
                            )
                            .on_click(move |_, window, cx| {
                                let _ = w3.update(cx, |this, cx| {
                                    this.copy_to_clipboard(n1.clone(), window, cx)
                                });
                            }),
                        )
                        .separator()
                        .item(
                            crate::row_actions::menu_item(
                                "Delete…",
                                crate::row_actions::RowKind::Delete,
                            )
                            .on_click(move |_, window, cx| {
                                let _ = w4.update(cx, |this, cx| {
                                    this.confirm_delete_tag(n2.clone(), window, cx)
                                });
                            }),
                        );
                    for r in &remotes {
                        let (w, n, r2) = (weak.clone(), n3.clone(), r.clone());
                        menu = menu.item(PopupMenuItem::new(format!("Delete on {r}…")).on_click(
                            move |_, window, cx| {
                                let _ = w.update(cx, |this, cx| {
                                    this.confirm_delete_remote_tag(
                                        n.clone(),
                                        r2.clone(),
                                        window,
                                        cx,
                                    )
                                });
                            },
                        ));
                    }
                    menu
                });
                list = list.child(
                    h_flex()
                        .id(("tag", ix))
                        .cursor_pointer()
                        .when(
                            selected_ref.as_ref() == Some(&RefRow::Tag(t.name.clone())),
                            |this| this.bg(cx.theme().list_active),
                        )
                        .on_click(cx.listener({
                            let row = RefRow::Tag(t.name.clone());
                            move |this, _, _, cx| this.select_ref(row.clone(), cx)
                        }))
                        .test_id(format!("tag-row:{}", t.name))
                        .group("row")
                        .px_3()
                        .py_1p5()
                        .gap_2()
                        .items_center()
                        .hover(|this| this.bg(cx.theme().list_hover))
                        .child(Icon::new(Lucide::Tag).xsmall())
                        .child(div().text_sm().flex_none().child(t.name.clone()))
                        .child(
                            div()
                                .text_xs()
                                .text_color(muted)
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(t.subject.clone()),
                        )
                        .when(t.annotated, |this| {
                            this.child(Tag::secondary().xsmall().child("annotated"))
                        })
                        .child(
                            div()
                                .text_xs()
                                .text_color(muted)
                                .flex_none()
                                .child(t.target[..t.target.len().min(7)].to_string()),
                        )
                        .child(
                            Button::new(("tag-menu", ix))
                                .when(!menu_buttons, |b| b.hidden())
                                .ghost()
                                .xsmall()
                                .icon(Lucide::Ellipsis)
                                .dropdown_menu(row_menu.build()),
                        )
                        .context_menu(row_menu.build()),
                );
            }
        }

        // Worktrees: the main checkout plus linked ones.
        if snapshot.worktrees.len() > 1 {
            list = list.child(
                h_flex()
                    .px_3()
                    .py_1()
                    .gap_2()
                    .text_xs()
                    .font_medium()
                    .text_color(muted)
                    .child("Worktrees")
                    .child(snapshot.worktrees.len().to_string()),
            );
            for (ix, w) in snapshot.worktrees.iter().enumerate() {
                let path = w.path.clone();
                let is_this =
                    Some(path.as_path()) == self.active_project().map(|p| p.path.as_path());
                let weak = weak.clone();
                let row_menu = crate::row_menu::RowMenu::new(move |menu, _, _| {
                    let (p1, p2) = (path.clone(), path.clone());
                    let (w1, w2) = (weak.clone(), weak.clone());
                    menu.item(
                        PopupMenuItem::new("Open as project")
                            .disabled(is_this)
                            .on_click(move |_, window, cx| {
                                let _ = w1.update(cx, |this, cx| {
                                    this.open_project_notify(&p1, window, cx)
                                });
                            }),
                    )
                    .separator()
                    .item(
                        PopupMenuItem::new("Remove worktree…")
                            .disabled(is_this)
                            .on_click(move |_, window, cx| {
                                let _ = w2.update(cx, |this, cx| {
                                    this.confirm_remove_worktree(p2.clone(), window, cx)
                                });
                            }),
                    )
                });
                list = list.child(
                    h_flex()
                        .id(("worktree", ix))
                        .px_3()
                        .py_1p5()
                        .gap_2()
                        .items_center()
                        .hover(|this| this.bg(cx.theme().list_hover))
                        .child(Icon::new(Lucide::FolderTree).xsmall())
                        .child(
                            div()
                                .text_sm()
                                .flex_none()
                                .child(w.branch.clone().unwrap_or_else(|| "detached".into())),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(muted)
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(crate::format::tilde(&w.path)),
                        )
                        .when(is_this, |this| {
                            this.child(Tag::primary().xsmall().child("this"))
                        })
                        .when(w.is_main && !is_this, |this| {
                            this.child(Tag::secondary().xsmall().child("main"))
                        })
                        .when(w.locked, |this| {
                            this.child(Tag::warning().xsmall().child("locked"))
                        })
                        .when(w.prunable, |this| {
                            this.child(Tag::danger().xsmall().child("missing"))
                        })
                        .child(
                            Button::new(("worktree-menu", ix))
                                .when(!menu_buttons, |b| b.hidden())
                                .ghost()
                                .xsmall()
                                .icon(Lucide::Ellipsis)
                                .dropdown_menu(row_menu.build()),
                        )
                        .context_menu(row_menu.build()),
                );
            }
        }

        // Submodules: path, where it points, whether it is checked out.
        if !snapshot.submodules.is_empty() {
            list = list.child(
                h_flex()
                    .px_3()
                    .py_1()
                    .gap_2()
                    .text_xs()
                    .font_medium()
                    .text_color(muted)
                    .child("Submodules")
                    .child(snapshot.submodules.len().to_string())
                    .child(div().flex_1())
                    .child(
                        Button::new("submodules-update-all")
                            .test_id("submodules-update-all")
                            .ghost()
                            .xsmall()
                            .label("Update all")
                            .tooltip("git submodule update --init --recursive")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.update_submodules(Vec::new(), window, cx)
                            })),
                    ),
            );
            for (ix, s) in snapshot.submodules.iter().enumerate() {
                let path = s.path.clone();
                list = list.child(
                    h_flex()
                        .id(("submodule", ix))
                        .px_3()
                        .py_1p5()
                        .gap_2()
                        .items_center()
                        .hover(|this| this.bg(cx.theme().list_hover))
                        .child(Icon::new(Lucide::Package).xsmall())
                        .child(div().text_sm().flex_none().child(s.path.clone()))
                        .child(
                            div()
                                .text_xs()
                                .text_color(muted)
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(s.url.clone()),
                        )
                        .child(match &s.head {
                            Some(h) => Tag::secondary()
                                .xsmall()
                                .child(h[..h.len().min(7)].to_string()),
                            None => Tag::warning().xsmall().child("not initialised"),
                        })
                        .child(
                            Button::new(("submodule-update", ix))
                                .ghost()
                                .xsmall()
                                .label("Update")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.update_submodules(vec![path.clone()], window, cx)
                                })),
                        ),
                );
            }
        }

        // Remotes with their URLs and a menu.
        if !snapshot.remotes.is_empty() {
            list = list.child(
                h_flex()
                    .px_3()
                    .py_1()
                    .gap_2()
                    .text_xs()
                    .font_medium()
                    .text_color(muted)
                    .child("Remotes")
                    .child(snapshot.remotes.len().to_string()),
            );
            for (ix, r) in snapshot.remotes.iter().enumerate() {
                let name = r.name.clone();
                let url = r.fetch_url.clone();
                let weak = weak.clone();
                let row_menu = crate::row_menu::RowMenu::new(move |menu, _, _| {
                    let (n1, n2, n3, u1) = (name.clone(), name.clone(), name.clone(), url.clone());
                    let (w1, w2, w3) = (weak.clone(), weak.clone(), weak.clone());
                    menu.item(
                        crate::row_actions::menu_item(
                            "Edit URL…",
                            crate::row_actions::RowKind::EditRemoteUrl,
                        )
                        .on_click(move |_, window, cx| {
                            let _ = w1.update(cx, |this, cx| {
                                this.prompt_remote_url(n1.clone(), u1.clone(), window, cx)
                            });
                        }),
                    )
                    .item(
                        crate::row_actions::menu_item(
                            "Rename…",
                            crate::row_actions::RowKind::Rename,
                        )
                        .on_click(move |_, window, cx| {
                            let _ = w2.update(cx, |this, cx| {
                                this.prompt_rename_remote(n2.clone(), window, cx)
                            });
                        }),
                    )
                    .separator()
                    .item(
                        crate::row_actions::menu_item(
                            "Remove",
                            crate::row_actions::RowKind::Delete,
                        )
                        .on_click(move |_, window, cx| {
                            let _ = w3.update(cx, |this, cx| {
                                this.confirm_remove_remote(n3.clone(), window, cx)
                            });
                        }),
                    )
                });
                list = list.child(
                    h_flex()
                        .id(("remote", ix))
                        .cursor_pointer()
                        .when(
                            selected_ref.as_ref() == Some(&RefRow::Remote(r.name.clone())),
                            |this| this.bg(cx.theme().list_active),
                        )
                        .on_click(cx.listener({
                            let row = RefRow::Remote(r.name.clone());
                            move |this, _, _, cx| this.select_ref(row.clone(), cx)
                        }))
                        .px_3()
                        .py_1p5()
                        .gap_2()
                        .items_center()
                        .hover(|this| this.bg(cx.theme().list_hover))
                        .child(Icon::new(Lucide::Globe).xsmall())
                        .child(div().text_sm().flex_none().child(r.name.clone()))
                        .child(
                            div()
                                .text_xs()
                                .text_color(muted)
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(r.fetch_url.clone()),
                        )
                        .child(
                            Button::new(("remote-menu", ix))
                                .when(!menu_buttons, |b| b.hidden())
                                .ghost()
                                .xsmall()
                                .icon(Lucide::Ellipsis)
                                .dropdown_menu(row_menu.build()),
                        )
                        .context_menu(row_menu.build()),
                );
            }
        }

        v_flex()
            .size_full()
            .child(
                h_flex()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(div().text_sm().font_medium().flex_none().child("Branches"))
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(format!("{} local, {} remote", local.len(), remote.len())),
                    )
                    .child(
                        Button::new("add-worktree")
                            .test_id("add-worktree")
                            .ghost()
                            .xsmall()
                            .icon(Lucide::FolderTree)
                            .label("Worktree")
                            .tooltip("Add a worktree")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.prompt_add_worktree(window, cx)
                            })),
                    )
                    .child(
                        Button::new("add-remote")
                            .test_id("add-remote")
                            .ghost()
                            .xsmall()
                            .icon(Lucide::Globe)
                            .label("Add remote")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.prompt_add_remote(window, cx)
                            })),
                    )
                    .child(
                        Button::new("new-branch")
                            .test_id("new-branch")
                            .outline()
                            .xsmall()
                            .icon(Lucide::GitBranchPlus)
                            .label("New branch")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.prompt_new_branch(None, window, cx)
                            })),
                    ),
            )
            .child(list)
            .into_any_element()
    }
}
