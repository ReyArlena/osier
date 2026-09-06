//! Branch creation, switching, renaming, deletion.

use crate::error::Result;
use crate::repo::Repository;

use super::validate_ref_name;

/// Create `name` at `start_point` (default HEAD), optionally switching to it.
pub fn create(
    repo: &Repository,
    name: &str,
    start_point: Option<&str>,
    checkout: bool,
) -> Result<()> {
    validate_ref_name(repo, name)?;
    let start = start_point.unwrap_or("HEAD");
    if checkout {
        repo.git(["switch", "--quiet", "-c", name, start])?;
    } else {
        repo.git(["branch", name, start])?;
    }
    Ok(())
}

/// Switch to a local branch. Fails with [`crate::GitError::Dirty`] when local changes would
/// be overwritten.
pub fn checkout(repo: &Repository, name: &str) -> Result<()> {
    repo.git(["switch", "--quiet", name])?;
    Ok(())
}

/// Create a local branch tracking `remote_branch` (e.g. `origin/feature`) and switch to it.
pub fn checkout_remote(repo: &Repository, remote_branch: &str, local_name: &str) -> Result<()> {
    validate_ref_name(repo, local_name)?;
    repo.git([
        "switch",
        "--quiet",
        "-c",
        local_name,
        "--track",
        remote_branch,
    ])?;
    Ok(())
}

/// Detached checkout of any revision.
pub fn checkout_detached(repo: &Repository, rev: &str) -> Result<()> {
    repo.git(["switch", "--quiet", "--detach", rev])?;
    Ok(())
}

/// Point `branch` at `upstream` (e.g. `origin/main`) without pushing.
pub fn set_upstream(repo: &Repository, branch: &str, upstream: &str) -> Result<()> {
    repo.git(["branch", "--set-upstream-to", upstream, branch])?;
    Ok(())
}

pub fn rename(repo: &Repository, old: &str, new: &str) -> Result<()> {
    validate_ref_name(repo, new)?;
    repo.git(["branch", "-m", old, new])?;
    Ok(())
}

/// Delete a local branch. Without `force` an unmerged branch fails with
/// [`crate::GitError::BranchNotMerged`].
pub fn delete(repo: &Repository, name: &str, force: bool) -> Result<()> {
    repo.git(["branch", if force { "-D" } else { "-d" }, name])?;
    Ok(())
}

/// Delete a branch on a remote (`git push origin --delete name`). Network operation.
pub fn delete_remote(repo: &Repository, remote: &str, name: &str) -> Result<()> {
    repo.git(["push", "--quiet", remote, "--delete", name])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GitError;
    use crate::refs::branches;
    use crate::testing::TempRepo;

    fn current(t: &TempRepo) -> String {
        t.git(["branch", "--show-current"]).trim().to_string()
    }

    #[test]
    fn create_checkout_rename_delete() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "a\n", "first");
        let repo = t.repo();
        create(&repo, "feature", None, true).unwrap();
        assert_eq!(current(&t), "feature");
        t.commit_file("b.txt", "b\n", "on feature");
        checkout(&repo, "main").unwrap();
        assert_eq!(current(&t), "main");
        create(&repo, "other", Some("feature"), false).unwrap();
        rename(&repo, "other", "renamed").unwrap();
        let names: Vec<String> = branches(&repo)
            .unwrap()
            .into_iter()
            .map(|b| b.name)
            .collect();
        assert_eq!(names, ["feature", "main", "renamed"]);

        assert!(matches!(
            delete(&repo, "feature", false),
            Err(GitError::BranchNotMerged(_))
        ));
        delete(&repo, "feature", true).unwrap();
        delete(&repo, "renamed", true).unwrap();
        assert_eq!(branches(&repo).unwrap().len(), 1);
    }

    #[test]
    fn set_upstream_links_a_branch() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "a\n", "first");
        t.add_bare_origin();
        t.git(["push", "-q", "origin", "main"]);
        let repo = t.repo();
        assert!(
            branches(&repo)
                .unwrap()
                .iter()
                .any(|b| b.name == "main" && b.upstream.is_none())
        );
        set_upstream(&repo, "main", "origin/main").unwrap();
        assert!(
            branches(&repo)
                .unwrap()
                .iter()
                .any(|b| b.name == "main" && b.upstream.as_deref() == Some("origin/main"))
        );
    }

    #[test]
    fn invalid_names_are_rejected_before_running_git() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "a\n", "first");
        let repo = t.repo();
        assert!(matches!(
            create(&repo, "bad name", None, false),
            Err(GitError::InvalidRefName(_))
        ));
        assert!(matches!(
            create(&repo, "", None, false),
            Err(GitError::InvalidRefName(_))
        ));
        assert!(matches!(
            create(&repo, "a..b", None, false),
            Err(GitError::InvalidRefName(_))
        ));
    }

    #[test]
    fn dirty_switch_is_typed_and_detached_checkout_works() {
        let t = TempRepo::new();
        let first = t.commit_file("a.txt", "a\n", "first");
        t.checkout_new("feature");
        t.commit_file("a.txt", "feature\n", "second");
        t.write("a.txt", "dirty\n");
        let repo = t.repo();
        assert!(matches!(checkout(&repo, "main"), Err(GitError::Dirty(_))));
        t.git(["checkout", "--", "a.txt"]);
        checkout_detached(&repo, &first).unwrap();
        assert_eq!(current(&t), "");
        assert_eq!(repo.head_oid().unwrap().as_deref(), Some(first.as_str()));
    }

    #[test]
    fn checkout_remote_creates_tracking_branch() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "a\n", "first");
        t.branch("remote-only");
        t.add_bare_origin();
        t.git(["push", "-q", "origin", "remote-only"]);
        t.git(["branch", "-D", "remote-only"]);
        t.git(["fetch", "-q", "origin"]);
        let repo = t.repo();
        checkout_remote(&repo, "origin/remote-only", "remote-only").unwrap();
        let b = branches(&repo).unwrap();
        let local = b
            .iter()
            .find(|b| b.name == "remote-only" && !b.is_remote)
            .unwrap();
        assert_eq!(local.upstream.as_deref(), Some("origin/remote-only"));
        assert!(local.is_head);
    }
}
