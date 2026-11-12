use std::collections::HashMap;

use super::*;
use crate::status::{StatusOptions, status};
use crate::testing::TempRepo;

/// Line identity in the shown diff: HEAD and the working tree do not change while staging, so
/// these numbers stay put across reloads.
type Key = (LineKind, Option<u32>, Option<u32>);

fn marks(s: &FileStaging) -> HashMap<Key, bool> {
    let mut out = HashMap::new();
    for (hix, h) in s.shown.hunks.iter().enumerate() {
        for (lix, l) in h.lines.iter().enumerate() {
            if let Some(staged) = s.line_staged(hix, lix) {
                out.insert((l.kind, l.old_no, l.new_no), staged);
            }
        }
    }
    out
}

fn load(t: &TempRepo, path: &str) -> FileStaging {
    file_staging(&t.repo(), path, None, 3)
        .expect("staging")
        .expect("changed")
}

fn numbered(n: usize) -> String {
    (1..=n).map(|i| format!("line {i}\n")).collect()
}

fn entry_check(t: &TempRepo, path: &str) -> Option<Check> {
    let s = status(&t.repo(), StatusOptions::default()).expect("status");
    s.entries
        .iter()
        .find(|e| e.path == path)
        .and_then(file_check)
}

/// Two changes far apart, so they are two hunks.
fn two_hunks() -> TempRepo {
    let t = TempRepo::new();
    t.commit_file("a.txt", &numbered(30), "first");
    let changed = numbered(30)
        .replace("line 3\n", "line three\n")
        .replace("line 25\n", "line twenty-five\n");
    t.write("a.txt", &changed);
    t
}

#[test]
fn nothing_staged_then_one_hunk_staged() {
    let t = two_hunks();
    let s = load(&t, "a.txt");
    assert_eq!(s.shown.hunks.len(), 2);
    assert_eq!(s.check(), Check::Unstaged);
    assert_eq!(s.hunk_counts(), (0, 2));
    assert_eq!(entry_check(&t, "a.txt"), Some(Check::Unstaged));

    assert!(stage_lines(&t.repo(), &s, |hix, _| hix == 0).unwrap());
    let s = load(&t, "a.txt");
    assert_eq!(s.hunk_check(0), Check::Staged);
    assert_eq!(s.hunk_check(1), Check::Unstaged);
    assert_eq!(s.check(), Check::Partial);
    assert_eq!(s.hunk_counts(), (1, 2));
    assert_eq!(entry_check(&t, "a.txt"), Some(Check::Partial));
    let index = t.git(["show", ":a.txt"]);
    assert!(index.contains("line three") && !index.contains("twenty-five"));
}

#[test]
fn unstage_one_hunk_of_a_fully_staged_file() {
    let t = two_hunks();
    t.git(["add", "a.txt"]);
    let s = load(&t, "a.txt");
    assert_eq!(s.check(), Check::Staged);
    assert_eq!(entry_check(&t, "a.txt"), Some(Check::Staged));

    assert!(unstage_lines(&t.repo(), &s, |hix, _| hix == 1).unwrap());
    let s = load(&t, "a.txt");
    assert_eq!(s.hunk_check(0), Check::Staged);
    assert_eq!(s.hunk_check(1), Check::Unstaged);
    assert!(
        t.read("a.txt").contains("twenty-five"),
        "working tree untouched"
    );
}

#[test]
fn some_lines_of_a_hunk() {
    let t = TempRepo::new();
    t.commit_file("a.txt", &numbered(20), "first");
    let changed = numbered(20)
        .replace("line 10\n", "ten\n")
        .replace("line 11\n", "eleven\n")
        .replace("line 12\n", "twelve\n");
    t.write("a.txt", &changed);
    let s = load(&t, "a.txt");
    assert_eq!(s.shown.hunks.len(), 1);
    // The first changed pair only: HEAD line 10 out, working-tree line 10 in.
    let pick = |_: usize, l: &Line| {
        l.old_no == Some(10) && l.kind == LineKind::Removed
            || l.new_no == Some(10) && l.kind == LineKind::Added
    };
    assert!(stage_lines(&t.repo(), &s, pick).unwrap());
    let s = load(&t, "a.txt");
    assert_eq!(s.hunk_check(0), Check::Partial);
    let index = t.git(["show", ":a.txt"]);
    assert!(index.contains("ten\n"));
    assert!(index.contains("line 11\n") && !index.contains("eleven"));
}

#[test]
fn revert_a_partly_staged_hunk_resets_index_and_working_tree() {
    let t = two_hunks();
    let s = load(&t, "a.txt");
    // Stage both hunks, then edit hunk 0 again so it is partly staged.
    stage_lines(&t.repo(), &s, |_, _| true).unwrap();
    let changed = t.read("a.txt").replace("line 4\n", "line four\n");
    t.write("a.txt", &changed);
    let s = load(&t, "a.txt");
    assert_eq!(s.hunk_check(0), Check::Partial);
    assert_eq!(s.hunk_check(1), Check::Staged);

    assert!(revert_lines(&t.repo(), &s, "a.txt", None, 3, |hix, _| hix == 0).unwrap());
    let work = t.read("a.txt");
    assert!(work.contains("line 3\n") && work.contains("line 4\n"));
    assert!(work.contains("twenty-five"), "the other hunk stays");
    let index = t.git(["show", ":a.txt"]);
    assert!(index.contains("line 3\n") && index.contains("twenty-five"));
    let s = load(&t, "a.txt");
    assert_eq!(s.shown.hunks.len(), 1);
    assert_eq!(s.check(), Check::Staged);
}

#[test]
fn new_file_staged_then_edited_and_unstaged() {
    let t = TempRepo::new();
    t.commit_file("keep.txt", "k\n", "first");
    t.write("new.txt", &numbered(5));
    t.git(["add", "new.txt"]);
    t.write("new.txt", &format!("{}extra\n", numbered(5)));
    let s = load(&t, "new.txt");
    assert_eq!(s.check(), Check::Partial);
    assert_eq!(entry_check(&t, "new.txt"), Some(Check::Partial));

    assert!(unstage_lines(&t.repo(), &s, |_, _| true).unwrap());
    let st = status(&t.repo(), StatusOptions::default()).unwrap();
    let e = st.entries.iter().find(|e| e.path == "new.txt").unwrap();
    assert!(
        e.is_untracked(),
        "a fully unstaged new file is untracked again"
    );
}

#[test]
fn unborn_branch_is_shown_against_the_empty_tree() {
    let t = TempRepo::new();
    t.write("a.txt", &numbered(3));
    t.git(["add", "a.txt"]);
    t.write("a.txt", &format!("{}four\n", numbered(3)));
    let s = load(&t, "a.txt");
    assert_eq!(s.check(), Check::Partial);
    assert!(stage_lines(&t.repo(), &s, |_, _| true).unwrap());
    assert_eq!(load(&t, "a.txt").check(), Check::Staged);
}

#[test]
fn a_mode_change_alone_has_nothing_to_show() {
    let t = TempRepo::new();
    t.commit_file("run.sh", "echo hi\n", "first");
    t.git(["update-index", "--chmod=+x", "run.sh"]);
    // Only the index changed mode; the working tree did not, so no textual difference.
    assert!(
        file_staging(&t.repo(), "run.sh", None, 3)
            .unwrap()
            .is_none()
    );
}

#[test]
fn renamed_and_edited_file_stages_by_hunk() {
    let t = TempRepo::new();
    t.commit_file("a.txt", &numbered(30), "first");
    t.git(["mv", "a.txt", "b.txt"]);
    let changed = numbered(30)
        .replace("line 3\n", "line three\n")
        .replace("line 25\n", "line twenty-five\n");
    t.write("b.txt", &changed);
    let s = file_staging(&t.repo(), "b.txt", Some("a.txt"), 3)
        .unwrap()
        .unwrap();
    assert_eq!(s.shown.old_path.as_deref(), Some("a.txt"));
    assert_eq!(s.shown.hunks.len(), 2);
    assert_eq!(
        s.check(),
        Check::Unstaged,
        "the rename alone is no line change"
    );

    assert!(stage_lines(&t.repo(), &s, |hix, _| hix == 1).unwrap());
    let s = file_staging(&t.repo(), "b.txt", Some("a.txt"), 3)
        .unwrap()
        .unwrap();
    assert_eq!(s.hunk_check(0), Check::Unstaged);
    assert_eq!(s.hunk_check(1), Check::Staged);
    assert!(t.git(["show", ":b.txt"]).contains("twenty-five"));
}

#[test]
fn a_small_renamed_file_edited_a_lot_is_still_one_renamed_file() {
    let t = TempRepo::new();
    // One of two lines changes: under git's 50% similarity, so plain `-M` reports the old
    // file deleted and the new one added.
    t.commit_file("tiny.txt", "Short note.\nTwo lines.\n", "first");
    t.git(["mv", "tiny.txt", "small.txt"]);
    t.write("small.txt", "Short note!\nTwo lines.\n");
    assert_eq!(
        t.git([
            "diff",
            "HEAD",
            "-M",
            "--name-status",
            "--",
            "small.txt",
            "tiny.txt"
        ]),
        "A\tsmall.txt\nD\ttiny.txt\n",
        "git alone does not pair them"
    );
    let s = file_staging(&t.repo(), "small.txt", Some("tiny.txt"), 3)
        .unwrap()
        .unwrap();
    assert_eq!(s.shown.status, crate::diff::FileStatus::Renamed);
    assert_eq!(s.shown.old_path.as_deref(), Some("tiny.txt"));
    assert_eq!(parse(&s.shown_text).unwrap().len(), 1, "one file shown");
    let changed: Vec<(LineKind, &str)> = s.shown.hunks[0]
        .lines
        .iter()
        .filter(|l| l.kind != LineKind::Context)
        .map(|l| (l.kind, l.text.as_str()))
        .collect();
    assert_eq!(
        changed,
        [
            (LineKind::Removed, "Short note."),
            (LineKind::Added, "Short note!")
        ]
    );
    assert_eq!(s.check(), Check::Unstaged);

    // Rolling the one changed line back leaves the plain rename.
    assert!(revert_lines(&t.repo(), &s, "small.txt", Some("tiny.txt"), 3, |_, _| true).unwrap());
    assert_eq!(t.read("small.txt"), "Short note.\nTwo lines.\n");
    assert_eq!(
        reason(&t, "small.txt", Some("tiny.txt")),
        Some(Unchanged::Renamed {
            from: "tiny.txt".into()
        })
    );

    // Staging it instead puts the edit in the index.
    t.write("small.txt", "Short note!\nTwo lines.\n");
    let s = file_staging(&t.repo(), "small.txt", Some("tiny.txt"), 3)
        .unwrap()
        .unwrap();
    assert!(stage_lines(&t.repo(), &s, |_, _| true).unwrap());
    assert_eq!(t.git(["show", ":small.txt"]), "Short note!\nTwo lines.\n");
}

#[test]
fn line_mapping_across_hunks() {
    // Old side 1..=10; one line inserted after 2, line 6 removed.
    let text = "diff --git a/f b/f\n--- a/f\n+++ b/f\n\
                @@ -2,0 +3 @@\n+new\n\
                @@ -6 +6,0 @@\n-six\n";
    let f = parse(text).unwrap().remove(0);
    assert_eq!(old_to_new(Some(&f), 1), Some(1));
    assert_eq!(old_to_new(Some(&f), 2), Some(2));
    assert_eq!(old_to_new(Some(&f), 3), Some(4));
    assert_eq!(old_to_new(Some(&f), 6), None);
    assert_eq!(old_to_new(Some(&f), 7), Some(7));
    assert_eq!(new_to_old(Some(&f), 3), None);
    assert_eq!(new_to_old(Some(&f), 4), Some(3));
    assert_eq!(new_to_old(Some(&f), 7), Some(7));
    assert_eq!(old_to_new(None, 5), Some(5));
}

/// Small deterministic generator for the property test (no extra dependencies).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn random_edit(rng: &mut Rng, text: &str, round: usize) -> String {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    for edit in 0..1 + rng.below(6) {
        let at = rng.below(lines.len() + 1);
        match rng.below(3) {
            0 if at < lines.len() => lines[at] = format!("changed {round}.{edit}"),
            1 if at < lines.len() => {
                lines.remove(at);
            }
            _ => lines.insert(at, format!("inserted {round}.{edit}")),
        }
    }
    lines.iter().map(|l| format!("{l}\n")).collect()
}

/// Shown with this much context the whole file is one hunk, so the expected index and working
/// tree can be rebuilt from it line by line. The patches still use small hunks
/// (`PATCH_CONTEXT`), so applying several hunks bottom-up is exercised.
const WHOLE_FILE: u32 = 100_000;

/// Rebuild a file from the shown hunk: `keep` says which changed lines it has.
fn rebuild(s: &FileStaging, keep: impl Fn(&Line, bool) -> bool) -> String {
    let h = &s.shown.hunks[0];
    let mut out = String::new();
    for (lix, l) in h.lines.iter().enumerate() {
        let present = match s.line_staged(0, lix) {
            None => true,
            Some(staged) => keep(l, staged),
        };
        if present {
            out.push_str(&l.text);
            if !l.no_newline {
                out.push('\n');
            }
        }
    }
    out
}

/// Random edits, then random stage / unstage / revert of random line sets. After each step the
/// index and the working tree hold the lines the step should leave, each line's staged
/// state follows, and git's own status agrees.
#[test]
fn property_random_edits_and_partial_staging() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    for case in 0..16 {
        let t = TempRepo::new();
        t.commit_file("f.txt", &numbered(40), "first");
        t.write("f.txt", &random_edit(&mut rng, &numbered(40), 0));
        for step in 0..10 {
            let Some(before) = file_staging(&t.repo(), "f.txt", None, WHOLE_FILE).unwrap() else {
                break;
            };
            assert_eq!(before.shown.hunks.len(), 1);
            let seed = rng.next();
            let pick = move |_: usize, l: &Line| {
                let k =
                    u64::from(l.old_no.unwrap_or(0)) * 7 + u64::from(l.new_no.unwrap_or(0)) * 13;
                (k ^ seed).is_multiple_of(3)
            };
            let chosen = |l: &Line| l.kind != LineKind::Context && pick(0, l);
            // A line is in the index when: a removed (HEAD) line is not staged, an added line is.
            let in_index = |l: &Line, staged: bool| (l.kind == LineKind::Added) == staged;
            // Index-only lines (staged, then changed again in the working tree): the shown
            // diff cannot hold them. A step takes out the ones in the region of a hunk it
            // touches; the rest stay. Their texts, as U removes them and as S adds them.
            let text_at = |f: &Option<DiffFile>, kind: LineKind, n: u32| {
                f.as_ref()
                    .into_iter()
                    .flat_map(|f| &f.hunks)
                    .flat_map(|h| &h.lines)
                    .find(|l| {
                        l.kind == kind
                            && if kind == LineKind::Removed {
                                l.old_no
                            } else {
                                l.new_no
                            } == Some(n)
                    })
                    .map(|l| l.text.clone())
                    .unwrap_or_default()
            };
            let hunks = picked_hunks(&before, &pick);
            let kept_u: Vec<String> = before
                .index_only_u
                .iter()
                .filter(|(w, _)| !before.hunk_at_new(*w).is_some_and(|h| hunks.contains(&h)))
                .map(|(_, i)| text_at(&before.unstaged, LineKind::Removed, *i))
                .collect();
            let kept_s: Vec<String> = before
                .index_only_s
                .iter()
                .filter(|(h, _)| !before.hunk_at_old(*h).is_some_and(|x| hunks.contains(&x)))
                .map(|(_, i)| text_at(&before.staged, LineKind::Added, *i))
                .collect();
            let with = |base: String, extra: &[String]| {
                let mut text = base;
                for e in extra {
                    text.push_str(e);
                    text.push('\n');
                }
                text
            };
            let action = rng.below(4);
            let (want_index, want_work) = match action {
                0 => {
                    stage_lines(&t.repo(), &before, pick).unwrap();
                    (
                        with(
                            rebuild(&before, |l, s| in_index(l, s || chosen(l))),
                            &kept_u,
                        ),
                        t.read("f.txt"),
                    )
                }
                1 => {
                    unstage_lines(&t.repo(), &before, pick).unwrap();
                    (
                        with(
                            rebuild(&before, |l, s| in_index(l, s && !chosen(l))),
                            &kept_s,
                        ),
                        t.read("f.txt"),
                    )
                }
                2 => {
                    revert_lines(&t.repo(), &before, "f.txt", None, WHOLE_FILE, pick).unwrap();
                    (
                        // Chosen lines go back to HEAD in both; the rest keep their state.
                        with(
                            rebuild(&before, |l, s| {
                                if chosen(l) {
                                    l.kind == LineKind::Removed
                                } else {
                                    in_index(l, s)
                                }
                            }),
                            &kept_s,
                        ),
                        rebuild(&before, |l, _| (l.kind == LineKind::Removed) == chosen(l)),
                    )
                }
                _ => {
                    // Change the working tree again: new lines appear unstaged.
                    let text = random_edit(&mut rng, &t.read("f.txt"), step + 1);
                    t.write("f.txt", &text);
                    continue;
                }
            };
            let ctx = format!("case {case} step {step} action {action}");
            // Compared as sets of lines: where a change block both removes and adds lines, the
            // shown diff and the index diff may order them differently (both are right), but no
            // line may be lost or doubled.
            let sorted = |text: String| {
                let mut v: Vec<String> = text.lines().map(str::to_string).collect();
                v.sort();
                v
            };
            assert_eq!(
                sorted(t.git(["show", ":f.txt"])),
                sorted(want_index),
                "{ctx}: index"
            );
            assert_eq!(
                sorted(t.read("f.txt")),
                sorted(want_work),
                "{ctx}: working tree"
            );

            // Stage and unstage leave the shown diff's line numbers alone: check each line.
            if action < 2
                && let Some(after) = file_staging(&t.repo(), "f.txt", None, WHOLE_FILE).unwrap()
            {
                let (old, new) = (marks(&before), marks(&after));
                for l in &before.shown.hunks[0].lines {
                    if l.kind == LineKind::Context {
                        continue;
                    }
                    let key = (l.kind, l.old_no, l.new_no);
                    let was = old[&key];
                    let want = if action == 0 {
                        was || chosen(l)
                    } else {
                        was && !chosen(l)
                    };
                    assert_eq!(new.get(&key), Some(&want), "{ctx}: line {key:?}");
                }
            }
            // git agrees: something staged iff the index differs from HEAD, something
            // unstaged iff the working tree differs from the index.
            let st = status(&t.repo(), StatusOptions::default()).unwrap();
            let after = file_staging(&t.repo(), "f.txt", None, WHOLE_FILE).unwrap();
            let lines = after.as_ref().map(marks).unwrap_or_default();
            let entry = st.entries.iter().find(|e| e.path == "f.txt");
            let index_only = after.as_ref().is_some_and(FileStaging::has_index_only);
            assert_eq!(
                entry.is_some_and(|e| e.index != '.'),
                lines.values().any(|s| *s) || index_only,
                "{ctx}: index flag"
            );
            assert_eq!(
                entry.is_some_and(|e| e.worktree != '.'),
                lines.values().any(|s| !*s) || index_only,
                "{ctx}: worktree flag"
            );
        }
    }
}

/// Ticking every hunk stages the whole file and unticking every hunk unstages it, including
/// index-only lines in a hunk's region, as long as none sit outside every hunk (only the
/// file's own checkbox reaches those).
#[test]
fn property_all_hunks_stage_and_unstage_the_whole_file() {
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    let mut checked = 0;
    for case in 0..40 {
        let t = TempRepo::new();
        t.commit_file("f.txt", &numbered(40), "first");
        t.write("f.txt", &random_edit(&mut rng, &numbered(40), 0));
        // Some staged state, then more edits, so index-only lines turn up.
        if let Some(s) = file_staging(&t.repo(), "f.txt", None, 3).unwrap() {
            let seed = rng.next();
            stage_lines(&t.repo(), &s, move |_, l| {
                (u64::from(l.new_no.unwrap_or(0)) ^ seed).is_multiple_of(2)
            })
            .unwrap();
        }
        t.write("f.txt", &random_edit(&mut rng, &t.read("f.txt"), 1));
        let Some(s) = file_staging(&t.repo(), "f.txt", None, 3).unwrap() else {
            continue;
        };
        let orphans = s
            .index_only_u
            .iter()
            .any(|(w, _)| s.hunk_at_new(*w).is_none())
            || s.index_only_s
                .iter()
                .any(|(h, _)| s.hunk_at_old(*h).is_none());
        if orphans {
            continue;
        }
        checked += 1;
        if case % 2 == 0 {
            stage_lines(&t.repo(), &s, |_, _| true).unwrap();
            assert_eq!(
                t.git(["show", ":f.txt"]),
                t.read("f.txt"),
                "case {case}: index = working tree"
            );
            let after = file_staging(&t.repo(), "f.txt", None, 3).unwrap().unwrap();
            assert_eq!(after.check(), Check::Staged, "case {case}");
        } else {
            unstage_lines(&t.repo(), &s, |_, _| true).unwrap();
            assert_eq!(
                t.git(["show", ":f.txt"]),
                t.git(["show", "HEAD:f.txt"]),
                "case {case}: index = HEAD"
            );
            let after = file_staging(&t.repo(), "f.txt", None, 3).unwrap().unwrap();
            assert_eq!(after.check(), Check::Unstaged, "case {case}");
        }
    }
    assert!(
        checked > 20,
        "only {checked} cases without index-only lines outside hunks"
    );
}

fn reason(t: &TempRepo, path: &str, orig: Option<&str>) -> Option<Unchanged> {
    assert!(
        file_staging(&t.repo(), path, orig, 3).unwrap().is_none(),
        "{path} has no diff to show"
    );
    unchanged_reason(&t.repo(), path, orig).unwrap()
}

#[cfg(unix)]
#[test]
fn a_mode_change_alone_is_named_staged_or_not() {
    use std::os::unix::fs::PermissionsExt as _;
    let t = TempRepo::new();
    t.commit_file("run.sh", "echo hi\n", "first");
    let file = t.path().join("run.sh");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mode = Some(Unchanged::Mode {
        old: "100644".into(),
        new: "100755".into(),
    });
    assert_eq!(reason(&t, "run.sh", None), mode);
    t.git(["add", "run.sh"]);
    assert_eq!(reason(&t, "run.sh", None), mode);
}

#[test]
fn a_staged_change_undone_in_the_file_is_staged_only() {
    let t = TempRepo::new();
    t.commit_file("a.txt", "one\n", "first");
    t.write("a.txt", "two\n");
    t.git(["add", "a.txt"]);
    t.write("a.txt", "one\n");
    assert_eq!(reason(&t, "a.txt", None), Some(Unchanged::StagedOnly));
}

#[test]
fn empty_new_and_deleted_files_and_plain_renames_are_named() {
    let t = TempRepo::new();
    t.commit_file("gone.txt", "", "first");
    t.commit_file("old.txt", "kept\n", "second");
    t.write("new.txt", "");
    t.git(["add", "new.txt"]);
    t.git(["rm", "-q", "gone.txt"]);
    t.git(["mv", "old.txt", "moved.txt"]);
    assert_eq!(reason(&t, "new.txt", None), Some(Unchanged::EmptyAdded));
    assert_eq!(reason(&t, "gone.txt", None), Some(Unchanged::EmptyDeleted));
    assert_eq!(
        reason(&t, "moved.txt", Some("old.txt")),
        Some(Unchanged::Renamed {
            from: "old.txt".into()
        })
    );
}

#[test]
fn a_file_with_a_diff_or_no_change_has_no_reason() {
    let t = TempRepo::new();
    t.commit_file("a.txt", "one\n", "first");
    assert_eq!(unchanged_reason(&t.repo(), "a.txt", None).unwrap(), None);
    t.write("a.txt", "two\n");
    assert_eq!(unchanged_reason(&t.repo(), "a.txt", None).unwrap(), None);
}
