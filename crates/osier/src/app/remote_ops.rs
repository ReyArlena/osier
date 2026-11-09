//! Fetch, pull and push from the UI: account resolution, progress, cancellation and the
//! dialogs for the cases that must stop and ask.

use std::path::PathBuf;
use std::sync::mpsc;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::ButtonVariant;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::DialogButtonProps;
use gpui_kit::component::radio::{Radio, RadioGroup};
use gpui_kit::component::{Sizable as _, v_flex};
use gpui_kit::*;
use osier_auth::RemoteKind;
use osier_config::history::{self, HistoryEntry};
use osier_git::ops::remote::{
    self, FetchOptions, PullOptions, PullStrategy, PushOptions, RemoteContext,
};
use osier_git::{CancelToken, GitError, Progress};

use crate::app::Osier;
use crate::app::ops::describe;
use crate::dialog_hint::{hint, hinted_radio};
use crate::notices::{Notice, WindowNoticeExt as _};

#[derive(Debug, Clone)]
pub enum RemoteOp {
    Fetch(FetchOptions),
    Pull(PullOptions),
    Push(PushOptions),
    /// "Update project": a pull with a merge or rebase picked up front and autostash, so it
    /// only stops for real conflicts.
    Update(PullOptions),
}

impl RemoteOp {
    pub fn fetch_all() -> Self {
        RemoteOp::Fetch(FetchOptions::default())
    }

    pub fn pull_default() -> Self {
        RemoteOp::Pull(PullOptions::default())
    }

    pub fn push_default() -> Self {
        RemoteOp::Push(PushOptions::default())
    }

    fn label(&self) -> String {
        match self {
            RemoteOp::Fetch(o) => match (&o.remote, o.prune) {
                (None, false) => "Fetch all".into(),
                (None, true) => "Fetch all and prune".into(),
                (Some(r), false) => format!("Fetch {r}"),
                (Some(r), true) => format!("Fetch {r} and prune"),
            },
            RemoteOp::Pull(o) => match o.strategy {
                PullStrategy::FfOnly => "Pull".into(),
                PullStrategy::Merge => "Pull with merge".into(),
                PullStrategy::Rebase => "Pull with rebase".into(),
            },
            RemoteOp::Push(o) if o.delete.is_some() => format!(
                "Delete {} on {}",
                o.delete
                    .as_deref()
                    .map(|r| r
                        .trim_start_matches("refs/tags/")
                        .trim_start_matches("refs/heads/"))
                    .unwrap_or_default(),
                o.remote.as_deref().unwrap_or("origin")
            ),
            RemoteOp::Update(o) => match o.strategy {
                PullStrategy::Rebase => "Update project (rebase)".into(),
                _ => "Update project (merge)".into(),
            },
            RemoteOp::Push(o) => match (o.set_upstream, o.force_with_lease) {
                (_, true) => "Force push with lease".into(),
                (true, false) => "Push and set upstream".into(),
                _ => "Push".into(),
            },
        }
    }

    /// Remote this operation talks to, for choosing credentials.
    fn remote_name(&self, upstream_remote: Option<&str>) -> String {
        let explicit = match self {
            RemoteOp::Fetch(o) => o.remote.clone(),
            RemoteOp::Pull(o) | RemoteOp::Update(o) => o.remote.clone(),
            RemoteOp::Push(o) => o.remote.clone(),
        };
        explicit
            .or_else(|| upstream_remote.map(str::to_string))
            .unwrap_or_else(|| "origin".into())
    }
}

/// A running remote operation, shown in the status bar.
pub struct RemoteJob {
    pub label: String,
    pub phase: String,
    pub percent: Option<u8>,
    pub cancel: CancelToken,
    rx: mpsc::Receiver<Progress>,
}

impl RemoteJob {
    /// A job that is not a fetch, pull or push (clone, create), with its own progress.
    pub(crate) fn new(label: String, cancel: CancelToken, rx: mpsc::Receiver<Progress>) -> Self {
        Self {
            label,
            phase: "starting".into(),
            percent: None,
            cancel,
            rx,
        }
    }
}

impl std::fmt::Debug for RemoteJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteJob")
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

/// Credentials could not be decided automatically.
struct AccountNeeded {
    host: String,
    candidates: Vec<String>,
}

impl Osier {
    pub(crate) fn load_gh_accounts(&mut self, cx: &mut Context<Self>) {
        let busy = self.background.begin(crate::background::GH_ACCOUNTS);
        cx.spawn(async move |this, cx| {
            let _busy = busy;
            let accounts = cx
                .background_spawn(async move { osier_auth::gh::accounts() })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.gh_accounts_loaded = true;
                match accounts {
                    Ok(list) => this.gh_accounts = list,
                    Err(e) => tracing::info!(error = %e, "gh accounts unavailable"),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Drain progress from the running job. Returns true when something changed.
    pub(crate) fn poll_remote_job(&mut self) -> bool {
        let Some(job) = &mut self.remote_job else {
            return false;
        };
        let mut changed = false;
        while let Ok(p) = job.rx.try_recv() {
            job.phase = p.phase;
            job.percent = p.percent;
            changed = true;
        }
        changed
    }

    pub(crate) fn cancel_remote_job(&mut self, cx: &mut Context<Self>) {
        if let Some(job) = &self.remote_job {
            job.cancel.cancel();
            cx.notify();
        }
    }

    /// The account to use for `host`, or the candidates to pick from.
    fn resolve_account(
        &self,
        repo_path: &str,
        host: &str,
    ) -> Result<Option<String>, AccountNeeded> {
        let candidates: Vec<String> = self
            .gh_accounts
            .iter()
            .filter(|a| a.host == host)
            .map(|a| a.login.clone())
            .collect();
        if candidates.is_empty() {
            // No gh account for this host: git's own helpers take over.
            return Ok(None);
        }
        if let Some(login) = self.state.accounts.get(repo_path)
            && candidates.contains(login)
        {
            return Ok(Some(login.clone()));
        }
        if let Some(login) = self.settings.auth.default_accounts.get(host)
            && candidates.contains(login)
        {
            return Ok(Some(login.clone()));
        }
        if candidates.len() == 1 {
            return Ok(Some(candidates[0].clone()));
        }
        Err(AccountNeeded {
            host: host.to_string(),
            candidates,
        })
    }

    fn remote_context(&self, op: &RemoteOp) -> Result<RemoteContext, AccountNeeded> {
        let mut ctx = RemoteContext {
            cancel: Some(CancelToken::new()),
            ..Default::default()
        };
        let Some(project) = self.active_project() else {
            return Ok(ctx);
        };
        let exe = crate::runtime_info::own_binary().ok();
        if self.settings.auth.askpass
            && let Some(exe) = &exe
        {
            ctx.env = osier_auth::askpass_env(exe);
        }
        let Some(snapshot) = &project.snapshot else {
            return Ok(ctx);
        };
        let upstream_remote = snapshot
            .status
            .branch
            .upstream
            .as_deref()
            .and_then(|u| u.split_once('/').map(|(r, _)| r));
        let remote_name = op.remote_name(upstream_remote);
        let Some(remote) = snapshot.remotes.iter().find(|r| r.name == remote_name) else {
            return Ok(ctx);
        };
        let url = match op {
            RemoteOp::Push(_) => remote.push_url.as_deref().unwrap_or(&remote.fetch_url),
            _ => &remote.fetch_url,
        };
        if let RemoteKind::Https { host } = osier_auth::classify_url(url) {
            let repo_path = project.path.to_string_lossy().into_owned();
            if let (Some(login), Some(exe)) = (self.resolve_account(&repo_path, &host)?, &exe) {
                ctx.config = osier_auth::credential_config(exe, &login, &host);
            }
        }
        Ok(ctx)
    }

    /// Start a fetch, pull or push on the active project.
    pub(crate) fn start_remote(
        &mut self,
        op: RemoteOp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_project().is_none() {
            return;
        }
        if let Some(job) = &self.remote_job {
            window.push_notice(
                Notice::warning(format!("{} is still running", job.label)),
                cx,
            );
            return;
        }
        let ctx = match self.remote_context(&op) {
            Ok(ctx) => ctx,
            Err(needed) => {
                self.prompt_account(needed, op, window, cx);
                return;
            }
        };
        let (tx, rx) = mpsc::channel();
        let label = op.label();
        self.remote_job = Some(RemoteJob {
            label: label.clone(),
            phase: "starting".into(),
            percent: None,
            cancel: ctx.cancel.clone().unwrap_or_default(),
            rx,
        });
        let Some(project) = self.active_project() else {
            self.remote_job = None;
            return;
        };
        let repo = project.repo.clone();
        let path = project.path.clone();
        let head_before = repo.head_oid().ok().flatten();
        let op2 = op.clone();
        cx.spawn_in(window, async move |this, cx| {
            let repo2 = repo.clone();
            let (result, stash_clashed) = cx
                .background_spawn(async move {
                    let progress = move |p: Progress| {
                        let _ = tx.send(p);
                    };
                    let before = osier_git::stash::autostash_count(&repo2);
                    let result = match &op2 {
                        RemoteOp::Fetch(o) => remote::fetch(&repo2, o, &ctx, progress),
                        RemoteOp::Pull(o) | RemoteOp::Update(o) => {
                            remote::pull(&repo2, o, &ctx, progress)
                        }
                        RemoteOp::Push(o) => remote::push(&repo2, o, &ctx, progress),
                    };
                    (result, osier_git::stash::autostash_count(&repo2) > before)
                })
                .await;
            let head_after = repo.head_oid().ok().flatten();
            let _ = this.update_in(cx, |this, window, cx| {
                this.remote_job = None;
                let mut entry = HistoryEntry::now(&path, label.clone());
                entry.head_before = head_before;
                entry.head_after = head_after;
                match &result {
                    Ok(_) => window.push_notice(Notice::success(label.clone()), cx),
                    Err(e) => {
                        entry.ok = false;
                        entry.error = Some(describe(e));
                    }
                }
                if let Err(e) = history::append(&this.paths, &entry) {
                    tracing::warn!(error = %e, "could not write history");
                }
                if let Some(ix) = this.projects.iter().position(|p| p.path == path) {
                    this.refresh_project(ix, cx);
                }
                if stash_clashed {
                    window.push_notice(crate::app::ops::autostash_clash_notice(), cx);
                }
                if let (RemoteOp::Pull(_) | RemoteOp::Update(_), Ok(_), Some(before), Some(after)) =
                    (&op, &result, &entry.head_before, &entry.head_after)
                {
                    this.offer_keep_ignored(
                        path.clone(),
                        before.clone(),
                        after.clone(),
                        window,
                        cx,
                    );
                }
                if matches!(
                    op,
                    RemoteOp::Fetch(_) | RemoteOp::Pull(_) | RemoteOp::Update(_)
                ) && result.is_ok()
                {
                    this.offer_delete_gone(path.clone(), window, cx);
                }
                if let Err(error) = result {
                    this.handle_remote_error(op, error, window, cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// After a fetch or pull: branches whose upstream was deleted on the remote and whose
    /// commits are all on its default branch (merged pull requests, usually) are offered for
    /// deletion, once per session. Branches with commits of their own are never offered.
    fn offer_delete_gone(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repo) = self
            .projects
            .iter()
            .find(|p| p.path == path)
            .map(|p| p.repo.clone())
        else {
            return;
        };
        cx.spawn_in(window, async move |this, cx| {
            let gone = cx
                .background_spawn(async move { osier_git::ops::branch::gone_branches(&repo) })
                .await
                .unwrap_or_default();
            let _ = this.update_in(cx, |this, window, cx| {
                // Only for the project still on screen: the deletion runs in the active one.
                if this.active_project().map(|p| &p.path) != Some(&path) {
                    return;
                }
                let names: Vec<String> = gone
                    .into_iter()
                    .filter(|b| b.merged)
                    .map(|b| b.name)
                    .filter(|n| !this.gone_offered.contains(&(path.clone(), n.clone())))
                    .collect();
                if names.is_empty() {
                    return;
                }
                for n in &names {
                    this.gone_offered.insert((path.clone(), n.clone()));
                }
                this.confirm_delete_gone(path.clone(), names, window, cx);
            });
        })
        .detach();
    }

    fn confirm_delete_gone(
        &mut self,
        path: PathBuf,
        names: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (title, body, ok) = match names.as_slice() {
            [one] => (
                "Delete a merged branch?".to_string(),
                format!(
                    "{one} was deleted on the remote and all of its commits are already on the \
                     remote's default branch, so deleting it here loses nothing."
                ),
                format!("Delete {one}"),
            ),
            many => (
                format!("Delete {} merged branches?", many.len()),
                format!(
                    "{} were deleted on the remote and all of their commits are already on the \
                     remote's default branch, so deleting them here loses nothing.",
                    crate::format::list(many)
                ),
                format!("Delete {} branches", many.len()),
            ),
        };
        self.confirm(
            title,
            body,
            ok,
            false,
            move |this, window, cx| {
                if this.active_project().map(|p| &p.path) != Some(&path) {
                    return;
                }
                let names = names.clone();
                let summary = match names.as_slice() {
                    [one] => format!("Deleted branch {one}"),
                    many => format!("Deleted {} merged branches", many.len()),
                };
                this.run_op(
                    summary,
                    move |repo| {
                        for name in &names {
                            osier_git::ops::branch::delete(repo, name, true)?;
                        }
                        Ok(())
                    },
                    window,
                    cx,
                );
            },
            window,
            cx,
        );
    }

    fn handle_remote_error(
        &mut self,
        op: RemoteOp,
        error: GitError,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let label = op.label();
        match (&op, &error) {
            (_, GitError::Cancelled(_)) => {
                window.push_notice(Notice::info(format!("{label} cancelled")), cx);
            }
            (RemoteOp::Pull(o), GitError::NonFastForward(_))
                if o.strategy == PullStrategy::FfOnly =>
            {
                self.prompt_pull_strategy(o.clone(), window, cx);
            }
            (RemoteOp::Push(o), GitError::NonFastForward(_)) if !o.force_with_lease => {
                self.prompt_push_rejected(o.clone(), window, cx);
            }
            (RemoteOp::Push(_), GitError::NonFastForward(_)) => {
                // Force with lease refused: the remote moved since the last fetch.
                window.push_notice(
                    Notice::warning(
                        "Someone pushed to the remote branch since your last fetch, so nothing \
                         was replaced. Fetch to see their commits, then decide again.",
                    )
                    .title("Force push stopped"),
                    cx,
                );
            }
            (RemoteOp::Pull(o), GitError::Dirty(record)) if !o.autostash => {
                let mut options = o.clone();
                options.autostash = true;
                // Rebase configured for this repository: git refuses any uncommitted change,
                // whichever files the incoming commits touch.
                let why = if record.stderr.contains("cannot pull with rebase") {
                    "This repository's git config makes pulls rebase, and git does not rebase \
                     while there are uncommitted changes, so it did not pull and nothing was \
                     changed."
                } else {
                    "The incoming commits change files you have uncommitted changes in, so git \
                     did not pull and nothing was changed."
                };
                self.confirm(
                    "Local changes in the way",
                    format!(
                        "{why} Stash and pull sets your uncommitted changes aside, pulls, then \
                         puts them back. If they clash with the incoming changes, those files \
                         show conflicts and a full copy stays in the Stashes tab."
                    ),
                    "Stash and pull",
                    false,
                    move |this, window, cx| {
                        this.start_remote(RemoteOp::Pull(options.clone()), window, cx)
                    },
                    window,
                    cx,
                );
            }
            (RemoteOp::Push(o), GitError::NoUpstream(_)) => {
                self.prompt_set_upstream_and_push(o.clone(), window, cx);
            }
            (RemoteOp::Pull(o) | RemoteOp::Update(o), GitError::NoUpstream(_)) => {
                self.prompt_set_upstream_and_pull(o.clone(), window, cx);
            }
            (RemoteOp::Pull(_) | RemoteOp::Update(_), GitError::UpstreamGone(_)) => {
                self.prompt_upstream_gone(op.clone(), window, cx);
            }
            (_, GitError::AuthRequired(_)) => {
                let text = describe(&error);
                let hint = if self.gh_accounts.is_empty() {
                    "For GitHub over https, log in once with `gh auth login` and retry. For ssh, add your key to the agent (`ssh-add`)."
                } else {
                    "Choose a GitHub account from the Push menu, or add your ssh key to the agent (`ssh-add`)."
                };
                window.push_notice(
                    Notice::error(format!("{text}. {hint}"))
                        .title(format!("{label} failed: authentication"))
                        .command_log(&error),
                    cx,
                );
            }
            (_, GitError::Network(_)) => {
                window.push_notice(
                    Notice::error(describe(&error))
                        .title(format!("{label} failed: remote unreachable"))
                        .command_log(&error),
                    cx,
                );
            }
            (_, GitError::Conflict(_)) => {
                window.push_notice(
                    Notice::warning("Resolve the conflicted files in Changes, then press Continue or Abort in the bar above the tabs.").title(format!("{label}: conflicts")),
                    cx,
                );
            }
            _ => {
                window.push_notice(
                    Notice::error(describe(&error))
                        .title(format!("{label} failed"))
                        .command_log(&error),
                    cx,
                );
            }
        }
    }

    /// "Branch diverged": merge, rebase or cancel. Always asks, with the choice last used for
    /// this repository preselected.
    fn prompt_pull_strategy(
        &mut self,
        options: PullOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let repo_path = self
            .active_project()
            .map(|p| p.path.to_string_lossy().into_owned())
            .unwrap_or_default();
        let last = self.state.pull_strategies.get(&repo_path);
        let choice = std::rc::Rc::new(std::cell::Cell::new(match last.map(String::as_str) {
            Some("rebase") => 1usize,
            _ => 0,
        }));
        let (flags, autostash) = self.autostash_flags();
        let flags = std::rc::Rc::new(flags);
        let weak = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            let (choice, weak, options, repo_path, autostash) = (
                choice.clone(),
                weak.clone(),
                options.clone(),
                repo_path.clone(),
                autostash.clone(),
            );
            let c2 = choice.clone();
            dialog
                .title("Branch has diverged")
                .child(
                    v_flex()
                        .gap_3()
                        .child(
                            "The remote branch has commits you do not have, and you have commits \
                             it does not. Nothing has been changed yet. Choose how to combine \
                             them.",
                        )
                        .child(
                            RadioGroup::vertical("pull-strategy")
                                .selected_index(Some(choice.get()))
                                .on_click(move |ix, window, _| {
                                    c2.set(*ix);
                                    window.refresh();
                                })
                                .child(hinted_radio(
                                    "merge",
                                    "Merge the remote branch into mine",
                                    "Keeps both sets of commits as they are and joins them with \
                                     a new merge commit.",
                                    cx,
                                ))
                                .child(hinted_radio(
                                    "rebase",
                                    "Rebase my commits on top of the remote branch",
                                    "Replays your commits after the remote ones, for a straight \
                                     history. Your commits get new IDs.",
                                    cx,
                                )),
                        )
                        .child(hint(
                            "Only your local branch changes. If both sides changed the same \
                             lines, the pull stops so you can resolve the conflicts. Nothing is \
                             pushed: push afterwards to send the result. Osier preselects your \
                             last choice for this repository next time; clear it in Settings > \
                             Projects > Per-project choices.",
                            cx,
                        ))
                        .pb_1()
                        .children(flags.iter().enumerate().map(|(ix, f)| f.checkbox(ix, cx))),
                )
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Pull")
                        .ok_variant(ButtonVariant::Primary)
                        .show_cancel(true),
                )
                .footer(
                    crate::dialog_buttons::DialogButtons::new("Pull")
                        .variant(ButtonVariant::Primary),
                )
                .on_ok(move |_, window, cx| {
                    let strategy = if choice.get() == 1 {
                        PullStrategy::Rebase
                    } else {
                        PullStrategy::Merge
                    };
                    let mut options = options.clone();
                    options.strategy = strategy;
                    options.autostash = autostash.get();
                    let (repo_path, weak) = (repo_path.clone(), weak.clone());
                    // After this dialog has closed: closing removes the topmost dialog, which
                    // would otherwise be the account picker `start_remote` may open.
                    window.defer(cx, move |window, cx| {
                        let _ = weak.update(cx, |this, cx| {
                            this.state.pull_strategies.insert(
                                repo_path,
                                if strategy == PullStrategy::Rebase {
                                    "rebase"
                                } else {
                                    "merge"
                                }
                                .into(),
                            );
                            this.save_state();
                            this.start_remote(RemoteOp::Pull(options), window, cx);
                        });
                    });
                    true
                })
        });
    }

    /// Push rejected: fetch first, or force with lease (behind its own confirmation).
    fn prompt_push_rejected(
        &mut self,
        options: PushOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let choice = std::rc::Rc::new(std::cell::Cell::new(0usize));
        let weak = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            let (choice, weak, options) = (choice.clone(), weak.clone(), options.clone());
            let c2 = choice.clone();
            dialog
                .title("Push rejected")
                .child(
                    v_flex()
                        .gap_3()
                        .pb_1()
                        .child("The remote branch has commits you do not have. Nothing was pushed.")
                        .child(
                            RadioGroup::vertical("push-rejected")
                                .selected_index(Some(choice.get()))
                                .on_click(move |ix, window, _| {
                                    c2.set(*ix);
                                    window.refresh();
                                })
                                .child(hinted_radio(
                                    "fetch",
                                    "Fetch now",
                                    "Downloads the remote's new commits. Your branch and files \
                                     are not changed. After this you can pull to merge or rebase \
                                     them into your branch, then try pushing again.",
                                    cx,
                                ))
                                .child(hinted_radio(
                                    "force",
                                    "Force push with lease",
                                    "Replaces the remote branch with yours. The remote commits \
                                     you do not have are removed from it. Git stops if anyone \
                                     pushed since your last fetch. You confirm this in the next \
                                     step.",
                                    cx,
                                )),
                        ),
                )
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Continue")
                        .ok_variant(ButtonVariant::Primary)
                        .show_cancel(true),
                )
                .footer(
                    crate::dialog_buttons::DialogButtons::new("Continue")
                        .variant(ButtonVariant::Primary),
                )
                .on_ok(move |_, window, cx| {
                    let force = choice.get() == 1;
                    let options = options.clone();
                    let weak = weak.clone();
                    // After this dialog has closed: closing removes the topmost dialog, which
                    // would otherwise be the force push confirmation opened here.
                    window.defer(cx, move |window, cx| {
                        let _ = weak.update(cx, |this, cx| {
                            if force {
                                this.confirm_force_push(options, window, cx);
                            } else {
                                this.start_remote(RemoteOp::fetch_all(), window, cx);
                            }
                        });
                    });
                    true
                })
        });
    }

    /// Branch name and the remote to pair it with (first remote, preferring `origin`).
    fn current_branch_and_remote(&self) -> Option<(String, String)> {
        let snapshot = self.active_project()?.snapshot.as_ref()?;
        let branch = snapshot.status.branch.head.clone()?;
        let remote = snapshot
            .remotes
            .iter()
            .find(|r| r.name == "origin")
            .or_else(|| snapshot.remotes.first())?
            .name
            .clone();
        Some((branch, remote))
    }

    fn prompt_set_upstream_and_push(
        &mut self,
        options: PushOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((branch, remote)) = self.current_branch_and_remote() else {
            window.push_notice(
                Notice::warning("Add a remote first in the Branches tab.")
                    .title("No upstream and no remote"),
                cx,
            );
            return;
        };
        self.confirm(
            "No upstream yet",
            format!("{branch} is not linked to a branch on {remote} yet. Push it to {remote}/{branch} and link the two, so later pushes and pulls use that branch? This creates {remote}/{branch} if it does not exist."),
            "Push and set upstream",
            false,
            move |this, window, cx| {
                let mut options = options.clone();
                options.remote = Some(remote.clone());
                options.branch = Some(branch.clone());
                options.set_upstream = true;
                this.start_remote(RemoteOp::Push(options), window, cx);
            },
            window,
            cx,
        );
    }

    /// The pull found the branch's upstream deleted on the remote (merged on GitHub, usually):
    /// offer to switch to the default branch, pull it, and delete the local branch when that
    /// loses nothing.
    fn prompt_upstream_gone(&mut self, op: RemoteOp, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repo) = self.active_project().map(|p| p.repo.clone()) else {
            return;
        };
        cx.spawn_in(window, async move |this, cx| {
            let plan = cx
                .background_spawn(async move { osier_git::ops::branch::gone_upstream(&repo) })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.show_upstream_gone(plan, op, window, cx)
            });
        })
        .detach();
    }

    fn show_upstream_gone(
        &mut self,
        plan: Option<osier_git::ops::branch::GoneUpstream>,
        op: RemoteOp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let title = "Branch deleted on the remote";
        let Some(plan) = plan else {
            window.push_notice(
                Notice::warning(
                    "The branch this one tracks no longer exists on the remote, usually because \
                     it was merged. Switch to another branch, or link this one to another \
                     upstream in the Branches tab.",
                )
                .title(title),
                cx,
            );
            return;
        };
        let (branch, remote) = (plan.branch.clone(), plan.remote.clone());
        let Some(default) = plan.default.clone() else {
            window.push_notice(
                Notice::warning(format!(
                    "{remote}/{branch} no longer exists, usually because {branch} was merged \
                     and its branch deleted. Switch to another branch, or link {branch} to \
                     another upstream in the Branches tab."
                ))
                .title(title),
                cx,
            );
            return;
        };
        let body = if plan.merged {
            format!(
                "{remote}/{branch} no longer exists, usually because {branch} was merged and \
                 its branch deleted. Everything on {branch} is already on {remote}/{default}. \
                 Switch to {default} and pull to carry on there."
            )
        } else {
            format!(
                "{remote}/{branch} no longer exists, usually because {branch} was merged and \
                 its branch deleted. Switch to {default} and pull to carry on there. {branch} \
                 stays here: it has commits that are not on {remote}/{default}."
            )
        };
        // Offered only when deleting loses nothing.
        let flag = plan.merged.then(|| {
            crate::app::actions::PromptFlag::new("Also delete the branch here", true).hint(
                "Nothing is lost: all of its commits are already on the remote's default branch.",
            )
        });
        let delete_value = flag.as_ref().map(|f| f.value.clone());
        let flags: Vec<_> = flag.into_iter().collect();
        self.confirm_with_flags(
            title,
            body,
            format!("Switch to {default} and pull"),
            false,
            flags,
            move |this, window, cx| {
                let (b, d) = (branch.clone(), default.clone());
                let delete = delete_value.as_ref().is_some_and(|v| v.get());
                let op = op.clone();
                this.run_op_then(
                    if delete {
                        format!("Switched to {d} and deleted {b}")
                    } else {
                        format!("Switched to {d}")
                    },
                    move |repo| {
                        osier_git::ops::branch::checkout(repo, &d)?;
                        if delete {
                            osier_git::ops::branch::delete(repo, &b, true)?;
                        }
                        Ok(())
                    },
                    move |this, window, cx| this.start_remote(op.clone(), window, cx),
                    window,
                    cx,
                );
            },
            window,
            cx,
        );
    }

    fn prompt_set_upstream_and_pull(
        &mut self,
        options: PullOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((branch, remote)) = self.current_branch_and_remote() else {
            window.push_notice(
                Notice::warning("Add a remote first in the Branches tab.")
                    .title("No upstream and no remote"),
                cx,
            );
            return;
        };
        let upstream = format!("{remote}/{branch}");
        self.confirm(
            "No upstream yet",
            format!("{branch} is not linked to a branch on {remote} yet. Link it to {upstream} and pull from there? This only works if {upstream} already exists on the remote."),
            "Set upstream and pull",
            false,
            move |this, window, cx| {
                let (b, u) = (branch.clone(), upstream.clone());
                let options = options.clone();
                this.run_op_then(
                    format!("Set upstream of {b} to {u}"),
                    move |repo| osier_git::ops::branch::set_upstream(repo, &b, &u),
                    move |this, window, cx| this.start_remote(RemoteOp::Pull(options.clone()), window, cx),
                    window,
                    cx,
                );
            },
            window,
            cx,
        );
    }

    pub(crate) fn confirm_force_push(
        &mut self,
        mut options: PushOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        options.force_with_lease = true;
        self.confirm(
            "Force push with lease?",
            "The remote branch is replaced with yours, and the remote commits your branch does not have are removed from it. Git stops if anyone pushed since your last fetch, so commits you have not seen yet are never removed.",
            "Force push",
            true,
            move |this, window, cx| this.start_remote(RemoteOp::Push(options.clone()), window, cx),
            window,
            cx,
        );
    }

    /// Account picker for hosts with several gh accounts.
    fn prompt_account(
        &mut self,
        needed: AccountNeeded,
        op: RemoteOp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let repo_path = self
            .active_project()
            .map(|p| p.path.to_string_lossy().into_owned())
            .unwrap_or_default();
        let active_ix = needed
            .candidates
            .iter()
            .position(|c| self.gh_accounts.iter().any(|a| a.active && &a.login == c))
            .unwrap_or(0);
        let choice = std::rc::Rc::new(std::cell::Cell::new(active_ix));
        // Never ticked by default: remembering an account is an explicit choice.
        let always = std::rc::Rc::new(std::cell::Cell::new(false));
        let weak = cx.entity().downgrade();
        let host = needed.host.clone();
        let candidates = needed.candidates.clone();
        window.open_dialog(cx, move |dialog, _, cx| {
            let (choice, always, weak, op, repo_path, candidates, host) = (
                choice.clone(),
                always.clone(),
                weak.clone(),
                op.clone(),
                repo_path.clone(),
                candidates.clone(),
                host.clone(),
            );
            let c2 = choice.clone();
            let a2 = always.clone();
            let mut group = RadioGroup::vertical("gh-account")
                .selected_index(Some(choice.get()))
                .on_click(move |ix, window, _| {
                    c2.set(*ix);
                    window.refresh();
                });
            for (ix, login) in candidates.iter().enumerate() {
                group = group.child(Radio::new(("account", ix)).label(login.clone()));
            }
            dialog
                .title(format!("Which {host} account?"))
                .child(
                    v_flex()
                        .gap_3()
                        .pb_1()
                        .child(format!("Several gh accounts are logged in to {host}. Pick the one to use for this repository."))
                        .child(group)
                        .child(
                            Checkbox::new("always-account")
                                .small()
                                .label("Always use this account for this project")
                                .checked(always.get())
                                .on_click(move |v, window, _| {
                                    a2.set(*v);
                                    window.refresh();
                                })
                                .child(hint(
                                    "Unticked, Osier asks again after it restarts. You can \
                                     change it later from the Push button's menu.",
                                    cx,
                                )),
                        ),
                )
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Use account")
                        .ok_variant(ButtonVariant::Primary)
                        .show_cancel(true),
                )
.footer(crate::dialog_buttons::DialogButtons::new("Use account").variant(ButtonVariant::Primary))
                .on_ok(move |_, window, cx| {
                    let login = candidates.get(choice.get()).cloned().unwrap_or_default();
                    let always = always.get();
                    let repo_path = repo_path.clone();
                    let op = op.clone();
                    let _ = weak.update(cx, |this, cx| {
                        this.set_project_account(&repo_path, &login, always);
                        this.start_remote(op, window, cx);
                    });
                    true
                })
        });
    }

    /// Record the account for a project. Without `always` it lasts for this session only.
    pub(crate) fn set_project_account(&mut self, repo_path: &str, login: &str, always: bool) {
        self.state
            .accounts
            .insert(repo_path.to_string(), login.to_string());
        if always {
            self.save_state();
        } else {
            self.session_only_accounts.insert(repo_path.to_string());
        }
    }

    /// Clear the choice so the next operation asks again.
    pub(crate) fn clear_project_account(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.active_project() else {
            return;
        };
        let key = p.path.to_string_lossy().into_owned();
        self.state.accounts.remove(&key);
        self.session_only_accounts.remove(&key);
        self.save_state();
        window.push_notice("GitHub account choice cleared for this project", cx);
        cx.notify();
    }

    /// Open the account picker directly (Push menu).
    pub(crate) fn choose_account(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(snapshot) = self.active_project().and_then(|p| p.snapshot.as_ref()) else {
            return;
        };
        let host =
            snapshot
                .remotes
                .iter()
                .find_map(|r| match osier_auth::classify_url(&r.fetch_url) {
                    RemoteKind::Https { host } => Some(host),
                    _ => None,
                });
        let Some(host) = host else {
            window.push_notice(
                "No https remote: accounts only apply to https remotes (ssh uses your keys)",
                cx,
            );
            return;
        };
        let candidates: Vec<String> = self
            .gh_accounts
            .iter()
            .filter(|a| a.host == host)
            .map(|a| a.login.clone())
            .collect();
        if candidates.is_empty() {
            window.push_notice(
                format!("gh has no account for {host}. Run `gh auth login` first."),
                cx,
            );
            return;
        }
        // Re-run nothing after choosing: a harmless fetch of nothing is confusing, so use a
        // no-op by pushing a Fetch of the same remote only when the user later triggers it.
        self.prompt_account(
            AccountNeeded { host, candidates },
            RemoteOp::fetch_all(),
            window,
            cx,
        );
    }
}
