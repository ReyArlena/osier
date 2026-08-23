//! Repository-scoped git configuration reads.

use crate::error::{GitError, Result};
use crate::repo::Repository;

/// Author identity as git would resolve it for this repository (honours `includeIf`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Identity {
    pub name: Option<String>,
    pub email: Option<String>,
}

pub fn identity(repo: &Repository) -> Result<Identity> {
    Ok(Identity {
        name: get(repo, "user.name")?,
        email: get(repo, "user.email")?,
    })
}

/// Read one config value as git resolves it in this repo. `None` when unset.
pub fn get(repo: &Repository, key: &str) -> Result<Option<String>> {
    let out = repo
        .runner()
        .run_unchecked(Some(repo.workdir()), ["config", "--get", key], None)?;
    match out.record.status {
        Some(0) => Ok(Some(out.stdout_str()?.trim_end_matches('\n').to_string())),
        Some(1) => Ok(None),
        Some(s) => Err(GitError::failed(s, out.record)),
        None => Err(GitError::timeout(0, out.record)),
    }
}

/// Read a boolean config value with git's own parsing. `None` when unset.
pub fn get_bool(repo: &Repository, key: &str) -> Result<Option<bool>> {
    let out = repo.runner().run_unchecked(
        Some(repo.workdir()),
        ["config", "--get", "--type=bool", key],
        None,
    )?;
    match out.record.status {
        Some(0) => Ok(Some(out.stdout_str()?.trim() == "true")),
        Some(1) => Ok(None),
        Some(s) => Err(GitError::failed(s, out.record)),
        None => Err(GitError::timeout(0, out.record)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempRepo;

    #[test]
    fn identity_and_missing_keys() {
        let t = TempRepo::new();
        let repo = t.repo();
        let id = identity(&repo).unwrap();
        assert_eq!(id.name.as_deref(), Some("Osier Test"));
        assert_eq!(id.email.as_deref(), Some("test@example.com"));
        assert_eq!(get(&repo, "osier.nothing").unwrap(), None);
        assert_eq!(get_bool(&repo, "commit.gpgsign").unwrap(), Some(false));
        assert_eq!(get_bool(&repo, "pull.rebase").unwrap(), None);
    }
}
