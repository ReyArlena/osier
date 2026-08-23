//! Throwaway repositories for tests.
//!
//! Every [`TempRepo`] is fully isolated from the user's git configuration (no global or
//! system config, fixed identity and dates), so tests behave the same on every machine.

// Test support: a failing setup step should panic loudly.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tempfile::TempDir;

use crate::repo::Repository;
use crate::runner::{GitRunner, MemorySink};

/// Temporary repository deleted on drop.
#[derive(Debug)]
pub struct TempRepo {
    dir: TempDir,
    runner: GitRunner,
    sink: Arc<MemorySink>,
    counter: std::cell::Cell<u32>,
}

impl Default for TempRepo {
    fn default() -> Self {
        Self::new()
    }
}

impl TempRepo {
    /// Create an empty repository with `main` as the initial branch.
    pub fn new() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("osier-test-")
            .tempdir()
            .expect("tempdir");
        let sink = MemorySink::new();
        let runner = isolated_runner(sink.clone());
        runner
            .run(Some(dir.path()), ["init", "-q", "-b", "main"])
            .expect("git init");
        let t = Self {
            dir,
            runner,
            sink,
            counter: std::cell::Cell::new(0),
        };
        t.git(["config", "user.name", "Osier Test"]);
        t.git(["config", "user.email", "test@example.com"]);
        t.git(["config", "commit.gpgsign", "false"]);
        t
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    pub fn git_dir(&self) -> PathBuf {
        self.dir.path().join(".git")
    }

    /// Runner wired to this repo's isolated environment.
    pub fn runner(&self) -> GitRunner {
        self.runner.clone()
    }

    pub fn sink(&self) -> Arc<MemorySink> {
        self.sink.clone()
    }

    pub fn repo(&self) -> Repository {
        Repository::discover(self.runner(), self.path()).expect("discover temp repo")
    }

    /// Run git in the repo, panicking on failure (tests only).
    pub fn git<I, S>(&self, args: I) -> String
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let out = self
            .runner
            .run(Some(self.path()), args)
            .unwrap_or_else(|e| panic!("git failed: {e}\n{:?}", e.record()));
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// Run git and return whether it succeeded.
    pub fn try_git<I, S>(&self, args: I) -> crate::Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let out = self.runner.run(Some(self.path()), args)?;
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    pub fn write(&self, rel: &str, content: &str) {
        let p = self.path().join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(p, content).expect("write");
    }

    pub fn remove(&self, rel: &str) {
        std::fs::remove_file(self.path().join(rel)).expect("remove");
    }

    pub fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.path().join(rel)).expect("read")
    }

    /// Write, add and commit one file. Returns the commit hash.
    pub fn commit_file(&self, rel: &str, content: &str, message: &str) -> String {
        self.write(rel, content);
        self.git(["add", "--", rel]);
        self.commit(message)
    }

    /// Commit whatever is staged with deterministic, increasing dates.
    pub fn commit(&self, message: &str) -> String {
        let n = self.counter.get() + 1;
        self.counter.set(n);
        let date = format!("2024-01-{:02}T12:00:00+00:00", (n % 28) + 1);
        let runner = self
            .runner
            .clone()
            .with_env("GIT_AUTHOR_DATE", &date)
            .with_env("GIT_COMMITTER_DATE", &date);
        runner
            .run(Some(self.path()), ["commit", "-q", "-m", message])
            .expect("commit");
        self.git(["rev-parse", "HEAD"]).trim().to_string()
    }

    pub fn branch(&self, name: &str) {
        self.git(["branch", name]);
    }

    pub fn checkout(&self, name: &str) {
        self.git(["checkout", "-q", name]);
    }

    pub fn checkout_new(&self, name: &str) {
        self.git(["checkout", "-q", "-b", name]);
    }

    pub fn head(&self) -> String {
        self.git(["rev-parse", "HEAD"]).trim().to_string()
    }

    /// Create a bare clone next to the repo and add it as `origin`.
    pub fn add_bare_origin(&self) -> PathBuf {
        let bare = self.bare_clone();
        self.git(["remote", "add", "origin", bare.to_string_lossy().as_ref()]);
        bare
    }

    /// Bare clone of the current state in a sibling directory (kept alive by the same TempDir).
    pub fn bare_clone(&self) -> PathBuf {
        let bare = self.dir.path().with_extension("bare.git");
        let _ = std::fs::remove_dir_all(&bare);
        self.runner
            .run(
                None,
                [
                    "clone",
                    "-q",
                    "--bare",
                    self.path().to_string_lossy().as_ref(),
                    bare.to_string_lossy().as_ref(),
                ],
            )
            .expect("bare clone");
        bare
    }

    /// Add a linked worktree on a new branch and return its path.
    pub fn add_worktree(&self, branch: &str) -> PathBuf {
        let wt = self.dir.path().with_extension(format!("wt-{branch}"));
        self.git([
            "worktree",
            "add",
            "-q",
            "-b",
            branch,
            wt.to_string_lossy().as_ref(),
        ]);
        wt
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        // Sibling directories created by bare_clone / add_worktree are outside the TempDir.
        if let Some(parent) = self.dir.path().parent()
            && let Some(stem) = self.dir.path().file_name()
        {
            let prefix = stem.to_string_lossy().into_owned();
            if let Ok(entries) = std::fs::read_dir(parent) {
                for e in entries.flatten() {
                    let name = e.file_name().to_string_lossy().into_owned();
                    if name != prefix && name.starts_with(&prefix) {
                        let _ = std::fs::remove_dir_all(e.path());
                    }
                }
            }
        }
    }
}

/// A runner that ignores the user's global and system git config.
pub fn isolated_runner(sink: Arc<MemorySink>) -> GitRunner {
    GitRunner::new(sink)
        .with_env("GIT_CONFIG_GLOBAL", "/dev/null")
        .with_env("GIT_CONFIG_NOSYSTEM", "1")
        .with_env("HOME", std::env::temp_dir())
        .with_env("GIT_AUTHOR_NAME", "Osier Test")
        .with_env("GIT_AUTHOR_EMAIL", "test@example.com")
        .with_env("GIT_COMMITTER_NAME", "Osier Test")
        .with_env("GIT_COMMITTER_EMAIL", "test@example.com")
}
