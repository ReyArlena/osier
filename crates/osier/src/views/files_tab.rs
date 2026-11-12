use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};

use crate::fade::EndText as _;
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
use crate::app::row_actions;
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

/// Status badges and the tracked set for the Files tab, for one snapshot.
#[derive(Clone, Default)]
pub(crate) struct FilesBadges {
    status_src: std::sync::Arc<osier_git::status::RepoStatus>,
    files_src: std::sync::Arc<Vec<String>>,
    /// Status letter per changed path.
    status: std::sync::Arc<BTreeMap<String, char>>,
    /// Changed entries inside each folder.
    dirty_dirs: std::sync::Arc<BTreeMap<String, usize>>,
    /// Tracked files and the folders holding them ("Untrack" in the context menu).
    tracked: std::sync::Arc<std::collections::HashSet<String>>,
}

impl FilesBadges {
    fn is_for(
        &self,
        status: &std::sync::Arc<osier_git::status::RepoStatus>,
        files: &std::sync::Arc<Vec<String>>,
    ) -> bool {
        std::sync::Arc::ptr_eq(&self.status_src, status)
            && std::sync::Arc::ptr_eq(&self.files_src, files)
    }

    fn new(
        status_src: std::sync::Arc<osier_git::status::RepoStatus>,
        files: std::sync::Arc<Vec<String>>,
    ) -> Self {
        // Status per path, and a "dirty" flag per folder, for badges.
        let mut status: BTreeMap<String, char> = BTreeMap::new();
        for e in &status_src.entries {
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
        // Tracked files and the folders holding them, for "Untrack" in the context menu.
        // `files` lists tracked plus untracked; status names the untracked (whole untracked
        // folders end in `/`).
        let untracked_dirs: Vec<&str> = status_src
            .entries
            .iter()
            .filter(|e| e.is_untracked() && e.path.ends_with('/'))
            .map(|e| e.path.as_str())
            .collect();
        let untracked_files: std::collections::HashSet<&str> = status_src
            .entries
            .iter()
            .filter(|e| e.is_untracked() && !e.path.ends_with('/'))
            .map(|e| e.path.as_str())
            .collect();
        let mut tracked: std::collections::HashSet<String> = Default::default();
        for f in files.iter() {
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
        Self {
            status: std::sync::Arc::new(status),
            dirty_dirs: std::sync::Arc::new(dirty_dirs),
            tracked: std::sync::Arc::new(tracked),
            status_src,
            files_src: files,
        }
    }
}

/// Case-insensitive substring match on the whole path, the same rule as the sidebar filter.
/// The Files tab's rows for one listing, ignored list and filter. Building them sorts and
/// walks every path, which with thousands of ignored files (eg a build folder) takes longer
/// than a frame, so it is done once per change rather than on every draw.
#[derive(Clone)]
pub(crate) struct FilesListing {
    files: std::sync::Arc<Vec<String>>,
    ignored: Option<std::sync::Arc<crate::project::IgnoredFiles>>,
    filter: String,
    /// Tracked, untracked and (when shown) ignored paths that pass the filter.
    paths: std::rc::Rc<Vec<String>>,
    ignored_rows: std::rc::Rc<BTreeMap<String, IgnoredRow>>,
    /// Paths before filtering.
    total: usize,
    filtering: bool,
    /// Hash of what the tree is built from: a new key rebuilds the tree.
    key: u64,
}

impl FilesListing {
    fn new(
        files: std::sync::Arc<Vec<String>>,
        ignored: Option<std::sync::Arc<crate::project::IgnoredFiles>>,
        filter: String,
    ) -> Self {
        let mut paths: Vec<String> = files.as_ref().clone();
        let rows = match &ignored {
            Some(ignored) => {
                paths.extend(ignored.files.iter().cloned());
                paths.sort();
                paths.dedup();
                ignored_rows(&paths, ignored)
            }
            None => BTreeMap::new(),
        };
        let total = paths.len();
        filter_paths(&mut paths, &filter);
        let filtering = !filter.trim().is_empty();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        paths.hash(&mut hasher);
        ignored.is_some().hash(&mut hasher);
        filtering.hash(&mut hasher);
        Self {
            key: hasher.finish(),
            files,
            ignored,
            filter,
            paths: std::rc::Rc::new(paths),
            ignored_rows: std::rc::Rc::new(rows),
            total,
            filtering,
        }
    }

    /// Whether this was built from exactly these lists (the same snapshot) and filter.
    fn is_for(
        &self,
        files: &std::sync::Arc<Vec<String>>,
        ignored: Option<&std::sync::Arc<crate::project::IgnoredFiles>>,
        filter: &str,
    ) -> bool {
        std::sync::Arc::ptr_eq(&self.files, files)
            && match (&self.ignored, ignored) {
                (Some(a), Some(b)) => std::sync::Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
            && self.filter == filter
    }
}

/// How an ignored row is shown: badges naming the files the rules come from, and a tooltip
/// with the rules themselves.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct IgnoredRow {
    pub(crate) sources: Vec<String>,
    pub(crate) tooltip: String,
}

/// Ignored rows by tree id: every ignored file, and every folder whose files are all ignored
/// (with each rule file that ignores something inside it).
pub(crate) fn ignored_rows(
    paths: &[String],
    ignored: &crate::project::IgnoredFiles,
) -> BTreeMap<String, IgnoredRow> {
    use std::collections::BTreeSet;
    let label = |source: &str| {
        if source.starts_with('/') {
            crate::format::tilde(std::path::Path::new(source))
        } else {
            source.to_string()
        }
    };
    let ignored_set: std::collections::HashSet<&str> =
        ignored.files.iter().map(String::as_str).collect();
    let mut rows = BTreeMap::new();
    // Per folder: (files inside, rule files of the ignored ones), or None once a file inside
    // is not ignored.
    let mut dirs: BTreeMap<&str, Option<BTreeSet<String>>> = BTreeMap::new();
    for p in paths {
        let is_ignored = ignored_set.contains(p.as_str());
        let rule = ignored.rules.get(p);
        if is_ignored {
            let (sources, tooltip) = match rule {
                Some(r) => (
                    vec![label(&r.source)],
                    format!(
                        "Ignored by {} in {} (line {})",
                        r.pattern,
                        label(&r.source),
                        r.line
                    ),
                ),
                None => (Vec::new(), "Ignored".to_string()),
            };
            rows.insert(p.clone(), IgnoredRow { sources, tooltip });
        }
        let mut end = 0;
        while let Some(slash) = p[end..].find('/') {
            end += slash;
            let dir = dirs
                .entry(&p[..end])
                .or_insert_with(|| Some(BTreeSet::new()));
            match (dir.as_mut(), is_ignored) {
                (Some(sources), true) => sources.extend(rule.map(|r| label(&r.source))),
                (Some(_), false) => *dir = None,
                (None, _) => {}
            }
            end += 1;
        }
    }
    for (dir, sources) in dirs {
        if let Some(sources) = sources {
            let tooltip = if sources.is_empty() {
                "Everything inside is ignored".to_string()
            } else {
                format!(
                    "Everything inside is ignored (rules in {})",
                    sources.iter().cloned().collect::<Vec<_>>().join(", ")
                )
            };
            rows.insert(
                dir.to_string(),
                IgnoredRow {
                    sources: sources.into_iter().collect(),
                    tooltip,
                },
            );
        }
    }
    rows
}

/// Open (true) or closed state of every folder in `tree`, collapsed ones' subfolders
/// included.
fn folder_states(
    tree: &gpui_kit::component::tree::TreeState,
) -> std::collections::HashMap<String, bool> {
    fn walk(item: &TreeItem, out: &mut std::collections::HashMap<String, bool>) {
        if item.is_folder() {
            out.insert(item.id.to_string(), item.is_expanded());
            for child in &item.children {
                walk(child, out);
            }
        }
    }
    let mut out = std::collections::HashMap::new();
    for entry in (0..).map_while(|ix| tree.entry(ix)) {
        if entry.is_root() {
            walk(entry.item(), &mut out);
        }
    }
    out
}

/// Give folders the state they had in `states`; new folders keep `build_items`' default.
fn apply_folder_states(items: &mut [TreeItem], states: &std::collections::HashMap<String, bool>) {
    for item in items {
        if let Some(&open) = states.get(item.id.as_ref()) {
            // Items share their state with their clones, so this sets `item` itself.
            let _ = item.clone().expanded(open);
        }
        apply_folder_states(&mut item.children, states);
    }
}

/// Every id `build_items` gives `paths`: each file and each folder above one.
fn tree_ids(paths: &[String]) -> std::collections::HashSet<&str> {
    let mut ids = std::collections::HashSet::new();
    for p in paths {
        ids.insert(p.as_str());
        let mut end = 0;
        while let Some(slash) = p[end..].find('/') {
            end += slash;
            ids.insert(&p[..end]);
            end += 1;
        }
    }
    ids
}

pub(crate) fn filter_paths(paths: &mut Vec<String>, filter: &str) {
    let filter = filter.trim().to_lowercase();
    if !filter.is_empty() {
        paths.retain(|p| p.to_lowercase().contains(&filter));
    }
}

impl Osier {
    /// Show the file under the Files tree's cursor when the cursor moved to another file (arrow
    /// keys, or the closest row after the file it was on left the tree). Folders leave the
    /// viewer alone, and so does the file it already shows (at any revision).
    pub(crate) fn follow_files_cursor(&mut self, cx: &mut Context<Self>) {
        if self.tab != crate::app::Tab::Files {
            return;
        }
        let Some(entry) = self.files_tree.read(cx).selected_entry() else {
            return;
        };
        if entry.is_folder() {
            return;
        }
        let path = entry.item().id.to_string();
        let showing = self.active_project().is_some_and(|p| {
            matches!(&p.selection, crate::project::Selection::WorkFile { path: q, .. } if *q == path)
        });
        if !showing {
            self.select_work_file(path, None, cx);
        }
    }

    pub(crate) fn render_files(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let (repo, status_src, files, ignored) = {
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
            // The ignored list comes with the snapshot. Ask for one when it is missing (just
            // switched on, or the last refresh ran on another tab); `loading` and `error` keep
            // this from asking again while a load runs or after one failed.
            if self.show_ignored
                && snapshot.ignored.is_none()
                && !project.loading
                && project.error.is_none()
                && let Some(ix) = self.active
            {
                let weak = cx.entity().downgrade();
                cx.defer(move |cx| {
                    let _ = weak.update(cx, |this, cx| this.refresh_project(ix, cx));
                });
            }
            (
                project.repo.clone(),
                snapshot.status.clone(),
                snapshot.files.clone(),
                snapshot.ignored.clone().filter(|_| self.show_ignored),
            )
        };
        // Badges and the tracked set are built once per snapshot, in the background: walking
        // every file of a big repository takes tens of milliseconds. Until the new set is back
        // the previous one is shown (or none, the first time).
        let fresh = self
            .files_badges
            .as_ref()
            .is_some_and(|b| b.is_for(&status_src, &files));
        let pending = self.files_badges_pending.as_ref().is_some_and(|(s, f)| {
            std::sync::Arc::ptr_eq(s, &status_src) && std::sync::Arc::ptr_eq(f, &files)
        });
        if !fresh && !pending {
            self.files_badges_pending = Some((status_src.clone(), files.clone()));
            let (s, f) = (status_src.clone(), files.clone());
            self.in_background_cx(
                crate::background::FILES_BADGES,
                move || FilesBadges::new(s, f),
                |this, badges, cx| {
                    let current = this
                        .files_badges_pending
                        .as_ref()
                        .is_some_and(|(s, f)| badges.is_for(s, f));
                    if current {
                        this.files_badges_pending = None;
                        this.files_badges = Some(badges);
                        cx.notify();
                    }
                },
                cx,
            );
        }
        let badges = self.files_badges.clone().unwrap_or_default();
        let FilesBadges {
            status,
            dirty_dirs,
            tracked,
            ..
        } = badges;
        // Folders are marked like sidebar projects (Settings > Appearance > Sidebar).
        let changes_dot = self.settings.appearance.changes_indicator
            == osier_config::settings::ChangesIndicator::Dot;
        let changes_color = cx.theme().warning;
        let filter = self.files_filter.read(cx).value().to_string();
        let listing = match &self.files_listing {
            Some(l) if l.is_for(&files, ignored.as_ref(), &filter) => l.clone(),
            _ => {
                let l = FilesListing::new(files, ignored, filter);
                self.files_listing = Some(l.clone());
                l
            }
        };
        let FilesListing {
            paths,
            ignored_rows,
            total,
            filtering,
            key,
            ..
        } = listing;
        if key != self.files_tree_key {
            self.files_tree_key = key;
            let root = repo.workdir().to_path_buf();
            let same_project = self.files_tree_root.as_ref() == Some(&root);
            self.files_tree_root = Some(root);
            // Folders keep the open or closed state they had, also when they leave the tree
            // for a while (a build folder being rebuilt, ignored files hidden). A filtered
            // tree opens every folder, so the state from before the filter is kept for when
            // it is cleared.
            if !same_project {
                self.files_tree_folders.clear();
            } else if !self.files_tree_filtering {
                let states = folder_states(self.files_tree.read(cx));
                self.files_tree_folders.extend(states);
            }
            self.files_tree_filtering = filtering;
            let mut items = build_items(&paths, filtering);
            if !filtering {
                apply_folder_states(&mut items, &self.files_tree_folders);
            }
            // Rebuilding the tree clears its cursor. Put it back on the same entry, or on
            // the closest one when that entry has gone (eg a file that was just ignored).
            let (was, cursor) = {
                let tree = self.files_tree.read(cx);
                let was: Vec<SharedString> = (0..)
                    .map_while(|ix| tree.entry(ix).map(|e| e.item().id.clone()))
                    .collect();
                (was, tree.selected_item().map(|i| i.id.clone()))
            };
            let ids = tree_ids(&paths);
            let next = cursor.filter(|_| same_project).and_then(|c| {
                if ids.contains(c.as_ref()) {
                    Some(c)
                } else {
                    crate::project::neighbour(&was, &c, |id| ids.contains(id.as_ref())).cloned()
                }
            });
            self.files_tree.update(cx, |state, cx| {
                state.set_items(items, cx);
                if let Some(id) = &next {
                    state.set_selected_item(Some(&TreeItem::new(id.clone(), id.clone())), cx);
                }
            });
            // The viewer follows the cursor to its new row. The tree's own notify comes
            // during this draw, which its observer does not hear, so follow after it.
            if next.is_some() {
                let weak = cx.entity().downgrade();
                cx.defer(move |cx| {
                    let _ = weak.update(cx, |this, cx| this.follow_files_cursor(cx));
                });
            }
        }

        let muted = cx.theme().muted_foreground;
        let text_overflow = self.settings.appearance.text_overflow;
        let weak = cx.entity().downgrade();
        let count = paths.len();

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
                            .end_text(
                                self.settings.appearance.text_overflow,
                                crate::fade::Behind::plain(cx),
                                if filtering {
                                    format!("{count} of {}", crate::format::count(total, "file"))
                                } else {
                                    crate::format::count(count, "file")
                                },
                            ),
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
                        let ignored_rows = ignored_rows.clone();
                        move |ix, entry, selected, _window, cx| {
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
                            let ignored_row = ignored_rows.get(&id);
                            let dim = ignored_row.is_some();
                            let weak = weak.clone();
                            let click_id = id.clone();
                            // The tree's own cursor, which is what gpui-kit paints as the
                            // selected row. Not the file the viewer shows: clicking a folder
                            // moves the cursor there and leaves the viewer on its file.
                            let is_selected = selected;
                            // The colours ListItem paints behind the name, for its fade: a
                            // selected row keeps its colour under the pointer.
                            let behind = {
                                let theme = cx.theme();
                                let bg = theme.background;
                                let selected_bg = if theme.list.active_highlight {
                                    theme.list_active
                                } else {
                                    theme.accent
                                };
                                let base = if is_selected {
                                    bg.blend(selected_bg)
                                } else {
                                    bg
                                };
                                let hover = if is_selected {
                                    base
                                } else {
                                    bg.blend(theme.list_hover)
                                };
                                crate::fade::Behind::colors(base, hover, "files-row")
                            };
                            // a11y: the tree names its own items.
                            ListItem::new(ix)
                                .test_id(|| format!("files-row:{id}"))
                                .group("files-row")
                                .w_full()
                                .px_2()
                                .pl(px(14.) * entry.depth() + px(8.))
                                .selected(is_selected)
                                .child(
                                    h_flex()
                                        // Tests: the row whose fade blends into the selected
                                        // colour.
                                        .when(is_selected, |row| {
                                            row.test_id(|| format!("files-row-selected:{id}"))
                                        })
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
                                            .test_id(|| format!("files-dirty:{id}"))
                                            .child(
                                                Icon::new(icon).xsmall().text_color(changes_color),
                                            )
                                            .into_any_element(),
                                            _ => Icon::new(icon)
                                                .xsmall()
                                                .when(dim, |i| i.text_color(muted))
                                                .into_any_element(),
                                        })
                                        .child(
                                            div()
                                                .test_id(|| format!("files-name:{id}"))
                                                .text_sm()
                                                .min_w_0()
                                                .flex_1()
                                                .when(dim, |d| d.text_color(muted))
                                                .end_text(
                                                    text_overflow,
                                                    behind,
                                                    item.label.clone(),
                                                ),
                                        )
                                        // Ignored: where the rule lives, the rule itself in
                                        // the tooltip.
                                        .when_some(ignored_row, |this, row| {
                                            this.child(
                                                super::hint(
                                                    SharedString::from(format!("ignored:{id}")),
                                                    row.tooltip.clone(),
                                                    "files-ignored-tooltip",
                                                    cx,
                                                )
                                                .test_id(|| format!("files-ignored:{id}"))
                                                .flex_none()
                                                .flex()
                                                .gap_1()
                                                .children(row.sources.iter().map(|source| {
                                                    Tag::secondary().xsmall().child(source.clone())
                                                })),
                                            )
                                        })
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
                                                    .test_id(|| format!("files-dirty:{id}"))
                                                    .flex_none()
                                                    .mx_1()
                                                    .size(px(6.))
                                                    .rounded_full()
                                                    .bg(changes_color),
                                                )
                                            },
                                        )
                                        .when_some(badge, |this, b| {
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
                                                .test_id(|| format!("files-status:{id}"))
                                                .child(super::changes::status_letter(b, cx)),
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
                            let row = crate::app::ignore_ops::RowState {
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
                                        row_actions::menu_item(
                                            "View at HEAD",
                                            row_actions::RowKind::ViewAtHead,
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
                                        row_actions::menu_item(
                                            "Blame",
                                            row_actions::RowKind::Blame,
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
                                        row_actions::menu_item(
                                            "History",
                                            row_actions::RowKind::FileHistory,
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
                                })
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
    use super::{IgnoredRow, build_items, filter_paths, ignored_rows};

    #[test]
    fn ignored_rows_badge_files_and_wholly_ignored_folders() {
        use osier_git::files::IgnoreRule;
        let rule = |source: &str, line: u32, pattern: &str| IgnoreRule {
            source: source.into(),
            line,
            pattern: pattern.into(),
        };
        let ignored = crate::project::IgnoredFiles {
            files: vec![
                "build/x.o".into(),
                "build/gen/y.log".into(),
                "site/b.tmp".into(),
                "odd.bin".into(),
            ],
            rules: [
                ("build/x.o".to_string(), rule(".gitignore", 2, "/build")),
                (
                    "build/gen/y.log".into(),
                    rule("/etc/osier/ignore", 1, "*.log"),
                ),
                ("site/b.tmp".into(), rule("site/.gitignore", 1, "*.tmp")),
            ]
            .into(),
        };
        let mut paths: Vec<String> = ["a.txt", "site/keep.txt"]
            .into_iter()
            .map(String::from)
            .chain(ignored.files.iter().cloned())
            .collect();
        paths.sort();
        let rows = ignored_rows(&paths, &ignored);
        let row = |sources: &[&str], tooltip: &str| IgnoredRow {
            sources: sources.iter().map(|s| s.to_string()).collect(),
            tooltip: tooltip.into(),
        };
        assert_eq!(
            rows.get("site/b.tmp"),
            Some(&row(
                &["site/.gitignore"],
                "Ignored by *.tmp in site/.gitignore (line 1)"
            ))
        );
        // A file git ignores without saying why still shows as ignored.
        assert_eq!(rows.get("odd.bin"), Some(&row(&[], "Ignored")));
        // Folders: only when every file inside is ignored, with each rule file involved.
        assert_eq!(
            rows.get("build"),
            Some(&row(
                &[".gitignore", "/etc/osier/ignore"],
                "Everything inside is ignored (rules in .gitignore, /etc/osier/ignore)"
            ))
        );
        assert!(rows.contains_key("build/gen"));
        assert!(!rows.contains_key("site"), "site/keep.txt is not ignored");
        assert!(!rows.contains_key("a.txt"));
    }

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
