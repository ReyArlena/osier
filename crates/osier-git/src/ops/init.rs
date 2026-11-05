//! A new local repository ("Create new repository…" with no remote): `git init` in a new
//! directory, plus the starter files the user picked.

use std::path::Path;

use crate::error::Result;
use crate::ops::remote::check_empty_target;
use crate::runner::GitRunner;

/// Starter files written into the new working tree, left uncommitted for the user to review.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scaffold {
    pub readme: Option<String>,
    pub gitignore: Option<String>,
    pub license: Option<String>,
}

/// Create `target` (missing or empty only) and run `git init` there.
pub fn init(runner: &GitRunner, target: &Path, scaffold: &Scaffold) -> Result<()> {
    check_empty_target(target)?;
    std::fs::create_dir_all(target)?;
    runner.run(None, ["init".as_ref(), "-q".as_ref(), target.as_os_str()])?;
    for (name, text) in [
        ("README.md", &scaffold.readme),
        (".gitignore", &scaffold.gitignore),
        ("LICENSE", &scaffold.license),
    ] {
        if let Some(text) = text {
            std::fs::write(target.join(name), text)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::GitError;
    use crate::testing::TempRepo;

    #[test]
    fn init_writes_the_picked_files_and_refuses_a_full_directory() {
        let t = TempRepo::new();
        let dest = tempfile::tempdir().unwrap();
        let target = dest.path().join("fresh");
        let scaffold = Scaffold {
            readme: Some("# fresh\n".into()),
            gitignore: Some("/target\n".into()),
            license: None,
        };
        init(&t.runner(), &target, &scaffold).unwrap();
        assert!(target.join(".git").is_dir());
        assert_eq!(
            std::fs::read_to_string(target.join("README.md")).unwrap(),
            "# fresh\n"
        );
        assert!(target.join(".gitignore").exists());
        assert!(!target.join("LICENSE").exists());

        let err = init(&t.runner(), &target, &Scaffold::default()).unwrap_err();
        assert!(matches!(err, GitError::Refused(m) if m.contains("not empty")));
    }
}
