use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::a11y::{A11yName as _, A11yRow as _};
use crate::app::Osier;
use crate::app::row_actions;
use crate::test_ids::TestId as _;
use osier_git::ops::integrate::MergeMode;
use row_actions::RefRow;

/// One row of the Branches list: a section header, or an entry by its index in the
/// snapshot's list (branches also by their position in their section, for element ids).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum RefListRow {
    Header(Section),
    Branch {
        section: Section,
        pos: usize,
        at: usize,
    },
    Tag(usize),
    Worktree(usize),
    Submodule(usize),
    Remote(usize),
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Section {
    Local,
    RemoteBranches,
    Tags,
    Worktrees,
    Submodules,
    Remotes,
}

impl Osier {
    /// The Branches tab: branches, tags, worktrees, submodules and remotes in one list that
    /// builds only the rows on screen (`crate::lazy_list`).
    pub(crate) fn render_branches(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
        let mut rows = Vec::new();
        let (mut local, mut remote) = (0, 0);
        for (section, is_remote) in [(Section::Local, false), (Section::RemoteBranches, true)] {
            let entries: Vec<usize> = snapshot
                .branches
                .iter()
                .enumerate()
                .filter(|(_, b)| b.is_remote == is_remote)
                .map(|(at, _)| at)
                .collect();
            if is_remote {
                remote = entries.len();
            } else {
                local = entries.len();
            }
            if entries.is_empty() {
                continue;
            }
            rows.push(RefListRow::Header(section));
            rows.extend(
                entries
                    .into_iter()
                    .enumerate()
                    .map(|(pos, at)| RefListRow::Branch { section, pos, at }),
            );
        }
        // Tags, newest first.
        if !snapshot.tags.is_empty() {
            rows.push(RefListRow::Header(Section::Tags));
            rows.extend((0..snapshot.tags.len()).map(RefListRow::Tag));
        }
        // Worktrees: the main checkout plus linked ones.
        if snapshot.worktrees.len() > 1 {
            rows.push(RefListRow::Header(Section::Worktrees));
            rows.extend((0..snapshot.worktrees.len()).map(RefListRow::Worktree));
        }
        // Submodules: path, where it points, whether it is checked out.
        if !snapshot.submodules.is_empty() {
            rows.push(RefListRow::Header(Section::Submodules));
            rows.extend((0..snapshot.submodules.len()).map(RefListRow::Submodule));
        }
        // Remotes with their URLs and a menu.
        if !snapshot.remotes.is_empty() {
            rows.push(RefListRow::Header(Section::Remotes));
            rows.extend((0..snapshot.remotes.len()).map(RefListRow::Remote));
        }
        let selected = self.ref_selected.as_ref().and_then(|sel| {
            rows.iter().position(|row| match (row, sel) {
                (RefListRow::Branch { at, .. }, RefRow::Branch { name, remote }) => {
                    let b = &snapshot.branches[*at];
                    &b.name == name && b.is_remote == *remote
                }
                (RefListRow::Tag(ix), RefRow::Tag(name)) => &snapshot.tags[*ix].name == name,
                (RefListRow::Remote(ix), RefRow::Remote(name)) => {
                    &snapshot.remotes[*ix].name == name
                }
                _ => false,
            })
        });
        let state = self.branches_list.sync(&rows, selected);
        let muted = cx.theme().muted_foreground;
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
                            .child(format!("{local} local, {remote} remote")),
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
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(
                        crate::lazy_list::rows(
                            state.clone(),
                            cx.processor(move |this, ix: usize, _, cx| {
                                this.render_ref_row(rows[ix], cx)
                            }),
                        )
                        .size_full(),
                    )
                    .vertical_scrollbar(&state),
            )
            .into_any_element()
    }

    /// One row of the Branches list, built only while it is on screen.
    fn render_ref_row(&self, row: RefListRow, cx: &mut Context<Self>) -> AnyElement {
        let Some(snapshot) = self.active_project().and_then(|p| p.snapshot.as_ref()) else {
            return div().into_any_element();
        };
        let menu_buttons = self.settings.appearance.row_menu_buttons;
        let selected_ref = self.ref_selected.clone();
        let on_github = self.active_github_url().is_some();
        let muted = cx.theme().muted_foreground;
        let weak = cx.entity().downgrade();
        match row {
            RefListRow::Header(Section::Local) => h_flex()
                .px_3()
                .py_1()
                .gap_2()
                .text_xs()
                .font_medium()
                .text_color(muted)
                .child("Local")
                .child(
                    snapshot
                        .branches
                        .iter()
                        .filter(|b| !b.is_remote)
                        .count()
                        .to_string(),
                )
                .into_any_element(),
            RefListRow::Header(Section::RemoteBranches) => h_flex()
                .px_3()
                .py_1()
                .gap_2()
                .text_xs()
                .font_medium()
                .text_color(muted)
                .child("Remote")
                .child(
                    snapshot
                        .branches
                        .iter()
                        .filter(|b| b.is_remote)
                        .count()
                        .to_string(),
                )
                .into_any_element(),
            RefListRow::Header(Section::Tags) => h_flex()
                .px_3()
                .py_1()
                .gap_2()
                .text_xs()
                .font_medium()
                .text_color(muted)
                .child("Tags")
                .child(snapshot.tags.len().to_string())
                .into_any_element(),
            RefListRow::Header(Section::Worktrees) => h_flex()
                .px_3()
                .py_1()
                .gap_2()
                .text_xs()
                .font_medium()
                .text_color(muted)
                .child("Worktrees")
                .child(snapshot.worktrees.len().to_string())
                .into_any_element(),
            RefListRow::Header(Section::Submodules) => h_flex()
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
                )
                .into_any_element(),
            RefListRow::Header(Section::Remotes) => h_flex()
                .px_3()
                .py_1()
                .gap_2()
                .text_xs()
                .font_medium()
                .text_color(muted)
                .child("Remotes")
                .child(snapshot.remotes.len().to_string())
                .into_any_element(),
            RefListRow::Branch {
                section,
                pos: ix,
                at,
            } => {
                let b = &snapshot.branches[at];
                let (title, icon) = match section {
                    Section::RemoteBranches => ("Remote", Lucide::Cloud),
                    _ => ("Local", Lucide::GitBranch),
                };

                let name = b.name.clone();
                let is_remote = b.is_remote;
                let is_head = b.is_head;
                let checkout_name = name.clone();
                let menu_name = name.clone();
                let weak = weak.clone();
                let checkout = (!is_head).then(|| (weak.clone(), checkout_name.clone()));
                // Pull requests exist for remote branches and for locals with an upstream.
                let pulls = (on_github && (is_remote || b.upstream.is_some()))
                    .then(|| (weak.clone(), menu_name.clone()));
                let pulls_item = move |menu: gpui_kit::component::menu::PopupMenu| match &pulls {
                    Some((w, n)) => {
                        let (w, n) = (w.clone(), n.clone());
                        menu.item(
                            row_actions::menu_item(
                                "Pull requests on GitHub",
                                row_actions::RowKind::PullRequests,
                            )
                            .on_click(move |_, window, cx| {
                                let _ = w.update(cx, |this, cx| {
                                    this.open_branch_pulls_on_github(
                                        n.clone(),
                                        is_remote,
                                        window,
                                        cx,
                                    )
                                });
                            }),
                        )
                    }
                    None => menu,
                };
                let inner = if is_remote {
                    let (wm, wc) = (weak.clone(), weak.clone());
                    let (m1, c1) = (menu_name.clone(), menu_name.clone());
                    crate::row_menu::RowMenu::new(move |menu, _, _| {
                        let (wm, wc) = (wm.clone(), wc.clone());
                        let (m1, c1) = (m1.clone(), c1.clone());
                        let pulls_item = pulls_item.clone();
                        menu.item(
                            row_actions::menu_item(
                                "Merge into current branch",
                                row_actions::RowKind::MergeIntoCurrent,
                            )
                            .on_click(move |_, window, cx| {
                                let _ = wm.update(cx, |this, cx| {
                                    this.merge_branch(m1.clone(), MergeMode::Default, window, cx)
                                });
                            }),
                        )
                        .item(
                            row_actions::menu_item(
                                "Compare with current branch",
                                row_actions::RowKind::CompareWithCurrent,
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
                        .map(pulls_item)
                    })
                } else {
                    crate::row_menu::RowMenu::new(move |menu, _, _| {
                        let pulls_item = pulls_item.clone();
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
                                row_actions::menu_item(
                                    "Merge into current branch",
                                    row_actions::RowKind::MergeIntoCurrent,
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
                                row_actions::menu_item(
                                    "Rebase current branch onto this…",
                                    row_actions::RowKind::RebaseCurrentOnto,
                                )
                                .disabled(is_head)
                                .on_click(move |_, window, cx| {
                                    let _ = wr.update(cx, |this, cx| {
                                        this.confirm_rebase_onto(r1.clone(), window, cx)
                                    });
                                }),
                            )
                            .item(
                                row_actions::menu_item(
                                    "Compare with current branch",
                                    row_actions::RowKind::CompareWithCurrent,
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
                            .map(pulls_item)
                            .separator();
                        menu.item(
                            row_actions::menu_item(
                                "New branch from here…",
                                row_actions::RowKind::NewBranchHere,
                            )
                            .on_click(move |_, window, cx| {
                                let _ = w1.update(cx, |this, cx| {
                                    this.prompt_new_branch(Some(n1.clone()), window, cx)
                                });
                            }),
                        )
                        .item(
                            row_actions::menu_item("Rename…", row_actions::RowKind::Rename)
                                .on_click(move |_, window, cx| {
                                    let _ = w2.update(cx, |this, cx| {
                                        this.prompt_rename_branch(n2.clone(), window, cx)
                                    });
                                }),
                        )
                        .separator()
                        .item(
                            row_actions::menu_item("Delete", row_actions::RowKind::Delete)
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
                                row_actions::menu_item("Checkout", row_actions::RowKind::Checkout)
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
                let row_selected = selected_ref.as_ref()
                    == Some(&RefRow::Branch {
                        name: b.name.clone(),
                        remote: b.is_remote,
                    });

                h_flex()
                    .id((title, ix))
                    .cursor_pointer()
                    .when(row_selected, |this| this.bg(cx.theme().list_active))
                    .on_click(cx.listener({
                        let row = RefRow::Branch {
                            name: b.name.clone(),
                            remote: b.is_remote,
                        };
                        move |this, _, _, cx| this.select_ref(row.clone(), cx)
                    }))
                    .test_id(|| format!("branch-row:{}", b.name))
                    .a11y_row(
                        || {
                            let kind = if b.is_head {
                                ", current branch"
                            } else if b.is_remote {
                                ", remote branch"
                            } else {
                                ""
                            };
                            format!("{}{kind}", b.name)
                        },
                        row_selected,
                    )
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
                                    .a11y_name(|| format!("Checkout {}", b.name))
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
                                    .test_id(|| format!("branch-menu:{}", b.name))
                                    .when(!menu_buttons, |d| d.hidden())
                                    .child(
                                        Button::new(SharedString::from(format!(
                                            "branch-menu-{title}-{ix}"
                                        )))
                                        .a11y_name(|| format!("Actions for {}", b.name))
                                        .ghost()
                                        .xsmall()
                                        .icon(Lucide::Ellipsis)
                                        .dropdown_menu(row_menu.build()),
                                    ),
                            ),
                    )
                    .context_menu(row_menu.build())
                    .into_any_element()
            }
            RefListRow::Tag(ix) => {
                let t = &snapshot.tags[ix];
                let remotes: Vec<String> =
                    snapshot.remotes.iter().map(|r| r.name.clone()).collect();

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
                            row_actions::menu_item("Show commit", row_actions::RowKind::ShowCommit)
                                .on_click(move |_, _, cx| {
                                    let _ = w1.update(cx, |this, cx| {
                                        this.select_commit_hash(t1.clone(), cx)
                                    });
                                }),
                        )
                        .item(
                            row_actions::menu_item(
                                "Check out (detached)",
                                row_actions::RowKind::Checkout,
                            )
                            .on_click(move |_, window, cx| {
                                let _ = w2.update(cx, |this, cx| {
                                    this.checkout_commit(t2.clone(), window, cx)
                                });
                            }),
                        )
                        .item(
                            row_actions::menu_item("Copy name", row_actions::RowKind::CopyName)
                                .on_click(move |_, window, cx| {
                                    let _ = w3.update(cx, |this, cx| {
                                        this.copy_to_clipboard(n1.clone(), window, cx)
                                    });
                                }),
                        )
                        .separator()
                        .item(
                            row_actions::menu_item("Delete…", row_actions::RowKind::Delete)
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
                let row_selected = selected_ref.as_ref() == Some(&RefRow::Tag(t.name.clone()));

                h_flex()
                    .id(("tag", ix))
                    .cursor_pointer()
                    .when(row_selected, |this| this.bg(cx.theme().list_active))
                    .on_click(cx.listener({
                        let row = RefRow::Tag(t.name.clone());
                        move |this, _, _, cx| this.select_ref(row.clone(), cx)
                    }))
                    .test_id(|| format!("tag-row:{}", t.name))
                    .a11y_row(|| format!("Tag {}", t.name), row_selected)
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
                            .a11y_name(|| format!("Actions for tag {}", t.name))
                            .when(!menu_buttons, |b| b.hidden())
                            .ghost()
                            .xsmall()
                            .icon(Lucide::Ellipsis)
                            .dropdown_menu(row_menu.build()),
                    )
                    .context_menu(row_menu.build())
                    .into_any_element()
            }
            RefListRow::Worktree(ix) => {
                let w = &snapshot.worktrees[ix];

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

                h_flex()
                    .id(("worktree", ix))
                    .a11y_item(|| format!("Worktree {}", crate::format::tilde(&w.path)))
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
                            .a11y_name(|| {
                                format!("Actions for worktree {}", crate::format::tilde(&w.path))
                            })
                            .when(!menu_buttons, |b| b.hidden())
                            .ghost()
                            .xsmall()
                            .icon(Lucide::Ellipsis)
                            .dropdown_menu(row_menu.build()),
                    )
                    .context_menu(row_menu.build())
                    .into_any_element()
            }
            RefListRow::Submodule(ix) => {
                let s = &snapshot.submodules[ix];

                let path = s.path.clone();

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
                    )
                    .into_any_element()
            }
            RefListRow::Remote(ix) => {
                let r = &snapshot.remotes[ix];

                let name = r.name.clone();
                let url = r.fetch_url.clone();
                let weak = weak.clone();
                let row_menu = crate::row_menu::RowMenu::new(move |menu, _, _| {
                    let (n1, n2, n3, u1) = (name.clone(), name.clone(), name.clone(), url.clone());
                    let (w1, w2, w3) = (weak.clone(), weak.clone(), weak.clone());
                    menu.item(
                        row_actions::menu_item("Edit URL…", row_actions::RowKind::EditRemoteUrl)
                            .on_click(move |_, window, cx| {
                                let _ = w1.update(cx, |this, cx| {
                                    this.prompt_remote_url(n1.clone(), u1.clone(), window, cx)
                                });
                            }),
                    )
                    .item(
                        row_actions::menu_item("Rename…", row_actions::RowKind::Rename).on_click(
                            move |_, window, cx| {
                                let _ = w2.update(cx, |this, cx| {
                                    this.prompt_rename_remote(n2.clone(), window, cx)
                                });
                            },
                        ),
                    )
                    .separator()
                    .item(
                        row_actions::menu_item("Remove", row_actions::RowKind::Delete).on_click(
                            move |_, window, cx| {
                                let _ = w3.update(cx, |this, cx| {
                                    this.confirm_remove_remote(n3.clone(), window, cx)
                                });
                            },
                        ),
                    )
                });
                let row_selected = selected_ref.as_ref() == Some(&RefRow::Remote(r.name.clone()));

                h_flex()
                    .id(("remote", ix))
                    .a11y_row(|| format!("Remote {}", r.name), row_selected)
                    .cursor_pointer()
                    .when(row_selected, |this| this.bg(cx.theme().list_active))
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
                            .a11y_name(|| format!("Actions for remote {}", r.name))
                            .when(!menu_buttons, |b| b.hidden())
                            .ghost()
                            .xsmall()
                            .icon(Lucide::Ellipsis)
                            .dropdown_menu(row_menu.build()),
                    )
                    .context_menu(row_menu.build())
                    .into_any_element()
            }
        }
    }
}
