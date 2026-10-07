//! What kind of authentication a remote URL needs.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteKind {
    /// `https://host/...`: credential helper territory.
    Https {
        host: String,
    },
    /// `git@host:...` or `ssh://host/...`: ssh keys and agent.
    Ssh {
        host: String,
    },
    /// Local path or `file://`.
    Local,
    Other,
}

impl RemoteKind {
    pub fn host(&self) -> Option<&str> {
        match self {
            RemoteKind::Https { host } | RemoteKind::Ssh { host } => Some(host),
            _ => None,
        }
    }
}

pub fn classify_url(url: &str) -> RemoteKind {
    let url = url.trim();
    if let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    {
        let host = rest.split(['/', ':']).next().unwrap_or("");
        let host = host.rsplit('@').next().unwrap_or(host);
        return RemoteKind::Https {
            host: host.to_string(),
        };
    }
    if let Some(rest) = url.strip_prefix("ssh://") {
        let hostport = rest.split('/').next().unwrap_or("");
        let host = hostport.rsplit('@').next().unwrap_or(hostport);
        let host = host.split(':').next().unwrap_or(host);
        return RemoteKind::Ssh {
            host: host.to_string(),
        };
    }
    if url.starts_with('/') || url.starts_with("file://") || url.starts_with('.') {
        return RemoteKind::Local;
    }
    // scp-like: user@host:path (no scheme, a colon before any slash)
    if let Some((before, _)) = url.split_once(':')
        && !before.contains('/')
    {
        let host = before.rsplit('@').next().unwrap_or(before);
        return RemoteKind::Ssh {
            host: host.to_string(),
        };
    }
    RemoteKind::Other
}

/// Browser address of a GitHub repository from one of its remote URLs, or `None` when the
/// remote is not on github.com.
pub fn github_web_url(url: &str) -> Option<String> {
    let host = classify_url(url).host()?.to_ascii_lowercase();
    if !matches!(
        host.as_str(),
        "github.com" | "www.github.com" | "ssh.github.com"
    ) {
        return None;
    }
    let url = url.trim();
    let path = match url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .or_else(|| url.strip_prefix("ssh://"))
    {
        Some(rest) => rest.split_once('/')?.1,
        None => url.split_once(':')?.1,
    };
    let mut parts = path.split('/').filter(|p| !p.is_empty());
    let owner = parts.next()?;
    let repo = parts.next()?;
    let repo = repo.strip_suffix(".git").unwrap_or(repo);
    if repo.is_empty() {
        return None;
    }
    Some(format!("https://github.com/{owner}/{repo}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_web_urls_from_remote_urls() {
        let want = Some("https://github.com/octo/repo".to_string());
        for url in [
            "https://github.com/octo/repo.git",
            "https://github.com/octo/repo",
            "https://user@github.com/octo/repo.git/",
            "git@github.com:octo/repo.git",
            "ssh://git@github.com/octo/repo.git",
            "ssh://git@ssh.github.com:443/octo/repo.git",
        ] {
            assert_eq!(github_web_url(url), want, "{url}");
        }
        for url in [
            "https://gitlab.com/octo/repo.git",
            "git@example.org:octo/repo.git",
            "/srv/git/repo.git",
            "https://github.com/octo",
        ] {
            assert_eq!(github_web_url(url), None, "{url}");
        }
    }

    #[test]
    fn classifies_common_urls() {
        assert_eq!(
            classify_url("https://github.com/a/b.git"),
            RemoteKind::Https {
                host: "github.com".into()
            }
        );
        assert_eq!(
            classify_url("https://user@gitlab.example.com:8443/a/b"),
            RemoteKind::Https {
                host: "gitlab.example.com".into()
            }
        );
        assert_eq!(
            classify_url("git@github.com:a/b.git"),
            RemoteKind::Ssh {
                host: "github.com".into()
            }
        );
        assert_eq!(
            classify_url("ssh://git@github.com:22/a/b.git"),
            RemoteKind::Ssh {
                host: "github.com".into()
            }
        );
        assert_eq!(classify_url("/tmp/repo.git"), RemoteKind::Local);
        assert_eq!(classify_url("file:///tmp/repo.git"), RemoteKind::Local);
        assert_eq!(classify_url("../sibling"), RemoteKind::Local);
        assert_eq!(classify_url("weird"), RemoteKind::Other);
    }
}
