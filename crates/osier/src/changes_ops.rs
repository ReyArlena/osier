//! Changes tab actions: staging, discarding, committing.

use gpui_kit::*;
use osier_git::ops::commit::CommitOptions;
use osier_git::ops::{commit as commit_ops, discard, stage};

use crate::app::Osier;
use crate::project::Area;

impl Osier {
    pub(crate) fn staged_count(&self) -> usize {
        self.active_project()
            .and_then(|p| p.snapshot.as_ref())
            .map(|s| s.status.staged().count())
            .unwrap_or(0)
    }

    pub(crate) fn commit_message_text(&self, cx: &App) -> String {
        self.commit_message.read(cx).value().to_string()
    }

    pub(crate) fn can_commit(&self, cx: &App) -> bool {
        (self.staged_count() > 0 || self.amend) && !self.commit_message_text(cx).trim().is_empty()
    }

    pub(crate) fn set_message(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let text = text.to_string();
        self.commit_message
            .update(cx, |state, cx| state.set_value(text, window, cx));
    }

    pub(crate) fn toggle_amend(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.amend = !self.amend;
        if self.amend && self.commit_message_text(cx).trim().is_empty() {
            // Prefill with HEAD's message, like PhpStorm's amend checkbox.
            let msg = self
                .active_project()
                .and_then(|p| commit_ops::head_message(&p.repo).ok().flatten());
            if let Some(msg) = msg {
                self.set_message(&msg, window, cx);
            }
        }
        cx.notify();
    }

    pub(crate) fn toggle_signoff(&mut self, cx: &mut Context<Self>) {
        self.signoff = !self.signoff;
        cx.notify();
    }

    pub(crate) fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_commit(cx) {
            return;
        }
        let message = self.commit_message_text(cx);
        let options = CommitOptions {
            message: message.clone(),
            amend: self.amend,
            signoff: self.signoff,
            ..Default::default()
        };
        let subject = message.lines().next().unwrap_or("").to_string();
        if let Some(p) = self.active_project() {
            let path = p.path.clone();
            self.state.push_commit_message(&path, &message);
            self.save_state();
        }
        let summary = if self.amend {
            format!("Amended: {subject}")
        } else {
            format!("Committed: {subject}")
        };
        self.set_message("", window, cx);
        self.amend = false;
        self.run_op(
            summary,
            move |repo| commit_ops::commit(repo, &options).map(|_| ()),
            window,
            cx,
        );
    }

    /// Commit, then push the current branch.
    pub(crate) fn commit_and_push(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_commit(cx) {
            return;
        }
        let message = self.commit_message_text(cx);
        let options = CommitOptions {
            message: message.clone(),
            amend: self.amend,
            signoff: self.signoff,
            ..Default::default()
        };
        let subject = message.lines().next().unwrap_or("").to_string();
        if let Some(p) = self.active_project() {
            let path = p.path.clone();
            self.state.push_commit_message(&path, &message);
            self.save_state();
        }
        self.set_message("", window, cx);
        self.amend = false;
        self.run_op_then(
            format!("Committed: {subject}"),
            move |repo| commit_ops::commit(repo, &options).map(|_| ()),
            |this, window, cx| {
                this.start_remote(crate::remote_ops::RemoteOp::push_default(), window, cx)
            },
            window,
            cx,
        );
    }

    pub(crate) fn stage_paths(
        &mut self,
        paths: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let n = paths.len();
        self.run_op(
            format!("Staged {}", plural(n, "file")),
            move |repo| stage::stage(repo, &paths),
            window,
            cx,
        );
    }

    pub(crate) fn unstage_paths(
        &mut self,
        paths: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let n = paths.len();
        self.run_op(
            format!("Unstaged {}", plural(n, "file")),
            move |repo| stage::unstage(repo, &paths),
            window,
            cx,
        );
    }

    pub(crate) fn stage_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run_op("Staged all changes", stage::stage_all, window, cx);
    }

    pub(crate) fn unstage_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run_op("Unstaged all changes", stage::unstage_all, window, cx);
    }

    /// Discard with a confirmation that says exactly what is lost.
    /// Messages committed before in the active repository, newest first.
    pub(crate) fn recent_commit_messages(&self) -> Vec<String> {
        let Some(p) = self.active_project() else {
            return Vec::new();
        };
        let key = p.path.to_string_lossy().into_owned();
        self.state
            .commit_messages
            .get(&key)
            .cloned()
            .unwrap_or_default()
    }

    /// Space in the Changes list: stage the selected file, or unstage it when staged.
    pub(crate) fn toggle_stage_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((path, area)) = self.diff_area() else {
            return;
        };
        match area {
            Area::Staged => self.unstage_paths(vec![path], window, cx),
            Area::Unstaged | Area::Untracked => self.stage_paths(vec![path], window, cx),
            // Conflicts are resolved first, then marked; staging them here would hide markers.
            Area::Conflicted => {}
        }
    }

    /// Delete in the Changes list: discard the selected file (with the usual confirmation).
    pub(crate) fn discard_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((path, area)) = self.diff_area() {
            self.discard_file(path, area, window, cx);
        }
    }

    pub(crate) fn discard_file(
        &mut self,
        path: String,
        area: Area,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (title, body, ok) = match area {
            Area::Untracked => (
                "Move to trash?",
                format!("{path} is untracked. It will be moved to the system trash."),
                "Move to trash",
            ),
            Area::Staged => (
                "Discard staged changes?",
                format!("{path} will be restored from HEAD. Staged and unstaged changes are lost."),
                "Discard",
            ),
            Area::Unstaged => (
                "Discard changes?",
                format!("Unstaged changes to {path} are lost. Staged changes are kept."),
                "Discard",
            ),
            Area::Conflicted => (
                "Discard conflict resolution?",
                format!("{path} will be restored from HEAD."),
                "Discard",
            ),
        };
        self.confirm(
            title,
            body,
            ok,
            true,
            move |this, window, cx| {
                let p = path.clone();
                let summary = format!("Discarded {p}");
                match area {
                    Area::Untracked => this.run_op(
                        format!("Trashed {p}"),
                        move |repo| discard::trash_untracked(repo, &[p]).map(|_| ()),
                        window,
                        cx,
                    ),
                    Area::Unstaged => this.run_op(
                        summary,
                        move |repo| discard::discard_worktree(repo, &[p]),
                        window,
                        cx,
                    ),
                    Area::Staged | Area::Conflicted => this.run_op(
                        summary,
                        move |repo| discard::discard_all(repo, &[p]),
                        window,
                        cx,
                    ),
                }
            },
            window,
            cx,
        );
    }
}

fn plural(n: usize, word: &str) -> String {
    if n == 1 {
        format!("1 {word}")
    } else {
        format!("{n} {word}s")
    }
}
