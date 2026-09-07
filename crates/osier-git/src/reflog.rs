//! HEAD reflog, the recovery view.

use crate::error::{GitError, Result};
use crate::repo::Repository;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflogEntry {
    pub hash: String,
    /// `HEAD@{0}` style selector.
    pub selector: String,
    /// `commit: subject`, `checkout: moving from a to b`, `reset: moving to HEAD~1`, ...
    pub message: String,
    pub at: i64,
}

impl ReflogEntry {
    /// The action word before the colon.
    pub fn action(&self) -> &str {
        self.message.split(':').next().unwrap_or("").trim()
    }
}

pub fn list(repo: &Repository, limit: usize) -> Result<Vec<ReflogEntry>> {
    let n = format!("-n{limit}");
    let out = repo.runner().run_allowing(
        Some(repo.workdir()),
        [
            "reflog",
            "show",
            "-z",
            "--format=%H%x1f%gd%x1f%gs%x1f%ct",
            &n,
        ],
        &[0, 128],
    )?;
    if out.record.status == Some(128) {
        // Unborn branch: no reflog yet.
        return Ok(Vec::new());
    }
    parse(&out.stdout)
}

pub fn parse(bytes: &[u8]) -> Result<Vec<ReflogEntry>> {
    bytes
        .split(|b| *b == 0)
        .filter(|r| !r.is_empty())
        .map(|rec| {
            let text = std::str::from_utf8(rec).map_err(|_| GitError::Utf8("reflog"))?;
            let f: Vec<&str> = text.splitn(4, '\x1f').collect();
            if f.len() != 4 {
                return Err(GitError::parse("reflog", text.to_string()));
            }
            Ok(ReflogEntry {
                hash: f[0].to_string(),
                selector: f[1].to_string(),
                message: f[2].to_string(),
                at: f[3]
                    .trim()
                    .parse()
                    .map_err(|_| GitError::parse("reflog", f[3]))?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempRepo;

    #[test]
    fn lists_head_moves_newest_first() {
        let t = TempRepo::new();
        assert!(list(&t.repo(), 10).unwrap().is_empty());
        t.commit_file("a.txt", "a\n", "first");
        t.commit_file("a.txt", "b\n", "second");
        t.git(["reset", "-q", "--hard", "HEAD~1"]);
        let r = list(&t.repo(), 10).unwrap();
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].selector, "HEAD@{0}");
        assert_eq!(r[0].action(), "reset");
        assert_eq!(r[1].message, "commit: second");
        assert_eq!(r[2].message, "commit (initial): first");
        assert_eq!(list(&t.repo(), 1).unwrap().len(), 1);
    }
}
