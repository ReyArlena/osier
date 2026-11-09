//! Squash in the Log: pick commits (shift-click a range, ctrl-click single ones), right-click
//! one of them, "Squash selected commits…". The checks live in `osier_git::ops::squash`; this file
//! plans off the UI thread, asks for the message in a dialog that says what will happen, and
//! selects the new commit afterwards.

use std::sync::{Arc, Mutex};

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::ButtonVariant;
use gpui_kit::component::dialog::DialogButtonProps;
use gpui_kit::component::input::{Textarea, TextareaState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::*;
use osier_git::ops::squash::{self, SquashPlan};

use crate::app::Osier;
use crate::project::Selection;
use crate::test_ids::TestId as _;

impl Osier {
    /// Plan a squash of the shift-click range and, if git agrees it is safe, open the dialog.
    pub(crate) fn confirm_squash(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.active_project() else {
            return;
        };
        let picked = project.picked.clone();
        if picked.len() < 2 {
            window.push_notification(
                Notification::warning(
                    "Pick at least two commits: shift-click selects a range, ctrl-click adds \
                     single commits.",
                )
                .title("Nothing to squash"),
                cx,
            );
            return;
        }
        let repo = project.repo.clone();
        cx.spawn_in(window, async move |this, cx| {
            let plan = cx
                .background_spawn(async move { squash::plan(&repo, &picked) })
                .await;
            let _ = this.update_in(cx, |this, window, cx| match plan {
                Ok(plan) => this.open_squash_dialog(plan, window, cx),
                Err(error) => window.push_notification(
                    Notification::warning(crate::app::ops::describe(&error))
                        .title("Cannot squash these commits"),
                    cx,
                ),
            });
        })
        .detach();
    }

    fn open_squash_dialog(
        &mut self,
        plan: SquashPlan,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let count = plan.commits.len();
        let message = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(8)
                .default_value(plan.message.clone())
        });
        let mut notes = vec![format!(
            "Combines {count} commits on {} into one with the message below. The code stays \
             exactly as it is; the old commits stay in the reflog.",
            plan.branch
        )];
        if !plan.between.is_empty() {
            notes.push(format!(
                "They are not next to each other: the later ones move down next to the oldest, \
                 and the {} between them stay, after the squashed commit. If moving them \
                 conflicts, nothing is changed.",
                crate::format::count(plan.between.len(), "commit")
            ));
        }
        if plan.replay > 0 {
            notes.push(format!(
                "{} after them will be replayed on top, unchanged.",
                crate::format::count(plan.replay, "later commit")
            ));
        }
        if !plan.other_refs.is_empty() {
            notes.push(format!(
                "{} also contain these commits and will keep the old ones: {}.",
                if plan.other_refs.len() == 1 {
                    "This branch or tag"
                } else {
                    "These branches and tags"
                },
                plan.other_refs.join(", ")
            ));
        }
        let warning = (!plan.published.is_empty()).then(|| {
            format!(
                "Already pushed to {}. Squashing rewrites that history: you will need to push \
                 with --force-with-lease, and anyone who pulled these commits has to reset.",
                plan.published.join(", ")
            )
        });
        let weak = cx.entity().downgrade();
        // Commits get replayed over uncommitted changes: git needs them set aside.
        let stash = std::rc::Rc::new(
            plan.needs_stash
                .then(crate::app::actions::PromptFlag::autostash),
        );
        let plan = Arc::new(plan);
        let focus = message.clone();
        window.open_dialog(cx, move |dialog, window, cx| {
            let (message, weak, plan, stash) =
                (message.clone(), weak.clone(), plan.clone(), stash.clone());
            let muted = cx.theme().muted_foreground;
            let danger = cx.theme().danger;
            let mut body = v_flex().gap_3().w_full();
            for note in &notes {
                body = body.child(div().text_sm().text_color(muted).child(note.clone()));
            }
            if let Some(w) = &warning {
                body = body.child(div().text_sm().text_color(danger).child(w.clone()));
            }
            body = body.child(
                div()
                    .test_id("squash-message")
                    .child(Textarea::new(&message)),
            );
            if let Some(flag) = stash.as_ref() {
                body = body.pb_1().child(flag.checkbox(0, cx));
            }
            let label: SharedString = format!("Squash {count} commits").into();
            let variant = if warning.is_some() {
                ButtonVariant::Danger
            } else {
                ButtonVariant::Primary
            };
            let _ = window;
            dialog
                .title(label.clone())
                .w(px(560.))
                .child(body)
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(label.clone())
                        .ok_variant(variant)
                        .show_cancel(true),
                )
                .footer(crate::dialog_buttons::DialogButtons::new(label).variant(variant))
                .on_ok(move |_, window, cx| {
                    let text = message.read(cx).value().to_string();
                    if text.trim().is_empty() {
                        return false;
                    }
                    let mut plan = (*plan).clone();
                    plan.autostash = stash.as_ref().as_ref().is_some_and(|f| f.value.get());
                    let _ = weak.update(cx, |this, cx| this.run_squash(plan, text, window, cx));
                    true
                })
        });
        focus.update(cx, |m, cx| m.focus(window, cx));
    }

    fn run_squash(
        &mut self,
        plan: SquashPlan,
        message: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let count = plan.commits.len();
        let new_commit = Arc::new(Mutex::new(None::<String>));
        let slot = new_commit.clone();
        self.run_op_then(
            format!("Squashed {count} commits"),
            move |repo| {
                let hash = squash::squash(repo, &plan, &message)?;
                if let Ok(mut s) = slot.lock() {
                    *s = Some(hash);
                }
                Ok(())
            },
            move |this, _, cx| {
                let hash = new_commit.lock().ok().and_then(|mut s| s.take());
                if let (Some(hash), Some(p)) = (hash, this.active_project_mut()) {
                    // The old commits are gone from the log: select the squashed one.
                    p.selection = Selection::Commit { hash };
                    p.selected_commit = None;
                    p.compare_commit = None;
                    this.reload_selection(cx);
                }
            },
            window,
            cx,
        );
    }
}
