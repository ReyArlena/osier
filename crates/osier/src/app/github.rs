//! "Open on GitHub": browser addresses for a project, its commits, and the pull requests its
//! commits and branches point at (from what git already knows, no API call).

use std::path::Path;

use gpui_kit::component::WindowExt as _;
use gpui_kit::*;
use osier_git::refs::Remote;
use osier_git::repo::Repository;

use crate::app::Osier;

/// The GitHub page of the remote a project tracks: the upstream's remote, else `origin`,
/// else the first remote on GitHub.
pub(crate) fn repo_url(remotes: &[Remote], upstream: Option<&str>) -> Option<String> {
    let web = |r: &Remote| osier_auth::remote::github_web_url(&r.fetch_url);
    let tracked = upstream.and_then(|u| {
        remotes
            .iter()
            .filter(|r| u.starts_with(&format!("{}/", r.name)))
            .max_by_key(|r| r.name.len())
    });
    tracked
        .and_then(web)
        .or_else(|| remotes.iter().find(|r| r.name == "origin").and_then(web))
        .or_else(|| remotes.iter().find_map(web))
}

/// The pull request a commit subject names: GitHub's merge commits ("Merge pull request #93
/// from ...") and squash merges ("subject (#93)").
pub(crate) fn pull_request_number(subject: &str) -> Option<u32> {
    let subject = subject.trim();
    if let Some(rest) = subject.strip_prefix("Merge pull request #") {
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        return digits.parse().ok();
    }
    let (_, tail) = subject.strip_suffix(')')?.rsplit_once("(#")?;
    (!tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit()))
        .then(|| tail.parse().ok())
        .flatten()
}

/// GitHub's pull request list filtered to one branch, open and closed: what git can offer
/// without asking the API whether a pull request exists.
pub(crate) fn branch_pulls_url(repo_web: &str, branch: &str) -> String {
    format!(
        "{repo_web}/pulls?q={}",
        query_encode(&format!("is:pr head:{branch}"))
    )
}

/// `application/x-www-form-urlencoded` for one query value.
fn query_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// `origin/feature/x` split into its remote and `feature/x` (the longest remote name wins,
/// so a remote called `a` does not claim `a/b`'s branches).
pub(crate) fn split_remote_branch<'a>(
    remotes: &'a [Remote],
    name: &'a str,
) -> Option<(&'a Remote, &'a str)> {
    remotes
        .iter()
        .filter_map(|r| name.strip_prefix(&format!("{}/", r.name)).map(|b| (r, b)))
        .max_by_key(|(r, _)| r.name.len())
}

/// The remote branches among a commit's decorations (`HEAD -> main`, `origin/main`,
/// `tag: v1`): the ones a pull request can be looked up for.
pub(crate) fn remote_branch_refs(refs: &[String], remotes: &[Remote]) -> Vec<String> {
    refs.iter()
        .map(|r| r.strip_prefix("HEAD -> ").unwrap_or(r))
        .filter(|r| !r.starts_with("tag: ") && split_remote_branch(remotes, r).is_some())
        .map(str::to_string)
        .collect()
}

impl Osier {
    /// GitHub page of the active project, when it has a GitHub remote.
    pub(crate) fn active_github_url(&self) -> Option<String> {
        let snapshot = self.active_project()?.snapshot.as_ref()?;
        repo_url(
            &snapshot.remotes,
            snapshot.status.branch.upstream.as_deref(),
        )
    }

    /// Open a project's GitHub page. Open projects use their snapshot; others read their
    /// remotes first.
    pub(crate) fn open_project_on_github(
        &mut self,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let known = self
            .projects
            .iter()
            .find(|p| p.path == path)
            .and_then(|p| p.snapshot.as_ref())
            .map(|s| repo_url(&s.remotes, s.status.branch.upstream.as_deref()));
        if let Some(url) = known {
            self.open_github(url, window, cx);
            return;
        }
        let runner = self.runner.clone();
        let path = path.to_path_buf();
        let task = cx.background_spawn(async move {
            let repo = Repository::discover(runner, &path).ok()?;
            let remotes = osier_git::refs::remotes(&repo).ok()?;
            repo_url(&remotes, None)
        });
        cx.spawn_in(window, async move |this, cx| {
            let url = task.await;
            let _ = this.update_in(cx, |this, window, cx| this.open_github(url, window, cx));
        })
        .detach();
    }

    /// Open one commit of the active project on GitHub.
    pub(crate) fn open_commit_on_github(
        &mut self,
        hash: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let url = self
            .active_github_url()
            .map(|u| format!("{u}/commit/{hash}"));
        self.open_github(url, window, cx);
    }

    /// Open the pull request a commit names, or failing that the pull requests of a remote
    /// branch the commit carries (the keyboard path has no menu to pick from).
    pub(crate) fn open_commit_pull_request(
        &mut self,
        hash: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(number) = self
            .commit_subject(&hash)
            .as_deref()
            .and_then(pull_request_number)
        {
            let url = self
                .active_github_url()
                .map(|u| format!("{u}/pull/{number}"));
            return self.open_github(url, window, cx);
        }
        let branch = self
            .active_project()
            .and_then(|p| p.snapshot.as_ref())
            .and_then(|s| {
                let commit = s.commits.iter().find(|c| c.hash == hash)?;
                remote_branch_refs(&commit.refs, &s.remotes)
                    .into_iter()
                    .next()
            });
        match branch {
            Some(branch) => self.open_branch_pulls_on_github(branch, true, window, cx),
            None => window.push_notification(
                "This commit does not name a pull request and is not the tip of a remote branch.",
                cx,
            ),
        }
    }

    /// GitHub's pull requests for a remote branch (`origin/feature`), or for the upstream of a
    /// local one.
    pub(crate) fn open_branch_pulls_on_github(
        &mut self,
        name: String,
        remote: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(snapshot) = self.active_project().and_then(|p| p.snapshot.as_ref()) else {
            return;
        };
        let upstream = if remote {
            Some(name)
        } else {
            snapshot
                .branches
                .iter()
                .find(|b| !b.is_remote && b.name == name)
                .and_then(|b| b.upstream.clone())
        };
        let Some(upstream) = upstream else {
            return window.push_notification(
                "This branch has no upstream, so GitHub has no pull request for it.",
                cx,
            );
        };
        let url = split_remote_branch(&snapshot.remotes, &upstream).and_then(|(r, branch)| {
            osier_auth::remote::github_web_url(&r.fetch_url)
                .map(|web| branch_pulls_url(&web, branch))
        });
        self.open_github(url, window, cx);
    }

    fn open_github(&mut self, url: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        match url {
            Some(url) => cx.open_url(&url),
            None => window.push_notification("This project has no GitHub remote.", cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        branch_pulls_url, pull_request_number, remote_branch_refs, repo_url, split_remote_branch,
    };
    use osier_git::refs::Remote;

    fn remote(name: &str, url: &str) -> Remote {
        Remote {
            name: name.into(),
            fetch_url: url.into(),
            push_url: None,
        }
    }

    #[test]
    fn prefers_the_tracked_remote_then_origin_then_any_github_remote() {
        let remotes = vec![
            remote("origin", "git@github.com:me/fork.git"),
            remote("upstream", "https://github.com/them/repo.git"),
            remote("lab", "git@gitlab.com:me/repo.git"),
        ];
        assert_eq!(
            repo_url(&remotes, Some("upstream/main")).as_deref(),
            Some("https://github.com/them/repo")
        );
        assert_eq!(
            repo_url(&remotes, None).as_deref(),
            Some("https://github.com/me/fork")
        );
        assert_eq!(
            repo_url(&remotes, Some("lab/main")).as_deref(),
            Some("https://github.com/me/fork"),
            "a non-GitHub upstream falls back to origin"
        );
        let only_lab = vec![remote("lab", "git@gitlab.com:me/repo.git")];
        assert_eq!(repo_url(&only_lab, None), None);
        let other = vec![remote("mine", "https://github.com/me/x")];
        assert_eq!(
            repo_url(&other, None).as_deref(),
            Some("https://github.com/me/x")
        );
    }

    #[test]
    fn pull_request_numbers_come_from_merge_and_squash_subjects() {
        assert_eq!(
            pull_request_number("Merge pull request #93 from ReyArlena/three-way"),
            Some(93)
        );
        assert_eq!(
            pull_request_number("feat(log): branch filter (#120)"),
            Some(120)
        );
        assert_eq!(pull_request_number("fix: handle empty repo"), None);
        assert_eq!(pull_request_number("Merge pull request #x from a/b"), None);
        assert_eq!(pull_request_number("mention (#12) in the middle"), None);
        assert_eq!(pull_request_number("empty (#)"), None);
    }

    #[test]
    fn branch_pull_requests_url_is_a_filtered_pulls_page() {
        assert_eq!(
            branch_pulls_url("https://github.com/octo/repo", "feature/x y"),
            "https://github.com/octo/repo/pulls?q=is%3Apr+head%3Afeature%2Fx+y"
        );
    }

    #[test]
    fn remote_branches_split_on_the_longest_remote_name() {
        let remotes = vec![
            remote("a", "https://github.com/me/a"),
            remote("a/b", "https://github.com/me/ab"),
        ];
        let (r, branch) = split_remote_branch(&remotes, "a/b/feature").expect("split");
        assert_eq!((r.name.as_str(), branch), ("a/b", "feature"));
        assert_eq!(split_remote_branch(&remotes, "other/x"), None);
        let refs: Vec<String> = ["HEAD -> main", "a/main", "tag: v1", "feature/local"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            remote_branch_refs(&refs, &remotes),
            vec!["a/main".to_string()]
        );
    }
}
