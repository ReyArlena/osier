//! Whole patches: create unified diffs or mailbox patches from the working tree or commits,
//! inspect a patch, and apply it with a dry run first.
//!
//! Output formats are plain git: `git diff --binary` for working tree changes and
//! `git format-patch --stdout` for commits. Both are what PhpStorm, GitHub and `git apply`
//! read, so patches travel in either direction.

use crate::error::{GitError, Result};
use crate::repo::Repository;

/// What goes into a patch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatchSource {
    /// Working tree against HEAD (staged and unstaged) for `paths` (empty = every tracked
    /// change), plus `untracked` files rendered as additions.
    Worktree {
        paths: Vec<String>,
        untracked: Vec<String>,
    },
    /// Index against HEAD.
    Staged { paths: Vec<String> },
    /// Working tree against the index.
    Unstaged { paths: Vec<String> },
    /// One mailbox patch per commit, in the order given.
    Commits { hashes: Vec<String> },
}

/// Build the patch text.
pub fn create(repo: &Repository, source: &PatchSource) -> Result<String> {
    let mut text = String::new();
    match source {
        PatchSource::Worktree { paths, untracked } => {
            text.push_str(&diff(repo, &["HEAD"], paths)?);
            for path in untracked {
                text.push_str(&crate::diff::untracked_patch_text(repo, path, 3)?);
            }
        }
        PatchSource::Staged { paths } => text.push_str(&diff(repo, &["--cached"], paths)?),
        PatchSource::Unstaged { paths } => text.push_str(&diff(repo, &[], paths)?),
        PatchSource::Commits { hashes } => {
            for hash in hashes {
                let out = repo.git([
                    "format-patch",
                    "--stdout",
                    "--no-signature",
                    "--binary",
                    "-1",
                    hash,
                ])?;
                text.push_str(out.stdout_str()?);
            }
        }
    }
    Ok(text)
}

fn diff(repo: &Repository, mode: &[&str], paths: &[String]) -> Result<String> {
    let mut args: Vec<&str> = vec![
        "diff",
        "--no-color",
        "--no-ext-diff",
        "--no-textconv",
        "--binary",
        "--src-prefix=a/",
        "--dst-prefix=b/",
    ];
    args.extend_from_slice(mode);
    if !paths.is_empty() {
        args.push("--");
        args.extend(paths.iter().map(String::as_str));
    }
    let out = repo.git(args)?;
    Ok(out.stdout_str()?.to_string())
}

/// Turn a patch from another tool into a clean git diff: keep only `diff --git` sections
/// (dropping mail headers, diffstats, signatures and PhpStorm's `Index:` / `===` /
/// `IDEA additional info` preamble), and strip the `(revision <hash>)` / `(date <ms>)`
/// suffix PhpStorm writes after the `---` / `+++` paths. git accepts that suffix only after
/// a tab, and copies through a terminal or chat turn the tab into spaces.
///
/// Text without any `diff --git` line (a plain `diff -u` patch) is returned with only the
/// suffixes stripped.
pub fn normalize(text: &str) -> String {
    if !text.lines().any(|l| l.starts_with("diff --git ")) {
        return strip_plain_suffixes(text);
    }
    enum State {
        Outside,
        Header,
        Hunk { old: u32, new: u32 },
        AfterHunk,
    }
    let mut out = String::with_capacity(text.len());
    let mut state = State::Outside;
    for raw in text.split_inclusive('\n') {
        let line = raw.trim_end_matches('\n').trim_end_matches('\r');
        if let State::AfterHunk = state {
            if line.starts_with('\\') {
                out.push_str(raw);
                continue;
            }
            // Only another hunk of the same file or the next file header continue the
            // patch; anything else (mail signature, PhpStorm preamble) starts junk.
            if !line.starts_with("@@ ") && !line.starts_with("diff --git ") {
                state = State::Outside;
            }
        }
        match state {
            State::Hunk { old, new } => {
                let (old, new) = match line.chars().next() {
                    Some(' ') | None => (old.saturating_sub(1), new.saturating_sub(1)),
                    Some('-') => (old.saturating_sub(1), new),
                    Some('+') => (old, new.saturating_sub(1)),
                    Some('\\') => (old, new),
                    // Malformed: the hunk ended early. Drop the line.
                    _ => {
                        state = State::Outside;
                        continue;
                    }
                };
                out.push_str(raw);
                state = if old == 0 && new == 0 {
                    State::AfterHunk
                } else {
                    State::Hunk { old, new }
                };
                continue;
            }
            State::Header | State::Outside | State::AfterHunk => {}
        }
        if line.starts_with("diff --git ") {
            out.push_str(raw);
            state = State::Header;
        } else if let Some(rest) = line.strip_prefix("@@ ") {
            if matches!(state, State::Outside) {
                continue;
            }
            match hunk_counts(rest) {
                Some((old, new)) => {
                    out.push_str(raw);
                    state = if old == 0 && new == 0 {
                        State::AfterHunk
                    } else {
                        State::Hunk { old, new }
                    };
                }
                None => state = State::Outside,
            }
        } else if let State::Header = state {
            match strip_revision_suffix(line) {
                Some(clean) => {
                    out.push_str(&clean);
                    out.push('\n');
                }
                None => out.push_str(raw),
            }
        }
    }
    out
}

/// Whether `git apply --3way` can work: it needs the blob ids from `index` lines, which
/// PhpStorm and plain `diff -u` patches do not carry.
pub fn supports_three_way(text: &str) -> bool {
    text.lines()
        .any(|l| l.starts_with("index ") && l.contains(".."))
}

fn hunk_counts(rest: &str) -> Option<(u32, u32)> {
    let (ranges, _) = rest.split_once(" @@")?;
    let mut it = ranges.split(' ');
    let old = it.next()?.strip_prefix('-')?;
    let new = it.next()?.strip_prefix('+')?;
    let count = |s: &str| -> Option<u32> {
        match s.split_once(',') {
            Some((_, n)) => n.parse().ok(),
            None => Some(1),
        }
    };
    Some((count(old)?, count(new)?))
}

/// `--- a/x.rs    (revision abc)` to `--- a/x.rs`. Only PhpStorm's known suffixes, and only
/// when separated from the path by whitespace.
fn strip_revision_suffix(line: &str) -> Option<String> {
    let (marker, rest) = if let Some(r) = line.strip_prefix("--- ") {
        ("--- ", r)
    } else {
        ("+++ ", line.strip_prefix("+++ ")?)
    };
    let trimmed = rest.trim_end();
    let inner_end = trimmed.strip_suffix(')')?;
    let open = inner_end.rfind('(')?;
    let inner = &inner_end[open + 1..];
    let known = inner.starts_with("revision ")
        || inner.starts_with("date ")
        || inner == "working copy"
        || inner == "nonexistent";
    if !known {
        return None;
    }
    let before = &inner_end[..open];
    let path = before.trim_end_matches([' ', '\t']);
    if path.len() == before.len() || path.is_empty() {
        return None;
    }
    Some(format!("{marker}{path}"))
}

/// Suffix stripping for patches without git headers: only a `---` line directly followed
/// by a `+++` line counts as a file header.
fn strip_plain_suffixes(text: &str) -> String {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let mut out = String::with_capacity(text.len());
    for (i, raw) in lines.iter().enumerate() {
        let line = raw.trim_end_matches('\n').trim_end_matches('\r');
        let paired = (line.starts_with("--- ")
            && lines.get(i + 1).is_some_and(|n| n.starts_with("+++ ")))
            || (line.starts_with("+++ ") && i > 0 && lines[i - 1].starts_with("--- "));
        match paired.then(|| strip_revision_suffix(line)).flatten() {
            Some(clean) => {
                out.push_str(&clean);
                out.push('\n');
            }
            None => out.push_str(raw),
        }
    }
    out
}

/// Per-file line counts from `git apply --numstat`; binary files report `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchFile {
    pub path: String,
    pub added: Option<u32>,
    pub deleted: Option<u32>,
}

/// List the files a patch touches without applying it.
pub fn inspect(repo: &Repository, patch: &str) -> Result<Vec<PatchFile>> {
    let patch = normalize(patch);
    let out = repo.runner().run_unchecked(
        Some(repo.workdir()),
        ["apply", "--numstat", "-z", "-"],
        Some(patch.as_bytes()),
    )?;
    if out.record.status != Some(0) {
        return Err(GitError::Parse {
            context: "patch",
            detail: first_line(&out.record.stderr)
                .unwrap_or("not a patch git understands")
                .to_string(),
        });
    }
    let text = out.stdout_str()?;
    let mut files = Vec::new();
    for line in text.split('\0').filter(|l| !l.is_empty()) {
        let mut parts = line.splitn(3, '\t');
        let (Some(a), Some(d), Some(path)) = (parts.next(), parts.next(), parts.next()) else {
            return Err(GitError::Parse {
                context: "apply --numstat",
                detail: line.to_string(),
            });
        };
        let path = path.trim_end_matches('\n').to_string();
        files.push(PatchFile {
            path,
            added: a.parse().ok(),
            deleted: d.parse().ok(),
        });
    }
    Ok(files)
}

fn first_line(s: &str) -> Option<&str> {
    s.lines().map(str::trim).find(|l| !l.is_empty())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ApplyOptions {
    /// Fall back to a three-way merge on conflicts (leaves markers, needs blobs in the repo).
    pub three_way: bool,
    pub reverse: bool,
    /// Update the index as well as the working tree.
    pub index: bool,
}

/// What `git apply` reported.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ApplyReport {
    /// Exit status zero.
    pub ok: bool,
    /// Files git refused to patch.
    pub rejected: Vec<String>,
    /// Files applied with conflict markers (three-way mode only).
    pub conflicts: Vec<String>,
    /// Trimmed stderr for the dialog.
    pub message: String,
}

impl ApplyReport {
    /// True when the working tree changed: a clean apply or a three-way apply with markers.
    pub fn changed_files(&self) -> bool {
        self.ok || !self.conflicts.is_empty()
    }
}

/// `git apply --check`: would the patch apply? Nothing is modified.
pub fn check(repo: &Repository, patch: &str, options: ApplyOptions) -> Result<ApplyReport> {
    run(repo, patch, options, true)
}

/// Apply the patch to the working tree (and index with `options.index`).
pub fn apply(repo: &Repository, patch: &str, options: ApplyOptions) -> Result<ApplyReport> {
    run(repo, patch, options, false)
}

fn run(repo: &Repository, patch: &str, options: ApplyOptions, check: bool) -> Result<ApplyReport> {
    let patch = normalize(patch);
    let mut args = vec!["apply", "--whitespace=nowarn"];
    if check {
        args.push("--check");
    }
    if options.three_way {
        args.push("--3way");
    }
    if options.reverse {
        args.push("-R");
    }
    if options.index && !options.three_way {
        args.push("--index");
    }
    args.push("-");
    let out = repo
        .runner()
        .run_unchecked(Some(repo.workdir()), args, Some(patch.as_bytes()))?;
    let Some(status) = out.record.status else {
        return Err(GitError::timeout(0, out.record));
    };
    let mut report = parse_report(&out.record.stderr);
    report.ok = status == 0;
    Ok(report)
}

/// Pull file names out of `git apply` diagnostics.
pub fn parse_report(stderr: &str) -> ApplyReport {
    let mut report = ApplyReport::default();
    for line in stderr.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("error: patch failed: ") {
            // `path:line`
            let path = rest.rsplit_once(':').map(|(p, _)| p).unwrap_or(rest);
            push_unique(&mut report.rejected, path);
        } else if let Some(rest) = line.strip_prefix("error: ") {
            for suffix in [
                ": patch does not apply",
                ": already exists in working directory",
                ": already exists in index",
                ": No such file or directory",
                ": does not exist in index",
                ": does not match index",
            ] {
                if let Some(path) = rest.strip_suffix(suffix) {
                    push_unique(&mut report.rejected, path);
                }
            }
        } else if let Some(rest) = line.strip_prefix("Applied patch to '")
            && let Some(path) = rest.strip_suffix("' with conflicts.")
        {
            push_unique(&mut report.conflicts, path);
        } else if let Some(rest) = line.strip_prefix("U ") {
            push_unique(&mut report.conflicts, rest);
        }
    }
    report.message = stderr
        .lines()
        .map(str::trim)
        .filter(|l| {
            !l.is_empty() && !l.starts_with("Checking patch") && !l.starts_with("Applied patch")
        })
        .collect::<Vec<_>>()
        .join("\n");
    report
}

fn push_unique(list: &mut Vec<String>, path: &str) {
    if !list.iter().any(|p| p == path) {
        list.push(path.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempRepo;

    #[test]
    fn worktree_patch_round_trips_through_apply() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "one\ntwo\nthree\n", "first");
        t.write("a.txt", "one\nTWO\nthree\n");
        t.write("new.txt", "fresh\n");
        let source = PatchSource::Worktree {
            paths: vec![],
            untracked: vec!["new.txt".into()],
        };
        let patch = create(&t.repo(), &source).unwrap();
        assert!(patch.contains("diff --git a/a.txt b/a.txt"));
        assert!(patch.contains("+++ b/new.txt"));

        let files = inspect(&t.repo(), &patch).unwrap();
        let names: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(names, ["a.txt", "new.txt"]);
        assert_eq!(files[0].added, Some(1));
        assert_eq!(files[0].deleted, Some(1));

        // Throw the changes away, then bring them back from the patch.
        t.git(["checkout", "--", "a.txt"]);
        t.remove("new.txt");
        let report = check(&t.repo(), &patch, ApplyOptions::default()).unwrap();
        assert!(report.ok, "{report:?}");
        assert_eq!(
            t.read("a.txt"),
            "one\ntwo\nthree\n",
            "check changes nothing"
        );
        let report = apply(&t.repo(), &patch, ApplyOptions::default()).unwrap();
        assert!(report.ok, "{report:?}");
        assert_eq!(t.read("a.txt"), "one\nTWO\nthree\n");
        assert_eq!(t.read("new.txt"), "fresh\n");

        // And undo it again.
        let report = apply(
            &t.repo(),
            &patch,
            ApplyOptions {
                reverse: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(report.ok, "{report:?}");
        assert_eq!(t.read("a.txt"), "one\ntwo\nthree\n");
        assert!(!t.path().join("new.txt").exists());
    }

    #[test]
    fn staged_and_unstaged_sources_are_separate() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "a\n", "first");
        t.commit_file("b.txt", "b\n", "second");
        t.write("a.txt", "A\n");
        t.git(["add", "a.txt"]);
        t.write("b.txt", "B\n");
        let staged = create(&t.repo(), &PatchSource::Staged { paths: vec![] }).unwrap();
        let unstaged = create(&t.repo(), &PatchSource::Unstaged { paths: vec![] }).unwrap();
        assert!(staged.contains("a/a.txt") && !staged.contains("b.txt"));
        assert!(unstaged.contains("a/b.txt") && !unstaged.contains("a.txt"));
        let only_b = create(
            &t.repo(),
            &PatchSource::Worktree {
                paths: vec!["b.txt".into()],
                untracked: vec![],
            },
        )
        .unwrap();
        assert!(!only_b.contains("a.txt"));
    }

    #[test]
    fn commit_patches_are_mailbox_format_and_apply() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "a\n", "first");
        let second = t.commit_file("a.txt", "a\nb\n", "add b");
        let patch = create(
            &t.repo(),
            &PatchSource::Commits {
                hashes: vec![second],
            },
        )
        .unwrap();
        assert!(patch.starts_with("From "));
        assert!(patch.contains("Subject: [PATCH] add b"));
        t.git(["reset", "--hard", "HEAD~1"]);
        let report = apply(&t.repo(), &patch, ApplyOptions::default()).unwrap();
        assert!(report.ok, "{report:?}");
        assert_eq!(t.read("a.txt"), "a\nb\n");
    }

    #[test]
    fn a_patch_that_does_not_fit_is_rejected_with_the_file_named() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "one\ntwo\nthree\n", "first");
        t.write("a.txt", "one\nTWO\nthree\n");
        let patch = create(&t.repo(), &PatchSource::Unstaged { paths: vec![] }).unwrap();
        // Now the file no longer contains the context the patch expects.
        t.write("a.txt", "completely\ndifferent\n");
        t.git(["commit", "-am", "diverge"]);
        let report = check(&t.repo(), &patch, ApplyOptions::default()).unwrap();
        assert!(!report.ok);
        assert_eq!(report.rejected, ["a.txt"]);
        assert!(
            report.message.contains("patch does not apply"),
            "{}",
            report.message
        );
        assert!(!report.changed_files());
        assert_eq!(t.read("a.txt"), "completely\ndifferent\n");
    }

    #[test]
    fn three_way_apply_leaves_conflict_markers() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "one\ntwo\nthree\n", "first");
        t.write("a.txt", "one\nTWO\nthree\n");
        let patch = create(&t.repo(), &PatchSource::Unstaged { paths: vec![] }).unwrap();
        t.write("a.txt", "one\n2\nthree\n");
        t.git(["commit", "-am", "diverge"]);
        let report = apply(
            &t.repo(),
            &patch,
            ApplyOptions {
                three_way: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!report.ok);
        assert_eq!(report.conflicts, ["a.txt"], "{report:?}");
        assert!(report.changed_files());
        assert!(t.read("a.txt").contains("<<<<<<<"));
    }

    #[test]
    fn garbage_is_not_a_patch() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "a\n", "first");
        assert!(inspect(&t.repo(), "hello world\n").is_err());
        assert!(inspect(&t.repo(), "").is_err() || inspect(&t.repo(), "").unwrap().is_empty());
    }

    use crate::testing::PHPSTORM_PATCH as PHPSTORM;

    fn phpstorm_spaced() -> String {
        PHPSTORM.replace("\t(revision", "    (revision")
    }

    #[test]
    fn normalize_cleans_phpstorm_patches() {
        for input in [PHPSTORM.to_string(), phpstorm_spaced()] {
            let clean = normalize(&input);
            assert!(!clean.contains("Index:"), "{clean}");
            assert!(!clean.contains("IDEA additional info"));
            assert!(!clean.contains("(revision"));
            assert!(!clean.contains("Subject:"));
            assert!(clean.starts_with("diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@"));
            assert!(clean.contains("--- /dev/null\n+++ b/notes/new.md\n"));
            // A content line starting with `--` inside a hunk is kept as content.
            assert!(clean.ends_with("+-- not a signature\n"), "{clean}");
            let files = crate::diff::parse(&clean).unwrap();
            assert_eq!(files.len(), 2);
            assert_eq!(files[1].hunks[0].lines.len(), 2);
        }
        assert!(!supports_three_way(PHPSTORM));
    }

    #[test]
    fn phpstorm_patches_apply_with_tabs_and_with_spaces() {
        for input in [PHPSTORM.to_string(), phpstorm_spaced()] {
            let t = TempRepo::new();
            t.commit_file("a.txt", "one\ntwo\nthree\n", "first");
            let files = inspect(&t.repo(), &input).unwrap();
            let names: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
            assert_eq!(names, ["a.txt", "notes/new.md"]);
            assert!(
                check(&t.repo(), &input, ApplyOptions::default())
                    .unwrap()
                    .ok
            );
            assert!(
                apply(&t.repo(), &input, ApplyOptions::default())
                    .unwrap()
                    .ok
            );
            assert_eq!(t.read("a.txt"), "one\nTWO\nthree\n");
            assert_eq!(t.read("notes/new.md"), "# Notes\n-- not a signature\n");
        }
    }

    #[test]
    fn normalize_keeps_every_hunk_of_a_multi_hunk_file() {
        // Two hunks in one file, PhpStorm style (no context text after the second `@@`).
        let text = "Index: a.txt\n===================================================================\ndiff --git a/a.txt b/a.txt\n--- a/a.txt\t(revision 1)\n+++ b/a.txt\t(revision 2)\n@@ -1,3 +1,3 @@\n l1\n-l2\n+L2\n l3\n@@ -8,3 +8,4 @@\n l8\n l9\n l10\n+l11\n";
        let clean = normalize(text);
        assert_eq!(clean.matches("\n@@ ").count(), 2, "{clean}");
        let files = crate::diff::parse(&clean).unwrap();
        assert_eq!(files[0].hunks.len(), 2);
        let t = TempRepo::new();
        let body: String = (1..=10).map(|n| format!("l{n}\n")).collect();
        t.commit_file("a.txt", &body, "ten lines");
        assert!(apply(&t.repo(), text, ApplyOptions::default()).unwrap().ok);
        let expected: String = (1..=11)
            .map(|n| {
                if n == 2 {
                    "L2\n".to_string()
                } else {
                    format!("l{n}\n")
                }
            })
            .collect();
        assert_eq!(t.read("a.txt"), expected);
    }

    #[test]
    fn normalize_never_drops_hunks_from_git_output() {
        // Round trip: a multi-file, multi-hunk git diff must survive normalize unchanged.
        let t = TempRepo::new();
        let body: String = (1..=40).map(|n| format!("line {n}\n")).collect();
        t.commit_file("a.txt", &body, "a");
        t.commit_file("b.txt", &body, "b");
        let changed = body
            .replace("line 2\n", "LINE 2\n")
            .replace("line 30\n", "LINE 30\n");
        t.write("a.txt", &changed);
        t.write("b.txt", &changed);
        let diff = t.git(["diff"]);
        assert_eq!(diff.matches("\n@@ ").count(), 4);
        assert_eq!(normalize(&diff), diff);
    }

    #[test]
    fn normalize_keeps_git_patches_and_drops_mail_trailers() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "a\n", "first");
        let hash = t.commit_file("a.txt", "a\nb\n", "add b");
        // format-patch with its default signature trailer, as another git user would send it.
        let mail = t.git(["format-patch", "--stdout", "-1", &hash]);
        assert!(mail.contains("\n-- \n"), "fixture has a signature");
        let clean = normalize(&mail);
        assert!(clean.starts_with("diff --git "));
        assert!(!clean.contains("\n-- \n"));
        assert!(clean.contains("\nindex "));
        assert!(supports_three_way(&clean));
        let files = crate::diff::parse(
            &mail
                .split_once("diff --git")
                .map(|(_, r)| format!("diff --git{r}"))
                .unwrap(),
        );
        assert!(files.is_err() || files.unwrap()[0].hunks[0].lines.len() == 2);
        assert_eq!(
            crate::diff::parse(&clean).unwrap()[0].hunks[0].lines.len(),
            2
        );
        // A plain git diff passes through unchanged.
        t.write("a.txt", "x\n");
        let plain = t.git(["diff"]);
        assert_eq!(normalize(&plain), plain);
    }

    #[test]
    fn plain_unified_diffs_get_suffixes_stripped_only_on_header_pairs() {
        let text = "--- a.txt    (revision 1)\n+++ a.txt    (working copy)\n@@ -1 +1 @@\n-old (revision 3)\n+new\n";
        let clean = normalize(text);
        assert!(clean.starts_with("--- a.txt\n+++ a.txt\n"));
        assert!(
            clean.contains("-old (revision 3)\n"),
            "content untouched: {clean}"
        );
    }

    #[test]
    fn parses_diagnostics() {
        let r = parse_report(
            "Checking patch a.txt...\nerror: patch failed: src/x.rs:12\nerror: src/x.rs: patch does not apply\nerror: new.txt: already exists in working directory\nFalling back to three-way merge...\nApplied patch to 'y.rs' with conflicts.\nU y.rs\n",
        );
        assert_eq!(r.rejected, ["src/x.rs", "new.txt"]);
        assert_eq!(r.conflicts, ["y.rs"]);
        assert!(!r.message.contains("Checking patch"));
    }
}
