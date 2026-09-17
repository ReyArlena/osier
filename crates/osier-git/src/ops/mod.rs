//! Operations that change the repository.
//!
//! Every function here encodes these safety rules: explicit paths, no
//! plain `--force`, typed errors for the cases the UI must handle (dirty tree, unmerged branch,
//! conflicts). Confirmation dialogs live in the UI; these functions assume the user already
//! agreed.

pub mod branch;
pub mod commit;
pub mod discard;
pub mod integrate;
pub mod patch;
pub mod patches;
pub mod rebase_interactive;
pub mod remote;
pub mod reset;
pub mod stage;
pub mod stash;
pub mod tag;
pub mod worktree;

use crate::error::{GitError, Result};

/// Reject names git would refuse, before running anything.
pub fn validate_ref_name(repo: &crate::Repository, name: &str) -> Result<()> {
    if name.trim().is_empty() {
        return Err(GitError::InvalidRefName(name.to_string()));
    }
    let out = repo.runner().run_allowing(
        Some(repo.workdir()),
        ["check-ref-format", "--branch", name],
        &[0, 1],
    )?;
    match out.record.status {
        Some(0) => Ok(()),
        _ => Err(GitError::InvalidRefName(name.to_string())),
    }
}

/// `--` plus paths, so a path starting with `-` is never read as an option.
fn with_paths<'a>(mut args: Vec<&'a str>, paths: &'a [String]) -> Vec<&'a str> {
    args.push("--");
    args.extend(paths.iter().map(String::as_str));
    args
}
