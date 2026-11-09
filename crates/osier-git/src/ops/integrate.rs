//! Merge, rebase, cherry-pick, revert continuation, and conflict resolution.
//!
//! Every multi-step operation runs with `GIT_EDITOR=true` so git never opens an editor; the
//! default messages are kept. Continuation and abort are explicit functions so the UI can
//! show an in-progress bar.

use std::path::Path;

use crate::error::{GitError, Result};
use crate::repo::{InProgress, Repository};
use crate::runner::GitRunner;

fn no_editor(repo: &Repository) -> GitRunner {
    repo.runner().clone().with_env("GIT_EDITOR", "true")
}

fn run(repo: &Repository, args: &[&str]) -> Result<()> {
    no_editor(repo).run(Some(repo.workdir()), args)?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeMode {
    /// Fast-forward when possible, otherwise a merge commit (git's default).
    Default,
    /// Always create a merge commit.
    NoFastForward,
    /// Fast-forward or fail.
    FastForwardOnly,
    /// Bring the changes into the index without committing.
    Squash,
}

/// Merge `branch` into the current branch. Conflicts surface as [`GitError::Conflict`] and
/// leave the repository in the merge state for [`continue_in_progress`] / [`abort`].
/// `autostash` sets uncommitted changes aside first; git puts them back when the merge ends
/// (including after a continue or abort).
pub fn merge(repo: &Repository, branch: &str, mode: MergeMode, autostash: bool) -> Result<()> {
    let mut args = vec!["merge", "--no-edit"];
    if autostash {
        args.push("--autostash");
    }
    match mode {
        MergeMode::Default => {}
        MergeMode::NoFastForward => args.push("--no-ff"),
        MergeMode::FastForwardOnly => args.push("--ff-only"),
        MergeMode::Squash => args.push("--squash"),
    }
    args.push(branch);
    run(repo, &args)
}

/// Rebase the current branch onto `onto`. `autostash` as for [`merge`].
pub fn rebase(repo: &Repository, onto: &str, autostash: bool) -> Result<()> {
    if autostash {
        run(repo, &["rebase", "--autostash", onto])
    } else {
        run(repo, &["rebase", onto])
    }
}

/// Cherry-pick one or more commits (oldest first). Each becomes its own commit.
pub fn cherry_pick(repo: &Repository, commits: &[String]) -> Result<()> {
    if commits.is_empty() {
        return Ok(());
    }
    let mut args: Vec<&str> = vec!["cherry-pick"];
    args.extend(commits.iter().map(String::as_str));
    run(repo, &args)
}

/// Continue whatever multi-step operation is in progress.
pub fn continue_in_progress(repo: &Repository) -> Result<()> {
    match repo.in_progress() {
        Some(InProgress::Merge) => {
            // A merge continues by committing the resolved index.
            run(repo, &["commit", "--no-edit"])
        }
        Some(InProgress::Rebase) => run(repo, &["rebase", "--continue"]),
        Some(InProgress::CherryPick) => run(repo, &["cherry-pick", "--continue"]),
        Some(InProgress::Revert) => run(repo, &["revert", "--continue"]),
        Some(InProgress::Bisect) | None => Err(GitError::parse("continue", "nothing in progress")),
    }
}

/// Skip the current commit of a rebase or cherry-pick.
pub fn skip_in_progress(repo: &Repository) -> Result<()> {
    match repo.in_progress() {
        Some(InProgress::Rebase) => run(repo, &["rebase", "--skip"]),
        Some(InProgress::CherryPick) => run(repo, &["cherry-pick", "--skip"]),
        Some(InProgress::Revert) => run(repo, &["revert", "--skip"]),
        _ => Err(GitError::parse("skip", "nothing skippable in progress")),
    }
}

/// Abort the operation in progress and return to the pre-operation state.
pub fn abort(repo: &Repository) -> Result<()> {
    match repo.in_progress() {
        Some(InProgress::Merge) => run(repo, &["merge", "--abort"]),
        Some(InProgress::Rebase) => run(repo, &["rebase", "--abort"]),
        Some(InProgress::CherryPick) => run(repo, &["cherry-pick", "--abort"]),
        Some(InProgress::Revert) => run(repo, &["revert", "--abort"]),
        Some(InProgress::Bisect) => run(repo, &["bisect", "reset"]),
        None => Err(GitError::parse("abort", "nothing in progress")),
    }
}

/// Which side to take for a conflicted file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictSide {
    /// The current branch (`--ours`). During a rebase this is the branch being rebased onto.
    Ours,
    /// The incoming branch (`--theirs`).
    Theirs,
}

/// Take one side for `paths` and mark them resolved (staged).
pub fn resolve_with(repo: &Repository, paths: &[String], side: ConflictSide) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    let flag = match side {
        ConflictSide::Ours => "--ours",
        ConflictSide::Theirs => "--theirs",
    };
    let mut args: Vec<&str> = vec!["checkout", flag, "--"];
    args.extend(paths.iter().map(String::as_str));
    repo.git(args)?;
    let mut add: Vec<&str> = vec!["add", "--"];
    add.extend(paths.iter().map(String::as_str));
    repo.git(add)?;
    Ok(())
}

/// Mark `paths` resolved after manual editing (stages them).
pub fn mark_resolved(repo: &Repository, paths: &[String]) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    let mut add: Vec<&str> = vec!["add", "--"];
    add.extend(paths.iter().map(String::as_str));
    repo.git(add)?;
    Ok(())
}

/// The three versions of a conflicted file: base (stage 1), ours (2), theirs (3).
/// A missing side (add/add or delete conflicts) is `None`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConflictVersions {
    pub base: Option<String>,
    pub ours: Option<String>,
    pub theirs: Option<String>,
}

pub fn conflict_versions(repo: &Repository, path: &str) -> Result<ConflictVersions> {
    let mut v = ConflictVersions::default();
    for (stage, slot) in [(1, &mut v.base), (2, &mut v.ours), (3, &mut v.theirs)] {
        let spec = format!(":{stage}:{path}");
        let out = repo
            .runner()
            .run_allowing(Some(repo.workdir()), ["show", &spec], &[0, 128])?;
        if out.record.status == Some(0) {
            *slot = Some(String::from_utf8_lossy(&out.stdout).into_owned());
        }
    }
    Ok(v)
}

/// Write `content` to the working tree copy of `path` (the merge editor's result).
pub fn write_resolution(repo: &Repository, path: &str, content: &str) -> Result<()> {
    let rel = Path::new(path);
    if rel.is_absolute()
        || rel
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(GitError::parse(
            "resolution",
            format!("{path} escapes the repository"),
        ));
    }
    std::fs::write(repo.workdir().join(rel), content)?;
    Ok(())
}

/// Text still containing conflict markers?
/// Whether the working-tree copy of `path` still has conflict markers. A file that cannot be
/// read as text (deleted, binary) has none to find, so it counts as clean.
pub fn file_has_conflict_markers(repo: &Repository, path: &str) -> bool {
    std::fs::read_to_string(repo.workdir().join(path)).is_ok_and(|t| has_conflict_markers(&t))
}

pub fn has_conflict_markers(text: &str) -> bool {
    text.lines().any(|l| {
        l.starts_with("<<<<<<< ")
            || l == "======="
            || l.starts_with(">>>>>>> ")
            || l.starts_with("||||||| ")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::{LogOptions, log};
    use crate::status::{StatusOptions, status};
    use crate::testing::TempRepo;

    /// main and feature both change `c.txt` from "base".
    fn diverged() -> TempRepo {
        let t = TempRepo::new();
        t.commit_file("c.txt", "base\n", "base");
        t.commit_file("keep.txt", "k\n", "keep");
        t.checkout_new("feature");
        t.commit_file("c.txt", "theirs\n", "theirs");
        t.commit_file("f.txt", "f\n", "feature only");
        t.checkout("main");
        t.commit_file("c.txt", "ours\n", "ours");
        t
    }

    #[test]
    fn autostash_puts_uncommitted_changes_back() {
        // keep.txt is edited but not committed; rebase refuses that without autostash.
        let t = diverged();
        t.git(["reset", "--hard", "HEAD~1"]);
        t.write("keep.txt", "edited\n");
        let repo = t.repo();
        assert!(rebase(&repo, "feature", false).is_err());
        rebase(&repo, "feature", true).unwrap();
        assert_eq!(t.read("keep.txt"), "edited\n");
        assert_eq!(crate::stash::autostash_count(&repo), 0);

        // A merge that stops for conflicts puts them back on abort.
        let t = diverged();
        t.write("keep.txt", "edited\n");
        let repo = t.repo();
        assert!(matches!(
            merge(&repo, "feature", MergeMode::Default, true),
            Err(GitError::Conflict(_))
        ));
        abort(&repo).unwrap();
        assert_eq!(t.read("keep.txt"), "edited\n");
        assert_eq!(crate::stash::autostash_count(&repo), 0);
    }

    #[test]
    fn autostash_that_clashes_is_left_in_the_stash_list() {
        // feature changes keep.txt; the uncommitted edit to it cannot go back cleanly.
        let t = TempRepo::new();
        t.commit_file("keep.txt", "k\n", "base");
        t.checkout_new("feature");
        t.commit_file("keep.txt", "theirs\n", "theirs");
        t.checkout("main");
        t.write("keep.txt", "mine\n");
        let repo = t.repo();
        let _ = rebase(&repo, "feature", true);
        assert_eq!(crate::stash::autostash_count(&repo), 1);
    }

    #[test]
    fn merge_modes_and_conflict_flow() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "a\n", "base");
        t.checkout_new("feature");
        t.commit_file("b.txt", "b\n", "feature");
        t.checkout("main");
        let repo = t.repo();
        merge(&repo, "feature", MergeMode::FastForwardOnly, false).unwrap();
        assert_eq!(
            t.git(["rev-list", "--merges", "--count", "HEAD"]).trim(),
            "0"
        );

        t.checkout("feature");
        t.commit_file("c.txt", "c\n", "more");
        t.checkout("main");
        t.commit_file("d.txt", "d\n", "main work");
        assert!(matches!(
            merge(&repo, "feature", MergeMode::FastForwardOnly, false),
            Err(GitError::NonFastForward(_)) | Err(GitError::Failed { .. })
        ));
        merge(&repo, "feature", MergeMode::NoFastForward, false).unwrap();
        assert_eq!(
            t.git(["rev-list", "--merges", "--count", "HEAD"]).trim(),
            "1"
        );

        t.checkout("feature");
        t.commit_file("e.txt", "e\n", "even more");
        t.checkout("main");
        merge(&repo, "feature", MergeMode::Squash, false).unwrap();
        let s = status(&repo, StatusOptions::default()).unwrap();
        assert!(
            s.staged().any(|e| e.path == "e.txt"),
            "squash leaves changes staged"
        );
        assert_eq!(repo.in_progress(), None);
    }

    #[test]
    fn merge_conflict_resolve_ours_and_continue() {
        let t = diverged();
        let repo = t.repo();
        assert!(matches!(
            merge(&repo, "feature", MergeMode::Default, false),
            Err(GitError::Conflict(_))
        ));
        assert_eq!(repo.in_progress(), Some(InProgress::Merge));
        let v = conflict_versions(&repo, "c.txt").unwrap();
        assert_eq!(v.base.as_deref(), Some("base\n"));
        assert_eq!(v.ours.as_deref(), Some("ours\n"));
        assert_eq!(v.theirs.as_deref(), Some("theirs\n"));
        assert!(has_conflict_markers(&t.read("c.txt")));

        resolve_with(&repo, &["c.txt".into()], ConflictSide::Ours).unwrap();
        assert_eq!(t.read("c.txt"), "ours\n");
        assert!(
            status(&repo, StatusOptions::default())
                .unwrap()
                .conflicted()
                .next()
                .is_none()
        );
        continue_in_progress(&repo).unwrap();
        assert_eq!(repo.in_progress(), None);
        assert_eq!(
            t.git(["rev-list", "--merges", "--count", "HEAD"]).trim(),
            "1"
        );
        assert!(t.path().join("f.txt").exists());
    }

    #[test]
    fn merge_conflict_manual_resolution_and_abort() {
        let t = diverged();
        let repo = t.repo();
        let head = t.head();
        assert!(merge(&repo, "feature", MergeMode::Default, false).is_err());
        write_resolution(&repo, "c.txt", "both\n").unwrap();
        mark_resolved(&repo, &["c.txt".into()]).unwrap();
        assert!(
            status(&repo, StatusOptions::default())
                .unwrap()
                .conflicted()
                .next()
                .is_none()
        );
        abort(&repo).unwrap();
        assert_eq!(repo.in_progress(), None);
        assert_eq!(t.head(), head);
        assert_eq!(t.read("c.txt"), "ours\n");
        assert!(write_resolution(&repo, "../x", "nope").is_err());
    }

    #[test]
    fn rebase_conflict_theirs_continue_and_skip() {
        let t = diverged();
        let repo = t.repo();
        t.checkout("feature");
        assert!(matches!(
            rebase(&repo, "main", false),
            Err(GitError::Conflict(_))
        ));
        assert_eq!(repo.in_progress(), Some(InProgress::Rebase));
        // During a rebase "theirs" is the commit being replayed (the feature change).
        resolve_with(&repo, &["c.txt".into()], ConflictSide::Theirs).unwrap();
        assert_eq!(t.read("c.txt"), "theirs\n");
        continue_in_progress(&repo).unwrap();
        assert_eq!(repo.in_progress(), None);
        let subjects: Vec<String> = log(&repo, &LogOptions::default())
            .unwrap()
            .into_iter()
            .map(|c| c.subject)
            .collect();
        assert_eq!(
            subjects[..3],
            [
                "feature only".to_string(),
                "theirs".to_string(),
                "ours".to_string()
            ]
        );

        // Skip: redo the rebase and drop the conflicting commit instead.
        t.git(["reset", "-q", "--hard", "main"]);
        t.commit_file("c.txt", "theirs again\n", "theirs again");
        t.checkout("main");
        t.commit_file("c.txt", "ours again\n", "ours again");
        t.checkout("feature");
        assert!(rebase(&repo, "main", false).is_err());
        skip_in_progress(&repo).unwrap();
        assert_eq!(repo.in_progress(), None);
        assert_eq!(t.read("c.txt"), "ours again\n");
    }

    #[test]
    fn cherry_pick_single_range_and_abort() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "a\n", "base");
        t.checkout_new("feature");
        let c1 = t.commit_file("one.txt", "1\n", "one");
        let c2 = t.commit_file("two.txt", "2\n", "two");
        t.commit_file("a.txt", "feature\n", "clash");
        t.checkout("main");
        let repo = t.repo();
        cherry_pick(&repo, std::slice::from_ref(&c1)).unwrap();
        assert!(t.path().join("one.txt").exists());
        cherry_pick(&repo, &[c2]).unwrap();
        assert_eq!(log(&repo, &LogOptions::default()).unwrap().len(), 3);

        t.commit_file("a.txt", "main\n", "main clash");
        let clash = t.git(["rev-parse", "feature"]).trim().to_string();
        assert!(matches!(
            cherry_pick(&repo, &[clash]),
            Err(GitError::Conflict(_))
        ));
        assert_eq!(repo.in_progress(), Some(InProgress::CherryPick));
        abort(&repo).unwrap();
        assert_eq!(repo.in_progress(), None);
        assert_eq!(t.read("a.txt"), "main\n");
        assert!(continue_in_progress(&repo).is_err());
    }
}
