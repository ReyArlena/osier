//! Working tree status from `git status --porcelain=v2 -z`.

use crate::error::{GitError, Result};
use crate::repo::Repository;

/// State of one path in the index and working tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusEntry {
    pub path: String,
    /// Original path for renames and copies.
    pub orig_path: Option<String>,
    /// Change between HEAD and index (`.` = none, `M`, `A`, `D`, `R`, `C`, `T`, `U`).
    pub index: char,
    /// Change between index and working tree.
    pub worktree: char,
    pub kind: EntryKind,
    pub submodule: bool,
    /// Rename/copy similarity score, when `kind` is a rename or copy.
    pub score: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    /// Tracked, changed in index and/or worktree.
    Changed,
    RenamedOrCopied,
    Unmerged,
    Untracked,
    Ignored,
}

impl StatusEntry {
    pub fn is_staged(&self) -> bool {
        matches!(self.kind, EntryKind::Changed | EntryKind::RenamedOrCopied) && self.index != '.'
    }

    pub fn is_unstaged(&self) -> bool {
        matches!(self.kind, EntryKind::Changed | EntryKind::RenamedOrCopied) && self.worktree != '.'
    }

    pub fn is_untracked(&self) -> bool {
        self.kind == EntryKind::Untracked
    }

    pub fn is_conflicted(&self) -> bool {
        self.kind == EntryKind::Unmerged
    }
}

/// Branch and upstream information from the status header.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BranchInfo {
    /// `None` on an unborn branch (no commits yet).
    pub oid: Option<String>,
    /// Branch name, or `None` when HEAD is detached.
    pub head: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub stash_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RepoStatus {
    pub branch: BranchInfo,
    pub entries: Vec<StatusEntry>,
}

impl RepoStatus {
    pub fn staged(&self) -> impl Iterator<Item = &StatusEntry> {
        self.entries.iter().filter(|e| e.is_staged())
    }

    pub fn unstaged(&self) -> impl Iterator<Item = &StatusEntry> {
        self.entries.iter().filter(|e| e.is_unstaged())
    }

    pub fn untracked(&self) -> impl Iterator<Item = &StatusEntry> {
        self.entries.iter().filter(|e| e.is_untracked())
    }

    pub fn conflicted(&self) -> impl Iterator<Item = &StatusEntry> {
        self.entries.iter().filter(|e| e.is_conflicted())
    }

    pub fn is_clean(&self) -> bool {
        self.entries.iter().all(|e| e.kind == EntryKind::Ignored)
    }
}

/// Options for [`status`].
#[derive(Debug, Clone, Copy, Default)]
pub struct StatusOptions {
    /// Include ignored files (`!` entries).
    pub ignored: bool,
}

/// Query the status. The runner sets `GIT_OPTIONAL_LOCKS=0`, so this never writes the index.
pub fn status(repo: &Repository, options: StatusOptions) -> Result<RepoStatus> {
    let mut args = vec![
        "status",
        "--porcelain=v2",
        "-z",
        "--branch",
        "--show-stash",
        "--untracked-files=all",
        "--renames",
    ];
    if options.ignored {
        args.push("--ignored=matching");
    }
    let out = repo.git(args)?;
    parse(&out.stdout)
}

/// Parse `--porcelain=v2 -z` output.
pub fn parse(bytes: &[u8]) -> Result<RepoStatus> {
    let mut status = RepoStatus::default();
    let mut fields = bytes.split(|b| *b == 0).peekable();
    while let Some(raw) = fields.next() {
        if raw.is_empty() {
            continue;
        }
        let line = std::str::from_utf8(raw).map_err(|_| GitError::Utf8("status"))?;
        let mut it = line.splitn(2, ' ');
        let tag = it.next().unwrap_or("");
        let rest = it.next().unwrap_or("");
        match tag {
            "#" => parse_header(rest, &mut status.branch)?,
            "1" => status.entries.push(parse_changed(rest)?),
            "2" => {
                let orig = fields
                    .next()
                    .ok_or_else(|| GitError::parse("status", "rename without original path"))?;
                let orig = std::str::from_utf8(orig).map_err(|_| GitError::Utf8("status"))?;
                status.entries.push(parse_renamed(rest, orig)?);
            }
            "u" => status.entries.push(parse_unmerged(rest)?),
            "?" => status.entries.push(simple(rest, EntryKind::Untracked)),
            "!" => status.entries.push(simple(rest, EntryKind::Ignored)),
            other => {
                return Err(GitError::parse(
                    "status",
                    format!("unknown entry type {other:?} in {line:?}"),
                ));
            }
        }
    }
    Ok(status)
}

fn parse_header(rest: &str, b: &mut BranchInfo) -> Result<()> {
    let (key, value) = rest.split_once(' ').unwrap_or((rest, ""));
    match key {
        "branch.oid" => b.oid = (value != "(initial)").then(|| value.to_string()),
        "branch.head" => b.head = (value != "(detached)").then(|| value.to_string()),
        "branch.upstream" => b.upstream = Some(value.to_string()),
        "branch.ab" => {
            for part in value.split(' ') {
                if let Some(n) = part.strip_prefix('+') {
                    b.ahead = n.parse().map_err(|_| GitError::parse("status", part))?;
                } else if let Some(n) = part.strip_prefix('-') {
                    b.behind = n.parse().map_err(|_| GitError::parse("status", part))?;
                }
            }
        }
        "stash" => {
            b.stash_count = value
                .parse()
                .map_err(|_| GitError::parse("status", value))?
        }
        // Future headers are not an error.
        _ => {}
    }
    Ok(())
}

fn xy(s: &str) -> Result<(char, char)> {
    let mut c = s.chars();
    match (c.next(), c.next(), c.next()) {
        (Some(x), Some(y), None) => Ok((x, y)),
        _ => Err(GitError::parse("status", format!("bad XY {s:?}"))),
    }
}

fn simple(path: &str, kind: EntryKind) -> StatusEntry {
    StatusEntry {
        path: path.to_string(),
        orig_path: None,
        index: '.',
        worktree: match kind {
            EntryKind::Untracked => '?',
            EntryKind::Ignored => '!',
            _ => '.',
        },
        kind,
        submodule: false,
        score: None,
    }
}

// 1 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>
fn parse_changed(rest: &str) -> Result<StatusEntry> {
    let parts: Vec<&str> = rest.splitn(8, ' ').collect();
    if parts.len() != 8 {
        return Err(GitError::parse("status", format!("short entry {rest:?}")));
    }
    let (index, worktree) = xy(parts[0])?;
    Ok(StatusEntry {
        path: parts[7].to_string(),
        orig_path: None,
        index,
        worktree,
        kind: EntryKind::Changed,
        submodule: parts[1].starts_with('S'),
        score: None,
    })
}

// 2 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <X><score> <path>   then NUL <origPath>
fn parse_renamed(rest: &str, orig: &str) -> Result<StatusEntry> {
    let parts: Vec<&str> = rest.splitn(9, ' ').collect();
    if parts.len() != 9 {
        return Err(GitError::parse("status", format!("short rename {rest:?}")));
    }
    let (index, worktree) = xy(parts[0])?;
    let score = parts[7].get(1..).and_then(|n| n.parse::<u8>().ok());
    Ok(StatusEntry {
        path: parts[8].to_string(),
        orig_path: Some(orig.to_string()),
        index,
        worktree,
        kind: EntryKind::RenamedOrCopied,
        submodule: parts[1].starts_with('S'),
        score,
    })
}

// u <XY> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path>
fn parse_unmerged(rest: &str) -> Result<StatusEntry> {
    let parts: Vec<&str> = rest.splitn(10, ' ').collect();
    if parts.len() != 10 {
        return Err(GitError::parse(
            "status",
            format!("short unmerged {rest:?}"),
        ));
    }
    let (index, worktree) = xy(parts[0])?;
    Ok(StatusEntry {
        path: parts[9].to_string(),
        orig_path: None,
        index,
        worktree,
        kind: EntryKind::Unmerged,
        submodule: parts[1].starts_with('S'),
        score: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempRepo;

    fn find<'a>(s: &'a RepoStatus, path: &str) -> &'a StatusEntry {
        s.entries
            .iter()
            .find(|e| e.path == path)
            .unwrap_or_else(|| panic!("no entry for {path}: {:?}", s.entries))
    }

    #[test]
    fn parses_fixture_with_every_entry_type() {
        let raw = b"# branch.oid abc123\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +2 -1\0# stash 3\0\
1 .M N... 100644 100644 100644 de98 de98 a.txt\0\
1 D. N... 100644 000000 000000 4bcf 0000 sub/deleted.txt\0\
2 R. N... 100644 100644 100644 aaaa aaaa R100 new name.txt\0old.txt\0\
u UU N... 100644 100644 100644 100644 1111 2222 3333 conflict.txt\0\
? untracked.txt\0\
! ignored.log\0";
        let s = parse(raw).unwrap();
        assert_eq!(s.branch.oid.as_deref(), Some("abc123"));
        assert_eq!(s.branch.head.as_deref(), Some("main"));
        assert_eq!(s.branch.upstream.as_deref(), Some("origin/main"));
        assert_eq!(
            (s.branch.ahead, s.branch.behind, s.branch.stash_count),
            (2, 1, 3)
        );
        assert_eq!(s.entries.len(), 6);
        let a = find(&s, "a.txt");
        assert!(a.is_unstaged() && !a.is_staged());
        let d = find(&s, "sub/deleted.txt");
        assert!(d.is_staged() && d.index == 'D');
        let r = find(&s, "new name.txt");
        assert_eq!(r.orig_path.as_deref(), Some("old.txt"));
        assert_eq!(r.score, Some(100));
        assert!(r.is_staged());
        assert!(find(&s, "conflict.txt").is_conflicted());
        assert!(find(&s, "untracked.txt").is_untracked());
        assert_eq!(find(&s, "ignored.log").kind, EntryKind::Ignored);
    }

    #[test]
    fn unborn_and_detached_headers() {
        let s = parse(b"# branch.oid (initial)\0# branch.head (detached)\0").unwrap();
        assert_eq!(s.branch.oid, None);
        assert_eq!(s.branch.head, None);
        assert!(s.is_clean());
    }

    #[test]
    fn unknown_entry_type_fails_closed() {
        assert!(parse(b"9 something\0").is_err());
    }

    #[test]
    fn live_status_on_temp_repo() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "a\nb\n", "first");
        t.commit_file("old.txt", "old\n", "second");
        t.write("a.txt", "a\nB\n");
        t.write("new.txt", "new\n");
        t.git(["mv", "old.txt", "renamed.txt"]);
        t.write(".gitignore", "*.log\n");
        t.write("x.log", "log\n");
        let s = status(&t.repo(), StatusOptions { ignored: true }).unwrap();
        assert_eq!(s.branch.head.as_deref(), Some("main"));
        assert!(s.branch.oid.is_some());
        assert_eq!(s.branch.upstream, None);
        assert!(find(&s, "a.txt").is_unstaged());
        assert!(find(&s, "new.txt").is_untracked());
        assert!(find(&s, ".gitignore").is_untracked());
        let r = find(&s, "renamed.txt");
        assert_eq!(r.kind, EntryKind::RenamedOrCopied);
        assert_eq!(r.orig_path.as_deref(), Some("old.txt"));
        assert_eq!(find(&s, "x.log").kind, EntryKind::Ignored);
        assert_eq!(s.staged().count(), 1);
        assert_eq!(s.unstaged().count(), 1);
        assert_eq!(s.untracked().count(), 2);
    }

    #[test]
    fn ahead_behind_with_upstream() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "a\n", "first");
        t.add_bare_origin();
        t.git(["push", "-q", "-u", "origin", "main"]);
        t.commit_file("b.txt", "b\n", "second");
        let s = status(&t.repo(), StatusOptions::default()).unwrap();
        assert_eq!(s.branch.upstream.as_deref(), Some("origin/main"));
        assert_eq!((s.branch.ahead, s.branch.behind), (1, 0));
    }

    #[test]
    fn conflict_shows_as_unmerged() {
        let t = TempRepo::new();
        t.commit_file("c.txt", "base\n", "base");
        t.checkout_new("other");
        t.commit_file("c.txt", "theirs\n", "theirs");
        t.checkout("main");
        t.commit_file("c.txt", "ours\n", "ours");
        assert!(t.try_git(["merge", "other"]).is_err());
        let s = status(&t.repo(), StatusOptions::default()).unwrap();
        assert!(find(&s, "c.txt").is_conflicted());
        assert_eq!(t.repo().in_progress(), Some(crate::repo::InProgress::Merge));
    }
}

#[cfg(test)]
mod perf_tests {
    use std::time::Instant;

    use super::*;
    use crate::testing::TempRepo;

    /// Rough guard against quadratic parsing: 10k tracked files, 2k of them modified.
    /// Run with `cargo test -p osier-git -- --ignored status_of_ten_thousand_files`.
    #[test]
    #[ignore = "slow: builds a 10k-file repository"]
    fn status_of_ten_thousand_files_is_quick() {
        let t = TempRepo::new();
        for dir in 0..100 {
            std::fs::create_dir_all(t.path().join(format!("d{dir}"))).unwrap();
            for f in 0..100 {
                std::fs::write(t.path().join(format!("d{dir}/f{f}.txt")), "x\n").unwrap();
            }
        }
        t.git(["add", "-A"]);
        t.commit("many files");
        for dir in 0..20 {
            for f in 0..100 {
                std::fs::write(t.path().join(format!("d{dir}/f{f}.txt")), "y\n").unwrap();
            }
        }
        let started = Instant::now();
        let s = status(&t.repo(), StatusOptions::default()).unwrap();
        let took = started.elapsed();
        assert_eq!(s.unstaged().count(), 2000);
        eprintln!("status of 10k files: {took:?}");
        assert!(took.as_secs() < 5, "status took {took:?}");
    }
}
