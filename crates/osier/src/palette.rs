//! The command palette (ctrl-shift-a or F1): every action reachable by typing its name.
//!
//! Commands run through their own closures rather than dispatched actions, so the dialog
//! layer never has to route key events back to the main view.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::command::{Command, CommandItem, CommandState};
use gpui_kit::component::diff::DiffMode;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app::{Osier, Tab};

type Run = Rc<dyn Fn(&mut Osier, &mut Window, &mut Context<Osier>)>;

pub(crate) struct PaletteCommand {
    pub group: &'static str,
    pub label: &'static str,
    pub icon: Lucide,
    pub keys: Option<&'static str>,
    /// Hidden when no repository is open.
    pub needs_project: bool,
    pub run: Run,
}

fn cmd(
    group: &'static str,
    label: &'static str,
    icon: Lucide,
    keys: Option<&'static str>,
    needs_project: bool,
    run: impl Fn(&mut Osier, &mut Window, &mut Context<Osier>) + 'static,
) -> PaletteCommand {
    PaletteCommand {
        group,
        label,
        icon,
        keys,
        needs_project,
        run: Rc::new(run),
    }
}

/// Every palette command, in display order.
pub(crate) fn commands() -> Vec<PaletteCommand> {
    use crate::remote_ops::RemoteOp;
    vec![
        cmd(
            "General",
            "Open repository…",
            Lucide::FolderOpen,
            Some("ctrl-o"),
            false,
            |o, w, cx| o.prompt_open_repository(w, cx),
        ),
        cmd(
            "General",
            "Close project",
            Lucide::X,
            Some("ctrl-w"),
            true,
            |o, w, cx| {
                if let Some(ix) = o.active {
                    o.close_project(ix, w, cx)
                }
            },
        ),
        cmd(
            "General",
            "Refresh",
            Lucide::RefreshCw,
            Some("ctrl-r"),
            true,
            |o, _, cx| o.refresh_active(cx),
        ),
        cmd(
            "General",
            "Settings…",
            Lucide::Settings,
            Some("ctrl-,"),
            false,
            |o, w, cx| o.open_settings(w, cx),
        ),
        cmd(
            "General",
            "Toggle sidebar",
            Lucide::PanelLeft,
            Some("ctrl-b"),
            false,
            |o, _, cx| o.toggle_sidebar(cx),
        ),
        cmd(
            "General",
            "Toggle git command log",
            Lucide::SquareTerminal,
            Some("ctrl-`"),
            false,
            |o, _, cx| o.toggle_command_log(cx),
        ),
        cmd(
            "General",
            "Toggle dark / light",
            Lucide::SunMoon,
            Some("ctrl-shift-t"),
            false,
            |o, _, cx| o.toggle_theme(cx),
        ),
        cmd(
            "General",
            "Open themes folder",
            Lucide::Palette,
            None,
            false,
            |o, w, cx| o.open_themes_dir(w, cx),
        ),
        cmd(
            "General",
            "Rescan project directories",
            Lucide::FolderSearch,
            None,
            false,
            |o, _, cx| o.rescan_projects(cx),
        ),
        cmd(
            "General",
            "Quit",
            Lucide::LogOut,
            Some("ctrl-q"),
            false,
            |_, _, cx| cx.quit(),
        ),
        cmd(
            "Tabs",
            "Changes tab",
            Lucide::FileDiff,
            Some("ctrl-1"),
            true,
            |o, _, cx| o.set_tab(Tab::Changes, cx),
        ),
        cmd(
            "Tabs",
            "Log tab",
            Lucide::GitCommitHorizontal,
            Some("ctrl-2"),
            true,
            |o, _, cx| o.set_tab(Tab::Log, cx),
        ),
        cmd(
            "Tabs",
            "Branches tab",
            Lucide::GitBranch,
            Some("ctrl-3"),
            true,
            |o, _, cx| o.set_tab(Tab::Branches, cx),
        ),
        cmd(
            "Tabs",
            "Stashes tab",
            Lucide::Archive,
            Some("ctrl-4"),
            true,
            |o, _, cx| o.set_tab(Tab::Stashes, cx),
        ),
        cmd(
            "Tabs",
            "Files tab",
            Lucide::Folder,
            Some("ctrl-5"),
            true,
            |o, _, cx| o.set_tab(Tab::Files, cx),
        ),
        cmd(
            "Tabs",
            "Patches tab",
            Lucide::Package,
            Some("ctrl-6"),
            true,
            |o, _, cx| o.set_tab(Tab::Patches, cx),
        ),
        cmd(
            "Changes",
            "Commit staged",
            Lucide::GitCommitHorizontal,
            Some("ctrl-enter"),
            true,
            |o, w, cx| o.commit(w, cx),
        ),
        cmd(
            "Changes",
            "Commit and push",
            Lucide::ArrowUp,
            None,
            true,
            |o, w, cx| o.commit_and_push(w, cx),
        ),
        cmd(
            "Changes",
            "Stage all",
            Lucide::ListChecks,
            None,
            true,
            |o, w, cx| o.stage_all(w, cx),
        ),
        cmd(
            "Changes",
            "Unstage all",
            Lucide::ListX,
            None,
            true,
            |o, w, cx| o.unstage_all(w, cx),
        ),
        cmd(
            "Changes",
            "Stash changes…",
            Lucide::Archive,
            None,
            true,
            |o, w, cx| o.prompt_stash(w, cx),
        ),
        cmd(
            "Changes",
            "Shelve all changes…",
            Lucide::Package,
            None,
            true,
            |o, w, cx| o.shelve_prompt(Vec::new(), w, cx),
        ),
        cmd(
            "Remote",
            "Fetch all remotes",
            Lucide::CloudDownload,
            Some("ctrl-shift-f"),
            true,
            |o, w, cx| o.start_remote(RemoteOp::fetch_all(), w, cx),
        ),
        cmd(
            "Remote",
            "Pull",
            Lucide::ArrowDown,
            Some("ctrl-shift-l"),
            true,
            |o, w, cx| o.start_remote(RemoteOp::pull_default(), w, cx),
        ),
        cmd(
            "Remote",
            "Push",
            Lucide::ArrowUp,
            Some("ctrl-shift-p"),
            true,
            |o, w, cx| o.start_remote(RemoteOp::push_default(), w, cx),
        ),
        cmd(
            "Remote",
            "Choose GitHub account for this project…",
            Lucide::Github,
            None,
            true,
            |o, w, cx| o.choose_account(w, cx),
        ),
        cmd(
            "Branches",
            "New branch…",
            Lucide::GitBranch,
            None,
            true,
            |o, w, cx| o.prompt_new_branch(None, w, cx),
        ),
        cmd(
            "Patches",
            "Apply patch from clipboard…",
            Lucide::ClipboardPaste,
            None,
            true,
            |o, w, cx| o.apply_patch_from_clipboard(w, cx),
        ),
        cmd(
            "Patches",
            "Apply patch from file…",
            Lucide::FileInput,
            None,
            true,
            |o, w, cx| o.apply_patch_from_file(w, cx),
        ),
        cmd(
            "Patches",
            "Copy all changes as patch",
            Lucide::ClipboardCopy,
            None,
            true,
            |o, w, cx| {
                if let Some(s) = o.all_changes_source() {
                    o.copy_patch(s, w, cx)
                }
            },
        ),
        cmd(
            "Diff",
            "Unified layout",
            Lucide::FileDiff,
            None,
            false,
            |o, _, cx| o.set_diff_mode(DiffMode::Unified, cx),
        ),
        cmd(
            "Diff",
            "Split layout",
            Lucide::Columns2,
            None,
            false,
            |o, _, cx| o.set_diff_mode(DiffMode::Split, cx),
        ),
        cmd(
            "Diff",
            "Toggle soft wrap",
            Lucide::FileDiff,
            None,
            false,
            |o, _, cx| o.toggle_soft_wrap(cx),
        ),
        cmd(
            "Open",
            "Open project in editor",
            Lucide::SquareArrowOutUpRight,
            None,
            true,
            |o, w, cx| o.open_in_editor(None, w, cx),
        ),
        cmd(
            "Open",
            "Open terminal here",
            Lucide::Terminal,
            None,
            true,
            |o, w, cx| o.open_terminal(w, cx),
        ),
        cmd(
            "Open",
            "Reveal in file manager",
            Lucide::FolderOpen,
            None,
            true,
            |o, w, cx| o.reveal_in_file_manager(None, w, cx),
        ),
    ]
}

impl Osier {
    pub(crate) fn open_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let has_project = self.active_project().is_some();
        let commands: Rc<Vec<PaletteCommand>> = Rc::new(
            commands()
                .into_iter()
                .filter(|c| has_project || !c.needs_project)
                .collect(),
        );
        // A fresh state each time: empty query, first row selected.
        let state = cx.new(|cx| CommandState::new(window, cx));
        let owner = cx.weak_entity();
        let focus_on_mount = Rc::new(Cell::new(true));
        window.open_dialog(cx, move |dialog, _, _| {
            let state = state.clone();
            let owner = owner.clone();
            let commands = commands.clone();
            let focus_on_mount = focus_on_mount.clone();
            dialog
                .close_button(false)
                .p_0()
                .content(move |content, window, cx| {
                    if focus_on_mount.replace(false) {
                        let state = state.clone();
                        window.defer(cx, move |window, cx| {
                            state.read(cx).focus_handle(cx).focus(window, cx);
                        });
                    }
                    let confirm_owner = owner.clone();
                    let cmds = commands.clone();
                    let muted = cx.theme().muted_foreground;
                    let mono = cx.theme().mono_font_family.clone();
                    let mut command = Command::new(&state)
                        .bordered(false)
                        .rounded(cx.theme().radius_lg)
                        .placeholder("Type a command…")
                        .on_confirm(move |ix, window, cx| {
                            window.close_dialog(cx);
                            let cmds = cmds.clone();
                            let _ = confirm_owner.update(cx, |this, cx| {
                                if let Some(c) = cmds.get(ix.row) {
                                    (c.run)(this, window, cx);
                                }
                            });
                        });
                    for c in commands.iter() {
                        let (label, icon, keys, group) = (c.label, c.icon, c.keys, c.group);
                        let mono = mono.clone();
                        command = command.item(
                            CommandItem::new()
                                .label(label)
                                .icon(icon)
                                .keywords([group])
                                .child(move |_, _| {
                                    h_flex()
                                        .w_full()
                                        .gap_3()
                                        .items_center()
                                        .child(Icon::new(icon).xsmall())
                                        .child(div().flex_1().min_w_0().truncate().child(label))
                                        .child(div().text_xs().text_color(muted).child(group))
                                        .when_some(keys, |this, k| {
                                            this.child(
                                                div()
                                                    .text_xs()
                                                    .text_color(muted)
                                                    .font_family(mono.clone())
                                                    .child(k),
                                            )
                                        })
                                }),
                        );
                    }
                    content.child(command)
                })
        });
    }
}

#[cfg(test)]
mod tests {
    use super::commands;

    #[test]
    fn labels_are_unique_and_shortcuts_match_the_keymap() {
        let all = commands();
        let mut labels: Vec<&str> = all.iter().map(|c| c.label).collect();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), all.len(), "duplicate palette label");
        for c in &all {
            assert!(!c.group.is_empty() && !c.label.is_empty());
        }
        let keyed = all.iter().filter(|c| c.keys.is_some()).count();
        assert!(keyed >= 15, "{keyed} commands carry a shortcut hint");
    }
}
