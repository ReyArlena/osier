//! Squash selected commits on the current branch into one.
//!
//! Safety, in order:
//! - [`plan`] refuses anything that is not on the checked-out branch's own line of history:
//!   detached HEAD, an operation in progress, commits off the branch (or only on a branch
//!   merged into it), or merge commits anywhere from the oldest selected commit up to HEAD.
//!   When commits must be replayed over uncommitted changes to tracked files, [`squash`]
//!   refuses unless the caller sets [`SquashPlan::autostash`].
//! - Neighbouring commits: the squashed commit is built with `commit-tree` from the newest
//!   commit's tree, so its content is exactly what the branch had. When the newest is HEAD
//!   the branch moves with a compare-and-swap `update-ref`; the working tree and the index
//!   are not touched, so staged changes stay staged and out of the squash. Otherwise the
//!   later commits are replayed with `rebase --onto`, which cannot conflict (the new base has
//!   the same tree); if it fails anyway it is aborted.
//! - Separated commits: an interactive rebase moves the later selected commits next to the
//!   oldest one and melds them (`fixup`), sets the message, then replays the commits that
//!   were in between. Moving commits past others can conflict; then the rebase is aborted
//!   and the branch is exactly as it was.
//! - The oldest commit's author is kept and commit signing is honoured. [`squash`] re-checks
//!   that HEAD has not moved since the plan was made. The old commits stay in the reflog.

use std::collections::HashSet;

use crate::error::{GitError, Result};
use crate::repo::Repository;

/// What a squash would do, checked and ready to confirm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SquashPlan {
    /// The commits to squash, newest first.
    pub commits: Vec<String>,
    /// Commits between the selected ones, newest first: they stay, replayed after the
    /// squashed commit. Empty when the selection is one unbroken run.
    pub between: Vec<String>,
    /// Parent of the oldest commit; `None` when the oldest is the root commit.
    pub base: Option<String>,
    /// HEAD when the plan was made.
    pub head: String,
    /// Branch being rewritten, without `refs/heads/`.
    pub branch: String,
    /// Commits after the newest selected one, replayed on top unchanged.
    pub replay: usize,
    /// Remote branches (e.g. `origin/main`) that already contain some of these commits:
    /// squashing rewrites published history and needs a force push.
    pub published: Vec<String>,
    /// Other local branches and tags that contain these commits. They keep the old commits.
    pub other_refs: Vec<String>,
    /// The commits' messages, oldest first, joined by blank lines: the default message.
    pub message: String,
    /// Commits must be replayed and there are uncommitted changes to tracked files: the
    /// squash only runs with [`autostash`](Self::autostash) set.
    pub needs_stash: bool,
    /// Set by the caller: let git set uncommitted changes aside while commits are replayed
    /// and put them back afterwards (also when the squash is aborted).
    pub autostash: bool,
}

impl SquashPlan {
    /// True when the selected commits sit next to each other (no commits in between).
    pub fn contiguous(&self) -> bool {
        self.between.is_empty()
    }
}

fn refused(reason: impl Into<String>) -> GitError {
    GitError::Refused(reason.into())
}

fn rev_parse(repo: &Repository, rev: &str) -> Result<String> {
    Ok(repo
        .git(["rev-parse", "--verify", "--quiet", rev])?
        .stdout_str()?
        .trim()
        .to_string())
}

/// Check that `selected` (any order) can be squashed into one commit.
pub fn plan(repo: &Repository, selected: &[String]) -> Result<SquashPlan> {
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
    let mut wanted: HashSet<String> = HashSet::new();
    for rev in selected {
        wanted.insert(rev_parse(repo, rev)?);
    }
    if wanted.len() < 2 {
        return Err(refused("Select at least two commits to squash."));
    }

    // The branch's own line, HEAD backwards along first parents, until every selected
    // commit has been seen.
    let out = repo.git(["rev-list", "--first-parent", "--parents", "HEAD"])?;
    let text = out.stdout_str()?;
    let mut line: Vec<(String, Vec<String>)> = Vec::new();
    let mut found = 0;
    for l in text.lines().filter(|l| !l.is_empty()) {
        let mut parts = l.split_whitespace().map(str::to_string);
        let hash = parts.next().unwrap_or_default();
        let parents: Vec<String> = parts.collect();
        let hit = wanted.contains(&hash);
        line.push((hash, parents));
        if hit {
            found += 1;
            if found == wanted.len() {
                break;
            }
        }
    }
    if found < wanted.len() {
        return Err(refused(format!(
            "Only commits on {branch}'s own line of history can be squashed; the selection \
             includes commits from another branch."
        )));
    }
    if line.iter().any(|(_, parents)| parents.len() > 1) {
        return Err(refused(
            "There are merge commits among or after the selected commits; they cannot be \
             replayed safely.",
        ));
    }

    let positions: Vec<usize> = line
        .iter()
        .enumerate()
        .filter(|(_, (h, _))| wanted.contains(h))
        .map(|(ix, _)| ix)
        .collect();
    let (newest_ix, oldest_ix) = (positions[0], positions[positions.len() - 1]);
    let commits: Vec<String> = positions.iter().map(|&ix| line[ix].0.clone()).collect();
    let between: Vec<String> = (newest_ix..oldest_ix)
        .filter(|ix| !wanted.contains(&line[*ix].0))
        .map(|ix| line[ix].0.clone())
        .collect();
    let base = line[oldest_ix].1.first().cloned();
    let replay = newest_ix;
    let rewritten = replay + between.len();
    let needs_stash = rewritten > 0 && {
        let dirty = repo.git(["status", "--porcelain", "--untracked-files=no"])?;
        !dirty.stdout_str()?.trim().is_empty()
    };

    // Refs that already contain the oldest selected commit keep the old history after the
    // squash: remote branches (published, needs a force push) and other local branches and
    // tags (they keep pointing at the old commits).
    let oldest = line[oldest_ix].0.clone();
    let containing = |prefix: &str| -> Result<Vec<String>> {
        let out = repo.git([
            "for-each-ref",
            "--format=%(refname:short)",
            "--contains",
            oldest.as_str(),
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
        between,
        base,
        head,
        branch,
        replay,
        published,
        other_refs,
        message,
        needs_stash,
        autostash: false,
    })
}

/// "1 commit", "3 commits".
pub(crate) fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
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
    if plan.needs_stash && !plan.autostash {
        let rewritten = plan.replay + plan.between.len();
        return Err(refused(format!(
            "Commit or stash your changes first: {} have to be replayed.",
            count(rewritten, "other commit")
        )));
    }
    if !plan.contiguous() {
        return squash_separated(repo, plan, message);
    }
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
        let replayed = if plan.autostash {
            repo.git(["rebase", "--autostash", "--onto", &new, newest])
        } else {
            repo.git(["rebase", "--onto", &new, newest])
        };
        if let Err(error) = replayed {
            let _ = repo.git(["rebase", "--abort"]);
            return Err(error);
        }
    }
    Ok(new)
}

/// Separated commits: move the later ones next to the oldest, meld them, set the message,
/// replay the rest. Aborted on any failure, leaving the branch as it was.
fn squash_separated(repo: &Repository, plan: &SquashPlan, message: &str) -> Result<String> {
    let Some(oldest) = plan.commits.last() else {
        return Err(refused("Nothing to squash."));
    };
    let dir = tempfile::Builder::new().prefix("osier-squash-").tempdir()?;
    let message_file = dir.path().join("message");
    std::fs::write(&message_file, format!("{}\n", message.trim_end()))?;
    // Oldest first: the oldest selected commit, the other selected ones melded into it, the
    // message, then everything else from the oldest up to HEAD in its original order.
    let out = repo.git([
        "rev-list",
        "--reverse",
        "--first-parent",
        &format!("{oldest}..HEAD"),
    ])?;
    let later: Vec<String> = out
        .stdout_str()?
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    let selected: HashSet<&String> = plan.commits.iter().collect();
    let mut todo = format!("pick {oldest}\n");
    for h in later.iter().filter(|h| selected.contains(h)) {
        todo.push_str(&format!("fixup {h}\n"));
    }
    todo.push_str(&format!(
        "exec git commit --amend --no-verify -F '{}'\n",
        message_file.to_string_lossy().replace('\'', "'\\''")
    ));
    for h in later.iter().filter(|h| !selected.contains(h)) {
        todo.push_str(&format!("pick {h}\n"));
    }
    let base = plan.base.clone().unwrap_or_else(|| "--root".to_string());
    if let Err(error) =
        crate::ops::rebase_interactive::run_todo(repo, &base, &todo, &dir, plan.autostash)
    {
        let _ = repo.git(["rebase", "--abort"]);
        return Err(match error {
            GitError::Conflict(_) => refused(
                "Moving these commits next to each other conflicts with the commits between \
                 them, so nothing was changed. Squash neighbouring commits instead, or use \
                 Interactive rebase to reorder and resolve the conflict by hand.",
            ),
            other => other,
        });
    }
    // The squashed commit sits where the oldest was: count back from HEAD past the replayed
    // commits that were between or after.
    let back = later.len() - (plan.commits.len() - 1);
    rev_parse(repo, &format!("HEAD~{back}"))
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

    fn sel(t: &TempRepo, revs: &[&str]) -> Vec<String> {
        revs.iter()
            .map(|r| rev_parse(&t.repo(), r).unwrap())
            .collect()
    }

    fn plan_of(t: &TempRepo, revs: &[&str]) -> Result<SquashPlan> {
        plan(&t.repo(), &sel(t, revs))
    }

    /// a, b, c, d, e on main, each in its own file.
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
        let p = plan_of(&t, &["HEAD", "HEAD~2", "HEAD~1"]).unwrap();
        assert_eq!(p.commits.len(), 3);
        assert!(p.contiguous());
        assert_eq!(p.replay, 0);
        assert_eq!(p.message, "c\n\nd\n\ne");
        assert!(p.published.is_empty() && p.other_refs.is_empty());
        let new = squash(&t.repo(), &p, "c, d and e").unwrap();
        assert_eq!(head(&t), new);
        assert_eq!(subjects(&t), ["c, d and e", "b", "a"]);
        assert_eq!(tree(&t, "HEAD"), tree_before, "same content");
        let staged = t.git(["diff", "--cached", "--name-only"]);
        assert_eq!(staged.trim(), "a.txt", "staged work stays staged");
        assert_eq!(
            rev_parse(&t.repo(), "HEAD~1").unwrap(),
            p.base.clone().unwrap()
        );
    }

    #[test]
    fn squashes_in_the_middle_and_replays_the_rest() {
        let t = five();
        let tree_before = tree(&t, "HEAD");
        let p = plan_of(&t, &["HEAD~3", "HEAD~2"]).unwrap();
        assert_eq!(p.replay, 2);
        squash(&t.repo(), &p, "b and c").unwrap();
        assert_eq!(subjects(&t), ["e", "d", "b and c", "a"]);
        assert_eq!(tree(&t, "HEAD"), tree_before);
        assert!(t.repo().in_progress().is_none());
    }

    #[test]
    fn squashes_separated_commits_and_keeps_the_ones_between() {
        let t = five();
        let tree_before = tree(&t, "HEAD");
        // b and d, with c between them and e after.
        let p = plan_of(&t, &["HEAD~1", "HEAD~3"]).unwrap();
        assert!(!p.contiguous());
        assert_eq!(p.between.len(), 1);
        assert_eq!(p.replay, 1);
        assert_eq!(p.message, "b\n\nd");
        let new = squash(&t.repo(), &p, "b and d").unwrap();
        assert_eq!(subjects(&t), ["e", "c", "b and d", "a"]);
        assert_eq!(rev_parse(&t.repo(), "HEAD~2").unwrap(), new);
        assert_eq!(tree(&t, "HEAD"), tree_before, "same content at the top");
        let files = t.git(["show", "--name-only", "--format=", &new]);
        let mut files: Vec<&str> = files.lines().filter(|l| !l.is_empty()).collect();
        files.sort_unstable();
        assert_eq!(
            files,
            ["b.txt", "d.txt"],
            "only the selected commits' changes"
        );
        assert!(t.repo().in_progress().is_none());
    }

    #[test]
    fn separated_commits_that_conflict_are_left_alone() {
        let t = TempRepo::new();
        t.commit_file("f.txt", "1\n", "one");
        t.commit_file("f.txt", "2\n", "two");
        t.commit_file("f.txt", "3\n", "three");
        let before = head(&t);
        // one and three both rewrite f.txt with two in between: moving three down conflicts.
        let p = plan_of(&t, &["HEAD", "HEAD~2"]).unwrap();
        let e = squash(&t.repo(), &p, "x").unwrap_err().to_string();
        assert!(e.contains("conflicts"), "{e}");
        assert_eq!(head(&t), before, "branch unchanged");
        assert!(t.repo().in_progress().is_none(), "rebase aborted");
        assert_eq!(subjects(&t), ["three", "two", "one"]);
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
        t.commit_file("c.txt", "c\n", "third");
        let p = plan_of(&t, &["HEAD~1", "HEAD~2"]).unwrap();
        assert_eq!(p.base, None);
        squash(&t.repo(), &p, "first two").unwrap();
        assert_eq!(subjects(&t), ["third", "first two"]);
        let author = t.git(["show", "-s", "--format=%an <%ae>", "HEAD~1"]);
        assert_eq!(author.trim(), "First Author <first@example.com>");
        // Separated, including the root.
        let t = five();
        let p = plan_of(&t, &["HEAD~4", "HEAD~2"]).unwrap();
        assert_eq!(p.base, None);
        squash(&t.repo(), &p, "a and c").unwrap();
        assert_eq!(subjects(&t), ["e", "d", "b", "a and c"]);
    }

    #[test]
    fn refuses_unsafe_selections() {
        let t = five();
        let r = |revs: &[&str]| plan_of(&t, revs).unwrap_err().to_string();
        assert!(r(&["HEAD", "HEAD"]).contains("at least two"));
        // A merge among the selection, and merges after it.
        t.git(["checkout", "-qb", "side", "HEAD~1"]);
        t.commit_file("side.txt", "s\n", "side");
        t.git(["checkout", "-q", "main"]);
        t.git(["merge", "-q", "--no-ff", "-m", "merge side", "side"]);
        t.commit_file("f.txt", "f\n", "f");
        assert!(r(&["HEAD", "HEAD~1"]).contains("merge"));
        assert!(r(&["HEAD~4", "HEAD~3"]).contains("merge"));
        // A commit that only reached the branch through the merge.
        assert!(
            r(&["side", "HEAD~2"]).contains("own line"),
            "{}",
            r(&["side", "HEAD~2"])
        );
        // Not on the current branch at all.
        t.git(["checkout", "-qb", "other", "HEAD~2"]);
        t.commit_file("o.txt", "o\n", "o1");
        t.commit_file("o.txt", "o2\n", "o2");
        t.git(["checkout", "-q", "main"]);
        assert!(r(&["other", "other~1"]).contains("another branch"));
        // Detached HEAD.
        t.git(["checkout", "-q", "--detach", "HEAD"]);
        assert!(r(&["HEAD", "HEAD~1"]).contains("detached"));
    }

    #[test]
    fn refuses_to_replay_over_local_changes_and_after_head_moves() {
        let t = five();
        t.write("a.txt", "local edit\n");
        let p = plan_of(&t, &["HEAD~3", "HEAD~2"]).unwrap();
        assert!(p.needs_stash);
        let e = squash(&t.repo(), &p, "x").unwrap_err().to_string();
        assert!(e.contains("Commit or stash"), "{e}");
        let p = plan_of(&t, &["HEAD", "HEAD~2"]).unwrap();
        let e = squash(&t.repo(), &p, "x").unwrap_err().to_string();
        assert!(
            e.contains("Commit or stash"),
            "separated needs a rebase: {e}"
        );
        // At HEAD local changes are fine.
        let p = plan_of(&t, &["HEAD", "HEAD~1"]).unwrap();
        t.git(["checkout", "-q", "--", "a.txt"]);
        t.commit_file("g.txt", "g\n", "g");
        let e = squash(&t.repo(), &p, "x").unwrap_err().to_string();
        assert!(e.contains("moved"), "{e}");
        assert_eq!(subjects(&t)[0], "g", "nothing changed");
    }

    #[test]
    fn autostash_squashes_over_local_changes_and_keeps_them() {
        for revs in [["HEAD~3", "HEAD~2"], ["HEAD", "HEAD~2"]] {
            let t = five();
            t.write("a.txt", "local edit\n");
            let mut p = plan_of(&t, &revs).unwrap();
            p.autostash = true;
            squash(&t.repo(), &p, "squashed").unwrap();
            assert_eq!(t.read("a.txt"), "local edit\n", "{revs:?}");
            assert_eq!(crate::stash::autostash_count(&t.repo()), 0, "{revs:?}");
        }
    }

    #[test]
    fn reports_published_commits_and_other_refs() {
        let t = five();
        t.add_bare_origin();
        t.git(["push", "-q", "-u", "origin", "HEAD"]);
        t.commit_file("f.txt", "f\n", "f");
        let p = plan_of(&t, &["HEAD", "HEAD~1"]).unwrap();
        assert_eq!(p.published, ["origin/main"]);
        t.git(["branch", "keep", "HEAD~2"]);
        t.git(["tag", "v1", "HEAD~1"]);
        let p = plan_of(&t, &["HEAD~1", "HEAD~2"]).unwrap();
        assert_eq!(p.other_refs, ["keep", "v1"]);
        let e = plan_of(&t, &["HEAD", "HEAD"]).map(|_| ()).unwrap_err();
        assert!(matches!(e, GitError::Refused(_)));
    }
}
