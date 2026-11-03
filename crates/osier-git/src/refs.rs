//! Branches, tags and remotes via `git for-each-ref` and `git config`.

use crate::error::{GitError, Result};
use crate::repo::Repository;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Branch {
    /// Short name: `main` or `origin/main`.
    pub name: String,
    /// Full ref: `refs/heads/main` or `refs/remotes/origin/main`.
    pub full_ref: String,
    pub oid: String,
    pub is_remote: bool,
    pub is_head: bool,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    /// Upstream configured but its ref no longer exists.
    pub upstream_gone: bool,
    /// Unix timestamp of the tip commit.
    pub committed_at: i64,
    pub subject: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tag {
    pub name: String,
    /// The tag object for annotated tags, the commit for lightweight ones.
    pub oid: String,
    /// The commit the tag points at.
    pub target: String,
    pub annotated: bool,
    pub created_at: i64,
    pub subject: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remote {
    pub name: String,
    pub fetch_url: String,
    pub push_url: Option<String>,
}

const BRANCH_FORMAT: &str = "%(refname)%00%(refname:short)%00%(objectname)%00%(upstream:short)%00%(upstream:track,nobracket)%00%(HEAD)%00%(committerdate:unix)%00%(subject)";

pub fn branches(repo: &Repository) -> Result<Vec<Branch>> {
    let out = repo.git([
        "for-each-ref",
        "--format",
        BRANCH_FORMAT,
        "--sort=refname",
        "refs/heads",
        "refs/remotes",
    ])?;
    parse_branches(out.stdout_str()?)
}

pub fn parse_branches(text: &str) -> Result<Vec<Branch>> {
    text.lines()
        .filter(|l| !l.is_empty())
        .map(|line| {
            let f: Vec<&str> = line.split('\0').collect();
            if f.len() != 8 {
                return Err(GitError::parse(
                    "for-each-ref",
                    format!("{} fields in {line:?}", f.len()),
                ));
            }
            let full_ref = f[0].to_string();
            let is_remote = full_ref.starts_with("refs/remotes/");
            // `origin/HEAD` is a symref, not a branch.
            let (mut ahead, mut behind, mut gone) = (0, 0, false);
            for part in f[4].split(", ") {
                if part == "gone" {
                    gone = true;
                } else if let Some(n) = part.strip_prefix("ahead ") {
                    ahead = n
                        .parse()
                        .map_err(|_| GitError::parse("for-each-ref", part))?;
                } else if let Some(n) = part.strip_prefix("behind ") {
                    behind = n
                        .parse()
                        .map_err(|_| GitError::parse("for-each-ref", part))?;
                }
            }
            Ok(Branch {
                name: f[1].to_string(),
                full_ref,
                oid: f[2].to_string(),
                is_remote,
                is_head: f[5] == "*",
                upstream: (!f[3].is_empty()).then(|| f[3].to_string()),
                ahead,
                behind,
                upstream_gone: gone,
                committed_at: f[6].parse().unwrap_or(0),
                subject: f[7].to_string(),
            })
        })
        .filter(|b| !matches!(b, Ok(b) if b.is_remote && b.full_ref.ends_with("/HEAD")))
        .collect()
}

const TAG_FORMAT: &str = "%(refname:short)%00%(objectname)%00%(*objectname)%00%(objecttype)%00%(creatordate:unix)%00%(subject)";

pub fn tags(repo: &Repository) -> Result<Vec<Tag>> {
    let out = repo.git([
        "for-each-ref",
        "--format",
        TAG_FORMAT,
        "--sort=-creatordate",
        "refs/tags",
    ])?;
    parse_tags(out.stdout_str()?)
}

pub fn parse_tags(text: &str) -> Result<Vec<Tag>> {
    text.lines()
        .filter(|l| !l.is_empty())
        .map(|line| {
            let f: Vec<&str> = line.split('\0').collect();
            if f.len() != 6 {
                return Err(GitError::parse("for-each-ref tags", line.to_string()));
            }
            let annotated = f[3] == "tag";
            Ok(Tag {
                name: f[0].to_string(),
                oid: f[1].to_string(),
                target: if annotated {
                    f[2].to_string()
                } else {
                    f[1].to_string()
                },
                annotated,
                created_at: f[4].parse().unwrap_or(0),
                subject: f[5].to_string(),
            })
        })
        .collect()
}

pub fn remotes(repo: &Repository) -> Result<Vec<Remote>> {
    let out = repo.runner().run_allowing(
        Some(repo.workdir()),
        ["config", "--get-regexp", r"^remote\..*\.(url|pushurl)$"],
        &[0, 1],
    )?;
    // Exit 1 means "no matches", which is a repo without remotes.
    match out.record.status {
        Some(0) | Some(1) => parse_remotes(out.stdout_str()?),
        Some(s) => Err(GitError::failed(s, out.record)),
        None => Err(GitError::timeout(0, out.record)),
    }
}

pub fn parse_remotes(text: &str) -> Result<Vec<Remote>> {
    let mut list: Vec<Remote> = Vec::new();
    for line in text.lines().filter(|l| !l.is_empty()) {
        let (key, value) = line
            .split_once(' ')
            .ok_or_else(|| GitError::parse("remotes", line.to_string()))?;
        let name_and_kind = key
            .strip_prefix("remote.")
            .ok_or_else(|| GitError::parse("remotes", line.to_string()))?;
        let (name, kind) = name_and_kind
            .rsplit_once('.')
            .ok_or_else(|| GitError::parse("remotes", line.to_string()))?;
        let entry = match list.iter_mut().find(|r| r.name == name) {
            Some(e) => e,
            None => {
                list.push(Remote {
                    name: name.to_string(),
                    fetch_url: String::new(),
                    push_url: None,
                });
                list.last_mut()
                    .ok_or_else(|| GitError::parse("remotes", "push"))?
            }
        };
        match kind {
            "url" => entry.fetch_url = value.to_string(),
            "pushurl" => entry.push_url = Some(value.to_string()),
            _ => {}
        }
    }
    Ok(list)
}

/// Local and remote-tracking branches that contain `commit`, sorted (`origin/HEAD` left out).
pub fn branches_containing(repo: &Repository, commit: &str) -> Result<Vec<String>> {
    let out = repo.git([
        "branch",
        "--all",
        "--format=%(refname:short)",
        "--contains",
        commit,
    ])?;
    let mut names: Vec<String> = out
        .stdout_str()?
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('(') && !l.ends_with("/HEAD"))
        .map(str::to_string)
        .collect();
    names.sort();
    names.dedup();
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempRepo;

    #[test]
    fn branches_containing_lists_local_and_remote_branches() {
        let t = TempRepo::new();
        let first = t.commit_file("a.txt", "a\n", "first");
        t.add_bare_origin();
        t.git(["push", "-q", "-u", "origin", "HEAD"]);
        t.git(["branch", "feature"]);
        let second = t.commit_file("a.txt", "b\n", "second");
        let mut want = vec!["feature", "main", "origin/main"];
        want.sort();
        assert_eq!(branches_containing(&t.repo(), first.trim()).unwrap(), want);
        assert_eq!(
            branches_containing(&t.repo(), second.trim()).unwrap(),
            ["main"]
        );
    }

    #[test]
    fn parses_branch_lines() {
        let text = "refs/heads/feature\x00feature\x00aaaa\x00\x00\x00 \x00100\x00first\n\
refs/heads/main\x00main\x00bbbb\x00origin/main\x00ahead 2, behind 1\x00*\x00200\x00second\n\
refs/heads/stale\x00stale\x00cccc\x00origin/stale\x00gone\x00 \x00300\x00third\n\
refs/remotes/origin/HEAD\x00origin/HEAD\x00bbbb\x00\x00\x00 \x00200\x00second\n\
refs/remotes/origin/main\x00origin/main\x00bbbb\x00\x00\x00 \x00200\x00second\n";
        let b = parse_branches(text).unwrap();
        assert_eq!(b.len(), 4, "origin/HEAD dropped: {b:?}");
        let main = b.iter().find(|b| b.name == "main").unwrap();
        assert!(main.is_head);
        assert_eq!((main.ahead, main.behind), (2, 1));
        assert_eq!(main.upstream.as_deref(), Some("origin/main"));
        assert!(b.iter().find(|b| b.name == "stale").unwrap().upstream_gone);
        assert!(
            b.iter()
                .find(|b| b.name == "origin/main")
                .unwrap()
                .is_remote
        );
    }

    #[test]
    fn parses_remotes_with_pushurl() {
        let text = "remote.origin.url https://example.com/a.git\nremote.origin.pushurl git@example.com:a.git\nremote.up.url https://example.com/b.git\n";
        let r = parse_remotes(text).unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].push_url.as_deref(), Some("git@example.com:a.git"));
        assert_eq!(r[1].name, "up");
    }

    #[test]
    fn live_branches_tags_remotes() {
        let t = TempRepo::new();
        let first = t.commit_file("a.txt", "a\n", "first");
        t.branch("feature");
        t.git(["tag", "v0.1"]);
        t.git(["tag", "-a", "v0.2", "-m", "release 0.2"]);
        let bare = t.add_bare_origin();
        t.git(["push", "-q", "-u", "origin", "main"]);
        t.commit_file("b.txt", "b\n", "second");

        let repo = t.repo();
        let b = branches(&repo).unwrap();
        let names: Vec<_> = b.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, ["feature", "main", "origin/main"]);
        let main = &b[1];
        assert!(main.is_head);
        assert_eq!(main.ahead, 1);
        assert_eq!(main.subject, "second");

        let tags = tags(&repo).unwrap();
        assert_eq!(tags.len(), 2);
        let v2 = tags.iter().find(|t| t.name == "v0.2").unwrap();
        assert!(v2.annotated);
        assert_eq!(v2.target, first);
        assert_eq!(v2.subject, "release 0.2");
        let v1 = tags.iter().find(|t| t.name == "v0.1").unwrap();
        assert!(!v1.annotated);
        assert_eq!(v1.target, first);

        let r = remotes(&repo).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].name, "origin");
        assert_eq!(r[0].fetch_url, bare.to_string_lossy());
    }

    #[test]
    fn no_remotes_is_empty_not_error() {
        let t = TempRepo::new();
        assert!(remotes(&t.repo()).unwrap().is_empty());
    }
}
