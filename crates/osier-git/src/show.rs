//! Details of one commit: metadata plus the list of changed files.

use crate::diff::FileStatus;
use crate::error::{GitError, Result};
use crate::log::{Commit, LogOptions};
use crate::repo::Repository;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    pub path: String,
    pub old_path: Option<String>,
    pub status: FileStatus,
    pub score: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitDetails {
    pub commit: Commit,
    pub files: Vec<ChangedFile>,
}

pub fn commit_details(repo: &Repository, rev: &str) -> Result<CommitDetails> {
    let commits = crate::log::log(
        repo,
        &LogOptions {
            revisions: vec![rev.to_string()],
            limit: Some(1),
            ..Default::default()
        },
    )?;
    let commit = commits
        .into_iter()
        .next()
        .ok_or_else(|| GitError::parse("show", format!("no commit for {rev}")))?;
    let out = repo.git([
        "diff-tree",
        "--no-commit-id",
        "--root",
        "-r",
        "-z",
        "-M",
        "--name-status",
        &commit.hash,
    ])?;
    let files = parse_name_status(&out.stdout)?;
    Ok(CommitDetails { commit, files })
}

/// Parse `--name-status -z`: `STATUS\0path\0` or `R100\0old\0new\0`.
pub fn parse_name_status(bytes: &[u8]) -> Result<Vec<ChangedFile>> {
    let mut fields = bytes.split(|b| *b == 0).filter(|f| !f.is_empty());
    let mut files = Vec::new();
    while let Some(status) = fields.next() {
        let status = std::str::from_utf8(status).map_err(|_| GitError::Utf8("name-status"))?;
        let mut chars = status.chars();
        let code = chars
            .next()
            .ok_or_else(|| GitError::parse("name-status", "empty status"))?;
        let score: Option<u8> = chars.as_str().parse().ok();
        let next = |fields: &mut dyn Iterator<Item = &[u8]>| -> Result<String> {
            let f = fields
                .next()
                .ok_or_else(|| GitError::parse("name-status", "missing path"))?;
            Ok(std::str::from_utf8(f)
                .map_err(|_| GitError::Utf8("name-status"))?
                .to_string())
        };
        let (kind, old_path, path) = match code {
            'M' => (FileStatus::Modified, None, next(&mut fields)?),
            'A' => (FileStatus::Added, None, next(&mut fields)?),
            'D' => (FileStatus::Deleted, None, next(&mut fields)?),
            'T' => (FileStatus::TypeChanged, None, next(&mut fields)?),
            'R' => {
                let old = next(&mut fields)?;
                (FileStatus::Renamed, Some(old), next(&mut fields)?)
            }
            'C' => {
                let old = next(&mut fields)?;
                (FileStatus::Copied, Some(old), next(&mut fields)?)
            }
            other => return Err(GitError::parse("name-status", format!("status {other:?}"))),
        };
        files.push(ChangedFile {
            path,
            old_path,
            status: kind,
            score,
        });
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempRepo;

    #[test]
    fn parses_name_status_fixture() {
        let f = parse_name_status(b"R100\0old.txt\0new name.txt\0A\0sub/x.txt\0D\0gone\0M\0m.rs\0")
            .unwrap();
        assert_eq!(f.len(), 4);
        assert_eq!(f[0].status, FileStatus::Renamed);
        assert_eq!(f[0].old_path.as_deref(), Some("old.txt"));
        assert_eq!(f[0].path, "new name.txt");
        assert_eq!(f[0].score, Some(100));
        assert_eq!(f[3].status, FileStatus::Modified);
    }

    #[test]
    fn live_commit_details() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "a\n", "first");
        t.write("b.txt", "b\n");
        t.git(["add", "b.txt"]);
        t.git(["mv", "a.txt", "c.txt"]);
        let hash = t.commit("second\n\nwith body");
        let d = commit_details(&t.repo(), &hash).unwrap();
        assert_eq!(d.commit.subject, "second");
        assert_eq!(d.commit.body, "with body");
        assert_eq!(d.files.len(), 2);
        assert!(
            d.files
                .iter()
                .any(|f| f.status == FileStatus::Added && f.path == "b.txt")
        );
        assert!(
            d.files
                .iter()
                .any(|f| f.status == FileStatus::Renamed && f.path == "c.txt")
        );

        let root = commit_details(&t.repo(), "HEAD~1").unwrap();
        assert_eq!(root.files.len(), 1);
        assert_eq!(root.files[0].status, FileStatus::Added);
    }
}
