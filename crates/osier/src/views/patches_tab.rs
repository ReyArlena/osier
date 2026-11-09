use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, StyledExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app::Osier;
use crate::app::row_actions;
use crate::test_ids::TestId as _;

impl Osier {
    pub(crate) fn render_patches(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let menu_buttons = self.settings.appearance.row_menu_buttons;
        let muted = cx.theme().muted_foreground;
        let has_project = self.active_project().is_some();
        let can_shelve = self.all_changes_source().is_some();

        let header = h_flex()
            .px_3()
            .py_2()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(div().text_sm().font_medium().child("Patches"))
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(self.patches.len().to_string()),
            )
            .child(div().flex_1())
            .child(
                Button::new("shelve-all")
                    .test_id("shelve-all")
                    .outline()
                    .xsmall()
                    .icon(Lucide::Package)
                    .label("Shelve changes")
                    .disabled(!can_shelve)
                    .tooltip("Save all working tree changes as a patch and discard them")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.shelve_prompt(Vec::new(), window, cx)
                    })),
            )
            .child(
                Button::new("apply-clipboard")
                    .test_id("apply-clipboard")
                    .ghost()
                    .xsmall()
                    .icon(Lucide::ClipboardPaste)
                    .disabled(!has_project)
                    .tooltip("Apply patch from clipboard")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.apply_patch_from_clipboard(window, cx)
                    })),
            )
            .child(
                Button::new("apply-file")
                    .test_id("apply-file")
                    .ghost()
                    .xsmall()
                    .icon(Lucide::FileInput)
                    .disabled(!has_project)
                    .tooltip("Apply patch from file…")
                    .on_click(
                        cx.listener(|this, _, window, cx| this.apply_patch_from_file(window, cx)),
                    ),
            )
            .child(
                Button::new("open-patches-dir")
                    .test_id("open-patches-dir")
                    .ghost()
                    .xsmall()
                    .icon(Lucide::FolderOpen)
                    .tooltip("Open the patches folder")
                    .on_click(cx.listener(|this, _, window, cx| this.open_patches_dir(window, cx))),
            );

        if self.patches.is_empty() {
            return v_flex()
                .size_full()
                .child(header)
                .child(self.empty_state(
                    "No patches",
                    "Shelve changes, or save a file or commit as a patch from its menu. Patches are kept across projects.",
                    cx,
                ))
                .into_any_element();
        }
        let mut list = v_flex()
            .flex_1()
            .min_h_0()
            .id("patch-list")
            .overflow_y_scrollbar();
        let w = cx.weak_entity();
        for (ix, p) in self.patches.iter().enumerate() {
            let w = w.clone();
            let id = p.id.clone();
            let name = p.meta.name.clone();
            let selected = self.patch_selected.as_deref() == Some(p.id.as_str());
            let mut detail = Vec::new();
            if let Some(repo) = &p.meta.source_repo
                && let Some(n) = repo.file_name()
            {
                detail.push(n.to_string_lossy().into_owned());
            }
            let n = p.meta.files.len();
            if n > 0 {
                detail.push(format!("{n} {}", if n == 1 { "file" } else { "files" }));
            }
            let detail = detail.join(" · ");
            let (i1, i2, i3, i4, i5, i6, i7) = (
                id.clone(),
                id.clone(),
                id.clone(),
                id.clone(),
                id.clone(),
                id.clone(),
                id.clone(),
            );
            let (n1, n2) = (name.clone(), name.clone());
            let (wa, ia, iu) = (w.clone(), id.clone(), id.clone());
            let inner = crate::row_menu::RowMenu::new(move |menu, _, _| {
                let (i4, i5, i6, i7) = (i4.clone(), i5.clone(), i6.clone(), i7.clone());
                let (n1, n2) = (n1.clone(), n2.clone());
                let (o1, o2, o3, o4) = (w.clone(), w.clone(), w.clone(), w.clone());
                menu.item(
                    row_actions::menu_item("Copy patch text", row_actions::RowKind::CopyAsPatch)
                        .icon(Lucide::ClipboardCopy)
                        .on_click(move |_, window, cx| {
                            let _ =
                                o1.update(cx, |this, cx| this.copy_stored(i4.clone(), window, cx));
                        }),
                )
                .item(
                    row_actions::menu_item("Export to file…", row_actions::RowKind::ExportPatch)
                        .icon(Lucide::FileOutput)
                        .on_click(move |_, window, cx| {
                            let _ = o2
                                .update(cx, |this, cx| this.export_stored(i5.clone(), window, cx));
                        }),
                )
                .item(
                    row_actions::menu_item("Rename…", row_actions::RowKind::Rename)
                        .icon(Lucide::Pencil)
                        .on_click(move |_, window, cx| {
                            let _ = o3.update(cx, |this, cx| {
                                this.rename_stored_prompt(i6.clone(), n1.clone(), window, cx)
                            });
                        }),
                )
                .separator()
                .item(
                    row_actions::menu_item("Delete…", row_actions::RowKind::Delete)
                        .icon(Lucide::Trash)
                        .on_click(move |_, window, cx| {
                            let _ = o4.update(cx, |this, cx| {
                                this.confirm_delete_stored(i7.clone(), n2.clone(), window, cx)
                            });
                        }),
                )
            });
            // Right-click also offers the row's hover actions.
            let row_menu = crate::row_menu::RowMenu::new(move |menu, window, cx| {
                let menu = if has_project {
                    let (w1, w2, i1, i2) = (wa.clone(), wa.clone(), ia.clone(), iu.clone());
                    menu.item(
                        row_actions::menu_item("Apply", row_actions::RowKind::Apply).on_click(
                            move |_, window, cx| {
                                let _ = w1.update(cx, |this, cx| {
                                    this.apply_stored(i1.clone(), false, window, cx)
                                });
                            },
                        ),
                    )
                    .item(
                        row_actions::menu_item("Unshelve", row_actions::RowKind::Unshelve)
                            .on_click(move |_, window, cx| {
                                let _ = w2.update(cx, |this, cx| {
                                    this.apply_stored(i2.clone(), true, window, cx)
                                });
                            }),
                    )
                    .separator()
                } else {
                    menu
                };
                (inner.build())(menu, window, cx)
            });
            list = list.child(
                h_flex()
                    .id(("patch", ix))
                    .test_id(|| format!("patch-row:{}", p.id))
                    .group("row")
                    .px_3()
                    .py_1p5()
                    .gap_2()
                    .items_center()
                    .cursor_pointer()
                    .when(selected, |this| this.bg(cx.theme().list_active))
                    .hover(|this| this.bg(cx.theme().list_hover))
                    .when(has_project, |this| {
                        this.on_click(
                            cx.listener(move |this, _, _, cx| this.select_patch(i1.clone(), cx)),
                        )
                    })
                    .child(Icon::new(Lucide::Package).xsmall())
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .child(div().text_sm().truncate().child(name.clone()))
                            .when(!detail.is_empty(), |this| {
                                this.child(
                                    div().text_xs().text_color(muted).truncate().child(detail),
                                )
                            }),
                    )
                    .child(
                        super::relative_time_label(("patch-time", ix), p.meta.created as i64, cx)
                            .text_xs()
                            .text_color(muted)
                            .flex_none(),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .flex_none()
                            .invisible()
                            .group_hover("row", |this| this.visible())
                            .child(
                                Button::new(("patch-apply", ix))
                                    .ghost()
                                    .xsmall()
                                    .label("Apply")
                                    .disabled(!has_project)
                                    .tooltip("Apply to the working tree and keep the patch")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.apply_stored(i2.clone(), false, window, cx)
                                    })),
                            )
                            .child(
                                Button::new(("patch-unshelve", ix))
                                    .ghost()
                                    .xsmall()
                                    .label("Unshelve")
                                    .disabled(!has_project)
                                    .tooltip("Apply and delete the patch")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.apply_stored(i3.clone(), true, window, cx)
                                    })),
                            )
                            .child(
                                Button::new(("patch-more", ix))
                                    .when(!menu_buttons, |b| b.hidden())
                                    .ghost()
                                    .xsmall()
                                    .icon(Lucide::Ellipsis)
                                    .dropdown_menu(row_menu.build()),
                            ),
                    )
                    .context_menu(row_menu.build()),
            );
        }
        v_flex()
            .size_full()
            .child(header)
            .child(list)
            .into_any_element()
    }
}
