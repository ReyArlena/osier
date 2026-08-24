//! Submodules declared in `.gitmodules`. Listed, never recursed into.

use crate::error::{GitError, Result};
use crate::repo::Repository;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Submodule {
    pub name: String,
    pub path: String,
    pub url: String,
    /// Checked-out commit when initialised, `None` when not initialised.
    pub head: Option<String>,
}

pub fn list(repo: &Repository) -> Result<Vec<Submodule>> {
    let modules = repo.workdir().join(".gitmodules");
    if !modules.is_file() {
        return Ok(Vec::new());
    }
    let out = repo.runner().run_allowing(
        Some(repo.workdir()),
        ["config", "--file", ".gitmodules", "--list", "-z"],
        &[0, 1],
    )?;
    let mut subs = match out.record.status {
        Some(0) => parse_config(&out.stdout)?,
        Some(1) => Vec::new(),
        Some(s) => return Err(GitError::failed(s, out.record)),
        None => return Err(GitError::timeout(0, out.record)),
    };
    // `submodule status` shows the checked-out commit; a leading `-` means not initialised.
    let status = repo.git(["submodule", "status"])?;
    for line in status.stdout_str()?.lines() {
        let line = line.trim_start_matches([' ', '+', 'U']);
        let (initialised, rest) = match line.strip_prefix('-') {
            Some(rest) => (false, rest),
            None => (true, line),
        };
        let mut it = rest.split(' ');
        let (Some(sha), Some(path)) = (it.next(), it.next()) else {
            continue;
        };
        if let Some(s) = subs.iter_mut().find(|s| s.path == path) {
            s.head = initialised.then(|| sha.to_string());
        }
    }
    Ok(subs)
}

/// Parse `git config --file .gitmodules --list -z` output.
pub fn parse_config(bytes: &[u8]) -> Result<Vec<Submodule>> {
    let mut subs: Vec<Submodule> = Vec::new();
    for rec in bytes.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let text = std::str::from_utf8(rec).map_err(|_| GitError::Utf8("gitmodules"))?;
        let (key, value) = text
            .split_once('\n')
            .ok_or_else(|| GitError::parse("gitmodules", text.to_string()))?;
        let Some(rest) = key.strip_prefix("submodule.") else {
            continue;
        };
        let Some((name, field)) = rest.rsplit_once('.') else {
            continue;
        };
        let entry = match subs.iter_mut().find(|s| s.name == name) {
            Some(e) => e,
            None => {
                subs.push(Submodule {
                    name: name.to_string(),
                    path: String::new(),
                    url: String::new(),
                    head: None,
                });
                subs.last_mut()
                    .ok_or_else(|| GitError::parse("gitmodules", "push"))?
            }
        };
        match field {
            "path" => entry.path = value.to_string(),
            "url" => entry.url = value.to_string(),
            _ => {}
        }
    }
    Ok(subs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempRepo;

    #[test]
    fn parses_gitmodules_config() {
        let s = parse_config(b"submodule.libs/dep.path\nlibs/dep\0submodule.libs/dep.url\nhttps://example.com/dep.git\0").unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].name, "libs/dep");
        assert_eq!(s[0].path, "libs/dep");
        assert_eq!(s[0].url, "https://example.com/dep.git");
    }

    #[test]
    fn repo_without_submodules_is_empty() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "a\n", "first");
        assert!(list(&t.repo()).unwrap().is_empty());
    }

    #[test]
    fn live_submodule_is_listed_with_head() {
        let dep = TempRepo::new();
        let dep_head = dep.commit_file("d.txt", "d\n", "dep");
        let t = TempRepo::new();
        t.commit_file("a.txt", "a\n", "first");
        t.git([
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "-q",
            dep.path().to_string_lossy().as_ref(),
            "libs/dep",
        ]);
        t.commit("add submodule");
        let subs = list(&t.repo()).unwrap();
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].path, "libs/dep");
        assert_eq!(subs[0].head.as_deref(), Some(dep_head.as_str()));
    }
}
