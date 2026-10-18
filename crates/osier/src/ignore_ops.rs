//! "Add to .gitignore" and "Untrack": menu entries shared by the Files and Changes tabs, the
//! confirmation dialog for tracked paths, and the notifications.
//!
//! Ignore rules do not apply to tracked files, so ignoring one offers to stop tracking it
//! (`git rm --cached`, on by default) and untracking offers to ignore it (on by default), so
//! the file is not added back by the next `git add .`.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::button::ButtonVariant;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::DialogButtonProps;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use osier_git::ops::ignore::{self, IgnoreFile};

use crate::app::Osier;
use crate::test_ids::TestId as _;

/// Which pattern to add.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PatternKind {
    /// Exactly this file or folder (`/path`, `/dir/`).
    Exact,
    /// Every file with this extension (`*.log`).
    Extension,
}

/// What the caller already knows about a row, to pick the menu entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RowState {
    pub is_dir: bool,
    /// The path is (or, for a folder, contains) a tracked file.
    pub tracked: bool,
}

impl Osier {
    /// Append "Add to .gitignore", "Add all *.ext files to .gitignore", "Ignore only on this
    /// machine" and, for tracked paths, "Untrack".
    pub(crate) fn ignore_menu(
        menu: PopupMenu,
        weak: WeakEntity<Osier>,
        path: String,
        row: RowState,
    ) -> PopupMenu {
        let what = if row.is_dir { "folder" } else { "file" };
        let ext = (!row.is_dir)
            .then(|| ignore::extension_pattern(&path))
            .flatten();
        let pick = |kind: PatternKind, file: IgnoreFile| {
            let weak = weak.clone();
            let path = path.clone();
            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                let _ = weak.update(cx, |this, cx| {
                    this.ignore_path(path.clone(), row, kind, file, window, cx)
                });
            }
        };
        let mut menu = menu.item(
            PopupMenuItem::new("Add to .gitignore")
                .on_click(pick(PatternKind::Exact, IgnoreFile::Gitignore)),
        );
        if let Some(pattern) = &ext {
            menu = menu.item(
                PopupMenuItem::new(format!("Add all {pattern} files to .gitignore"))
                    .on_click(pick(PatternKind::Extension, IgnoreFile::Gitignore)),
            );
        }
        menu = menu.item(
            PopupMenuItem::new("Ignore only on this machine")
                .on_click(pick(PatternKind::Exact, IgnoreFile::LocalExclude)),
        );
        if !row.tracked {
            return menu;
        }
        menu.item(PopupMenuItem::new(format!("Untrack this {what}")).on_click(
            move |_, window, cx| {
                let _ = weak.update(cx, |this, cx| {
                    this.untrack_prompt(path.clone(), row, window, cx)
                });
            },
        ))
    }

    /// Ignore `path`. Untracked: add the pattern, or say it is already ignored. Tracked: ask
    /// whether to stop tracking it as well.
    pub(crate) fn ignore_path(
        &mut self,
        path: String,
        row: RowState,
        kind: PatternKind,
        file: IgnoreFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(repo) = self.active_project().map(|p| p.repo.clone()) else {
            return;
        };
        let pattern = match kind {
            PatternKind::Exact => ignore::exact_pattern(&path, row.is_dir),
            PatternKind::Extension => match ignore::extension_pattern(&path) {
                Some(p) => p,
                None => return,
            },
        };
        let check = cx.background_spawn({
            let path = path.clone();
            async move {
                let tracked = ignore::is_tracked(&repo, &path)?;
                let ignored = ignore::is_ignored(&repo, &path)?;
                osier_git::Result::Ok((tracked, ignored))
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = check.await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Err(e) => window.push_notification(
                    Notification::error("Could not check the path").message(e.to_string()),
                    cx,
                ),
                // Already ignored, and git is not tracking it: nothing to do.
                Ok((false, true)) => window.push_notification(
                    Notification::info(format!("{path} is already ignored")),
                    cx,
                ),
                Ok((false, false)) => {
                    let summary = format!("Ignored {pattern} in {}", file.label());
                    this.run_op(
                        summary,
                        move |repo| ignore::add_pattern(repo, file, &pattern).map(|_| ()),
                        window,
                        cx,
                    );
                }
                Ok((true, ignored)) => {
                    let step = (!ignored).then_some((file, pattern));
                    this.tracked_dialog(path, row, Mode::Ignore(step), window, cx);
                }
            });
        })
        .detach();
    }

    /// "Untrack this file": confirm, offering to ignore it too.
    pub(crate) fn untrack_prompt(
        &mut self,
        path: String,
        row: RowState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(repo) = self.active_project().map(|p| p.repo.clone()) else {
            return;
        };
        let check = cx.background_spawn({
            let path = path.clone();
            async move { ignore::is_ignored(&repo, &path) }
        });
        cx.spawn_in(window, async move |this, cx| {
            let ignored = check.await.unwrap_or(false);
            let _ = this.update_in(cx, |this, window, cx| {
                let step = (!ignored).then(|| {
                    (
                        IgnoreFile::Gitignore,
                        ignore::exact_pattern(&path, row.is_dir),
                    )
                });
                this.tracked_dialog(path, row, Mode::Untrack(step), window, cx);
            });
        })
        .detach();
    }

    /// The dialog for a tracked path, from either menu entry. The checkbox covers the step the
    /// user did not pick directly; it is on by default.
    fn tracked_dialog(
        &mut self,
        path: String,
        row: RowState,
        mode: Mode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let what = if row.is_dir { "folder" } else { "file" };
        let warning = format!(
            "Stopping tracking stages the {what}'s removal from the repository. The {what} stays \
             on your disk, but when the commit is pulled into another clone, git deletes it \
             there."
        );
        // What the checkbox runs, shown as a command. Folders untrack every file inside.
        let rm_command = if row.is_dir {
            format!("git rm --cached -r {path}")
        } else {
            format!("git rm --cached {path}")
        };
        let (title, body, check_label, ok_label, extra) = match &mode {
            Mode::Ignore(Some((file, pattern))) => (
                format!("Add {path} to {}", file.label()),
                format!(
                    "You've chosen to add {path} to {} but it is a tracked {what}. Ignore rules \
                     do not apply to tracked files. Do you also want to stop tracking the {what} \
                     in this repository? (The {what} will not be deleted.)",
                    file.label()
                ),
                Some(("Run", rm_command.clone(), "")),
                format!("Add to {}", file.label()),
                Some(pattern.clone()),
            ),
            Mode::Ignore(None) => (
                format!("{path} is already ignored"),
                format!(
                    "{path} matches an ignore rule but git is still tracking it, so its changes \
                     keep showing. Stop tracking it? (The {what} will not be deleted.)"
                ),
                None,
                "Stop tracking".to_string(),
                None,
            ),
            Mode::Untrack(step) => (
                format!("Untrack {path}"),
                format!("Git stops tracking {path}. The {what} will not be deleted."),
                step.as_ref().map(|(_, p)| {
                    (
                        "Also add",
                        p.clone(),
                        "to .gitignore so it is not added again",
                    )
                }),
                "Untrack".to_string(),
                step.as_ref().map(|(_, p)| p.clone()),
            ),
        };
        let checked = Rc::new(Cell::new(true));
        let weak = cx.entity().downgrade();
        let file = match &mode {
            Mode::Ignore(Some((file, _))) => *file,
            _ => IgnoreFile::Gitignore,
        };
        let is_untrack = matches!(mode, Mode::Untrack(_));
        window.open_dialog(cx, move |dialog, _, cx| {
            let c1 = checked.clone();
            let c2 = checked.clone();
            let weak = weak.clone();
            let path = path.clone();
            let extra = extra.clone();
            dialog
                .title(title.clone())
                .child(
                    v_flex()
                        .test_id("tracked-dialog")
                        .gap_3()
                        .child(body.clone())
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(warning.clone()),
                        )
                        .when_some(check_label.clone(), |this, (before, code, after)| {
                            // The checkbox plus its words, with the command or pattern as
                            // inline code. Clicking the words toggles it too.
                            let c3 = c1.clone();
                            this.child(
                                h_flex()
                                    .id("tracked-dialog-option")
                                    .test_id("tracked-dialog-option")
                                    .gap_2()
                                    .items_center()
                                    .flex_wrap()
                                    .cursor_pointer()
                                    .on_click(move |_, window, _| {
                                        c3.set(!c3.get());
                                        window.refresh();
                                    })
                                    .child(
                                        Checkbox::new("tracked-dialog-check")
                                            .test_id("tracked-dialog-check")
                                            .small()
                                            .checked(c1.get())
                                            .on_click(move |v, window, cx| {
                                                c1.set(*v);
                                                cx.stop_propagation();
                                                window.refresh();
                                            }),
                                    )
                                    .child(before)
                                    .child(crate::inline_code::inline_code(code, cx))
                                    .when(!after.is_empty(), |this| this.child(after)),
                            )
                        }),
                )
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
                    let checked = c2.get();
                    let path = path.clone();
                    let extra = extra.clone();
                    let _ = weak.update(cx, |this, cx| {
                        // Which steps run: the one picked from the menu, plus the checkbox's.
                        let (add, untrack) = match (is_untrack, &extra) {
                            (true, Some(p)) => (checked.then(|| p.clone()), true),
                            (true, None) => (None, true),
                            (false, Some(p)) => (Some(p.clone()), checked),
                            (false, None) => (None, true),
                        };
                        this.run_ignore_steps(path, file, add, untrack, window, cx);
                    });
                    true
                })
        });
    }

    fn run_ignore_steps(
        &mut self,
        path: String,
        file: IgnoreFile,
        pattern: Option<String>,
        untrack: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let summary = match (&pattern, untrack) {
            (Some(p), true) => format!("Ignored {p} and stopped tracking {path}"),
            (Some(p), false) => format!("Ignored {p} in {} (still tracked)", file.label()),
            (None, true) => format!("Stopped tracking {path}"),
            (None, false) => return,
        };
        self.run_op(
            summary,
            move |repo| {
                if let Some(p) = &pattern {
                    ignore::add_pattern(repo, file, p)?;
                }
                if untrack {
                    ignore::untrack(repo, &path)?;
                }
                Ok(())
            },
            window,
            cx,
        );
    }
}

/// What the tracked-path dialog is for. The option is the ignore step still to do (pattern
/// and file), `None` when an ignore rule already matches.
enum Mode {
    Ignore(Option<(IgnoreFile, String)>),
    Untrack(Option<(IgnoreFile, String)>),
}
