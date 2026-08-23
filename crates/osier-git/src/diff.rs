//! Unified diff parsing into files, hunks and lines.

use crate::error::{GitError, Result};
use crate::repo::Repository;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffFile {
    /// Path after the change (`b/` side).
    pub path: String,
    /// Path before the change when renamed or copied.
    pub old_path: Option<String>,
    pub status: FileStatus,
    pub binary: bool,
    pub old_mode: Option<String>,
    pub new_mode: Option<String>,
    pub hunks: Vec<Hunk>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Modified,
    Added,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    /// Text after the second `@@`.
    pub context: String,
    pub lines: Vec<Line>,
    /// The raw `@@ ... @@` header, needed to rebuild patches.
    pub header: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub kind: LineKind,
    /// Content without the leading marker and without the trailing newline.
    pub text: String,
    /// `\ No newline at end of file` followed this line.
    pub no_newline: bool,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Added,
    Removed,
}

impl DiffFile {
    pub fn added(&self) -> usize {
        self.hunks
            .iter()
            .flat_map(|h| &h.lines)
            .filter(|l| l.kind == LineKind::Added)
            .count()
    }

    pub fn removed(&self) -> usize {
        self.hunks
            .iter()
            .flat_map(|h| &h.lines)
            .filter(|l| l.kind == LineKind::Removed)
            .count()
    }
}

/// What to diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffTarget {
    /// Working tree against the index (unstaged changes).
    Unstaged,
    /// Index against HEAD (staged changes).
    Staged,
    /// Working tree against HEAD.
    WorktreeToHead,
    /// Changes introduced by one commit (against its first parent, or empty tree for a root).
    Commit(String),
    /// Arbitrary `from..to`.
    Range { from: String, to: String },
}

#[derive(Debug, Clone)]
pub struct DiffOptions {
    pub target: DiffTarget,
    /// Limit to these paths. Empty means everything.
    pub paths: Vec<String>,
    pub context_lines: u32,
    pub ignore_whitespace: bool,
    pub detect_renames: bool,
}

impl DiffOptions {
    pub fn new(target: DiffTarget) -> Self {
        Self {
            target,
            paths: Vec::new(),
            context_lines: 3,
            ignore_whitespace: false,
            detect_renames: true,
        }
    }

    pub fn path(mut self, p: impl Into<String>) -> Self {
        self.paths.push(p.into());
        self
    }
}

/// Arguments for `git diff` / `git show` matching `options`.
fn args_for(options: &DiffOptions) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    match &options.target {
        DiffTarget::Commit(rev) => {
            // A merge is compared with its first parent; the combined format has no two-sided
            // representation.
            args.extend([
                "show".into(),
                "--format=".into(),
                "--first-parent".into(),
                "--no-show-signature".into(),
                rev.clone(),
            ]);
        }
        other => {
            args.push("diff".into());
            match other {
                DiffTarget::Unstaged => {}
                DiffTarget::Staged => args.push("--cached".into()),
                DiffTarget::WorktreeToHead => args.push("HEAD".into()),
                DiffTarget::Range { from, to } => args.push(format!("{from}..{to}")),
                DiffTarget::Commit(_) => {}
            }
        }
    }
    args.push("--no-color".into());
    args.push("--no-ext-diff".into());
    args.push("--no-textconv".into());
    args.push("--patch".into());
    args.push("--src-prefix=a/".into());
    args.push("--dst-prefix=b/".into());
    args.push(format!("-U{}", options.context_lines));
    args.push(if options.detect_renames {
        "-M".into()
    } else {
        "--no-renames".into()
    });
    if options.ignore_whitespace {
        args.push("-w".into());
    }
    args.push("--".into());
    args.extend(options.paths.iter().cloned());
    args
}

/// The raw unified diff text, for viewers that parse patches themselves.
pub fn patch_text(repo: &Repository, options: &DiffOptions) -> Result<String> {
    let out = repo.git(args_for(options))?;
    Ok(out.stdout_str()?.to_string())
}

/// Parsed diff.
pub fn diff(repo: &Repository, options: &DiffOptions) -> Result<Vec<DiffFile>> {
    parse(&patch_text(repo, options)?)
}

/// Raw patch for an untracked file, rendered as an addition.
pub fn untracked_patch_text(repo: &Repository, path: &str, context_lines: u32) -> Result<String> {
    let out = repo.runner().run_unchecked(
        Some(repo.workdir()),
        [
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--no-index",
            &format!("-U{context_lines}"),
            "--",
            "/dev/null",
            path,
        ],
        None,
    )?;
    // `--no-index` exits 1 when there are differences, which is the expected case.
    match out.record.status {
        Some(0) | Some(1) => {
            // Rewrite the headers so the file shows under its repository path.
            let text = out.stdout_str()?;
            Ok(text.replacen(
                &format!("diff --git a/dev/null b/{path}"),
                &format!("diff --git a/{path} b/{path}"),
                1,
            ))
        }
        Some(s) => Err(GitError::failed(s, out.record)),
        None => Err(GitError::timeout(0, out.record)),
    }
}

/// Diff of an untracked file against nothing, so it renders like an addition.
pub fn diff_untracked(repo: &Repository, path: &str, context_lines: u32) -> Result<Vec<DiffFile>> {
    let mut files = parse(&untracked_patch_text(repo, path, context_lines)?)?;
    for f in &mut files {
        f.status = FileStatus::Added;
        f.path = path.to_string();
    }
    Ok(files)
}

/// Parse unified diff text as produced by `git diff` / `git show`.
pub fn parse(text: &str) -> Result<Vec<DiffFile>> {
    let mut files: Vec<DiffFile> = Vec::new();
    let mut lines = text.split_inclusive('\n').peekable();

    while let Some(line) = lines.next() {
        let line = line.strip_suffix('\n').unwrap_or(line);
        if let Some(rest) = line.strip_prefix("diff --git ") {
            let (a, b) = split_git_header(rest);
            files.push(DiffFile {
                path: b,
                old_path: None,
                status: FileStatus::Modified,
                binary: false,
                old_mode: None,
                new_mode: None,
                hunks: Vec::new(),
            });
            let file = files
                .last_mut()
                .ok_or_else(|| GitError::parse("diff", "push"))?;
            let _ = a;
            // Extended headers until `---`, `Binary files`, or the next `diff --git`.
            while let Some(peek) = lines.peek() {
                let h = peek.strip_suffix('\n').unwrap_or(peek);
                if h.starts_with("diff --git ") || h.starts_with("@@ ") {
                    break;
                }
                let h = h.to_string();
                lines.next();
                if let Some(m) = h.strip_prefix("old mode ") {
                    file.old_mode = Some(m.to_string());
                } else if let Some(m) = h.strip_prefix("new mode ") {
                    file.new_mode = Some(m.to_string());
                    if file.status == FileStatus::Modified {
                        file.status = FileStatus::TypeChanged;
                    }
                } else if let Some(m) = h.strip_prefix("new file mode ") {
                    file.status = FileStatus::Added;
                    file.new_mode = Some(m.to_string());
                } else if let Some(m) = h.strip_prefix("deleted file mode ") {
                    file.status = FileStatus::Deleted;
                    file.old_mode = Some(m.to_string());
                } else if let Some(p) = h.strip_prefix("rename from ") {
                    file.status = FileStatus::Renamed;
                    file.old_path = Some(p.to_string());
                } else if let Some(p) = h.strip_prefix("rename to ") {
                    file.path = p.to_string();
                } else if let Some(p) = h.strip_prefix("copy from ") {
                    file.status = FileStatus::Copied;
                    file.old_path = Some(p.to_string());
                } else if let Some(p) = h.strip_prefix("copy to ") {
                    file.path = p.to_string();
                } else if h.starts_with("Binary files ") || h.starts_with("GIT binary patch") {
                    file.binary = true;
                } else if let Some(p) = h.strip_prefix("+++ ") {
                    if let Some(p) = p.strip_prefix("b/") {
                        file.path = p.to_string();
                    }
                } else if let Some(p) = h.strip_prefix("--- ")
                    && let Some(p) = p.strip_prefix("a/")
                    && file.old_path.is_none()
                    && file.status != FileStatus::Added
                {
                    // Plain modification: old path equals new path, keep None. Record only when
                    // it differs (should not happen without a rename header).
                    if p != file.path && file.status == FileStatus::Modified {
                        file.old_path = Some(p.to_string());
                    }
                }
                // `index`, `similarity index`, `dissimilarity index`, `--- /dev/null`: ignored.
            }
            // A new-file diff where `+++ b/path` was found keeps that path. Type changes with
            // mode headers only are fine too.
            if file.status == FileStatus::TypeChanged && file.old_mode.is_none() {
                file.status = FileStatus::Modified;
            }
        } else if let Some(rest) = line.strip_prefix("@@ ") {
            let file = files
                .last_mut()
                .ok_or_else(|| GitError::parse("diff", "hunk before file header"))?;
            let mut hunk = parse_hunk_header(line, rest)?;
            let (mut old_no, mut new_no) = (hunk.old_start, hunk.new_start);
            while let Some(peek) = lines.peek() {
                let l = peek.strip_suffix('\n').unwrap_or(peek);
                if l.starts_with("diff --git ") || l.starts_with("@@ ") {
                    break;
                }
                let l = l.to_string();
                lines.next();
                if l.starts_with('\\') {
                    if let Some(last) = hunk.lines.last_mut() {
                        last.no_newline = true;
                    }
                    continue;
                }
                let (kind, text) = match l.chars().next() {
                    Some('+') => (LineKind::Added, &l[1..]),
                    Some('-') => (LineKind::Removed, &l[1..]),
                    Some(' ') => (LineKind::Context, &l[1..]),
                    // Empty context line when the source line was empty and git omitted the space.
                    None => (LineKind::Context, ""),
                    _ => return Err(GitError::parse("diff", format!("unexpected line {l:?}"))),
                };
                let (o, n) = match kind {
                    LineKind::Context => {
                        let r = (Some(old_no), Some(new_no));
                        old_no += 1;
                        new_no += 1;
                        r
                    }
                    LineKind::Added => {
                        let r = (None, Some(new_no));
                        new_no += 1;
                        r
                    }
                    LineKind::Removed => {
                        let r = (Some(old_no), None);
                        old_no += 1;
                        r
                    }
                };
                hunk.lines.push(Line {
                    kind,
                    text: text.to_string(),
                    no_newline: false,
                    old_no: o,
                    new_no: n,
                });
            }
            file.hunks.push(hunk);
        } else if line.is_empty() {
            continue;
        } else {
            return Err(GitError::parse(
                "diff",
                format!("unexpected header {line:?}"),
            ));
        }
    }
    Ok(files)
}

/// `a/path b/path` from the `diff --git` line. Paths with spaces are ambiguous here; the
/// `---`/`+++` and rename headers that follow are authoritative and override this.
fn split_git_header(rest: &str) -> (String, String) {
    // Try the unambiguous split: the string contains " b/" exactly where the second path starts
    // when both paths are equal (the common case).
    let (a, b) = rest.split_once(" b/").unwrap_or((rest, ""));
    (a.strip_prefix("a/").unwrap_or(a).to_string(), b.to_string())
}

fn parse_hunk_header(full: &str, rest: &str) -> Result<Hunk> {
    // -old_start[,old_lines] +new_start[,new_lines] @@ context
    let (ranges, context) = rest
        .split_once(" @@")
        .ok_or_else(|| GitError::parse("diff", format!("bad hunk {full:?}")))?;
    let mut it = ranges.split(' ');
    let old = it.next().and_then(|s| s.strip_prefix('-'));
    let new = it.next().and_then(|s| s.strip_prefix('+'));
    let (old, new) = match (old, new) {
        (Some(o), Some(n)) => (o, n),
        _ => return Err(GitError::parse("diff", format!("bad hunk {full:?}"))),
    };
    let range = |s: &str| -> Result<(u32, u32)> {
        let (start, len) = s.split_once(',').unwrap_or((s, "1"));
        Ok((
            start
                .parse()
                .map_err(|_| GitError::parse("diff", s.to_string()))?,
            len.parse()
                .map_err(|_| GitError::parse("diff", s.to_string()))?,
        ))
    };
    let (old_start, old_lines) = range(old)?;
    let (new_start, new_lines) = range(new)?;
    Ok(Hunk {
        old_start,
        old_lines,
        new_start,
        new_lines,
        context: context.strip_prefix(' ').unwrap_or(context).to_string(),
        lines: Vec::new(),
        header: full.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempRepo;

    const SAMPLE: &str = "diff --git a/a.txt b/a.txt\nindex 1111..2222 100644\n--- a/a.txt\n+++ b/a.txt\n@@ -1,3 +1,3 @@ fn main()\n a\n-b\n+B\n c\ndiff --git a/bin.dat b/bin.dat\nnew file mode 100644\nindex 0000000..8352675\nBinary files /dev/null and b/bin.dat differ\ndiff --git a/nonl.txt b/nonl.txt\nnew file mode 100644\nindex 0000000..c1b0730\n--- /dev/null\n+++ b/nonl.txt\n@@ -0,0 +1 @@\n+x\n\\ No newline at end of file\ndiff --git a/old.txt b/new name.txt\nsimilarity index 100%\nrename from old.txt\nrename to new name.txt\n";

    #[test]
    fn parses_modified_binary_nonewline_and_rename() {
        let files = parse(SAMPLE).unwrap();
        assert_eq!(files.len(), 4);
        let a = &files[0];
        assert_eq!(a.path, "a.txt");
        assert_eq!(a.status, FileStatus::Modified);
        assert_eq!(a.hunks.len(), 1);
        let h = &a.hunks[0];
        assert_eq!(
            (h.old_start, h.old_lines, h.new_start, h.new_lines),
            (1, 3, 1, 3)
        );
        assert_eq!(h.context, "fn main()");
        assert_eq!(h.lines.len(), 4);
        assert_eq!(h.lines[1].kind, LineKind::Removed);
        assert_eq!(h.lines[1].old_no, Some(2));
        assert_eq!(h.lines[2].kind, LineKind::Added);
        assert_eq!(h.lines[2].new_no, Some(2));
        assert_eq!(h.lines[3].old_no, Some(3));
        assert_eq!((a.added(), a.removed()), (1, 1));

        let bin = &files[1];
        assert!(bin.binary);
        assert_eq!(bin.status, FileStatus::Added);
        assert!(bin.hunks.is_empty());

        let nonl = &files[2];
        assert_eq!(nonl.status, FileStatus::Added);
        assert!(nonl.hunks[0].lines[0].no_newline);

        let ren = &files[3];
        assert_eq!(ren.status, FileStatus::Renamed);
        assert_eq!(ren.old_path.as_deref(), Some("old.txt"));
        assert_eq!(ren.path, "new name.txt");
    }

    #[test]
    fn empty_input_is_no_files() {
        assert!(parse("").unwrap().is_empty());
    }

    #[test]
    fn garbage_fails_closed() {
        assert!(parse("hello\n").is_err());
        assert!(parse("@@ -1 +1 @@\n x\n").is_err());
    }

    #[test]
    fn live_staged_unstaged_commit_and_untracked() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "a\nb\nc\n", "first");
        t.write("a.txt", "a\nB\nc\n");
        t.git(["add", "a.txt"]);
        t.write("a.txt", "a\nB\nc\nd\n");
        t.write("u.txt", "u\n");
        let repo = t.repo();

        let staged = diff(&repo, &DiffOptions::new(DiffTarget::Staged)).unwrap();
        assert_eq!(staged.len(), 1);
        assert_eq!((staged[0].added(), staged[0].removed()), (1, 1));

        let unstaged = diff(&repo, &DiffOptions::new(DiffTarget::Unstaged)).unwrap();
        assert_eq!((unstaged[0].added(), unstaged[0].removed()), (1, 0));

        let head = t.head();
        let commit = diff(&repo, &DiffOptions::new(DiffTarget::Commit(head))).unwrap();
        assert_eq!(
            commit[0].status,
            FileStatus::Added,
            "root commit diffs against empty tree"
        );
        assert_eq!(commit[0].added(), 3);

        let untracked = diff_untracked(&repo, "u.txt", 3).unwrap();
        assert_eq!(untracked.len(), 1);
        assert_eq!(untracked[0].path, "u.txt");
        assert_eq!(untracked[0].status, FileStatus::Added);
        assert_eq!(untracked[0].added(), 1);
    }

    #[test]
    fn live_rename_with_space_in_name() {
        let t = TempRepo::new();
        t.commit_file("old.txt", "same\n", "first");
        t.git(["mv", "old.txt", "new name.txt"]);
        let files = diff(&t.repo(), &DiffOptions::new(DiffTarget::Staged)).unwrap();
        assert_eq!(files[0].status, FileStatus::Renamed);
        assert_eq!(files[0].path, "new name.txt");
        assert_eq!(files[0].old_path.as_deref(), Some("old.txt"));
    }
}
