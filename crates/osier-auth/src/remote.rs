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

#[cfg(test)]
mod tests {
    use super::*;

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
