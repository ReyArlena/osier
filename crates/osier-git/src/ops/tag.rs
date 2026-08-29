//! Tags.

use crate::error::Result;
use crate::repo::Repository;

use super::validate_ref_name;

/// Create a tag at `target` (default HEAD). A message makes it annotated.
pub fn create(
    repo: &Repository,
    name: &str,
    target: Option<&str>,
    message: Option<&str>,
) -> Result<()> {
    validate_ref_name(repo, name)?;
    let target = target.unwrap_or("HEAD");
    match message {
        Some(m) => repo.git(["tag", "-a", "-m", m, name, target])?,
        None => repo.git(["tag", name, target])?,
    };
    Ok(())
}

pub fn delete(repo: &Repository, name: &str) -> Result<()> {
    repo.git(["tag", "-d", name])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::refs::tags;
    use crate::testing::TempRepo;

    #[test]
    fn create_and_delete() {
        let t = TempRepo::new();
        let first = t.commit_file("a.txt", "a\n", "first");
        t.commit_file("b.txt", "b\n", "second");
        let repo = t.repo();
        create(&repo, "v1", Some(&first), Some("release one")).unwrap();
        create(&repo, "light", None, None).unwrap();
        let list = tags(&repo).unwrap();
        let v1 = list.iter().find(|t| t.name == "v1").unwrap();
        assert!(v1.annotated);
        assert_eq!(v1.target, first);
        assert!(!list.iter().find(|t| t.name == "light").unwrap().annotated);
        delete(&repo, "v1").unwrap();
        assert_eq!(tags(&repo).unwrap().len(), 1);
    }
}
