//! Updating Osier: `osier --update` and Settings > About > Updates share these steps.
//!
//! - [`latest_release`]: `github.com/<repo>/releases/latest` redirects to the newest
//!   published (non-draft, non-pre-release) release; a plain web redirect, so the API rate
//!   limit does not apply. The installer uses the same lookup.
//! - [`decide`]: compare this build with it (development builds never count as outdated).
//! - [`install`]: run the release's own `install.sh` for that tag, into the folder this
//!   binary lives in (`OSIER_PREFIX`). Only for install-script installs
//!   ([`InstallRoute::Script`]); the installer is prebuilt-binaries only.
//! - [`restart`]: start the new binary once this process has exited.
//!
//! CLI: `--check` only reports (exit 10 when an update is available); `--force` reinstalls
//! the same version or replaces a development or newer build; `OSIER_VERSION=v1.2.3`
//! installs that release.

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

fn has_cmd(cmd: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|d| d.join(cmd).is_file()))
}

fn output(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    // wget --spider prints headers on stderr and may exit non-zero on the redirect.
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Some(text)
}

/// The latest published release tag (`v0.3.0`).
pub fn latest_release() -> Result<String, String> {
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
    match install(bin_dir, &target, false) {
        Ok(_) => {
            println!(
                "Updated to {target}. An Osier window that is already open keeps running the old \
                 version until you restart it (and `osier <path>` opens paths in that window)."
            );
            0
        }
        Err(e) => {
            eprintln!("osier --update: {e}");
            1
        }
    }
}

/// The release page of `tag`, for "what's new".
pub fn release_url(tag: &str) -> String {
    format!("https://github.com/{REPO}/releases/tag/{tag}")
}

/// Run the install script of release `tag` into the prefix that holds `bin_dir`. With
/// `capture`, its output is returned (for the app) instead of going to the terminal.
pub fn install(bin_dir: &Path, tag: &str, capture: bool) -> Result<String, String> {
    let prefix = bin_dir
        .parent()
        .ok_or_else(|| format!("cannot tell the install prefix of {}", bin_dir.display()))?;
    if !has_cmd("curl") && !has_cmd("wget") {
        return Err("curl or wget is needed to download the installer".into());
    }
    let script = std::env::temp_dir().join(format!("osier-install-{}.sh", std::process::id()));
    let script_url = format!("https://raw.githubusercontent.com/{REPO}/{tag}/install.sh");
    if !download(&script_url, &script) {
        let _ = std::fs::remove_file(&script);
        return Err(format!(
            "could not download {script_url} (is {tag} a published release?)"
        ));
    }
    let mut cmd = Command::new("sh");
    cmd.arg(&script)
        .env("OSIER_PREFIX", prefix)
        .env("OSIER_VERSION", tag)
        .stdin(Stdio::null());
    let result = if capture {
        cmd.output().map(|o| {
            let mut text = String::from_utf8_lossy(&o.stdout).into_owned();
            text.push_str(&String::from_utf8_lossy(&o.stderr));
            (o.status, text)
        })
    } else {
        cmd.status().map(|s| (s, String::new()))
    };
    let _ = std::fs::remove_file(&script);
    match result {
        Ok((status, text)) if status.success() => Ok(text),
        Ok((status, text)) => Err(format!(
            "the installer failed ({}){}",
            status
                .code()
                .map_or_else(|| "killed".to_string(), |c| format!("exit {c}")),
            if text.trim().is_empty() {
                String::new()
            } else {
                format!(": {}", text.trim())
            }
        )),
        Err(e) => Err(format!("could not run the installer: {e}")),
    }
}

/// Start `binary` as soon as process `pid` (this one) has exited, so the new Osier takes
/// over the single-instance socket and reopens the saved session. The caller quits next.
/// `current_exe` cannot be used for this: after an update it names the deleted old file.
#[cfg_attr(test, allow(dead_code))]
pub fn restart(binary: &Path, pid: u32) -> std::io::Result<()> {
    Command::new("sh")
        .args([
            "-c",
            "while kill -0 \"$1\" 2>/dev/null; do sleep 0.1; done; exec \"$2\"",
            "osier-restart",
            &pid.to_string(),
        ])
        .arg(binary)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::{Decision, decide, tag_from_url};

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
    fn reads_tags_from_redirects() {
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
    }
}
