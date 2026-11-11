//! The rows of the palette's Fetch, Pull, Push, Update branch and Switch branch submenus.
//!
//! The remote submenus offer what the title bar's dropdowns offer. Their shortcuts
//! (`ctrl-shift-f/l/p`, `ctrl-t`) open the submenu; each row is also an action of its own in
//! the keymap, unbound by default, and shows its keys once bound.

use gpui_kit::SharedString;
use gpui_kit::assets::IconName as Lucide;
use osier_git::ops::remote::{FetchOptions, PullOptions, PullStrategy, PushOptions};

use crate::app::Osier;
use crate::app::remote_ops::RemoteOp;
use crate::palette::{Choice, ChoicePick, PaletteMode};

fn row(section: &str, label: impl Into<SharedString>, icon: Lucide, pick: ChoicePick) -> Choice {
    Choice {
        section: section.to_string().into(),
        label: label.into(),
        detail: None,
        icon,
        pick,
        action: None,
    }
}

impl Choice {
    fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    fn keys(mut self, action: &'static str) -> Self {
        self.action = Some(action);
        self
    }
}

impl Osier {
    /// The rows of a remote submenu; empty for any other mode.
    pub(crate) fn remote_choices(&self, mode: PaletteMode) -> Vec<Choice> {
        let remote =
            |label: &str, icon, op| row("", label.to_string(), icon, ChoicePick::Remote(op));
        match mode {
            PaletteMode::Fetch => vec![
                remote(
                    "Fetch all remotes",
                    Lucide::CloudDownload,
                    RemoteOp::Fetch(FetchOptions::default()),
                )
                .keys("fetch_all"),
                remote(
                    "Fetch all and prune deleted branches",
                    Lucide::CloudDownload,
                    RemoteOp::Fetch(FetchOptions {
                        prune: true,
                        ..Default::default()
                    }),
                )
                .keys("fetch_prune"),
                remote(
                    "Fetch origin",
                    Lucide::CloudDownload,
                    RemoteOp::Fetch(FetchOptions {
                        remote: Some("origin".into()),
                        ..Default::default()
                    }),
                )
                .keys("fetch_origin"),
            ],
            PaletteMode::Pull => vec![
                remote(
                    "Pull (fast-forward only)",
                    Lucide::ArrowDown,
                    RemoteOp::Pull(PullOptions::default()),
                )
                .keys("pull_ff_only"),
                remote(
                    "Pull with rebase",
                    Lucide::ArrowDown,
                    RemoteOp::Pull(PullOptions {
                        strategy: PullStrategy::Rebase,
                        ..Default::default()
                    }),
                )
                .keys("pull_rebase"),
                remote(
                    "Pull with merge",
                    Lucide::ArrowDown,
                    RemoteOp::Pull(PullOptions {
                        strategy: PullStrategy::Merge,
                        ..Default::default()
                    }),
                )
                .keys("pull_merge"),
            ],
            PaletteMode::Push => vec![
                remote(
                    "Push",
                    Lucide::ArrowUp,
                    RemoteOp::Push(PushOptions::default()),
                )
                .keys("push_branch"),
                remote(
                    "Push and set upstream (origin)",
                    Lucide::ArrowUp,
                    RemoteOp::Push(PushOptions {
                        remote: Some("origin".into()),
                        set_upstream: true,
                        ..Default::default()
                    }),
                )
                .keys("push_set_upstream"),
                row(
                    "",
                    "Force push with lease…",
                    Lucide::TriangleAlert,
                    ChoicePick::ForcePush,
                )
                .keys("force_push"),
            ],
            PaletteMode::UpdateBranch => {
                let mut rows = vec![
                    row(
                        "",
                        "Update branch",
                        Lucide::FolderSync,
                        ChoicePick::Update(None),
                    )
                    .detail("Asks how, or uses the method you remembered")
                    .keys("update_branch_now"),
                    row(
                        "",
                        "Update branch with merge",
                        Lucide::FolderSync,
                        ChoicePick::Update(Some(PullStrategy::Merge)),
                    )
                    .keys("update_branch_merge"),
                    row(
                        "",
                        "Update branch with rebase",
                        Lucide::FolderSync,
                        ChoicePick::Update(Some(PullStrategy::Rebase)),
                    )
                    .keys("update_branch_rebase"),
                ];
                let remembered = self.active_project().is_some_and(|p| {
                    self.state
                        .update_strategies
                        .contains_key(p.path.to_string_lossy().as_ref())
                });
                if remembered {
                    rows.push(
                        row(
                            "",
                            "Forget the update method for this project",
                            Lucide::Undo2,
                            ChoicePick::ForgetUpdateMethod,
                        )
                        .detail("Update branch asks again next time")
                        .keys("forget_update_method"),
                    );
                }
                rows
            }
            _ => Vec::new(),
        }
    }

    /// "Switch branch": local branches by their last commit, newest first, then remote
    /// branches that have no local branch of the same name yet. The current branch is left
    /// out, so enter on the first row goes back to the branch you were on before.
    pub(crate) fn branch_choices(&self) -> Vec<Choice> {
        let Some(snapshot) = self.active_project().and_then(|p| p.snapshot.as_ref()) else {
            return Vec::new();
        };
        let mut local: Vec<_> = snapshot.branches.iter().filter(|b| !b.is_remote).collect();
        local.sort_by_key(|b| std::cmp::Reverse(b.committed_at));
        let mut remote: Vec<_> = snapshot
            .branches
            .iter()
            .filter(|b| b.is_remote && !b.name.ends_with("/HEAD"))
            .filter(|b| {
                let short = b.name.split_once('/').map_or(b.name.as_str(), |(_, n)| n);
                !local.iter().any(|l| l.name == short)
            })
            .collect();
        remote.sort_by_key(|b| std::cmp::Reverse(b.committed_at));
        let detail = |b: &osier_git::refs::Branch| {
            let mut parts = vec![crate::format::relative_time(b.committed_at)];
            if let Some(ab) = crate::format::ahead_behind(b.ahead, b.behind) {
                parts.push(ab);
            }
            if !b.subject.is_empty() {
                parts.push(b.subject.clone());
            }
            parts.join(" · ")
        };
        local
            .into_iter()
            .filter(|b| !b.is_head)
            .map(|b| {
                row(
                    "Local",
                    b.name.clone(),
                    Lucide::GitBranch,
                    ChoicePick::Checkout {
                        name: b.name.clone(),
                        remote: false,
                    },
                )
                .detail(detail(b))
            })
            .chain(remote.into_iter().map(|b| {
                row(
                    "Remote",
                    b.name.clone(),
                    Lucide::Cloud,
                    ChoicePick::Checkout {
                        name: b.name.clone(),
                        remote: true,
                    },
                )
                .detail(detail(b))
            }))
            .collect()
    }

    /// A submenu row's own keymap action (`push_set_upstream`…): run that row. Does nothing
    /// without a project, or when the row is not offered right now ("Forget the update
    /// method" with nothing remembered).
    pub(crate) fn run_menu_action(
        &mut self,
        id: &str,
        window: &mut gpui_kit::Window,
        cx: &mut gpui_kit::Context<Self>,
    ) {
        if self.active_project().is_none() {
            return;
        }
        let pick = PaletteMode::REMOTE
            .iter()
            .flat_map(|&mode| self.remote_choices(mode))
            .find(|c| c.action == Some(id))
            .map(|c| c.pick);
        if let Some(pick) = pick {
            self.run_choice(pick, window, cx);
        }
    }

    /// What picking a submenu row does, from the palette or from its keymap action.
    pub(crate) fn run_choice(
        &mut self,
        pick: ChoicePick,
        window: &mut gpui_kit::Window,
        cx: &mut gpui_kit::Context<Self>,
    ) {
        match pick {
            ChoicePick::CloneOther => self.prompt_clone(None, window, cx),
            ChoicePick::CloneRepo(account, repo) => {
                self.prompt_clone(Some((account, repo)), window, cx)
            }
            ChoicePick::CreateAccount(account) => self.prompt_create(Some(account), window, cx),
            ChoicePick::CreateLocal => self.prompt_create(None, window, cx),
            ChoicePick::Remote(op) => self.start_remote(op, window, cx),
            ChoicePick::ForcePush => self.confirm_force_push(Default::default(), window, cx),
            ChoicePick::Update(None) => self.update_branch(window, cx),
            ChoicePick::Update(Some(strategy)) => self.update_branch_with(strategy, window, cx),
            ChoicePick::ForgetUpdateMethod => self.forget_update_method(window, cx),
            ChoicePick::Checkout { name, remote } => self.checkout_branch(name, remote, window, cx),
            // Opens the next submenu instead; the palette handles it.
            ChoicePick::CloneAccount(_) => {}
        }
    }

    /// With `palette.search_submenus` on, the submenu rows searched from the commands list:
    /// Switch branch, then the remote submenus in the order the commands list them, each
    /// headed by its submenu's name. Switch project's rows come from `entries_for`.
    pub(crate) fn searchable_sub_choices(&self) -> Vec<Choice> {
        if !self.settings.palette.search_submenus || self.active_project().is_none() {
            return Vec::new();
        }
        let titled = |rows: Vec<Choice>, title: &'static str| {
            rows.into_iter().map(move |mut c| {
                c.section = title.into();
                c
            })
        };
        let mut out: Vec<Choice> = titled(self.branch_choices(), "Switch branch").collect();
        for mode in PaletteMode::REMOTE {
            if let Some(title) = mode.badge() {
                out.extend(titled(self.remote_choices(mode), title));
            }
        }
        out
    }
}
