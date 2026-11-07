//! Commit details under the Log (toggle: ctrl-shift-d or the header button): the message on
//! the left, hash, author, dates, parents, refs and the branches containing it on the right.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use osier_git::log::Commit;

use crate::app::Osier;
use crate::test_ids::TestId as _;

impl Osier {
    pub(crate) fn toggle_commit_details(&mut self, cx: &mut Context<Self>) {
        self.state.layout.log_details_hidden = !self.state.layout.log_details_hidden;
        self.save_state();
        cx.notify();
    }

    pub(crate) fn commit_details_visible(&self) -> bool {
        !self.state.layout.log_details_hidden
    }

    /// Look up the branches containing `hash` in the background, once per hash.
    fn ensure_commit_branches(&mut self, hash: &str, cx: &mut Context<Self>) {
        if self.commit_branches.contains_key(hash)
            || self.commit_branches_loading.as_deref() == Some(hash)
        {
            return;
        }
        let Some(repo) = self.active_project().map(|p| p.repo.clone()) else {
            return;
        };
        self.commit_branches_loading = Some(hash.to_string());
        let busy = self.background.begin(crate::background::BRANCHES);
        let hash = hash.to_string();
        cx.spawn(async move |this, cx| {
            let _busy = busy;
            let h = hash.clone();
            let found = cx
                .background_spawn(async move { osier_git::refs::branches_containing(&repo, &h) })
                .await
                .unwrap_or_default();
            let _ = this.update(cx, |this, cx| {
                if this.commit_branches_loading.as_deref() == Some(hash.as_str()) {
                    this.commit_branches_loading = None;
                }
                this.commit_branches.insert(hash, found);
                cx.notify();
            });
        })
        .detach();
    }

    /// The panel's content for the selected commit.
    pub(crate) fn render_commit_details(&self, cx: &mut Context<Self>) -> AnyElement {
        let commit = self
            .active_project()
            .and_then(|p| p.selected_commit.and_then(|ix| p.commit_at(ix)))
            .cloned();
        let theme = cx.theme();
        let (muted, border, bg) = (theme.muted_foreground, theme.border, theme.background);
        let Some(commit) = commit else {
            return div()
                .test_id("commit-details")
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(bg)
                .text_sm()
                .text_color(muted)
                .child("Pick a commit to see its details.")
                .into_any_element();
        };
        let branches = self.commit_branches.get(&commit.hash).cloned();
        if branches.is_none() {
            let hash = commit.hash.clone();
            let weak = cx.entity().downgrade();
            cx.defer(move |cx| {
                let _ = weak.update(cx, |this, cx| this.ensure_commit_branches(&hash, cx));
            });
        }
        let body = commit.body.trim().to_string();
        h_flex()
            .test_id("commit-details")
            .size_full()
            .items_start()
            .bg(bg)
            .child(
                // Left: the message.
                v_flex()
                    .id("commit-details-message")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_y_scroll()
                    .p_3()
                    .gap_2()
                    .child(div().text_base().font_weight(FontWeight::SEMIBOLD).child(
                        if commit.subject.is_empty() {
                            "(no subject)".to_string()
                        } else {
                            commit.subject.clone()
                        },
                    ))
                    .when(!body.is_empty(), |this| {
                        this.child(
                            div()
                                .text_sm()
                                .text_color(muted)
                                .whitespace_normal()
                                .children(
                                    body.lines()
                                        .map(|l| div().min_h(px(16.)).child(l.to_string())),
                                ),
                        )
                    }),
            )
            .child(
                // Right: who, when, where.
                v_flex()
                    .id("commit-details-meta")
                    .w(px(300.))
                    .flex_none()
                    .h_full()
                    .overflow_y_scroll()
                    .border_l_1()
                    .border_color(border)
                    .p_3()
                    .gap_2()
                    .text_sm()
                    .child(self.meta_hash(&commit, cx))
                    .child(meta_row(
                        "Author",
                        format!("{} <{}>", commit.author_name, commit.author_email),
                        muted,
                    ))
                    .child(meta_row(
                        "Date",
                        crate::format::full_time(commit.authored_at),
                        muted,
                    ))
                    .when(
                        commit.committer_name != commit.author_name
                            || commit.committer_email != commit.author_email,
                        |this| {
                            this.child(meta_row(
                                "Committer",
                                format!("{} <{}>", commit.committer_name, commit.committer_email),
                                muted,
                            ))
                        },
                    )
                    .when(
                        (commit.committed_at - commit.authored_at).abs() > 60,
                        |this| {
                            this.child(meta_row(
                                "Committed",
                                crate::format::full_time(commit.committed_at),
                                muted,
                            ))
                        },
                    )
                    .when(!commit.parents.is_empty(), |this| {
                        this.child(self.meta_parents(&commit, cx))
                    })
                    .when(!commit.refs.is_empty(), |this| {
                        this.child(
                            v_flex().gap_1().child(label("Refs", muted)).child(
                                h_flex()
                                    .flex_wrap()
                                    .gap_1()
                                    .children(commit.refs.iter().map(|r| {
                                        let is_head = r.starts_with("HEAD");
                                        let is_tag = r.starts_with("tag: ");
                                        let name = r
                                            .trim_start_matches("HEAD -> ")
                                            .trim_start_matches("tag: ")
                                            .to_string();
                                        super::log_tab::ref_tag(name, is_head, is_tag, cx)
                                    })),
                            ),
                        )
                    })
                    .child(meta_row(
                        "In branches",
                        match &branches {
                            None => "…".to_string(),
                            Some(b) if b.is_empty() => "None (not on any branch)".to_string(),
                            Some(b) if b.len() == 1 => format!("1 branch: {}", b[0]),
                            Some(b) => format!("{} branches: {}", b.len(), b.join(", ")),
                        },
                        muted,
                    )),
            )
            .into_any_element()
    }

    fn meta_hash(&self, commit: &Commit, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let mono = cx.theme().mono_font_family.clone();
        let hash = commit.hash.clone();
        v_flex().gap_0p5().child(label("Commit", muted)).child(
            h_flex()
                .gap_1()
                .items_center()
                .child(
                    div()
                        .font_family(mono)
                        .text_xs()
                        .child(commit.hash[..commit.hash.len().min(12)].to_string()),
                )
                .child(
                    Button::new("copy-commit-hash")
                        .test_id("copy-commit-hash")
                        .ghost()
                        .xsmall()
                        .icon(Lucide::Copy)
                        .tooltip("Copy the full hash")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.copy_to_clipboard(hash.clone(), window, cx)
                        })),
                ),
        )
    }

    fn meta_parents(&self, commit: &Commit, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let mono = cx.theme().mono_font_family.clone();
        v_flex()
            .gap_0p5()
            .child(label(
                if commit.parents.len() == 1 {
                    "Parent"
                } else {
                    "Parents"
                },
                muted,
            ))
            .child(
                h_flex()
                    .gap_2()
                    .children(commit.parents.iter().enumerate().map(|(ix, parent)| {
                        let p = parent.clone();
                        Button::new(("commit-parent", ix))
                            .ghost()
                            .xsmall()
                            .label(SharedString::from(
                                parent[..parent.len().min(7)].to_string(),
                            ))
                            .tooltip("Show this commit")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_commit_hash(p.clone(), cx)
                            }))
                            .font_family(mono.clone())
                    })),
            )
    }
}

fn label(text: &'static str, muted: Hsla) -> Div {
    div().text_xs().text_color(muted).child(text)
}

fn meta_row(name: &'static str, value: String, muted: Hsla) -> Div {
    v_flex()
        .gap_0p5()
        .child(label(name, muted))
        .child(div().whitespace_normal().child(value))
}
