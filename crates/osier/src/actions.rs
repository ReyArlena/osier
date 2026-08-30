//! Branch, stash, tag and history actions, plus small input dialogs.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::ButtonVariant;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::DialogButtonProps;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{Sizable as _, v_flex};
use gpui_kit::*;
use osier_git::ops::reset::ResetMode;
use osier_git::ops::stash::StashOptions;
use osier_git::ops::{branch, reset, stash, tag};

use crate::app::{Osier, Tab};

/// A labelled checkbox inside a prompt dialog.
pub(crate) struct PromptFlag {
    pub label: &'static str,
    pub value: Rc<Cell<bool>>,
}

impl PromptFlag {
    pub fn new(label: &'static str, default: bool) -> Self {
        Self {
            label,
            value: Rc::new(Cell::new(default)),
        }
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
        window.open_dialog(cx, move |dialog, _, _| {
            let input = input.clone();
            let flags = flags.clone();
            let weak = weak.clone();
            let on_ok = on_ok.clone();
            let mut body = v_flex().gap_3().w_full().child(Input::new(&input));
            for (ix, flag) in flags.iter().enumerate() {
                let value = flag.value.clone();
                body = body.child(
                    Checkbox::new(("prompt-flag", ix))
                        .small()
                        .label(flag.label)
                        .checked(value.get())
                        .on_click(move |v, window, _| {
                            value.set(*v);
                            window.refresh();
                        }),
                );
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
                .on_ok(move |_, window, cx| {
                    let text = input.read(cx).value().to_string();
                    let _ = weak.update(cx, |this, cx| on_ok(this, text, window, cx));
                    true
                })
        });
    }

    // ----- branches -----

    pub(crate) fn prompt_new_branch(
        &mut self,
        start_point: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let checkout = PromptFlag::new("Check out the new branch", true);
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
            self.run_op(
                format!("Checked out {local} tracking {remote}"),
                move |repo| branch::checkout_remote(repo, &remote, &local),
                window,
                cx,
            );
        } else {
            let n = name.clone();
            self.run_op(
                format!("Switched to {name}"),
                move |repo| branch::checkout(repo, &n),
                window,
                cx,
            );
        }
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
                "{name} is deleted even if it has unmerged commits. They stay in the reflog for a while."
            )
        } else {
            format!("{name} is deleted. Git refuses if it has commits not merged elsewhere.")
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
        let untracked = PromptFlag::new("Include untracked files", false);
        let keep_index = PromptFlag::new("Keep staged changes in the index", false);
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
            format!("HEAD becomes detached at {short}. Create a branch to keep work made there."),
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
        let annotated = PromptFlag::new("Annotated tag (uses the name as message)", true);
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
                    "Move the branch to {short} and discard every later commit's changes AND all uncommitted work. Only the reflog can bring it back."
                ),
                true,
            ),
        };
        let label = match mode {
            ResetMode::Soft => "Reset (soft)",
            ResetMode::Mixed => "Reset (mixed)",
            ResetMode::Hard => "Reset (hard)",
        };
        self.confirm(
            title,
            body,
            label,
            danger,
            move |this, window, cx| {
                let h = hash.clone();
                this.run_op(
                    format!("Reset to {short}"),
                    move |repo| reset::reset(repo, &h, mode),
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
        self.confirm(
            "Revert commit?",
            format!("A new commit undoing {short} is created on the current branch."),
            "Revert",
            false,
            move |this, window, cx| {
                let h = hash.clone();
                this.run_op(
                    format!("Reverted {short}"),
                    move |repo| reset::revert(repo, &h).map(|_| ()),
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
