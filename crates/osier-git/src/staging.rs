//! Staging for a one-list Changes view: a file's changes shown once (HEAD against the working
//! tree), each changed line marked staged or not, and stage / unstage / revert for any set of
//! those lines.
//!
//! The index sits between HEAD and the working tree, so two more diffs say where each changed
//! line of the shown diff (W, HEAD -> working tree) is, with no line-number translation:
//! - a line W removes (HEAD line h) is staged when HEAD -> index (S) removes h too;
//! - a line W adds (working-tree line w) is unstaged when index -> working tree (U) adds w.
//!
//! Staging builds a patch from U restricted to the chosen lines and applies it to the index;
//! unstaging builds one from S and applies it to the index in reverse; reverting unstages and
//! then discards from the working tree. Hunks are applied from the bottom of the file up, so a
//! hunk applied first never moves the lines of one still to come.

use std::collections::HashSet;

use crate::diff::{DiffFile, DiffOptions, DiffTarget, Line, LineKind, parse, patch_text};
use crate::error::Result;
use crate::ops::patch::lines_patch;
use crate::ops::stage::{ApplyTarget, apply_patch};
use crate::repo::Repository;
use crate::status::{EntryKind, StatusEntry};

/// How much of a file, hunk or set of lines is staged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Check {
    /// All of it (a ticked checkbox).
    Staged,
    /// None of it.
    Unstaged,
    /// Some of it (a dash).
    Partial,
}

impl Check {
    fn of(staged: usize, total: usize) -> Check {
        if total > 0 && staged == total {
            Check::Staged
        } else if staged == 0 {
            Check::Unstaged
        } else {
            Check::Partial
        }
    }
}

/// The check for a tracked status entry, from what git reports for its index and working tree.
/// `None` for untracked, ignored and conflicted entries (they are not ticked this way).
pub fn file_check(entry: &StatusEntry) -> Option<Check> {
    if !matches!(entry.kind, EntryKind::Changed | EntryKind::RenamedOrCopied) {
        return None;
    }
    Some(match (entry.index != '.', entry.worktree != '.') {
        (true, false) => Check::Staged,
        (false, _) => Check::Unstaged,
        (true, true) => Check::Partial,
    })
}

/// A file's changes as one diff (HEAD -> working tree) with each changed line marked.
#[derive(Debug, Clone)]
pub struct FileStaging {
    /// What is shown: HEAD (or the empty tree on an unborn branch) against the working tree.
    pub shown: DiffFile,
    /// `shown` as unified diff text, for viewers that parse patches themselves.
    pub shown_text: String,
    /// HEAD -> index; `None` when nothing is staged.
    staged: Option<DiffFile>,
    /// Index -> working tree; `None` when nothing is unstaged.
    unstaged: Option<DiffFile>,
    /// Per hunk of `shown`, per line: whether a changed line is staged (`None` for context).
    marks: Vec<Vec<Option<bool>>>,
    /// Lines only the index has (staged, then changed again in the working tree): neither
    /// side of the shown diff has them. As U removes them: (working-tree line they sit
    /// before, their index line).
    index_only_u: Vec<(u32, u32)>,
    /// The same lines as S adds them: (HEAD line they sit before, their index line).
    index_only_s: Vec<(u32, u32)>,
}

impl FileStaging {
    /// Whether line `line` of hunk `hunk` is staged; `None` for context lines.
    pub fn line_staged(&self, hunk: usize, line: usize) -> Option<bool> {
        self.marks.get(hunk)?.get(line).copied().flatten()
    }

    /// How much of hunk `hunk` is staged. A hunk with index-only lines in its region is
    /// partly staged whatever its shown lines say: the index differs from both sides there.
    pub fn hunk_check(&self, hunk: usize) -> Check {
        if self.hunk_has_index_only(hunk) {
            return Check::Partial;
        }
        let marks = self.marks.get(hunk).map(Vec::as_slice).unwrap_or(&[]);
        let changed = marks.iter().flatten().count();
        let staged = marks.iter().flatten().filter(|s| **s).count();
        Check::of(staged, changed)
    }

    fn hunk_has_index_only(&self, hunk: usize) -> bool {
        self.index_only_u
            .iter()
            .any(|(w, _)| self.hunk_at_new(*w) == Some(hunk))
            || self
                .index_only_s
                .iter()
                .any(|(h, _)| self.hunk_at_old(*h) == Some(hunk))
    }

    /// Whether the index holds lines the shown diff cannot show (see `index_only_u`). Only
    /// the file's own checkbox reaches those outside every hunk.
    pub fn has_index_only(&self) -> bool {
        !self.index_only_u.is_empty()
    }

    /// The hunk of the shown diff whose working-tree lines include, or sit next to, line `w`.
    fn hunk_at_new(&self, w: u32) -> Option<usize> {
        self.shown
            .hunks
            .iter()
            .position(|h| (h.new_start..=h.new_start + h.new_lines).contains(&w))
    }

    /// The same on the HEAD side.
    fn hunk_at_old(&self, h_line: u32) -> Option<usize> {
        self.shown
            .hunks
            .iter()
            .position(|h| (h.old_start..=h.old_start + h.old_lines).contains(&h_line))
    }

    /// How many hunks have something staged, out of all hunks ("1 of 6 changes").
    pub fn hunk_counts(&self) -> (usize, usize) {
        let total = self.shown.hunks.len();
        let some = (0..total)
            .filter(|ix| self.hunk_check(*ix) != Check::Unstaged)
            .count();
        (some, total)
    }

    /// How much of the whole file is staged, from its lines (partly, when the index holds
    /// lines neither side has).
    pub fn check(&self) -> Check {
        if self.has_index_only() {
            return Check::Partial;
        }
        let all: Vec<bool> = self.marks.iter().flatten().flatten().copied().collect();
        Check::of(all.iter().filter(|s| **s).count(), all.len())
    }
}

/// The empty tree's id, for diffing on an unborn branch.
fn empty_tree(repo: &Repository) -> Result<String> {
    let out = repo.runner().run_unchecked(
        Some(repo.workdir()),
        ["hash-object", "-t", "tree", "--stdin"],
        Some(b""),
    )?;
    Ok(out.stdout_str()?.trim().to_string())
}

fn one_file(
    repo: &Repository,
    target: DiffTarget,
    paths: &[&str],
    context: u32,
) -> Result<Option<DiffFile>> {
    Ok(one_file_text(repo, target, paths, context)?.0)
}

/// [`one_file`], with the diff's text.
fn one_file_text(
    repo: &Repository,
    target: DiffTarget,
    paths: &[&str],
    context: u32,
) -> Result<(Option<DiffFile>, String)> {
    let mut options = DiffOptions::new(target);
    options.context_lines = context;
    options.paths = paths.iter().map(|p| p.to_string()).collect();
    // With the path a rename came from, the two are one file whatever git's similarity says.
    options.pair_renames = paths.len() > 1;
    let text = patch_text(repo, &options)?;
    let files = parse(&text)?;
    // A rename lists both paths; the file of interest is the one that ends at `paths[0]`.
    let file = files
        .into_iter()
        .find(|f| f.path == paths[0])
        .filter(|f| !f.hunks.is_empty() || f.binary);
    Ok((file, text))
}

/// What the shown diff compares the working tree with: HEAD, or the empty tree on an unborn
/// branch.
fn head_or_empty(repo: &Repository) -> Result<String> {
    Ok(match repo.head_oid()? {
        Some(_) => "HEAD".to_string(),
        None => empty_tree(repo)?,
    })
}

/// Why git lists a file as changed when its text is the same in HEAD and the working tree, so
/// there is no diff to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unchanged {
    /// Only the file mode differs (git modes, such as `100644` and `100755`).
    Mode { old: String, new: String },
    /// Moved or copied from `from` without an edit.
    Renamed { from: String },
    /// A new file with nothing in it.
    EmptyAdded,
    /// A deleted file that had nothing in it.
    EmptyDeleted,
    /// The working tree is back at HEAD, but the index still holds a staged change.
    StagedOnly,
}

/// The reason [`file_staging`] found nothing to show for `path`. `None` when no reason is
/// found (the file changed again in the meantime, or git no longer lists it).
pub fn unchanged_reason(
    repo: &Repository,
    path: &str,
    orig_path: Option<&str>,
) -> Result<Option<Unchanged>> {
    let mut paths = vec![path];
    paths.extend(orig_path);
    let file_in = |target: DiffTarget| -> Result<Option<DiffFile>> {
        let mut options = DiffOptions::new(target);
        options.paths = paths.iter().map(|p| p.to_string()).collect();
        options.pair_renames = paths.len() > 1;
        Ok(parse(&patch_text(repo, &options)?)?
            .into_iter()
            .find(|f| f.path == path))
    };
    let Some(file) = file_in(DiffTarget::WorktreeTo(head_or_empty(repo)?))? else {
        // HEAD and the working tree agree: git lists the file because the index differs.
        return Ok(file_in(DiffTarget::Staged)?.map(|_| Unchanged::StagedOnly));
    };
    if !file.hunks.is_empty() || file.binary {
        return Ok(None);
    }
    Ok(match file.status {
        crate::diff::FileStatus::Renamed | crate::diff::FileStatus::Copied => {
            file.old_path.map(|from| Unchanged::Renamed { from })
        }
        crate::diff::FileStatus::Added => Some(Unchanged::EmptyAdded),
        crate::diff::FileStatus::Deleted => Some(Unchanged::EmptyDeleted),
        _ => match (file.old_mode, file.new_mode) {
            (Some(old), Some(new)) if old != new => Some(Unchanged::Mode { old, new }),
            _ => None,
        },
    })
}

/// Context lines of the two diffs the patches are built from: their hunks only need to be
/// found by `git apply`, whatever context the shown diff uses.
const PATCH_CONTEXT: u32 = 3;

/// Load `path`'s staging (`orig_path`: where a rename came from; `context`: context lines of
/// the shown diff). `None` when the file has no textual changes against HEAD (only a mode
/// change, or nothing at all).
pub fn file_staging(
    repo: &Repository,
    path: &str,
    orig_path: Option<&str>,
    context: u32,
) -> Result<Option<FileStaging>> {
    let mut paths = vec![path];
    paths.extend(orig_path);
    let base = head_or_empty(repo)?;
    let (Some(shown), shown_text) =
        one_file_text(repo, DiffTarget::WorktreeTo(base), &paths, context)?
    else {
        return Ok(None);
    };
    let staged = one_file(repo, DiffTarget::Staged, &paths, PATCH_CONTEXT)?;
    let unstaged = one_file(repo, DiffTarget::Unstaged, &[path], PATCH_CONTEXT)?;
    let staged_removed: HashSet<u32> =
        changed_lines(staged.as_ref(), LineKind::Removed, |l| l.old_no);
    let unstaged_added: HashSet<u32> =
        changed_lines(unstaged.as_ref(), LineKind::Added, |l| l.new_no);
    let marks = shown
        .hunks
        .iter()
        .map(|h| {
            h.lines
                .iter()
                .map(|l| match l.kind {
                    LineKind::Context => None,
                    LineKind::Removed => {
                        Some(l.old_no.is_some_and(|n| staged_removed.contains(&n)))
                    }
                    LineKind::Added => Some(l.new_no.is_some_and(|n| !unstaged_added.contains(&n))),
                })
                .collect()
        })
        .collect();
    // Index-only lines: U removes them, S adds them. Each side records where they sit.
    let unstaged_removed = changed_lines(unstaged.as_ref(), LineKind::Removed, |l| l.old_no);
    let index_only_u = anchored(unstaged.as_ref(), LineKind::Removed, |i| {
        new_to_old(staged.as_ref(), i).is_none()
    });
    let index_only_s = anchored(staged.as_ref(), LineKind::Added, |i| {
        unstaged_removed.contains(&i)
    });
    Ok(Some(FileStaging {
        shown,
        shown_text,
        staged,
        unstaged,
        marks,
        index_only_u,
        index_only_s,
    }))
}

/// The changed lines of `kind` in `file` whose own number (`old_no` for removed lines,
/// `new_no` for added ones) passes `keep`, each with the line of the other side it sits before.
fn anchored(
    file: Option<&DiffFile>,
    kind: LineKind,
    keep: impl Fn(u32) -> bool,
) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    for h in file.into_iter().flat_map(|f| &f.hunks) {
        // Position on the other side: the next line there, counted as lines go by.
        let mut other = if kind == LineKind::Removed {
            h.new_start
        } else {
            h.old_start
        };
        for l in &h.lines {
            let own = if kind == LineKind::Removed {
                l.old_no
            } else {
                l.new_no
            };
            if l.kind == kind {
                if let Some(n) = own.filter(|n| keep(*n)) {
                    out.push((other, n));
                }
            } else if l.kind == LineKind::Context || l.kind != kind {
                // A line the other side has moves the position on.
                let on_other = if kind == LineKind::Removed {
                    l.new_no
                } else {
                    l.old_no
                };
                if on_other.is_some() {
                    other += 1;
                }
            }
        }
    }
    out
}

fn changed_lines(
    file: Option<&DiffFile>,
    kind: LineKind,
    number: impl Fn(&Line) -> Option<u32>,
) -> HashSet<u32> {
    file.into_iter()
        .flat_map(|f| &f.hunks)
        .flat_map(|h| &h.lines)
        .filter(|l| l.kind == kind)
        .filter_map(number)
        .collect()
}

/// Line `old` of `file`'s old side on its new side; `None` when the diff removes it.
fn old_to_new(file: Option<&DiffFile>, old: u32) -> Option<u32> {
    map_line(file, old, true)
}

/// Line `new` of `file`'s new side on its old side; `None` when the diff adds it.
fn new_to_old(file: Option<&DiffFile>, new: u32) -> Option<u32> {
    map_line(file, new, false)
}

fn map_line(file: Option<&DiffFile>, n: u32, from_old: bool) -> Option<u32> {
    let Some(file) = file else {
        return Some(n);
    };
    let mut delta: i64 = 0;
    for h in &file.hunks {
        let (start, len, other_len) = if from_old {
            (h.old_start, h.old_lines, h.new_lines)
        } else {
            (h.new_start, h.new_lines, h.old_lines)
        };
        // A hunk with no lines on this side sits after line `start`.
        let before = if len == 0 { n <= start } else { n < start };
        if before {
            break;
        }
        if len > 0 && n < start + len {
            return h.lines.iter().find_map(|l| {
                let (this, that) = if from_old {
                    (l.old_no, l.new_no)
                } else {
                    (l.new_no, l.old_no)
                };
                (this == Some(n)).then_some(that).flatten()
            });
        }
        delta += i64::from(other_len) - i64::from(len);
    }
    u32::try_from(i64::from(n) + delta).ok()
}

/// Apply `file` restricted to the lines `pick` chooses, hunk by hunk from the bottom up.
fn apply_lines(
    repo: &Repository,
    file: &DiffFile,
    target: ApplyTarget,
    reverse: bool,
    pick: impl Fn(&Line) -> bool,
) -> Result<bool> {
    let mut applied = false;
    for ix in (0..file.hunks.len()).rev() {
        if let Some(patch) = lines_patch(file, ix, reverse, &pick) {
            apply_patch(repo, &patch, target, reverse)?;
            applied = true;
        }
    }
    Ok(applied)
}

/// The changed lines of the shown diff that `pick` chooses and whose staged state is `staged`,
/// as (HEAD lines removed, working-tree lines added).
fn chosen(
    staging: &FileStaging,
    staged: bool,
    pick: &impl Fn(usize, &Line) -> bool,
) -> (HashSet<u32>, HashSet<u32>) {
    let (mut removed, mut added) = (HashSet::new(), HashSet::new());
    for (hix, h) in staging.shown.hunks.iter().enumerate() {
        for (lix, l) in h.lines.iter().enumerate() {
            if staging.line_staged(hix, lix) != Some(staged) || !pick(hix, l) {
                continue;
            }
            match l.kind {
                LineKind::Removed => removed.extend(l.old_no),
                LineKind::Added => added.extend(l.new_no),
                LineKind::Context => {}
            }
        }
    }
    (removed, added)
}

/// The hunks of the shown diff with at least one changed line `pick` chooses.
fn picked_hunks(staging: &FileStaging, pick: &impl Fn(usize, &Line) -> bool) -> HashSet<usize> {
    staging
        .shown
        .hunks
        .iter()
        .enumerate()
        .filter(|(hix, h)| {
            h.lines
                .iter()
                .any(|l| l.kind != LineKind::Context && pick(*hix, l))
        })
        .map(|(hix, _)| hix)
        .collect()
}

/// Stage the unstaged lines of the shown diff that `pick` chooses (hunk index, line).
/// Returns whether anything was applied.
pub fn stage_lines(
    repo: &Repository,
    staging: &FileStaging,
    pick: impl Fn(usize, &Line) -> bool,
) -> Result<bool> {
    let Some(unstaged) = &staging.unstaged else {
        return Ok(false);
    };
    let (removed, added) = chosen(staging, false, &pick);
    // A HEAD line still in the index: its number in the index, where U removes it.
    let mut removed: HashSet<u32> = removed
        .iter()
        .filter_map(|h| old_to_new(staging.staged.as_ref(), *h))
        .collect();
    // Index-only lines in a chosen hunk's region go too: staging moves the index to the
    // working tree there.
    let hunks = picked_hunks(staging, &pick);
    removed.extend(
        staging
            .index_only_u
            .iter()
            .filter(|(w, _)| staging.hunk_at_new(*w).is_some_and(|h| hunks.contains(&h)))
            .map(|(_, i)| *i),
    );
    apply_lines(repo, unstaged, ApplyTarget::Index, false, |l| {
        match l.kind {
            LineKind::Added => l.new_no.is_some_and(|n| added.contains(&n)),
            LineKind::Removed => l.old_no.is_some_and(|n| removed.contains(&n)),
            LineKind::Context => false,
        }
    })
}

/// Unstage the staged lines of the shown diff that `pick` chooses. Returns whether anything
/// was applied.
pub fn unstage_lines(
    repo: &Repository,
    staging: &FileStaging,
    pick: impl Fn(usize, &Line) -> bool,
) -> Result<bool> {
    let Some(staged) = &staging.staged else {
        return Ok(false);
    };
    let (removed, added) = chosen(staging, true, &pick);
    // A working-tree line already in the index: its number in the index, where S adds it.
    let mut added: HashSet<u32> = added
        .iter()
        .filter_map(|w| new_to_old(staging.unstaged.as_ref(), *w))
        .collect();
    // Index-only lines in a chosen hunk's region go too: unstaging moves the index back to
    // HEAD there.
    let hunks = picked_hunks(staging, &pick);
    added.extend(
        staging
            .index_only_s
            .iter()
            .filter(|(h, _)| staging.hunk_at_old(*h).is_some_and(|x| hunks.contains(&x)))
            .map(|(_, i)| *i),
    );
    apply_lines(repo, staged, ApplyTarget::Index, true, |l| match l.kind {
        LineKind::Added => l.new_no.is_some_and(|n| added.contains(&n)),
        LineKind::Removed => l.old_no.is_some_and(|n| removed.contains(&n)),
        LineKind::Context => false,
    })
}

/// Put the lines `pick` chooses back as they are in HEAD, in the index and the working tree
/// (PhpStorm's rollback). `path` and `orig_path` reload the staging between the two steps.
pub fn revert_lines(
    repo: &Repository,
    staging: &FileStaging,
    path: &str,
    orig_path: Option<&str>,
    context: u32,
    pick: impl Fn(usize, &Line) -> bool,
) -> Result<bool> {
    let mut changed = unstage_lines(repo, staging, &pick)?;
    // The shown diff's lines keep their numbers (HEAD and the working tree are untouched), so
    // the same choice applies to the reloaded staging.
    let Some(fresh) = file_staging(repo, path, orig_path, context)? else {
        return Ok(changed);
    };
    let Some(unstaged) = &fresh.unstaged else {
        return Ok(changed);
    };
    let (removed, added) = chosen(&fresh, false, &|hix, l: &Line| {
        // Match by line numbers: hunk indexes can differ after the index changed.
        let _ = hix;
        staging.shown.hunks.iter().enumerate().any(|(oix, h)| {
            h.lines.iter().any(|o| {
                o.kind == l.kind && o.old_no == l.old_no && o.new_no == l.new_no && pick(oix, o)
            })
        })
    });
    let removed: HashSet<u32> = removed
        .iter()
        .filter_map(|h| old_to_new(fresh.staged.as_ref(), *h))
        .collect();
    changed |= apply_lines(repo, unstaged, ApplyTarget::Worktree, true, |l| {
        match l.kind {
            LineKind::Added => l.new_no.is_some_and(|n| added.contains(&n)),
            LineKind::Removed => l.old_no.is_some_and(|n| removed.contains(&n)),
            LineKind::Context => false,
        }
    })?;
    Ok(changed)
}

#[cfg(test)]
mod tests;
