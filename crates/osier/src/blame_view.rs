//! Blame in the diff pane, and the file history filter for the Log tab.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use osier_git::blame::BlameLine;

use crate::app::ops::describe;
use crate::app::{Osier, Tab};
use crate::format::iso_date;
use crate::notices::{Notice, WindowNoticeExt as _};
use crate::project::Selection;

#[derive(Debug, Clone)]
pub(crate) struct BlameView {
    pub path: String,
    pub rev: Option<String>,
    pub lines: Option<Vec<BlameLine>>,
    pub error: Option<String>,
}

impl Osier {
    /// Blame `path` (working tree, or at `rev`) in the diff pane.
    pub(crate) fn show_blame(&mut self, path: String, rev: Option<String>, cx: &mut Context<Self>) {
        let Some(project) = self.active_project() else {
            return;
        };
        let repo = project.repo.clone();
        self.editor = None;
        self.blame = Some(BlameView {
            path: path.clone(),
            rev: rev.clone(),
            lines: None,
            error: None,
        });
        cx.notify();
        cx.spawn(async move |this, cx| {
            let (p, r) = (path.clone(), rev.clone());
            let result = cx
                .background_spawn(async move { osier_git::blame::blame(&repo, &p, r.as_deref()) })
                .await;
            let _ = this.update(cx, |this, cx| {
                let Some(view) = this.blame.as_mut() else {
                    return;
                };
                if view.path != path || view.rev != rev {
                    return;
                }
                match result {
                    Ok(lines) => view.lines = Some(lines),
                    Err(e) => view.error = Some(describe(&e)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn close_blame(&mut self, cx: &mut Context<Self>) {
        self.blame = None;
        cx.notify();
    }

    /// Switch the Log tab to the history of one file (`log --follow`).
    pub(crate) fn show_file_history(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(ix) = self.active else {
            return;
        };
        if let Some(p) = self.projects.get_mut(ix) {
            p.log_path = Some(path);
            p.selected_commit = None;
        }
        self.set_tab(Tab::Log, cx);
        self.refresh_project(ix, cx);
    }

    pub(crate) fn clear_file_history(&mut self, cx: &mut Context<Self>) {
        let Some(ix) = self.active else {
            return;
        };
        if let Some(p) = self.projects.get_mut(ix) {
            p.log_path = None;
        }
        self.refresh_project(ix, cx);
    }

    /// Show a commit by hash: through the Log when it is loaded, directly otherwise.
    pub(crate) fn select_commit_hash(&mut self, hash: String, cx: &mut Context<Self>) {
        let loaded = self
            .active_project()
            .and_then(|p| p.snapshot.as_ref())
            .and_then(|s| s.commits.iter().position(|c| c.hash == hash));
        self.set_tab(Tab::Log, cx);
        match loaded {
            Some(ix) => self.select_commit(ix, cx),
            None => {
                self.editor = None;
                if let Some(p) = self.active_project_mut() {
                    p.selection = Selection::Commit { hash };
                    p.selected_commit = None;
                }
                self.reload_selection(cx);
                cx.notify();
            }
        }
    }

    pub(crate) fn render_blame(&self, view: &BlameView, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let mono = cx.theme().mono_font_family.clone();
        let Some(lines) = &view.lines else {
            if let Some(error) = &view.error {
                return self
                    .empty_state("Blame failed", error, cx)
                    .into_any_element();
            }
            return self
                .empty_state("Blame", "Reading history…", cx)
                .into_any_element();
        };
        if lines.is_empty() {
            return self
                .empty_state(
                    "Blame",
                    "Nothing to annotate: the file is empty or untracked.",
                    cx,
                )
                .into_any_element();
        }
        let lines = std::rc::Rc::new(lines.clone());
        let count = lines.len();
        let border = cx.theme().border;
        let accent = cx.theme().primary;
        let owner = cx.entity().downgrade();
        let width = (count.to_string().len() as f32 * 8.0).max(32.0);
        uniform_list("blame", count, move |range, _, cx| {
            let mut rows = Vec::with_capacity(range.len());
            for ix in range {
                let l = &lines[ix];
                let first_of_block = ix == 0 || lines[ix - 1].hash != l.hash;
                let hash = l.hash.clone();
                let local = l.is_uncommitted();
                let owner = owner.clone();
                let hash_label: String = if local {
                    "local".into()
                } else {
                    l.hash[..l.hash.len().min(7)].to_string()
                };
                rows.push(
                    h_flex()
                        .id(("blame-row", ix))
                        .w_full()
                        .gap_2()
                        .px_2()
                        .text_xs()
                        .when(first_of_block && ix > 0, |this| {
                            this.border_t_1().border_color(border)
                        })
                        .child(
                            div()
                                .w(px(236.))
                                .flex_none()
                                .gap_2()
                                .flex()
                                .items_center()
                                .text_color(muted)
                                .when(first_of_block, |this| {
                                    this.child(
                                        div()
                                            .id(("blame-hash", ix))
                                            .font_family(mono.clone())
                                            .text_color(if local { muted } else { accent })
                                            .cursor_pointer()
                                            .hover(|s| s.underline())
                                            .when(!local, |this| {
                                                this.on_click(move |_, _, cx| {
                                                    let hash = hash.clone();
                                                    let _ = owner.update(cx, |this, cx| {
                                                        this.select_commit_hash(hash, cx)
                                                    });
                                                })
                                            })
                                            .child(hash_label),
                                    )
                                    .child(div().w(px(96.)).truncate().child(if local {
                                        "uncommitted".to_string()
                                    } else {
                                        l.author_name.clone()
                                    }))
                                    .when(!local, |this| {
                                        this.child(crate::views::time_label(
                                            ("blame-time", ix),
                                            iso_date(l.authored_at),
                                            l.authored_at,
                                            cx,
                                        ))
                                    })
                                }),
                        )
                        .child(
                            div()
                                .w(px(width))
                                .flex_none()
                                .text_right()
                                .text_color(muted)
                                .font_family(mono.clone())
                                .child(l.line_no.to_string()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .font_family(mono.clone())
                                .text_color(cx.theme().foreground)
                                .child(l.text.clone()),
                        ),
                );
            }
            rows
        })
        .size_full()
        .into_any_element()
    }

    pub(crate) fn update_submodules(
        &mut self,
        paths: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let summary = match paths.as_slice() {
            [one] => format!("Updated submodule {one}"),
            _ => "Updated submodules".to_string(),
        };
        if self
            .active_project()
            .and_then(|p| p.snapshot.as_ref())
            .is_some_and(|s| s.submodules.is_empty())
        {
            window.push_notice(Notice::warning("No submodules in this repository"), cx);
            return;
        }
        self.run_op(
            summary,
            move |repo| osier_git::ops::submodule::update(repo, &paths),
            window,
            cx,
        );
    }

    /// Small toolbar pair used by the diff header and the Files tab.
    pub(crate) fn blame_history_buttons(
        &self,
        path: String,
        rev: Option<String>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (p1, p2) = (path.clone(), path);
        h_flex()
            .gap_1()
            .child(
                Button::new("blame-file")
                    .ghost()
                    .xsmall()
                    .icon(Lucide::UserRoundSearch)
                    .tooltip("Blame: who last changed each line")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.show_blame(p1.clone(), rev.clone(), cx)
                    })),
            )
            .child(
                Button::new("history-file")
                    .ghost()
                    .xsmall()
                    .icon(Lucide::FileClock)
                    .tooltip("History of this file in the Log")
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.show_file_history(p2.clone(), cx)),
                    ),
            )
    }
}
