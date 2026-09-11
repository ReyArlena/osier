//! Working tree file listings, gitignore-aware through git itself.

use crate::error::Result;
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
}
