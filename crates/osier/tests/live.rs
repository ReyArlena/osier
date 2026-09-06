//! Live tests against GitHub. Ignored unless `OSIER_LIVE_TESTS=1`; they need the gh accounts
//! on the developer's machine and network access. Tokens never leave the helper process.
//!
//! Run: `OSIER_LIVE_TESTS=1 cargo test -p osier --test live -- --ignored`

use std::path::Path;
use std::sync::Arc;

use osier_git::{GitError, GitRunner, MemorySink};

const TESTING_REPO: &str = "https://github.com/ReyArlena/oser-testing-repository.git";

fn live() -> bool {
    std::env::var_os("OSIER_LIVE_TESTS").is_some()
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
    args.extend(["ls-remote".into(), "--heads".into(), TESTING_REPO.into()]);
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
    ls_remote_as("ReyArlena").expect("ReyArlena owns the testing repository");
}

#[test]
#[ignore = "needs OSIER_LIVE_TESTS=1, gh accounts and network"]
fn other_account_is_refused_so_account_choice_matters() {
    if !live() {
        return;
    }
    match ls_remote_as("other-account") {
        Err(GitError::AuthRequired(_)) => {}
        Err(GitError::Failed { record, .. }) if record.stderr.contains("not found") || record.stderr.contains("403") => {}
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
    assert!(logins.contains(&"ReyArlena"), "{logins:?}");
    assert!(logins.contains(&"other-account"), "{logins:?}");
}
