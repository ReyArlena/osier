//! Squash a run of consecutive commits on the current branch into one.
//!
//! Safety, in order:
//! - [`plan`] refuses anything that is not a plain line of history on the checked-out branch:
//!   detached HEAD, an operation in progress, commits off the branch, merge commits in the
//!   range or after it, or (when later commits must be replayed) uncommitted changes to
//!   tracked files.
//! - The squashed commit is built with `commit-tree` from the newest commit's tree, so its
//!   content is exactly what the branch had. The oldest commit's author is kept and
//!   `commit.gpgsign` is honoured.
//! - When the newest selected commit is HEAD, the branch moves with a compare-and-swap
//!   `update-ref`; the working tree and the index are not touched, so staged changes stay
//!   staged and out of the squash. Otherwise the later commits are replayed with
//!   `rebase --onto`, which cannot conflict (the new base has the same tree); if it fails
//!   anyway it is aborted and the branch is left as it was.
//! - [`squash`] re-checks that HEAD has not moved since the plan was made.
//! - The old commits stay in the reflog.

use std::collections::HashSet;

use crate::error::{GitError, Result};
use crate::repo::Repository;

/// What a squash would do, checked and ready to confirm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SquashPlan {
    /// The commits to squash, newest first.
    pub commits: Vec<String>,
    /// Parent of the oldest commit; `None` when the oldest is the root commit.
    pub base: Option<String>,
    /// HEAD when the plan was made.
    pub head: String,
    /// Branch being rewritten, without `refs/heads/`.
    pub branch: String,
    /// Commits after the newest selected one that will be replayed on top.
    pub replay: usize,
    /// Remote branches (e.g. `origin/main`) that already contain some of these commits:
    /// squashing rewrites published history and needs a force push.
    pub published: Vec<String>,
    /// Other local branches and tags that contain these commits. They keep the old commits.
    pub other_refs: Vec<String>,
    /// The commits' messages, oldest first, joined by blank lines: the default message.
    pub message: String,
}

fn refused(reason: impl Into<String>) -> GitError {
    GitError::Refused(reason.into())
}

/// Exit status of a git command that answers yes (0) or no (1).
fn yes(repo: &Repository, args: &[&str]) -> Result<bool> {
    let out = repo
        .runner()
        .run_allowing(Some(repo.workdir()), args, &[0, 1])?;
    Ok(out.record.status == Some(0))
}

fn rev_parse(repo: &Repository, rev: &str) -> Result<String> {
    Ok(repo
        .git(["rev-parse", "--verify", "--quiet", rev])?
        .stdout_str()?
        .trim()
        .to_string())
}

/// `hash parent...` lines of `git rev-list --first-parent --parents <range>`, newest first.
fn chain(repo: &Repository, range: &str) -> Result<Vec<(String, Vec<String>)>> {
    let out = repo.git(["rev-list", "--first-parent", "--parents", range])?;
    Ok(out
        .stdout_str()?
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| {
            let mut parts = l.split_whitespace().map(str::to_string);
            let hash = parts.next().unwrap_or_default();
            (hash, parts.collect())
        })
        .collect())
}

/// Check that the commits between `a` and `b` (either order, both included) can be squashed.
pub fn plan(repo: &Repository, a: &str, b: &str) -> Result<SquashPlan> {
    if let Some(op) = repo.in_progress() {
        return Err(refused(format!(
            "Finish or abort the {} in progress first.",
            op.label()
        )));
    }
    let branch_ref = repo.runner().run_allowing(
        Some(repo.workdir()),
        ["symbolic-ref", "--quiet", "HEAD"],
        &[0, 1],
    )?;
    if branch_ref.record.status != Some(0) {
        return Err(refused("Check out a branch first: HEAD is detached."));
    }
    let branch = branch_ref
        .stdout_str()?
        .trim()
        .trim_start_matches("refs/heads/")
        .to_string();
    let head = rev_parse(repo, "HEAD")?;
    let (a, b) = (rev_parse(repo, a)?, rev_parse(repo, b)?);
    if a == b {
        return Err(refused("Select at least two commits to squash."));
    }
    let (older, newer) = if yes(repo, &["merge-base", "--is-ancestor", &a, &b])? {
        (a, b)
    } else if yes(repo, &["merge-base", "--is-ancestor", &b, &a])? {
        (b, a)
    } else {
        return Err(refused(
            "These commits are on different lines of history and cannot be squashed together.",
        ));
    };
    if !yes(repo, &["merge-base", "--is-ancestor", &newer, "HEAD"])? {
        return Err(refused(format!(
            "Only commits on the current branch ({branch}) can be squashed."
        )));
    }

    // The selection: newer back to older along first parents, no merges.
    let mut commits = Vec::new();
    let mut expect = newer.clone();
    for (hash, parents) in chain(repo, &format!("{older}..{newer}"))? {
        if hash != expect || parents.len() != 1 {
            return Err(refused(
                "The selection includes a merge, or is not one straight line of commits.",
            ));
        }
        commits.push(hash);
        expect = parents[0].clone();
    }
    if expect != older {
        return Err(refused(
            "The selection is not one straight line of commits.",
        ));
    }
    let older_parents: Vec<String> = repo
        .git(["rev-list", "--parents", "-n", "1", &older])?
        .stdout_str()?
        .split_whitespace()
        .skip(1)
        .map(str::to_string)
        .collect();
    if older_parents.len() > 1 {
        return Err(refused(
            "The oldest selected commit is a merge; merges cannot be squashed.",
        ));
    }
    commits.push(older.clone());
    let base = older_parents.into_iter().next();

    // Later commits on the branch, replayed on top: a straight line, no merges.
    let mut replay = 0;
    let mut expect_parent = newer.clone();
    let later = chain(repo, &format!("{newer}..HEAD"))?;
    for (_, parents) in later.iter().rev() {
        if parents.len() != 1 || parents[0] != expect_parent {
            return Err(refused(
                "There are merge commits after the selection; they cannot be replayed safely.",
            ));
        }
        replay += 1;
        expect_parent = later[later.len() - replay].0.clone();
    }
    if replay > 0 {
        let dirty = repo.git(["status", "--porcelain", "--untracked-files=no"])?;
        if !dirty.stdout_str()?.trim().is_empty() {
            return Err(refused(format!(
                "Commit or stash your changes first: the {} after the selection have to be replayed.",
                if replay == 1 {
                    "commit".to_string()
                } else {
                    format!("{replay} commits")
                }
            )));
        }
    }

    // Refs that already contain the oldest selected commit keep the old history after the
    // squash: remote branches (published, needs a force push) and other local branches and
    // tags (they keep pointing at the old commits).
    let containing = |prefix: &str| -> Result<Vec<String>> {
        let out = repo.git([
            "for-each-ref",
            "--format=%(refname:short)",
            "--contains",
            older.as_str(),
            prefix,
        ])?;
        Ok(out
            .stdout_str()?
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.ends_with("/HEAD"))
            .map(str::to_string)
            .collect())
    };
    let published = containing("refs/remotes")?;
    let mut other_refs = containing("refs/heads")?;
    other_refs.retain(|b| *b != branch);
    other_refs.extend(containing("refs/tags")?);

    let message = messages(repo, &commits)?;
    Ok(SquashPlan {
        commits,
        base,
        head,
        branch,
        replay,
        published,
        other_refs,
        message,
    })
}

/// Messages of `commits` (newest first), oldest first, joined by blank lines.
fn messages(repo: &Repository, commits: &[String]) -> Result<String> {
    let mut args = vec!["show".to_string(), "-s".into(), "--format=%B%x00".into()];
    args.extend(commits.iter().cloned());
    let out = repo.git(args)?;
    let text = out.stdout_str()?;
    let mut parts: Vec<&str> = text
        .split('\0')
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .collect();
    parts.reverse();
    Ok(parts.join("\n\n"))
}

/// Squash as planned, with `message`. Returns the new commit's hash.
pub fn squash(repo: &Repository, plan: &SquashPlan, message: &str) -> Result<String> {
    if message.trim().is_empty() {
        return Err(refused("The commit message is empty."));
    }
    if repo.head_oid()?.as_deref() != Some(plan.head.as_str()) {
        return Err(refused(
            "The branch moved since the squash was planned. Try again.",
        ));
    }
    if repo.in_progress().is_some() {
        return Err(refused(
            "Another operation started; finish or abort it first.",
        ));
    }
    let (Some(newest), Some(oldest)) = (plan.commits.first(), plan.commits.last()) else {
        return Err(refused("Nothing to squash."));
    };
    let tree = rev_parse(repo, &format!("{newest}^{{tree}}"))?;
    let author = repo.git(["show", "-s", "--format=%an%x00%ae%x00%aI", oldest])?;
    let author = author.stdout_str()?.trim_end().to_string();
    let mut fields = author.split('\0');
    let (name, email, date) = (
        fields.next().unwrap_or_default().to_string(),
        fields.next().unwrap_or_default().to_string(),
        fields.next().unwrap_or_default().to_string(),
    );
    let sign = repo
        .runner()
        .run_allowing(
            Some(repo.workdir()),
            ["config", "--bool", "commit.gpgsign"],
            &[0, 1],
        )?
        .stdout_str()?
        .trim()
        == "true";

    let dir = tempfile::Builder::new().prefix("osier-squash-").tempdir()?;
    let message_file = dir.path().join("message");
    std::fs::write(&message_file, format!("{}\n", message.trim_end()))?;
    let message_path = message_file.to_string_lossy().into_owned();
    let mut args: Vec<String> = vec!["commit-tree".into(), tree];
    if let Some(base) = &plan.base {
        args.extend(["-p".into(), base.clone()]);
    }
    if sign {
        args.push("-S".into());
    }
    args.extend(["-F".into(), message_path]);
    let runner = repo
        .runner()
        .clone()
        .with_env("GIT_AUTHOR_NAME", name)
        .with_env("GIT_AUTHOR_EMAIL", email)
        .with_env("GIT_AUTHOR_DATE", date);
    let new = runner
        .run(Some(repo.workdir()), args)?
        .stdout_str()?
        .trim()
        .to_string();

    let reason = format!("osier: squash {} commits", plan.commits.len());
    if plan.replay == 0 {
        // Compare-and-swap: fails if HEAD moved in the meantime.
        repo.git(["update-ref", "-m", &reason, "HEAD", &new, &plan.head])?;
    } else {
        let replayed = repo.git(["rebase", "--onto", &new, newest]);
        if let Err(error) = replayed {
            let _ = repo.git(["rebase", "--abort"]);
            return Err(error);
        }
    }
    Ok(new)
}

/// Hashes of the first-parent line from `newer` back to `older` within `commits` (a log
/// page: each entry is `(hash, parents)`), both included; `None` when `older` is not on
/// that line or a merge is in the way. The UI uses it to highlight what a squash would take.
pub fn first_parent_run<'a>(
    commits: impl IntoIterator<Item = (&'a str, &'a [String])>,
    newer: &str,
    older: &str,
) -> Option<Vec<String>> {
    let parents: std::collections::HashMap<&str, &[String]> = commits.into_iter().collect();
    let mut run = vec![newer.to_string()];
    let mut seen = HashSet::new();
    let mut at = newer;
    while at != older {
        let p = parents.get(at)?;
        if p.len() != 1 || !seen.insert(at) {
            return None;
        }
        at = p[0].as_str();
        run.push(at.to_string());
    }
    if parents.get(older).is_some_and(|p| p.len() > 1) {
        return None;
    }
    Some(run)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::{LogOptions, log};
    use crate::testing::TempRepo;

    fn subjects(t: &TempRepo) -> Vec<String> {
        log(&t.repo(), &LogOptions::default())
            .unwrap()
            .into_iter()
            .map(|c| c.subject)
            .collect()
    }

    fn head(t: &TempRepo) -> String {
        t.repo().head_oid().unwrap().unwrap()
    }

    fn tree(t: &TempRepo, rev: &str) -> String {
        rev_parse(&t.repo(), &format!("{rev}^{{tree}}")).unwrap()
    }

    fn rev(t: &TempRepo, r: &str) -> String {
        rev_parse(&t.repo(), r).unwrap()
    }

    /// a, b, c, d, e on main.
    fn five() -> TempRepo {
        let t = TempRepo::new();
        for (n, s) in ["a", "b", "c", "d", "e"].iter().enumerate() {
            t.commit_file(&format!("{s}.txt"), &format!("{n}\n"), s);
        }
        t
    }

    #[test]
    fn squashes_the_top_commits_without_touching_staged_work() {
        let t = five();
        let tree_before = tree(&t, "HEAD");
        t.write("a.txt", "staged change\n");
        t.git(["add", "a.txt"]);
        let p = plan(&t.repo(), &rev(&t, "HEAD"), &rev(&t, "HEAD~2")).unwrap();
        assert_eq!(p.commits.len(), 3);
        assert_eq!(p.replay, 0);
        assert_eq!(p.message, "c\n\nd\n\ne");
        assert!(p.published.is_empty() && p.other_refs.is_empty());
        let new = squash(&t.repo(), &p, "c, d and e").unwrap();
        assert_eq!(head(&t), new);
        assert_eq!(subjects(&t), ["c, d and e", "b", "a"]);
        assert_eq!(tree(&t, "HEAD"), tree_before, "same content");
        let staged = t.git(["diff", "--cached", "--name-only"]);
        assert_eq!(staged.trim(), "a.txt", "staged work stays staged");
        assert_eq!(rev(&t, "HEAD~1"), p.base.clone().unwrap());
    }

    #[test]
    fn squashes_in_the_middle_and_replays_the_rest() {
        let t = five();
        let tree_before = tree(&t, "HEAD");
        let p = plan(&t.repo(), &rev(&t, "HEAD~3"), &rev(&t, "HEAD~2")).unwrap();
        assert_eq!(p.replay, 2);
        squash(&t.repo(), &p, "b and c").unwrap();
        assert_eq!(subjects(&t), ["e", "d", "b and c", "a"]);
        assert_eq!(tree(&t, "HEAD"), tree_before);
        assert!(t.repo().in_progress().is_none());
    }

    #[test]
    fn squashes_down_to_the_root_and_keeps_the_oldest_author() {
        let t = TempRepo::new();
        t.write("a.txt", "a\n");
        t.git(["add", "a.txt"]);
        t.git([
            "commit",
            "-qm",
            "root",
            "--author",
            "First Author <first@example.com>",
        ]);
        t.commit_file("b.txt", "b\n", "second");
        let p = plan(&t.repo(), "HEAD", "HEAD~1").unwrap();
        assert_eq!(p.base, None);
        squash(&t.repo(), &p, "everything").unwrap();
        assert_eq!(subjects(&t), ["everything"]);
        let author = t.git(["show", "-s", "--format=%an <%ae>", "HEAD"]);
        assert_eq!(author.trim(), "First Author <first@example.com>");
    }

    #[test]
    fn refuses_unsafe_selections() {
        let t = five();
        let r = |a: &str, b: &str| plan(&t.repo(), a, b).unwrap_err().to_string();
        assert!(r("HEAD", "HEAD").contains("at least two"));
        // A merge in the range.
        t.git(["checkout", "-qb", "side", "HEAD~1"]);
        t.commit_file("side.txt", "s\n", "side");
        t.git(["checkout", "-q", "main"]);
        t.git(["merge", "-q", "--no-ff", "-m", "merge side", "side"]);
        t.commit_file("f.txt", "f\n", "f");
        assert!(
            r("HEAD", "HEAD~3").contains("merge"),
            "{}",
            r("HEAD", "HEAD~3")
        );
        // Merges after the selection.
        assert!(r("HEAD~4", "HEAD~3").contains("merge"));
        // Two lines of history (side branched off before e).
        assert!(
            r("side", "HEAD~2").contains("different lines"),
            "{}",
            r("side", "HEAD~2")
        );
        // A branch merged in: the merge sits after the selection.
        assert!(r("side", "HEAD~4").contains("merge"));
        // Not on the current branch at all.
        t.git(["checkout", "-qb", "other", "HEAD~2"]);
        t.commit_file("o.txt", "o\n", "o1");
        t.commit_file("o.txt", "o2\n", "o2");
        t.git(["checkout", "-q", "main"]);
        assert!(r("other", "other~1").contains("current branch"));
        // Detached HEAD.
        t.git(["checkout", "-q", "--detach", "HEAD"]);
        assert!(r("HEAD", "HEAD~1").contains("detached"));
    }

    #[test]
    fn refuses_to_replay_over_local_changes_and_after_head_moves() {
        let t = five();
        t.write("a.txt", "local edit\n");
        let e = plan(&t.repo(), "HEAD~3", "HEAD~2").unwrap_err().to_string();
        assert!(e.contains("Commit or stash"), "{e}");
        // At HEAD local changes are fine.
        let p = plan(&t.repo(), "HEAD", "HEAD~1").unwrap();
        t.git(["checkout", "-q", "--", "a.txt"]);
        t.commit_file("g.txt", "g\n", "g");
        let e = squash(&t.repo(), &p, "x").unwrap_err().to_string();
        assert!(e.contains("moved"), "{e}");
        assert_eq!(subjects(&t)[0], "g", "nothing changed");
    }

    #[test]
    fn reports_published_commits() {
        let t = five();
        t.add_bare_origin();
        t.git(["push", "-q", "-u", "origin", "HEAD"]);
        t.commit_file("f.txt", "f\n", "f");
        let p = plan(&t.repo(), "HEAD", "HEAD~1").unwrap();
        assert_eq!(p.published, ["origin/main"]);
        t.git(["branch", "keep", "HEAD~2"]);
        t.git(["tag", "v1", "HEAD~1"]);
        let p = plan(&t.repo(), "HEAD~1", "HEAD~2").unwrap();
        assert_eq!(p.other_refs, ["keep", "v1"]);
        let p = plan(&t.repo(), "HEAD", "HEAD").map(|_| ()).unwrap_err();
        assert!(matches!(p, GitError::Refused(_)));
    }

    #[test]
    fn first_parent_run_follows_one_line() {
        let c = |p: &[&str]| p.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let (pd, pc, pb) = (c(&["c"]), c(&["b"]), c(&["a"]));
        let commits = vec![
            ("d", pd.as_slice()),
            ("c", pc.as_slice()),
            ("b", pb.as_slice()),
        ];
        assert_eq!(
            first_parent_run(commits.clone(), "d", "b").unwrap(),
            ["d", "c", "b"]
        );
        assert_eq!(first_parent_run(commits, "b", "d"), None);
    }
}
