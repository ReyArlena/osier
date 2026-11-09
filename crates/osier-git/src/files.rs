//! Working tree file listings, gitignore-aware through git itself.

use crate::error::{GitError, Result};
use crate::repo::Repository;

/// Paths tracked in the index (NUL separated, so any file name works).
pub fn tracked(repo: &Repository) -> Result<Vec<String>> {
    let out = repo.git(["ls-files", "-z", "--cached"])?;
    Ok(split(&out.stdout))
}

/// Untracked files that are not ignored.
pub fn untracked(repo: &Repository) -> Result<Vec<String>> {
    let out = repo.git(["ls-files", "-z", "--others", "--exclude-standard"])?;
    Ok(split(&out.stdout))
}

/// Ignored files (for the "show ignored" toggle). Directories that are ignored as a whole are
/// reported as `dir/` entries by git with `--directory`, so this lists files only.
pub fn ignored(repo: &Repository) -> Result<Vec<String>> {
    let out = repo.git([
        "ls-files",
        "-z",
        "--others",
        "--ignored",
        "--exclude-standard",
    ])?;
    Ok(split(&out.stdout))
}

/// The rule that makes git ignore a path: the last pattern that matches it, which is the one
/// that decides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoreRule {
    /// The file holding the pattern, as git names it: relative to the working tree
    /// (`.gitignore`, `site/.gitignore`, `.git/info/exclude`) or absolute (the global excludes
    /// file, or `info/exclude` of a linked worktree's main repository).
    pub source: String,
    /// 1-based line of the pattern in `source`.
    pub line: u32,
    pub pattern: String,
}

/// The deciding rule for each of `paths` (ignored files, eg from [`ignored`]), in git's
/// order. Paths git does not ignore are left out.
pub fn ignore_rules(repo: &Repository, paths: &[String]) -> Result<Vec<(String, IgnoreRule)>> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let mut input = Vec::new();
    for p in paths {
        input.extend_from_slice(p.as_bytes());
        input.push(0);
    }
    // Exit 1: none of the paths is ignored.
    let out = repo.runner().run_allowing_with_stdin(
        Some(repo.workdir()),
        ["check-ignore", "--verbose", "-z", "--stdin"],
        &input,
        &[0, 1],
    )?;
    parse_ignore_rules(&out.stdout)
}

/// Parse `git check-ignore --verbose -z`: `source NUL line NUL pattern NUL path NUL` per path.
pub fn parse_ignore_rules(bytes: &[u8]) -> Result<Vec<(String, IgnoreRule)>> {
    let mut fields: Vec<&[u8]> = bytes.split(|b| *b == 0).collect();
    if fields.last().is_some_and(|f| f.is_empty()) {
        fields.pop();
    }
    if !fields.len().is_multiple_of(4) {
        return Err(GitError::parse(
            "check-ignore",
            format!("{} fields, expected groups of 4", fields.len()),
        ));
    }
    let text = |f: &[u8]| String::from_utf8_lossy(f).into_owned();
    fields
        .chunks(4)
        .map(|c| {
            let line = std::str::from_utf8(c[1])
                .ok()
                .and_then(|l| l.parse().ok())
                .ok_or_else(|| GitError::parse("check-ignore", format!("line {:?}", text(c[1]))))?;
            if c[0].is_empty() || c[3].is_empty() {
                return Err(GitError::parse("check-ignore", "empty source or path"));
            }
            Ok((
                text(c[3]),
                IgnoreRule {
                    source: text(c[0]),
                    line,
                    pattern: text(c[2]),
                },
            ))
        })
        .collect()
}

/// Ignored paths as git sees them, for the file watcher: whole ignored directories (git
/// collapses a directory only when nothing inside it is tracked) and single ignored files in
/// directories that are not ignored as a whole. Paths are relative to the working tree,
/// directories without their trailing `/`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IgnoredLayout {
    pub dirs: Vec<String>,
    pub files: Vec<String>,
}

pub fn ignored_layout(repo: &Repository) -> Result<IgnoredLayout> {
    let out = repo.git([
        "ls-files",
        "-z",
        "--others",
        "--ignored",
        "--exclude-standard",
        "--directory",
        "--no-empty-directory",
    ])?;
    let mut layout = IgnoredLayout::default();
    for entry in split(&out.stdout) {
        match entry.strip_suffix('/') {
            Some(dir) => layout.dirs.push(dir.to_string()),
            None => layout.files.push(entry),
        }
    }
    Ok(layout)
}

/// The user's global excludes file (`core.excludesFile`), or git's default location
/// (`$XDG_CONFIG_HOME/git/ignore`, else `~/.config/git/ignore`) when it is not set.
pub fn excludes_file(repo: &Repository) -> Result<Option<std::path::PathBuf>> {
    let out = repo.runner().run_allowing(
        Some(repo.workdir()),
        ["config", "--path", "--get", "core.excludesFile"],
        &[0, 1],
    )?;
    let set = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !set.is_empty() {
        return Ok(Some(std::path::PathBuf::from(set)));
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")));
    Ok(base.map(|b| b.join("git/ignore")))
}

fn split(bytes: &[u8]) -> Vec<String> {
    bytes
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| String::from_utf8_lossy(p).into_owned())
        .collect()
}

/// Contents of `path` at `rev` (`HEAD`, a hash, a branch). `None` when it does not exist there.
pub fn show_at(repo: &Repository, rev: &str, path: &str) -> Result<Option<Vec<u8>>> {
    let spec = format!("{rev}:{path}");
    let out = repo
        .runner()
        .run_allowing(Some(repo.workdir()), ["show", &spec], &[0, 128])?;
    Ok((out.record.status == Some(0)).then_some(out.stdout))
}

/// Heuristic used by git itself: a NUL byte in the first 8000 bytes means binary.
/// A Git LFS pointer file: the real content lives in the LFS store, this is a stub.
pub fn is_lfs_pointer(bytes: &[u8]) -> bool {
    bytes.len() < 1024
        && bytes.starts_with(b"version https://git-lfs.github.com/spec/v1")
        && bytes.windows(11).any(|w| w == b"oid sha256:")
}

/// `(oid, size)` from a pointer file, when both lines parse.
pub fn lfs_pointer_info(text: &str) -> Option<(String, u64)> {
    let mut oid = None;
    let mut size = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("oid sha256:") {
            oid = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("size ") {
            size = rest.trim().parse().ok();
        }
    }
    Some((oid?, size?))
}

pub fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8000).any(|b| *b == 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempRepo;

    #[test]
    fn lists_tracked_untracked_and_ignored() {
        let t = TempRepo::new();
        t.commit_file("src/main.rs", "fn main() {}\n", "first");
        t.commit_file(".gitignore", "*.log\n", "ignore");
        t.write("notes.txt", "n\n");
        t.write("debug.log", "x\n");
        let repo = t.repo();
        assert_eq!(tracked(&repo).unwrap(), [".gitignore", "src/main.rs"]);
        assert_eq!(untracked(&repo).unwrap(), ["notes.txt"]);
        assert_eq!(ignored(&repo).unwrap(), ["debug.log"]);
        assert_eq!(
            show_at(&repo, "HEAD", "src/main.rs").unwrap().as_deref(),
            Some(b"fn main() {}\n".as_slice())
        );
        assert!(show_at(&repo, "HEAD", "notes.txt").unwrap().is_none());
        assert!(looks_binary(b"ab\0cd"));
        assert!(!looks_binary(b"plain text"));
    }

    #[test]
    fn ignore_rules_name_the_deciding_file_line_and_pattern() {
        let t = TempRepo::new();
        t.commit_file(".gitignore", "# build output\n/build\n*.log\n", "ignore");
        t.commit_file("site/.gitignore", "*.tmp\n", "site");
        std::fs::write(t.git_dir().join("info/exclude"), "secret.txt\n").unwrap();
        for f in [
            "a.log",
            "build/deep/x.o",
            "site/b.tmp",
            "site/c.log",
            "secret.txt",
        ] {
            t.write(f, "x\n");
        }
        t.write("kept.txt", "k\n");
        let repo = t.repo();
        let files = ignored(&repo).unwrap();
        let rules = ignore_rules(&repo, &files).unwrap();
        let rule = |source: &str, line: u32, pattern: &str| IgnoreRule {
            source: source.into(),
            line,
            pattern: pattern.into(),
        };
        assert_eq!(
            rules,
            [
                ("a.log".to_string(), rule(".gitignore", 3, "*.log")),
                ("build/deep/x.o".into(), rule(".gitignore", 2, "/build")),
                (
                    "secret.txt".into(),
                    rule(".git/info/exclude", 1, "secret.txt")
                ),
                ("site/b.tmp".into(), rule("site/.gitignore", 1, "*.tmp")),
                ("site/c.log".into(), rule(".gitignore", 3, "*.log")),
            ]
        );
        // Not ignored: no rule, and git's "nothing matched" exit is not an error.
        assert!(
            ignore_rules(&repo, &["kept.txt".into()])
                .unwrap()
                .is_empty()
        );
        assert!(ignore_rules(&repo, &[]).unwrap().is_empty());
    }

    #[test]
    fn ignored_layout_collapses_only_directories_without_tracked_files() {
        let t = TempRepo::new();
        t.commit_file(".gitignore", "target/\n*.log\nvendor/\n", "ignore");
        t.write("target/a/b/x", "x\n");
        t.write("vendor/x", "tracked\n");
        t.write("vendor/y", "y\n");
        t.write("vendor/sub/z", "z\n");
        t.write("debug.log", "l\n");
        t.write("src/m.rs", "m\n");
        t.git(["add", "-f", "vendor/x"]);
        t.git(["commit", "-q", "-m", "vendored"]);
        let layout = ignored_layout(&t.repo()).unwrap();
        assert_eq!(layout.dirs, ["target", "vendor/sub"]);
        assert_eq!(layout.files, ["debug.log", "vendor/y"]);
    }

    #[test]
    fn excludes_file_reads_config_or_falls_back() {
        let t = TempRepo::new();
        t.commit_file("a", "a\n", "first");
        let default = excludes_file(&t.repo()).unwrap();
        assert!(default.is_some_and(|p| p.ends_with("git/ignore")));
        t.git(["config", "core.excludesFile", "/tmp/osier-excludes"]);
        assert_eq!(
            excludes_file(&t.repo()).unwrap(),
            Some(std::path::PathBuf::from("/tmp/osier-excludes"))
        );
    }
}

#[cfg(test)]
mod lfs_tests {
    use super::*;

    #[test]
    fn lfs_pointer_is_detected() {
        let p = "version https://git-lfs.github.com/spec/v1\noid sha256:4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393\nsize 12345\n";
        assert!(is_lfs_pointer(p.as_bytes()));
        assert_eq!(
            lfs_pointer_info(p),
            Some((
                "4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393".into(),
                12345
            ))
        );
        assert!(!is_lfs_pointer(b"version 1\nhello\n"));
        assert!(!is_lfs_pointer(b"fn main() {}\n"));
    }
}
