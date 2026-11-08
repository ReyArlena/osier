//! Stash list.

use crate::error::{GitError, Result};
use crate::repo::Repository;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StashEntry {
    /// Position, 0 is the newest.
    pub index: usize,
    /// `stash@{0}` style selector.
    pub selector: String,
    pub oid: String,
    /// Reflog message, e.g. `WIP on main: abc123 subject` or `On main: my message`.
    pub message: String,
    pub created_at: i64,
}

impl StashEntry {
    /// Branch the stash was made on, when the message follows git's default shape.
    pub fn branch(&self) -> Option<&str> {
        let rest = self
            .message
            .strip_prefix("WIP on ")
            .or_else(|| self.message.strip_prefix("On "))?;
        rest.split_once(':').map(|(b, _)| b)
    }
}

/// How many stash entries git's `--autostash` left behind. It names them `autostash` and
/// keeps one only when putting the changes back clashed, so a rise after an operation means
/// the changes are partly in the files (with conflicts) and fully in that stash.
pub fn autostash_count(repo: &Repository) -> usize {
    list(repo)
        .map(|entries| {
            entries
                .iter()
                .filter(|e| e.message == "autostash" || e.message.ends_with(": autostash"))
                .count()
        })
        .unwrap_or(0)
}

pub fn list(repo: &Repository) -> Result<Vec<StashEntry>> {
    let out = repo.git(["stash", "list", "-z", "--format=%gd%x1f%H%x1f%gs%x1f%ct"])?;
    parse(&out.stdout)
}

pub fn parse(bytes: &[u8]) -> Result<Vec<StashEntry>> {
    bytes
        .split(|b| *b == 0)
        .filter(|r| !r.is_empty())
        .enumerate()
        .map(|(index, rec)| {
            let text = std::str::from_utf8(rec).map_err(|_| GitError::Utf8("stash"))?;
            let f: Vec<&str> = text.splitn(4, '\x1f').collect();
            if f.len() != 4 {
                return Err(GitError::parse("stash list", text.to_string()));
            }
            Ok(StashEntry {
                index,
                selector: f[0].to_string(),
                oid: f[1].to_string(),
                message: f[2].to_string(),
                created_at: f[3]
                    .trim()
                    .parse()
                    .map_err(|_| GitError::parse("stash list", f[3]))?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempRepo;

    #[test]
    fn live_stash_list() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "a\n", "first");
        assert!(list(&t.repo()).unwrap().is_empty());
        t.write("a.txt", "changed\n");
        t.git(["stash", "push", "-q", "-m", "my work"]);
        t.write("a.txt", "again\n");
        t.git(["stash", "push", "-q"]);
        let s = list(&t.repo()).unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].selector, "stash@{0}");
        assert!(s[0].message.starts_with("WIP on main"));
        assert_eq!(s[0].branch(), Some("main"));
        assert_eq!(s[1].message, "On main: my work");
        assert_eq!(s[1].index, 1);
    }
}
