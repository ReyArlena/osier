//! Live tests against GitHub. Ignored unless `OSIER_LIVE_TESTS=1`; they need gh accounts on
//! the developer's machine and network access. Tokens never leave the helper process.
//!
//! Set up in the environment:
//! - `OSIER_LIVE_REPO`: https URL of a private repository the owner account can push to
//!   (the push test creates and deletes an `osier-live-<secs>` branch there)
//! - `OSIER_LIVE_OWNER`: the gh login that owns it
//! - `OSIER_LIVE_OTHER`: optional, a second gh login without access; the refusal test is
//!   skipped without it
//!
//! Run: `OSIER_LIVE_TESTS=1 OSIER_LIVE_REPO=... OSIER_LIVE_OWNER=... cargo test -p osier --test live -- --ignored`

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::Path;

use osier_git::{GitError, GitRunner, MemorySink};

fn live() -> bool {
    std::env::var_os("OSIER_LIVE_TESTS").is_some()
}

fn setting(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// The private testing repository (`OSIER_LIVE_REPO`).
fn testing_repo() -> String {
    setting("OSIER_LIVE_REPO").expect("set OSIER_LIVE_REPO to the testing repository's https URL")
}

/// The gh login that owns the testing repository (`OSIER_LIVE_OWNER`).
fn owner() -> String {
    setting("OSIER_LIVE_OWNER").expect("set OSIER_LIVE_OWNER to the gh login that owns it")
}

/// A second gh login without access (`OSIER_LIVE_OTHER`), if one is configured.
fn other_account() -> Option<String> {
    setting("OSIER_LIVE_OTHER")
}

/// `git ls-remote` with Osier as the only credential helper for `account`.
fn ls_remote_as(account: &str) -> Result<(), GitError> {
    let exe = Path::new(env!("CARGO_BIN_EXE_osier"));
    let sink = MemorySink::new();
    let runner = GitRunner::new(sink.clone());
    let mut args: Vec<String> = Vec::new();
    for (k, v) in osier_auth::credential_config(exe, account, "github.com") {
        args.push("-c".into());
        args.push(format!("{k}={v}"));
    }
    args.extend(["ls-remote".into(), "--heads".into(), testing_repo()]);
    let result = runner.run(None, &args).map(|_| ());
    // The command log must never contain a token.
    for rec in sink.records() {
        let joined = rec.argv.join(" ");
        assert!(!joined.contains("gho_"), "token leaked into argv: {joined}");
        assert!(!rec.stderr.contains("gho_"), "token leaked into stderr");
    }
    result
}

#[test]
#[ignore = "needs OSIER_LIVE_TESTS=1, gh accounts and network"]
fn owner_account_can_read_the_private_testing_repo() {
    if !live() {
        return;
    }
    let owner = owner();
    ls_remote_as(&owner)
        .unwrap_or_else(|e| panic!("{owner} should read the testing repository: {e}"));
}

#[test]
#[ignore = "needs OSIER_LIVE_TESTS=1, gh accounts and network"]
fn other_account_is_refused_so_account_choice_matters() {
    if !live() {
        return;
    }
    let Some(other) = other_account() else {
        eprintln!("skipped: set OSIER_LIVE_OTHER to a gh login without access to the repository");
        return;
    };
    match ls_remote_as(&other) {
        Err(GitError::AuthRequired(_)) => {}
        Err(GitError::Failed { record, .. })
            if record.stderr.contains("not found") || record.stderr.contains("403") => {}
        other => panic!("expected an access failure for the non-owner account, got {other:?}"),
    }
}

#[test]
#[ignore = "needs OSIER_LIVE_TESTS=1, gh accounts and network"]
fn gh_accounts_are_discovered() {
    if !live() {
        return;
    }
    let accounts = osier_auth::gh::accounts().expect("gh auth status");
    let logins: Vec<&str> = accounts.iter().map(|a| a.login.as_str()).collect();
    assert!(logins.contains(&owner().as_str()), "{logins:?}");
    if let Some(other) = other_account() {
        assert!(logins.contains(&other.as_str()), "{logins:?}");
    }
}

/// A runner whose every command carries Osier's credential helper for `account`, the way
/// the app configures remote operations.
fn context_for(account: &str) -> osier_git::ops::remote::RemoteContext {
    let exe = Path::new(env!("CARGO_BIN_EXE_osier"));
    osier_git::ops::remote::RemoteContext {
        config: osier_auth::credential_config(exe, account, "github.com"),
        env: Vec::new(),
        cancel: None,
    }
}

fn config_args(ctx: &osier_git::ops::remote::RemoteContext) -> Vec<String> {
    let mut args = Vec::new();
    for (k, v) in &ctx.config {
        args.push("-c".to_string());
        args.push(format!("{k}={v}"));
    }
    args
}

/// Clone, branch, push, hit a non-fast-forward, force-with-lease, and delete the branch
/// again. Everything happens on a branch named `osier-live-<unix seconds>` that this test
/// removes at the end.
#[test]
#[ignore = "needs OSIER_LIVE_TESTS=1, gh accounts and network; pushes to the testing repo"]
fn push_round_trip_on_the_testing_repo_as_owner() {
    use osier_git::ops::remote::{PushOptions, push};
    if !live() {
        return;
    }
    let ctx = context_for(&owner());
    let sink = MemorySink::new();
    let runner = GitRunner::new(sink.clone());
    let dir = tempfile::tempdir().expect("tempdir");
    let dest = dir.path().join("clone");
    let mut args = config_args(&ctx);
    args.extend([
        "clone".to_string(),
        "-q".into(),
        testing_repo(),
        dest.to_string_lossy().into_owned(),
    ]);
    runner.run(None, &args).expect("clone as owner");
    let repo = osier_git::Repository::discover(runner.clone(), &dest).expect("repo");
    repo.git(["config", "user.name", "Osier live test"])
        .unwrap();
    repo.git(["config", "user.email", "osier-live-test@example.invalid"])
        .unwrap();

    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let branch = format!("osier-live-{secs}");
    osier_git::ops::branch::create(&repo, &branch, None, true).expect("create branch");
    std::fs::write(dest.join("osier-live.txt"), "hello from the live test\n").unwrap();
    osier_git::ops::stage::stage(&repo, &["osier-live.txt".into()]).unwrap();
    osier_git::ops::commit::commit(
        &repo,
        &osier_git::ops::commit::CommitOptions {
            message: "osier live test".into(),
            amend: false,
            signoff: false,
            no_verify: false,
            allow_empty: false,
        },
    )
    .expect("commit");

    let cleanup = |branch: &str| {
        let mut args = config_args(&ctx);
        args.extend([
            "push".to_string(),
            "origin".into(),
            "--delete".into(),
            branch.to_string(),
        ]);
        let _ = runner.run(Some(&dest), &args);
    };

    // 1. First push with upstream.
    let result = push(
        &repo,
        &PushOptions {
            remote: Some("origin".into()),
            branch: Some(branch.clone()),
            set_upstream: true,
            force_with_lease: false,
            tags: false,
            delete: None,
        },
        &ctx,
        |_| {},
    );
    if let Err(e) = &result {
        cleanup(&branch);
        panic!("first push failed: {e:?}");
    }
    let heads = {
        let mut a = config_args(&ctx);
        a.extend([
            "ls-remote".to_string(),
            "--heads".into(),
            "origin".into(),
            branch.clone(),
        ]);
        String::from_utf8(runner.run(Some(&dest), &a).unwrap().stdout).unwrap()
    };
    assert!(heads.contains(&format!("refs/heads/{branch}")), "{heads}");

    // 2. Rewrite the tip locally: a plain push must now be refused as non-fast-forward.
    repo.git(["commit", "--amend", "-m", "osier live test (amended)"])
        .unwrap();
    let refused = push(
        &repo,
        &PushOptions {
            remote: Some("origin".into()),
            branch: Some(branch.clone()),
            set_upstream: false,
            force_with_lease: false,
            tags: false,
            delete: None,
        },
        &ctx,
        |_| {},
    );
    match refused {
        Err(GitError::NonFastForward(_)) => {}
        other => {
            cleanup(&branch);
            panic!("expected NonFastForward, got {other:?}");
        }
    }

    // 3. force-with-lease is the sanctioned way through.
    let forced = push(
        &repo,
        &PushOptions {
            remote: Some("origin".into()),
            branch: Some(branch.clone()),
            set_upstream: false,
            force_with_lease: true,
            tags: false,
            delete: None,
        },
        &ctx,
        |_| {},
    );
    cleanup(&branch);
    forced.expect("force-with-lease push");

    // 4. Gone again, and no token anywhere in the command log.
    let mut a = config_args(&ctx);
    a.extend([
        "ls-remote".to_string(),
        "--heads".into(),
        "origin".into(),
        branch.clone(),
    ]);
    let after = String::from_utf8(runner.run(Some(&dest), &a).unwrap().stdout).unwrap();
    assert!(
        !after.contains(&branch),
        "branch still on the remote: {after}"
    );
    for rec in sink.records() {
        assert!(!rec.argv.join(" ").contains("gho_"), "token in argv");
        assert!(!rec.stderr.contains("gho_"), "token in stderr");
    }
}
