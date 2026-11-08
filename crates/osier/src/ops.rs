//! Running mutating git operations from the UI: background execution, notifications,
//! history and refresh.

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::dialog::DialogButtonProps;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{Sizable as _, v_flex};
use gpui_kit::*;
use osier_config::history::{self, HistoryEntry};
use osier_git::{GitError, Repository};

use crate::app::Osier;

/// One line a user can act on, with the stderr detail for failures.
pub(crate) fn describe(error: &GitError) -> String {
    match error.record() {
        Some(rec) => {
            let detail = rec
                .stderr
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .unwrap_or("");
            if detail.is_empty() {
                error.to_string()
            } else {
                format!("{error}: {detail}")
            }
        }
        None => error.to_string(),
    }
}

impl Osier {
    /// Run `op` on the active repository off the UI thread, then notify, record and refresh.
    pub(crate) fn run_op<F>(
        &mut self,
        summary: impl Into<String>,
        op: F,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) where
        F: FnOnce(&Repository) -> osier_git::Result<()> + Send + 'static,
    {
        self.run_op_then(summary, op, |_, _, _| {}, window, cx);
    }

    /// Like [`run_op`](Self::run_op); `then` runs after a successful operation.
    pub(crate) fn run_op_then<F>(
        &mut self,
        summary: impl Into<String>,
        op: F,
        then: impl FnOnce(&mut Osier, &mut Window, &mut Context<Osier>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) where
        F: FnOnce(&Repository) -> osier_git::Result<()> + Send + 'static,
    {
        self.run_op_inner(
            summary,
            move |repo| op(repo).map(|()| None),
            then,
            window,
            cx,
        );
    }

    /// Like [`run_op`](Self::run_op), but the operation reports its own success message (shown
    /// and recorded instead of `summary`), e.g. where a line was written.
    pub(crate) fn run_op_reporting<F>(
        &mut self,
        summary: impl Into<String>,
        op: F,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) where
        F: FnOnce(&Repository) -> osier_git::Result<String> + Send + 'static,
    {
        self.run_op_inner(
            summary,
            move |repo| op(repo).map(Some),
            |_, _, _| {},
            window,
            cx,
        );
    }

    fn run_op_inner<F>(
        &mut self,
        summary: impl Into<String>,
        op: F,
        then: impl FnOnce(&mut Osier, &mut Window, &mut Context<Osier>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) where
        F: FnOnce(&Repository) -> osier_git::Result<Option<String>> + Send + 'static,
    {
        let Some(project) = self.active_project() else {
            return;
        };
        let summary = summary.into();
        let repo = project.repo.clone();
        let path = project.path.clone();
        let head_before = repo.head_oid().ok().flatten();
        cx.spawn_in(window, async move |this, cx| {
            let repo2 = repo.clone();
            let (result, stash_clashed) = cx
                .background_spawn(async move {
                    let before = osier_git::stash::autostash_count(&repo2);
                    let result = op(&repo2);
                    (result, osier_git::stash::autostash_count(&repo2) > before)
                })
                .await;
            let head_after = repo.head_oid().ok().flatten();
            let _ = this.update_in(cx, |this, window, cx| {
                let summary = match &result {
                    Ok(Some(message)) => message.clone(),
                    _ => summary.clone(),
                };
                let mut entry = HistoryEntry::now(&path, summary.clone());
                entry.head_before = head_before;
                entry.head_after = head_after;
                match &result {
                    Ok(_) => {
                        window.push_notification(Notification::success(summary.clone()), cx);
                    }
                    Err(error) => {
                        let text = describe(error);
                        tracing::warn!(%summary, %text, "operation failed");
                        entry.ok = false;
                        entry.error = Some(text.clone());
                        let owner = cx.weak_entity();
                        window.push_notification(
                            Notification::error(text)
                                .title(format!("{summary} failed"))
                                .action(move |_, _, _| {
                                    let owner = owner.clone();
                                    Button::new("show-command-log")
                                        .ghost()
                                        .xsmall()
                                        .label("Command log")
                                        .on_click(move |_, _, cx| {
                                            let _ = owner.update(cx, |this, cx| {
                                                if !this.command_log_visible {
                                                    this.toggle_command_log(cx);
                                                }
                                            });
                                        })
                                }),
                            cx,
                        );
                    }
                }
                if stash_clashed {
                    window.push_notification(autostash_clash_notice(), cx);
                }
                if let Err(e) = history::append(&this.paths, &entry) {
                    tracing::warn!(error = %e, "could not write history");
                }
                if let Some(ix) = this.projects.iter().position(|p| p.path == path) {
                    this.refresh_project(ix, cx);
                }
                if result.is_ok() {
                    then(this, window, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Confirmation dialog. `on_ok` runs when the user confirms.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn confirm(
        &mut self,
        title: impl Into<SharedString>,
        body: impl Into<SharedString>,
        ok_label: impl Into<SharedString>,
        danger: bool,
        on_ok: impl Fn(&mut Osier, &mut Window, &mut Context<Osier>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm_with_flags(title, body, ok_label, danger, Vec::new(), on_ok, window, cx);
    }

    /// `confirm` with checkboxes under the body. `on_ok` reads them through their values.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn confirm_with_flags(
        &mut self,
        title: impl Into<SharedString>,
        body: impl Into<SharedString>,
        ok_label: impl Into<SharedString>,
        danger: bool,
        flags: Vec<crate::actions::PromptFlag>,
        on_ok: impl Fn(&mut Osier, &mut Window, &mut Context<Osier>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let flags = std::rc::Rc::new(flags);
        let title = title.into();
        let body = body.into();
        let ok_label = ok_label.into();
        let weak = cx.entity().downgrade();
        let on_ok = std::rc::Rc::new(on_ok);
        window.open_dialog(cx, move |dialog, _, cx| {
            let weak = weak.clone();
            let on_ok = on_ok.clone();
            // pb_1: the dialog clips at the body's bottom edge, where a last checkbox's focus
            // ring sits.
            let mut content = v_flex().gap_3().pb_1().w_full().child(body.clone());
            for (ix, flag) in flags.iter().enumerate() {
                content = content.child(flag.checkbox(ix, cx));
            }
            dialog
                .title(title.clone())
                .child(content)
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(ok_label.clone())
                        .ok_variant(if danger {
                            ButtonVariant::Danger
                        } else {
                            ButtonVariant::Primary
                        })
                        .show_cancel(true),
                )
                .footer(
                    crate::dialog_buttons::DialogButtons::new(ok_label.clone()).variant(
                        if danger {
                            ButtonVariant::Danger
                        } else {
                            ButtonVariant::Primary
                        },
                    ),
                )
                .on_ok(move |_, window, cx| {
                    // Run once this dialog has closed. Closing removes the topmost dialog, so a
                    // dialog opened straight from here would be the one that closes.
                    let (weak, on_ok) = (weak.clone(), on_ok.clone());
                    window.defer(cx, move |window, cx| {
                        let _ = weak.update(cx, |this, cx| on_ok(this, window, cx));
                    });
                    true
                })
        });
    }
}

/// Shown when git could not put stashed changes back cleanly after an operation.
pub(crate) fn autostash_clash_notice() -> Notification {
    Notification::warning(
        "Some of your uncommitted changes clash with the new commits. Those files are marked as \
         conflicts in Changes, and a full copy of your changes is saved in the Stashes tab as \
         \"autostash\". Drop that stash once the files look right.",
    )
    .title("Your changes were put back with conflicts")
}
