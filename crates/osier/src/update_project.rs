//! "Update project" (title bar, left of Fetch): fetch and join the remote's commits with
//! yours by merge or rebase, with uncommitted changes set aside and put back (autostash), so it
//! only stops for real conflicts. Like PhpStorm's Update Project.
//!
//! The method is asked for each time, preselecting the last choice for the repository,
//! unless "Remember my choice for this repository" was ticked. That choice is kept apart from
//! the diverged-pull dialog's (`State::pull_strategies`); both, and the account choice, are
//! listed and cleared in Settings > Projects > Per-project choices.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::button::ButtonVariant;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::DialogButtonProps;
use gpui_kit::component::radio::RadioGroup;
use gpui_kit::component::{Sizable as _, WindowExt as _, v_flex};
use gpui_kit::*;
use osier_config::state::UpdateChoice;
use osier_git::ops::remote::{PullOptions, PullStrategy};

use crate::app::Osier;
use crate::dialog_hint::{hint, hinted_radio};
use crate::remote_ops::RemoteOp;
use crate::test_ids::TestId as _;

fn strategy_name(s: PullStrategy) -> &'static str {
    match s {
        PullStrategy::Rebase => "rebase",
        _ => "merge",
    }
}

fn parse_strategy(s: &str) -> PullStrategy {
    if s == "rebase" {
        PullStrategy::Rebase
    } else {
        PullStrategy::Merge
    }
}

fn update_op(strategy: PullStrategy) -> RemoteOp {
    RemoteOp::Update(PullOptions {
        strategy,
        autostash: true,
        ..PullOptions::default()
    })
}

impl Osier {
    /// The title bar's Update project button.
    pub(crate) fn update_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.active_project() else {
            return;
        };
        let key = project.path.to_string_lossy().into_owned();
        match self.state.update_strategies.get(&key) {
            Some(choice) if choice.remember => {
                let strategy = parse_strategy(&choice.strategy);
                self.start_remote(update_op(strategy), window, cx);
            }
            last => {
                let preselect = match last {
                    Some(choice) => parse_strategy(&choice.strategy),
                    None => self.git_config_strategy(),
                };
                self.update_dialog(key, preselect, window, cx);
            }
        }
    }

    /// What the repository's git config asks pulls to do on the current branch
    /// (`branch.<name>.rebase`, else `pull.rebase`): the first-time preselection.
    fn git_config_strategy(&self) -> PullStrategy {
        let Some(project) = self.active_project() else {
            return PullStrategy::Merge;
        };
        let repo = &project.repo;
        let branch = project
            .snapshot
            .as_ref()
            .and_then(|s| s.status.branch.head.clone());
        let read = |key: &str| osier_git::config::get(repo, key).ok().flatten();
        let value = branch
            .and_then(|b| read(&format!("branch.{b}.rebase")))
            .or_else(|| read("pull.rebase"));
        // `true`, `merges`, `interactive` all rebase; `false` (or unset) merges.
        match value.as_deref().map(str::to_ascii_lowercase).as_deref() {
            None | Some("false" | "no" | "off" | "0") => PullStrategy::Merge,
            Some(_) => PullStrategy::Rebase,
        }
    }

    fn update_dialog(
        &mut self,
        key: String,
        preselect: PullStrategy,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let choice = Rc::new(Cell::new(usize::from(preselect == PullStrategy::Rebase)));
        let remember = Rc::new(Cell::new(false));
        let weak = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            let (choice, remember, weak, key) =
                (choice.clone(), remember.clone(), weak.clone(), key.clone());
            let (c2, r2) = (choice.clone(), remember.clone());
            dialog
                .title("Update project")
                .child(
                    v_flex()
                        .test_id("update-dialog")
                        .gap_3()
                        .pb_1()
                        .child(
                            "Fetches the remote branch and joins its new commits with yours. \
                             Uncommitted changes are set aside first and put back afterwards.",
                        )
                        .child(
                            RadioGroup::vertical("update-strategy")
                                .selected_index(Some(choice.get()))
                                .on_click(move |ix, window, _| {
                                    c2.set(*ix);
                                    window.refresh();
                                })
                                .child(hinted_radio(
                                    "update-merge",
                                    "Merge",
                                    "When both sides have new commits, joins them with a merge \
                                     commit. Otherwise your branch just moves forward.",
                                    cx,
                                ))
                                .child(hinted_radio(
                                    "update-rebase",
                                    "Rebase",
                                    "When both sides have new commits, replays yours after the \
                                     remote ones for a straight history; your commits get new \
                                     IDs. Otherwise your branch just moves forward.",
                                    cx,
                                )),
                        )
                        .child(
                            Checkbox::new("update-remember")
                                .test_id("update-remember")
                                .small()
                                .label("Remember my choice for this repository")
                                .checked(remember.get())
                                .on_click(move |v, window, _| {
                                    r2.set(*v);
                                    window.refresh();
                                })
                                .child(hint(
                                    "Update project then runs straight away, without asking.",
                                    cx,
                                )),
                        )
                        .child(hint(
                            "This choice is saved separately from the one in the \"Branch has \
                             diverged\" pull dialog. Both can be changed in Settings > Projects \
                             > Per-project choices. If both sides changed the same lines, the \
                             update stops so you can resolve the conflicts. Nothing is pushed.",
                            cx,
                        )),
                )
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Update")
                        .ok_variant(ButtonVariant::Primary)
                        .show_cancel(true),
                )
                .footer(
                    crate::dialog_buttons::DialogButtons::new("Update")
                        .variant(ButtonVariant::Primary),
                )
                .on_ok(move |_, window, cx| {
                    let strategy = if choice.get() == 1 {
                        PullStrategy::Rebase
                    } else {
                        PullStrategy::Merge
                    };
                    let (key, remember, weak) = (key.clone(), remember.get(), weak.clone());
                    // After this dialog has closed: closing removes the topmost dialog, which
                    // would otherwise be the account picker `start_remote` may open.
                    window.defer(cx, move |window, cx| {
                        let _ = weak.update(cx, |this, cx| {
                            this.state.update_strategies.insert(
                                key,
                                UpdateChoice {
                                    strategy: strategy_name(strategy).into(),
                                    remember,
                                },
                            );
                            this.save_state();
                            this.start_remote(update_op(strategy), window, cx);
                        });
                    });
                    true
                })
        });
    }

    /// Settings > Projects > Per-project choices: forget the diverged-pull choice.
    pub(crate) fn clear_pull_strategy(&mut self, repo: &str, cx: &mut Context<Self>) {
        self.state.pull_strategies.remove(repo);
        self.save_state();
        cx.notify();
    }

    /// Settings > Projects > Per-project choices: forget the Update project choice, so the
    /// dialog asks again.
    pub(crate) fn clear_update_strategy(&mut self, repo: &str, cx: &mut Context<Self>) {
        self.state.update_strategies.remove(repo);
        self.save_state();
        cx.notify();
    }

    /// Settings > Projects > Per-project choices: "Reset all".
    pub(crate) fn clear_all_project_choices(&mut self, cx: &mut Context<Self>) {
        self.state.pull_strategies.clear();
        self.state.update_strategies.clear();
        self.clear_all_account_overrides(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_strategy, strategy_name};
    use osier_git::ops::remote::PullStrategy;

    #[test]
    fn strategy_names_round_trip() {
        for s in [PullStrategy::Merge, PullStrategy::Rebase] {
            assert_eq!(parse_strategy(strategy_name(s)), s);
        }
        assert_eq!(parse_strategy("anything"), PullStrategy::Merge);
    }
}
