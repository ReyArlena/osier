//! Creating commits.

use crate::error::{GitError, Result};
use crate::repo::Repository;

#[derive(Debug, Clone, Default)]
pub struct CommitOptions {
    pub message: String,
    pub amend: bool,
    pub signoff: bool,
    /// Skip pre-commit and commit-msg hooks. Off unless the user asks per commit.
    pub no_verify: bool,
    pub allow_empty: bool,
}

/// Commit the index. Returns the new HEAD hash. The message goes through stdin so it never
/// appears in the command log's argv.
pub fn commit(repo: &Repository, options: &CommitOptions) -> Result<String> {
    if options.message.trim().is_empty() {
        return Err(GitError::Parse {
            context: "commit",
            detail: "empty commit message".into(),
        });
    }
    let mut args = vec!["commit", "--quiet", "--file=-", "--cleanup=strip"];
    if options.amend {
        args.push("--amend");
    }
    if options.signoff {
        args.push("--signoff");
    }
    if options.no_verify {
        args.push("--no-verify");
    }
    if options.allow_empty {
        args.push("--allow-empty");
    }
    let out = repo.runner().run_unchecked(
        Some(repo.workdir()),
        args,
        Some(options.message.as_bytes()),
    )?;
    match out.record.status {
        Some(0) => {}
        Some(s) => return Err(GitError::failed(s, out.record)),
        None => return Err(GitError::timeout(0, out.record)),
    }
    repo.head_oid()?
        .ok_or_else(|| GitError::parse("commit", "no HEAD after commit"))
}

/// Subject and body of HEAD, for pre-filling an amend.
pub fn head_message(repo: &Repository) -> Result<Option<String>> {
    if repo.head_oid()?.is_none() {
        return Ok(None);
    }
    let out = repo.git(["log", "-1", "--format=%B"])?;
    Ok(Some(out.stdout_str()?.trim_end().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::{LogOptions, log};
    use crate::testing::TempRepo;

    #[test]
    fn commit_amend_and_signoff() {
        let t = TempRepo::new();
        t.write("a.txt", "a\n");
        t.git(["add", "a.txt"]);
        let repo = t.repo();
        let first = commit(
            &repo,
            &CommitOptions {
                message: "Subject\n\nBody line".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(repo.head_oid().unwrap().as_deref(), Some(first.as_str()));
        let c = &log(&repo, &LogOptions::default()).unwrap()[0];
        assert_eq!(c.subject, "Subject");
        assert_eq!(c.body, "Body line");
        assert_eq!(
            head_message(&repo).unwrap().as_deref(),
            Some("Subject\n\nBody line")
        );

        t.write("a.txt", "b\n");
        t.git(["add", "a.txt"]);
        let second = commit(
            &repo,
            &CommitOptions {
                message: "Amended".into(),
                amend: true,
                signoff: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_ne!(first, second);
        let commits = log(&repo, &LogOptions::default()).unwrap();
        assert_eq!(commits.len(), 1, "amend replaced the commit");
        assert!(
            commits[0]
                .body
                .contains("Signed-off-by: Osier Test <test@example.com>")
        );
    }

    #[test]
    fn empty_message_and_nothing_staged_fail() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "a\n", "first");
        let repo = t.repo();
        assert!(
            commit(
                &repo,
                &CommitOptions {
                    message: "  ".into(),
                    ..Default::default()
                }
            )
            .is_err()
        );
        assert!(matches!(
            commit(
                &repo,
                &CommitOptions {
                    message: "x".into(),
                    ..Default::default()
                }
            ),
            Err(GitError::Failed { status: 1, .. })
        ));
        assert!(head_message(&TempRepo::new().repo()).unwrap().is_none());
    }
}
