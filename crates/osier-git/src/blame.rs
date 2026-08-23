//! Line annotations via `git blame --line-porcelain`.

use std::collections::HashMap;

use crate::error::{GitError, Result};
use crate::repo::Repository;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlameLine {
    /// 1-based line number in the blamed (current) file.
    pub line_no: u32,
    /// Line number in the commit that introduced it.
    pub orig_line_no: u32,
    pub hash: String,
    pub author_name: String,
    pub author_email: String,
    pub authored_at: i64,
    pub summary: String,
    /// Path of the file in that commit (differs across renames).
    pub filename: String,
    /// The commit is a boundary (root or outside the requested range).
    pub boundary: bool,
    pub text: String,
}

impl BlameLine {
    /// Uncommitted lines come from the all-zero hash.
    pub fn is_uncommitted(&self) -> bool {
        self.hash.chars().all(|c| c == '0')
    }
}

/// Blame `path` at `rev` (working tree when `None`).
pub fn blame(repo: &Repository, path: &str, rev: Option<&str>) -> Result<Vec<BlameLine>> {
    let mut args = vec!["blame", "--line-porcelain"];
    if let Some(rev) = rev {
        args.push(rev);
    }
    args.push("--");
    args.push(path);
    let out = repo.git(args)?;
    parse(out.stdout_str()?)
}

pub fn parse(text: &str) -> Result<Vec<BlameLine>> {
    // Header keys repeat per line with --line-porcelain, but `author` etc. are only guaranteed
    // on first sight in plain porcelain; cache by hash to be safe for both.
    let mut cache: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut lines_out = Vec::new();
    let mut lines = text.lines();
    while let Some(head) = lines.next() {
        if head.is_empty() {
            continue;
        }
        let mut parts = head.split(' ');
        let hash = parts
            .next()
            .ok_or_else(|| GitError::parse("blame", head.to_string()))?
            .to_string();
        let orig: u32 = parts
            .next()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| GitError::parse("blame", head.to_string()))?;
        let final_no: u32 = parts
            .next()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| GitError::parse("blame", head.to_string()))?;
        let mut meta = cache.get(&hash).cloned().unwrap_or_default();
        let mut boundary = meta.contains_key("boundary");
        let mut content: Option<String> = None;
        for l in lines.by_ref() {
            if let Some(t) = l.strip_prefix('\t') {
                content = Some(t.to_string());
                break;
            }
            if l == "boundary" {
                boundary = true;
                meta.insert("boundary".into(), String::new());
                continue;
            }
            if let Some((k, v)) = l.split_once(' ') {
                meta.insert(k.to_string(), v.to_string());
            }
        }
        let text = content.ok_or_else(|| GitError::parse("blame", "missing content line"))?;
        let get = |k: &str| meta.get(k).cloned().unwrap_or_default();
        lines_out.push(BlameLine {
            line_no: final_no,
            orig_line_no: orig,
            author_name: get("author"),
            author_email: get("author-mail").trim_matches(['<', '>']).to_string(),
            authored_at: get("author-time").parse().unwrap_or(0),
            summary: get("summary"),
            filename: get("filename"),
            boundary,
            text,
            hash: hash.clone(),
        });
        cache.insert(hash, meta);
    }
    Ok(lines_out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempRepo;

    #[test]
    fn live_blame_tracks_commits_and_uncommitted_lines() {
        let t = TempRepo::new();
        let first = t.commit_file("f.txt", "one\ntwo\n", "first");
        let second = t.commit_file("f.txt", "one\nTWO\nthree\n", "second");
        t.write("f.txt", "one\nTWO\nthree\nfour\n");
        let b = blame(&t.repo(), "f.txt", None).unwrap();
        assert_eq!(b.len(), 4);
        assert_eq!(b[0].hash, first);
        assert!(b[0].boundary, "root commit is a boundary");
        assert_eq!(b[0].summary, "first");
        assert_eq!(b[0].author_email, "test@example.com");
        assert_eq!(b[1].hash, second);
        assert_eq!(b[1].text, "TWO");
        assert_eq!((b[2].line_no, b[2].orig_line_no), (3, 3));
        assert!(b[3].is_uncommitted());

        let at_first = blame(&t.repo(), "f.txt", Some(&first)).unwrap();
        assert_eq!(at_first.len(), 2);
    }

    #[test]
    fn missing_content_fails_closed() {
        assert!(parse("abcd 1 1 1\nauthor X\n").is_err());
    }
}
