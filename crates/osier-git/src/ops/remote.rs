//! Fetch, pull, push and remote management. Network operations stream progress and can be
//! cancelled.

use std::path::{Path, PathBuf};

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
        self.with_env(repo.runner())
    }

    fn with_env(&self, base: &GitRunner) -> GitRunner {
        let mut runner = base.clone();
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

#[derive(Debug, Clone)]
pub struct CloneOptions {
    pub url: String,
    /// The new working directory. It must not exist yet, or be an empty directory.
    pub target: PathBuf,
    pub recurse_submodules: bool,
}

/// `git clone` into a new directory. Refuses a target that already has files in it, so a
/// clone can never mix into or replace existing work. git removes what it created when the
/// clone fails or is cancelled.
pub fn clone(
    runner: &GitRunner,
    options: &CloneOptions,
    ctx: &RemoteContext,
    on_progress: impl FnMut(Progress) + Send + 'static,
) -> Result<String> {
    if options.url.trim().is_empty() {
        return Err(GitError::Refused("No repository address to clone.".into()));
    }
    // An address starting with `-` would read as an option (the `--` below also guards it).
    if options.url.trim_start().starts_with('-') {
        return Err(GitError::Refused(format!(
            "{} is not a repository address.",
            options.url
        )));
    }
    check_empty_target(&options.target)?;
    let mut args = ctx.config_args();
    args.extend(["clone".to_string(), "--progress".to_string()]);
    if options.recurse_submodules {
        args.push("--recurse-submodules".into());
    }
    args.push("--".into());
    args.push(options.url.trim().to_string());
    args.push(options.target.to_string_lossy().into_owned());
    let mut on_progress = on_progress;
    let out = ctx
        .with_env(runner)
        .run_streaming(None, args, ctx.cancel.clone(), move |line| {
            if let Some(p) = Progress::parse(line) {
                on_progress(p);
            }
        })?;
    Ok(out.record.stderr)
}

/// A clone or new repository may only go where nothing is yet: a missing path or an empty
/// directory.
pub fn check_empty_target(target: &Path) -> Result<()> {
    if !target.is_absolute() {
        return Err(GitError::Refused(format!(
            "{} is not an absolute path.",
            target.display()
        )));
    }
    match std::fs::read_dir(target) {
        Ok(mut entries) => {
            if entries.next().is_some() {
                return Err(GitError::Refused(format!(
                    "{} already exists and is not empty.",
                    target.display()
                )));
            }
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotADirectory => Err(GitError::Refused(format!(
            "{} already exists and is a file.",
            target.display()
        ))),
        Err(e) => Err(GitError::Io(e)),
    }
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
    /// `--autostash`: set uncommitted changes aside and put them back afterwards.
    pub autostash: bool,
}

impl Default for PullOptions {
    fn default() -> Self {
        Self {
            strategy: PullStrategy::FfOnly,
            remote: None,
            branch: None,
            prune: false,
            autostash: false,
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
    if options.autostash {
        args.push("--autostash".into());
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
    /// Delete this ref on the remote instead of pushing (`push <remote> --delete <ref>`),
    /// e.g. `refs/tags/v1`. Uses `remote` or `origin`; the other flags are ignored.
    pub delete: Option<String>,
}

pub fn push(
    repo: &Repository,
    options: &PushOptions,
    ctx: &RemoteContext,
    on_progress: impl FnMut(Progress) + Send + 'static,
) -> Result<String> {
    let mut args: Vec<String> = vec!["push".into(), "--progress".into()];
    if let Some(refname) = &options.delete {
        args.push(options.remote.clone().unwrap_or_else(|| "origin".into()));
        args.push("--delete".into());
        args.push(refname.clone());
        return run(repo, ctx, args, on_progress);
    }
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

    #[test]
    fn push_delete_removes_a_remote_tag() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "a\n", "first");
        let bare = t.add_bare_origin();
        t.git(["tag", "v1"]);
        t.git(["push", "-q", "origin", "v1"]);
        let tags_on_remote = || {
            let out = std::process::Command::new("git")
                .args(["-C", bare.to_string_lossy().as_ref(), "tag"])
                .output()
                .unwrap();
            String::from_utf8(out.stdout).unwrap()
        };
        assert_eq!(tags_on_remote().trim(), "v1");
        push(
            &t.repo(),
            &PushOptions {
                remote: Some("origin".into()),
                delete: Some("refs/tags/v1".into()),
                ..Default::default()
            },
            &RemoteContext::default(),
            |_| {},
        )
        .unwrap();
        assert_eq!(tags_on_remote().trim(), "");
        // git treats deleting a missing ref as success (with a warning); an unknown remote
        // is the failure path.
        assert!(
            push(
                &t.repo(),
                &PushOptions {
                    remote: Some("nope".into()),
                    delete: Some("refs/tags/v1".into()),
                    ..Default::default()
                },
                &RemoteContext::default(),
                |_| {},
            )
            .is_err()
        );
    }

    #[test]
    fn clone_into_a_new_directory_and_refuse_a_full_one() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "a\n", "first");
        let bare = t.bare_clone();
        let dest = tempfile::tempdir().unwrap();
        let target = dest.path().join("copy");
        let options = CloneOptions {
            url: bare.to_string_lossy().into_owned(),
            target: target.clone(),
            recurse_submodules: true,
        };
        clone(
            &t.runner(),
            &options,
            &RemoteContext::default(),
            no_progress,
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(target.join("a.txt")).unwrap(),
            "a\n"
        );

        // Again into the same, now full, directory: refused before git runs.
        let err = clone(
            &t.runner(),
            &options,
            &RemoteContext::default(),
            no_progress,
        )
        .unwrap_err();
        assert!(matches!(err, GitError::Refused(m) if m.contains("not empty")));

        // An empty existing directory is fine.
        let empty = dest.path().join("empty");
        std::fs::create_dir(&empty).unwrap();
        let options = CloneOptions {
            target: empty.clone(),
            recurse_submodules: false,
            ..options
        };
        clone(
            &t.runner(),
            &options,
            &RemoteContext::default(),
            no_progress,
        )
        .unwrap();
        assert!(empty.join("a.txt").exists());
    }

    #[test]
    fn a_failed_clone_leaves_nothing_behind() {
        let t = TempRepo::new();
        let dest = tempfile::tempdir().unwrap();
        let target = dest.path().join("missing");
        let options = CloneOptions {
            url: dest
                .path()
                .join("no-such-repo")
                .to_string_lossy()
                .into_owned(),
            target: target.clone(),
            recurse_submodules: false,
        };
        assert!(
            clone(
                &t.runner(),
                &options,
                &RemoteContext::default(),
                no_progress
            )
            .is_err()
        );
        assert!(!target.exists());
        let options = CloneOptions {
            url: "--upload-pack=touch /tmp/x".into(),
            ..options
        };
        let err = clone(
            &t.runner(),
            &options,
            &RemoteContext::default(),
            no_progress,
        )
        .unwrap_err();
        assert!(matches!(err, GitError::Refused(_)));
        let options = CloneOptions {
            url: "/tmp/repo".into(),
            target: "relative/dir".into(),
            recurse_submodules: false,
        };
        assert!(matches!(
            clone(
                &t.runner(),
                &options,
                &RemoteContext::default(),
                no_progress
            ),
            Err(GitError::Refused(_))
        ));
    }
}
