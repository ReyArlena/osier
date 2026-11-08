//! Branch, stash, tag and history actions, plus small input dialogs.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::ButtonVariant;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::DialogButtonProps;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{Sizable as _, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use osier_git::ops::integrate::{self, ConflictSide, MergeMode};
use osier_git::ops::reset::ResetMode;
use osier_git::ops::stash::StashOptions;
use osier_git::ops::{branch, remote, reset, stash, tag};
use osier_git::repo::InProgress;

use crate::app::{Osier, Tab};
use crate::dialog_hint::hint;

/// A labelled checkbox inside a prompt dialog.
pub(crate) struct PromptFlag {
    pub label: &'static str,
    /// What the option does, shown under the label.
    pub hint: Option<&'static str>,
    pub value: Rc<Cell<bool>>,
}

impl PromptFlag {
    pub fn new(label: &'static str, default: bool) -> Self {
        Self {
            label,
            hint: None,
            value: Rc::new(Cell::new(default)),
        }
    }

    pub fn hint(mut self, hint: &'static str) -> Self {
        self.hint = Some(hint);
        self
    }

    /// For merge, rebase and pull, where git itself sets the changes aside and puts them back.
    pub fn autostash() -> Self {
        Self::new("Stash my uncommitted changes while this runs", true).hint(
            "Your uncommitted changes are set aside first and put back when git finishes. If it \
             stops for conflicts, they come back once you continue or abort. If they clash with \
             the new commits, those files show conflicts and a full copy stays in the Stashes \
             tab.",
        )
    }

    /// For steps git cannot autostash: the changes are saved, never put back by Osier.
    pub fn stash_first(default: bool, hint: &'static str) -> Self {
        Self::new(
            "Save my uncommitted changes to the Stashes tab first",
            default,
        )
        .hint(hint)
    }

    /// The checkbox for this flag inside a dialog, with its hint underneath.
    pub fn checkbox(&self, ix: usize, cx: &App) -> Checkbox {
        let value = self.value.clone();
        Checkbox::new(("prompt-flag", ix))
            .small()
            .label(self.label)
            .checked(value.get())
            .on_click(move |v, window, _| {
                value.set(*v);
                window.refresh();
            })
            .when_some(self.hint, |this, text| this.child(hint(text, cx)))
    }
}

impl Osier {
    /// Dialog with one text input and optional checkboxes. `on_ok` gets the text.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prompt_text(
        &mut self,
        title: impl Into<SharedString>,
        placeholder: impl Into<SharedString>,
        default: impl Into<SharedString>,
        ok_label: impl Into<SharedString>,
        flags: Vec<PromptFlag>,
        on_ok: impl Fn(&mut Osier, String, &mut Window, &mut Context<Osier>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let title = title.into();
        let ok_label = ok_label.into();
        let placeholder = placeholder.into();
        let default = default.into();
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(placeholder)
                .default_value(default)
        });
        let flags = Rc::new(flags);
        let weak = cx.entity().downgrade();
        let on_ok = Rc::new(on_ok);
        let focus = input.clone();
        window.open_dialog(cx, move |dialog, _, cx| {
            let input = input.clone();
            let flags = flags.clone();
            let weak = weak.clone();
            let on_ok = on_ok.clone();
            // pb_1: the dialog clips at the body's bottom edge, where a last checkbox's focus
            // ring sits.
            let mut body = v_flex().gap_3().pb_1().w_full().child(Input::new(&input));
            for (ix, flag) in flags.iter().enumerate() {
                body = body.child(flag.checkbox(ix, cx));
            }
            dialog
                .title(title.clone())
                .child(body)
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(ok_label.clone())
                        .ok_variant(ButtonVariant::Primary)
                        .show_cancel(true),
                )
                .footer(
                    crate::dialog_buttons::DialogButtons::new(ok_label.clone())
                        .variant(ButtonVariant::Primary),
                )
                .on_ok(move |_, window, cx| {
                    let text = input.read(cx).value().to_string();
                    // Run once this dialog has closed. Closing removes the topmost dialog, so a
                    // dialog opened straight from here would be the one that closes.
                    let (weak, on_ok) = (weak.clone(), on_ok.clone());
                    window.defer(cx, move |window, cx| {
                        let _ = weak.update(cx, |this, cx| on_ok(this, text, window, cx));
                    });
                    true
                })
        });
        // Type straight away, as in the clone and create dialogs.
        focus.update(cx, |i, cx| i.focus(window, cx));
    }

    // ----- branches -----

    pub(crate) fn prompt_new_branch(
        &mut self,
        start_point: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let checkout = PromptFlag::new("Check out the new branch", true)
            .hint("Switches to the new branch right after creating it.");
        let checkout_value = checkout.value.clone();
        let title = match &start_point {
            Some(s) => format!("New branch at {}", &s[..s.len().min(7)]),
            None => "New branch".to_string(),
        };
        self.prompt_text(
            title,
            "branch-name",
            "",
            "Create",
            vec![checkout],
            move |this, name, window, cx| {
                let name = name.trim().to_string();
                if name.is_empty() {
                    return;
                }
                let start = start_point.clone();
                let checkout = checkout_value.get();
                this.run_op(
                    format!("Created branch {name}"),
                    move |repo| branch::create(repo, &name, start.as_deref(), checkout),
                    window,
                    cx,
                );
                this.set_tab(Tab::Branches, cx);
            },
            window,
            cx,
        );
    }

    pub(crate) fn checkout_branch(
        &mut self,
        name: String,
        is_remote: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if is_remote {
            // `origin/feature` becomes a local `feature` tracking it.
            let local = name
                .split_once('/')
                .map(|(_, b)| b.to_string())
                .unwrap_or(name.clone());
            let remote = name.clone();
            let retry = name.clone();
            self.run_op_or_else(
                format!("Checked out {local} tracking {remote}"),
                move |repo| branch::checkout_remote(repo, &remote, &local),
                move |this, error, window, cx| {
                    this.offer_stash_and_switch(error, retry, true, window, cx)
                },
                window,
                cx,
            );
        } else {
            let n = name.clone();
            self.run_op_or_else(
                format!("Switched to {name}"),
                move |repo| branch::checkout(repo, &n),
                move |this, error, window, cx| {
                    this.offer_stash_and_switch(error, name, false, window, cx)
                },
                window,
                cx,
            );
        }
    }

    /// git would not switch because uncommitted changes are in the way: offer to save them
    /// to the Stashes tab and switch. They stay there; switching does not bring them along.
    fn offer_stash_and_switch(
        &mut self,
        error: &osier_git::GitError,
        name: String,
        is_remote: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !matches!(error, osier_git::GitError::Dirty(_)) {
            return false;
        }
        self.confirm(
            format!("Switch to {name}?"),
            format!(
                "Switching to {name} would overwrite your uncommitted changes to some files, so \
                 git stopped and nothing was changed. Save and switch puts your uncommitted \
                 changes in the Stashes tab first, then switches. They do not come with you: \
                 apply them from the Stashes tab when you want them, on any branch."
            ),
            "Save and switch",
            false,
            move |this, window, cx| {
                let name = name.clone();
                let note = format!("Saved before switching to {name}");
                this.run_op(
                    format!("Switched to {name}"),
                    move |repo| {
                        stash_before(repo, note)?;
                        if is_remote {
                            let local = name
                                .split_once('/')
                                .map(|(_, b)| b.to_string())
                                .unwrap_or(name.clone());
                            branch::checkout_remote(repo, &name, &local)
                        } else {
                            branch::checkout(repo, &name)
                        }
                    },
                    window,
                    cx,
                );
            },
            window,
            cx,
        );
        true
    }

    pub(crate) fn prompt_rename_branch(
        &mut self,
        old: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let default = old.clone();
        self.prompt_text(
            format!("Rename {old}"),
            "new-name",
            default,
            "Rename",
            vec![],
            move |this, new, window, cx| {
                let new = new.trim().to_string();
                if new.is_empty() || new == old {
                    return;
                }
                let o = old.clone();
                let n = new.clone();
                this.run_op(
                    format!("Renamed {o} to {n}"),
                    move |repo| branch::rename(repo, &o, &n),
                    window,
                    cx,
                );
            },
            window,
            cx,
        );
    }

    pub(crate) fn confirm_delete_branch(
        &mut self,
        name: String,
        force: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let body = if force {
            format!(
                "{name} is deleted even if it has commits that are on no other branch. Those \
                 commits are not erased straight away, but they are hard to find again."
            )
        } else {
            format!(
                "{name} is deleted. Git refuses if some of its commits are not on its upstream \
                 branch yet, or on the current branch when it has no upstream."
            )
        };
        self.confirm(
            if force {
                "Force delete branch?"
            } else {
                "Delete branch?"
            },
            body,
            if force { "Force delete" } else { "Delete" },
            true,
            move |this, window, cx| {
                let n = name.clone();
                this.run_op(
                    format!("Deleted branch {n}"),
                    move |repo| branch::delete(repo, &n, force),
                    window,
                    cx,
                );
            },
            window,
            cx,
        );
    }

    // ----- stash -----

    pub(crate) fn prompt_stash(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let untracked = PromptFlag::new("Include untracked files", false)
            .hint("New files git is not tracking yet are stashed too. Otherwise they stay where they are.");
        let keep_index = PromptFlag::new("Keep staged changes", false)
            .hint("Staged changes are saved in the stash and also stay staged in your files.");
        let (u, k) = (untracked.value.clone(), keep_index.value.clone());
        self.prompt_text(
            "Stash changes",
            "Optional message",
            "",
            "Stash",
            vec![untracked, keep_index],
            move |this, message, window, cx| {
                let options = StashOptions {
                    message: Some(message.trim().to_string()).filter(|m| !m.is_empty()),
                    keep_index: k.get(),
                    include_untracked: u.get(),
                    paths: Vec::new(),
                };
                this.run_op(
                    "Stashed changes",
                    move |repo| stash::push(repo, &options),
                    window,
                    cx,
                );
                this.set_tab(Tab::Stashes, cx);
            },
            window,
            cx,
        );
    }

    pub(crate) fn stash_apply(
        &mut self,
        index: usize,
        pop: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let summary = if pop {
            format!("Popped stash@{{{index}}}")
        } else {
            format!("Applied stash@{{{index}}}")
        };
        self.run_op(
            summary,
            move |repo| {
                if pop {
                    stash::pop(repo, index)
                } else {
                    stash::apply(repo, index)
                }
            },
            window,
            cx,
        );
    }

    pub(crate) fn confirm_stash_drop(
        &mut self,
        index: usize,
        message: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm(
            "Drop stash?",
            format!(
                "stash@{{{index}}} \"{message}\" is deleted. This cannot be undone from the UI."
            ),
            "Drop",
            true,
            move |this, window, cx| {
                this.run_op(
                    format!("Dropped stash@{{{index}}}"),
                    move |repo| stash::drop(repo, index),
                    window,
                    cx,
                );
            },
            window,
            cx,
        );
    }

    // ----- log / history -----

    pub(crate) fn checkout_commit(
        &mut self,
        hash: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let short = hash[..hash.len().min(7)].to_string();
        self.confirm(
            "Check out commit?",
            format!("You will be on commit {short} instead of on a branch. Git calls this a detached HEAD. Commits made here belong to no branch and are easy to lose, so create a branch first if you plan to make changes."),
            "Check out",
            false,
            move |this, window, cx| {
                let h = hash.clone();
                this.run_op(
                    format!("Checked out {short}"),
                    move |repo| branch::checkout_detached(repo, &h),
                    window,
                    cx,
                );
            },
            window,
            cx,
        );
    }

    pub(crate) fn prompt_tag(
        &mut self,
        target: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let annotated = PromptFlag::new("Annotated tag", true)
            .hint("Also records who made the tag and when. The tag name is used as its message.");
        let a = annotated.value.clone();
        let short = target[..target.len().min(7)].to_string();
        self.prompt_text(
            format!("Tag {short}"),
            "v1.0.0",
            "",
            "Create tag",
            vec![annotated],
            move |this, name, window, cx| {
                let name = name.trim().to_string();
                if name.is_empty() {
                    return;
                }
                let t = target.clone();
                let n = name.clone();
                let msg = a.get().then(|| name.clone());
                this.run_op(
                    format!("Tagged {short} as {n}"),
                    move |repo| tag::create(repo, &n, Some(&t), msg.as_deref()),
                    window,
                    cx,
                );
            },
            window,
            cx,
        );
    }

    pub(crate) fn confirm_delete_tag(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm(
            "Delete tag?",
            format!("Tag {name} is deleted locally. Copies already pushed to a remote stay there."),
            "Delete",
            true,
            move |this, window, cx| {
                let n = name.clone();
                this.run_op(
                    format!("Deleted tag {n}"),
                    move |repo| tag::delete(repo, &n),
                    window,
                    cx,
                );
            },
            window,
            cx,
        );
    }

    /// Delete a tag on a remote. Goes through the remote job so the usual account choice
    /// and progress apply.
    pub(crate) fn confirm_delete_remote_tag(
        &mut self,
        name: String,
        remote_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm(
            format!("Delete tag on {remote_name}?"),
            format!("Tag {name} is removed from {remote_name}. Other clones keep their copy until they prune tags."),
            "Delete on remote",
            true,
            move |this, window, cx| {
                this.start_remote(
                    crate::remote_ops::RemoteOp::Push(remote::PushOptions {
                        remote: Some(remote_name.clone()),
                        delete: Some(format!("refs/tags/{name}")),
                        ..Default::default()
                    }),
                    window,
                    cx,
                );
            },
            window,
            cx,
        );
    }

    pub(crate) fn confirm_reset(
        &mut self,
        hash: String,
        mode: ResetMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let short = hash[..hash.len().min(7)].to_string();
        let (title, body, danger) = match mode {
            ResetMode::Soft => (
                "Soft reset?",
                format!("Move the branch to {short}. Later commits' changes stay staged."),
                false,
            ),
            ResetMode::Mixed => (
                "Mixed reset?",
                format!(
                    "Move the branch to {short}. Later commits' changes stay in the working tree, unstaged."
                ),
                false,
            ),
            ResetMode::Hard => (
                "Hard reset?",
                format!(
                    "Move the branch to {short}. Later commits leave the branch and all \
                     uncommitted changes are lost. The later commits can be recovered from the \
                     reflog for a while. The uncommitted changes cannot be recovered."
                ),
                true,
            ),
        };
        let label = match mode {
            ResetMode::Soft => "Soft reset",
            ResetMode::Mixed => "Mixed reset",
            ResetMode::Hard => "Hard reset",
        };
        let body = if mode == ResetMode::Hard && self.has_uncommitted_changes() {
            format!(
                "Move the branch to {short}. Later commits leave the branch, and your uncommitted \
                 changes are thrown away unless you save them below. The later commits can be \
                 recovered from the reflog for a while."
            )
        } else {
            body
        };
        let (flags, save) = if mode == ResetMode::Hard {
            self.stash_first_flags(
                true,
                "They are kept there instead of being lost. Osier does not put them back \
                 afterwards: apply them from the Stashes tab when you want them again.",
            )
        } else {
            (Vec::new(), Rc::new(Cell::new(false)))
        };
        self.confirm_with_flags(
            title,
            body,
            label,
            danger,
            flags,
            move |this, window, cx| {
                let h = hash.clone();
                let save = save.get();
                this.run_op(
                    format!("Reset to {short}"),
                    move |repo| {
                        if save {
                            stash_before(
                                repo,
                                format!("Saved before hard reset to {}", &h[..h.len().min(7)]),
                            )?;
                        }
                        reset::reset(repo, &h, mode)
                    },
                    window,
                    cx,
                );
            },
            window,
            cx,
        );
    }

    pub(crate) fn confirm_revert(
        &mut self,
        hash: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let short = hash[..hash.len().min(7)].to_string();
        let (flags, save) = self.stash_first_flags(false, STASH_FIRST_ONLY_IF_REFUSED);
        self.confirm_with_flags(
            "Revert commit?",
            format!("A new commit undoing {short} is created on the current branch."),
            "Revert",
            false,
            flags,
            move |this, window, cx| {
                let h = hash.clone();
                let save = save.get();
                let note = format!("Saved before reverting {short}");
                this.run_op(
                    format!("Reverted {short}"),
                    move |repo| {
                        if save {
                            stash_before(repo, note)?;
                        }
                        reset::revert(repo, &h).map(|_| ())
                    },
                    window,
                    cx,
                );
            },
            window,
            cx,
        );
    }

    pub(crate) fn confirm_undo_commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm(
            "Undo last commit?",
            "The last commit is removed from the branch. Its changes stay staged so you can fix and recommit.",
            "Undo commit",
            false,
            |this, window, cx| {
                this.run_op("Undid last commit", reset::undo_last_commit, window, cx);
            },
            window,
            cx,
        );
    }

    // ----- merge, rebase, cherry-pick, in-progress -----

    pub(crate) fn merge_branch(
        &mut self,
        branch: String,
        mode: MergeMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let label = match mode {
            MergeMode::Default => format!("Merged {branch}"),
            MergeMode::NoFastForward => format!("Merged {branch} (merge commit)"),
            MergeMode::FastForwardOnly => format!("Fast-forwarded to {branch}"),
            MergeMode::Squash => format!("Squashed {branch} into the index"),
        };
        if !self.has_uncommitted_changes() {
            let b = branch.clone();
            self.run_op(
                label,
                move |repo| integrate::merge(repo, &b, mode, false),
                window,
                cx,
            );
            return;
        }
        // Only asks when there is something git might refuse over.
        let (flags, autostash) = self.autostash_flags();
        self.confirm_with_flags(
            format!("Merge {branch}?"),
            format!(
                "You have uncommitted changes. Without stashing them, git stops before \
                 merging if {branch} changes any of the files you have edited."
            ),
            "Merge",
            false,
            flags,
            move |this, window, cx| {
                let b = branch.clone();
                let autostash = autostash.get();
                this.run_op(
                    label.clone(),
                    move |repo| integrate::merge(repo, &b, mode, autostash),
                    window,
                    cx,
                );
            },
            window,
            cx,
        );
    }

    pub(crate) fn confirm_rebase_onto(
        &mut self,
        onto: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current = self
            .active_project()
            .and_then(|p| p.snapshot.as_ref())
            .and_then(|s| s.status.branch.head.clone())
            .unwrap_or_else(|| "the current branch".into());
        let (flags, autostash) = self.autostash_flags();
        self.confirm_with_flags(
            "Rebase?",
            format!("{current} is replayed on top of {onto}. Its commits get new hashes. If it was already pushed, the next push needs force-with-lease."),
            "Rebase",
            false,
            flags,
            move |this, window, cx| {
                let o = onto.clone();
                let autostash = autostash.get();
                this.run_op(
                    format!("Rebased onto {o}"),
                    move |repo| integrate::rebase(repo, &o, autostash),
                    window,
                    cx,
                );
            },
            window,
            cx,
        );
    }

    pub(crate) fn confirm_cherry_pick(
        &mut self,
        hash: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let short = hash[..hash.len().min(7)].to_string();
        let (flags, save) = self.stash_first_flags(false, STASH_FIRST_ONLY_IF_REFUSED);
        self.confirm_with_flags(
            "Cherry-pick?",
            format!("{short} is applied as a new commit on the current branch."),
            "Cherry-pick",
            false,
            flags,
            move |this, window, cx| {
                let h = hash.clone();
                let save = save.get();
                let note = format!("Saved before cherry-picking {short}");
                this.run_op(
                    format!("Cherry-picked {short}"),
                    move |repo| {
                        if save {
                            stash_before(repo, note)?;
                        }
                        integrate::cherry_pick(repo, std::slice::from_ref(&h))
                    },
                    window,
                    cx,
                );
            },
            window,
            cx,
        );
    }

    /// Uncommitted changes to tracked files, the ones a stash would set aside. Untracked
    /// files do not count: git leaves them alone.
    pub(crate) fn has_uncommitted_changes(&self) -> bool {
        self.active_project()
            .and_then(|p| p.snapshot.as_ref())
            .is_some_and(|s| {
                s.status
                    .entries
                    .iter()
                    .any(|e| e.is_staged() || e.is_unstaged())
            })
    }

    /// `[PromptFlag::autostash()]` when there is something to stash, else nothing.
    pub(crate) fn autostash_flags(&self) -> (Vec<PromptFlag>, Rc<Cell<bool>>) {
        if self.has_uncommitted_changes() {
            let flag = PromptFlag::autostash();
            let value = flag.value.clone();
            (vec![flag], value)
        } else {
            (Vec::new(), Rc::new(Cell::new(false)))
        }
    }

    /// `[PromptFlag::stash_first(..)]` when there is something to save, else nothing.
    fn stash_first_flags(
        &self,
        default: bool,
        hint: &'static str,
    ) -> (Vec<PromptFlag>, Rc<Cell<bool>>) {
        if self.has_uncommitted_changes() {
            let flag = PromptFlag::stash_first(default, hint);
            let value = flag.value.clone();
            (vec![flag], value)
        } else {
            (Vec::new(), Rc::new(Cell::new(false)))
        }
    }

    pub(crate) fn in_progress(&self) -> Option<InProgress> {
        self.active_project()?.snapshot.as_ref()?.in_progress
    }

    pub(crate) fn continue_in_progress(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run_op("Continued", integrate::continue_in_progress, window, cx);
    }

    pub(crate) fn skip_in_progress(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run_op(
            "Skipped the current commit",
            integrate::skip_in_progress,
            window,
            cx,
        );
    }

    pub(crate) fn confirm_abort_in_progress(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let what = match self.in_progress() {
            Some(InProgress::Merge) => "merge",
            Some(InProgress::Rebase) => "rebase",
            Some(InProgress::CherryPick) => "cherry-pick",
            Some(InProgress::Revert) => "revert",
            Some(InProgress::Bisect) => "bisect",
            None => return,
        };
        let body = match what {
            "merge" => "Your branch returns to where it was before the merge started, and \
                        conflicts you resolved so far are lost. If you had uncommitted changes \
                        when the merge started, git may not be able to bring them back exactly."
                .to_string(),
            "bisect" => "Git goes back to the commit you were on before the bisect started, and \
                         the good and bad marks made so far are lost."
                .to_string(),
            _ => format!(
                "Your branch returns to where it was before the {what} started, and conflicts \
                 you resolved so far are lost."
            ),
        };
        self.confirm(
            format!("Abort the {what}?"),
            body,
            "Abort",
            true,
            |this, window, cx| this.run_op("Aborted", integrate::abort, window, cx),
            window,
            cx,
        );
    }

    pub(crate) fn resolve_conflict_with(
        &mut self,
        path: String,
        side: ConflictSide,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let label = match side {
            ConflictSide::Ours => format!("Took ours for {path}"),
            ConflictSide::Theirs => format!("Took theirs for {path}"),
        };
        self.run_op(
            label,
            move |repo| integrate::resolve_with(repo, std::slice::from_ref(&path), side),
            window,
            cx,
        );
    }

    pub(crate) fn mark_resolved(
        &mut self,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_op(
            format!("Marked {path} resolved"),
            move |repo| integrate::mark_resolved(repo, std::slice::from_ref(&path)),
            window,
            cx,
        );
    }

    /// Interactive rebase dialog (rebase_dialog module).
    pub(crate) fn open_interactive_rebase(
        &mut self,
        base: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_rebase_dialog(base, window, cx);
    }

    // ----- worktrees -----

    /// New worktree on a new branch, in a sibling directory `<repo>-<branch>` by default.
    pub(crate) fn prompt_add_worktree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.active_project() else {
            return;
        };
        let root = project.path.clone();
        let existing = PromptFlag::new(
            "Check out an existing branch instead of creating one",
            false,
        );
        let existing_value = existing.value.clone();
        self.prompt_text(
            "New worktree: branch name",
            "feature-x",
            "",
            "Next",
            vec![existing],
            move |this, branch, window, cx| {
                let branch = branch.trim().to_string();
                if branch.is_empty() {
                    return;
                }
                let default_dir = root
                    .parent()
                    .map(|p| {
                        p.join(format!(
                            "{}-{}",
                            root.file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default(),
                            branch.replace('/', "-")
                        ))
                    })
                    .unwrap_or_else(|| root.join(&branch));
                let new_branch = !existing_value.get();
                this.prompt_text(
                    "New worktree: directory",
                    "path",
                    default_dir.to_string_lossy().into_owned(),
                    "Create worktree",
                    vec![],
                    move |this, dir, window, cx| {
                        let dir = osier_config::scan::expand_home(std::path::Path::new(dir.trim()));
                        if dir.as_os_str().is_empty() {
                            return;
                        }
                        let (b, d) = (branch.clone(), dir.clone());
                        this.run_op(
                            format!("Added worktree {} on {b}", crate::format::tilde(&d)),
                            move |repo| {
                                osier_git::ops::worktree::add(repo, &d, &b, new_branch, None)
                            },
                            window,
                            cx,
                        );
                    },
                    window,
                    cx,
                );
            },
            window,
            cx,
        );
    }

    pub(crate) fn confirm_remove_worktree(
        &mut self,
        path: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = crate::format::tilde(&path);
        let force = PromptFlag::new("Remove even with uncommitted changes", false).hint(
            "Changed and untracked files in that worktree are lost. Otherwise git stops if there are any.",
        );
        let force_value = force.value.clone();
        self.confirm_with_flags(
            "Remove worktree?",
            format!(
                "The folder {name} and everything in it is deleted from your disk. Its branch \
                 and the commits on it are kept."
            ),
            "Remove",
            true,
            vec![force],
            move |this, window, cx| {
                let p = path.clone();
                let force = force_value.get();
                this.run_op(
                    format!("Removed worktree {name}"),
                    move |repo| osier_git::ops::worktree::remove(repo, &p, force),
                    window,
                    cx,
                );
            },
            window,
            cx,
        );
    }

    // ----- remotes -----

    pub(crate) fn prompt_add_remote(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt_text(
            "Add remote: name",
            "origin",
            "",
            "Next",
            vec![],
            move |this, name, window, cx| {
                let name = name.trim().to_string();
                if name.is_empty() {
                    return;
                }
                this.prompt_text(
                    format!("Add remote {name}: URL"),
                    "https://github.com/user/repo.git",
                    "",
                    "Add remote",
                    vec![],
                    move |this, url, window, cx| {
                        let url = url.trim().to_string();
                        if url.is_empty() {
                            return;
                        }
                        let (n, u) = (name.clone(), url.clone());
                        this.run_op(
                            format!("Added remote {n}"),
                            move |repo| remote::add(repo, &n, &u),
                            window,
                            cx,
                        );
                    },
                    window,
                    cx,
                );
            },
            window,
            cx,
        );
    }

    pub(crate) fn prompt_remote_url(
        &mut self,
        name: String,
        current: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prompt_text(
            format!("URL for {name}"),
            "https://github.com/user/repo.git",
            current,
            "Save",
            vec![],
            move |this, url, window, cx| {
                let url = url.trim().to_string();
                if url.is_empty() {
                    return;
                }
                let (n, u) = (name.clone(), url.clone());
                this.run_op(
                    format!("Updated URL of {n}"),
                    move |repo| remote::set_url(repo, &n, &u, false),
                    window,
                    cx,
                );
            },
            window,
            cx,
        );
    }

    pub(crate) fn prompt_rename_remote(
        &mut self,
        old: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let default = old.clone();
        self.prompt_text(
            format!("Rename remote {old}"),
            "name",
            default,
            "Rename",
            vec![],
            move |this, new, window, cx| {
                let new = new.trim().to_string();
                if new.is_empty() || new == old {
                    return;
                }
                let (o, n) = (old.clone(), new.clone());
                this.run_op(
                    format!("Renamed remote {o} to {n}"),
                    move |repo| remote::rename(repo, &o, &n),
                    window,
                    cx,
                );
            },
            window,
            cx,
        );
    }

    pub(crate) fn confirm_remove_remote(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm(
            "Remove remote?",
            format!("{name} and its remote-tracking branches are removed from this repository. The remote repository itself is untouched."),
            "Remove",
            true,
            move |this, window, cx| {
                let n = name.clone();
                this.run_op(format!("Removed remote {n}"), move |repo| remote::remove(repo, &n), window, cx);
            },
            window,
            cx,
        );
    }

    pub(crate) fn copy_to_clipboard(
        &mut self,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
        window.push_notification(format!("Copied {text}"), cx);
    }
}

/// Hint for the stash-first option where git would mostly cope without it.
const STASH_FIRST_ONLY_IF_REFUSED: &str = "Only needed if git refuses because the commit \
     changes files you have edited. Osier does not put them back afterwards: apply them from \
     the Stashes tab when you want them again.";

/// Save uncommitted changes to tracked files as a stash with `message` (untracked files are
/// left where they are, as the operations that follow leave them alone too).
fn stash_before(repo: &osier_git::Repository, message: String) -> osier_git::Result<()> {
    stash::push(
        repo,
        &StashOptions {
            message: Some(message),
            ..Default::default()
        },
    )
}
