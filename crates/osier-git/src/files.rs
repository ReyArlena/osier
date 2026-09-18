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
