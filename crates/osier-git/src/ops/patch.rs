//! Build patches for one hunk or a subset of its lines, for `git apply`.

use crate::diff::{DiffFile, FileStatus, Hunk, Line, LineKind};

/// Patch containing only hunk `hunk_ix` of `file`.
pub fn hunk_patch(file: &DiffFile, hunk_ix: usize) -> Option<String> {
    lines_patch(file, hunk_ix, false, |_| true)
}

/// Patch containing hunk `hunk_ix` with only the changed lines for which `selected` is true.
/// Returns `None` when nothing is selected or the file is binary.
///
/// `reverse` says how the patch will be applied, because the lines left out have to match
/// what the patch is applied to. Applied forwards (staging) that is the old side: unselected
/// added lines are dropped and unselected removed lines become context. Applied in reverse
/// (unstaging, discarding) it is the new side: unselected added lines become context and
/// unselected removed lines are dropped.
pub fn lines_patch(
    file: &DiffFile,
    hunk_ix: usize,
    reverse: bool,
    selected: impl Fn(&Line) -> bool,
) -> Option<String> {
    if file.binary {
        return None;
    }
    let hunk = file.hunks.get(hunk_ix)?;
    let mut body = String::new();
    let (mut old_count, mut new_count, mut changed) = (0u32, 0u32, 0usize);
    for line in &hunk.lines {
        // What an unselected changed line turns into: context when the side the patch is
        // applied to has the line, nothing when it does not.
        let (marker, in_target) = match line.kind {
            LineKind::Context => (' ', true),
            LineKind::Added => ('+', reverse),
            LineKind::Removed => ('-', !reverse),
        };
        let keep = if line.kind == LineKind::Context {
            Some(' ')
        } else if selected(line) {
            changed += 1;
            Some(marker)
        } else if in_target {
            Some(' ')
        } else {
            None
        };
        if let Some(marker) = keep {
            if marker != '+' {
                old_count += 1;
            }
            if marker != '-' {
                new_count += 1;
            }
            body.push(marker);
            body.push_str(&line.text);
            body.push('\n');
            if line.no_newline {
                body.push_str("\\ No newline at end of file\n");
            }
        }
    }
    if changed == 0 {
        return None;
    }
    // A single hunk applied on its own starts at the same line on both sides, counted on the
    // side the patch is applied to. A side with no lines at all (a brand new or deleted file)
    // starts at 0.
    let (old_start, new_start) = if reverse {
        let new_start = hunk.new_start;
        let old_start = if new_count == 0 && new_start == 0 {
            1
        } else {
            new_start.max(1)
        };
        (old_start, new_start)
    } else {
        let old_start = hunk.old_start;
        let new_start = if old_count == 0 && old_start == 0 {
            1
        } else {
            old_start.max(1)
        };
        (old_start, new_start)
    };
    let mut out = String::new();
    let old_path = file.old_path.as_deref().unwrap_or(&file.path);
    out.push_str(&format!("diff --git a/{old_path} b/{}\n", file.path));
    // With some lines left out, a new file's patch still has old lines and a deleted file's
    // still has new ones: git only takes those as an edit of the file.
    match file.status {
        FileStatus::Added if old_count == 0 => {
            out.push_str(&format!(
                "new file mode {}\n",
                file.new_mode.as_deref().unwrap_or("100644")
            ));
            out.push_str("--- /dev/null\n");
            out.push_str(&format!("+++ b/{}\n", file.path));
        }
        FileStatus::Deleted if new_count == 0 => {
            out.push_str(&format!(
                "deleted file mode {}\n",
                file.old_mode.as_deref().unwrap_or("100644")
            ));
            out.push_str(&format!("--- a/{old_path}\n"));
            out.push_str("+++ /dev/null\n");
        }
        _ => {
            out.push_str(&format!("--- a/{old_path}\n"));
            out.push_str(&format!("+++ b/{}\n", file.path));
        }
    }
    out.push_str(&header(
        old_start,
        old_count,
        new_start,
        new_count,
        &hunk.context,
    ));
    out.push_str(&body);
    Some(out)
}

fn header(old_start: u32, old_count: u32, new_start: u32, new_count: u32, context: &str) -> String {
    let range = |start: u32, count: u32| {
        if count == 1 {
            format!("{start}")
        } else {
            format!("{start},{count}")
        }
    };
    let mut h = format!(
        "@@ -{} +{} @@",
        range(old_start, old_count),
        range(new_start, new_count)
    );
    if !context.is_empty() {
        h.push(' ');
        h.push_str(context);
    }
    h.push('\n');
    h
}

/// Hunks of `file` in display order, with their line ranges on the new side (for mapping a
/// viewer's selection back to lines).
pub fn hunk_for_new_line(file: &DiffFile, new_line: u32) -> Option<(usize, &Hunk)> {
    file.hunks.iter().enumerate().find(|(_, h)| {
        let end = h.new_start + h.new_lines.max(1) - 1;
        (h.new_start..=end).contains(&new_line)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{DiffOptions, DiffTarget, diff};
    use crate::ops::stage::{ApplyTarget, apply_patch};
    use crate::status::{StatusOptions, status};
    use crate::testing::TempRepo;

    const BASE: &str = "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n";

    fn two_hunk_repo() -> TempRepo {
        let t = TempRepo::new();
        t.commit_file("f.txt", BASE, "base");
        t.write(
            "f.txt",
            "ONE\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nTEN\n",
        );
        t
    }

    #[test]
    fn stage_one_hunk_of_two() {
        let t = two_hunk_repo();
        let repo = t.repo();
        let files = diff(&repo, &DiffOptions::new(DiffTarget::Unstaged)).unwrap();
        assert_eq!(files[0].hunks.len(), 2);
        let patch = hunk_patch(&files[0], 1).unwrap();
        apply_patch(&repo, &patch, ApplyTarget::Index, false).unwrap();

        let staged = diff(&repo, &DiffOptions::new(DiffTarget::Staged)).unwrap();
        assert_eq!(staged[0].hunks.len(), 1);
        assert_eq!(
            staged[0].hunks[0]
                .lines
                .iter()
                .find(|l| l.kind == LineKind::Added)
                .unwrap()
                .text,
            "TEN"
        );
        let unstaged = diff(&repo, &DiffOptions::new(DiffTarget::Unstaged)).unwrap();
        assert_eq!(unstaged[0].hunks.len(), 1);
        assert_eq!(
            unstaged[0].hunks[0]
                .lines
                .iter()
                .find(|l| l.kind == LineKind::Added)
                .unwrap()
                .text,
            "ONE"
        );

        // Unstage it again with the staged hunk reversed.
        let back = hunk_patch(&staged[0], 0).unwrap();
        apply_patch(&repo, &back, ApplyTarget::Index, true).unwrap();
        assert!(
            diff(&repo, &DiffOptions::new(DiffTarget::Staged))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn stage_selected_lines_only() {
        let t = TempRepo::new();
        t.commit_file("f.txt", "a\nb\nc\n", "base");
        t.write("f.txt", "a\nB\nc\nd\n");
        let repo = t.repo();
        let files = diff(&repo, &DiffOptions::new(DiffTarget::Unstaged)).unwrap();
        // Select only the added "d" line (new line 4): "b" stays, "B" is not staged.
        let patch = lines_patch(&files[0], 0, false, |l| {
            l.kind == LineKind::Added && l.text == "d"
        })
        .unwrap();
        apply_patch(&repo, &patch, ApplyTarget::Index, false).unwrap();
        let staged = diff(&repo, &DiffOptions::new(DiffTarget::Staged)).unwrap();
        let added: Vec<&str> = staged[0].hunks[0]
            .lines
            .iter()
            .filter(|l| l.kind == LineKind::Added)
            .map(|l| l.text.as_str())
            .collect();
        assert_eq!(added, ["d"]);
        assert_eq!(staged[0].removed(), 0, "unselected removal became context");
        let unstaged = diff(&repo, &DiffOptions::new(DiffTarget::Unstaged)).unwrap();
        assert_eq!((unstaged[0].added(), unstaged[0].removed()), (1, 1));
    }

    fn kinds(file: &DiffFile, kind: LineKind) -> Vec<&str> {
        file.hunks
            .iter()
            .flat_map(|h| &h.lines)
            .filter(|l| l.kind == kind)
            .map(|l| l.text.as_str())
            .collect()
    }

    #[test]
    fn unstage_selected_lines_only() {
        let t = TempRepo::new();
        t.commit_file("f.txt", "a\nb\nc\n", "base");
        t.write("f.txt", "a\nB\nc\nd\n");
        t.git(["add", "f.txt"]);
        let repo = t.repo();
        let staged = diff(&repo, &DiffOptions::new(DiffTarget::Staged)).unwrap();
        // Unstage only the added "d": the b -> B change stays staged.
        let patch = lines_patch(&staged[0], 0, true, |l| {
            l.kind == LineKind::Added && l.text == "d"
        })
        .unwrap();
        apply_patch(&repo, &patch, ApplyTarget::Index, true).unwrap();
        let staged = diff(&repo, &DiffOptions::new(DiffTarget::Staged)).unwrap();
        assert_eq!(kinds(&staged[0], LineKind::Added), ["B"]);
        assert_eq!(kinds(&staged[0], LineKind::Removed), ["b"]);
        let unstaged = diff(&repo, &DiffOptions::new(DiffTarget::Unstaged)).unwrap();
        assert_eq!(kinds(&unstaged[0], LineKind::Added), ["d"]);
        assert_eq!(unstaged[0].removed(), 0);
        assert_eq!(t.read("f.txt"), "a\nB\nc\nd\n", "working tree untouched");
    }

    #[test]
    fn unstage_a_selected_removal_only() {
        let t = TempRepo::new();
        t.commit_file("f.txt", "a\nb\nc\n", "base");
        t.write("f.txt", "a\nB\nc\nd\n");
        t.git(["add", "f.txt"]);
        let repo = t.repo();
        let staged = diff(&repo, &DiffOptions::new(DiffTarget::Staged)).unwrap();
        // Unstage only the removal of "b": the index keeps it next to the staged "B".
        let patch = lines_patch(&staged[0], 0, true, |l| l.kind == LineKind::Removed).unwrap();
        apply_patch(&repo, &patch, ApplyTarget::Index, true).unwrap();
        let staged = diff(&repo, &DiffOptions::new(DiffTarget::Staged)).unwrap();
        assert_eq!(kinds(&staged[0], LineKind::Added), ["B", "d"]);
        assert_eq!(staged[0].removed(), 0);
    }

    #[test]
    fn unstage_lines_of_a_new_file() {
        let t = TempRepo::new();
        t.commit_file("keep.txt", "k\n", "base");
        t.write("n.txt", "one\ntwo\nthree\n");
        t.git(["add", "n.txt"]);
        let repo = t.repo();
        let staged = diff(&repo, &DiffOptions::new(DiffTarget::Staged)).unwrap();
        let patch = lines_patch(&staged[0], 0, true, |l| l.text == "two").unwrap();
        apply_patch(&repo, &patch, ApplyTarget::Index, true).unwrap();
        let staged = diff(&repo, &DiffOptions::new(DiffTarget::Staged)).unwrap();
        assert_eq!(kinds(&staged[0], LineKind::Added), ["one", "three"]);
        let unstaged = diff(&repo, &DiffOptions::new(DiffTarget::Unstaged)).unwrap();
        assert_eq!(kinds(&unstaged[0], LineKind::Added), ["two"]);
    }

    #[test]
    fn unstage_every_line_of_a_new_file_by_selection() {
        let t = TempRepo::new();
        t.commit_file("keep.txt", "k\n", "base");
        t.write("n.txt", "one\ntwo\n");
        t.git(["add", "n.txt"]);
        let repo = t.repo();
        let staged = diff(&repo, &DiffOptions::new(DiffTarget::Staged)).unwrap();
        let patch = lines_patch(&staged[0], 0, true, |_| true).unwrap();
        assert!(patch.contains("new file mode"));
        apply_patch(&repo, &patch, ApplyTarget::Index, true).unwrap();
        assert!(
            diff(&repo, &DiffOptions::new(DiffTarget::Staged))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn unstage_part_of_a_staged_deletion() {
        let t = TempRepo::new();
        t.commit_file("gone.txt", "one\ntwo\nthree\n", "base");
        t.git(["rm", "-q", "gone.txt"]);
        let repo = t.repo();
        let staged = diff(&repo, &DiffOptions::new(DiffTarget::Staged)).unwrap();
        // Unstage the removal of "two" only: the index gets the file back with that line.
        let patch = lines_patch(&staged[0], 0, true, |l| l.text == "two").unwrap();
        apply_patch(&repo, &patch, ApplyTarget::Index, true).unwrap();
        let staged = diff(&repo, &DiffOptions::new(DiffTarget::Staged)).unwrap();
        assert_eq!(kinds(&staged[0], LineKind::Removed), ["one", "three"]);
        assert_eq!(staged[0].added(), 0);
    }

    #[test]
    fn stage_part_of_a_deleted_file() {
        let t = TempRepo::new();
        t.commit_file("gone.txt", "one\ntwo\nthree\n", "base");
        t.remove("gone.txt");
        let repo = t.repo();
        let files = diff(&repo, &DiffOptions::new(DiffTarget::Unstaged)).unwrap();
        // Stage the removal of "two" only: the index keeps the file with the other lines.
        let patch = lines_patch(&files[0], 0, false, |l| l.text == "two").unwrap();
        apply_patch(&repo, &patch, ApplyTarget::Index, false).unwrap();
        let staged = diff(&repo, &DiffOptions::new(DiffTarget::Staged)).unwrap();
        assert_eq!(kinds(&staged[0], LineKind::Removed), ["two"]);
        assert_eq!(staged[0].added(), 0);
    }

    #[test]
    fn discard_selected_lines_only() {
        let t = TempRepo::new();
        t.commit_file("f.txt", "a\nb\nc\n", "base");
        t.write("f.txt", "a\nB\nc\nd\n");
        let repo = t.repo();
        let files = diff(&repo, &DiffOptions::new(DiffTarget::Unstaged)).unwrap();
        let patch = lines_patch(&files[0], 0, true, |l| {
            l.kind == LineKind::Added && l.text == "d"
        })
        .unwrap();
        apply_patch(&repo, &patch, ApplyTarget::Worktree, true).unwrap();
        assert_eq!(
            t.read("f.txt"),
            "a\nB\nc\n",
            "only the selected line is gone"
        );
    }

    #[test]
    fn discard_a_hunk_in_the_worktree() {
        let t = two_hunk_repo();
        let repo = t.repo();
        let files = diff(&repo, &DiffOptions::new(DiffTarget::Unstaged)).unwrap();
        let patch = hunk_patch(&files[0], 0).unwrap();
        apply_patch(&repo, &patch, ApplyTarget::Worktree, true).unwrap();
        assert!(t.read("f.txt").starts_with("one\n"), "first hunk reverted");
        assert!(t.read("f.txt").ends_with("TEN\n"), "second hunk kept");
    }

    #[test]
    fn new_file_hunk_and_no_newline() {
        let t = TempRepo::new();
        t.commit_file("keep.txt", "k\n", "base");
        t.write("n.txt", "x");
        let repo = t.repo();
        let files = crate::diff::diff_untracked(&repo, "n.txt", 3).unwrap();
        let patch = hunk_patch(&files[0], 0).unwrap();
        assert!(patch.contains("new file mode"));
        assert!(patch.contains("\\ No newline at end of file"));
        // Intent-to-add makes the untracked file stageable by patch.
        t.git(["add", "-N", "n.txt"]);
        apply_patch(&repo, &patch, ApplyTarget::Index, false).unwrap();
        let s = status(&repo, StatusOptions::default()).unwrap();
        assert!(s.staged().any(|e| e.path == "n.txt"));
    }

    #[test]
    fn nothing_selected_is_none() {
        let t = two_hunk_repo();
        let files = diff(&t.repo(), &DiffOptions::new(DiffTarget::Unstaged)).unwrap();
        assert!(lines_patch(&files[0], 0, false, |_| false).is_none());
        assert!(hunk_patch(&files[0], 5).is_none());
    }

    #[test]
    fn hunk_lookup_by_new_line() {
        let t = two_hunk_repo();
        let files = diff(&t.repo(), &DiffOptions::new(DiffTarget::Unstaged)).unwrap();
        assert_eq!(hunk_for_new_line(&files[0], 1).map(|(i, _)| i), Some(0));
        assert_eq!(hunk_for_new_line(&files[0], 10).map(|(i, _)| i), Some(1));
        assert_eq!(hunk_for_new_line(&files[0], 5), None);
    }
}
