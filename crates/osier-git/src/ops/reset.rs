//! Moving HEAD: reset, revert, undo.

use crate::error::{GitError, Result};
use crate::repo::Repository;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetMode {
    /// Move HEAD, keep index and working tree.
    Soft,
    /// Move HEAD and index, keep working tree.
    Mixed,
    /// Move everything. Loses uncommitted work.
    Hard,
}

pub fn reset(repo: &Repository, target: &str, mode: ResetMode) -> Result<()> {
    let flag = match mode {
        ResetMode::Soft => "--soft",
        ResetMode::Mixed => "--mixed",
        ResetMode::Hard => "--hard",
    };
    repo.git(["reset", "--quiet", flag, target, "--"])?;
    Ok(())
}

/// Create a commit that undoes `commit`. Conflicts surface as [`GitError::Conflict`].
pub fn revert(repo: &Repository, commit: &str) -> Result<String> {
    repo.git(["revert", "--no-edit", commit])?;
    repo.head_oid()?
        .ok_or_else(|| GitError::parse("revert", "no HEAD"))
}

/// Soft-reset to the parent: the last commit's changes return to the index.
pub fn undo_last_commit(repo: &Repository) -> Result<()> {
    let has_parent = repo
        .runner()
        .run_allowing(
            Some(repo.workdir()),
            ["rev-parse", "--verify", "--quiet", "HEAD~1"],
            &[0, 1],
        )?
        .record
        .status
        == Some(0);
    if !has_parent {
        return Err(GitError::Parse {
            context: "undo",
            detail: "the root commit cannot be undone this way".into(),
        });
    }
    reset(repo, "HEAD~1", ResetMode::Soft)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::{LogOptions, log};
    use crate::status::{StatusOptions, status};
    use crate::testing::TempRepo;

    #[test]
    fn reset_modes() {
        let t = TempRepo::new();
        let first = t.commit_file("a.txt", "1\n", "first");
        t.commit_file("a.txt", "2\n", "second");
        let repo = t.repo();

        reset(&repo, &first, ResetMode::Soft).unwrap();
        assert_eq!(repo.head_oid().unwrap().as_deref(), Some(first.as_str()));
        assert_eq!(
            status(&repo, StatusOptions::default())
                .unwrap()
                .staged()
                .count(),
            1
        );
        assert_eq!(t.read("a.txt"), "2\n");

        reset(&repo, &first, ResetMode::Mixed).unwrap();
        let s = status(&repo, StatusOptions::default()).unwrap();
        assert_eq!((s.staged().count(), s.unstaged().count()), (0, 1));

        reset(&repo, &first, ResetMode::Hard).unwrap();
        assert!(status(&repo, StatusOptions::default()).unwrap().is_clean());
        assert_eq!(t.read("a.txt"), "1\n");
    }

    #[test]
    fn revert_and_undo() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "1\n", "first");
        let second = t.commit_file("a.txt", "2\n", "second");
        let repo = t.repo();
        let reverted = revert(&repo, &second).unwrap();
        assert_eq!(t.read("a.txt"), "1\n");
        assert_eq!(
            log(&repo, &LogOptions::default()).unwrap()[0].hash,
            reverted
        );

        undo_last_commit(&repo).unwrap();
        assert_eq!(repo.head_oid().unwrap().as_deref(), Some(second.as_str()));
        assert_eq!(
            status(&repo, StatusOptions::default())
                .unwrap()
                .staged()
                .count(),
            1
        );

        reset(&repo, "HEAD", ResetMode::Hard).unwrap();
        undo_last_commit(&repo).unwrap();
        assert!(undo_last_commit(&repo).is_err(), "root commit");
    }

    #[test]
    fn revert_conflict_is_typed() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "1\n", "first");
        let second = t.commit_file("a.txt", "2\n", "second");
        t.commit_file("a.txt", "3\n", "third");
        assert!(matches!(
            revert(&t.repo(), &second),
            Err(GitError::Conflict(_))
        ));
        assert_eq!(
            t.repo().in_progress(),
            Some(crate::repo::InProgress::Revert)
        );
    }
}
