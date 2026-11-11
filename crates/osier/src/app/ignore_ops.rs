//! "Add to .gitignore" and "Untrack": menu entries shared by the Files and Changes tabs, the
//! confirmation dialog for tracked paths, and the notifications.
//!
//! Ignore rules do not apply to tracked files, so ignoring one offers to stop tracking it
//! (`git rm --cached`, on by default) and untracking offers to ignore it (on by default), so
//! the file is not added back by the next `git add .`.

use crate::app_dialog::AppDialogExt as _;
use std::cell::Cell;
use std::rc::Rc;

use crate::checkbox::Checkbox;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::button::ButtonVariant;
use gpui_kit::component::dialog::DialogButtonProps;
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use osier_git::ops::ignore::{self, IgnoreFile};

use crate::a11y::A11yName as _;
use crate::app::Osier;
use crate::notices::{Notice, WindowNoticeExt as _};
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
            crate::app::row_actions::menu_item(
                "Add to .gitignore",
                crate::app::row_actions::RowKind::AddToGitignore,
            )
            .on_click(pick(PatternKind::Exact, IgnoreFile::Gitignore)),
        );
        if let Some(pattern) = &ext {
            menu = menu.item(
                crate::app::row_actions::menu_item(
                    format!("Add all {pattern} files to .gitignore"),
                    crate::app::row_actions::RowKind::AddExtensionToGitignore,
                )
                .on_click(pick(PatternKind::Extension, IgnoreFile::Gitignore)),
            );
        }
        menu = menu.item(
            crate::app::row_actions::menu_item(
                "Ignore only on this machine",
                crate::app::row_actions::RowKind::IgnoreLocally,
            )
            .on_click(pick(PatternKind::Exact, IgnoreFile::LocalExclude)),
        );
        if !row.tracked {
            return menu;
        }
        menu.item(
            crate::app::row_actions::menu_item(
                format!("Untrack this {what}"),
                crate::app::row_actions::RowKind::Untrack,
            )
            .on_click(move |_, window, cx| {
                let _ = weak.update(cx, |this, cx| {
                    this.untrack_prompt(path.clone(), row, window, cx)
                });
            }),
        )
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
                Err(e) => window.push_notice(
                    Notice::error(e.to_string()).title("Could not check the path"),
                    cx,
                ),
                // Already ignored, and git is not tracking it: nothing to do.
                Ok((false, true)) => {
                    window.push_notice(Notice::info(format!("{path} is already ignored")), cx)
                }
                Ok((false, false)) => {
                    let summary = format!("Ignored {pattern} in {}", file.label());
                    this.run_op_reporting(
                        summary,
                        move |repo| {
                            let added = ignore::add_pattern(repo, file, &pattern, &path)?;
                            Ok(format!("Ignored {}", where_added(&added, file)))
                        },
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
            "Stopping tracking leaves the {what} on your disk and stages its removal from the \
             repository for your next commit. Once that commit is pushed, anyone who pulls it \
             will most likely have the {what} deleted from their disk. Git only keeps their copy \
             when they have uncommitted edits to it, and then stops their pull until they deal \
             with those. If they pull with Osier, it offers to put their copy back."
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
                     in this repository?",
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
                     keep showing. Stop tracking it?"
                ),
                None,
                "Stop tracking".to_string(),
                None,
            ),
            Mode::Untrack(step) => (
                format!("Untrack {path}"),
                format!("Git stops tracking {path}."),
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
        window.open_app_dialog(cx, move |dialog, _, cx| {
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
                                    .items_start()
                                    .pb_1()
                                    .cursor_pointer()
                                    .on_click(move |_, window, _| {
                                        c3.set(!c3.get());
                                        window.refresh();
                                    })
                                    .child(
                                        Checkbox::new("tracked-dialog-check")
                                            .test_id("tracked-dialog-check")
                                            // The sentence next to it is built word by word,
                                            // so it is not the checkbox's own label.
                                            .a11y_name(|| format!("{before} {code} {after}"))
                                            .small()
                                            .checked(c1.get())
                                            .on_click(move |v, window, cx| {
                                                c1.set(*v);
                                                cx.stop_propagation();
                                                window.refresh();
                                            }),
                                    )
                                    // One element per word, so the sentence wraps like text
                                    // and wrapped lines start under its first word, not under
                                    // the checkbox.
                                    .child(
                                        h_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .flex_wrap()
                                            .items_center()
                                            .gap_x_1()
                                            .children(
                                                before
                                                    .split_whitespace()
                                                    .map(|w| div().child(w.to_string())),
                                            )
                                            .child(crate::inline_code::inline_code(code, cx))
                                            .children(
                                                after
                                                    .split_whitespace()
                                                    .map(|w| div().child(w.to_string())),
                                            ),
                                    ),
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

    /// After a pull moved `path`'s HEAD from `before` to `after`: if it deleted files an
    /// ignore rule now matches (someone stopped tracking them), offer to write the local
    /// copies back. Settings > Git > Pull turns the offer off.
    pub(crate) fn offer_keep_ignored(
        &mut self,
        path: std::path::PathBuf,
        before: String,
        after: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.settings.git.offer_keep_ignored || before == after {
            return;
        }
        let Some(repo) = self
            .projects
            .iter()
            .find(|p| p.path == path)
            .map(|p| p.repo.clone())
        else {
            return;
        };
        let check = cx.background_spawn({
            let before = before.clone();
            async move { ignore::removed_ignored(&repo, &before, &after) }
        });
        cx.spawn_in(window, async move |this, cx| {
            let files = match check.await {
                Ok(files) if !files.is_empty() => files,
                Ok(_) => return,
                Err(e) => {
                    tracing::warn!(error = %e, "could not list files the pull removed");
                    return;
                }
            };
            let _ = this.update_in(cx, |this, window, cx| {
                // The restore runs on the active project, so only offer while it is still
                // the one that was pulled.
                if this.active_project().is_some_and(|p| p.path == path) {
                    this.keep_ignored_dialog(before, files, window, cx);
                }
            });
        })
        .detach();
    }

    fn keep_ignored_dialog(
        &mut self,
        before: String,
        files: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        const SHOWN: usize = 8;
        let count = files.len();
        let (title, body) = if count == 1 {
            (
                format!("Keep your copy of {}?", files[0]),
                "The pull stopped tracking this file, which is now ignored, so git deleted your \
                 copy. Osier can put it back as it was before the pull."
                    .to_string(),
            )
        } else {
            (
                format!("Keep your copies of {count} ignored files?"),
                format!(
                    "The pull stopped tracking these {count} files, which are now ignored, so git \
                     deleted your copies. Osier can put them back as they were before the pull."
                ),
            )
        };
        let hint = "Restored files stay untracked and ignored, so they do not show up as changes. \
                    You can turn this offer off in Settings > Git > Pull.";
        let ok_label: SharedString = if count == 1 {
            "Keep my copy".into()
        } else {
            "Keep my copies".into()
        };
        let weak = cx.entity().downgrade();
        let files = Rc::new(files);
        window.open_app_dialog(cx, move |dialog, _, cx| {
            let weak = weak.clone();
            let files = files.clone();
            let before = before.clone();
            let mut list = v_flex().gap_1();
            for f in files.iter().take(SHOWN) {
                list = list.child(h_flex().child(crate::inline_code::inline_code(f.clone(), cx)));
            }
            if count > SHOWN {
                list = list.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("and {} more", count - SHOWN)),
                );
            }
            dialog
                .title(title.clone())
                .child(
                    v_flex()
                        .test_id("keep-ignored-dialog")
                        .gap_3()
                        .pb_1()
                        .child(body.clone())
                        .child(list)
                        .child(crate::dialog_hint::hint(hint, cx)),
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
                    let files = (*files).clone();
                    let before = before.clone();
                    let _ = weak.update(cx, |this, cx| {
                        let summary = if files.len() == 1 {
                            format!("Kept your copy of {}", files[0])
                        } else {
                            format!("Kept your copies of {} ignored files", files.len())
                        };
                        this.run_op_reporting(
                            summary.clone(),
                            move |repo| {
                                ignore::restore_worktree(repo, &before, &files)?;
                                Ok(summary)
                            },
                            window,
                            cx,
                        );
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
        self.run_op_reporting(
            summary.clone(),
            move |repo| {
                let added = match &pattern {
                    Some(p) => Some(ignore::add_pattern(repo, file, p, &path)?),
                    None => None,
                };
                if untrack {
                    ignore::untrack(repo, &path)?;
                }
                Ok(match (added, untrack) {
                    (Some(a), true) => {
                        format!(
                            "Ignored {} and stopped tracking {path}",
                            where_added(&a, file)
                        )
                    }
                    (Some(a), false) => {
                        format!("Ignored {} (still tracked)", where_added(&a, file))
                    }
                    (None, _) => summary,
                })
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

/// "/x in .gitignore (line 42, next to y)" for a notification.
fn where_added(added: &ignore::Added, file: IgnoreFile) -> String {
    let mut out = format!("{} in {} (line {}", added.pattern, file.label(), added.line);
    if let Some(near) = &added.near {
        out.push_str(&format!(", next to {near}"));
    }
    if !added.changed {
        out.push_str(", already there");
    }
    out.push(')');
    out
}
