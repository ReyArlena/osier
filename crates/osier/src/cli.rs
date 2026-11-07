//! Command line modes. Besides the GUI, the binary serves as git's credential helper and as
//! an askpass program, so prompts never need a terminal.

use std::io::{Read, Write};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// The GUI, optionally on a repository. Without `--new-instance` a running Osier gets
    /// the path instead (see `instance.rs`).
    Gui {
        path: Option<PathBuf>,
        new_instance: bool,
    },
    /// `osier --credential-helper --account <login> --host <host> <get|store|erase>`
    CredentialHelper {
        account: String,
        host: String,
        operation: String,
    },
    /// `osier --gh-api --account <login> --host <host> -- <gh api args>`: the app's GitHub
    /// calls as one account (`osier_auth::hub`), so tokens stay out of the app.
    GhApi {
        account: String,
        host: String,
        args: Vec<String>,
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
        Some("--gh-api") => {
            let mut account = String::new();
            let mut host = String::new();
            let mut it = args[1..].iter();
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--account" => account = it.next().cloned().unwrap_or_default(),
                    "--host" => host = it.next().cloned().unwrap_or_default(),
                    "--" => break,
                    _ => {}
                }
            }
            Mode::GhApi {
                account,
                host,
                args: it.cloned().collect(),
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
        _ => {
            let new_instance = args.iter().any(|a| a == "--new-instance");
            let path = args
                .iter()
                .find(|a| a.as_str() != "--new-instance")
                .map(PathBuf::from);
            Mode::Gui { path, new_instance }
        }
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

/// Run `gh api` as one account for the app. Exit code for the process.
pub fn run_gh_api(account: &str, host: &str, args: &[String]) -> i32 {
    if account.is_empty() || host.is_empty() {
        eprintln!("osier --gh-api needs --account and --host");
        return 2;
    }
    match osier_auth::hub::run_as(account, host, args) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("osier --gh-api: {e}");
            1
        }
    }
}

pub const HELP: &str =
    "osier [path]              open a repository; a running Osier opens it and comes to the front
osier --new-instance [path]   start a separate window instead
osier --version [--verbose]   (verbose adds git version, display backend and paths)
osier --credential-helper --account <login> --host <host> <get|store|erase>
osier --gh-api --account <login> --host <host> -- <gh api args>   (used by the app)
osier --askpass <prompt>   (also used by ssh and git through SSH_ASKPASS / GIT_ASKPASS)";

#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &[&str]) -> Mode {
        parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn parses_modes() {
        let gui = |path: Option<&str>, new_instance: bool| Mode::Gui {
            path: path.map(PathBuf::from),
            new_instance,
        };
        assert_eq!(p(&[]), gui(None, false));
        assert_eq!(p(&["/tmp/x"]), gui(Some("/tmp/x"), false));
        assert_eq!(p(&["--new-instance"]), gui(None, true));
        assert_eq!(p(&["--new-instance", "/tmp/x"]), gui(Some("/tmp/x"), true));
        assert_eq!(p(&["/tmp/x", "--new-instance"]), gui(Some("/tmp/x"), true));
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

    #[test]
    fn gh_api_mode_keeps_everything_after_the_separator() {
        assert_eq!(
            p(&[
                "--gh-api",
                "--account",
                "alice",
                "--host",
                "github.com",
                "--",
                "--paginate",
                "--host",
                "user/repos"
            ]),
            Mode::GhApi {
                account: "alice".into(),
                host: "github.com".into(),
                args: vec!["--paginate".into(), "--host".into(), "user/repos".into()],
            }
        );
    }
}
