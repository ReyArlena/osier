use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use osier_git::refs::Branch;

use crate::app::Osier;
use crate::format::relative_time;

impl Osier {
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
                list =
                    list.child(
                        h_flex()
                            .id((title, ix))
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
                                div()
                                    .text_xs()
                                    .text_color(muted)
                                    .flex_none()
                                    .w(px(78.))
                                    .text_right()
                                    .child(relative_time(b.committed_at)),
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
                                    .when(!is_remote, |this| {
                                        this.child(
                                            Button::new(SharedString::from(format!(
                                                "branch-menu-{title}-{ix}"
                                            )))
                                            .ghost()
                                            .xsmall()
                                            .icon(Lucide::Ellipsis)
                                            .dropdown_menu(move |menu, _, _| {
                                                let n1 = menu_name.clone();
                                                let n2 = menu_name.clone();
                                                let n3 = menu_name.clone();
                                                let n4 = menu_name.clone();
                                                let (w1, w2, w3, w4) = (
                                                    weak.clone(),
                                                    weak.clone(),
                                                    weak.clone(),
                                                    weak.clone(),
                                                );
                                                menu.item(
                                                    PopupMenuItem::new("New branch from here…")
                                                        .on_click(move |_, window, cx| {
                                                            let _ = w1.update(cx, |this, cx| {
                                                                this.prompt_new_branch(
                                                                    Some(n1.clone()),
                                                                    window,
                                                                    cx,
                                                                )
                                                            });
                                                        }),
                                                )
                                                .item(PopupMenuItem::new("Rename…").on_click(
                                                    move |_, window, cx| {
                                                        let _ = w2.update(cx, |this, cx| {
                                                            this.prompt_rename_branch(
                                                                n2.clone(),
                                                                window,
                                                                cx,
                                                            )
                                                        });
                                                    },
                                                ))
                                                .separator()
                                                .item(
                                                    PopupMenuItem::new("Delete")
                                                        .disabled(is_head)
                                                        .on_click(move |_, window, cx| {
                                                            let _ = w3.update(cx, |this, cx| {
                                                                this.confirm_delete_branch(
                                                                    n3.clone(),
                                                                    false,
                                                                    window,
                                                                    cx,
                                                                )
                                                            });
                                                        }),
                                                )
                                                .item(
                                                    PopupMenuItem::new("Force delete")
                                                        .disabled(is_head)
                                                        .on_click(move |_, window, cx| {
                                                            let _ = w4.update(cx, |this, cx| {
                                                                this.confirm_delete_branch(
                                                                    n4.clone(),
                                                                    true,
                                                                    window,
                                                                    cx,
                                                                )
                                                            });
                                                        }),
                                                )
                                            }),
                                        )
                                    }),
                            ),
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
                    .child(div().text_sm().font_medium().child("Branches"))
                    .child(div().text_xs().text_color(muted).child(format!(
                        "{} local, {} remote",
                        local.len(),
                        remote.len()
                    )))
                    .child(div().flex_1())
                    .child(
                        Button::new("new-branch")
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
