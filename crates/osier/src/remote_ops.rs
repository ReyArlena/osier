//! Fetch, pull and push from the UI: account resolution, progress, cancellation and the
//! dialogs for the cases that must stop and ask.

use std::sync::mpsc;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::ButtonVariant;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::DialogButtonProps;
use gpui_kit::component::notification::Notification;
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
use crate::ops::describe;

#[derive(Debug, Clone)]
pub enum RemoteOp {
    Fetch(FetchOptions),
    Pull(PullOptions),
    Push(PushOptions),
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
            RemoteOp::Pull(o) => o.remote.clone(),
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
        cx.spawn(async move |this, cx| {
            let accounts = cx
                .background_spawn(async move { osier_auth::gh::accounts() })
                .await;
            let _ = this.update(cx, |this, cx| {
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
        let exe = std::env::current_exe().ok();
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
            window.push_notification(
                Notification::warning(format!("{} is still running", job.label)),
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
            let result = cx
                .background_spawn(async move {
                    let progress = move |p: Progress| {
                        let _ = tx.send(p);
                    };
                    match &op2 {
                        RemoteOp::Fetch(o) => remote::fetch(&repo2, o, &ctx, progress),
                        RemoteOp::Pull(o) => remote::pull(&repo2, o, &ctx, progress),
                        RemoteOp::Push(o) => remote::push(&repo2, o, &ctx, progress),
                    }
                })
                .await;
            let head_after = repo.head_oid().ok().flatten();
            let _ = this.update_in(cx, |this, window, cx| {
                this.remote_job = None;
                let mut entry = HistoryEntry::now(&path, label.clone());
                entry.head_before = head_before;
                entry.head_after = head_after;
                match &result {
                    Ok(_) => window.push_notification(Notification::success(label.clone()), cx),
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
                if let Err(error) = result {
                    this.handle_remote_error(op, error, window, cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
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
                window.push_notification(Notification::info(format!("{label} cancelled")), cx);
            }
            (RemoteOp::Pull(o), GitError::NonFastForward(_))
                if o.strategy == PullStrategy::FfOnly =>
            {
                self.prompt_pull_strategy(o.clone(), window, cx);
            }
            (RemoteOp::Push(o), GitError::NonFastForward(_)) if !o.force_with_lease => {
                self.prompt_push_rejected(o.clone(), window, cx);
            }
            (RemoteOp::Push(o), GitError::NoUpstream(_)) => {
                self.prompt_set_upstream_and_push(o.clone(), window, cx);
            }
            (RemoteOp::Pull(o), GitError::NoUpstream(_)) => {
                self.prompt_set_upstream_and_pull(o.clone(), window, cx);
            }
            (_, GitError::AuthRequired(_)) => {
                let text = describe(&error);
                let hint = if self.gh_accounts.is_empty() {
                    "For GitHub over https, log in once with `gh auth login` and retry. For ssh, add your key to the agent (`ssh-add`)."
                } else {
                    "Choose a GitHub account from the Push menu, or add your ssh key to the agent (`ssh-add`)."
                };
                window.push_notification(
                    Notification::error(format!("{label} failed: authentication"))
                        .message(format!("{text}. {hint}")),
                    cx,
                );
            }
            (_, GitError::Network(_)) => {
                window.push_notification(
                    Notification::error(format!("{label} failed: remote unreachable"))
                        .message(describe(&error)),
                    cx,
                );
            }
            (_, GitError::Conflict(_)) => {
                window.push_notification(
                    Notification::warning(format!("{label}: conflicts"))
                        .message("Resolve the conflicted files in Changes, then continue or abort from the status bar."),
                    cx,
                );
            }
            _ => {
                window.push_notification(
                    Notification::error(format!("{label} failed")).message(describe(&error)),
                    cx,
                );
            }
        }
    }

    /// "Branch diverged": merge, rebase or cancel, optionally remembered per repo.
    fn prompt_pull_strategy(
        &mut self,
        mut options: PullOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let repo_path = self
            .active_project()
            .map(|p| p.path.to_string_lossy().into_owned())
            .unwrap_or_default();
        // A remembered strategy skips the dialog.
        if let Some(s) = self.state.pull_strategies.get(&repo_path) {
            options.strategy = if s == "rebase" {
                PullStrategy::Rebase
            } else {
                PullStrategy::Merge
            };
            self.start_remote(RemoteOp::Pull(options), window, cx);
            return;
        }
        let choice = std::rc::Rc::new(std::cell::Cell::new(0usize));
        let remember = std::rc::Rc::new(std::cell::Cell::new(false));
        let weak = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, _| {
            let (choice, remember, weak, options, repo_path) =
                (choice.clone(), remember.clone(), weak.clone(), options.clone(), repo_path.clone());
            let c2 = choice.clone();
            let r2 = remember.clone();
            dialog
                .title("Branch has diverged")
                .child(
                    v_flex()
                        .gap_3()
                        .child("The remote has commits you do not have, and you have commits it does not. A fast-forward is not possible. Nothing was changed.")
                        .child(
                            RadioGroup::vertical("pull-strategy")
                                .selected_index(Some(choice.get()))
                                .on_click(move |ix, window, _| {
                                    c2.set(*ix);
                                    window.refresh();
                                })
                                .child(Radio::new("merge").label("Merge the remote branch into mine (creates a merge commit)"))
                                .child(Radio::new("rebase").label("Rebase my commits on top of the remote branch")),
                        )
                        .child(
                            Checkbox::new("remember-pull")
                                .small()
                                .label("Remember for this repository")
                                .checked(remember.get())
                                .on_click(move |v, window, _| {
                                    r2.set(*v);
                                    window.refresh();
                                }),
                        ),
                )
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Pull")
                        .ok_variant(ButtonVariant::Primary)
                        .show_cancel(true),
                )
                .on_ok(move |_, window, cx| {
                    let strategy = if choice.get() == 1 { PullStrategy::Rebase } else { PullStrategy::Merge };
                    let mut options = options.clone();
                    options.strategy = strategy;
                    let remember = remember.get();
                    let repo_path = repo_path.clone();
                    let _ = weak.update(cx, |this, cx| {
                        if remember {
                            this.state.pull_strategies.insert(
                                repo_path,
                                if strategy == PullStrategy::Rebase { "rebase" } else { "merge" }.into(),
                            );
                            this.save_state();
                        }
                        this.start_remote(RemoteOp::Pull(options), window, cx);
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
        window.open_dialog(cx, move |dialog, _, _| {
            let (choice, weak, options) = (choice.clone(), weak.clone(), options.clone());
            let c2 = choice.clone();
            dialog
                .title("Push rejected")
                .child(
                    v_flex()
                        .gap_3()
                        .child("The remote branch has commits you do not have. Nothing was pushed.")
                        .child(
                            RadioGroup::vertical("push-rejected")
                                .selected_index(Some(choice.get()))
                                .on_click(move |ix, window, _| {
                                    c2.set(*ix);
                                    window.refresh();
                                })
                                .child(Radio::new("fetch").label("Fetch now, then merge or rebase and push again"))
                                .child(Radio::new("force").label("Force push with lease (replaces the remote branch, only if nobody pushed since your last fetch)")),
                        ),
                )
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Continue")
                        .ok_variant(ButtonVariant::Primary)
                        .show_cancel(true),
                )
                .on_ok(move |_, window, cx| {
                    let force = choice.get() == 1;
                    let options = options.clone();
                    let _ = weak.update(cx, |this, cx| {
                        if force {
                            this.confirm_force_push(options, window, cx);
                        } else {
                            this.start_remote(RemoteOp::fetch_all(), window, cx);
                        }
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
        mut options: PushOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((branch, remote)) = self.current_branch_and_remote() else {
            window.push_notification(
                Notification::warning("No upstream and no remote")
                    .message("Add a remote first (Branches tab)."),
                cx,
            );
            return;
        };
        self.confirm(
            "No upstream yet",
            format!("{branch} is not linked to a remote branch. Push it to {remote}/{branch} and remember that as its upstream?"),
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

    fn prompt_set_upstream_and_pull(
        &mut self,
        options: PullOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((branch, remote)) = self.current_branch_and_remote() else {
            window.push_notification(
                Notification::warning("No upstream and no remote")
                    .message("Add a remote first (Branches tab)."),
                cx,
            );
            return;
        };
        let upstream = format!("{remote}/{branch}");
        self.confirm(
            "No upstream yet",
            format!("{branch} is not linked to a remote branch. Use {upstream} as its upstream and pull from it? (The remote branch must exist.)"),
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
            "The remote branch is replaced with yours. git refuses if the remote moved since your last fetch, so nobody else's work is overwritten silently.",
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
        let always = std::rc::Rc::new(std::cell::Cell::new(true));
        let weak = cx.entity().downgrade();
        let host = needed.host.clone();
        let candidates = needed.candidates.clone();
        window.open_dialog(cx, move |dialog, _, _| {
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
                                }),
                        ),
                )
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Use account")
                        .ok_variant(ButtonVariant::Primary)
                        .show_cancel(true),
                )
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
        window.push_notification("GitHub account choice cleared for this project", cx);
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
            window.push_notification(
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
            window.push_notification(
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
