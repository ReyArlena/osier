//! Throwing changes away. Every function here loses data; the UI confirms first.

use std::path::PathBuf;

use crate::error::{GitError, Result};
use crate::repo::Repository;

use super::with_paths;

/// Drop unstaged changes: restore the working tree copy from the index.
pub fn discard_worktree(repo: &Repository, paths: &[String]) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    repo.git(with_paths(vec!["restore", "--worktree"], paths))?;
    Ok(())
}

/// Drop staged and unstaged changes: restore index and working tree from HEAD.
pub fn discard_all(repo: &Repository, paths: &[String]) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    repo.git(with_paths(
        vec!["restore", "--source=HEAD", "--staged", "--worktree"],
        paths,
    ))?;
    Ok(())
}

/// Move untracked files to the system trash instead of deleting them.
pub fn trash_untracked(repo: &Repository, paths: &[String]) -> Result<Vec<PathBuf>> {
    let mut moved = Vec::new();
    for p in paths {
        let rel = std::path::Path::new(p);
        let escapes = rel.is_absolute()
            || rel
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir));
        let full = repo.workdir().join(rel);
        if escapes || !full.starts_with(repo.workdir()) {
            return Err(GitError::Parse {
                context: "trash",
                detail: format!("{p} escapes the repository"),
            });
        }
        if !full.exists() {
            continue;
        }
        trash::delete(&full).map_err(|e| GitError::Io(std::io::Error::other(e.to_string())))?;
        moved.push(full);
    }
    Ok(moved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::{StatusOptions, status};
    use crate::testing::TempRepo;

    #[test]
    fn discard_worktree_keeps_staged_part() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "base\n", "first");
        t.write("a.txt", "staged\n");
        t.git(["add", "a.txt"]);
        t.write("a.txt", "staged\nmore\n");
        discard_worktree(&t.repo(), &["a.txt".into()]).unwrap();
        assert_eq!(t.read("a.txt"), "staged\n");
        let s = status(&t.repo(), StatusOptions::default()).unwrap();
        assert_eq!(s.staged().count(), 1);
        assert_eq!(s.unstaged().count(), 0);
    }

    #[test]
    fn discard_all_restores_head() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "base\n", "first");
        t.write("a.txt", "staged\n");
        t.git(["add", "a.txt"]);
        t.write("a.txt", "and more\n");
        discard_all(&t.repo(), &["a.txt".into()]).unwrap();
        assert_eq!(t.read("a.txt"), "base\n");
        assert!(
            status(&t.repo(), StatusOptions::default())
                .unwrap()
                .is_clean()
        );
    }

    #[test]
    fn trash_refuses_paths_outside_the_repo() {
        let t = TempRepo::new();
        assert!(trash_untracked(&t.repo(), &["../escape.txt".into()]).is_err());
    }
}
