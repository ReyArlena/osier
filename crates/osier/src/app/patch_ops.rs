//! Patches from the UI: copy, save to the patch clipboard, export, shelve, and apply with a
//! dry run first.

use std::rc::Rc;

use gpui_kit::*;
use osier_config::history::HistoryEntry;
use osier_config::patches::{self, StoredPatch};
use osier_git::ops::discard;
use osier_git::ops::patches::{self as git_patches, ApplyOptions, ApplyReport, PatchSource};

use crate::app::ops::describe;
use crate::app::{Osier, Tab};
use crate::notices::{Notice, WindowNoticeExt as _};
use crate::project::{Area, Selection};

type After = Rc<dyn Fn(&mut Osier, &mut Window, &mut Context<Osier>)>;

/// Number of `diff --git` headers in a patch text.
pub(crate) fn count_files(text: &str) -> usize {
    text.lines()
        .filter(|l| l.starts_with("diff --git "))
        .count()
}

impl Osier {
    /// Re-read the patch clipboard folder (one file read per patch) in the background.
    pub(crate) fn reload_patches(&mut self, cx: &mut Context<Self>) {
        let paths = self.paths.clone();
        self.patches_load += 1;
        let load = self.patches_load;
        self.in_background_cx(
            crate::background::READING_PATCHES,
            move || patches::list(&paths),
            move |this, listed, cx| {
                // A later reload (another save or delete) wins.
                if this.patches_load != load {
                    return;
                }
                this.patches = listed.unwrap_or_else(|error| {
                    tracing::warn!(%error, "could not list patches");
                    Vec::new()
                });
                cx.notify();
            },
            cx,
        );
    }

    /// Read a stored patch's text in the background, then `then` with it. A read error is
    /// shown as a toast.
    fn read_stored_then(
        &mut self,
        patch: StoredPatch,
        then: impl FnOnce(&mut Osier, String, &mut Window, &mut Context<Osier>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.in_background_user(
            crate::background::READING_PATCHES,
            move || patch.text(),
            move |this, text, window, cx| match text {
                Ok(text) => then(this, text, window, cx),
                Err(e) => window.push_notice(
                    Notice::error(e.to_string()).title("Could not read patch"),
                    cx,
                ),
            },
            window,
            cx,
        );
    }

    /// Everything in the working tree, tracked and untracked. `None` when clean.
    pub(crate) fn all_changes_source(&self) -> Option<PatchSource> {
        let status = &self.active_project()?.snapshot.as_ref()?.status;
        if status.is_clean() {
            return None;
        }
        Some(PatchSource::Worktree {
            paths: Vec::new(),
            untracked: status.untracked().map(|e| e.path.clone()).collect(),
        })
    }

    /// One file from the Changes tab.
    pub(crate) fn file_source(path: &str, area: Area) -> PatchSource {
        match area {
            Area::Untracked => PatchSource::Worktree {
                paths: vec![path.to_string()],
                untracked: vec![path.to_string()],
            },
            // All of a changed file's changes against HEAD, staged or not.
            Area::Changed => PatchSource::Worktree {
                paths: vec![path.to_string()],
                untracked: Vec::new(),
            },
            Area::Conflicted => PatchSource::Unstaged {
                paths: vec![path.to_string()],
            },
        }
    }

    pub(crate) fn commit_subject(&self, hash: &str) -> Option<String> {
        self.active_project()?
            .snapshot
            .as_ref()?
            .commits
            .iter()
            .find(|c| c.hash == hash)
            .map(|c| c.subject.clone())
    }

    /// Build the patch text off the UI thread, then hand it to `then`.
    fn create_patch_then(
        &mut self,
        source: PatchSource,
        then: impl FnOnce(&mut Osier, String, &mut Window, &mut Context<Osier>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.active_project() else {
            return;
        };
        let repo = project.repo.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move { git_patches::create(&repo, &source) })
                .await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(text) if text.trim().is_empty() => {
                    window.push_notice(Notice::warning("Nothing to put in a patch"), cx);
                }
                Ok(text) => then(this, text, window, cx),
                Err(error) => {
                    window.push_notice(
                        Notice::error(describe(&error)).title("Could not create patch"),
                        cx,
                    );
                }
            });
        })
        .detach();
    }

    pub(crate) fn copy_patch(
        &mut self,
        source: PatchSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.create_patch_then(
            source,
            |_, text, window, cx| {
                let n = count_files(&text);
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                window.push_notice(
                    Notice::success(format!("Copied patch ({n} {})", plural(n, "file"))),
                    cx,
                );
            },
            window,
            cx,
        );
    }

    pub(crate) fn save_patch_prompt(
        &mut self,
        source: PatchSource,
        default_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prompt_text(
            "Save to Patches",
            "Patch name",
            default_name,
            "Save",
            Vec::new(),
            move |this, name, window, cx| this.save_patch(source.clone(), name, window, cx),
            window,
            cx,
        );
    }

    pub(crate) fn save_patch(
        &mut self,
        source: PatchSource,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.create_patch_then(
            source,
            move |this, text, window, cx| {
                this.store_patch_then(
                    name.clone(),
                    text,
                    |this, _, _, cx| this.set_tab(Tab::Patches, cx),
                    window,
                    cx,
                );
            },
            window,
            cx,
        );
    }

    /// Write a patch into the patch clipboard and refresh the list, then `then` with it. The
    /// file list (`git apply --numstat`) and the write run in the background; a failure is
    /// shown as a toast and `then` is not called.
    fn store_patch_then(
        &mut self,
        name: String,
        text: String,
        then: impl FnOnce(&mut Osier, StoredPatch, &mut Window, &mut Context<Osier>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let project = self.active_project();
        let repo = project.map(|p| p.repo.clone());
        let repo_path = project.map(|p| p.path.clone());
        let paths = self.paths.clone();
        self.in_background_user(
            crate::background::SAVING_PATCH,
            move || {
                let files: Vec<String> = repo
                    .as_ref()
                    .and_then(|repo| git_patches::inspect(repo, &text).ok())
                    .map(|files| files.into_iter().map(|f| f.path).collect())
                    .unwrap_or_default();
                patches::save(&paths, &name, repo_path.as_deref(), files, &text)
            },
            move |this, saved, window, cx| match saved {
                Ok(stored) => {
                    this.reload_patches(cx);
                    window.push_notice(
                        Notice::success(format!("Saved patch \"{}\"", stored.meta.name)),
                        cx,
                    );
                    then(this, stored, window, cx);
                }
                Err(error) => window.push_notice(
                    Notice::error(error.to_string()).title("Could not save patch"),
                    cx,
                ),
            },
            window,
            cx,
        );
    }

    pub(crate) fn export_patch(
        &mut self,
        source: PatchSource,
        suggested: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.create_patch_then(
            source,
            move |this, text, window, cx| this.write_patch_file(text, suggested, window, cx),
            window,
            cx,
        );
    }

    fn write_patch_file(
        &mut self,
        text: String,
        suggested: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dir = self
            .active_project()
            .map(|p| p.path.clone())
            .or_else(|| std::env::var_os("HOME").map(Into::into))
            .unwrap_or_default();
        let rx = cx.prompt_for_new_path(&dir, Some(&suggested));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = rx.await else {
                return;
            };
            let target = path.clone();
            let result = cx
                .background_spawn(async move { std::fs::write(&target, text) })
                .await;
            let _ = this.update_in(cx, |_, window, cx| match result {
                Ok(()) => window.push_notice(
                    Notice::success(format!("Wrote {}", crate::format::tilde(&path))),
                    cx,
                ),
                Err(e) => window.push_notice(
                    Notice::error(e.to_string()).title("Could not write patch"),
                    cx,
                ),
            });
        })
        .detach();
    }

    // ----- shelve -----

    /// Ask for a name, then shelve `files` (empty = every change).
    pub(crate) fn shelve_prompt(
        &mut self,
        files: Vec<(String, Area)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let default = match files.as_slice() {
            [(path, _)] => format!("Shelved {path}"),
            _ => format!(
                "Shelved {}",
                crate::format::iso_date(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0)
                )
            ),
        };
        self.prompt_text(
            "Shelve changes",
            "Shelf name",
            default,
            "Shelve",
            Vec::new(),
            move |this, name, window, cx| this.shelve_named(name, files.clone(), window, cx),
            window,
            cx,
        );
    }

    /// Save the changes as a patch, then discard them (tracked files restored from HEAD,
    /// untracked files moved to the trash).
    pub(crate) fn shelve_named(
        &mut self,
        name: String,
        files: Vec<(String, Area)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(status) = self
            .active_project()
            .and_then(|p| p.snapshot.as_ref())
            .map(|s| s.status.clone())
        else {
            return;
        };
        let (tracked, untracked): (Vec<String>, Vec<String>) = if files.is_empty() {
            (
                status
                    .entries
                    .iter()
                    .filter(|e| !e.is_untracked())
                    .map(|e| e.path.clone())
                    .collect(),
                status.untracked().map(|e| e.path.clone()).collect(),
            )
        } else {
            let mut t = Vec::new();
            let mut u = Vec::new();
            for (path, area) in &files {
                if *area == Area::Untracked {
                    u.push(path.clone());
                } else {
                    t.push(path.clone());
                }
            }
            (t, u)
        };
        if tracked.is_empty() && untracked.is_empty() {
            window.push_notice(Notice::warning("Nothing to shelve"), cx);
            return;
        }
        let source = PatchSource::Worktree {
            paths: tracked.clone(),
            untracked: untracked.clone(),
        };
        self.create_patch_then(
            source,
            move |this, text, window, cx| {
                // Only once the patch is safely stored are the changes discarded.
                let (t, u, name) = (tracked.clone(), untracked.clone(), name.clone());
                this.store_patch_then(
                    name.clone(),
                    text,
                    move |this, _, window, cx| {
                        this.run_op(
                            format!("Shelved \"{name}\""),
                            move |repo| {
                                discard::discard_all(repo, &t)?;
                                discard::trash_untracked(repo, &u)?;
                                Ok(())
                            },
                            window,
                            cx,
                        );
                        this.set_tab(Tab::Patches, cx);
                    },
                    window,
                    cx,
                );
            },
            window,
            cx,
        );
    }

    // ----- apply -----

    /// Dry run, then confirm, then apply. On a failed dry run offer a three-way merge.
    pub(crate) fn apply_patch_text(
        &mut self,
        text: String,
        label: String,
        after: Option<After>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.active_project() else {
            window.push_notice(Notice::warning("Open a repository to apply a patch"), cx);
            return;
        };
        let repo = project.repo.clone();
        let probe_text = text.clone();
        cx.spawn_in(window, async move |this, cx| {
            let probe = cx
                .background_spawn(async move {
                    let files = git_patches::inspect(&repo, &probe_text)?;
                    let report = git_patches::check(&repo, &probe_text, ApplyOptions::default())?;
                    Ok::<_, osier_git::GitError>((files, report))
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| match probe {
                Err(error) => window.push_notice(
                    Notice::error(describe(&error)).title("Not a patch git can read"),
                    cx,
                ),
                Ok((files, _)) if files.is_empty() => {
                    window.push_notice(Notice::warning("The patch is empty"), cx)
                }
                Ok((files, report)) if report.ok => {
                    let list = file_list(files.iter().map(|f| f.path.as_str()));
                    let n = files.len();
                    let text = text.clone();
                    let label2 = label.clone();
                    let after = after.clone();
                    this.confirm(
                        format!("Apply {label}?"),
                        format!("{n} {} will change:\n{list}", plural(n, "file")),
                        "Apply",
                        false,
                        move |this, window, cx| {
                            this.run_apply(
                                text.clone(),
                                ApplyOptions::default(),
                                label2.clone(),
                                after.clone(),
                                window,
                                cx,
                            )
                        },
                        window,
                        cx,
                    );
                }
                Ok((_, report)) if !git_patches::supports_three_way(&text) => {
                    // PhpStorm and plain `diff -u` patches carry no blob ids, so git cannot
                    // fall back to a three-way merge. Say so instead of offering it.
                    let rejected = file_list(report.rejected.iter().map(String::as_str));
                    window.push_notice(
                        Notice::error(format!(
                                "Rejected:\n{rejected}\n\n{}\n\nThis patch has no blob ids (PhpStorm or plain diff), so a three-way merge is not possible. Bring the files closer to the patch's base and try again.",
                                report.message
                            )).title(format!("{label} does not apply cleanly")),
                        cx,
                    );
                }
                Ok((_, report)) => {
                    let rejected = file_list(report.rejected.iter().map(String::as_str));
                    let text = text.clone();
                    let label2 = label.clone();
                    let after = after.clone();
                    this.confirm(
                        format!("{label} does not apply cleanly"),
                        format!(
                            "Rejected:\n{rejected}\n\n{}\n\nTry a three-way merge? Conflicts are left in the files as markers for you to resolve.",
                            report.message
                        ),
                        "Try 3-way merge",
                        true,
                        move |this, window, cx| {
                            this.run_apply(
                                text.clone(),
                                ApplyOptions {
                                    three_way: true,
                                    ..Default::default()
                                },
                                label2.clone(),
                                after.clone(),
                                window,
                                cx,
                            )
                        },
                        window,
                        cx,
                    );
                }
            });
        })
        .detach();
    }

    pub(crate) fn run_apply(
        &mut self,
        text: String,
        options: ApplyOptions,
        label: String,
        after: Option<After>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.active_project() else {
            return;
        };
        let repo = project.repo.clone();
        let path = project.path.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move { git_patches::apply(&repo, &text, options) })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                let summary = format!("Applied {label}");
                let mut entry = HistoryEntry::now(&path, summary.clone());
                let changed = match &result {
                    Ok(ApplyReport { ok: true, .. }) => {
                        window.push_notice(Notice::success(summary), cx);
                        true
                    }
                    Ok(report) if !report.conflicts.is_empty() => {
                        let n = report.conflicts.len();
                        entry.error = Some(format!("conflicts in {n} {}", plural(n, "file")));
                        window.push_notice(
                            Notice::warning(file_list(report.conflicts.iter().map(String::as_str)))
                                .title(format!(
                                    "{summary} with conflicts in {n} {}",
                                    plural(n, "file")
                                )),
                            cx,
                        );
                        true
                    }
                    Ok(report) => {
                        entry.ok = false;
                        entry.error = Some(report.message.clone());
                        window.push_notice(
                            Notice::error(report.message.clone())
                                .title(format!("{label} was not applied")),
                            cx,
                        );
                        false
                    }
                    Err(error) => {
                        entry.ok = false;
                        entry.error = Some(describe(error));
                        window.push_notice(
                            Notice::error(describe(error))
                                .title(format!("{label} was not applied")),
                            cx,
                        );
                        false
                    }
                };
                this.record_history(entry, cx);
                if let Some(ix) = this.projects.iter().position(|p| p.path == path) {
                    this.refresh_project(ix, cx);
                }
                if changed {
                    this.set_tab(Tab::Changes, cx);
                    if let Some(after) = after {
                        after(this, window, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn apply_patch_from_clipboard(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .unwrap_or_default();
        if text.trim().is_empty() {
            window.push_notice(Notice::warning("The clipboard has no text"), cx);
            return;
        }
        self.apply_patch_text(text, "clipboard patch".into(), None, window, cx);
    }

    pub(crate) fn apply_patch_from_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Apply patch".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let source = path.clone();
            let text = cx
                .background_spawn(async move { std::fs::read_to_string(&source) })
                .await;
            let _ = this.update_in(cx, |this, window, cx| match text {
                Ok(text) => {
                    let label = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "patch".into());
                    this.apply_patch_text(text, label, None, window, cx);
                }
                Err(e) => window.push_notice(
                    Notice::error(e.to_string()).title("Could not read patch"),
                    cx,
                ),
            });
        })
        .detach();
    }

    // ----- stored patches -----

    pub(crate) fn stored_patch(&self, id: &str) -> Option<&StoredPatch> {
        self.patches.iter().find(|p| p.id == id)
    }

    /// Apply a stored patch; `unshelve` deletes it once the working tree changed.
    pub(crate) fn apply_stored(
        &mut self,
        id: String,
        unshelve: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(patch) = self.stored_patch(&id) else {
            return;
        };
        let label = format!("\"{}\"", patch.meta.name);
        let patch = patch.clone();
        self.read_stored_then(
            patch,
            move |this, text, window, cx| {
                let after: Option<After> = unshelve.then(|| {
                    let id = id.clone();
                    Rc::new(
                        move |this: &mut Osier, window: &mut Window, cx: &mut Context<Osier>| {
                            this.delete_stored(id.clone(), window, cx)
                        },
                    ) as After
                });
                this.apply_patch_text(text, label, after, window, cx);
            },
            window,
            cx,
        );
    }

    pub(crate) fn select_patch(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(patch) = self.stored_patch(&id) else {
            return;
        };
        let selection = Selection::Patch {
            id: id.clone(),
            name: patch.meta.name.clone(),
            file: patch.path.clone(),
        };
        self.diff_force_large = false;
        self.editor = None;
        self.patch_selected = Some(id);
        if let Some(p) = self.active_project_mut() {
            p.selection = selection;
        }
        self.reload_selection(cx);
        cx.notify();
    }

    pub(crate) fn confirm_delete_stored(
        &mut self,
        id: String,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm(
            "Delete patch?",
            format!("\"{name}\" is removed from the patch clipboard. This cannot be undone."),
            "Delete",
            true,
            move |this, window, cx| this.delete_stored(id.clone(), window, cx),
            window,
            cx,
        );
    }

    pub(crate) fn delete_stored(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = patches::delete(&self.paths, &id) {
            window.push_notice(
                Notice::error(error.to_string()).title("Could not delete patch"),
                cx,
            );
            return;
        }
        if self.patch_selected.as_deref() == Some(id.as_str()) {
            self.patch_selected = None;
            if let Some(p) = self.active_project_mut()
                && matches!(&p.selection, Selection::Patch { id: s, .. } if s == &id)
            {
                p.selection = Selection::None;
                self.reload_selection(cx);
            }
        }
        self.reload_patches(cx);
    }

    pub(crate) fn rename_stored_prompt(
        &mut self,
        id: String,
        current: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prompt_text(
            "Rename patch",
            "Patch name",
            current,
            "Rename",
            Vec::new(),
            move |this, name, window, cx| {
                if name.trim().is_empty() {
                    return;
                }
                if let Err(error) = patches::rename(&this.paths, &id, &name) {
                    window.push_notice(
                        Notice::error(error.to_string()).title("Could not rename patch"),
                        cx,
                    );
                }
                this.reload_patches(cx);
            },
            window,
            cx,
        );
    }

    pub(crate) fn copy_stored(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(patch) = self.stored_patch(&id) else {
            return;
        };
        let patch = patch.clone();
        self.read_stored_then(
            patch,
            |_, text, window, cx| {
                let n = count_files(&text);
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                window.push_notice(
                    Notice::success(format!("Copied patch ({n} {})", plural(n, "file"))),
                    cx,
                );
            },
            window,
            cx,
        );
    }

    pub(crate) fn export_stored(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(patch) = self.stored_patch(&id) else {
            return;
        };
        let suggested = format!("{}.patch", patches::slug(&patch.meta.name));
        let patch = patch.clone();
        self.read_stored_then(
            patch,
            move |this, text, window, cx| this.write_patch_file(text, suggested, window, cx),
            window,
            cx,
        );
    }

    pub(crate) fn open_patches_dir(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dir = self.paths.patches_dir();
        let create = dir.clone();
        self.in_background_user(
            crate::background::OPENING_FOLDER,
            move || {
                if let Err(error) = std::fs::create_dir_all(&create) {
                    tracing::warn!(%error, "could not create the patches directory");
                }
            },
            move |this, (), window, cx| this.open_externally(&dir, window, cx),
            window,
            cx,
        );
    }
}

fn plural(n: usize, word: &str) -> String {
    if n == 1 {
        word.to_string()
    } else {
        format!("{word}s")
    }
}

/// Bulleted list capped at ten entries.
fn file_list<'a>(items: impl Iterator<Item = &'a str>) -> String {
    let items: Vec<&str> = items.collect();
    let mut out: Vec<String> = items.iter().take(10).map(|p| format!("  {p}")).collect();
    if items.len() > 10 {
        out.push(format!("  … and {} more", items.len() - 10));
    }
    out.join("\n")
}
