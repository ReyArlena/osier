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
    /// `osier --update [--check] [--force]` (see `update.rs`).
    Update {
        check: bool,
        force: bool,
    },
    /// `osier merge <left> [<base>] <right> <output>`: the merge tool in a window of its own
    /// (`git mergetool`). Without a base every difference is a conflict.
    Merge {
        left: PathBuf,
        base: Option<PathBuf>,
        right: PathBuf,
        output: PathBuf,
    },
    /// `osier diff <left> <right>`: a read-only diff window (`git difftool`).
    Diff {
        left: PathBuf,
        right: PathBuf,
    },
    Help,
    /// Arguments that make no sense: the message says why.
    Invalid(String),
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
        Some("merge") => {
            let files = match positionals(&args[1..]) {
                Ok(f) => f,
                Err(why) => return Mode::Invalid(why),
            };
            match files.as_slice() {
                [left, right, output] => Mode::Merge {
                    left: left.clone(),
                    base: None,
                    right: right.clone(),
                    output: output.clone(),
                },
                [left, base, right, output] => Mode::Merge {
                    left: left.clone(),
                    base: Some(base.clone()),
                    right: right.clone(),
                    output: output.clone(),
                },
                _ => Mode::Invalid(
                    "merge takes <left> [<base>] <right> <output>: three or four files".into(),
                ),
            }
        }
        Some("diff") => {
            let files = match positionals(&args[1..]) {
                Ok(f) => f,
                Err(why) => return Mode::Invalid(why),
            };
            match files.as_slice() {
                [left, right] => Mode::Diff {
                    left: left.clone(),
                    right: right.clone(),
                },
                _ => Mode::Invalid("diff takes two files: <left> <right>".into()),
            }
        }
        Some("--version" | "-V") => Mode::Version {
            verbose: args.iter().skip(1).any(|a| a == "--verbose" || a == "-v"),
        },
        Some("--help" | "-h") => Mode::Help,
        Some("--update") => {
            let (mut check, mut force) = (false, false);
            for a in &args[1..] {
                match a.as_str() {
                    "--check" => check = true,
                    "--force" => force = true,
                    other => return Mode::Invalid(format!("--update does not take {other}")),
                }
            }
            Mode::Update { check, force }
        }
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
            let mut new_instance = false;
            let mut path: Option<PathBuf> = None;
            let mut only_paths = false;
            for a in &args {
                if !only_paths && a == "--" {
                    only_paths = true;
                } else if !only_paths && (a == "--new-instance" || a == "-n") {
                    new_instance = true;
                } else if !only_paths && a.starts_with('-') && a.len() > 1 {
                    return Mode::Invalid(format!(
                        "unknown option {a} (a path starting with - goes after --)"
                    ));
                } else if path.is_some() {
                    return Mode::Invalid("open one path at a time".into());
                } else {
                    path = Some(PathBuf::from(a));
                }
            }
            Mode::Gui { path, new_instance }
        }
    }
}

/// File arguments of `merge` and `diff`; `--` ends options so a file may start with `-`.
fn positionals(args: &[String]) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    let mut only_paths = false;
    for a in args {
        if !only_paths && a == "--" {
            only_paths = true;
        } else if !only_paths && a.starts_with('-') && a.len() > 1 {
            return Err(format!(
                "unknown option {a} (a file starting with - goes after --)"
            ));
        } else {
            out.push(PathBuf::from(a));
        }
    }
    Ok(out)
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

pub const HELP: &str = "\
osier [options] [path]
osier merge <left> [<base>] <right> <output>
osier diff <left> <right>

  path                  a repository (or a folder inside one); without one, Osier reopens
                        the last session. A running Osier opens it and comes to the front.
  merge                 merge two versions of a file in a window of its own and write the
                        result to <output>. <base> is the version both started from; without
                        it every difference is a conflict. Exits 0 after Apply, 1 on Cancel.
                        Works as git's mergetool (see the README).
  diff                  compare two files in a window of its own (git's difftool).

  -n, --new-instance    start a separate window instead
      --update          install the latest release over this Osier (install-script installs)
      --update --check  only report whether an update exists (exit code 10 when one does)
      --update --force  reinstall, or replace a development or newer build
  -V, --version         print the version (--verbose adds git, display, install and paths)
  -h, --help            this help
      --                end of options: a path starting with - goes after it

Used by git and ssh, not typed by hand: --credential-helper, --askpass, --gh-api.";

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
        assert_eq!(p(&["-n", "/tmp/x"]), gui(Some("/tmp/x"), true));
        assert_eq!(p(&["--", "-odd"]), gui(Some("-odd"), false));
        assert_eq!(
            p(&["--update"]),
            Mode::Update {
                check: false,
                force: false
            }
        );
        assert_eq!(
            p(&["--update", "--check"]),
            Mode::Update {
                check: true,
                force: false
            }
        );
        assert_eq!(
            p(&["--update", "--force"]),
            Mode::Update {
                check: false,
                force: true
            }
        );
        assert!(matches!(p(&["--update", "now"]), Mode::Invalid(_)));
        assert!(matches!(p(&["--udpate"]), Mode::Invalid(_)));
        assert!(matches!(p(&["/a", "/b"]), Mode::Invalid(_)));
        assert_eq!(
            p(&["merge", "l", "b", "r", "out"]),
            Mode::Merge {
                left: "l".into(),
                base: Some("b".into()),
                right: "r".into(),
                output: "out".into()
            }
        );
        assert_eq!(
            p(&["merge", "l", "r", "out"]),
            Mode::Merge {
                left: "l".into(),
                base: None,
                right: "r".into(),
                output: "out".into()
            }
        );
        assert!(matches!(p(&["merge", "l", "r"]), Mode::Invalid(_)));
        assert!(matches!(
            p(&["merge", "--x", "l", "r", "o"]),
            Mode::Invalid(_)
        ));
        assert_eq!(
            p(&["merge", "--", "-l", "r", "o"]),
            Mode::Merge {
                left: "-l".into(),
                base: None,
                right: "r".into(),
                output: "o".into()
            }
        );
        assert_eq!(
            p(&["diff", "a", "b"]),
            Mode::Diff {
                left: "a".into(),
                right: "b".into()
            }
        );
        assert!(matches!(p(&["diff", "a"]), Mode::Invalid(_)));
        // A folder called merge still opens with a path that is not the bare word.
        assert_eq!(p(&["./merge"]), gui(Some("./merge"), false));
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
