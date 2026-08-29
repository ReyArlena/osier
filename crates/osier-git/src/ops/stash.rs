//! Stash push, apply, pop, drop.

use crate::error::Result;
use crate::repo::Repository;

#[derive(Debug, Clone, Default)]
pub struct StashOptions {
    pub message: Option<String>,
    pub keep_index: bool,
    pub include_untracked: bool,
    /// Only these paths; empty means everything.
    pub paths: Vec<String>,
}

pub fn push(repo: &Repository, options: &StashOptions) -> Result<()> {
    let mut args: Vec<String> = vec!["stash".into(), "push".into(), "--quiet".into()];
    if options.keep_index {
        args.push("--keep-index".into());
    }
    if options.include_untracked {
        args.push("--include-untracked".into());
    }
    if let Some(m) = &options.message {
        args.push("--message".into());
        args.push(m.clone());
    }
    if !options.paths.is_empty() {
        args.push("--".into());
        args.extend(options.paths.iter().cloned());
    }
    repo.git(args)?;
    Ok(())
}

fn selector(index: usize) -> String {
    format!("stash@{{{index}}}")
}

pub fn apply(repo: &Repository, index: usize) -> Result<()> {
    repo.git(["stash", "apply", &selector(index)])?;
    Ok(())
}

pub fn pop(repo: &Repository, index: usize) -> Result<()> {
    repo.git(["stash", "pop", &selector(index)])?;
    Ok(())
}

pub fn drop(repo: &Repository, index: usize) -> Result<()> {
    repo.git(["stash", "drop", "--quiet", &selector(index)])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stash::list;
    use crate::testing::TempRepo;

    #[test]
    fn push_apply_pop_drop() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "a\n", "first");
        t.write("a.txt", "work\n");
        t.write("u.txt", "untracked\n");
        let repo = t.repo();
        push(
            &repo,
            &StashOptions {
                message: Some("wip".into()),
                include_untracked: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(t.read("a.txt"), "a\n");
        assert!(!t.path().join("u.txt").exists());
        let stashes = list(&repo).unwrap();
        assert_eq!(stashes.len(), 1);
        assert_eq!(stashes[0].message, "On main: wip");

        apply(&repo, 0).unwrap();
        assert_eq!(t.read("a.txt"), "work\n");
        assert!(t.path().join("u.txt").exists());
        assert_eq!(list(&repo).unwrap().len(), 1, "apply keeps the stash");

        t.git(["checkout", "--", "a.txt"]);
        t.remove("u.txt");
        pop(&repo, 0).unwrap();
        assert_eq!(t.read("a.txt"), "work\n");
        assert!(list(&repo).unwrap().is_empty());

        push(&repo, &StashOptions::default()).unwrap();
        drop(&repo, 0).unwrap();
        assert!(list(&repo).unwrap().is_empty());
    }

    #[test]
    fn apply_conflict_is_typed() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "a\n", "first");
        t.write("a.txt", "stashed\n");
        let repo = t.repo();
        push(&repo, &StashOptions::default()).unwrap();
        t.commit_file("a.txt", "other\n", "second");
        assert!(matches!(apply(&repo, 0), Err(crate::GitError::Conflict(_))));
    }
}
