//! Account discovery (gh, ssh) and credential plumbing.
//!
//! Design: Osier never runs `gh auth switch`. For an https remote it
//! injects credentials per command through a credential helper that is Osier itself
//! (`osier --credential-helper --account <login> --host <host>`), which asks
//! `gh auth token --user <login>` at the moment git needs it. Tokens never touch disk, argv
//! or the command log.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod gh;
pub mod remote;
pub mod ssh;

use std::path::Path;

pub use gh::GhAccount;
pub use remote::RemoteKind;
pub use ssh::SshStatus;

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("gh is not installed or not on PATH")]
    GhMissing,
    #[error("gh failed: {0}")]
    Gh(String),
    #[error("could not parse gh output: {0}")]
    Parse(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, AuthError>;

/// `-c` config pairs that make git use Osier as the credential helper for one command.
/// The empty first value clears helpers configured elsewhere so only ours answers.
pub fn credential_config(exe: &Path, account: &str, host: &str) -> Vec<(String, String)> {
    let cmd = format!(
        "!{} --credential-helper --account {} --host {}",
        shell_quote(&exe.to_string_lossy()),
        shell_quote(account),
        shell_quote(host)
    );
    vec![
        ("credential.helper".into(), String::new()),
        ("credential.helper".into(), cmd),
    ]
}

/// Environment that routes ssh and git prompts to Osier's askpass window instead of a
/// terminal that does not exist.
pub fn askpass_env(exe: &Path) -> Vec<(String, String)> {
    let exe = exe.to_string_lossy().into_owned();
    let mut env = vec![
        ("SSH_ASKPASS".to_string(), exe.clone()),
        ("SSH_ASKPASS_REQUIRE".to_string(), "force".to_string()),
        ("GIT_ASKPASS".to_string(), exe),
    ];
    // OpenSSH only consults SSH_ASKPASS when DISPLAY is set, even under Wayland.
    if std::env::var_os("DISPLAY").is_none() {
        env.push(("DISPLAY".to_string(), ":0".to_string()));
    }
    env
}

fn shell_quote(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:@".contains(c))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// Parse git's credential protocol (`key=value` lines, blank line ends).
pub fn parse_credential_request(input: &str) -> std::collections::BTreeMap<String, String> {
    input
        .lines()
        .take_while(|l| !l.is_empty())
        .filter_map(|l| {
            l.split_once('=')
                .map(|(k, v)| (k.to_string(), v.to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_config_clears_then_sets_helper() {
        let c = credential_config(Path::new("/opt/osier bin/osier"), "ReyArlena", "github.com");
        assert_eq!(c[0], ("credential.helper".into(), String::new()));
        assert_eq!(
            c[1].1,
            "!'/opt/osier bin/osier' --credential-helper --account ReyArlena --host github.com"
        );
    }

    #[test]
    fn parses_credential_request() {
        let r = parse_credential_request(
            "protocol=https\nhost=github.com\npath=a/b.git\n\nignored=x\n",
        );
        assert_eq!(r.get("host").map(String::as_str), Some("github.com"));
        assert_eq!(r.get("protocol").map(String::as_str), Some("https"));
        assert!(!r.contains_key("ignored"));
    }

    #[test]
    fn askpass_env_names_the_binary() {
        let e = askpass_env(Path::new("/usr/bin/osier"));
        assert!(
            e.iter()
                .any(|(k, v)| k == "SSH_ASKPASS" && v == "/usr/bin/osier")
        );
        assert!(
            e.iter()
                .any(|(k, v)| k == "SSH_ASKPASS_REQUIRE" && v == "force")
        );
    }
}
