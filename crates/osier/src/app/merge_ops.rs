//! The three-way merge tool in the diff pane: shown for conflicted files when the diff mode is
//! `three-way`. One [`MergeView`] per conflicted file, kept for the session so moving between
//! files keeps the progress made in each; dropped once the file is no longer conflicted.

use std::collections::HashMap;
use std::path::PathBuf;

use gpui_kit::*;
use osier_git::Repository;
use osier_git::ops::integrate;

use crate::app::Osier;
use crate::merge_view::{MergeEvent, MergeHost, MergeInput, MergeOptions, MergeView};
use crate::project::{Area, Selection};
use osier_config::settings::DiffWhitespace;
use osier_git::merge3::Whitespace;

/// Project root and file path.
pub(crate) type MergeKey = (PathBuf, String);

pub(crate) struct MergeSlot {
    pub(crate) view: Entity<MergeView>,
    _events: Subscription,
}

/// Reading the three versions of the file the pane is about to show.
#[derive(Debug)]
pub(crate) enum MergeLoad {
    Loading(MergeKey),
    Ready(MergeKey, MergeInput),
    Failed(MergeKey, String),
}

/// Everything the merge tool needs about a conflicted file. Blocking (runs git).
pub(crate) fn load_merge_input(repo: &Repository, path: &str) -> Result<MergeInput, String> {
    let versions = integrate::conflict_versions(repo, path).map_err(|e| e.to_string())?;
    let sides = integrate::conflict_sides(repo);
    let (left, right) = sides.pick(&versions);
    let binary = |t: &Option<String>| t.as_ref().is_some_and(|t| t.contains('\0'));
    if binary(&left) || binary(&right) || binary(&versions.base) {
        return Err(
            "This is a binary file. Pick Yours or Theirs for it in the Conflicts list.".into(),
        );
    }
    let label = |label: String, text: &Option<String>| match text {
        Some(_) => label,
        None => format!("{label}, deleted"),
    };
    Ok(MergeInput {
        path: path.to_string(),
        base: versions.base,
        left_label: label(sides.left_label, &left),
        right_label: label(sides.right_label, &right),
        left: left.unwrap_or_default(),
        right: right.unwrap_or_default(),
    })
}

#[derive(Default)]
pub(crate) struct Merges {
    pub(crate) views: HashMap<MergeKey, MergeSlot>,
    pub(crate) load: Option<MergeLoad>,
    pub(crate) task: Option<Task<()>>,
}

impl std::fmt::Debug for Merges {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Merges")
            .field("views", &self.views.len())
            .field("load", &self.load)
            .finish()
    }
}

fn to_merge_whitespace(w: DiffWhitespace) -> Whitespace {
    match w {
        DiffWhitespace::None => Whitespace::Exact,
        DiffWhitespace::Trim => Whitespace::Trim,
        DiffWhitespace::All => Whitespace::Ignore,
    }
}

fn from_merge_whitespace(w: Whitespace) -> DiffWhitespace {
    match w {
        Whitespace::Exact => DiffWhitespace::None,
        Whitespace::Trim => DiffWhitespace::Trim,
        Whitespace::Ignore => DiffWhitespace::All,
    }
}

impl Osier {
    /// The diff view settings (gear menus, wrap button), shared by every layout.
    pub(crate) fn diff_options(&self) -> MergeOptions {
        let d = &self.settings.diff;
        MergeOptions {
            sync_scroll: d.sync_scroll,
            whitespace: to_merge_whitespace(d.whitespace),
            highlight_words: d.word_highlights,
            soft_wrap: d.soft_wrap,
        }
    }

    /// Change the shared diff view settings: saved, applied to the diff and every merge
    /// view, and the diff reloaded when the whitespace rule changed.
    pub(crate) fn set_diff_options(
        &mut self,
        o: MergeOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let whitespace = from_merge_whitespace(o.whitespace);
        let reload = whitespace != self.settings.diff.whitespace;
        let d = &mut self.settings.diff;
        d.sync_scroll = o.sync_scroll;
        d.whitespace = whitespace;
        d.word_highlights = o.highlight_words;
        d.soft_wrap = o.soft_wrap;
        self.save_settings();
        self.apply_word_highlights(cx);
        let views: Vec<Entity<MergeView>> =
            self.merges.views.values().map(|s| s.view.clone()).collect();
        for view in views {
            view.update(cx, |v, cx| {
                if v.options() != o {
                    v.set_options(o, window, cx)
                }
            });
        }
        if reload {
            self.reload_selection(cx);
        }
        cx.notify();
    }

    /// Word highlights in the two-way diff follow the shared setting.
    pub(crate) fn apply_word_highlights(&mut self, cx: &mut Context<Self>) {
        let unit = self
            .settings
            .diff
            .word_highlights
            .then_some(gpui_kit::component::diff::DiffInlineUnit::Word);
        self.diff.update(cx, |s, cx| {
            if s.inline_unit() != unit {
                s.set_inline_unit(unit, cx)
            }
        });
    }

    /// The conflicted file the diff pane shows with the merge tool, if it does.
    pub(crate) fn three_way_target(&self) -> Option<MergeKey> {
        if self.settings.diff.conflicts != osier_config::settings::ConflictView::ThreeWay
            || self.blame.is_some()
            || self.editor.is_some()
        {
            return None;
        }
        let project = self.active_project()?;
        match &project.selection {
            Selection::File {
                path,
                area: Area::Conflicted,
            } => Some((project.path.clone(), path.clone())),
            _ => None,
        }
    }

    /// Conflicted files of the active project, in list order.
    fn conflicted_paths(&self) -> Vec<String> {
        self.active_project()
            .and_then(|p| p.snapshot.as_ref())
            .map(|s| s.status.conflicted().map(|e| e.path.clone()).collect())
            .unwrap_or_default()
    }

    /// Forget merge views of files that are no longer conflicted.
    fn prune_merges(&mut self) {
        let Some(project) = self.active_project() else {
            return;
        };
        if project.snapshot.is_none() {
            return;
        }
        let root = project.path.clone();
        let conflicted = self.conflicted_paths();
        self.merges
            .views
            .retain(|(r, p), _| *r != root || conflicted.contains(p));
    }

    /// The merge view for the current target, creating it once its versions are read.
    /// `Ok(None)` while reading.
    pub(crate) fn merge_view(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Option<Entity<MergeView>>, String> {
        self.prune_merges();
        let Some(key) = self.three_way_target() else {
            return Ok(None);
        };
        if !self.merges.views.contains_key(&key) {
            match self.merges.load.take() {
                Some(MergeLoad::Ready(k, input)) if k == key => {
                    let options = self.diff_options();
                    let view = cx.new(|cx| {
                        let mut v = MergeView::new(input, MergeHost::App, window, cx);
                        v.set_options(options, window, cx);
                        v
                    });
                    let event_key = key.clone();
                    let events = cx.subscribe_in(&view, window, move |this, _, ev, window, cx| {
                        this.on_merge_event(event_key.clone(), ev, window, cx)
                    });
                    self.merges.views.insert(
                        key.clone(),
                        MergeSlot {
                            view,
                            _events: events,
                        },
                    );
                }
                Some(MergeLoad::Failed(k, error)) if k == key => {
                    self.merges.load = Some(MergeLoad::Failed(k, error.clone()));
                    return Err(error);
                }
                Some(MergeLoad::Loading(k)) if k == key => {
                    self.merges.load = Some(MergeLoad::Loading(k));
                    return Ok(None);
                }
                _ => {
                    self.start_merge_load(key, cx);
                    return Ok(None);
                }
            }
        }
        let conflicted = self.conflicted_paths();
        let position = conflicted
            .iter()
            .position(|p| *p == key.1)
            .map(|i| (i + 1, conflicted.len()));
        let expanded = self.diff_expanded;
        let view = self.merges.views.get(&key).map(|s| s.view.clone());
        if let Some(view) = &view {
            view.update(cx, |v, cx| {
                if v.expanded != expanded || v.file_position != position {
                    v.expanded = expanded;
                    v.file_position = position;
                    cx.notify();
                }
            });
        }
        Ok(view)
    }

    fn start_merge_load(&mut self, key: MergeKey, cx: &mut Context<Self>) {
        let Some(repo) = self
            .projects
            .iter()
            .find(|p| p.path == key.0)
            .map(|p| p.repo.clone())
        else {
            return;
        };
        self.merges.load = Some(MergeLoad::Loading(key.clone()));
        let path = key.1.clone();
        self.merges.task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { load_merge_input(&repo, &path) })
                .await;
            let _ = this.update(cx, |this, cx| {
                // Only if nothing else started meanwhile.
                if matches!(&this.merges.load, Some(MergeLoad::Loading(k)) if *k == key) {
                    this.merges.load = Some(match result {
                        Ok(input) => MergeLoad::Ready(key, input),
                        Err(error) => MergeLoad::Failed(key, error),
                    });
                }
                cx.notify();
            });
        }));
    }

    /// Move to the conflicted file `delta` places away (wrapping).
    pub(crate) fn step_conflicted_file(&mut self, delta: isize, cx: &mut Context<Self>) {
        let files = self.conflicted_paths();
        if files.is_empty() {
            return;
        }
        let current = match self.active_project().map(|p| &p.selection) {
            Some(Selection::File { path, .. }) => files.iter().position(|p| p == path),
            _ => None,
        };
        let next = match current {
            Some(i) => (i as isize + delta).rem_euclid(files.len() as isize) as usize,
            None => 0,
        };
        self.select_file(files[next].clone(), Area::Conflicted, cx);
    }

    fn on_merge_event(
        &mut self,
        key: MergeKey,
        event: &MergeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = key.1.clone();
        match event {
            MergeEvent::Apply(text) => {
                let (p, text) = (path.clone(), text.clone());
                self.run_op(
                    format!("Resolved {path}"),
                    move |repo| {
                        integrate::write_resolution(repo, &p, &text)?;
                        integrate::mark_resolved(repo, std::slice::from_ref(&p))
                    },
                    window,
                    cx,
                );
                self.merges.views.remove(&key);
                // Expanded, go straight on to the next conflicted file.
                if self.diff_expanded {
                    let next = self.conflicted_paths().into_iter().find(|p| *p != path);
                    match next {
                        Some(next) => self.select_file(next, Area::Conflicted, cx),
                        None => self.diff_expanded = false,
                    }
                }
                cx.notify();
            }
            MergeEvent::Save(text) => {
                let (p, text) = (path.clone(), text.clone());
                self.run_op(
                    format!("Saved {path}"),
                    move |repo| integrate::write_resolution(repo, &p, &text),
                    window,
                    cx,
                );
            }
            MergeEvent::Options(o) => self.set_diff_options(*o, window, cx),
            MergeEvent::PreviousFile => self.step_conflicted_file(-1, cx),
            MergeEvent::NextFile => self.step_conflicted_file(1, cx),
            MergeEvent::Cancel => {}
        }
    }
}
