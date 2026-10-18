//! Ignoring and untracking paths: `.gitignore` / `.git/info/exclude` edits and
//! `git rm --cached`.
//!
//! Ignore rules never apply to files git already tracks. Ignoring a tracked file therefore
//! takes two steps: add the pattern, then stop tracking it (`untrack`). The removal is staged
//! and, once committed, deletes the file from every other clone that pulls it; the UI warns
//! about that before calling `untrack`.

use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::repo::Repository;

/// Where a pattern goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IgnoreFile {
    /// The repository's root `.gitignore` (committed, shared with everyone).
    Gitignore,
    /// `.git/info/exclude` (this clone only, never committed).
    LocalExclude,
}

impl IgnoreFile {
    pub fn path(self, repo: &Repository) -> PathBuf {
        match self {
            IgnoreFile::Gitignore => repo.workdir().join(".gitignore"),
            IgnoreFile::LocalExclude => repo.common_dir().join("info").join("exclude"),
        }
    }

    /// Short name for messages.
    pub fn label(self) -> &'static str {
        match self {
            IgnoreFile::Gitignore => ".gitignore",
            IgnoreFile::LocalExclude => ".git/info/exclude",
        }
    }
}

/// Pattern that matches exactly `path` (relative to the repository root, `/` separated):
/// anchored with a leading `/`, special characters escaped, folders with a trailing `/`.
pub fn exact_pattern(path: &str, is_dir: bool) -> String {
    let path = path.trim_matches('/');
    let mut out = String::with_capacity(path.len() + 2);
    out.push('/');
    for c in path.chars() {
        if matches!(c, '*' | '?' | '[' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    // Trailing spaces are dropped by git unless escaped.
    let trailing = out.len() - out.trim_end_matches(' ').len();
    if trailing > 0 {
        out.truncate(out.len() - trailing);
        out.push_str(&"\\ ".repeat(trailing));
    }
    if is_dir {
        out.push('/');
    }
    out
}

/// `*.ext` for a file name with an extension, else `None`. Dotfiles (`.env`) have none.
pub fn extension_pattern(path: &str) -> Option<String> {
    let name = path.rsplit('/').next()?;
    let (stem, ext) = name.rsplit_once('.')?;
    if stem.is_empty() || ext.is_empty() || ext.contains(['*', '?', '[', '\\', ' ']) {
        return None;
    }
    Some(format!("*.{ext}"))
}

/// Whether any tracked file is at or under `path`.
pub fn is_tracked(repo: &Repository, path: &str) -> Result<bool> {
    let out = repo.git(["ls-files", "-z", "--cached", "--", path])?;
    Ok(!out.stdout.is_empty())
}

/// Whether an ignore rule matches `path`, tracked or not (`check-ignore --no-index`).
pub fn is_ignored(repo: &Repository, path: &str) -> Result<bool> {
    let out = repo.runner().run_allowing(
        Some(repo.workdir()),
        ["check-ignore", "-q", "--no-index", "--", path],
        &[0, 1],
    )?;
    Ok(out.record.status == Some(0))
}

/// Append `pattern` to `file` unless an identical line is already there. Returns whether the
/// file changed. Creates the file (and `.git/info`) when missing; keeps a final newline.
pub fn add_pattern(repo: &Repository, file: IgnoreFile, pattern: &str) -> Result<bool> {
    let path = file.path(repo);
    let existing = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.into()),
    };
    if existing
        .lines()
        .any(|l| l.trim_end_matches('\r') == pattern)
    {
        return Ok(false);
    }
    let mut text = existing;
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(pattern);
    text.push('\n');
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    write_atomic(&path, &text)?;
    Ok(true)
}

/// Stop tracking `path` (a file, or every tracked file under a folder) without touching the
/// working tree: `git rm --cached -r`. The removal is staged.
pub fn untrack(repo: &Repository, path: &str) -> Result<()> {
    repo.git(["rm", "--cached", "-r", "--quiet", "--", path])?;
    Ok(())
}

fn write_atomic(path: &Path, text: &str) -> Result<()> {
    let tmp = path.with_extension("osier-tmp");
    std::fs::write(&tmp, text)?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempRepo;

    #[test]
    fn exact_patterns_are_anchored_and_escaped() {
        assert_eq!(exact_pattern("build/out.log", false), "/build/out.log");
        assert_eq!(exact_pattern("target", true), "/target/");
        assert_eq!(exact_pattern("a*b?[c].txt", false), "/a\\*b\\?\\[c].txt");
        assert_eq!(exact_pattern("#notes", false), "/#notes");
        assert_eq!(exact_pattern("!bang", false), "/!bang");
        assert_eq!(exact_pattern("odd ", false), "/odd\\ ");
        assert_eq!(extension_pattern("logs/app.log").as_deref(), Some("*.log"));
        assert_eq!(extension_pattern(".env"), None);
        assert_eq!(extension_pattern("Makefile"), None);
    }

    #[test]
    fn escaped_patterns_match_only_their_path() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "a\n", "first");
        for name in ["a*b?[c].txt", "#notes", "!bang", "odd "] {
            t.write(name, "x\n");
        }
        t.write("axbyc.txt", "x\n");
        let repo = t.repo();
        for name in ["a*b?[c].txt", "#notes", "!bang", "odd "] {
            add_pattern(&repo, IgnoreFile::Gitignore, &exact_pattern(name, false)).unwrap();
            assert!(is_ignored(&repo, name).unwrap(), "{name:?}");
        }
        assert!(
            !is_ignored(&repo, "axbyc.txt").unwrap(),
            "wildcards were escaped"
        );
    }

    #[test]
    fn add_pattern_appends_once_and_keeps_newlines() {
        let t = TempRepo::new();
        t.commit_file(".gitignore", "target/", "no final newline");
        let repo = t.repo();
        assert!(add_pattern(&repo, IgnoreFile::Gitignore, "/debug.log").unwrap());
        assert!(!add_pattern(&repo, IgnoreFile::Gitignore, "/debug.log").unwrap());
        let text = std::fs::read_to_string(t.path().join(".gitignore")).unwrap();
        assert_eq!(text, "target/\n/debug.log\n");

        // The local exclude file is created when missing and never shows as a change.
        std::fs::remove_file(t.path().join(".git/info/exclude")).ok();
        t.write("local.env", "x\n");
        assert!(add_pattern(&repo, IgnoreFile::LocalExclude, "/local.env").unwrap());
        assert!(is_ignored(&repo, "local.env").unwrap());
        let status = t.git(["status", "--porcelain"]);
        assert!(!status.contains("local.env"), "{status}");
    }

    #[test]
    fn tracked_files_need_untracking_to_be_ignored() {
        let t = TempRepo::new();
        t.commit_file("config/local.php", "secret\n", "tracked");
        let repo = t.repo();
        assert!(is_tracked(&repo, "config/local.php").unwrap());
        assert!(
            is_tracked(&repo, "config").unwrap(),
            "folders count their files"
        );
        add_pattern(
            &repo,
            IgnoreFile::Gitignore,
            &exact_pattern("config/local.php", false),
        )
        .unwrap();
        assert!(is_ignored(&repo, "config/local.php").unwrap());
        assert!(
            is_tracked(&repo, "config/local.php").unwrap(),
            "still tracked"
        );

        untrack(&repo, "config/local.php").unwrap();
        assert!(!is_tracked(&repo, "config/local.php").unwrap());
        assert!(
            t.path().join("config/local.php").exists(),
            "the file stays on disk"
        );
        let status = t.git(["status", "--porcelain"]);
        assert!(
            status.contains("D  config/local.php"),
            "removal is staged: {status}"
        );
        assert!(
            !status.contains("?? config/local.php"),
            "and now ignored: {status}"
        );
    }

    #[test]
    fn untrack_fails_on_unknown_paths() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "a\n", "first");
        assert!(untrack(&t.repo(), "missing.txt").is_err());
    }
}
