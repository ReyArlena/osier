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

/// A branch whose upstream was deleted on the remote (usually merged on GitHub and the
/// branch removed), and what to offer instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoneUpstream {
    /// The current branch.
    pub branch: String,
    /// The remote it tracked.
    pub remote: String,
    /// The remote's default branch, as a local name (`main`), when known and not `branch`.
    pub default: Option<String>,
    /// Every commit of `branch` is already in the remote's default branch, so deleting the
    /// local branch loses nothing.
    pub merged: bool,
}

/// The local name of `remote`'s default branch: its `HEAD`, else a `main` or `master` it has.
pub fn default_branch(repo: &Repository, remote: &str) -> Option<String> {
    let head = format!("refs/remotes/{remote}/HEAD");
    if let Ok(out) = repo.git(["symbolic-ref", "--quiet", "--short", &head]) {
        let full = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if let Some(name) = full.strip_prefix(&format!("{remote}/"))
            && !name.is_empty()
        {
            return Some(name.to_string());
        }
    }
    ["main", "master"]
        .into_iter()
        .find(|name| {
            let r = format!("refs/remotes/{remote}/{name}");
            repo.git(["rev-parse", "--verify", "--quiet", &r]).is_ok()
        })
        .map(str::to_string)
}

/// Whether every commit of `branch` is in `target`.
pub fn is_merged_into(repo: &Repository, branch: &str, target: &str) -> bool {
    repo.runner()
        .run_allowing(
            Some(repo.workdir()),
            ["merge-base", "--is-ancestor", branch, target],
            &[0, 1],
        )
        .is_ok_and(|o| o.record.status == Some(0))
}

/// What to offer after a pull failed with [`crate::GitError::UpstreamGone`].
pub fn gone_upstream(repo: &Repository) -> Option<GoneUpstream> {
    let branch = String::from_utf8_lossy(
        &repo
            .git(["symbolic-ref", "--quiet", "--short", "HEAD"])
            .ok()?
            .stdout,
    )
    .trim()
    .to_string();
    let key = format!("branch.{branch}.remote");
    let remote = repo
        .git(["config", "--get", &key])
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|r| !r.is_empty() && r != ".")
        .unwrap_or_else(|| "origin".to_string());
    let default = default_branch(repo, &remote).filter(|d| *d != branch);
    let merged = default
        .as_ref()
        .is_some_and(|d| is_merged_into(repo, &branch, &format!("{remote}/{d}")));
    Some(GoneUpstream {
        branch,
        remote,
        default,
        merged,
    })
}

/// A local branch whose upstream was deleted on the remote (git's `[gone]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoneBranch {
    pub name: String,
    /// Every commit of it is on its remote's default branch: deleting it loses nothing.
    pub merged: bool,
}

/// Local branches (not the current one) whose upstream is gone, as seen since the last
/// fetch with prune.
pub fn gone_branches(repo: &Repository) -> Result<Vec<GoneBranch>> {
    let out = repo.git([
        "for-each-ref",
        "--format=%(refname:short)%00%(upstream:track)%00%(HEAD)",
        "refs/heads",
    ])?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let mut gone = Vec::new();
    for line in text.lines() {
        let mut parts = line.split('\0');
        let (Some(name), Some(track), Some(head)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        if track != "[gone]" || head == "*" {
            continue;
        }
        let key = format!("branch.{name}.remote");
        let remote = repo
            .git(["config", "--get", &key])
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|r| !r.is_empty() && r != ".")
            .unwrap_or_else(|| "origin".to_string());
        let merged = default_branch(repo, &remote)
            .filter(|d| d != name)
            .is_some_and(|d| is_merged_into(repo, name, &format!("{remote}/{d}")));
        gone.push(GoneBranch {
            name: name.to_string(),
            merged,
        });
    }
    Ok(gone)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GitError;

    #[test]
    fn pruned_branches_are_listed_with_whether_they_are_merged() {
        let t = crate::testing::TempRepo::new();
        t.commit_file("a.txt", "a\n", "first");
        let bare = t.add_bare_origin();
        t.git(["push", "-q", "-u", "origin", "main"]);
        t.git(["remote", "set-head", "origin", "main"]);
        // merged: pushed, merged into main on the remote, deleted there.
        t.checkout_new("merged");
        t.commit_file("m.txt", "m\n", "merged work");
        t.git(["push", "-q", "-u", "origin", "merged"]);
        t.git(["push", "-q", "origin", "merged:main"]);
        // unmerged: pushed, deleted on the remote without merging.
        t.checkout("main");
        t.checkout_new("unmerged");
        t.commit_file("u.txt", "u\n", "unmerged work");
        t.git(["push", "-q", "-u", "origin", "unmerged"]);
        // kept: its upstream still exists.
        t.checkout("main");
        t.checkout_new("kept");
        t.git(["push", "-q", "-u", "origin", "kept"]);
        t.checkout("main");
        for b in ["merged", "unmerged"] {
            t.git(["--git-dir", bare.to_str().unwrap(), "branch", "-D", b]);
        }
        let repo = t.repo();
        assert!(
            gone_branches(&repo).unwrap().is_empty(),
            "not gone before a prune"
        );
        t.git(["fetch", "-q", "--prune", "origin"]);
        assert_eq!(
            gone_branches(&repo).unwrap(),
            vec![
                GoneBranch {
                    name: "merged".into(),
                    merged: true
                },
                GoneBranch {
                    name: "unmerged".into(),
                    merged: false
                },
            ]
        );
        // The current branch is never offered.
        t.checkout("merged");
        assert_eq!(gone_branches(&repo).unwrap().len(), 1);
    }

    #[test]
    fn a_merged_and_deleted_upstream_is_typed_and_explained() {
        use crate::ops::remote::{PullOptions, RemoteContext, pull};
        let t = crate::testing::TempRepo::new();
        t.commit_file("a.txt", "a\n", "first");
        let bare = t.add_bare_origin();
        t.git(["fetch", "-q", "origin"]);
        t.git(["remote", "set-head", "origin", "main"]);
        t.checkout_new("feature");
        t.commit_file("b.txt", "b\n", "feature work");
        t.git(["push", "-q", "-u", "origin", "feature"]);
        // Merged on the remote (a merge commit), then the branch deleted there.
        t.git(["push", "-q", "origin", "feature:main"]);
        let bare_git = |args: &[&str]| {
            let mut all = vec!["--git-dir", bare.to_str().unwrap()];
            all.extend_from_slice(args);
            t.git(all)
        };
        bare_git(&["branch", "-D", "feature"]);
        let repo = t.repo();
        let err = pull(
            &repo,
            &PullOptions::default(),
            &RemoteContext::default(),
            |_| {},
        );
        assert!(matches!(err, Err(GitError::UpstreamGone(_))), "{err:?}");
        let gone = gone_upstream(&repo).unwrap();
        assert_eq!(
            gone,
            GoneUpstream {
                branch: "feature".into(),
                remote: "origin".into(),
                default: Some("main".into()),
                merged: true,
            }
        );
        // A commit that never reached the remote: not merged, the branch must be kept.
        t.commit_file("c.txt", "c\n", "local only");
        assert!(!gone_upstream(&repo).unwrap().merged);
    }
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
