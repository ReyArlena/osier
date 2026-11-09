//! The three-way merge tool in the diff pane: shown for conflicted files when the diff mode is
//! `three-way`. One [`MergeView`] per conflicted file, kept for the session so moving between
//! files keeps the progress made in each; dropped once the file is no longer conflicted.

use std::collections::HashMap;
use std::path::PathBuf;

use gpui_kit::*;
use osier_git::Repository;
use osier_git::ops::integrate;

use crate::app::Osier;
use crate::merge_view::{MergeEvent, MergeHost, MergeInput, MergeView};
use crate::project::{Area, Selection};

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
    /// The merge tool fills the window: sidebar and centre column hidden.
    pub(crate) expanded: bool,
}

impl std::fmt::Debug for Merges {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Merges")
            .field("views", &self.views.len())
            .field("load", &self.load)
            .field("expanded", &self.expanded)
            .finish()
    }
}

impl Osier {
    /// The conflicted file the diff pane shows with the merge tool, if it does.
    pub(crate) fn three_way_target(&self) -> Option<MergeKey> {
        if self.settings.diff.mode != osier_config::settings::DiffMode::ThreeWay
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

    /// The merge tool fills the window right now.
    pub(crate) fn merge_fills_window(&self) -> bool {
        self.merges.expanded && self.three_way_target().is_some()
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
                    let view = cx.new(|cx| MergeView::new(input, MergeHost::App, window, cx));
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
        let expanded = self.merges.expanded;
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

    pub(crate) fn toggle_merge_expanded(&mut self, cx: &mut Context<Self>) {
        self.merges.expanded = !self.merges.expanded;
        cx.notify();
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
                if self.merges.expanded {
                    let next = self.conflicted_paths().into_iter().find(|p| *p != path);
                    match next {
                        Some(next) => self.select_file(next, Area::Conflicted, cx),
                        None => self.merges.expanded = false,
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
            MergeEvent::ToggleExpand => self.toggle_merge_expanded(cx),
            MergeEvent::PreviousFile => self.step_conflicted_file(-1, cx),
            MergeEvent::NextFile => self.step_conflicted_file(1, cx),
            MergeEvent::Cancel => {}
        }
    }
}
