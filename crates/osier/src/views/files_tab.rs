use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::list::ListItem;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::tree::{TreeItem, tree};
use gpui_kit::component::{
    ActiveTheme as _, Icon, Selectable as _, Sizable as _, StyledExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app::Osier;
use crate::test_ids::TestId as _;

/// Build a tree from flat paths. Folders first, then files, both alphabetical.
/// `expand_all` opens every folder (used while filtering so matches are visible).
pub(crate) fn build_items(paths: &[String], expand_all: bool) -> Vec<TreeItem> {
    #[derive(Default)]
    struct Node {
        dirs: BTreeMap<String, Node>,
        files: Vec<String>,
    }
    let mut root = Node::default();
    for p in paths {
        let mut parts: Vec<&str> = p.split('/').collect();
        let file = parts.pop().unwrap_or("");
        let mut node = &mut root;
        for d in parts {
            node = node.dirs.entry(d.to_string()).or_default();
        }
        node.files.push(file.to_string());
    }
    fn to_items(node: &Node, prefix: &str, depth: usize, expand_all: bool) -> Vec<TreeItem> {
        let mut out = Vec::new();
        for (name, child) in &node.dirs {
            let id = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            out.push(
                TreeItem::new(id.clone(), name.clone())
                    .expanded(expand_all || (depth == 0 && node.dirs.len() <= 3))
                    .children(to_items(child, &id, depth + 1, expand_all)),
            );
        }
        for f in &node.files {
            let id = if prefix.is_empty() {
                f.clone()
            } else {
                format!("{prefix}/{f}")
            };
            out.push(TreeItem::new(id, f.clone()));
        }
        out
    }
    to_items(&root, "", 0, expand_all)
}

/// Case-insensitive substring match on the whole path, the same rule as the sidebar filter.
pub(crate) fn filter_paths(paths: &mut Vec<String>, filter: &str) {
    let filter = filter.trim().to_lowercase();
    if !filter.is_empty() {
        paths.retain(|p| p.to_lowercase().contains(&filter));
    }
}

impl Osier {
    pub(crate) fn render_files(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let (repo, entries, files, selection) = {
            let Some(project) = self.active_project() else {
                return self
                    .empty_state(
                        "No repository open",
                        "Open a repository to browse its files.",
                        cx,
                    )
                    .into_any_element();
            };
            let Some(snapshot) = &project.snapshot else {
                let name = project.name.clone();
                return self
                    .empty_state(&name, "Reading files…", cx)
                    .into_any_element();
            };
            (
                project.repo.clone(),
                snapshot.status.entries.clone(),
                snapshot.files.clone(),
                project.selection.clone(),
            )
        };
        // Status per path, and a "dirty" flag per folder, for badges.
        let mut status: BTreeMap<String, char> = BTreeMap::new();
        for e in &entries {
            let c = if e.is_conflicted() {
                'U'
            } else if e.is_untracked() {
                '?'
            } else if e.index != '.' {
                e.index
            } else {
                e.worktree
            };
            status.insert(e.path.clone(), c);
        }
        // Changed entries inside each folder, at any depth (an untracked folder counts once).
        let mut dirty_dirs: BTreeMap<String, usize> = BTreeMap::new();
        for p in status.keys() {
            let mut acc = String::new();
            let parts: Vec<&str> = p.trim_end_matches('/').split('/').collect();
            for part in &parts[..parts.len().saturating_sub(1)] {
                if !acc.is_empty() {
                    acc.push('/');
                }
                acc.push_str(part);
                *dirty_dirs.entry(acc.clone()).or_default() += 1;
            }
        }
        // Folders are marked like sidebar projects (Settings > Appearance > Sidebar).
        let changes_dot = self.settings.appearance.changes_indicator
            == osier_config::settings::ChangesIndicator::Dot;
        let changes_color = cx.theme().warning;
        // Tracked files and the folders holding them, for "Untrack" in the context menu.
        // `files` lists tracked plus untracked; status names the untracked (whole untracked
        // folders end in `/`).
        let untracked_dirs: Vec<&str> = entries
            .iter()
            .filter(|e| e.is_untracked() && e.path.ends_with('/'))
            .map(|e| e.path.as_str())
            .collect();
        let untracked_files: std::collections::HashSet<&str> = entries
            .iter()
            .filter(|e| e.is_untracked() && !e.path.ends_with('/'))
            .map(|e| e.path.as_str())
            .collect();
        let mut tracked: std::collections::HashSet<String> = Default::default();
        for f in &files {
            if untracked_files.contains(f.as_str())
                || untracked_dirs.iter().any(|d| f.starts_with(d))
            {
                continue;
            }
            let mut acc = String::new();
            for part in f.split('/') {
                if !acc.is_empty() {
                    acc.push('/');
                }
                acc.push_str(part);
                tracked.insert(acc.clone());
            }
        }
        let tracked = std::rc::Rc::new(tracked);
        let mut paths = files;
        if self.show_ignored
            && let Ok(ignored) = osier_git::files::ignored(&repo)
        {
            paths.extend(ignored);
            paths.sort();
            paths.dedup();
        }
        let total = paths.len();
        let filter = self.files_filter.read(cx).value().to_string();
        filter_paths(&mut paths, &filter);
        let filtering = !filter.trim().is_empty();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        paths.hash(&mut hasher);
        self.show_ignored.hash(&mut hasher);
        filtering.hash(&mut hasher);
        let key = hasher.finish();
        if key != self.files_tree_key {
            self.files_tree_key = key;
            let items = build_items(&paths, filtering);
            self.files_tree
                .update(cx, |state, cx| state.set_items(items, cx));
        }

        let muted = cx.theme().muted_foreground;
        let selected_path = match &selection {
            crate::project::Selection::WorkFile { path, .. } => Some(path.clone()),
            _ => None,
        };
        let weak = cx.entity().downgrade();
        let count = paths.len();
        let status = std::rc::Rc::new(status);
        let dirty_dirs = std::rc::Rc::new(dirty_dirs);

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
                    .child(div().text_sm().font_medium().flex_none().child("Files"))
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(if filtering {
                                format!("{count} of {}", crate::format::count(total, "file"))
                            } else {
                                crate::format::count(count, "file")
                            }),
                    )
                    .child(
                        div()
                            .w(px(180.))
                            .flex_none()
                            .test_id("files-filter")
                            .on_action(cx.listener(
                                |this, _: &gpui_kit::component::input::Escape, window, cx| {
                                    let input = this.files_filter.clone();
                                    this.escape_filter(input, window, cx)
                                },
                            ))
                            .child(
                                Input::new(&self.files_filter)
                                    .xsmall()
                                    .cleanable(true)
                                    .prefix(Icon::new(Lucide::Search).xsmall()),
                            ),
                    )
                    .child(
                        Button::new("show-ignored")
                            .test_id("show-ignored")
                            .ghost()
                            .xsmall()
                            .label("Ignored")
                            .selected(self.show_ignored)
                            .tooltip("Show files ignored by .gitignore")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_ignored = !this.show_ignored;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div().flex_1().min_h_0().child(
                    tree(&self.files_tree, {
                        let weak = weak.clone();
                        let status = status.clone();
                        let dirty_dirs = dirty_dirs.clone();
                        move |ix, entry, _selected, _window, cx| {
                            let item = entry.item();
                            let id = item.id.to_string();
                            let is_folder = entry.is_folder();
                            let changed_inside = if is_folder {
                                dirty_dirs.get(&id).copied()
                            } else {
                                None
                            };
                            let badge = if is_folder {
                                None
                            } else {
                                status.get(&id).copied()
                            };
                            let icon = if !is_folder {
                                Lucide::File
                            } else if entry.is_expanded() {
                                Lucide::FolderOpen
                            } else {
                                Lucide::Folder
                            };
                            let weak = weak.clone();
                            let click_id = id.clone();
                            let is_selected = selected_path.as_deref() == Some(id.as_str());
                            ListItem::new(ix)
                                .w_full()
                                .px_2()
                                .pl(px(14.) * entry.depth() + px(8.))
                                .selected(is_selected)
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .items_center()
                                        .min_w_0()
                                        .child(match changed_inside {
                                            // Icon style: the folder icon itself is tinted.
                                            Some(n) if !changes_dot => super::hint(
                                                SharedString::from(format!("dirty-icon:{id}")),
                                                format!(
                                                    "{} inside",
                                                    crate::format::count(n, "changed file")
                                                ),
                                                "files-change-tooltip",
                                                cx,
                                            )
                                            .test_id(format!("files-dirty:{id}"))
                                            .child(
                                                Icon::new(icon).xsmall().text_color(changes_color),
                                            )
                                            .into_any_element(),
                                            _ => Icon::new(icon).xsmall().into_any_element(),
                                        })
                                        .child(
                                            div()
                                                .text_sm()
                                                .truncate()
                                                .min_w_0()
                                                .flex_1()
                                                .child(item.label.clone()),
                                        )
                                        // Dot style: a dot in the sidebar's colour.
                                        .when_some(
                                            changed_inside.filter(|_| changes_dot),
                                            |this, n| {
                                                this.child(
                                                    super::hint(
                                                        SharedString::from(format!(
                                                            "dirty-dot:{id}"
                                                        )),
                                                        format!(
                                                            "{} inside",
                                                            crate::format::count(n, "changed file")
                                                        ),
                                                        "files-change-tooltip",
                                                        cx,
                                                    )
                                                    .test_id(format!("files-dirty:{id}"))
                                                    .flex_none()
                                                    .mx_1()
                                                    .size(px(6.))
                                                    .rounded_full()
                                                    .bg(changes_color),
                                                )
                                            },
                                        )
                                        .when_some(badge, |this, b| {
                                            let tag = match b {
                                                'M' => Tag::warning().xsmall().child("M"),
                                                'A' | '?' => {
                                                    Tag::success().xsmall().child(b.to_string())
                                                }
                                                'D' => Tag::danger().xsmall().child("D"),
                                                'U' => Tag::danger().xsmall().child("U"),
                                                other => Tag::secondary()
                                                    .xsmall()
                                                    .child(other.to_string()),
                                            };
                                            let meaning = match b {
                                                'M' => "Modified",
                                                'A' => "Added",
                                                '?' => "Untracked (new, not added yet)",
                                                'D' => "Deleted",
                                                'U' => "Conflicted",
                                                'R' => "Renamed",
                                                'C' => "Copied",
                                                'T' => "Type changed",
                                                _ => "Changed",
                                            };
                                            this.child(
                                                super::hint(
                                                    SharedString::from(format!("status:{id}")),
                                                    meaning,
                                                    "files-status-tooltip",
                                                    cx,
                                                )
                                                .flex_none()
                                                .child(tag),
                                            )
                                        }),
                                )
                                .on_click(move |_, _, cx| {
                                    if !is_folder {
                                        let _ = weak.update(cx, |this, cx| {
                                            this.select_work_file(click_id.clone(), None, cx)
                                        });
                                    }
                                })
                        }
                    })
                    .context_menu({
                        let weak = weak.clone();
                        let tracked = tracked.clone();
                        move |_ix, entry, menu, _window, _cx| {
                            let path = entry.item().id.to_string();
                            let is_folder = entry.is_folder();
                            let row = crate::ignore_ops::RowState {
                                is_dir: is_folder,
                                tracked: tracked.contains(&path),
                            };
                            let (w9, p9) = (weak.clone(), path.clone());
                            let (w1, w2, w3) = (weak.clone(), weak.clone(), weak.clone());
                            let (p1, p2, p3) = (path.clone(), path.clone(), path.clone());
                            let (w4, w5, w6) = (weak.clone(), weak.clone(), weak.clone());
                            let (p4, p5, p6) = (path.clone(), path.clone(), path.clone());
                            let menu = menu
                                .when(!is_folder, |m| {
                                    m.item(
                                        crate::row_actions::menu_item(
                                            "View at HEAD",
                                            crate::row_actions::RowKind::ViewAtHead,
                                        )
                                        .on_click(
                                            move |_, _, cx| {
                                                let _ = w6.update(cx, |this, cx| {
                                                    this.select_work_file(
                                                        p6.clone(),
                                                        Some("HEAD".into()),
                                                        cx,
                                                    )
                                                });
                                            },
                                        ),
                                    )
                                    .item(
                                        crate::row_actions::menu_item(
                                            "Blame",
                                            crate::row_actions::RowKind::Blame,
                                        )
                                        .on_click(
                                            move |_, _, cx| {
                                                let _ = w4.update(cx, |this, cx| {
                                                    this.show_blame(p4.clone(), None, cx)
                                                });
                                            },
                                        ),
                                    )
                                    .item(
                                        crate::row_actions::menu_item(
                                            "History",
                                            crate::row_actions::RowKind::FileHistory,
                                        )
                                        .on_click(
                                            move |_, _, cx| {
                                                let _ = w5.update(cx, |this, cx| {
                                                    this.show_file_history(p5.clone(), cx)
                                                });
                                            },
                                        ),
                                    )
                                    .separator()
                                })
                                .when(!is_folder, |m| {
                                    m.item(
                                        crate::row_actions::menu_item(
                                            "Open in editor",
                                            crate::row_actions::RowKind::OpenInEditor,
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
                                })
                                .item(
                                    crate::row_actions::menu_item(
                                        "Open in file manager",
                                        crate::row_actions::RowKind::OpenInFileManager,
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
                                    crate::row_actions::menu_item(
                                        "Copy path",
                                        crate::row_actions::RowKind::CopyPath,
                                    )
                                    .on_click(
                                        move |_, window, cx| {
                                            let _ = w3.update(cx, |this, cx| {
                                                this.copy_to_clipboard(p3.clone(), window, cx)
                                            });
                                        },
                                    ),
                                )
                                .separator();
                            Osier::ignore_menu(menu, w9, p9, row)
                        }
                    })
                    .size_full(),
                ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{build_items, filter_paths};

    #[test]
    fn filter_matches_anywhere_in_the_path_case_insensitively() {
        let mut paths = vec![
            "src/main.rs".to_string(),
            "src/views/Files.rs".to_string(),
            "README.md".to_string(),
        ];
        filter_paths(&mut paths, " files ");
        assert_eq!(paths, ["src/views/Files.rs"]);
        let mut all = vec!["a".to_string()];
        filter_paths(&mut all, "   ");
        assert_eq!(all, ["a"]);
    }

    #[test]
    fn filtering_expands_every_folder() {
        let paths = vec![
            "a/b/c/d.rs".to_string(),
            "e/f.rs".to_string(),
            "g/h.rs".to_string(),
            "i/j.rs".to_string(),
            "k/l.rs".to_string(),
        ];
        let collapsed = build_items(&paths, false);
        assert!(
            collapsed.iter().all(|i| !i.is_expanded()),
            "more than 3 roots stay closed"
        );
        let open = build_items(&paths, true);
        assert!(open.iter().all(|i| i.is_expanded()));
    }
}
