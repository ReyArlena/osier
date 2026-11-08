//! Commit history via `git log -z` with a NUL/unit-separator format.

use crate::error::{GitError, Result};
use crate::repo::Repository;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub hash: String,
    pub parents: Vec<String>,
    pub author_name: String,
    pub author_email: String,
    pub authored_at: i64,
    pub committer_name: String,
    pub committer_email: String,
    pub committed_at: i64,
    /// Decorations: `HEAD -> main`, `origin/main`, `tag: v1.0`.
    pub refs: Vec<String>,
    pub subject: String,
    pub body: String,
}

impl Commit {
    pub fn short_hash(&self) -> &str {
        &self.hash[..self.hash.len().min(7)]
    }

    pub fn is_merge(&self) -> bool {
        self.parents.len() > 1
    }
}

/// Which commits to list.
#[derive(Debug, Clone, Default)]
pub struct LogOptions {
    /// Revisions to start from. Empty means HEAD. `--all` for everything.
    pub revisions: Vec<String>,
    pub all: bool,
    pub skip: usize,
    pub limit: Option<usize>,
    /// Restrict to a path; `follow` tracks renames (single path only).
    pub path: Option<String>,
    pub follow: bool,
    /// Row order. The graph needs children before parents, which `Topo` and `Date` give.
    pub order: LogOrder,
    pub author: Option<String>,
    pub grep: Option<String>,
}

/// How `git log` orders commits.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LogOrder {
    /// git's default: reverse commit date, parents may come before children.
    #[default]
    Default,
    /// `--topo-order`: children before parents, each branch's commits kept together.
    Topo,
    /// `--date-order`: children before parents, branches interleaved by commit date (how
    /// PhpStorm orders its log).
    Date,
}

const FORMAT: &str = "%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%cn%x1f%ce%x1f%ct%x1f%D%x1f%s%x1f%b";

pub fn log(repo: &Repository, options: &LogOptions) -> Result<Vec<Commit>> {
    let mut args: Vec<String> = vec![
        "log".into(),
        "-z".into(),
        format!("--format={FORMAT}"),
        "--decorate=full".into(),
        "--decorate-refs-exclude=refs/prefetch".into(),
    ];
    match options.order {
        LogOrder::Default => {}
        LogOrder::Topo => args.push("--topo-order".into()),
        LogOrder::Date => args.push("--date-order".into()),
    }
    if options.all {
        args.push("--all".into());
    }
    if options.skip > 0 {
        args.push(format!("--skip={}", options.skip));
    }
    if let Some(n) = options.limit {
        args.push(format!("-n{n}"));
    }
    if let Some(a) = &options.author {
        args.push(format!("--author={a}"));
    }
    if let Some(g) = &options.grep {
        args.push(format!("--grep={g}"));
    }
    if options.follow && options.path.is_some() {
        args.push("--follow".into());
    }
    args.extend(options.revisions.iter().cloned());
    if let Some(p) = &options.path {
        args.push("--".into());
        args.push(p.clone());
    }
    let out = repo
        .runner()
        .run_unchecked(Some(repo.workdir()), &args, None)?;
    match out.record.status {
        Some(0) => parse(&out.stdout),
        // Unborn branch: nothing to show.
        Some(128) if out.record.stderr.contains("does not have any commits") => Ok(Vec::new()),
        Some(s) => Err(GitError::failed(s, out.record)),
        None => Err(GitError::timeout(0, out.record)),
    }
}

/// Commits on local branches that no remote branch contains yet ("not pushed"), up to
/// `limit`. Empty when the repository has no remote-tracking branches at all: with nowhere to
/// push, nothing counts as unpushed.
pub fn unpushed(repo: &Repository, limit: usize) -> Result<std::collections::HashSet<String>> {
    let remotes = repo.git([
        "for-each-ref",
        "--count=1",
        "--format=%(refname)",
        "refs/remotes",
    ])?;
    if remotes.stdout_str()?.trim().is_empty() {
        return Ok(Default::default());
    }
    let out = repo.runner().run_allowing(
        Some(repo.workdir()),
        [
            "rev-list",
            &format!("--max-count={limit}"),
            "--branches",
            "--not",
            "--remotes",
        ],
        &[0, 128],
    )?;
    if out.record.status != Some(0) {
        return Ok(Default::default());
    }
    Ok(out
        .stdout_str()?
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect())
}

/// Full messages of the last `n` commits on HEAD, newest first, duplicates removed. Empty on
/// an unborn branch (git exits 128 there).
pub fn recent_messages(repo: &Repository, n: usize) -> Result<Vec<String>> {
    let out = repo.runner().run_allowing(
        Some(repo.workdir()),
        ["log", &format!("-{n}"), "--format=%B%x00", "HEAD"],
        &[0, 128],
    )?;
    if out.record.status != Some(0) {
        return Ok(Vec::new());
    }
    let mut messages: Vec<String> = Vec::new();
    for m in out.stdout_str()?.split('\0') {
        let m = m.trim();
        if !m.is_empty() && !messages.iter().any(|x| x == m) {
            messages.push(m.to_string());
        }
    }
    Ok(messages)
}

pub fn parse(bytes: &[u8]) -> Result<Vec<Commit>> {
    bytes
        .split(|b| *b == 0)
        .filter(|rec| !rec.is_empty())
        .map(parse_record)
        .collect()
}

fn parse_record(rec: &[u8]) -> Result<Commit> {
    let text = std::str::from_utf8(rec).map_err(|_| GitError::Utf8("log"))?;
    let f: Vec<&str> = text.splitn(11, '\x1f').collect();
    if f.len() != 11 {
        return Err(GitError::parse("log", format!("{} fields", f.len())));
    }
    let refs = parse_decorations(f[8]);
    Ok(Commit {
        hash: f[0].to_string(),
        parents: f[1]
            .split(' ')
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .collect(),
        author_name: f[2].to_string(),
        author_email: f[3].to_string(),
        authored_at: f[4].parse().map_err(|_| GitError::parse("log", f[4]))?,
        committer_name: f[5].to_string(),
        committer_email: f[6].to_string(),
        committed_at: f[7].parse().map_err(|_| GitError::parse("log", f[7]))?,
        refs,
        subject: f[9].to_string(),
        body: f[10].trim_end_matches('\n').to_string(),
    })
}

/// `HEAD -> refs/heads/main, refs/remotes/origin/main, tag: refs/tags/v1` into short names.
fn parse_decorations(d: &str) -> Vec<String> {
    d.split(", ")
        .filter(|s| !s.is_empty())
        .map(|s| {
            let s = s.trim();
            let shorten = |r: &str| {
                r.strip_prefix("refs/heads/")
                    .or_else(|| r.strip_prefix("refs/remotes/"))
                    .or_else(|| r.strip_prefix("refs/tags/"))
                    .unwrap_or(r)
                    .to_string()
            };
            if let Some(rest) = s.strip_prefix("HEAD -> ") {
                format!("HEAD -> {}", shorten(rest))
            } else if let Some(rest) = s.strip_prefix("tag: ") {
                format!("tag: {}", shorten(rest))
            } else {
                shorten(s)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unpushed_lists_local_only_commits_and_nothing_without_remotes() {
        let t = crate::testing::TempRepo::new();
        t.commit_file("a.txt", "a\n", "a");
        t.commit_file("a.txt", "b\n", "b");
        assert!(
            unpushed(&t.repo(), 100).unwrap().is_empty(),
            "no remote: nothing unpushed"
        );
        t.add_bare_origin();
        t.git(["push", "-q", "-u", "origin", "HEAD"]);
        assert!(unpushed(&t.repo(), 100).unwrap().is_empty());
        let c = t.commit_file("a.txt", "c\n", "c");
        t.git(["checkout", "-qb", "feature"]);
        let d = t.commit_file("d.txt", "d\n", "d");
        let got = unpushed(&t.repo(), 100).unwrap();
        assert_eq!(got.len(), 2);
        assert!(got.contains(c.trim()) && got.contains(d.trim()));
    }
    use crate::testing::TempRepo;

    #[test]
    fn parses_record_with_body_and_decorations() {
        let raw = b"aaaa\x1fbbbb cccc\x1fAnn\x1fann@example.com\x1f1700000000\x1fCom\x1fcom@example.com\x1f1700000001\x1fHEAD -> refs/heads/main, refs/remotes/origin/main, tag: refs/tags/v1\x1fSubject line\x1fBody line one\n\nBody line two\n\0";
        let c = parse(raw).unwrap();
        assert_eq!(c.len(), 1);
        let c = &c[0];
        assert_eq!(c.parents, ["bbbb", "cccc"]);
        assert!(c.is_merge());
        assert_eq!(c.refs, ["HEAD -> main", "origin/main", "tag: v1"]);
        assert_eq!(c.body, "Body line one\n\nBody line two");
        assert_eq!(c.short_hash(), "aaaa");
    }

    #[test]
    fn short_record_fails() {
        assert!(parse(b"aaaa\x1fbbbb\0").is_err());
    }

    #[test]
    fn live_log_with_pagination_and_follow() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "1\n", "first");
        t.commit_file("a.txt", "2\n", "second");
        t.git(["mv", "a.txt", "b.txt"]);
        t.commit("rename");
        t.commit_file("c.txt", "c\n", "fourth");
        let repo = t.repo();

        let all = log(&repo, &LogOptions::default()).unwrap();
        assert_eq!(all.len(), 4);
        assert_eq!(all[0].subject, "fourth");
        assert_eq!(all[0].refs, ["HEAD -> main"]);
        assert_eq!(all[3].parents.len(), 0);
        assert_eq!(all[0].author_email, "test@example.com");

        let page = log(
            &repo,
            &LogOptions {
                skip: 1,
                limit: Some(2),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            page.iter().map(|c| c.subject.as_str()).collect::<Vec<_>>(),
            ["rename", "second"]
        );

        let followed = log(
            &repo,
            &LogOptions {
                path: Some("b.txt".into()),
                follow: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(followed.len(), 3, "follow crosses the rename");
    }

    #[test]
    fn date_order_interleaves_branches_and_topo_keeps_them_together() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "base\n", "base");
        t.git(["switch", "-q", "-c", "side"]);
        t.commit_file("s.txt", "1\n", "s1");
        t.commit_file("s.txt", "2\n", "s2");
        t.git(["switch", "-q", "main"]);
        t.commit_file("m.txt", "1\n", "m1");
        t.git(["switch", "-q", "side"]);
        t.commit_file("s.txt", "3\n", "s3");
        let subjects = |order| {
            let options = LogOptions {
                all: true,
                order,
                ..Default::default()
            };
            log(&t.repo(), &options)
                .unwrap()
                .into_iter()
                .map(|c| c.subject)
                .collect::<Vec<_>>()
        };
        assert_eq!(subjects(LogOrder::Date), ["s3", "m1", "s2", "s1", "base"]);
        let topo = subjects(LogOrder::Topo);
        let s = |x: &str| topo.iter().position(|y| y == x).unwrap();
        assert_eq!((s("s2"), s("s1")), (s("s3") + 1, s("s3") + 2), "{topo:?}");
        assert_eq!(topo.last().map(String::as_str), Some("base"));
    }

    #[test]
    fn empty_repo_has_empty_log() {
        let t = TempRepo::new();
        assert!(log(&t.repo(), &LogOptions::default()).unwrap().is_empty());
    }

    #[test]
    fn recent_messages_come_from_head_newest_first() {
        let t = TempRepo::new();
        assert!(
            recent_messages(&t.repo(), 5).unwrap().is_empty(),
            "unborn branch"
        );
        t.commit_file("a.txt", "a\n", "first");
        t.write("a.txt", "b\n");
        t.git(["commit", "-qam", "second\n\nwith a body"]);
        for n in 3..=8 {
            t.commit_file("a.txt", &format!("{n}\n"), &format!("commit {n}"));
        }
        let m = recent_messages(&t.repo(), 5).unwrap();
        assert_eq!(m.len(), 5);
        assert_eq!(m[0], "commit 8");
        let all = recent_messages(&t.repo(), 50).unwrap();
        assert!(
            all.contains(&"second\n\nwith a body".to_string()),
            "{all:?}"
        );
    }
}
