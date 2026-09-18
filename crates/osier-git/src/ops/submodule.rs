//! Submodule maintenance. Listing lives in [`crate::submodule`].

use crate::error::Result;
use crate::repo::Repository;

use super::with_paths;

/// `git submodule update --init --recursive [-- paths]`: clone missing submodules and check
/// out the recorded commits. Empty `paths` means all of them.
pub fn update(repo: &Repository, paths: &[String]) -> Result<()> {
    let args = vec!["submodule", "update", "--init", "--recursive"];
    if paths.is_empty() {
        repo.git(args)?;
    } else {
        repo.git(with_paths(args, paths))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::submodule::list;
    use crate::testing::TempRepo;

    #[test]
    fn update_initialises_a_cloned_submodule() {
        let dep = TempRepo::new();
        dep.commit_file("d.txt", "d\n", "dep");
        let sup = TempRepo::new();
        sup.commit_file("a.txt", "a\n", "first");
        sup.git([
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "-q",
            dep.path().to_string_lossy().as_ref(),
            "libs/dep",
        ]);
        sup.commit("add submodule");

        // A fresh clone has the submodule recorded but not checked out.
        let clone_dir = tempfile::tempdir().unwrap();
        let dest = clone_dir.path().join("clone");
        sup.git([
            "clone",
            "-q",
            sup.path().to_string_lossy().as_ref(),
            dest.to_string_lossy().as_ref(),
        ]);
        // file:// submodule URLs are refused by default (CVE-2022-39253); the test runner
        // allows them through the environment, which is what a user would do deliberately.
        let runner = sup.runner().with_env("GIT_ALLOW_PROTOCOL", "file");
        let clone = Repository::discover(runner, &dest).unwrap();
        let before = list(&clone).unwrap();
        assert_eq!(before.len(), 1);
        assert!(before[0].head.is_none(), "not initialised yet");

        update(&clone, &["libs/dep".into()]).unwrap();
        let after = list(&clone).unwrap();
        assert!(after[0].head.is_some(), "{after:?}");
        assert!(dest.join("libs/dep/d.txt").is_file());

        // Updating everything on an up-to-date tree is a no-op.
        update(&clone, &[]).unwrap();
    }

    #[test]
    fn update_fails_for_an_unknown_path() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "a\n", "first");
        assert!(update(&t.repo(), &["nope".into()]).is_err());
    }
}
