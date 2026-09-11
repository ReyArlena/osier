//! Find repositories under the configured scan directories.

use std::path::{Path, PathBuf};

/// Immediate children (and deeper, up to `depth`) that are git repositories, sorted by name.
/// Hidden directories are skipped. A repository's own children are not descended into.
pub fn scan(dirs: &[PathBuf], depth: u8) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for dir in dirs {
        let dir = expand_home(dir);
        walk(&dir, depth.max(1), &mut found);
    }
    found.sort();
    found.dedup();
    found
}

fn walk(dir: &Path, depth: u8, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut children: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .filter(|p| {
            !p.file_name()
                .map(|n| n.to_string_lossy().starts_with('.'))
                .unwrap_or(true)
        })
        .collect();
    children.sort();
    for child in children {
        if child.join(".git").exists() {
            found.push(child);
        } else if depth > 1 {
            walk(&child, depth - 1, found);
        }
    }
}

/// `~` and `~/x` to the home directory.
pub fn expand_home(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix('~')
        && let Some(home) = std::env::var_os("HOME")
    {
        let rest = rest.trim_start_matches('/');
        return if rest.is_empty() {
            PathBuf::from(home)
        } else {
            PathBuf::from(home).join(rest)
        };
    }
    p.to_path_buf()
}

/// The scan directory that provides `repo`, if any.
pub fn provider<'a>(dirs: &'a [PathBuf], repo: &Path) -> Option<&'a PathBuf> {
    dirs.iter().find(|d| repo.starts_with(expand_home(d)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_repos_one_level_down_and_skips_hidden() {
        let root = tempfile::tempdir().unwrap();
        for name in ["a", "b", ".hidden", "plain"] {
            std::fs::create_dir_all(root.path().join(name)).unwrap();
        }
        for name in ["a", "b", ".hidden"] {
            std::fs::create_dir_all(root.path().join(name).join(".git")).unwrap();
        }
        std::fs::create_dir_all(root.path().join("plain/nested/.git")).unwrap();
        let found = scan(&[root.path().to_path_buf()], 1);
        let names: Vec<String> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["a", "b"]);
        let deeper = scan(&[root.path().to_path_buf()], 2);
        assert!(deeper.iter().any(|p| p.ends_with("plain/nested")));
        assert_eq!(
            provider(&[root.path().to_path_buf()], &found[0]),
            Some(&root.path().to_path_buf())
        );
        assert_eq!(provider(&[PathBuf::from("/nowhere")], &found[0]), None);
    }

    #[test]
    fn expands_tilde() {
        let home = std::env::var("HOME").unwrap();
        assert_eq!(
            expand_home(Path::new("~/x")),
            PathBuf::from(format!("{home}/x"))
        );
        assert_eq!(expand_home(Path::new("~")), PathBuf::from(&home));
        assert_eq!(expand_home(Path::new("/abs")), PathBuf::from("/abs"));
    }
}
