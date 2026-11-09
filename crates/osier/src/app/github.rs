//! "Open on GitHub": browser addresses for a project and its commits.

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

    fn open_github(&mut self, url: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        match url {
            Some(url) => cx.open_url(&url),
            None => window.push_notification("This project has no GitHub remote.", cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::repo_url;
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
}
