use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::Collapsible;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::component::resizable::{HandleEdge, resize_handle};
use gpui_kit::component::sidebar::{
    Sidebar, SidebarFooter, SidebarGroup, SidebarHeader, SidebarItem, SidebarMenuItem,
};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _, StyledExt as _, ThemeStyled as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app::Osier;
use crate::app::panes::ResizeSidebar;
use crate::format::{ahead_behind as arrows_text, tilde};
use crate::test_ids::TestId as _;

type MoveHandler = std::rc::Rc<dyn Fn(usize, usize, &mut App)>;

// Geometry of gpui-kit's sidebar row: 8px padding, 16px icon, 8px gap, 28px tall.
const DOT_SIZE: f32 = 6.0;
const DOT_GAP: f32 = 14.0;
const DOT_LEFT: f32 = 8.0 + 16.0 + 8.0 + 2.0;
const DOT_TOP: f32 = (28.0 - DOT_SIZE) / 2.0;

/// Label style for every row: one line, so the fade below lines up with it; with room for
/// the changes dot when `dot`.
fn label_style(dot: bool) -> StyleRefinement {
    // The negative right margin lets the name use gpui-kit's 8px gap before the suffix slot;
    // the fade below ends exactly at the slot, so nothing shows through the gap.
    let style = StyleRefinement::default()
        .whitespace_nowrap()
        .mr(px(-SUFFIX_GAP));
    if dot { style.pl(px(DOT_GAP)) } else { style }
}

/// gpui-kit's gap between a row's name and its suffix slot.
const SUFFIX_GAP: f32 = 8.0;
/// Sidebar rows are short on space, so their fade is tighter than the log's.
const ROW_FADE_WIDTH: f32 = 14.0;

/// Colours behind a sidebar row, for the name fade (see `fade.rs`).
#[derive(Clone, Copy)]
struct RowFade {
    base: Hsla,
    hover: Hsla,
    active: Hsla,
}

impl RowFade {
    fn new(cx: &App) -> Self {
        let base = cx.theme().sidebar;
        Self {
            base,
            hover: base.blend(cx.theme().sidebar_accent.opacity(0.8)),
            active: base.blend(cx.theme().sidebar_accent),
        }
    }
}

/// The suffix of every row: a fade over the end of the name, then the badges. The name runs
/// up to the suffix's left edge (see `label_style`), and the fade is pinned there, reaching
/// back over the last letters. The suffix keeps a fixed height so the fade covers the line
/// even with no badges.
fn row_suffix(fade: RowFade, active: bool) -> Div {
    let (color, hover) = if active {
        (fade.active, fade.active)
    } else {
        (fade.base, fade.hover)
    };
    h_flex().relative().h_5().gap_1().items_center().child(
        div()
            .absolute()
            .top_0()
            .bottom_0()
            .left(px(-ROW_FADE_WIDTH))
            .w(px(ROW_FADE_WIDTH))
            .bg(crate::fade::fade_fill(color))
            .group_hover("sidebar-row", move |s| s.bg(crate::fade::fade_fill(hover))),
    )
}

impl ReorderMenu {
    /// Add a row, with a dot in front of its label when `dot` is set.
    fn row(self, test_id: String, item: SidebarMenuItem, dot: Option<Hsla>) -> Self {
        match dot {
            Some(color) => self.child_with_dot(test_id, item, color),
            None => self.child(test_id, item),
        }
    }
}

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
    /// Rows that get a changes dot before their name, and its colour.
    dots: Vec<Option<Hsla>>,
    drag: Option<(Vec<(usize, SharedString)>, MoveHandler)>,
    collapsed: bool,
}

impl ReorderMenu {
    fn new() -> Self {
        Self {
            items: Vec::new(),
            ids: Vec::new(),
            dots: Vec::new(),
            drag: None,
            collapsed: false,
        }
    }

    fn child(mut self, test_id: String, item: SidebarMenuItem) -> Self {
        self.items.push(item);
        self.ids.push(test_id);
        self.dots.push(None);
        self
    }

    /// A row with a dot drawn in front of its label.
    fn child_with_dot(mut self, test_id: String, item: SidebarMenuItem, dot: Hsla) -> Self {
        self.items.push(item);
        self.ids.push(test_id);
        self.dots.push(Some(dot));
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
        let dots = self.dots;
        v_flex()
            .gap_2()
            .children(self.items.into_iter().enumerate().map(|(row, item)| {
                let row_id = SharedString::from(format!("{id}-{row}"));
                let el = item
                    .collapsed(collapsed)
                    .render(row_id.clone(), window, cx)
                    .into_any_element();
                let test_id = ids.get(row).cloned().unwrap_or_default();
                // gpui-kit rows have no slot before the label. The label gets DOT_GAP of left
                // padding (`dot_label_style`) and the dot is drawn into that space.
                let el = match dots.get(row).copied().flatten() {
                    Some(color) if !collapsed => div()
                        .relative()
                        .child(el)
                        .child(
                            div()
                                .absolute()
                                .left(px(DOT_LEFT))
                                .top(px(DOT_TOP))
                                .size(px(DOT_SIZE))
                                .rounded_full()
                                .bg(color),
                        )
                        .into_any_element(),
                    _ => el,
                };
                match drag
                    .as_ref()
                    .and_then(|(map, h)| map.get(row).map(|e| (e.clone(), h.clone())))
                {
                    None => div()
                        .group("sidebar-row")
                        .test_id(test_id)
                        .child(el)
                        .into_any_element(),
                    Some(((project_ix, name), on_move)) => div()
                        .id(SharedString::from(format!("{row_id}-drag")))
                        .test_id(test_id)
                        .group("sidebar-row")
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
    /// A row for a project that is not open, with its last known status: ↑/↓ suffix and the
    /// changes marker (icon colour or a dot, per `appearance.changes_indicator`).
    fn status_item(
        &self,
        path: &std::path::Path,
        name: &str,
        icon: Lucide,
        cx: &App,
    ) -> (SidebarMenuItem, Option<Hsla>) {
        let badge = self.recent_badge(path);
        let color = cx.theme().warning;
        let dirty =
            self.recent_status.shows_changes() && badge.and_then(|b| b.dirty).unwrap_or(false);
        let by_dot = dirty
            && self.settings.appearance.changes_indicator
                == osier_config::settings::ChangesIndicator::Dot;
        let arrows = badge.and_then(|b| arrows_text(b.ahead, b.behind));
        let fade = RowFade::new(cx);
        let item = SidebarMenuItem::new(name.to_string())
            .icon(Icon::new(icon).when(dirty && !by_dot, |i| i.text_color(color)))
            .label_style(label_style(by_dot))
            .suffix(move |_, _| {
                row_suffix(fade, false).when_some(arrows.clone(), |this, a| {
                    this.child(Tag::secondary().xsmall().child(a))
                })
            });
        (item, by_dot.then_some(color))
    }

    /// The `is:` suggestion dropdown under the focused filter box, painted above the sidebar.
    fn render_filter_suggestions(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        use osier_config::settings::RecentStatusMode as M;
        if !self
            .sidebar_filter
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
        {
            return None;
        }
        let text = self.sidebar_filter.read(cx).value().to_string();
        let items = crate::sidebar_query::suggestions(&text);
        if items.is_empty() {
            return None;
        }
        let bounds = self.sidebar_filter_bounds.get();
        let muted = cx.theme().muted_foreground;
        let note = match self.settings.projects.recent_status {
            M::Never => Some(
                "Dynamic searches only apply to open projects. Update Projects > Show status \
                 in settings to cover recent projects.",
            ),
            M::Watch => Some(
                "is:changed only applies to open projects while Projects > Show status \
                 watches ahead/behind only.",
            ),
            M::Cached | M::Check => None,
        };
        // Same surface and item shape as gpui-kit's popup menus, at the filter box's width.
        let mut list = v_flex().gap_y_0p5();
        for (term, desc) in items {
            list = list.child(
                v_flex()
                    .id(SharedString::from(format!("filter-suggestion:{term}")))
                    .test_id(|| format!("filter-suggestion:{term}"))
                    .px_2()
                    .py_1()
                    .rounded(cx.theme().radius)
                    .cursor_pointer()
                    .hover(|this| this.bg(cx.theme().accent))
                    // Mouse down, so the pick lands before the input loses focus.
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.pick_filter_suggestion(term, window, cx);
                        }),
                    )
                    .child(
                        div()
                            .font_family(cx.theme().mono_font_family.clone())
                            .text_sm()
                            .child(term),
                    )
                    .child(div().text_xs().text_color(muted).truncate().child(desc)),
            );
        }
        let panel = v_flex()
            .occlude()
            .test_id("filter-suggestions")
            .w(bounds.size.width)
            .p_1()
            .popover_style(cx)
            .child(list)
            .when_some(note, |this, note| {
                this.child(
                    v_flex()
                        .mt_1()
                        .px_2()
                        .pt_1p5()
                        .pb_1()
                        .gap_1()
                        .border_t_1()
                        .border_color(cx.theme().border)
                        .text_xs()
                        .text_color(muted)
                        .whitespace_normal()
                        .child(note)
                        .child(
                            div()
                                .id("filter-open-settings")
                                .test_id("filter-open-settings")
                                .text_color(cx.theme().link)
                                .cursor_pointer()
                                .hover(|this| this.underline())
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.open_settings_at(
                                            crate::settings_panel::SettingsTarget::RECENT_STATUS,
                                            window,
                                            cx,
                                        );
                                    }),
                                )
                                .child("Open settings"),
                        ),
                )
            });
        Some(
            deferred(
                anchored()
                    .position(point(bounds.left(), bounds.bottom() + px(4.)))
                    .snap_to_window_with_margin(px(8.))
                    .child(panel),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }

    /// The project sidebar's content (a pane, `crate::pane`); [`Self::render_sidebar_frame`]
    /// gives it its width and draggable edge.
    pub(crate) fn render_sidebar(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let weak = cx.entity().downgrade();
        let filter = self.sidebar_filter_text(cx);
        let query = self.sidebar_query(cx);
        let project_count = self.projects.len();
        let name_hints = self.sidebar_name_hints();

        let mut open = ReorderMenu::new();
        let mut open_rows: Vec<(usize, SharedString)> = Vec::new();
        let mut shown_open = 0;
        for (ix, project) in self.projects.iter().enumerate() {
            if !query.matches(&project.path, Self::open_row_status(project)) {
                continue;
            }
            shown_open += 1;
            let label = crate::format::name_with_hint(&project.name, name_hints.get(&project.path));
            open_rows.push((ix, label.clone().into()));
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
            let changes_color = cx.theme().warning;
            let by_dot = dirty
                && self.settings.appearance.changes_indicator
                    == osier_config::settings::ChangesIndicator::Dot;
            let by_icon = dirty && !by_dot;
            let fade = RowFade::new(cx);
            let is_active = self.active == Some(ix);
            let on_github = project.snapshot.as_ref().is_some_and(|s| {
                crate::app::github::repo_url(&s.remotes, s.status.branch.upstream.as_deref())
                    .is_some()
            });
            let item = SidebarMenuItem::new(label)
                .icon(
                    Icon::new(Lucide::FolderGit2)
                        .when(by_icon, |icon| icon.text_color(changes_color)),
                )
                .label_style(label_style(by_dot))
                .active(is_active)
                .on_click(cx.listener(move |this, _, window, cx| this.activate(ix, window, cx)))
                .suffix(move |_, _| {
                    // Ahead/behind the upstream, like the status bar, only when non-zero.
                    let arrows = arrows_text(ahead, behind);
                    row_suffix(fade, is_active)
                        .when(loading, |this| {
                            this.child(Tag::secondary().xsmall().child("…"))
                        })
                        .when_some(arrows.filter(|_| !loading), |this, a| {
                            this.child(Tag::secondary().xsmall().child(a))
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
                    .item(PopupMenuItem::new("Open in file manager").on_click(act(
                        |this, ix, window, cx| {
                            this.activate(ix, window, cx);
                            this.reveal_in_file_manager(None, window, cx);
                        },
                    )))
                    .when(on_github, |menu| {
                        let (w, p) = (w.clone(), p.clone());
                        menu.item(PopupMenuItem::new("Open on GitHub").on_click(
                            move |_, window, cx| {
                                let _ = w.update(cx, |this, cx| {
                                    this.open_project_on_github(&p, window, cx)
                                });
                            },
                        ))
                    })
                    .item(PopupMenuItem::new("Git config").on_click({
                        let (w, p) = (w.clone(), p.clone());
                        move |_, window, cx| {
                            let _ = w.update(cx, |this, cx| {
                                this.open_repo_config(p.clone(), window, cx)
                            });
                        }
                    }))
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
            let test_id = format!("sidebar-open:{}", project.name);
            open = if by_dot {
                open.child_with_dot(test_id, item, changes_color)
            } else {
                open.child(test_id, item)
            };
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
            let missing = self.project_missing(&path);
            let (open_path, menu_path, menu_weak) = (path.clone(), path.clone(), weak.clone());
            let (item, dot) = self.status_item(
                &path,
                &crate::format::name_with_hint(&name, name_hints.get(&path)),
                if missing {
                    Lucide::FolderX
                } else {
                    Lucide::Pin
                },
                cx,
            );
            pinned_menu = pinned_menu.row(
                format!("sidebar-pinned:{name}"),
                item.on_click(cx.listener(move |this, _, window, cx| {
                    this.open_project_notify(&open_path, window, cx);
                }))
                .context_menu(move |menu, _, _| {
                    let (w1, w2) = (menu_weak.clone(), menu_weak.clone());
                    let (p1, p2) = (menu_path.clone(), menu_path.clone());
                    let (w3, p3) = (menu_weak.clone(), menu_path.clone());
                    let (w4, p4) = (menu_weak.clone(), menu_path.clone());
                    menu.item(PopupMenuItem::new("Unpin").on_click(move |_, _, cx| {
                        let _ = w1.update(cx, |this, cx| this.toggle_pin(&p1, cx));
                    }))
                    .item(
                        PopupMenuItem::new("Open on GitHub").on_click(move |_, window, cx| {
                            let _ = w3.update(cx, |this, cx| {
                                this.open_project_on_github(&p3, window, cx)
                            });
                        }),
                    )
                    .item(
                        PopupMenuItem::new("Git config").on_click(move |_, window, cx| {
                            let _ = w4.update(cx, |this, cx| {
                                this.open_repo_config(p4.clone(), window, cx)
                            });
                        }),
                    )
                    .item(
                        PopupMenuItem::new("Copy path").on_click(move |_, window, cx| {
                            let p = p2.to_string_lossy().into_owned();
                            let _ = w2.update(cx, |this, cx| this.copy_to_clipboard(p, window, cx));
                        }),
                    )
                }),
                dot,
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
            let missing = self.project_missing(&path);
            let open_path = path.clone();
            let menu_weak = weak.clone();
            let menu_path = path.clone();
            let from_scan = provider.is_some();
            let icon = if missing {
                Lucide::FolderX
            } else if from_scan {
                Lucide::FolderSearch
            } else {
                Lucide::Folder
            };
            let label = crate::format::name_with_hint(&name, name_hints.get(&path));
            let (item, dot) = self.status_item(&path, &label, icon, cx);
            recent = recent.row(
                format!("sidebar-recent:{name}"),
                item.on_click(cx.listener(move |this, _, window, cx| {
                    this.open_project_notify(&open_path, window, cx);
                }))
                .context_menu(move |menu, _, _| {
                    let (w1, w2, w3) = (menu_weak.clone(), menu_weak.clone(), menu_weak.clone());
                    let (p1, p2, p3) = (menu_path.clone(), menu_path.clone(), menu_path.clone());
                    let (w4, p4) = (menu_weak.clone(), menu_path.clone());
                    let (w5, p5) = (menu_weak.clone(), menu_path.clone());
                    menu.item(PopupMenuItem::new("Pin").on_click(move |_, _, cx| {
                        let _ = w3.update(cx, |this, cx| this.toggle_pin(&p3, cx));
                    }))
                    .item(
                        PopupMenuItem::new("Open on GitHub").on_click(move |_, window, cx| {
                            let _ = w4.update(cx, |this, cx| {
                                this.open_project_on_github(&p4, window, cx)
                            });
                        }),
                    )
                    .when(!missing, |menu| {
                        menu.item(PopupMenuItem::new("Git config").on_click(
                            move |_, window, cx| {
                                let _ = w5.update(cx, |this, cx| {
                                    this.open_repo_config(p5.clone(), window, cx)
                                });
                            },
                        ))
                    })
                    .item(PopupMenuItem::new("Remove from recents").on_click(
                        move |_, window, cx| {
                            let _ = w1.update(cx, |this, cx| this.forget_recent(&p1, window, cx));
                        },
                    ))
                    .item(
                        PopupMenuItem::new("Copy path").on_click(move |_, window, cx| {
                            let p = p2.to_string_lossy().into_owned();
                            let _ = w2.update(cx, |this, cx| this.copy_to_clipboard(p, window, cx));
                        }),
                    )
                }),
                dot,
            );
        }

        // Not collapsible: Osier hides the sidebar as a whole, and a collapsible one animates
        // every change of width, which trails behind the pointer while its edge is dragged.
        let sidebar = Sidebar::new("projects")
            .collapsible(false)
            .w(px(self.sidebar_width(window)))
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
                            div()
                                .test_id("sidebar-filter")
                                .relative()
                                .on_action(cx.listener(
                                    |this, _: &gpui_kit::component::input::Escape, window, cx| {
                                        let input = this.sidebar_filter.clone();
                                        this.escape_filter(input, window, cx)
                                    },
                                ))
                                .child(
                            Input::new(&self.sidebar_filter)
                                .small()
                                .cleanable(true)
                                .prefix(Icon::new(Lucide::Search).xsmall()))
                                .child({
                                    let cell = self.sidebar_filter_bounds.clone();
                                    canvas(move |b, _, _| cell.set(b), |_, _, _, _| {})
                                        .absolute()
                                        .top_0()
                                        .left_0()
                                        .size_full()
                                })
                                .when_some(self.render_filter_suggestions(window, cx), |this, d| {
                                    this.child(d)
                                }),
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
                                .tooltip(match crate::keymap::hint("open_repository", cx) {
                                    Some(k) => format!("Open a repository ({k})"),
                                    None => "Open a repository".to_string(),
                                })
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.prompt_open_repository(window, cx)
                                })),
                        ),
                ),
            );
        sidebar.into_any_element()
    }

    /// The sidebar pane with a draggable right edge (shell). The drag is followed here rather
    /// than on the handle, since the pointer leaves the handle as soon as it moves.
    pub(crate) fn render_sidebar_frame(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .test_id("sidebar-pane")
            .relative()
            .flex()
            .flex_none()
            .h_full()
            .w(px(self.sidebar_width(window)))
            .child(crate::pane::cached(&self.panes.sidebar, window))
            .child(
                resize_handle("sidebar-edge", Axis::Horizontal)
                    .inside(HandleEdge::Trailing)
                    .on_drag(ResizeSidebar, |drag, _, _, cx| {
                        cx.stop_propagation();
                        cx.new(|_| drag.clone())
                    }),
            )
            .on_drag_move(
                cx.listener(|this, e: &DragMoveEvent<ResizeSidebar>, window, cx| {
                    this.resize_sidebar(e.event.position.x - e.bounds.left(), window, cx)
                }),
            )
    }
}
