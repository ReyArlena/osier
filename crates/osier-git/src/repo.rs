//! Repository discovery and the handle every query hangs off.

use std::path::{Path, PathBuf};

use crate::error::{GitError, Result};
use crate::runner::GitRunner;

/// An opened repository. Holds resolved paths and the runner to use.
#[derive(Debug, Clone)]
pub struct Repository {
    runner: GitRunner,
    /// Top of the working tree. For bare repositories this is the git dir itself.
    workdir: PathBuf,
    /// The `.git` directory of this checkout (worktrees have their own).
    git_dir: PathBuf,
    /// Shared git directory. Differs from `git_dir` inside a linked worktree.
    common_dir: PathBuf,
    bare: bool,
}

impl Repository {
    /// Discover the repository containing `path` (a file or directory inside it).
    pub fn discover(runner: GitRunner, path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let probe = if path.is_dir() {
            path.to_path_buf()
        } else {
            path.parent().map(Path::to_path_buf).unwrap_or_default()
        };
        if !probe.is_dir() {
            return Err(GitError::NotARepo(path.to_path_buf()));
        }
        // `--show-toplevel` fails in a bare repository, so ask about bareness first.
        let out = runner.run(
            Some(&probe),
            [
                "rev-parse",
                "--is-bare-repository",
                "--absolute-git-dir",
                "--git-common-dir",
            ],
        );
        let out = match out {
            Ok(o) => o,
            Err(GitError::NotARepo(_)) => return Err(GitError::NotARepo(path.to_path_buf())),
            Err(e) => return Err(e),
        };
        let text = out.stdout_str()?;
        let mut lines = text.lines();
        let bare = lines.next() == Some("true");
        let git_dir = PathBuf::from(
            lines
                .next()
                .ok_or_else(|| GitError::parse("rev-parse", "missing git dir"))?,
        );
        // `--git-common-dir` is relative to the cwd unless absolute.
        let common_dir = {
            let p = PathBuf::from(
                lines
                    .next()
                    .ok_or_else(|| GitError::parse("rev-parse", "missing common dir"))?,
            );
            if p.is_absolute() { p } else { probe.join(p) }
        };
        let workdir = if bare {
            git_dir.clone()
        } else {
            let out = runner.run(Some(&probe), ["rev-parse", "--show-toplevel"])?;
            PathBuf::from(out.stdout_str()?.trim_end_matches('\n'))
        };
        let common_dir = std::fs::canonicalize(&common_dir).unwrap_or(common_dir);
        Ok(Self {
            runner,
            workdir,
            git_dir,
            common_dir,
            bare,
        })
    }

    /// The same repository driven by another runner (e.g. one that logs only failures).
    pub fn with_runner(&self, runner: GitRunner) -> Self {
        Self {
            runner,
            ..self.clone()
        }
    }

    pub fn runner(&self) -> &GitRunner {
        &self.runner
    }

    pub fn workdir(&self) -> &Path {
        &self.workdir
    }

    pub fn git_dir(&self) -> &Path {
        &self.git_dir
    }

    pub fn common_dir(&self) -> &Path {
        &self.common_dir
    }

    pub fn is_bare(&self) -> bool {
        self.bare
    }

    /// True inside a linked worktree (not the main checkout).
    pub fn is_linked_worktree(&self) -> bool {
        self.git_dir != self.common_dir
    }

    /// Directory name, used as the default display name.
    pub fn name(&self) -> String {
        self.workdir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.workdir.display().to_string())
    }

    /// Run git inside this repository.
    pub fn git<I, S>(&self, args: I) -> Result<crate::runner::Output>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        self.runner.run(Some(&self.workdir), args)
    }

    /// Current HEAD commit, `None` on an unborn branch.
    pub fn head_oid(&self) -> Result<Option<String>> {
        let out = self.runner.run_allowing(
            Some(&self.workdir),
            ["rev-parse", "--verify", "--quiet", "HEAD"],
            &[0, 1],
        )?;
        match out.record.status {
            Some(0) => Ok(Some(out.stdout_str()?.trim().to_string())),
            Some(1) => Ok(None),
            Some(s) => Err(GitError::failed(s, out.record)),
            None => Err(GitError::timeout(0, out.record)),
        }
    }

    /// Operation git is in the middle of, if any (merge, rebase, ...).
    pub fn in_progress(&self) -> Option<InProgress> {
        let g = &self.git_dir;
        if g.join("rebase-merge").is_dir() || g.join("rebase-apply").is_dir() {
            Some(InProgress::Rebase)
        } else if g.join("MERGE_HEAD").exists() {
            Some(InProgress::Merge)
        } else if g.join("CHERRY_PICK_HEAD").exists() {
            Some(InProgress::CherryPick)
        } else if g.join("REVERT_HEAD").exists() {
            Some(InProgress::Revert)
        } else if g.join("BISECT_LOG").exists() {
            Some(InProgress::Bisect)
        } else {
            None
        }
    }

    /// True when `index.lock` exists.
    pub fn is_locked(&self) -> bool {
        self.git_dir.join("index.lock").exists()
    }
}

impl InProgress {
    /// Lower-case name for messages: "merge", "rebase", ...
    pub fn label(self) -> &'static str {
        match self {
            InProgress::Merge => "merge",
            InProgress::Rebase => "rebase",
            InProgress::CherryPick => "cherry-pick",
            InProgress::Revert => "revert",
            InProgress::Bisect => "bisect",
        }
    }
}

/// Multi-step git operation left in progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InProgress {
    Merge,
    Rebase,
    CherryPick,
    Revert,
    Bisect,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempRepo;

    #[test]
    fn discovers_from_subdirectory_and_file() {
        let t = TempRepo::new();
        t.write("src/lib.rs", "fn x() {}\n");
        let repo = Repository::discover(t.runner(), t.path().join("src")).unwrap();
        assert_eq!(repo.workdir(), t.path());
        assert!(!repo.is_bare());
        assert!(!repo.is_linked_worktree());
        let repo = Repository::discover(t.runner(), t.path().join("src/lib.rs")).unwrap();
        assert_eq!(repo.workdir(), t.path());
        assert_eq!(repo.name(), t.path().file_name().unwrap().to_string_lossy());
    }

    #[test]
    fn non_repo_is_not_a_repo() {
        let dir = tempfile::tempdir().unwrap();
        match Repository::discover(GitRunner::default(), dir.path()) {
            Err(GitError::NotARepo(_)) => {}
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn bare_repository() {
        let t = TempRepo::new();
        let bare = t.bare_clone();
        let repo = Repository::discover(t.runner(), &bare).unwrap();
        assert!(repo.is_bare());
    }

    #[test]
    fn linked_worktree() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "a\n", "first");
        let wt = t.add_worktree("feature");
        let repo = Repository::discover(t.runner(), &wt).unwrap();
        assert!(repo.is_linked_worktree());
        assert_eq!(repo.common_dir(), t.git_dir());
    }

    #[test]
    fn nothing_in_progress_on_clean_repo() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "a\n", "first");
        assert_eq!(t.repo().in_progress(), None);
        assert!(!t.repo().is_locked());
    }
}
