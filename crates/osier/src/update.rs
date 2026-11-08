//! `osier --update`: install the latest release over this binary by running the install
//! script, the same way the README's one-liner does.
//!
//! - Only for install-script installs ([`InstallRoute::Script`]); packages, Flatpak, cargo
//!   and development builds are refused with what to do instead.
//! - The latest release comes from `github.com/<repo>/releases/latest`, which redirects to
//!   the newest published (non-draft, non-pre-release) release without using the rate
//!   limited API; `git ls-remote` tags are the fallback, as in the installer.
//! - The installer is fetched from that release's tag and told to install exactly that tag,
//!   into the folder this binary lives in (`OSIER_PREFIX`).
//! - `--check` only reports (exit 10 when an update is available). `--force` reinstalls the
//!   same version or replaces a development or newer build. `OSIER_VERSION=v1.2.3` installs
//!   that version; `OSIER_FROM_SOURCE=1` passes through to the installer.

use std::path::Path;
use std::process::{Command, Stdio};

use crate::install_route::InstallRoute;
use crate::runtime_info::VERSION;

const REPO: &str = "ReyArlena/osier";

/// Exit code of `--check` when a newer release exists.
pub const UPDATE_AVAILABLE: i32 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// The latest release is newer.
    Update,
    UpToDate,
    /// This build is not a release (git build metadata, `+...`).
    DevBuild,
    /// This release is newer than the latest published one.
    Newer,
}

fn triple(v: &str) -> Option<(u64, u64, u64)> {
    let v = v.strip_prefix('v').unwrap_or(v);
    let core = v.split(['+', '-']).next()?;
    let mut it = core.split('.').map(|p| p.parse::<u64>().ok());
    let t = (it.next()??, it.next()??, it.next()??);
    it.next().is_none().then_some(t)
}

/// Compare this build (`0.1.0`, `0.1.0+12.g4a1b2c3`) with the latest release tag.
pub fn decide(current: &str, latest: &str) -> Option<Decision> {
    if current.contains('+') {
        return Some(Decision::DevBuild);
    }
    let (c, l) = (triple(current)?, triple(latest)?);
    Some(match c.cmp(&l) {
        std::cmp::Ordering::Less => Decision::Update,
        std::cmp::Ordering::Equal => Decision::UpToDate,
        std::cmp::Ordering::Greater => Decision::Newer,
    })
}

/// `https://github.com/o/r/releases/tag/v1.2.3` (or a `Location:` header line) to `v1.2.3`;
/// only stable `vX.Y.Z` tags count.
pub fn tag_from_url(url: &str) -> Option<String> {
    let tail = url.trim().rsplit_once("/releases/tag/")?.1;
    let tag = tail.split(['?', '#', '\r', '\n', ' ']).next()?;
    (tag.starts_with('v') && !tag.contains('-') && triple(tag).is_some()).then(|| tag.to_string())
}

/// The newest stable tag among `git ls-remote --tags` lines.
pub fn newest_tag(ls_remote: &str) -> Option<String> {
    ls_remote
        .lines()
        .filter_map(|l| l.rsplit_once("refs/tags/").map(|(_, t)| t.trim()))
        .filter(|t| t.starts_with('v') && !t.contains('-'))
        .filter_map(|t| triple(t).map(|n| (n, t.to_string())))
        .max_by_key(|(n, _)| *n)
        .map(|(_, t)| t)
}

fn has_cmd(cmd: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|d| d.join(cmd).is_file()))
}

fn output(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        // Never wait for a username or password (git asks for one on private or missing repos).
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .ok()?;
    // wget --spider prints headers on stderr and may exit non-zero on the redirect.
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Some(text)
}

/// The latest published release tag.
fn latest_release() -> Result<String, String> {
    let url = format!("https://github.com/{REPO}/releases/latest");
    if has_cmd("curl")
        && let Some(final_url) = output(
            "curl",
            &["-fsSLI", "-o", "/dev/null", "-w", "%{url_effective}", &url],
        )
        && let Some(tag) = tag_from_url(&final_url)
    {
        return Ok(tag);
    }
    if has_cmd("wget")
        && let Some(headers) = output("wget", &["-q", "-S", "--spider", "--max-redirect=0", &url])
        && let Some(tag) = headers
            .lines()
            .filter(|l| l.trim_start().to_ascii_lowercase().starts_with("location:"))
            .find_map(tag_from_url)
    {
        return Ok(tag);
    }
    let repo_url = format!("https://github.com/{REPO}.git");
    if let Some(tags) = output("git", &["ls-remote", "--tags", "--refs", &repo_url])
        && let Some(tag) = newest_tag(&tags)
    {
        return Ok(tag);
    }
    Err("could not find the latest release (no release yet, or GitHub is unreachable)".into())
}

fn download(url: &str, dest: &Path) -> bool {
    let dest = dest.to_string_lossy();
    if has_cmd("curl") {
        return Command::new("curl")
            .args(["-fsSL", "-o", &dest, url])
            .status()
            .is_ok_and(|s| s.success());
    }
    has_cmd("wget")
        && Command::new("wget")
            .args(["-qO", &dest, url])
            .status()
            .is_ok_and(|s| s.success())
}

/// Run `osier --update [--check] [--force]`; returns the process exit code.
pub fn run(check: bool, force: bool) -> i32 {
    let route = InstallRoute::detect();
    let refusal = route.update_refusal();
    if let Some(why) = &refusal
        && !check
    {
        eprintln!("{why}");
        return 2;
    }
    let pinned = std::env::var("OSIER_VERSION")
        .ok()
        .filter(|v| !v.trim().is_empty());
    let target = match &pinned {
        Some(v) => v.clone(),
        None => match latest_release() {
            Ok(tag) => tag,
            Err(e) => {
                eprintln!("osier --update: {e}");
                return 1;
            }
        },
    };
    println!(
        "Osier v{VERSION} ({}); {} {target}.",
        route.label(),
        if pinned.is_some() {
            "requested"
        } else {
            "latest release"
        }
    );
    let decision = match &pinned {
        Some(_) => Decision::Update,
        None => decide(VERSION, &target).unwrap_or(Decision::Update),
    };

    if check {
        let code = match decision {
            Decision::Update => {
                println!("Update available: {target}.");
                UPDATE_AVAILABLE
            }
            Decision::UpToDate => {
                println!("Up to date.");
                0
            }
            Decision::DevBuild => {
                println!("This is a development build, not a release.");
                0
            }
            Decision::Newer => {
                println!("This build is newer than the latest release.");
                0
            }
        };
        if let Some(why) = refusal {
            println!("{why}");
        }
        return code;
    }

    if !force {
        let hold = match decision {
            Decision::Update => None,
            Decision::UpToDate => Some(format!("Already up to date ({target}).")),
            Decision::DevBuild => Some(format!(
                "v{VERSION} is a development build; updating would replace it with {target}."
            )),
            Decision::Newer => Some(format!(
                "v{VERSION} is newer than the latest release ({target}); updating would downgrade."
            )),
        };
        if let Some(msg) = hold {
            println!("{msg} Run `osier --update --force` to install {target} anyway.");
            return 0;
        }
    }

    let InstallRoute::Script { bin_dir } = &route else {
        return 2;
    };
    let Some(prefix) = bin_dir.parent() else {
        eprintln!(
            "osier --update: cannot tell the install prefix of {}",
            bin_dir.display()
        );
        return 1;
    };
    let script = std::env::temp_dir().join(format!("osier-install-{}.sh", std::process::id()));
    let script_url = format!("https://raw.githubusercontent.com/{REPO}/{target}/install.sh");
    if !has_cmd("curl") && !has_cmd("wget") {
        eprintln!("osier --update: curl or wget is needed to download the installer");
        return 1;
    }
    if !download(&script_url, &script) {
        eprintln!(
            "osier --update: could not download {script_url} (is {target} a published release?)"
        );
        let _ = std::fs::remove_file(&script);
        return 1;
    }
    let status = Command::new("sh")
        .arg(&script)
        .env("OSIER_PREFIX", prefix)
        .env("OSIER_VERSION", &target)
        .status();
    let _ = std::fs::remove_file(&script);
    match status {
        Ok(s) if s.success() => {
            println!(
                "Updated to {target}. An Osier window that is already open keeps running the old \
                 version until you restart it (and `osier <path>` opens paths in that window)."
            );
            0
        }
        Ok(s) => s.code().unwrap_or(1),
        Err(e) => {
            eprintln!("osier --update: could not run the installer: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Decision, decide, newest_tag, tag_from_url};

    #[test]
    fn decides_against_the_latest_release() {
        assert_eq!(decide("0.1.0", "v0.2.0"), Some(Decision::Update));
        assert_eq!(decide("0.9.0", "v0.10.0"), Some(Decision::Update));
        assert_eq!(decide("0.2.0", "v0.2.0"), Some(Decision::UpToDate));
        assert_eq!(decide("0.3.0", "v0.2.0"), Some(Decision::Newer));
        assert_eq!(
            decide("0.2.0+12.g4a1b2c3", "v0.2.0"),
            Some(Decision::DevBuild)
        );
        assert_eq!(
            decide("0.2.0+g4a1b2c3-dirty", "v0.1.0"),
            Some(Decision::DevBuild)
        );
        assert_eq!(decide("0.2.0", "nightly"), None);
    }

    #[test]
    fn reads_tags_from_redirects_and_ls_remote() {
        assert_eq!(
            tag_from_url("https://github.com/ReyArlena/osier/releases/tag/v1.2.3").as_deref(),
            Some("v1.2.3")
        );
        assert_eq!(
            tag_from_url("location: https://github.com/o/r/releases/tag/v0.10.0\r").as_deref(),
            Some("v0.10.0")
        );
        assert_eq!(tag_from_url("https://github.com/o/r/releases"), None);
        assert_eq!(
            tag_from_url("https://github.com/o/r/releases/tag/v1.0.0-rc.1"),
            None
        );
        let ls = "a\trefs/tags/v0.9.2\nb\trefs/tags/v0.10.0\nc\trefs/tags/v1.0.0-rc.1\nd\trefs/tags/nightly\n";
        assert_eq!(newest_tag(ls).as_deref(), Some("v0.10.0"));
        assert_eq!(newest_tag(""), None);
    }
}
