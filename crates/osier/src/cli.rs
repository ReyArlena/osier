//! Command line modes. Besides the GUI, the binary serves as git's credential helper and as
//! an askpass program, so prompts never need a terminal.

use std::io::{Read, Write};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Gui(Option<PathBuf>),
    /// `osier --credential-helper --account <login> --host <host> <get|store|erase>`
    CredentialHelper {
        account: String,
        host: String,
        operation: String,
    },
    /// `osier --askpass "<prompt>"` (ssh and git call it this way)
    Askpass(String),
    /// `--version`, with `--verbose` adding runtime details for bug reports.
    Version {
        verbose: bool,
    },
    Help,
}

pub fn parse(args: impl IntoIterator<Item = String>) -> Mode {
    let args: Vec<String> = args.into_iter().collect();
    match args.first().map(String::as_str) {
        Some("--credential-helper") => {
            let mut account = String::new();
            let mut host = String::new();
            let mut operation = String::new();
            let mut it = args[1..].iter();
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--account" => account = it.next().cloned().unwrap_or_default(),
                    "--host" => host = it.next().cloned().unwrap_or_default(),
                    other => operation = other.to_string(),
                }
            }
            Mode::CredentialHelper {
                account,
                host,
                operation,
            }
        }
        Some("--askpass") => Mode::Askpass(args.get(1).cloned().unwrap_or_default()),
        Some("--version" | "-V") => Mode::Version {
            verbose: args.iter().skip(1).any(|a| a == "--verbose" || a == "-v"),
        },
        Some("--help" | "-h") => Mode::Help,
        // ssh runs `$SSH_ASKPASS "<prompt>"` with no flag; a prompt is not a path.
        Some(p)
            if p.ends_with(':')
                || p.ends_with(": ")
                || p.contains("passphrase")
                || p.contains("assword") =>
        {
            Mode::Askpass(p.to_string())
        }
        Some(p) => Mode::Gui(Some(PathBuf::from(p))),
        None => Mode::Gui(None),
    }
}

/// Answer git's credential request from gh. Exit code for the process.
pub fn run_credential_helper(account: &str, host: &str, operation: &str) -> i32 {
    if operation != "get" {
        // store/erase: gh owns the token lifecycle, nothing to do.
        return 0;
    }
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        return 1;
    }
    let request = osier_auth::parse_credential_request(&input);
    let wanted = request.get("host").map(String::as_str).unwrap_or(host);
    if !host.is_empty() && wanted != host {
        // Not our host: say nothing so git moves on to the next helper.
        return 0;
    }
    match osier_auth::gh::token(account, wanted) {
        Ok(token) => {
            let mut out = std::io::stdout().lock();
            let _ = writeln!(out, "username={account}");
            let _ = writeln!(out, "password={token}");
            let _ = writeln!(out);
            0
        }
        Err(e) => {
            eprintln!("osier credential helper: {e}");
            1
        }
    }
}

pub const HELP: &str = "osier [path]              open the GUI (optionally on a repository)
osier --version [--verbose]   (verbose adds git version, display backend and paths)
osier --credential-helper --account <login> --host <host> <get|store|erase>
osier --askpass <prompt>   (also used by ssh and git through SSH_ASKPASS / GIT_ASKPASS)";

#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &[&str]) -> Mode {
        parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn parses_modes() {
        assert_eq!(p(&[]), Mode::Gui(None));
        assert_eq!(p(&["/tmp/x"]), Mode::Gui(Some(PathBuf::from("/tmp/x"))));
        assert_eq!(
            p(&[
                "--credential-helper",
                "--account",
                "alice",
                "--host",
                "github.com",
                "get"
            ]),
            Mode::CredentialHelper {
                account: "alice".into(),
                host: "github.com".into(),
                operation: "get".into()
            }
        );
        assert_eq!(
            p(&["--askpass", "Password:"]),
            Mode::Askpass("Password:".into())
        );
        assert_eq!(
            p(&["Enter passphrase for key '/home/x/.ssh/id_ed25519': "]),
            Mode::Askpass("Enter passphrase for key '/home/x/.ssh/id_ed25519': ".into())
        );
        assert_eq!(p(&["--version"]), Mode::Version { verbose: false });
        assert_eq!(
            p(&["--version", "--verbose"]),
            Mode::Version { verbose: true }
        );
    }
}
