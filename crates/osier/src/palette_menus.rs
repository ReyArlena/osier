//! The rows of the palette's Fetch, Pull, Push, Update branch and Switch branch submenus.
//!
//! The remote submenus offer what the title bar's dropdowns offer. The row that does what the
//! shortcut does carries its key hint (`ctrl-shift-p` runs a plain push, so "Push" shows it).

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
                .keys("fetch"),
                remote(
                    "Fetch all and prune deleted branches",
                    Lucide::CloudDownload,
                    RemoteOp::Fetch(FetchOptions {
                        prune: true,
                        ..Default::default()
                    }),
                ),
                remote(
                    "Fetch origin",
                    Lucide::CloudDownload,
                    RemoteOp::Fetch(FetchOptions {
                        remote: Some("origin".into()),
                        ..Default::default()
                    }),
                ),
            ],
            PaletteMode::Pull => vec![
                remote(
                    "Pull (fast-forward only)",
                    Lucide::ArrowDown,
                    RemoteOp::Pull(PullOptions::default()),
                )
                .keys("pull"),
                remote(
                    "Pull with rebase",
                    Lucide::ArrowDown,
                    RemoteOp::Pull(PullOptions {
                        strategy: PullStrategy::Rebase,
                        ..Default::default()
                    }),
                ),
                remote(
                    "Pull with merge",
                    Lucide::ArrowDown,
                    RemoteOp::Pull(PullOptions {
                        strategy: PullStrategy::Merge,
                        ..Default::default()
                    }),
                ),
            ],
            PaletteMode::Push => vec![
                remote(
                    "Push",
                    Lucide::ArrowUp,
                    RemoteOp::Push(PushOptions::default()),
                )
                .keys("push"),
                remote(
                    "Push and set upstream (origin)",
                    Lucide::ArrowUp,
                    RemoteOp::Push(PushOptions {
                        remote: Some("origin".into()),
                        set_upstream: true,
                        ..Default::default()
                    }),
                ),
                row(
                    "",
                    "Force push with lease…",
                    Lucide::TriangleAlert,
                    ChoicePick::ForcePush,
                ),
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
                    .keys("update_branch"),
                    row(
                        "",
                        "Update branch with merge",
                        Lucide::FolderSync,
                        ChoicePick::Update(Some(PullStrategy::Merge)),
                    ),
                    row(
                        "",
                        "Update branch with rebase",
                        Lucide::FolderSync,
                        ChoicePick::Update(Some(PullStrategy::Rebase)),
                    ),
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
                        .detail("Update branch asks again next time"),
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
}
