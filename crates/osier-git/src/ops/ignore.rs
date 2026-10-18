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

/// Where a new pattern went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Added {
    /// False when an identical line was already there (nothing written).
    pub changed: bool,
    /// The line as written (may differ from the request to match its neighbours' style).
    pub pattern: String,
    /// 1-based line number in the file.
    pub line: usize,
    /// The related line it was placed next to, if any.
    pub near: Option<String>,
}

/// Add `pattern` (for `path`, relative to the repository root) to `file`, next to the lines it
/// belongs with (see [`place`]). If the chosen spot leaves `path` not ignored (a later `!`
/// rule re-includes it), the line moves to the end of the file, where nothing can override it.
/// Creates the file (and `.git/info`) when missing; keeps the file's line endings.
pub fn add_pattern(
    repo: &Repository,
    file: IgnoreFile,
    pattern: &str,
    path: &str,
) -> Result<Added> {
    let target = file.path(repo);
    let existing = match std::fs::read_to_string(&target) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.into()),
    };
    let crlf = existing.contains("\r\n");
    let lines: Vec<String> = existing
        .lines()
        .map(|l| l.trim_end_matches('\r').to_string())
        .collect();
    let placement = place(&lines, pattern);
    if lines
        .iter()
        .any(|l| l == pattern || l == &placement.pattern)
    {
        let line = lines
            .iter()
            .position(|l| l == pattern || l == &placement.pattern)
            .map_or(0, |i| i + 1);
        return Ok(Added {
            changed: false,
            pattern: placement.pattern,
            line,
            near: None,
        });
    }
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let write = |index: usize, text: &str| -> Result<()> {
        let mut out = lines.clone();
        out.insert(index, text.to_string());
        let sep = if crlf { "\r\n" } else { "\n" };
        let mut body = out.join(sep);
        body.push_str(sep);
        write_atomic(&target, &body)
    };
    write(placement.index, &placement.pattern)?;
    let mut added = Added {
        changed: true,
        pattern: placement.pattern.clone(),
        line: placement.index + 1,
        near: placement.near.clone(),
    };
    if placement.index < lines.len() && !is_ignored(repo, path)? {
        // Overridden by a later rule: the end of the file always wins.
        write(lines.len(), pattern)?;
        added = Added {
            changed: true,
            pattern: pattern.to_string(),
            line: lines.len() + 1,
            near: None,
        };
    }
    Ok(added)
}

/// Where a new pattern goes in an ignore file, and how it is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    /// Insert before this 0-based line (`lines.len()` appends).
    pub index: usize,
    pub pattern: String,
    pub near: Option<String>,
}

/// Choose a spot for `pattern` among `lines`.
///
/// Lines are grouped into blocks (runs of patterns between blank lines and comments). A path
/// pattern goes into the block holding the line that shares the most leading folders with it
/// (at least one); an extension pattern (`*.log`) into the block with the most `*.ext` lines.
/// Inside the block it takes its sorted position when the block is sorted, else it goes after
/// the last best match. With no related block it goes at the end. When its neighbours do not
/// start with `/` and the path has a folder in it (already anchored by git), the leading `/`
/// is dropped to match.
pub fn place(lines: &[String], pattern: &str) -> Placement {
    let end = Placement {
        index: lines.len(),
        pattern: pattern.to_string(),
        near: None,
    };
    let blocks = blocks(lines);
    let is_ext = pattern.starts_with("*.") && !pattern.contains('/');
    let score = |line: &str| -> usize {
        if is_ext {
            usize::from(line.trim().starts_with("*.") && !line.contains('/'))
        } else {
            common_prefix(&literal_parts(pattern), &literal_parts(line))
        }
    };
    // Best line overall: highest score, last occurrence on ties (so we join the group's end).
    let mut best: Option<(usize, usize)> = None; // (score, line index)
    for (start, stop) in &blocks {
        for (i, line) in lines.iter().enumerate().take(*stop).skip(*start) {
            let sc = score(line);
            if sc > 0 && best.is_none_or(|(b, _)| sc >= b) {
                best = Some((sc, i));
            }
        }
    }
    let Some((top, at)) = best else {
        return end;
    };
    let Some(&(start, stop)) = blocks.iter().find(|(s, e)| (*s..*e).contains(&at)) else {
        return end;
    };
    let neighbour = lines[at].trim().to_string();
    let mut written = pattern.to_string();
    if !is_ext
        && pattern.starts_with('/')
        && !neighbour.starts_with('/')
        && !neighbour.starts_with("!/")
        && pattern.trim_matches('/').contains('/')
    {
        written = pattern[1..].to_string();
    }
    let block = &lines[start..stop];
    let key = |l: &str| {
        l.trim()
            .trim_start_matches('!')
            .trim_start_matches('/')
            .to_string()
    };
    let sorted = block.len() > 1 && block.windows(2).all(|w| key(&w[0]) <= key(&w[1]));
    let index = if sorted {
        let k = key(&written);
        start + block.iter().take_while(|l| key(l) <= k).count()
    } else {
        // After the last line in the block that scores as well as the best.
        let last = (start..stop)
            .rev()
            .find(|&i| score(&lines[i]) == top)
            .unwrap_or(at);
        last + 1
    };
    Placement {
        index,
        pattern: written,
        near: Some(neighbour),
    }
}

/// (start, end) line ranges of pattern lines between blank lines and comments.
fn blocks(lines: &[String]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim();
        let is_pattern = !t.is_empty() && !t.starts_with('#');
        match (is_pattern, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                out.push((s, i));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push((s, lines.len()));
    }
    out
}

/// Leading path components of a pattern up to the first one with a wildcard.
fn literal_parts(pattern: &str) -> Vec<&str> {
    let p = pattern
        .trim()
        .trim_start_matches('!')
        .trim_start_matches('/');
    p.trim_end_matches('/')
        .split('/')
        .take_while(|c| !c.is_empty() && !c.contains(['*', '?', '[']))
        .collect()
}

fn common_prefix(a: &[&str], b: &[&str]) -> usize {
    a.iter().zip(b).take_while(|(x, y)| x == y).count()
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
            add_pattern(
                &repo,
                IgnoreFile::Gitignore,
                &exact_pattern(name, false),
                name,
            )
            .unwrap();
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
        t.write("debug.log", "x\n");
        assert!(
            add_pattern(&repo, IgnoreFile::Gitignore, "/debug.log", "debug.log")
                .unwrap()
                .changed
        );
        assert!(
            !add_pattern(&repo, IgnoreFile::Gitignore, "/debug.log", "debug.log")
                .unwrap()
                .changed
        );
        let text = std::fs::read_to_string(t.path().join(".gitignore")).unwrap();
        assert_eq!(text, "target/\n/debug.log\n");

        // The local exclude file is created when missing and never shows as a change.
        std::fs::remove_file(t.path().join(".git/info/exclude")).ok();
        t.write("local.env", "x\n");
        assert!(
            add_pattern(&repo, IgnoreFile::LocalExclude, "/local.env", "local.env")
                .unwrap()
                .changed
        );
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
            "config/local.php",
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

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_string).collect()
    }

    #[test]
    fn placement_joins_the_block_with_the_closest_folders() {
        let file = lines(
            "# build\n/target/\n*.log\n\n# local binaries\nlocal-bin/.local/bin/zed\n\
             local-bin/.local/bin/btop\nlocal-bin/.cache/\n\n# editors\n.idea/\n",
        );
        let p = place(&file, "/local-bin/.local/bin/new-app");
        assert_eq!(
            p.index, 7,
            "after the last of the closest matches, block unsorted"
        );
        assert_eq!(
            p.pattern, "local-bin/.local/bin/new-app",
            "neighbours have no leading /"
        );
        assert_eq!(p.near.as_deref(), Some("local-bin/.local/bin/btop"));

        // A sorted block keeps its order.
        let sorted = lines("a/x\nlocal-bin/.local/bin/btop\nlocal-bin/.local/bin/zed\nz/y\n");
        let p = place(&sorted, "/local-bin/.local/bin/new-app");
        assert_eq!(p.index, 2);

        // Nothing related: the end, unchanged.
        let p = place(&file, "/README.md");
        assert_eq!(
            (p.index, p.pattern.as_str(), p.near),
            (file.len(), "/README.md", None)
        );

        // A single name keeps its anchor even next to unanchored lines.
        let p = place(&lines("docs/a.md\ndocs/b.md\n"), "/docs/c.md");
        assert_eq!(p.pattern, "docs/c.md");
        let p = place(&lines("build/\n"), "/build.rs");
        assert_eq!(p.index, 1, "no shared folder: end");
        assert_eq!(p.pattern, "/build.rs");

        // Extension patterns join the other extension patterns.
        let p = place(&file, "*.tmp");
        assert_eq!(p.index, 3);
    }

    #[test]
    fn placement_is_undone_when_a_later_rule_re_includes_the_path() {
        let t = TempRepo::new();
        // The closest block is the first one, but `!tools/**` further down re-includes
        // everything under tools/, so a line placed there would not ignore anything.
        t.commit_file(
            ".gitignore",
            "tools/bin/a\ntools/bin/b\n\n!tools/**\n",
            "ignore",
        );
        t.write("tools/bin/new", "x\n");
        let repo = t.repo();
        let added = add_pattern(
            &repo,
            IgnoreFile::Gitignore,
            "/tools/bin/new",
            "tools/bin/new",
        )
        .unwrap();
        assert_eq!(added.line, 5, "moved to the end");
        assert_eq!(added.near, None);
        assert!(is_ignored(&repo, "tools/bin/new").unwrap());
        let text = std::fs::read_to_string(t.path().join(".gitignore")).unwrap();
        assert_eq!(
            text,
            "tools/bin/a\ntools/bin/b\n\n!tools/**\n/tools/bin/new\n"
        );
    }

    #[test]
    fn placement_writes_mid_file_and_keeps_crlf() {
        let t = TempRepo::new();
        t.commit_file(
            ".gitignore",
            "local-bin/.local/bin/zed\r\n\r\n# other\r\n*.log\r\n",
            "crlf",
        );
        t.write("local-bin/.local/bin/new-app", "x\n");
        let repo = t.repo();
        let added = add_pattern(
            &repo,
            IgnoreFile::Gitignore,
            "/local-bin/.local/bin/new-app",
            "local-bin/.local/bin/new-app",
        )
        .unwrap();
        assert_eq!(added.line, 2);
        let text = std::fs::read_to_string(t.path().join(".gitignore")).unwrap();
        assert_eq!(
            text,
            "local-bin/.local/bin/zed\r\nlocal-bin/.local/bin/new-app\r\n\r\n# other\r\n*.log\r\n"
        );
        assert!(is_ignored(&repo, "local-bin/.local/bin/new-app").unwrap());
    }

    #[test]
    fn untrack_fails_on_unknown_paths() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "a\n", "first");
        assert!(untrack(&t.repo(), "missing.txt").is_err());
    }
}
