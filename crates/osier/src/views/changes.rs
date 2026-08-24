use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
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
        'A' | '?' => Tag::success()
            .xsmall()
            .child(if code == '?' { "?" } else { "A" }),
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
            list = list.child(
                h_flex()
                    .px_3()
                    .py_1()
                    .gap_2()
                    .text_xs()
                    .font_medium()
                    .text_color(cx.theme().muted_foreground)
                    .child(title)
                    .child(div().child(entries.len().to_string())),
            );
            for (ix, entry) in entries.iter().enumerate() {
                let selected = matches!(&selection, Selection::File { path, area: a } if path == &entry.path && *a == area);
                let path = entry.path.clone();
                let display = match &entry.orig_path {
                    Some(old) => format!("{old} → {}", entry.path),
                    None => entry.path.clone(),
                };
                list = list.child(
                    h_flex()
                        .id((title, ix))
                        .px_3()
                        .py_1()
                        .gap_2()
                        .items_center()
                        .cursor_pointer()
                        .when(selected, |this| this.bg(cx.theme().list_active))
                        .hover(|this| this.bg(cx.theme().list_hover))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.select_file(path.clone(), area, cx)
                        }))
                        .child(
                            Checkbox::new(SharedString::from(format!("stage-{title}-{ix}")))
                                .small()
                                .checked(area == Area::Staged)
                                .disabled(true),
                        )
                        .child(status_tag(entry, area))
                        .child(div().text_sm().flex_1().min_w_0().truncate().child(display)),
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
                            .text_color(cx.theme().muted_foreground)
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
        let later = "Committing is not available yet";

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
                            .text_color(cx.theme().muted_foreground)
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
                            .disabled(true)
                            .tooltip(later),
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
                    .child(
                        div()
                            .h(px(96.))
                            .w_full()
                            .rounded(cx.theme().radius)
                            .border_1()
                            .border_color(cx.theme().border)
                            .bg(cx.theme().muted.opacity(0.3))
                            .p_2()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("Commit message"),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .truncate()
                                    .flex_1()
                                    .min_w_0()
                                    .child(identity),
                            )
                            .child(
                                Button::new("commit-push")
                                    .outline()
                                    .small()
                                    .label("Commit and Push")
                                    .disabled(true)
                                    .tooltip(later),
                            )
                            .child(
                                Button::new("commit")
                                    .primary()
                                    .small()
                                    .icon(Lucide::GitCommitHorizontal)
                                    .label(format!("Commit {} files", staged.len()))
                                    .disabled(true)
                                    .tooltip(later),
                            ),
                    ),
            )
            .into_any_element()
    }
}
