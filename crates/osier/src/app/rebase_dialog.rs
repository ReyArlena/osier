//! Interactive rebase: a dialog listing the commits above `base`, with per-row actions and
//! reordering. Runs `ops::rebase_interactive::run` with the resulting todo list.

use crate::app_dialog::AppDialogExt as _;
use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::dialog::DialogButtonProps;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::*;
use osier_git::ops::rebase_interactive::{self, TodoAction, TodoItem};

use crate::app::Osier;
use crate::notices::{Notice, WindowNoticeExt as _};

fn action_label(a: &TodoAction) -> &'static str {
    match a {
        TodoAction::Pick => "pick",
        TodoAction::Reword(_) => "reword",
        TodoAction::Edit => "edit",
        TodoAction::Squash => "squash",
        TodoAction::Fixup => "fixup",
        TodoAction::Drop => "drop",
    }
}

impl Osier {
    pub(crate) fn open_rebase_dialog(
        &mut self,
        base: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.active_project() else {
            return;
        };
        if self.in_progress().is_some() {
            window.push_notice(
                Notice::warning("Finish or abort the current operation first"),
                cx,
            );
            return;
        }
        // `git log` over the range can take a while on a big repository: in the background,
        // then the dialog opens with the list.
        let repo = project.repo.clone();
        let since = base.clone();
        self.in_background(
            crate::background::LISTING_COMMITS,
            move || rebase_interactive::commits_since(&repo, &since),
            move |this, items, window, cx| match items {
                Ok(items) => this.show_rebase_dialog(base, items, window, cx),
                Err(e) => window.push_notice(
                    Notice::error(e.to_string()).title("Cannot list commits"),
                    cx,
                ),
            },
            window,
            cx,
        );
    }

    /// The interactive rebase dialog for the commits above `base`.
    fn show_rebase_dialog(
        &mut self,
        base: String,
        items: Vec<TodoItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if items.is_empty() {
            window.push_notice("No commits above that point to rebase", cx);
            return;
        }
        let items: Rc<RefCell<Vec<TodoItem>>> = Rc::new(RefCell::new(items));
        let weak = cx.entity().downgrade();
        let short = base[..base.len().min(7)].to_string();
        let (flags, autostash) = self.autostash_flags();
        let flags = Rc::new(flags);
        window.open_app_dialog(cx, move |dialog, _, cx| {
            let autostash = autostash.clone();
            let items = items.clone();
            let weak = weak.clone();
            let base = base.clone();
            let muted = cx.theme().muted_foreground;
            let count = items.borrow().len();
            let mut rows = v_flex().gap_1().w_full();
            for ix in 0..count {
                let (subject, action, hash) = {
                    let b = items.borrow();
                    (b[ix].subject.clone(), b[ix].action.clone(), b[ix].hash.clone())
                };
                let set = {
                    let items = items.clone();
                    move |ix: usize, a: TodoAction, window: &mut Window| {
                        items.borrow_mut()[ix].action = a;
                        window.refresh();
                    }
                };
                let items_up = items.clone();
                let items_down = items.clone();
                let weak_reword = weak.clone();
                let items_reword = items.clone();
                let subject_for_reword = subject.clone();
                rows = rows.child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            h_flex()
                                .gap_0()
                                .child(
                                    Button::new(("up", ix))
                                        .ghost()
                                        .xsmall()
                                        .label("▲")
                                        .disabled(ix == 0)
                                        .on_click(move |_, window, _| {
                                            if ix > 0 {
                                                items_up.borrow_mut().swap(ix, ix - 1);
                                                window.refresh();
                                            }
                                        }),
                                )
                                .child(
                                    Button::new(("down", ix))
                                        .ghost()
                                        .xsmall()
                                        .label("▼")
                                        .disabled(ix + 1 >= count)
                                        .on_click(move |_, window, _| {
                                            if ix + 1 < count {
                                                items_down.borrow_mut().swap(ix, ix + 1);
                                                window.refresh();
                                            }
                                        }),
                                ),
                        )
                        .child(
                            Button::new(("action", ix))
                                .outline()
                                .xsmall()
                                .w(px(84.))
                                .label(action_label(&action))
                                .dropdown_menu(move |menu, _, _| {
                                    let s1 = set.clone();
                                    let s2 = set.clone();
                                    let s3 = set.clone();
                                    let s4 = set.clone();
                                    let s5 = set.clone();
                                    let weak_reword = weak_reword.clone();
                                    let items_reword = items_reword.clone();
                                    let subject = subject_for_reword.clone();
                                    menu.item(PopupMenuItem::new("pick: keep as is").on_click(move |_, w, _| s1(ix, TodoAction::Pick, w)))
                                        .item(PopupMenuItem::new("reword: change the message…").on_click(move |_, window, cx| {
                                            let items = items_reword.clone();
                                            let subject = subject.clone();
                                            let _ = weak_reword.update(cx, |this, cx| {
                                                this.prompt_text(
                                                    "New commit message",
                                                    "message",
                                                    subject.clone(),
                                                    "Set",
                                                    vec![],
                                                    move |_, text, window, _| {
                                                        let text = text.trim().to_string();
                                                        if !text.is_empty() {
                                                            items.borrow_mut()[ix].action = TodoAction::Reword(text);
                                                            window.refresh();
                                                        }
                                                    },
                                                    window,
                                                    cx,
                                                );
                                            });
                                        }))
                                        .item(PopupMenuItem::new("edit: stop here to amend").on_click(move |_, w, _| s2(ix, TodoAction::Edit, w)))
                                        .item(PopupMenuItem::new("squash: meld into previous, keep messages").disabled(ix == 0).on_click(move |_, w, _| s3(ix, TodoAction::Squash, w)))
                                        .item(PopupMenuItem::new("fixup: meld into previous, drop message").disabled(ix == 0).on_click(move |_, w, _| s4(ix, TodoAction::Fixup, w)))
                                        .separator()
                                        .item(PopupMenuItem::new("drop: remove the commit").on_click(move |_, w, _| s5(ix, TodoAction::Drop, w)))
                                }),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(muted)
                                .flex_none()
                                .child(hash[..7].to_string()),
                        )
                        .child(
                            div()
                                .text_sm()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(match &action {
                                    TodoAction::Reword(m) => m.lines().next().unwrap_or("").to_string(),
                                    _ => subject,
                                }),
                        ),
                );
            }
            dialog
                .title(format!("Interactive rebase onto {short}"))
                .w(px(760.))
                .child(
                    v_flex()
                        .gap_3()
                        .pb_1()
                        .child(
                            div()
                                .text_sm()
                                .text_color(muted)
                                .child("Oldest first, applied top to bottom. Reorder with the arrows, pick an action per commit. Conflicts stop the rebase so you can resolve and continue."),
                        )
                        .child(rows)
                        .children(flags.iter().enumerate().map(|(ix, f)| f.checkbox(ix, cx))),
                )
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Rebase")
                        .ok_variant(ButtonVariant::Primary)
                        .show_cancel(true),
                )
.footer(crate::dialog_buttons::DialogButtons::new("Rebase").variant(ButtonVariant::Primary))
                .on_ok(move |_, window, cx| {
                    let list = items.borrow().clone();
                    let base = base.clone();
                    let autostash = autostash.get();
                    let _ = weak.update(cx, |this, cx| {
                        let n = list.len();
                        this.run_op(
                            format!("Interactive rebase of {n} commits"),
                            move |repo| rebase_interactive::run(repo, &base, &list, autostash),
                            window,
                            cx,
                        );
                    });
                    true
                })
        });
    }
}
