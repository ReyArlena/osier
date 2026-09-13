//! Linked worktrees.

use std::path::{Path, PathBuf};

use crate::error::{GitError, Result};
use crate::repo::Repository;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    pub path: PathBuf,
    pub head: Option<String>,
    /// Branch name without `refs/heads/`, `None` when detached.
    pub branch: Option<String>,
    pub is_main: bool,
    pub is_bare: bool,
    pub locked: bool,
    pub prunable: bool,
}

/// `git worktree list --porcelain -z` parser.
pub fn list(repo: &Repository) -> Result<Vec<Worktree>> {
    let out = repo.git(["worktree", "list", "--porcelain", "-z"])?;
    parse(&out.stdout)
}

pub fn parse(bytes: &[u8]) -> Result<Vec<Worktree>> {
    let text = std::str::from_utf8(bytes).map_err(|_| GitError::Utf8("worktree list"))?;
    let mut out = Vec::new();
    let mut current: Option<Worktree> = None;
    // Records are separated by an empty field; attributes are NUL separated lines.
    for field in text.split('\0') {
        if field.is_empty() {
            if let Some(w) = current.take() {
                out.push(w);
            }
            continue;
        }
        let (key, value) = field.split_once(' ').unwrap_or((field, ""));
        match key {
            "worktree" => {
                if let Some(w) = current.take() {
                    out.push(w);
                }
                current = Some(Worktree {
                    path: PathBuf::from(value),
                    head: None,
                    branch: None,
                    is_main: out.is_empty(),
                    is_bare: false,
                    locked: false,
                    prunable: false,
                });
            }
            "HEAD" => {
                if let Some(w) = &mut current {
                    w.head = Some(value.to_string());
                }
            }
            "branch" => {
                if let Some(w) = &mut current {
                    w.branch = Some(
                        value
                            .strip_prefix("refs/heads/")
                            .unwrap_or(value)
                            .to_string(),
                    );
                }
            }
            "bare" => {
                if let Some(w) = &mut current {
                    w.is_bare = true;
                }
            }
            "locked" => {
                if let Some(w) = &mut current {
                    w.locked = true;
                }
            }
            "prunable" => {
                if let Some(w) = &mut current {
                    w.prunable = true;
                }
            }
            // `detached` and unknown keys: nothing to record.
            _ => {}
        }
    }
    if let Some(w) = current.take() {
        out.push(w);
    }
    Ok(out)
}

/// Add a worktree at `path`. With `new_branch`, creates that branch from `start` (HEAD by
/// default); otherwise checks out the existing `branch`.
pub fn add(
    repo: &Repository,
    path: &Path,
    branch: &str,
    new_branch: bool,
    start: Option<&str>,
) -> Result<()> {
    if branch.trim().is_empty() {
        return Err(GitError::InvalidRefName(branch.to_string()));
    }
    let path_s = path.to_string_lossy().into_owned();
    let mut args: Vec<&str> = vec!["worktree", "add"];
    if new_branch {
        args.extend(["-b", branch, &path_s]);
        if let Some(s) = start {
            args.push(s);
        }
    } else {
        args.extend([path_s.as_str(), branch]);
    }
    repo.git(args)?;
    Ok(())
}

/// Remove a worktree. `force` also removes one with local changes.
pub fn remove(repo: &Repository, path: &Path, force: bool) -> Result<()> {
    let path_s = path.to_string_lossy().into_owned();
    let mut args = vec!["worktree", "remove"];
    if force {
        args.push("--force");
    }
    args.push(&path_s);
    repo.git(args)?;
    Ok(())
}

/// Drop bookkeeping for worktrees whose directories are gone.
pub fn prune(repo: &Repository) -> Result<()> {
    repo.git(["worktree", "prune"])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempRepo;

    #[test]
    fn parses_porcelain() {
        let raw = b"worktree /r\0HEAD aaaa\0branch refs/heads/main\0\0worktree /r-feat\0HEAD bbbb\0branch refs/heads/feat\0locked\0\0worktree /r-det\0HEAD cccc\0detached\0\0";
        let w = parse(raw).unwrap();
        assert_eq!(w.len(), 3);
        assert!(w[0].is_main);
        assert_eq!(w[0].branch.as_deref(), Some("main"));
        assert!(w[1].locked);
        assert_eq!(w[1].branch.as_deref(), Some("feat"));
        assert_eq!(w[2].branch, None);
    }

    #[test]
    fn add_list_remove() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "a\n", "first");
        let repo = t.repo();
        assert_eq!(list(&repo).unwrap().len(), 1);
        let wt = t.path().with_extension("wt-new");
        add(&repo, &wt, "feature", true, None).unwrap();
        let l = list(&repo).unwrap();
        assert_eq!(l.len(), 2);
        assert_eq!(l[1].branch.as_deref(), Some("feature"));
        assert!(l[1].path.ends_with(wt.file_name().unwrap()));
        std::fs::write(wt.join("dirty.txt"), "x").unwrap();
        assert!(
            remove(&repo, &wt, false).is_err(),
            "untracked file blocks removal"
        );
        remove(&repo, &wt, true).unwrap();
        assert_eq!(list(&repo).unwrap().len(), 1);
        assert!(add(&repo, &wt, "", true, None).is_err());
        prune(&repo).unwrap();
    }
}
