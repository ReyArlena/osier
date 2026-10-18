use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
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
use osier_git::status::StatusEntry;

use crate::app::Osier;
use crate::project::{Area, Selection};
use crate::test_ids::TestId as _;
use osier_git::ops::integrate::ConflictSide;

fn status_tag(entry: &StatusEntry, area: Area) -> Tag {
    let code = match area {
        Area::Staged => entry.index,
        Area::Unstaged => entry.worktree,
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

impl Osier {
    pub(crate) fn render_changes(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(project) = self.active_project() else {
            return self
                .empty_state(
                    "No repository open",
                    "Open a repository from the sidebar or press ctrl-o.",
                    cx,
                )
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
        let staged: Vec<&StatusEntry> = status.staged().collect();
        let unstaged: Vec<&StatusEntry> = status.unstaged().collect();
        let untracked: Vec<&StatusEntry> = status.untracked().collect();
        let total = conflicted.len() + staged.len() + unstaged.len() + untracked.len();
        let selection = project.selection.clone();
        let muted = cx.theme().muted_foreground;

        let mut list = v_flex()
            .flex_1()
            .min_h_0()
            .id("changes-list")
            .overflow_y_scrollbar();
        for (area, title, entries) in [
            (Area::Conflicted, "Conflicts", &conflicted),
            (Area::Staged, "Staged", &staged),
            (Area::Unstaged, "Unstaged", &unstaged),
            (Area::Untracked, "Untracked", &untracked),
        ] {
            if entries.is_empty() {
                continue;
            }
            let group_paths: Vec<String> = entries.iter().map(|e| e.path.clone()).collect();
            list = list.child(
                h_flex()
                    .px_3()
                    .py_1()
                    .gap_2()
                    .items_center()
                    .text_xs()
                    .font_medium()
                    .text_color(muted)
                    .child(title)
                    .child(div().child(entries.len().to_string()))
                    .child(div().flex_1())
                    .when(area == Area::Staged, |this| {
                        let paths = group_paths.clone();
                        this.child(
                            Button::new("unstage-group")
                                .test_id("unstage-group")
                                .ghost()
                                .xsmall()
                                .label("Unstage all")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.unstage_paths(paths.clone(), window, cx)
                                })),
                        )
                    })
                    .when(matches!(area, Area::Unstaged | Area::Untracked), |this| {
                        let paths = group_paths.clone();
                        this.child(
                            Button::new(SharedString::from(format!("stage-group-{title}")))
                                .ghost()
                                .xsmall()
                                .label("Stage all")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.stage_paths(paths.clone(), window, cx)
                                })),
                        )
                    }),
            );
            for (ix, entry) in entries.iter().enumerate() {
                let selected = matches!(&selection, Selection::File { path, area: a } if path == &entry.path && *a == area);
                let path = entry.path.clone();
                let display = match &entry.orig_path {
                    Some(old) => format!("{old} → {}", entry.path),
                    None => entry.path.clone(),
                };
                let click_path = path.clone();
                let toggle_path = path.clone();
                let discard_path = path.clone();
                list = list.child(
                    h_flex()
                        .id((title, ix))
                        .test_id(format!(
                            "change-row:{}:{}",
                            title.to_lowercase(),
                            entry.path
                        ))
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
                                let w9 = weak.clone();
                                let p9 = p.clone();
                                let (w1, w2, w3) = (weak.clone(), weak.clone(), weak.clone());
                                let (p1, p2, p3) = (p.clone(), p.clone(), p.clone());
                                let (w4, w5, w6) = (weak.clone(), weak.clone(), weak.clone());
                                let (p4, p5, p6) = (p.clone(), p.clone(), p.clone());
                                let (w7, w8) = (weak.clone(), weak.clone());
                                let (p7, p8) = (p.clone(), p.clone());
                                let menu = menu
                                    .item(PopupMenuItem::new("Open in editor").on_click(
                                        move |_, window, cx| {
                                            let _ = w1.update(cx, |this, cx| {
                                                this.open_in_editor(
                                                    Some((p1.clone(), None)),
                                                    window,
                                                    cx,
                                                )
                                            });
                                        },
                                    ))
                                    .item(PopupMenuItem::new("Open in file manager").on_click(
                                        move |_, window, cx| {
                                            let _ = w2.update(cx, |this, cx| {
                                                this.reveal_in_file_manager(
                                                    Some(p2.clone()),
                                                    window,
                                                    cx,
                                                )
                                            });
                                        },
                                    ))
                                    .item(PopupMenuItem::new("Copy path").on_click(
                                        move |_, window, cx| {
                                            let _ = w3.update(cx, |this, cx| {
                                                this.copy_to_clipboard(p3.clone(), window, cx)
                                            });
                                        },
                                    ))
                                    .separator()
                                    .item(PopupMenuItem::new("Blame").on_click(move |_, _, cx| {
                                        let _ = w7.update(cx, |this, cx| {
                                            this.show_blame(p7.clone(), None, cx)
                                        });
                                    }))
                                    .item(PopupMenuItem::new("History").on_click(
                                        move |_, _, cx| {
                                            let _ = w8.update(cx, |this, cx| {
                                                this.show_file_history(p8.clone(), cx)
                                            });
                                        },
                                    ))
                                    .separator()
                                    .item(PopupMenuItem::new("Copy as patch").on_click(
                                        move |_, window, cx| {
                                            let _ = w4.update(cx, |this, cx| {
                                                let s = Osier::file_source(&p4, area);
                                                this.copy_patch(s, window, cx)
                                            });
                                        },
                                    ))
                                    .item(PopupMenuItem::new("Save to Patches…").on_click(
                                        move |_, window, cx| {
                                            let _ = w5.update(cx, |this, cx| {
                                                let s = Osier::file_source(&p5, area);
                                                this.save_patch_prompt(s, p5.clone(), window, cx)
                                            });
                                        },
                                    ))
                                    .item(PopupMenuItem::new("Shelve…").on_click(
                                        move |_, window, cx| {
                                            let _ = w6.update(cx, |this, cx| {
                                                this.shelve_prompt(
                                                    vec![(p6.clone(), area)],
                                                    window,
                                                    cx,
                                                )
                                            });
                                        },
                                    ));
                                if area == Area::Conflicted {
                                    return menu;
                                }
                                let row = crate::ignore_ops::RowState {
                                    // Untracked folders are listed whole, as `dir/`.
                                    is_dir: p9.ends_with('/'),
                                    tracked: area != Area::Untracked,
                                };
                                let path = p9.trim_end_matches('/').to_string();
                                Osier::ignore_menu(menu.separator(), w9, path, row)
                            }
                        })
                        .child(
                            Checkbox::new(SharedString::from(format!("stage-{title}-{ix}")))
                                .test_id(format!(
                                    "change-stage:{}:{}",
                                    title.to_lowercase(),
                                    entry.path
                                ))
                                .small()
                                .checked(area == Area::Staged)
                                .disabled(area == Area::Conflicted)
                                .on_click(cx.listener(move |this, checked: &bool, window, cx| {
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
                                            .outline()
                                            .xsmall()
                                            .label("Resolved")
                                            .tooltip("Mark as resolved after editing the file")
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
                                            "discard-{title}-{ix}"
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
                                            "Discard changes"
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
                        }),
                );
            }
        }
        if total == 0 {
            list = list.child(
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
            );
        }

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
        let staged_n = staged.len();
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
                                "{} staged, {} unstaged, {} untracked",
                                staged.len(),
                                unstaged.len(),
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
                            .disabled(unstaged.is_empty() && untracked.is_empty())
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
                            .disabled(staged.is_empty())
                            .on_click(
                                cx.listener(|this, _, window, cx| this.unstage_all(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("refresh-changes")
                            .test_id("refresh-changes")
                            .ghost()
                            .xsmall()
                            .icon(Lucide::RefreshCw)
                            .tooltip("Refresh")
                            .on_click(cx.listener(|this, _, _, cx| this.refresh_active(cx))),
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
                            .child(div().flex_1())
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
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.commit(window, cx)),
                                    ),
                            ),
                    ),
            )
            .into_any_element()
    }
}
