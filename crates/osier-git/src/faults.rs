//! Fault injection: what happens when git, the repository or the filesystem misbehave.
//! Every case must surface as a typed error, never a panic.

use std::os::unix::fs::PermissionsExt;

use crate::error::GitError;
use crate::ops::stage;
use crate::repo::Repository;
use crate::status::{StatusOptions, status};
use crate::testing::TempRepo;

#[test]
fn parsers_fail_closed_on_garbage() {
    assert!(crate::status::parse(b"this is not porcelain\0").is_err());
    assert!(crate::log::parse(b"\x1f\x1f\x1fgarbage").is_err());
    assert!(crate::diff::parse("@@ -1,2 +1,2 @@ without a header\n-a\n+b\n").is_err());
    assert!(crate::blame::parse("not porcelain blame").is_err());
    assert!(crate::reflog::parse(b"no separators here").is_err());
    assert!(crate::version::GitVersion::parse("gti verison").is_err());
    // Empty output is a valid empty result where that makes sense.
    assert!(crate::status::parse(b"").unwrap().entries.is_empty());
    assert!(crate::diff::parse("").unwrap().is_empty());
}

#[test]
fn repository_removed_under_us_is_an_error_not_a_panic() {
    let t = TempRepo::new();
    t.commit_file("a.txt", "a\n", "first");
    let repo = t.repo();
    std::fs::remove_dir_all(t.path()).unwrap();
    let err = status(&repo, StatusOptions::default()).unwrap_err();
    assert!(
        matches!(
            err,
            GitError::NotARepo(_) | GitError::Failed { .. } | GitError::Io(_)
        ),
        "{err:?}"
    );
    assert!(Repository::discover(t.runner(), t.path()).is_err());
}

#[test]
fn stale_index_lock_is_reported_as_locked() {
    let t = TempRepo::new();
    t.commit_file("a.txt", "a\n", "first");
    t.write("a.txt", "b\n");
    std::fs::write(t.git_dir().join("index.lock"), "").unwrap();
    match stage::stage(&t.repo(), &["a.txt".into()]) {
        Err(GitError::Locked(_)) => {}
        other => panic!("expected Locked, got {other:?}"),
    }
    assert!(t.repo().is_locked());
    std::fs::remove_file(t.git_dir().join("index.lock")).unwrap();
    stage::stage(&t.repo(), &["a.txt".into()]).unwrap();
}

#[test]
fn permission_denied_is_a_typed_failure() {
    if unsafe_is_root() {
        return; // root ignores file modes
    }
    let t = TempRepo::new();
    t.commit_file("a.txt", "a\n", "first");
    t.write("a.txt", "b\n");
    let repo = t.repo();
    let objects = t.git_dir().join("objects");
    let original = std::fs::metadata(&objects).unwrap().permissions();
    std::fs::set_permissions(&objects, std::fs::Permissions::from_mode(0o000)).unwrap();
    let result = stage::stage(&repo, &["a.txt".into()]);
    std::fs::set_permissions(&objects, original).unwrap();
    // git calls an unreadable objects directory "not a git repository"; either typed
    // error is acceptable, a panic or a silent Ok is not.
    match result {
        Err(GitError::Failed { .. } | GitError::NotARepo(_)) => {}
        other => panic!("expected a failure, got {other:?}"),
    }
}

#[test]
fn git_that_prints_garbage_is_a_parse_error() {
    // A fake "git" that answers every command with nonsense.
    let dir = tempfile::tempdir().unwrap();
    let fake = dir.path().join("git");
    std::fs::write(&fake, "#!/bin/sh\nprintf 'lorem ipsum'\nexit 0\n").unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let t = TempRepo::new();
    t.commit_file("a.txt", "a\n", "first");
    let runner = t.runner().with_git(&fake);
    assert!(matches!(
        crate::version::GitVersion::detect(&runner),
        Err(GitError::Parse { .. })
    ));
    let repo = Repository::discover(runner, t.path());
    // Discovery either fails on the nonsense rev-parse output or yields a repo whose status
    // fails to parse; both are errors, neither is a panic.
    if let Ok(repo) = repo {
        assert!(status(&repo, StatusOptions::default()).is_err());
    }
}

fn unsafe_is_root() -> bool {
    std::fs::metadata("/root")
        .map(|m| {
            m.permissions().mode() & 0o700 == 0o700
                && std::env::var_os("USER").is_some_and(|u| u == "root")
        })
        .unwrap_or(false)
}
