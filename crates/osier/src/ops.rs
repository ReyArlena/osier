//! Running mutating git operations from the UI: background execution, notifications,
//! history and refresh.

use gpui_kit::component::Sizable as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::dialog::DialogButtonProps;
use gpui_kit::component::notification::Notification;
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
            let result = cx.background_spawn(async move { op(&repo2) }).await;
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
                            Notification::error(format!("{summary} failed"))
                                .message(text)
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
        let title = title.into();
        let body = body.into();
        let ok_label = ok_label.into();
        let weak = cx.entity().downgrade();
        let on_ok = std::rc::Rc::new(on_ok);
        window.open_dialog(cx, move |dialog, _, _| {
            let weak = weak.clone();
            let on_ok = on_ok.clone();
            dialog
                .title(title.clone())
                .child(body.clone())
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
                    let _ = weak.update(cx, |this, cx| on_ok(this, window, cx));
                    true
                })
        });
    }
}
