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
    let out =
        repo.runner()
            .run_allowing(Some(repo.workdir()), ["config", "--get", key], &[0, 1])?;
    match out.record.status {
        Some(0) => Ok(Some(out.stdout_str()?.trim_end_matches('\n').to_string())),
        Some(1) => Ok(None),
        Some(s) => Err(GitError::failed(s, out.record)),
        None => Err(GitError::timeout(0, out.record)),
    }
}

/// Read a boolean config value with git's own parsing. `None` when unset.
pub fn get_bool(repo: &Repository, key: &str) -> Result<Option<bool>> {
    let out = repo.runner().run_allowing(
        Some(repo.workdir()),
        ["config", "--get", "--type=bool", key],
        &[0, 1],
    )?;
    match out.record.status {
        Some(0) => Ok(Some(out.stdout_str()?.trim() == "true")),
        Some(1) => Ok(None),
        Some(s) => Err(GitError::failed(s, out.record)),
        None => Err(GitError::timeout(0, out.record)),
    }
}

/// Where a config value comes from (`git config --show-scope`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// `/etc/gitconfig` (or the build's system file).
    System,
    /// The user's `~/.gitconfig` or `~/.config/git/config`, and files they include.
    Global,
    /// The repository's `.git/config`, and files it includes.
    Local,
    /// `.git/config.worktree` (with `extensions.worktreeConfig`).
    Worktree,
    /// `git -c` or the `GIT_CONFIG_*` / `GIT_CONFIG_PARAMETERS` environment.
    Command,
    Other(String),
}

impl Scope {
    fn parse(s: &str) -> Self {
        match s {
            "system" => Scope::System,
            "global" => Scope::Global,
            "local" => Scope::Local,
            "worktree" => Scope::Worktree,
            "command" => Scope::Command,
            other => Scope::Other(other.to_string()),
        }
    }
}

/// One `key = value` line from any config source, in the order git reads them (later lines
/// win for single-valued keys).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigEntry {
    /// As git prints it: section and name lower case, subsection as written
    /// (`branch.Main.rebase` stays `branch.Main.rebase`).
    pub key: String,
    /// `None` for a bare `key` line, which git reads as boolean true.
    pub value: Option<String>,
    pub scope: Scope,
    /// The file (`file:<path>`, relative paths are relative to the repository root) or other
    /// origin (`command line:`, `blob:<id>`, `standard input:`).
    pub origin: String,
}

impl ConfigEntry {
    /// The file the value comes from, when it comes from a file.
    pub fn file(&self) -> Option<&str> {
        self.origin.strip_prefix("file:")
    }
}

/// The settings Osier passes with `-c` on every command; they are not the user's.
const OWN_OVERRIDES: [(&str, &str); 2] = [("color.ui", "never"), ("core.quotepath", "false")];

/// Every config value git sees in this repository, with its scope and origin, in git's
/// reading order. Osier's own `-c` overrides are left out, so this is the user's setup.
pub fn list_all(repo: &Repository) -> Result<Vec<ConfigEntry>> {
    let out = repo.git(["config", "--list", "--show-origin", "--show-scope", "-z"])?;
    parse_list(&out.stdout)
}

/// Parse `config --list --show-origin --show-scope -z`: per entry `scope NUL origin NUL
/// key LF value NUL`, or `key NUL` without a value.
fn parse_list(raw: &[u8]) -> Result<Vec<ConfigEntry>> {
    let text = std::str::from_utf8(raw).map_err(|_| GitError::Utf8("config --list"))?;
    let mut fields = text.split('\0');
    let mut out = Vec::new();
    while let Some(scope) = fields.next() {
        if scope.is_empty() {
            break;
        }
        let (Some(origin), Some(kv)) = (fields.next(), fields.next()) else {
            return Err(GitError::Parse {
                context: "config --list",
                detail: format!("entry for scope {scope:?} is cut short"),
            });
        };
        let (key, value) = match kv.split_once('\n') {
            Some((k, v)) => (k, Some(v.to_string())),
            None => (kv, None),
        };
        if key.is_empty() {
            return Err(GitError::Parse {
                context: "config --list",
                detail: format!("empty key from {origin:?}"),
            });
        }
        let scope = Scope::parse(scope);
        if scope == Scope::Command
            && OWN_OVERRIDES
                .iter()
                .any(|(k, v)| key == *k && value.as_deref() == Some(v))
        {
            continue;
        }
        out.push(ConfigEntry {
            key: key.to_string(),
            value,
            scope,
            origin: origin.to_string(),
        });
    }
    Ok(out)
}

/// The hooks folder git uses (honours `core.hooksPath`) and the hooks in it that git would
/// run: executable files, not the `.sample` ones `git init` writes. Sorted by name.
pub fn hooks(repo: &Repository) -> Result<(std::path::PathBuf, Vec<String>)> {
    let out = repo.git(["rev-parse", "--path-format=absolute", "--git-path", "hooks"])?;
    let dir = std::path::PathBuf::from(out.stdout_str()?.trim_end_matches('\n'));
    let mut names = Vec::new();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((dir, names)),
        Err(e) => return Err(e.into()),
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".sample") {
            continue;
        }
        let executable = {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::metadata(entry.path())
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        };
        if executable {
            names.push(name);
        }
    }
    names.sort();
    Ok((dir, names))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempRepo;

    #[test]
    fn list_all_has_scope_and_origin_and_drops_osier_overrides() {
        let t = TempRepo::new();
        t.git(["config", "pull.rebase", "true"]);
        t.git(["config", "branch.Main.rebase", "false"]);
        let entries = list_all(&t.repo()).unwrap();
        let rebase = entries.iter().find(|e| e.key == "pull.rebase").unwrap();
        assert_eq!(rebase.value.as_deref(), Some("true"));
        assert_eq!(rebase.scope, Scope::Local);
        assert_eq!(rebase.file(), Some(".git/config"));
        assert!(
            entries.iter().any(|e| e.key == "branch.Main.rebase"),
            "subsection case kept"
        );
        assert!(
            !entries
                .iter()
                .any(|e| e.key == "color.ui" || e.key == "core.quotepath"),
            "Osier's own -c values are not listed"
        );
    }

    #[test]
    fn parse_list_reads_bare_keys_and_rejects_garbage() {
        let raw = b"local\0file:.git/config\0core.bare\nfalse\0local\0file:.git/config\0x.flag\0";
        let entries = parse_list(raw).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1].key, "x.flag");
        assert_eq!(entries[1].value, None);
        assert!(parse_list(b"local\0file:.git/config\0").is_err());
        assert!(parse_list(b"local\0file:x\0\nvalue\0").is_err());
        assert!(parse_list(b"").unwrap().is_empty());
    }

    #[test]
    fn hooks_lists_executable_non_sample_files() {
        use std::os::unix::fs::PermissionsExt as _;
        let t = TempRepo::new();
        let dir = t.git_dir().join("hooks");
        std::fs::create_dir_all(&dir).unwrap();
        for (name, mode) in [
            ("pre-commit", 0o755),
            ("pre-push.sample", 0o755),
            ("commit-msg", 0o644),
        ] {
            let p = dir.join(name);
            std::fs::write(&p, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
        }
        let (found_dir, names) = hooks(&t.repo()).unwrap();
        assert_eq!(found_dir, dir);
        assert_eq!(names, vec!["pre-commit".to_string()]);
    }

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
