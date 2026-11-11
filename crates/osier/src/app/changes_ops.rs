//! Changes tab actions: staging, discarding, committing.

use gpui_kit::*;
use osier_git::ops::commit::CommitOptions;
use osier_git::ops::{commit as commit_ops, discard, stage};

use osier_git::staging::Check;

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
        if self.amend
            && self.commit_message_text(cx).trim().is_empty()
            && let Some(repo) = self.active_project().map(|p| p.repo.clone())
        {
            // Prefill with HEAD's message, like PhpStorm's amend checkbox (`git log` in the
            // background). Only if amend is still on and the box still empty when it is back.
            self.in_background_user(
                crate::background::READING_COMMIT,
                move || commit_ops::head_message(&repo).ok().flatten(),
                |this, msg, window, cx| {
                    if let Some(msg) = msg
                        && this.amend
                        && this.commit_message_text(cx).trim().is_empty()
                    {
                        this.set_message(&msg, window, cx);
                    }
                },
                window,
                cx,
            );
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
        self.set_message("", window, cx);
        self.amend = false;
        self.run_op_then(
            format!("Committed: {subject}"),
            move |repo| commit_ops::commit(repo, &options).map(|_| ()),
            |this, window, cx| {
                this.start_remote(crate::app::remote_ops::RemoteOp::push_default(), window, cx)
            },
            window,
            cx,
        );
    }

    /// What `git add` needs so that `paths` end up fully staged: only the ones with something
    /// left to stage. A deletion that is already staged has no file left, and naming it makes
    /// `git add` fail for every path. Paths the status does not list are kept as given.
    fn paths_to_stage(&self, paths: Vec<String>) -> Vec<String> {
        paths
            .into_iter()
            .filter(|p| {
                self.change_entry(p)
                    .is_none_or(|e| e.is_untracked() || e.is_unstaged())
            })
            .collect()
    }

    /// What `git restore --staged` needs so that `paths` end up fully unstaged: the ones with
    /// something staged, and for a staged rename also the path it came from (otherwise the old
    /// path stays deleted in the index).
    fn paths_to_unstage(&self, paths: Vec<String>) -> Vec<String> {
        let mut out = Vec::new();
        for p in paths {
            match self.change_entry(&p) {
                Some(e) if !e.is_staged() => {}
                Some(e) => {
                    out.extend(e.orig_path.filter(|_| e.index == 'R'));
                    out.push(p);
                }
                None => out.push(p),
            }
        }
        out
    }

    pub(crate) fn stage_paths(
        &mut self,
        paths: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let n = paths.len();
        let paths = self.paths_to_stage(paths);
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
        let paths = self.paths_to_unstage(paths);
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
    /// The last five commit messages on HEAD (from any tool), newest first.
    pub(crate) fn recent_commit_messages(&self) -> Vec<String> {
        self.active_project()
            .and_then(|p| p.snapshot.as_ref())
            .map(|s| s.recent_messages.clone())
            .unwrap_or_default()
    }

    /// The status entry of a changed or untracked `path` in the active project.
    pub(crate) fn change_entry(&self, path: &str) -> Option<osier_git::status::StatusEntry> {
        self.active_project()
            .and_then(|p| p.snapshot.as_ref())
            .and_then(|s| s.status.entries.iter().find(|e| e.path == path).cloned())
    }

    /// The checkbox of `path` in the Changes list: what clicking it does follows from this.
    pub(crate) fn change_check(&self, path: &str) -> Option<Check> {
        self.change_entry(path)
            .as_ref()
            .and_then(osier_git::staging::file_check)
    }

    /// Space in the Changes list: tick the selected file (stage all of it), or clear it when
    /// it is ticked (unstage it). A dash ticks, like a click on the box.
    pub(crate) fn toggle_stage_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((path, area)) = self.diff_area() else {
            return;
        };
        match area {
            Area::Changed if self.change_check(&path) == Some(Check::Staged) => {
                self.unstage_paths(vec![path], window, cx)
            }
            Area::Changed | Area::Untracked => self.stage_paths(vec![path], window, cx),
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
        // A file that is new in the index has no HEAD copy to go back to: rolling it back
        // unstages it and moves it to the trash, like an unversioned file.
        let added = area == Area::Changed
            && self
                .change_entry(&path)
                .is_some_and(|e| e.index == 'A' && e.orig_path.is_none());
        let (title, body, ok) = match area {
            Area::Untracked => (
                "Move to trash?",
                format!("{path} is untracked. It will be moved to the system trash."),
                "Move to trash",
            ),
            Area::Changed if added => (
                "Roll back new file?",
                format!("{path} is new. It will be unstaged and moved to the system trash."),
                "Move to trash",
            ),
            Area::Changed => (
                "Roll back changes?",
                format!(
                    "{path} will be restored from HEAD. Its staged and unstaged changes are lost."
                ),
                "Roll back",
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
                let summary = format!("Rolled back {p}");
                match area {
                    Area::Untracked => this.run_op(
                        format!("Trashed {p}"),
                        move |repo| discard::trash_untracked(repo, &[p]).map(|_| ()),
                        window,
                        cx,
                    ),
                    Area::Changed if added => this.run_op(
                        format!("Trashed {p}"),
                        move |repo| {
                            stage::unstage(repo, std::slice::from_ref(&p))?;
                            discard::trash_untracked(repo, &[p]).map(|_| ())
                        },
                        window,
                        cx,
                    ),
                    Area::Changed | Area::Conflicted => this.run_op(
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
