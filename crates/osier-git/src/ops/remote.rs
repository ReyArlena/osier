//! Fetch, pull, push and remote management. Network operations stream progress and can be
//! cancelled.

use crate::error::{GitError, Result};
use crate::repo::Repository;
use crate::runner::{CancelToken, GitRunner, Progress};

/// Per-call additions for authentication: `-c key=value` pairs and environment variables.
/// Built by the auth layer; the git layer only forwards them.
#[derive(Debug, Clone, Default)]
pub struct RemoteContext {
    pub config: Vec<(String, String)>,
    pub env: Vec<(String, String)>,
    pub cancel: Option<CancelToken>,
}

impl RemoteContext {
    fn runner(&self, repo: &Repository) -> GitRunner {
        let mut runner = repo.runner().clone();
        for (k, v) in &self.env {
            runner = runner.with_env(k.clone(), v.clone());
        }
        runner
    }

    fn config_args(&self) -> Vec<String> {
        self.config
            .iter()
            .flat_map(|(k, v)| ["-c".to_string(), format!("{k}={v}")])
            .collect()
    }
}

fn run(
    repo: &Repository,
    ctx: &RemoteContext,
    args: Vec<String>,
    on_progress: impl FnMut(Progress) + Send + 'static,
) -> Result<String> {
    let mut full = ctx.config_args();
    full.extend(args);
    let mut on_progress = on_progress;
    let out = ctx.runner(repo).run_streaming(
        Some(repo.workdir()),
        full,
        ctx.cancel.clone(),
        move |line| {
            if let Some(p) = Progress::parse(line) {
                on_progress(p);
            }
        },
    )?;
    Ok(out.record.stderr)
}

#[derive(Debug, Clone, Default)]
pub struct FetchOptions {
    /// `None` fetches every remote (`--all`).
    pub remote: Option<String>,
    pub prune: bool,
    pub tags: bool,
}

pub fn fetch(
    repo: &Repository,
    options: &FetchOptions,
    ctx: &RemoteContext,
    on_progress: impl FnMut(Progress) + Send + 'static,
) -> Result<String> {
    let mut args: Vec<String> = vec!["fetch".into(), "--progress".into()];
    if options.prune {
        args.push("--prune".into());
    }
    if options.tags {
        args.push("--tags".into());
    }
    match &options.remote {
        Some(r) => args.push(r.clone()),
        None => args.push("--all".into()),
    }
    run(repo, ctx, args, on_progress)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PullStrategy {
    /// Fail unless the update is a fast-forward (the default).
    FfOnly,
    Merge,
    Rebase,
}

#[derive(Debug, Clone)]
pub struct PullOptions {
    pub strategy: PullStrategy,
    /// Remote and branch; `None` uses the configured upstream.
    pub remote: Option<String>,
    pub branch: Option<String>,
    pub prune: bool,
}

impl Default for PullOptions {
    fn default() -> Self {
        Self {
            strategy: PullStrategy::FfOnly,
            remote: None,
            branch: None,
            prune: false,
        }
    }
}

pub fn pull(
    repo: &Repository,
    options: &PullOptions,
    ctx: &RemoteContext,
    on_progress: impl FnMut(Progress) + Send + 'static,
) -> Result<String> {
    let mut args: Vec<String> = vec!["pull".into(), "--progress".into(), "--no-edit".into()];
    args.push(match options.strategy {
        PullStrategy::FfOnly => "--ff-only".into(),
        PullStrategy::Merge => "--no-rebase".into(),
        PullStrategy::Rebase => "--rebase".into(),
    });
    if options.prune {
        args.push("--prune".into());
    }
    if let Some(r) = &options.remote {
        args.push(r.clone());
        if let Some(b) = &options.branch {
            args.push(b.clone());
        }
    }
    run(repo, ctx, args, on_progress)
}

#[derive(Debug, Clone, Default)]
pub struct PushOptions {
    pub remote: Option<String>,
    /// Local branch to push; `None` pushes the current branch.
    pub branch: Option<String>,
    pub set_upstream: bool,
    /// `--force-with-lease`. Never plain `--force`.
    pub force_with_lease: bool,
    pub tags: bool,
}

pub fn push(
    repo: &Repository,
    options: &PushOptions,
    ctx: &RemoteContext,
    on_progress: impl FnMut(Progress) + Send + 'static,
) -> Result<String> {
    let mut args: Vec<String> = vec!["push".into(), "--progress".into()];
    if options.set_upstream {
        args.push("--set-upstream".into());
    }
    if options.force_with_lease {
        args.push("--force-with-lease".into());
    }
    if options.tags {
        args.push("--tags".into());
    }
    if let Some(r) = &options.remote {
        args.push(r.clone());
        if let Some(b) = &options.branch {
            args.push(b.clone());
        } else if options.set_upstream {
            // Explicit current branch so `--set-upstream` has a name to record.
            let head = repo.git(["branch", "--show-current"])?;
            let name = head.stdout_str()?.trim().to_string();
            if name.is_empty() {
                return Err(GitError::parse(
                    "push",
                    "detached HEAD has no branch to push",
                ));
            }
            args.push(name);
        }
    }
    run(repo, ctx, args, on_progress)
}

// ----- remote management -----

pub fn add(repo: &Repository, name: &str, url: &str) -> Result<()> {
    repo.git(["remote", "add", name, url])?;
    Ok(())
}

pub fn remove(repo: &Repository, name: &str) -> Result<()> {
    repo.git(["remote", "remove", name])?;
    Ok(())
}

pub fn rename(repo: &Repository, old: &str, new: &str) -> Result<()> {
    repo.git(["remote", "rename", old, new])?;
    Ok(())
}

pub fn set_url(repo: &Repository, name: &str, url: &str, push: bool) -> Result<()> {
    if push {
        repo.git(["remote", "set-url", "--push", name, url])?;
    } else {
        repo.git(["remote", "set-url", name, url])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::refs::branches;
    use crate::testing::TempRepo;

    fn no_progress(_: Progress) {}

    #[test]
    fn fetch_pull_ff_and_prune() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "a\n", "first");
        let bare = t.add_bare_origin();
        t.git(["push", "-q", "-u", "origin", "main"]);
        // A second clone adds a commit and a branch, then deletes the branch.
        let other = TempRepo::new();
        other.git(["remote", "add", "origin", bare.to_string_lossy().as_ref()]);
        other.git(["fetch", "-q", "origin"]);
        other.git(["checkout", "-q", "-b", "main", "origin/main"]);
        other.commit_file("b.txt", "b\n", "second");
        other.git(["push", "-q", "origin", "main"]);
        other.git(["push", "-q", "origin", "main:temp"]);

        let repo = t.repo();
        let ctx = RemoteContext::default();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s2 = seen.clone();
        fetch(&repo, &FetchOptions::default(), &ctx, move |p| {
            s2.lock().unwrap().push(p)
        })
        .unwrap();
        let names: Vec<String> = branches(&repo)
            .unwrap()
            .into_iter()
            .map(|b| b.name)
            .collect();
        assert!(names.contains(&"origin/temp".to_string()));
        assert!(
            branches(&repo)
                .unwrap()
                .iter()
                .any(|b| b.name == "main" && b.behind == 1)
        );

        pull(&repo, &PullOptions::default(), &ctx, no_progress).unwrap();
        assert_eq!(t.read("b.txt"), "b\n");

        other.git(["push", "-q", "origin", "--delete", "temp"]);
        fetch(
            &repo,
            &FetchOptions {
                prune: true,
                ..Default::default()
            },
            &ctx,
            no_progress,
        )
        .unwrap();
        let names: Vec<String> = branches(&repo)
            .unwrap()
            .into_iter()
            .map(|b| b.name)
            .collect();
        assert!(!names.contains(&"origin/temp".to_string()), "pruned");
    }

    #[test]
    fn pull_ff_only_refuses_divergence_then_merge_and_rebase_work() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "a\n", "first");
        let bare = t.add_bare_origin();
        t.git(["push", "-q", "-u", "origin", "main"]);
        let other = TempRepo::new();
        other.git(["remote", "add", "origin", bare.to_string_lossy().as_ref()]);
        other.git(["fetch", "-q", "origin"]);
        other.git(["checkout", "-q", "-b", "main", "origin/main"]);
        other.commit_file("remote.txt", "r\n", "remote work");
        other.git(["push", "-q", "origin", "main"]);
        t.commit_file("local.txt", "l\n", "local work");

        let repo = t.repo();
        let ctx = RemoteContext::default();
        let err = pull(&repo, &PullOptions::default(), &ctx, no_progress);
        assert!(matches!(err, Err(GitError::NonFastForward(_))), "{err:?}");

        pull(
            &repo,
            &PullOptions {
                strategy: PullStrategy::Rebase,
                ..Default::default()
            },
            &ctx,
            no_progress,
        )
        .unwrap();
        assert!(t.path().join("remote.txt").exists());
        assert_eq!(
            t.git(["rev-list", "--count", "HEAD"]).trim(),
            "3",
            "rebased, no merge commit"
        );

        other.commit_file("more.txt", "m\n", "more remote work");
        other.git(["push", "-q", "origin", "main"]);
        t.commit_file("local2.txt", "l2\n", "more local work");
        pull(
            &repo,
            &PullOptions {
                strategy: PullStrategy::Merge,
                ..Default::default()
            },
            &ctx,
            no_progress,
        )
        .unwrap();
        assert_eq!(
            t.git(["rev-list", "--merges", "--count", "HEAD"]).trim(),
            "1"
        );
    }

    #[test]
    fn push_set_upstream_non_ff_and_force_with_lease() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "a\n", "first");
        let bare = t.add_bare_origin();
        let repo = t.repo();
        let ctx = RemoteContext::default();
        push(
            &repo,
            &PushOptions {
                remote: Some("origin".into()),
                set_upstream: true,
                ..Default::default()
            },
            &ctx,
            no_progress,
        )
        .unwrap();
        let main = branches(&repo)
            .unwrap()
            .into_iter()
            .find(|b| b.name == "main")
            .unwrap();
        assert_eq!(main.upstream.as_deref(), Some("origin/main"));

        let other = TempRepo::new();
        other.git(["remote", "add", "origin", bare.to_string_lossy().as_ref()]);
        other.git(["fetch", "-q", "origin"]);
        other.git(["checkout", "-q", "-b", "main", "origin/main"]);
        other.commit_file("theirs.txt", "t\n", "theirs");
        other.git(["push", "-q", "origin", "main"]);

        t.commit_file("mine.txt", "m\n", "mine");
        let err = push(&repo, &PushOptions::default(), &ctx, no_progress);
        assert!(matches!(err, Err(GitError::NonFastForward(_))), "{err:?}");

        // Stale lease (we never fetched) is also rejected; after a fetch it goes through.
        let err = push(
            &repo,
            &PushOptions {
                force_with_lease: true,
                ..Default::default()
            },
            &ctx,
            no_progress,
        );
        assert!(matches!(err, Err(GitError::NonFastForward(_))), "{err:?}");
        fetch(&repo, &FetchOptions::default(), &ctx, no_progress).unwrap();
        push(
            &repo,
            &PushOptions {
                force_with_lease: true,
                ..Default::default()
            },
            &ctx,
            no_progress,
        )
        .unwrap();
        assert_eq!(
            other
                .git(["ls-remote", "origin", "main"])
                .split_whitespace()
                .next()
                .unwrap(),
            t.head()
        );
    }

    #[test]
    fn missing_upstream_is_typed() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "a\n", "first");
        t.add_bare_origin();
        let repo = t.repo();
        let ctx = RemoteContext::default();
        assert!(matches!(
            push(&repo, &PushOptions::default(), &ctx, no_progress),
            Err(GitError::NoUpstream(_))
        ));
        assert!(matches!(
            pull(&repo, &PullOptions::default(), &ctx, no_progress),
            Err(GitError::NoUpstream(_))
        ));
    }

    #[test]
    fn remote_management() {
        let t = TempRepo::new();
        let repo = t.repo();
        add(&repo, "up", "https://example.com/a.git").unwrap();
        set_url(&repo, "up", "git@example.com:a.git", true).unwrap();
        rename(&repo, "up", "upstream").unwrap();
        let remotes = crate::refs::remotes(&repo).unwrap();
        assert_eq!(remotes.len(), 1);
        assert_eq!(remotes[0].name, "upstream");
        assert_eq!(
            remotes[0].push_url.as_deref(),
            Some("git@example.com:a.git")
        );
        remove(&repo, "upstream").unwrap();
        assert!(crate::refs::remotes(&repo).unwrap().is_empty());
    }

    #[test]
    fn unreachable_remote_is_a_network_error() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "a\n", "first");
        let repo = t.repo();
        add(&repo, "origin", "/nonexistent/path/to/repo.git").unwrap();
        let ctx = RemoteContext::default();
        let err = fetch(
            &repo,
            &FetchOptions {
                remote: Some("origin".into()),
                ..Default::default()
            },
            &ctx,
            no_progress,
        );
        assert!(
            matches!(
                err,
                Err(GitError::Network(_)) | Err(GitError::Failed { .. })
            ),
            "{err:?}"
        );
    }

    #[test]
    fn context_config_is_forwarded() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "a\n", "first");
        t.add_bare_origin();
        let repo = t.repo();
        let ctx = RemoteContext {
            config: vec![("osier.marker".into(), "yes".into())],
            ..Default::default()
        };
        fetch(&repo, &FetchOptions::default(), &ctx, no_progress).unwrap();
        let last = t.sink().records().last().unwrap().clone();
        assert!(
            last.argv
                .windows(2)
                .any(|w| w[0] == "-c" && w[1] == "osier.marker=yes"),
            "{:?}",
            last.argv
        );
    }
}
