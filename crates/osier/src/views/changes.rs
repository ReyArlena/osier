use crate::checkbox::Checkbox;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Textarea;
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::separator::Separator;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, StyledExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use osier_git::staging::{Check, file_check};
use osier_git::status::StatusEntry;

use crate::a11y::{A11yName as _, A11yRow as _};
use crate::app::Osier;
use crate::app::row_actions;
use crate::project::{Area, Selection};
use crate::test_ids::TestId as _;
use osier_git::ops::integrate::ConflictSide;

fn status_tag(entry: &StatusEntry, area: Area) -> Tag {
    let code = match area {
        // What the file is against HEAD: the index's change when there is one (a new file
        // stays "A" while it is edited further), else the working tree's.
        Area::Changed if entry.index != '.' => entry.index,
        Area::Changed => entry.worktree,
        Area::Untracked => '?',
        Area::Conflicted => 'U',
    };
    match code {
        'A' => Tag::success().xsmall().child("A"),
        '?' => Tag::success().xsmall().child("?"),
        'D' => Tag::danger().xsmall().child("D"),
        'R' => Tag::info().xsmall().child("R"),
        'C' => Tag::info().xsmall().child("C"),
        'U' => Tag::danger().xsmall().child("U"),
        'T' => Tag::warning().xsmall().child("T"),
        _ => Tag::warning().xsmall().child("M"),
    }
}

/// Height of every row in the Changes list (headings and changes alike).
const CHANGE_ROW_H: f32 = 28.;

/// A row of the Changes list. `key` names the group in test ids (`conflicts`, `changes`,
/// `unversioned`).
#[derive(Clone, Copy)]
enum ChangeRow {
    Heading {
        area: Area,
        title: &'static str,
        key: &'static str,
        count: usize,
    },
    /// The `ix`th change of `area`.
    Entry {
        area: Area,
        key: &'static str,
        ix: usize,
    },
}

/// Position of an area in the list's order.
fn area_slot(area: Area) -> usize {
    match area {
        Area::Conflicted => 0,
        Area::Changed => 1,
        Area::Untracked => 2,
    }
}

/// Above this many, ticking "Unversioned Files" asks first: a forgotten build folder can hold
/// thousands of files.
pub(crate) const ADD_ALL_CONFIRM: usize = 100;

/// The tri-state of a group's checkbox from its files' checks.
fn group_check(checks: impl Iterator<Item = Option<Check>>) -> Check {
    let (mut staged, mut unstaged) = (false, false);
    for c in checks {
        match c {
            Some(Check::Staged) => staged = true,
            Some(Check::Partial) => return Check::Partial,
            _ => unstaged = true,
        }
    }
    match (staged, unstaged) {
        (true, false) => Check::Staged,
        (true, true) => Check::Partial,
        _ => Check::Unstaged,
    }
}

impl Osier {
    /// The paths of one area of the Changes list, read when "Stage all" or "Unstage all" is
    /// clicked rather than copied on every frame.
    pub(crate) fn area_paths(&self, area: Area) -> Vec<String> {
        let Some(status) = self
            .active_project()
            .and_then(|p| p.snapshot.as_ref())
            .map(|s| s.status.clone())
        else {
            return Vec::new();
        };
        let entries: Vec<&StatusEntry> = match area {
            Area::Conflicted => status.conflicted().collect(),
            Area::Changed => status.changed().collect(),
            Area::Untracked => status.untracked().collect(),
        };
        entries.into_iter().map(|e| e.path.clone()).collect()
    }

    pub(crate) fn render_changes(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(project) = self.active_project() else {
            let hint = match crate::keymap::hint("open_repository", cx) {
                Some(k) => format!("Open a repository from the sidebar or press {k}."),
                None => "Open a repository from the sidebar.".to_string(),
            };
            return self
                .empty_state("No repository open", &hint, cx)
                .into_any_element();
        };
        let Some(snapshot) = &project.snapshot else {
            let msg = project
                .error
                .clone()
                .unwrap_or_else(|| "Reading repository…".into());
            return self.empty_state(&project.name, &msg, cx).into_any_element();
        };
        let status = &snapshot.status;
        let conflicted: Vec<&StatusEntry> = status.conflicted().collect();
        let changed: Vec<&StatusEntry> = status.changed().collect();
        let untracked: Vec<&StatusEntry> = status.untracked().collect();
        let staged_n = status.staged().count();
        let total = conflicted.len() + changed.len() + untracked.len();
        let muted = cx.theme().muted_foreground;

        // One row per group heading and per change, in a virtualised list: only the rows
        // on screen are built each frame, so a big untracked folder or a mass rename stays
        // cheap. All rows share one height (`uniform_list` measures the first one).
        let mut rows: Vec<ChangeRow> = Vec::new();
        for (area, title, key, count) in [
            (Area::Conflicted, "Conflicts", "conflicts", conflicted.len()),
            (Area::Changed, "Changes", "changes", changed.len()),
            (
                Area::Untracked,
                "Unversioned Files",
                "unversioned",
                untracked.len(),
            ),
        ] {
            if count > 0 {
                rows.push(ChangeRow::Heading {
                    area,
                    title,
                    key,
                    count,
                });
                rows.extend((0..count).map(|ix| ChangeRow::Entry { area, key, ix }));
            }
        }
        let rows = std::rc::Rc::new(rows);
        let list = if total == 0 {
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .child(
                    v_flex()
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .gap_1()
                        .p_6()
                        .child(div().font_medium().child("Working tree clean"))
                        .child(
                            div()
                                .text_sm()
                                .text_color(muted)
                                .child("Nothing to commit."),
                        ),
                )
                .into_any_element()
        } else {
            let status = snapshot.status.clone();
            let row_count = rows.len();
            div()
                .id("changes-list")
                .flex_1()
                .min_h_0()
                .child(
                    uniform_list(
                        "changes-rows",
                        row_count,
                        cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                            let selection = this
                                .active_project()
                                .map(|p| p.selection.clone())
                                .unwrap_or_default();
                            // The entries of each area, in display order (cheap: no copies).
                            let lists: [Vec<&StatusEntry>; 3] = [
                                status.conflicted().collect(),
                                status.changed().collect(),
                                status.untracked().collect(),
                            ];
                            range
                                .map(|row_ix| match rows[row_ix] {
                                    ChangeRow::Heading {
                                        area,
                                        title,
                                        key,
                                        count,
                                    } => {
                                        let check = match area {
                                            Area::Changed => group_check(
                                                lists[1].iter().map(|e| file_check(e)),
                                            ),
                                            _ => Check::Unstaged,
                                        };
                                        let files = if count == 1 {
                                            "1 file".to_string()
                                        } else {
                                            format!("{count} files")
                                        };
                h_flex()
                    .w_full()
                    .h(px(CHANGE_ROW_H))
                    .px_3()
                    .py_1()
                    .gap_2()
                    .items_center()
                    .text_xs()
                    .font_medium()
                    .text_color(muted)
                    .when(area != Area::Conflicted, |this| {
                        this.child(
                            Checkbox::new(SharedString::from(format!("check-group-{key}")))
                                .test_id(|| format!("change-check-group:{key}"))
                                .a11y_name(|| match area {
                                    Area::Untracked => "Add all unversioned files".to_string(),
                                    _ if check == Check::Staged => "Unstage all changes".to_string(),
                                    _ => "Stage all changes".to_string(),
                                })
                                .small()
                                .checked(check == Check::Staged)
                                .indeterminate(check == Check::Partial)
                                .on_click(cx.listener(move |this, checked: &bool, window, cx| {
                                    let paths = this.area_paths(area);
                                    if !*checked {
                                        this.unstage_paths(paths, window, cx);
                                    } else if area == Area::Untracked
                                        && paths.len() > ADD_ALL_CONFIRM
                                    {
                                        let n = paths.len();
                                        this.confirm(
                                            "Add all unversioned files?",
                                            format!(
                                                "{n} unversioned files will be added to git. \
                                                 Check that no build output or other generated \
                                                 files are among them."
                                            ),
                                            "Add all",
                                            false,
                                            move |this, window, cx| {
                                                this.stage_paths(paths.clone(), window, cx)
                                            },
                                            window,
                                            cx,
                                        );
                                    } else {
                                        this.stage_paths(paths, window, cx);
                                    }
                                })),
                        )
                    })
                    .child(title)
                    .child(div().font_normal().child(files))
                                        .into_any_element()
                                    }
                                    ChangeRow::Entry { area, key, ix } => {
                                        let entry = lists[area_slot(area)][ix];
                                        let has_markers = this
                                            .active_project()
                                            .and_then(|p| p.snapshot.as_ref())
                                            .is_some_and(|s| s.conflict_markers.contains(&entry.path));
                let selected = matches!(&selection, Selection::File { path, area: a } if path == &entry.path && *a == area);
                let check = match area {
                    Area::Changed => file_check(entry).unwrap_or(Check::Unstaged),
                    _ => Check::Unstaged,
                };
                let partial = (check == Check::Partial)
                    .then(|| this.active_project()
                        .and_then(|p| p.snapshot.as_ref())
                        .and_then(|s| s.partial_changes.get(&entry.path).copied()))
                    .flatten();
                let path = entry.path.clone();
                let display = match &entry.orig_path {
                    Some(old) => format!("{old} → {}", entry.path),
                    None => entry.path.clone(),
                };
                let click_path = path.clone();
                let toggle_path = path.clone();
                let discard_path = path.clone();
                    h_flex()
                        .id((key, ix))
                        .w_full()
                    .h(px(CHANGE_ROW_H))
                        .test_id(|| format!("change-row:{key}:{}", entry.path))
                        .a11y_row(
                            || {
                                let state = match (area, check) {
                                    (Area::Conflicted, _) => "conflicted",
                                    (Area::Untracked, _) => "unversioned",
                                    (_, Check::Staged) => "staged",
                                    (_, Check::Partial) => "partly staged",
                                    (_, Check::Unstaged) => "not staged",
                                };
                                format!("{display}, {state}")
                            },
                            selected,
                        )
                        .group("row")
                        .px_3()
                        .py_1()
                        .gap_2()
                        .items_center()
                        .cursor_pointer()
                        .when(selected, |this| this.bg(cx.theme().list_active))
                        .hover(|this| this.bg(cx.theme().list_hover))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.select_file(click_path.clone(), area, cx)
                        }))
                        .context_menu({
                            let weak = cx.entity().downgrade();
                            let p = path.clone();
                            move |menu, _, _| {
                                let (w9, w10) = (weak.clone(), weak.clone());
                                let (p9, p10) = (p.clone(), p.clone());
                                let (w1, w2, w3) = (weak.clone(), weak.clone(), weak.clone());
                                let (p1, p2, p3) = (p.clone(), p.clone(), p.clone());
                                let (w4, w5, w6) = (weak.clone(), weak.clone(), weak.clone());
                                let (p4, p5, p6) = (p.clone(), p.clone(), p.clone());
                                let (w7, w8) = (weak.clone(), weak.clone());
                                let (p7, p8) = (p.clone(), p.clone());
                                let menu = menu
                                    .item(
                                        row_actions::menu_item(
                                            "Open in editor",
                                            row_actions::RowKind::OpenInEditor,
                                        )
                                        .on_click(
                                            move |_, window, cx| {
                                                let _ = w1.update(cx, |this, cx| {
                                                    this.open_in_editor(
                                                        Some((p1.clone(), None)),
                                                        window,
                                                        cx,
                                                    )
                                                });
                                            },
                                        ),
                                    )
                                    .item(
                                        row_actions::menu_item(
                                            "Open in file manager",
                                            row_actions::RowKind::OpenInFileManager,
                                        )
                                        .on_click(
                                            move |_, window, cx| {
                                                let _ = w2.update(cx, |this, cx| {
                                                    this.reveal_in_file_manager(
                                                        Some(p2.clone()),
                                                        window,
                                                        cx,
                                                    )
                                                });
                                            },
                                        ),
                                    )
                                    .item(
                                        row_actions::menu_item(
                                            "Copy path",
                                            row_actions::RowKind::CopyPath,
                                        )
                                        .on_click(
                                            move |_, window, cx| {
                                                let _ = w3.update(cx, |this, cx| {
                                                    this.copy_to_clipboard(p3.clone(), window, cx)
                                                });
                                            },
                                        ),
                                    )
                                    .separator()
                                    .item(
                                        row_actions::menu_item(
                                            "Blame",
                                            row_actions::RowKind::Blame,
                                        )
                                        .on_click(
                                            move |_, _, cx| {
                                                let _ = w7.update(cx, |this, cx| {
                                                    this.show_blame(p7.clone(), None, cx)
                                                });
                                            },
                                        ),
                                    )
                                    .item(
                                        row_actions::menu_item(
                                            "History",
                                            row_actions::RowKind::FileHistory,
                                        )
                                        .on_click(
                                            move |_, _, cx| {
                                                let _ = w8.update(cx, |this, cx| {
                                                    this.show_file_history(p8.clone(), cx)
                                                });
                                            },
                                        ),
                                    )
                                    .separator()
                                    .item(
                                        row_actions::menu_item(
                                            "Copy as patch",
                                            row_actions::RowKind::CopyAsPatch,
                                        )
                                        .on_click(
                                            move |_, window, cx| {
                                                let _ = w4.update(cx, |this, cx| {
                                                    let s = Osier::file_source(&p4, area);
                                                    this.copy_patch(s, window, cx)
                                                });
                                            },
                                        ),
                                    )
                                    .item(
                                        row_actions::menu_item(
                                            "Save to Patches…",
                                            row_actions::RowKind::SaveToPatches,
                                        )
                                        .on_click(
                                            move |_, window, cx| {
                                                let _ = w5.update(cx, |this, cx| {
                                                    let s = Osier::file_source(&p5, area);
                                                    this.save_patch_prompt(
                                                        s,
                                                        p5.clone(),
                                                        window,
                                                        cx,
                                                    )
                                                });
                                            },
                                        ),
                                    )
                                    .item(
                                        row_actions::menu_item(
                                            "Shelve…",
                                            row_actions::RowKind::Shelve,
                                        )
                                        .on_click(
                                            move |_, window, cx| {
                                                let _ = w6.update(cx, |this, cx| {
                                                    this.shelve_prompt(
                                                        vec![(p6.clone(), area)],
                                                        window,
                                                        cx,
                                                    )
                                                });
                                            },
                                        ),
                                    );
                                if area == Area::Conflicted {
                                    return menu;
                                }
                                let row = crate::app::ignore_ops::RowState {
                                    // Untracked folders are listed whole, as `dir/`.
                                    is_dir: p9.ends_with('/'),
                                    tracked: area != Area::Untracked,
                                };
                                let path = p9.trim_end_matches('/').to_string();
                                // Same as the row's hover button.
                                let discard = if area == Area::Untracked {
                                    "Move to trash…"
                                } else {
                                    "Roll back changes…"
                                };
                                Osier::ignore_menu(menu.separator(), w9, path, row)
                                    .separator()
                                    .item(
                                        row_actions::action_menu_item(discard, "discard").on_click(
                                            move |_, window, cx| {
                                                let _ = w10.update(cx, |this, cx| {
                                                    this.discard_file(p10.clone(), area, window, cx)
                                                });
                                            },
                                        ),
                                    )
                            }
                        })
                        .child(
                            Checkbox::new(SharedString::from(format!("stage-{key}-{ix}")))
                                .test_id(|| format!("change-stage:{key}:{}", entry.path))
                                .a11y_name(|| {
                                    if check == Check::Staged {
                                        format!("Unstage {}", entry.path)
                                    } else {
                                        format!("Stage {}", entry.path)
                                    }
                                })
                                .small()
                                .checked(check == Check::Staged)
                                .indeterminate(check == Check::Partial)
                                .disabled(area == Area::Conflicted)
                                .on_click(cx.listener(move |this, checked: &bool, window, cx| {
                                    // Staging leaves the selection alone: the row's own click
                                    // would select it first.
                                    cx.stop_propagation();
                                    let paths = vec![toggle_path.clone()];
                                    if *checked {
                                        this.stage_paths(paths, window, cx);
                                    } else {
                                        this.unstage_paths(paths, window, cx);
                                    }
                                })),
                        )
                        .child(status_tag(entry, area))
                        .child(div().text_sm().flex_1().min_w_0().truncate().child(display))
                        .when_some(partial, |this, (some, total)| {
                            this.child(
                                div()
                                    .flex_none()
                                    .text_xs()
                                    .text_color(muted)
                                    .test_id(|| format!("change-partial:{}", entry.path))
                                    .child(format!("{some} of {total} changes")),
                            )
                        })
                        .when(area == Area::Conflicted, |this| {
                            let (p1, p2, p3) = (path.clone(), path.clone(), path.clone());

                            this.child(
                                h_flex()
                                    .gap_1()
                                    .child(
                                        Button::new(SharedString::from(format!("ours-{ix}")))
                                            .ghost()
                                            .xsmall()
                                            .label("Ours")
                                            .tooltip("Take the current branch's version")
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.resolve_conflict_with(
                                                    p1.clone(),
                                                    ConflictSide::Ours,
                                                    window,
                                                    cx,
                                                )
                                            })),
                                    )
                                    .child(
                                        Button::new(SharedString::from(format!("theirs-{ix}")))
                                            .ghost()
                                            .xsmall()
                                            .label("Theirs")
                                            .tooltip("Take the incoming version")
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.resolve_conflict_with(
                                                    p2.clone(),
                                                    ConflictSide::Theirs,
                                                    window,
                                                    cx,
                                                )
                                            })),
                                    )
                                    .child(
                                        Button::new(SharedString::from(format!("resolved-{ix}")))
                                            .test_id(|| format!("mark-resolved:{}", entry.path))
                                            .outline()
                                            .xsmall()
                                            .label("Mark resolved")
                                            .disabled(has_markers)
                                            .tooltip(if has_markers {
                                                "Still has conflict markers: resolve them first, \
                                                 or use Apply resolution"
                                            } else {
                                                "Use after fixing the file in another editor"
                                            })
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.mark_resolved(p3.clone(), window, cx)
                                            })),
                                    ),
                            )
                        })
                        .when(area != Area::Conflicted, |this| {
                            this.child(
                                div()
                                    .invisible()
                                    .group_hover("row", |this| this.visible())
                                    .child(
                                        Button::new(SharedString::from(format!(
                                            "discard-{key}-{ix}"
                                        )))
                                        .ghost()
                                        .xsmall()
                                        .icon(if area == Area::Untracked {
                                            Lucide::Trash
                                        } else {
                                            Lucide::Undo2
                                        })
                                        .tooltip(if area == Area::Untracked {
                                            "Move to trash"
                                        } else {
                                            "Roll back changes"
                                        })
                                        .on_click(
                                            cx.listener(move |this, _, window, cx| {
                                                this.discard_file(
                                                    discard_path.clone(),
                                                    area,
                                                    window,
                                                    cx,
                                                )
                                            }),
                                        ),
                                    ),
                            )
                        })
                                        .into_any_element()
                                    }
                                })
                                .collect()
                        }),
                    )
                    .track_scroll(&self.changes_scroll)
                    .size_full(),
                )
                .vertical_scrollbar(&self.changes_scroll)
                .into_any_element()
        };

        let identity = match (&snapshot.identity.name, &snapshot.identity.email) {
            (Some(n), Some(e)) => format!("{n} <{e}>"),
            (Some(n), None) => n.clone(),
            (None, Some(e)) => e.clone(),
            (None, None) => "No git identity configured".into(),
        };
        let can_commit = self.can_commit(cx);
        let message = self.commit_message_text(cx);
        let subject_len = message
            .lines()
            .next()
            .map(|l| l.chars().count())
            .unwrap_or(0);
        let commit_label = if self.amend {
            "Amend commit".to_string()
        } else if staged_n == 1 {
            "Commit 1 file".to_string()
        } else {
            format!("Commit {staged_n} files")
        };

        v_flex()
            .size_full()
            .child(
                h_flex()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(div().text_sm().font_medium().flex_none().child("Changes"))
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(format!(
                                "{} changed, {staged_n} staged, {} unversioned",
                                changed.len(),
                                untracked.len()
                            )),
                    )
                    .child(
                        Button::new("stage-all")
                            .test_id("stage-all")
                            .ghost()
                            .xsmall()
                            .icon(Lucide::ListChecks)
                            .tooltip("Stage all")
                            .disabled(
                                changed.iter().all(|e| !e.is_unstaged()) && untracked.is_empty(),
                            )
                            .on_click(
                                cx.listener(|this, _, window, cx| this.stage_all(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("unstage-all")
                            .test_id("unstage-all")
                            .ghost()
                            .xsmall()
                            .icon(Lucide::ListX)
                            .tooltip("Unstage all")
                            .disabled(staged_n == 0)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.unstage_all(window, cx)),
                            ),
                    ),
            )
            .child(list)
            .child(Separator::horizontal())
            .child(
                v_flex()
                    .p_3()
                    .gap_2()
                    .child(
                        div()
                            .test_id("commit-message")
                            .child(Textarea::new(&self.commit_message)),
                    )
                    .child(
                        h_flex()
                            .gap_3()
                            .items_center()
                            .text_xs()
                            .text_color(muted)
                            .child(div().truncate().flex_1().min_w_0().child(identity))
                            .child({
                                let messages = self.recent_commit_messages();
                                let weak = cx.weak_entity();
                                Button::new("message-history")
                                    .test_id("message-history")
                                    .ghost()
                                    .xsmall()
                                    .icon(Lucide::Clock)
                                    .tooltip("Recent commit messages")
                                    .disabled(messages.is_empty())
                                    .dropdown_menu(move |mut menu, _, _| {
                                        for msg in &messages {
                                            let first = msg.lines().next().unwrap_or("").trim();
                                            let label: String = if first.chars().count() > 60 {
                                                first.chars().take(59).chain(['…']).collect()
                                            } else {
                                                first.to_string()
                                            };
                                            let (w, m) = (weak.clone(), msg.clone());
                                            menu = menu.item(PopupMenuItem::new(label).on_click(
                                                move |_, window, cx| {
                                                    let _ = w.update(cx, |this, cx| {
                                                        this.set_message(&m, window, cx)
                                                    });
                                                },
                                            ));
                                        }
                                        menu
                                    })
                            })
                            .when(subject_len > 0, |this| {
                                this.child(
                                    div()
                                        .flex_none()
                                        .when(subject_len > 72, |d| d.text_color(cx.theme().danger))
                                        .when(subject_len > 50 && subject_len <= 72, |d| {
                                            d.text_color(cx.theme().warning)
                                        })
                                        .child(format!("{subject_len}/72")),
                                )
                            }),
                    )
                    .child(
                        h_flex()
                            .gap_3()
                            .gap_y_2()
                            .items_center()
                            .flex_wrap()
                            .child(
                                Checkbox::new("amend")
                                    .test_id("amend")
                                    .small()
                                    .label("Amend")
                                    .checked(self.amend)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.toggle_amend(window, cx)
                                    })),
                            )
                            .child(
                                Checkbox::new("signoff")
                                    .small()
                                    .label("Sign-off")
                                    .checked(self.signoff)
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.toggle_signoff(cx)),
                                    ),
                            )
                            // Both buttons as one group, pushed to the right: when the
                            // pane is too narrow they drop to the next line together.
                            .child(
                                h_flex()
                                    .ml_auto()
                                    .gap_3()
                                    .gap_y_2()
                                    .items_center()
                                    .justify_end()
                                    .flex_wrap()
                                    .child(
                                        Button::new("commit-push")
                                            .test_id("commit-push")
                                            .outline()
                                            .small()
                                            .label("Commit and Push")
                                            .disabled(!can_commit || self.remote_job.is_some())
                                            .tooltip("Commit, then push the current branch")
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.commit_and_push(window, cx)
                                            })),
                                    )
                                    .child(
                                        Button::new("commit")
                                            .test_id("commit")
                                            .primary()
                                            .small()
                                            .icon(Lucide::GitCommitHorizontal)
                                            .label(commit_label)
                                            .disabled(!can_commit)
                                            .tooltip("Commit (ctrl-enter)")
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.commit(window, cx)
                                            })),
                                    ),
                            ),
                    ),
            )
            .into_any_element()
    }
}
