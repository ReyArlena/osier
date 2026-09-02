//! GitHub CLI accounts.

use std::process::Command;

use serde::Deserialize;

use crate::{AuthError, Result};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GhAccount {
    pub host: String,
    pub login: String,
    #[serde(default)]
    pub active: bool,
    #[serde(default, rename = "gitProtocol")]
    pub git_protocol: String,
    #[serde(default)]
    pub state: String,
}

#[derive(Debug, Deserialize)]
struct StatusJson {
    hosts: std::collections::BTreeMap<String, Vec<GhAccount>>,
}

/// Parse `gh auth status --json hosts`.
pub fn parse_status(json: &str) -> Result<Vec<GhAccount>> {
    let parsed: StatusJson =
        serde_json::from_str(json).map_err(|e| AuthError::Parse(e.to_string()))?;
    let mut accounts: Vec<GhAccount> = parsed
        .hosts
        .into_values()
        .flatten()
        .filter(|a| a.state.is_empty() || a.state == "success")
        .collect();
    accounts.sort_by(|a, b| a.host.cmp(&b.host).then(a.login.cmp(&b.login)));
    Ok(accounts)
}

/// Logged-in gh accounts across hosts. No tokens are read.
pub fn accounts() -> Result<Vec<GhAccount>> {
    let out = Command::new("gh")
        .args(["auth", "status", "--json", "hosts"])
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                AuthError::GhMissing
            } else {
                AuthError::Io(e)
            }
        })?;
    // gh exits 1 when some account is logged out but still prints the JSON.
    let text = String::from_utf8_lossy(&out.stdout);
    if text.trim().is_empty() {
        return Err(AuthError::Gh(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    parse_status(&text)
}

/// The token for `login` on `host`, straight from gh. Only the credential helper calls this.
pub fn token(login: &str, host: &str) -> Result<String> {
    let out = Command::new("gh")
        .args(["auth", "token", "--user", login, "--hostname", host])
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                AuthError::GhMissing
            } else {
                AuthError::Io(e)
            }
        })?;
    if !out.status.success() {
        return Err(AuthError::Gh(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    let token = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if token.is_empty() {
        return Err(AuthError::Gh(format!("no token for {login}@{host}")));
    }
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_two_accounts() {
        let json = r#"{"hosts":{"github.com":[
            {"state":"success","active":true,"host":"github.com","login":"alice","tokenSource":"x","scopes":"repo","gitProtocol":"https"},
            {"state":"success","active":false,"host":"github.com","login":"bob","tokenSource":"x","scopes":"repo","gitProtocol":"ssh"}
        ]}}"#;
        let a = parse_status(json).unwrap();
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].login, "alice");
        assert!(a[0].active);
        assert_eq!(a[1].git_protocol, "ssh");
    }

    #[test]
    fn empty_hosts_is_empty() {
        assert!(parse_status(r#"{"hosts":{}}"#).unwrap().is_empty());
        assert!(parse_status("nope").is_err());
    }
}
