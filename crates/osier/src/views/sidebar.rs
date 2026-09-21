use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::Collapsible;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::component::sidebar::{
    Sidebar, SidebarFooter, SidebarGroup, SidebarHeader, SidebarItem, SidebarMenuItem,
};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app::Osier;
use crate::format::tilde;
use crate::test_ids::TestId as _;

type MoveHandler = std::rc::Rc<dyn Fn(usize, usize, &mut App)>;

/// What is being dragged in the Open list: the project's index and its name for the preview.
#[derive(Clone)]
struct ProjectDrag {
    ix: usize,
    name: SharedString,
}

struct DragPreview(SharedString);

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .text_color(cx.theme().popover_foreground)
            .text_sm()
            .child(self.0.clone())
    }
}

/// A sidebar menu whose rows can optionally be reordered by drag and drop. gpui-kit's
/// `SidebarMenuItem` has no drag hooks, so each row is wrapped in a drag source that is also a
/// drop target. `projects[ix]` maps display rows to project indices (the filter hides rows).
#[derive(Clone)]
pub(crate) struct ReorderMenu {
    items: Vec<SidebarMenuItem>,
    /// Test id per row (see `test_ids.rs`).
    ids: Vec<String>,
    drag: Option<(Vec<(usize, SharedString)>, MoveHandler)>,
    collapsed: bool,
}

impl ReorderMenu {
    fn new() -> Self {
        Self {
            items: Vec::new(),
            ids: Vec::new(),
            drag: None,
            collapsed: false,
        }
    }

    fn child(mut self, test_id: String, item: SidebarMenuItem) -> Self {
        self.items.push(item);
        self.ids.push(test_id);
        self
    }

    fn reorderable(mut self, projects: Vec<(usize, SharedString)>, on_move: MoveHandler) -> Self {
        self.drag = Some((projects, on_move));
        self
    }
}

impl Collapsible for ReorderMenu {
    fn collapsed(mut self, collapsed: bool) -> Self {
        self.collapsed = collapsed;
        self
    }

    fn is_collapsed(&self) -> bool {
        self.collapsed
    }
}

impl SidebarItem for ReorderMenu {
    fn render(
        self,
        id: impl Into<ElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        let id: SharedString = format!("{}", id.into()).into();
        let marker = cx.theme().primary;
        let collapsed = self.collapsed;
        let drag = self.drag;
        let ids = self.ids;
        v_flex()
            .gap_2()
            .children(self.items.into_iter().enumerate().map(|(row, item)| {
                let row_id = SharedString::from(format!("{id}-{row}"));
                let el = item
                    .collapsed(collapsed)
                    .render(row_id.clone(), window, cx)
                    .into_any_element();
                let test_id = ids.get(row).cloned().unwrap_or_default();
                match drag
                    .as_ref()
                    .and_then(|(map, h)| map.get(row).map(|e| (e.clone(), h.clone())))
                {
                    None => div().test_id(test_id).child(el).into_any_element(),
                    Some(((project_ix, name), on_move)) => div()
                        .id(SharedString::from(format!("{row_id}-drag")))
                        .test_id(test_id)
                        .on_drag(
                            ProjectDrag {
                                ix: project_ix,
                                name,
                            },
                            |d, _, _, cx| cx.new(|_| DragPreview(d.name.clone())),
                        )
                        .drag_over::<ProjectDrag>(move |style, _, _, _| {
                            style.border_t_2().border_color(marker)
                        })
                        .on_drop(move |d: &ProjectDrag, _, cx| on_move(d.ix, project_ix, cx))
                        .child(el)
                        .into_any_element(),
                }
            }))
    }
}

impl Osier {
    pub(crate) fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let weak = cx.entity().downgrade();
        let filter = self.sidebar_filter_text(cx);
        let project_count = self.projects.len();

        let mut open = ReorderMenu::new();
        let mut open_rows: Vec<(usize, SharedString)> = Vec::new();
        let mut shown_open = 0;
        for (ix, project) in self.projects.iter().enumerate() {
            if !Self::matches_filter(&project.path, &filter) {
                continue;
            }
            shown_open += 1;
            open_rows.push((ix, project.name.clone().into()));
            let pinned = self.state.is_pinned(&project.path);
            let (dirty, ahead, behind) = project
                .snapshot
                .as_ref()
                .map(|s| {
                    let b = &s.status.branch;
                    (!s.status.entries.is_empty(), b.ahead, b.behind)
                })
                .unwrap_or_default();
            let loading = project.loading;
            let menu_weak = weak.clone();
            let path = project.path.clone();
            let item = SidebarMenuItem::new(project.name.clone())
                .icon(Lucide::FolderGit2)
                .active(self.active == Some(ix))
                .on_click(cx.listener(move |this, _, window, cx| this.activate(ix, window, cx)))
                .suffix(move |_, cx| {
                    // Ahead/behind the upstream, like the status bar, only when non-zero; a dot
                    // marks uncommitted changes in the working tree.
                    let arrows = match (ahead, behind) {
                        (0, 0) => None,
                        (a, 0) => Some(format!("↑{a}")),
                        (0, b) => Some(format!("↓{b}")),
                        (a, b) => Some(format!("↑{a} ↓{b}")),
                    };
                    let warning = cx.theme().warning;
                    h_flex()
                        .gap_1()
                        .items_center()
                        .when(loading, |this| {
                            this.child(Tag::secondary().xsmall().child("…"))
                        })
                        .when_some(arrows.filter(|_| !loading), |this, a| {
                            this.child(Tag::secondary().xsmall().child(a))
                        })
                        .when(dirty && !loading, |this| {
                            this.child(div().size(px(6.)).rounded_full().bg(warning))
                        })
                        .when(pinned, |this| this.child(Icon::new(Lucide::Pin).xsmall()))
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
                    .item(
                        PopupMenuItem::new(if pinned { "Unpin" } else { "Pin" }).on_click({
                            let (w, p) = (w.clone(), p.clone());
                            move |_, _, cx| {
                                let _ = w.update(cx, |this, cx| this.toggle_pin(&p, cx));
                            }
                        }),
                    )
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
            open = open.child(format!("sidebar-open:{}", project.name), item);
        }

        // Drag to reorder; the menu's Move up/down stays for keyboard users.
        let move_weak = weak.clone();
        let open = open.reorderable(
            open_rows,
            std::rc::Rc::new(move |from, to, cx| {
                let _ = move_weak.update(cx, |this, cx| this.move_project_to(from, to, cx));
            }),
        );

        // Pinned repositories that are not open right now.
        let pinned = self.pinned_entries(cx);
        let has_pinned = !pinned.is_empty();
        let mut pinned_menu = ReorderMenu::new();
        for path in pinned {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            let missing = !path.is_dir();
            let (open_path, menu_path, menu_weak) = (path.clone(), path.clone(), weak.clone());
            pinned_menu = pinned_menu.child(
                format!("sidebar-pinned:{name}"),
                SidebarMenuItem::new(name.clone())
                    .icon(if missing {
                        Lucide::FolderX
                    } else {
                        Lucide::Pin
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_project_notify(&open_path, window, cx);
                    }))
                    .context_menu(move |menu, _, _| {
                        let (w1, w2) = (menu_weak.clone(), menu_weak.clone());
                        let (p1, p2) = (menu_path.clone(), menu_path.clone());
                        menu.item(PopupMenuItem::new("Unpin").on_click(move |_, _, cx| {
                            let _ = w1.update(cx, |this, cx| this.toggle_pin(&p1, cx));
                        }))
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

        let recents = self.recent_entries(cx);
        let has_recent = !recents.is_empty();
        let mut recent = ReorderMenu::new();
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
                format!("sidebar-recent:{name}"),
                SidebarMenuItem::new(name.clone())
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
                        let (w1, w2, w3) =
                            (menu_weak.clone(), menu_weak.clone(), menu_weak.clone());
                        let (p1, p2, p3) =
                            (menu_path.clone(), menu_path.clone(), menu_path.clone());
                        menu.item(PopupMenuItem::new("Pin").on_click(move |_, _, cx| {
                            let _ = w3.update(cx, |this, cx| this.toggle_pin(&p3, cx));
                        }))
                        .item(
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
                            div().test_id("sidebar-filter").child(
                            Input::new(&self.sidebar_filter)
                                .small()
                                .cleanable(true)
                                .prefix(Icon::new(Lucide::Search).xsmall())),
                        ),
                ),
            )
            .when(has_pinned, |this| this.child(SidebarGroup::new("Pinned").child(pinned_menu)))
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
                            Button::new("open-repo").test_id("open-repo")
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
