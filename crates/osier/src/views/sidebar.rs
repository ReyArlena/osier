use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::component::sidebar::{
    Sidebar, SidebarFooter, SidebarGroup, SidebarHeader, SidebarMenu, SidebarMenuItem,
};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app::Osier;
use crate::format::tilde;

impl Osier {
    pub(crate) fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let weak = cx.entity().downgrade();
        let filter = self.sidebar_filter_text(cx);
        let project_count = self.projects.len();

        let mut open = SidebarMenu::new();
        let mut shown_open = 0;
        for (ix, project) in self.projects.iter().enumerate() {
            if !Self::matches_filter(&project.path, &filter) {
                continue;
            }
            shown_open += 1;
            let changes = project
                .snapshot
                .as_ref()
                .map(|s| s.status.entries.len())
                .unwrap_or(0);
            let loading = project.loading;
            let menu_weak = weak.clone();
            let path = project.path.clone();
            let item = SidebarMenuItem::new(project.name.clone())
                .icon(Lucide::FolderGit2)
                .active(self.active == Some(ix))
                .on_click(cx.listener(move |this, _, window, cx| this.activate(ix, window, cx)))
                .suffix(move |_, _| {
                    h_flex()
                        .gap_1()
                        .items_center()
                        .when(loading, |this| {
                            this.child(Tag::secondary().xsmall().child("…"))
                        })
                        .when(changes > 0 && !loading, |this| {
                            this.child(Tag::secondary().xsmall().child(changes.to_string()))
                        })
                        .into_any_element()
                })
                .context_menu(move |menu, _, _| {
                    let w = menu_weak.clone();
                    let p = path.clone();
                    let act = |f: fn(&mut Osier, usize, &mut Window, &mut Context<Osier>)| {
                        let w = w.clone();
                        move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                            let _ = w.update(cx, |this, cx| f(this, ix, window, cx));
                        }
                    };
                    menu.item(PopupMenuItem::new("Open in editor").on_click(act(
                        |this, ix, window, cx| {
                            this.activate(ix, window, cx);
                            this.open_in_editor(None, window, cx);
                        },
                    )))
                    .item(PopupMenuItem::new("Open in terminal").on_click(act(
                        |this, ix, window, cx| {
                            this.activate(ix, window, cx);
                            this.open_terminal(window, cx);
                        },
                    )))
                    .item(PopupMenuItem::new("Reveal in file manager").on_click(act(
                        |this, ix, window, cx| {
                            this.activate(ix, window, cx);
                            this.reveal_in_file_manager(None, window, cx);
                        },
                    )))
                    .separator()
                    .item(
                        PopupMenuItem::new("Move up")
                            .disabled(ix == 0)
                            .on_click(act(|this, ix, _, cx| this.move_project(ix, -1, cx))),
                    )
                    .item(
                        PopupMenuItem::new("Move down")
                            .disabled(ix + 1 >= project_count)
                            .on_click(act(|this, ix, _, cx| this.move_project(ix, 1, cx))),
                    )
                    .separator()
                    .item(PopupMenuItem::new("Copy path").on_click({
                        let w = w.clone();
                        move |_, window, cx| {
                            let p = p.to_string_lossy().into_owned();
                            let _ = w.update(cx, |this, cx| this.copy_to_clipboard(p, window, cx));
                        }
                    }))
                    .item(PopupMenuItem::new("Close project").on_click(act(
                        |this, ix, window, cx| this.close_project(ix, window, cx),
                    )))
                });
            open = open.child(item);
        }

        let recents = self.recent_entries(cx);
        let has_recent = !recents.is_empty();
        let mut recent = SidebarMenu::new();
        for (path, provider) in recents {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            let missing = !path.is_dir();
            let open_path = path.clone();
            let menu_weak = weak.clone();
            let menu_path = path.clone();
            let from_scan = provider.is_some();
            recent = recent.child(
                SidebarMenuItem::new(name)
                    .icon(if missing {
                        Lucide::FolderX
                    } else if from_scan {
                        Lucide::FolderSearch
                    } else {
                        Lucide::Folder
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_project_notify(&open_path, window, cx);
                    }))
                    .context_menu(move |menu, _, _| {
                        let (w1, w2) = (menu_weak.clone(), menu_weak.clone());
                        let (p1, p2) = (menu_path.clone(), menu_path.clone());
                        menu.item(
                            PopupMenuItem::new(if from_scan {
                                "Remove from recents (from a scan directory)"
                            } else {
                                "Remove from recents"
                            })
                            .on_click(move |_, window, cx| {
                                let _ =
                                    w1.update(cx, |this, cx| this.forget_recent(&p1, window, cx));
                            }),
                        )
                        .item(
                            PopupMenuItem::new("Copy path").on_click(move |_, window, cx| {
                                let p = p2.to_string_lossy().into_owned();
                                let _ =
                                    w2.update(cx, |this, cx| this.copy_to_clipboard(p, window, cx));
                            }),
                        )
                    }),
            );
        }

        Sidebar::new("projects")
            .w(px(240.))
            .header(
                SidebarHeader::new().child(
                    v_flex()
                        .w_full()
                        .gap_2()
                        .child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .size_8()
                                        .rounded(cx.theme().radius)
                                        .bg(cx.theme().sidebar_primary)
                                        .text_color(cx.theme().sidebar_primary_foreground)
                                        .child(Icon::new(Lucide::GitGraph)),
                                )
                                .child(
                                    v_flex()
                                        .child(div().font_medium().child("Open Projects"))
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(match project_count {
                                                    1 => "1 repository".to_string(),
                                                    n => format!("{n} repositories"),
                                                }),
                                        ),
                                ),
                        )
                        .child(
                            Input::new(&self.sidebar_filter)
                                .small()
                                .cleanable(true)
                                .prefix(Icon::new(Lucide::Search).xsmall()),
                        ),
                ),
            )
            .when(shown_open > 0, |this| this.child(SidebarGroup::new("Open").child(open)))
            .when(has_recent, |this| this.child(SidebarGroup::new("Recent").child(recent)))
            .footer(
                SidebarFooter::new().child(
                    v_flex()
                        .w_full()
                        .gap_2()
                        .when(project_count == 0 && !has_recent, |this| {
                            this.child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("No repositories yet. Open one below or run `osier <path>`."),
                            )
                        })
                        .when(!filter.is_empty() && shown_open == 0 && !has_recent, |this| {
                            this.child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Nothing matches the filter."),
                            )
                        })
                        .when_some(self.active_project(), |this, p| {
                            this.child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .truncate()
                                    .child(tilde(&p.path)),
                            )
                        })
                        .child(
                            Button::new("open-repo")
                                .outline()
                                .small()
                                .w_full()
                                .icon(Lucide::FolderPlus)
                                .label("Open repository")
                                .tooltip("Open a repository (ctrl-o)")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.prompt_open_repository(window, cx)
                                })),
                        ),
                ),
            )
    }
}
