//! Index updates: stage, unstage, apply patches.

use crate::error::{GitError, Result};
use crate::repo::Repository;

use super::with_paths;

/// Stage files (adds, modifications and deletions alike).
pub fn stage(repo: &Repository, paths: &[String]) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    repo.git(with_paths(vec!["add", "-A"], paths))?;
    Ok(())
}

/// Stage everything in the working tree.
pub fn stage_all(repo: &Repository) -> Result<()> {
    repo.git(["add", "-A"])?;
    Ok(())
}

/// Move files back out of the index. Works on an unborn branch too.
pub fn unstage(repo: &Repository, paths: &[String]) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    if repo.head_oid()?.is_some() {
        repo.git(with_paths(vec!["restore", "--staged"], paths))?;
    } else {
        repo.git(with_paths(vec!["rm", "-r", "--cached", "--quiet"], paths))?;
    }
    Ok(())
}

pub fn unstage_all(repo: &Repository) -> Result<()> {
    if repo.head_oid()?.is_some() {
        repo.git(["reset", "-q"])?;
    } else {
        repo.git(["rm", "-r", "--cached", "--quiet", "--ignore-unmatch", "."])?;
    }
    Ok(())
}

/// Where a patch is applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyTarget {
    /// Change the index only (`--cached`).
    Index,
    /// Change the working tree only.
    Worktree,
}

/// Apply a unified diff produced by [`super::patch`]. `reverse` undoes it.
pub fn apply_patch(
    repo: &Repository,
    patch: &str,
    target: ApplyTarget,
    reverse: bool,
) -> Result<()> {
    let mut args = vec!["apply", "--whitespace=nowarn"];
    if target == ApplyTarget::Index {
        args.push("--cached");
    }
    if reverse {
        args.push("-R");
    }
    let out = repo
        .runner()
        .run_unchecked(Some(repo.workdir()), args, Some(patch.as_bytes()))?;
    match out.record.status {
        Some(0) => Ok(()),
        Some(s) => Err(GitError::failed(s, out.record)),
        None => Err(GitError::timeout(0, out.record)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::{StatusOptions, status};
    use crate::testing::TempRepo;

    #[test]
    fn stage_and_unstage_round_trip() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "a\n", "first");
        t.write("a.txt", "changed\n");
        t.write("new.txt", "new\n");
        t.remove("a.txt");
        t.write("a.txt", "changed\n");
        let repo = t.repo();

        stage(&repo, &["a.txt".into(), "new.txt".into()]).unwrap();
        let s = status(&repo, StatusOptions::default()).unwrap();
        assert_eq!(s.staged().count(), 2);
        assert_eq!(s.untracked().count(), 0);

        unstage(&repo, &["new.txt".into()]).unwrap();
        let s = status(&repo, StatusOptions::default()).unwrap();
        assert_eq!(s.staged().count(), 1);
        assert_eq!(s.untracked().count(), 1);

        unstage_all(&repo).unwrap();
        let s = status(&repo, StatusOptions::default()).unwrap();
        assert_eq!(s.staged().count(), 0);

        stage_all(&repo).unwrap();
        assert_eq!(
            status(&repo, StatusOptions::default())
                .unwrap()
                .staged()
                .count(),
            2
        );
    }

    #[test]
    fn stage_records_a_deletion() {
        let t = TempRepo::new();
        t.commit_file("gone.txt", "x\n", "first");
        t.remove("gone.txt");
        stage(&t.repo(), &["gone.txt".into()]).unwrap();
        let s = status(&t.repo(), StatusOptions::default()).unwrap();
        let e = s.entries.iter().find(|e| e.path == "gone.txt").unwrap();
        assert_eq!(e.index, 'D');
    }

    #[test]
    fn unstage_on_unborn_branch() {
        let t = TempRepo::new();
        t.write("a.txt", "a\n");
        stage(&t.repo(), &["a.txt".into()]).unwrap();
        assert_eq!(
            status(&t.repo(), StatusOptions::default())
                .unwrap()
                .staged()
                .count(),
            1
        );
        unstage(&t.repo(), &["a.txt".into()]).unwrap();
        let s = status(&t.repo(), StatusOptions::default()).unwrap();
        assert_eq!(s.staged().count(), 0);
        assert_eq!(s.untracked().count(), 1);
        unstage_all(&t.repo()).unwrap();
    }

    #[test]
    fn empty_path_list_is_a_no_op() {
        let t = TempRepo::new();
        let repo = t.repo();
        let before = t.sink().records().len();
        stage(&repo, &[]).unwrap();
        unstage(&repo, &[]).unwrap();
        assert_eq!(t.sink().records().len(), before, "no git command ran");
    }
}
