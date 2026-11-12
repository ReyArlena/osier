//! Throwing changes away. Every function here loses data; the UI confirms first.

use std::path::PathBuf;

use crate::error::{GitError, Result};
use crate::repo::Repository;

use super::with_paths;

/// Drop unstaged changes: restore the working tree copy from the index.
pub fn discard_worktree(repo: &Repository, paths: &[String]) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    repo.git(with_paths(vec!["restore", "--worktree"], paths))?;
    Ok(())
}

/// Drop staged and unstaged changes: restore index and working tree from HEAD.
///
/// git replaces every file it restores (it removes the file, then writes it again), even one
/// whose content already matches HEAD, and a program watching the file can catch the moment it
/// is missing. With `skip_unchanged`, a path whose working copy has nothing that differs from
/// HEAD only gets its index entries reset and is not touched on disk.
pub fn discard_all(repo: &Repository, paths: &[String], skip_unchanged: bool) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    if !skip_unchanged {
        return restore_from_head(repo, paths, true);
    }
    // Content and mode, HEAD against the working tree: what a restore would really change.
    let out = repo.git(with_paths(
        vec!["diff", "--name-only", "-z", "--no-renames", "HEAD"],
        paths,
    ))?;
    let listed = out.stdout_str()?;
    let differing: Vec<&str> = listed.split('\0').filter(|d| !d.is_empty()).collect();
    let (rewrite, index_only): (Vec<String>, Vec<String>) = paths
        .iter()
        .cloned()
        .partition(|p| !plain_path(p) || differing.iter().any(|d| covers(p, d)));
    restore_from_head(repo, &rewrite, true)?;
    restore_from_head(repo, &index_only, false)
}

/// `git restore` from HEAD into the index, and into the working tree too with `worktree`.
fn restore_from_head(repo: &Repository, paths: &[String], worktree: bool) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    let mut args = vec!["restore", "--source=HEAD", "--staged"];
    if worktree {
        args.push("--worktree");
    }
    repo.git(with_paths(args, paths))?;
    Ok(())
}

/// Whether `path` names one file or folder as written, so the files git lists under it can be
/// matched by name. Anything git would expand itself is restored in full instead.
fn plain_path(path: &str) -> bool {
    !path.is_empty()
        && path != "."
        && !path.starts_with(':')
        && !path.contains(['*', '?', '[', '\\'])
}

/// Whether `file` (a path git listed) is `path` or lies inside the folder `path`.
fn covers(path: &str, file: &str) -> bool {
    let path = path.trim_end_matches('/');
    file == path
        || file
            .strip_prefix(path)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// Move untracked files to the system trash instead of deleting them.
pub fn trash_untracked(repo: &Repository, paths: &[String]) -> Result<Vec<PathBuf>> {
    let mut moved = Vec::new();
    for p in paths {
        let rel = std::path::Path::new(p);
        let escapes = rel.is_absolute()
            || rel
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir));
        let full = repo.workdir().join(rel);
        if escapes || !full.starts_with(repo.workdir()) {
            return Err(GitError::Parse {
                context: "trash",
                detail: format!("{p} escapes the repository"),
            });
        }
        if !full.exists() {
            continue;
        }
        trash::delete(&full).map_err(|e| GitError::Io(std::io::Error::other(e.to_string())))?;
        moved.push(full);
    }
    Ok(moved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::{StatusOptions, status};
    use crate::testing::TempRepo;

    #[test]
    fn discard_worktree_keeps_staged_part() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "base\n", "first");
        t.write("a.txt", "staged\n");
        t.git(["add", "a.txt"]);
        t.write("a.txt", "staged\nmore\n");
        discard_worktree(&t.repo(), &["a.txt".into()]).unwrap();
        assert_eq!(t.read("a.txt"), "staged\n");
        let s = status(&t.repo(), StatusOptions::default()).unwrap();
        assert_eq!(s.staged().count(), 1);
        assert_eq!(s.unstaged().count(), 0);
    }

    #[test]
    fn discard_all_restores_head() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "base\n", "first");
        t.write("a.txt", "staged\n");
        t.git(["add", "a.txt"]);
        t.write("a.txt", "and more\n");
        discard_all(&t.repo(), &["a.txt".into()], false).unwrap();
        assert_eq!(t.read("a.txt"), "base\n");
        assert!(
            status(&t.repo(), StatusOptions::default())
                .unwrap()
                .is_clean()
        );
    }

    fn clean(t: &TempRepo) -> bool {
        status(&t.repo(), StatusOptions::default())
            .unwrap()
            .is_clean()
    }

    #[cfg(unix)]
    fn inode(t: &TempRepo, rel: &str) -> u64 {
        use std::os::unix::fs::MetadataExt as _;
        std::fs::metadata(t.path().join(rel)).unwrap().ino()
    }

    #[cfg(unix)]
    #[test]
    fn skip_unchanged_leaves_a_file_that_matches_head_alone() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "base\n", "first");
        // Staged, then put back in the file: only the index differs from HEAD.
        t.write("a.txt", "staged\n");
        t.git(["add", "a.txt"]);
        // Written in place, like an editor undoing the change (same inode).
        t.write("a.txt", "base\n");
        let before = inode(&t, "a.txt");
        discard_all(&t.repo(), &["a.txt".into()], true).unwrap();
        assert_eq!(inode(&t, "a.txt"), before, "the file was not replaced");
        assert_eq!(t.read("a.txt"), "base\n");
        assert!(clean(&t));
    }

    #[cfg(unix)]
    #[test]
    fn skip_unchanged_still_restores_what_differs() {
        use std::os::unix::fs::PermissionsExt as _;
        let t = TempRepo::new();
        t.commit_file("text.txt", "base\n", "first");
        t.commit_file("mode.sh", "echo hi\n", "second");
        t.commit_file("dir/same.txt", "same\n", "third");
        t.commit_file("dir/edited.txt", "old\n", "fourth");
        t.commit_file("kept.txt", "kept\n", "fifth");
        t.write("text.txt", "staged\n");
        t.git(["add", "text.txt"]);
        t.write("text.txt", "and more\n");
        std::fs::set_permissions(
            t.path().join("mode.sh"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        t.write("dir/edited.txt", "new\n");
        t.write("added.txt", "new file\n");
        t.git(["add", "added.txt"]);
        let kept = inode(&t, "kept.txt");
        let paths: Vec<String> = ["text.txt", "mode.sh", "dir", "added.txt", "kept.txt"]
            .map(String::from)
            .to_vec();
        discard_all(&t.repo(), &paths, true).unwrap();
        assert_eq!(t.read("text.txt"), "base\n");
        assert_eq!(t.read("dir/edited.txt"), "old\n");
        let mode = std::fs::metadata(t.path().join("mode.sh")).unwrap();
        assert_eq!(mode.permissions().mode() & 0o111, 0, "not executable again");
        // A file that is not in HEAD goes, as without the option.
        assert!(!t.path().join("added.txt").exists());
        assert_eq!(inode(&t, "kept.txt"), kept);
        assert!(clean(&t));
    }

    #[test]
    fn only_plain_paths_are_matched_by_name() {
        assert!(plain_path("src/a.txt") && plain_path("dir/"));
        assert!(!plain_path(".") && !plain_path(":/") && !plain_path("src/*.rs"));
        assert!(covers("dir", "dir/a.txt") && covers("dir/", "dir/a.txt"));
        assert!(covers("a.txt", "a.txt") && !covers("dir", "dirty.txt"));
    }

    #[test]
    fn trash_refuses_paths_outside_the_repo() {
        let t = TempRepo::new();
        assert!(trash_untracked(&t.repo(), &["../escape.txt".into()]).is_err());
    }
}
