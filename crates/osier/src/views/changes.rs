use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::Textarea;
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
                        .child(
                            Checkbox::new(SharedString::from(format!("stage-{title}-{ix}")))
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
                        .child(
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
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.discard_file(discard_path.clone(), area, window, cx)
                                    })),
                                ),
                        ),
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
                    .child(Textarea::new(&self.commit_message))
                    .child(
                        h_flex()
                            .gap_3()
                            .items_center()
                            .text_xs()
                            .text_color(muted)
                            .child(div().truncate().flex_1().min_w_0().child(identity))
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
                                    .outline()
                                    .small()
                                    .label("Commit and Push")
                                    .disabled(true)
                                    .tooltip("Push is not available yet"),
                            )
                            .child(
                                Button::new("commit")
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
