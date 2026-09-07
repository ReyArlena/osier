//! Interactive rebase driven by a todo list we write ourselves.
//!
//! git is told to use `cp <our file>` as the sequence editor, so it never opens an editor
//! and runs exactly the list the user built in the UI. Rewords are expressed as a `pick`
//! followed by `exec git commit --amend -F <message file>`, which keeps git's own editor out
//! of the loop too.

use std::path::PathBuf;

use crate::error::{GitError, Result};
use crate::repo::Repository;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TodoAction {
    Pick,
    /// Keep the commit, replace its message.
    Reword(String),
    /// Stop after applying, so the user can amend.
    Edit,
    /// Meld into the previous commit, keeping both messages (git opens no editor: the
    /// concatenated message is kept as is).
    Squash,
    /// Meld into the previous commit, discarding this message.
    Fixup,
    Drop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoItem {
    pub hash: String,
    pub subject: String,
    pub action: TodoAction,
}

/// Commits that `rebase -i <base>` would list, oldest first.
pub fn commits_since(repo: &Repository, base: &str) -> Result<Vec<TodoItem>> {
    let range = format!("{base}..HEAD");
    let out = repo.git(["log", "--reverse", "--format=%H%x1f%s", &range])?;
    out.stdout_str()?
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| {
            let (hash, subject) = l
                .split_once('\x1f')
                .ok_or_else(|| GitError::parse("rebase todo", l.to_string()))?;
            Ok(TodoItem {
                hash: hash.to_string(),
                subject: subject.to_string(),
                action: TodoAction::Pick,
            })
        })
        .collect()
}

/// Render the todo file. Message files for rewords are written into `dir`.
pub fn render_todo(items: &[TodoItem], dir: &std::path::Path) -> std::io::Result<String> {
    let mut todo = String::new();
    for (ix, item) in items.iter().enumerate() {
        let short = &item.hash[..item.hash.len().min(12)];
        match &item.action {
            TodoAction::Pick => todo.push_str(&format!("pick {short} {}\n", item.subject)),
            TodoAction::Edit => todo.push_str(&format!("edit {short} {}\n", item.subject)),
            TodoAction::Squash => todo.push_str(&format!("squash {short} {}\n", item.subject)),
            TodoAction::Fixup => todo.push_str(&format!("fixup {short} {}\n", item.subject)),
            TodoAction::Drop => todo.push_str(&format!("drop {short} {}\n", item.subject)),
            TodoAction::Reword(message) => {
                let file = dir.join(format!("reword-{ix}.txt"));
                std::fs::write(&file, message)?;
                todo.push_str(&format!("pick {short} {}\n", item.subject));
                todo.push_str(&format!(
                    "exec git commit --amend --no-verify -F {}\n",
                    shell_quote(&file.to_string_lossy())
                ));
            }
        }
    }
    Ok(todo)
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Run `rebase -i <base>` with the given list. Conflicts leave the rebase in progress for
/// the usual continue/skip/abort. The first item cannot be a squash or fixup.
pub fn run(repo: &Repository, base: &str, items: &[TodoItem]) -> Result<()> {
    if matches!(
        items.first().map(|i| &i.action),
        Some(TodoAction::Squash | TodoAction::Fixup)
    ) {
        return Err(GitError::parse(
            "rebase todo",
            "the first commit cannot be squashed into nothing",
        ));
    }
    let dir = tempfile::Builder::new().prefix("osier-rebase-").tempdir()?;
    let todo_text = render_todo(items, dir.path())?;
    let todo_path: PathBuf = dir.path().join("todo");
    std::fs::write(&todo_path, todo_text)?;
    let runner = repo
        .runner()
        .clone()
        .with_env(
            "GIT_SEQUENCE_EDITOR",
            format!("cp {}", shell_quote(&todo_path.to_string_lossy())),
        )
        .with_env("GIT_EDITOR", "true");
    let out = runner.run(Some(repo.workdir()), ["rebase", "-i", base]);
    // Keep the temp dir alive until git has read the todo (it copies it immediately, but the
    // exec lines reference message files until the rebase finishes). A rebase that stops on a
    // conflict still needs those files later, so leak them into the git dir instead.
    if let Err(e) = &out
        && matches!(e, GitError::Conflict(_))
    {
        let keep = repo.git_dir().join("osier-rebase");
        let _ = std::fs::create_dir_all(&keep);
        for entry in std::fs::read_dir(dir.path())?.flatten() {
            let _ = std::fs::copy(entry.path(), keep.join(entry.file_name()));
        }
    }
    out.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::{LogOptions, log};
    use crate::testing::TempRepo;

    fn subjects(t: &TempRepo) -> Vec<String> {
        log(&t.repo(), &LogOptions::default())
            .unwrap()
            .into_iter()
            .map(|c| c.subject)
            .collect()
    }

    #[test]
    fn reorder_reword_squash_fixup_drop() {
        let t = TempRepo::new();
        t.commit_file("base.txt", "b\n", "base");
        let base = t.head();
        t.commit_file("1.txt", "1\n", "one");
        t.commit_file("2.txt", "2\n", "two");
        t.commit_file("3.txt", "3\n", "three");
        t.commit_file("4.txt", "4\n", "four");
        let repo = t.repo();
        let mut items = commits_since(&repo, &base).unwrap();
        assert_eq!(
            items.iter().map(|i| i.subject.as_str()).collect::<Vec<_>>(),
            ["one", "two", "three", "four"]
        );

        // two first (reworded), then one with three squashed in, drop four.
        items.swap(0, 1);
        items[0].action = TodoAction::Reword("two, reworded".into());
        items[2].action = TodoAction::Fixup;
        items[3].action = TodoAction::Drop;
        run(&repo, &base, &items).unwrap();
        assert_eq!(subjects(&t), ["one", "two, reworded", "base"]);
        assert!(t.path().join("3.txt").exists(), "fixup kept the change");
        assert!(!t.path().join("4.txt").exists(), "dropped");
        assert_eq!(repo.in_progress(), None);
    }

    #[test]
    fn squash_first_is_rejected_and_conflict_leaves_rebase_in_progress() {
        let t = TempRepo::new();
        t.commit_file("f.txt", "a\n", "base");
        let base = t.head();
        t.commit_file("f.txt", "b\n", "one");
        t.commit_file("f.txt", "c\n", "two");
        let repo = t.repo();
        let mut items = commits_since(&repo, &base).unwrap();
        items[0].action = TodoAction::Squash;
        assert!(run(&repo, &base, &items).is_err());

        let mut items = commits_since(&repo, &base).unwrap();
        items.swap(0, 1);
        assert!(matches!(
            run(&repo, &base, &items),
            Err(GitError::Conflict(_))
        ));
        assert_eq!(repo.in_progress(), Some(crate::repo::InProgress::Rebase));
        crate::ops::integrate::abort(&repo).unwrap();
        assert_eq!(subjects(&t), ["two", "one", "base"]);
    }
}
