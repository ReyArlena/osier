//! Running mutating git operations from the UI: background execution, notifications,
//! history and refresh.

use crate::app_dialog::AppDialogExt as _;
use gpui_kit::component::button::ButtonVariant;
use gpui_kit::component::dialog::DialogButtonProps;
use gpui_kit::component::v_flex;
use gpui_kit::*;
use osier_config::history::{self, HistoryEntry};
use osier_git::{GitError, Repository};

use crate::app::Osier;
use crate::notices::{Notice, WindowNoticeExt as _};

/// One line a user can act on, with the stderr detail for failures.
pub(crate) fn describe(error: &GitError) -> String {
    if let Some(text) = plain_reason(error) {
        return text;
    }
    match error.record() {
        Some(rec) => {
            // git's own error line, not progress such as "From <url>" printed before it.
            let lines = || rec.stderr.lines().map(str::trim).filter(|l| !l.is_empty());
            let detail = lines()
                .find(|l| l.starts_with("fatal:") || l.starts_with("error:"))
                .or_else(|| lines().next())
                .map(|l| {
                    l.trim_start_matches("fatal:")
                        .trim_start_matches("error:")
                        .trim()
                })
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

/// Plain words for git refusals people meet in normal use. git runs with `LC_ALL=C`, so its
/// English messages are stable enough to match. `None` falls back to git's own first line.
fn plain_reason(error: &GitError) -> Option<String> {
    let stderr = error.record().map(|r| r.stderr.as_str()).unwrap_or("");
    match error {
        GitError::BranchNotMerged(_) => Some(
            "The branch has commits that are not on its upstream branch, or on the current \
             branch when it has no upstream, so git kept it. Use Force delete if you no longer \
             need those commits."
                .into(),
        ),
        GitError::Dirty(_) => Some(
            "Your uncommitted changes to some of the same files are in the way, so git did not \
             start. Commit or stash them first, then try again."
                .into(),
        ),
        _ if stderr.contains("contains modified or untracked files") => Some(
            "The worktree has changed or untracked files, so git kept it. Tick \"Remove even \
             with uncommitted changes\" to remove it anyway."
                .into(),
        ),
        _ if stderr.contains("the requested upstream branch")
            && stderr.contains("does not exist") =>
        {
            let name = stderr.split('\'').nth(1).unwrap_or("That remote branch");
            Some(format!(
                "{name} does not exist on the remote yet, so there is nothing to pull from. Push \
                 the branch first to create it."
            ))
        }
        _ => None,
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
            None,
            window,
            cx,
        );
    }

    /// Like [`run_op`](Self::run_op); `on_error` sees a failure first and returns true when it
    /// has dealt with it (for example by opening a dialog), which skips the error notification.
    pub(crate) fn run_op_or_else<F>(
        &mut self,
        summary: impl Into<String>,
        op: F,
        on_error: impl FnOnce(&mut Osier, &GitError, &mut Window, &mut Context<Osier>) -> bool + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) where
        F: FnOnce(&Repository) -> osier_git::Result<()> + Send + 'static,
    {
        self.run_op_inner(
            summary,
            move |repo| op(repo).map(|()| None),
            |_, _, _| {},
            Some(Box::new(on_error)),
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
            None,
            window,
            cx,
        );
    }

    fn run_op_inner<F>(
        &mut self,
        summary: impl Into<String>,
        op: F,
        then: impl FnOnce(&mut Osier, &mut Window, &mut Context<Osier>) + 'static,
        on_error: Option<ErrorHandler>,
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
                        window.push_notice(Notice::success(summary.clone()), cx);
                    }
                    Err(error) => {
                        let text = describe(error);
                        tracing::warn!(%summary, %text, "operation failed");
                        entry.ok = false;
                        entry.error = Some(text.clone());
                        let handled = on_error.is_some_and(|h| h(this, error, window, cx));
                        if !handled {
                            window.push_notice(
                                Notice::error(text)
                                    .title(failure_title(&summary))
                                    .command_log(error),
                                cx,
                            );
                        }
                    }
                }
                if stash_clashed {
                    window.push_notice(autostash_clash_notice(), cx);
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
        flags: Vec<crate::app::actions::PromptFlag>,
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
        window.open_app_dialog(cx, move |dialog, _, cx| {
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
pub(crate) fn autostash_clash_notice() -> Notice {
    Notice::warning(
        "Some of your uncommitted changes clash with the new commits. Those files are marked as \
         conflicts in Changes, and a full copy of your changes is saved in the Stashes tab as \
         \"autostash\". Drop that stash once the files look right.",
    )
    .title("Your changes were put back with conflicts")
}

/// See [`Osier::run_op_or_else`].
type ErrorHandler =
    Box<dyn FnOnce(&mut Osier, &GitError, &mut Window, &mut Context<Osier>) -> bool>;

/// Title for a failed operation. Summaries are written for success ("Deleted branch x"), so
/// turn the leading verb round: "Could not delete branch x". Unknown verbs keep the summary.
pub(crate) fn failure_title(summary: &str) -> String {
    const VERBS: &[(&str, &str)] = &[
        ("Aborted", "abort"),
        ("Added", "add"),
        ("Applied", "apply"),
        ("Checked", "check"),
        ("Cherry-picked", "cherry-pick"),
        ("Committed", "commit"),
        ("Continued", "continue"),
        ("Copied", "copy"),
        ("Created", "create"),
        ("Deleted", "delete"),
        ("Discarded", "discard"),
        ("Dropped", "drop"),
        ("Fast-forwarded", "fast-forward"),
        ("Marked", "mark"),
        ("Merged", "merge"),
        ("Moved", "move"),
        ("Popped", "pop"),
        ("Pruned", "prune"),
        ("Rebased", "rebase"),
        ("Removed", "remove"),
        ("Renamed", "rename"),
        ("Reset", "reset"),
        ("Resolved", "resolve"),
        ("Restored", "restore"),
        ("Reverted", "revert"),
        ("Saved", "save"),
        ("Set", "set"),
        ("Shelved", "shelve"),
        ("Skipped", "skip"),
        ("Squashed", "squash"),
        ("Staged", "stage"),
        ("Stashed", "stash"),
        ("Switched", "switch"),
        ("Tagged", "tag"),
        ("Trashed", "move to the trash"),
        ("Undid", "undo"),
        ("Unshelved", "unshelve"),
        ("Unstaged", "unstage"),
        ("Untracked", "untrack"),
        ("Updated", "update"),
    ];
    let (first, rest) = summary.split_once(' ').unwrap_or((summary, ""));
    match VERBS.iter().find(|(past, _)| *past == first) {
        Some((_, verb)) if rest.is_empty() => format!("Could not {verb}"),
        Some((_, verb)) => format!("Could not {verb} {rest}"),
        None => format!("{summary} failed"),
    }
}

#[cfg(test)]
mod tests {
    use super::failure_title;

    #[test]
    fn failure_titles_read_as_what_did_not_happen() {
        assert_eq!(
            failure_title("Deleted branch x"),
            "Could not delete branch x"
        );
        assert_eq!(
            failure_title("Checked out abc1234"),
            "Could not check out abc1234"
        );
        assert_eq!(failure_title("Continued"), "Could not continue");
        assert_eq!(
            failure_title("Interactive rebase of 3 commits"),
            "Interactive rebase of 3 commits failed"
        );
    }
}
