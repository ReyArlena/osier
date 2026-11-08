//! How this Osier binary was installed: shown in Settings > About and
//! `osier --version --verbose`, and the gate for `osier --update`, which only replaces
//! binaries the install script put there. Everything else is updated the way it was
//! installed (a package manager, Flatpak, cargo, a rebuild).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallRoute {
    /// The install script (or a manual copy into a writable `bin/`): `--update` may replace it.
    Script {
        bin_dir: PathBuf,
    },
    /// Built with cargo from a checkout (`target/…`, or running under `cargo run`).
    DevBuild,
    Flatpak,
    /// Owned by a system package (pacman also covers the AUR).
    Package {
        manager: &'static str,
        name: String,
    },
    /// `cargo install` into `$CARGO_HOME/bin`.
    Cargo,
    Nix,
    Snap,
    AppImage,
    /// None of the above; `reason` says why it cannot be updated in place.
    Other {
        reason: String,
    },
}

/// What detection looks at, so tests can supply their own answers.
pub struct Probe<'a> {
    pub exe: PathBuf,
    pub env: &'a dyn Fn(&str) -> Option<OsString>,
    pub in_flatpak: bool,
    /// `(manager, package)` owning a file, if any.
    pub owner: &'a dyn Fn(&Path) -> Option<(&'static str, String)>,
    pub writable: &'a dyn Fn(&Path) -> bool,
}

impl InstallRoute {
    /// Detect for the running binary. Runs the package managers' owner queries, so call it
    /// once and keep the result.
    pub fn detect() -> Self {
        let exe = std::env::current_exe()
            .map(|p| std::fs::canonicalize(&p).unwrap_or(p))
            .unwrap_or_default();
        let env = |k: &str| std::env::var_os(k);
        Self::detect_with(&Probe {
            exe,
            env: &env,
            in_flatpak: Path::new("/.flatpak-info").exists(),
            owner: &package_owning,
            writable: &dir_writable,
        })
    }

    pub fn detect_with(p: &Probe) -> Self {
        let exe = &p.exe;
        if (p.env)("CARGO_MANIFEST_DIR").is_some() || in_cargo_target(exe) {
            return Self::DevBuild;
        }
        if p.in_flatpak {
            return Self::Flatpak;
        }
        if (p.env)("SNAP").is_some() {
            return Self::Snap;
        }
        if (p.env)("APPIMAGE").is_some() {
            return Self::AppImage;
        }
        if exe.starts_with("/nix/store") {
            return Self::Nix;
        }
        if let Some((manager, name)) = (p.owner)(exe) {
            return Self::Package { manager, name };
        }
        let cargo_home = (p.env)("CARGO_HOME")
            .map(PathBuf::from)
            .or_else(|| (p.env)("HOME").map(|h| PathBuf::from(h).join(".cargo")));
        if let Some(home) = cargo_home
            && exe.starts_with(home.join("bin"))
        {
            return Self::Cargo;
        }
        let Some(dir) = exe.parent() else {
            return Self::Other {
                reason: "cannot tell where the binary lives".into(),
            };
        };
        if dir.file_name().is_none_or(|n| n != "bin") {
            return Self::Other {
                reason: format!("{} is not in a bin folder", crate::format::tilde(exe)),
            };
        }
        if !(p.writable)(dir) {
            return Self::Other {
                reason: format!("{} is not writable", crate::format::tilde(dir)),
            };
        }
        Self::Script {
            bin_dir: dir.to_path_buf(),
        }
    }

    /// One line for About and `--version --verbose`.
    pub fn label(&self) -> String {
        match self {
            Self::Script { bin_dir } => {
                format!("install script ({})", crate::format::tilde(bin_dir))
            }
            Self::DevBuild => "development build".into(),
            Self::Flatpak => "Flatpak".into(),
            Self::Package { manager, name } => format!("{manager} ({name})"),
            Self::Cargo => "cargo install".into(),
            Self::Nix => "Nix".into(),
            Self::Snap => "Snap".into(),
            Self::AppImage => "AppImage".into(),
            Self::Other { reason } => format!("unknown ({reason})"),
        }
    }

    /// Why `osier --update` will not run, and what to do instead. `None` when it may.
    pub fn update_refusal(&self) -> Option<String> {
        Some(match self {
            Self::Script { .. } => return None,
            Self::DevBuild => "This is a development build; pull and rebuild with cargo.".into(),
            Self::Flatpak => "Osier runs as a Flatpak; update it with `flatpak update`.".into(),
            Self::Package { manager, name } => {
                let how = match *manager {
                    "pacman" => "pacman or your AUR helper (yay, paru)",
                    "dpkg" => "apt",
                    "rpm" => "dnf or your package manager",
                    _ => "your package manager",
                };
                format!("Osier was installed by {manager} as {name}; update it with {how}.")
            }
            Self::Cargo => "Osier was installed with `cargo install`; run it again to update \
                            (`cargo install --locked --git https://github.com/ReyArlena/osier osier`)."
                .into(),
            Self::Nix => "Osier comes from Nix; update it through Nix.".into(),
            Self::Snap => "Osier runs as a Snap; update it with `snap refresh`.".into(),
            Self::AppImage => "Osier runs as an AppImage; download the new AppImage.".into(),
            Self::Other { reason } => {
                format!("Cannot update in place: {reason}. Update it the way it was installed.")
            }
        })
    }
}

/// `target/debug/osier`, `target/release/osier`, `target/<triple>/release/osier`.
fn in_cargo_target(exe: &Path) -> bool {
    let Some(dir) = exe.parent() else {
        return false;
    };
    let profile = dir.file_name().and_then(|n| n.to_str());
    matches!(profile, Some("debug" | "release"))
        && dir
            .ancestors()
            .skip(1)
            .take(2)
            .any(|a| a.file_name().is_some_and(|n| n == "target"))
}

fn has_cmd(cmd: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|d| d.join(cmd).is_file()))
}

fn capture(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd)
        .args(args)
        .env("LC_ALL", "C")
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The system package owning `path`, from whichever package manager is present.
fn package_owning(path: &Path) -> Option<(&'static str, String)> {
    let file = path.to_string_lossy();
    if has_cmd("pacman")
        && let Some(out) = capture("pacman", &["-Qqo", &file])
    {
        // `-Qqo` prints just the package name.
        return out.lines().next().map(|n| ("pacman", n.trim().to_string()));
    }
    if has_cmd("dpkg")
        && let Some(out) = capture("dpkg", &["-S", &file])
    {
        // "osier: /usr/bin/osier"
        return out
            .split_once(':')
            .map(|(n, _)| ("dpkg", n.trim().to_string()));
    }
    if has_cmd("rpm")
        && let Some(out) = capture("rpm", &["-qf", "--qf", "%{NAME}\\n", &file])
    {
        return out.lines().next().map(|n| ("rpm", n.trim().to_string()));
    }
    None
}

/// Whether we may create files in `dir` (the installer replaces the binary there).
fn dir_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".osier-write-test-{}", std::process::id()));
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{InstallRoute, Probe};
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    fn detect(
        exe: &str,
        env: &[(&str, &str)],
        flatpak: bool,
        owner: Option<(&'static str, &str)>,
        writable: bool,
    ) -> InstallRoute {
        let env: Vec<(String, String)> = env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let lookup = move |k: &str| {
            env.iter()
                .find(|(n, _)| n == k)
                .map(|(_, v)| OsString::from(v))
        };
        let owner = move |_: &Path| owner.map(|(m, n)| (m, n.to_string()));
        let writable = move |_: &Path| writable;
        InstallRoute::detect_with(&Probe {
            exe: PathBuf::from(exe),
            env: &lookup,
            in_flatpak: flatpak,
            owner: &owner,
            writable: &writable,
        })
    }

    const HOME: (&str, &str) = ("HOME", "/home/x");

    #[test]
    fn script_installs_can_update() {
        let r = detect("/home/x/.local/bin/osier", &[HOME], false, None, true);
        assert_eq!(
            r,
            InstallRoute::Script {
                bin_dir: "/home/x/.local/bin".into()
            }
        );
        assert!(r.update_refusal().is_none());
        // A custom prefix works too, wherever the bin folder is.
        let r = detect("/opt/tools/bin/osier", &[HOME], false, None, true);
        assert!(matches!(r, InstallRoute::Script { .. }));
    }

    #[test]
    fn everything_else_is_refused_with_its_own_advice() {
        let cases: Vec<(InstallRoute, &str)> = vec![
            (
                detect(
                    "/home/x/osier/target/debug/osier",
                    &[HOME],
                    false,
                    None,
                    true,
                ),
                "development build",
            ),
            (
                detect(
                    "/home/x/osier/target/x86_64-unknown-linux-gnu/release/osier",
                    &[HOME],
                    false,
                    None,
                    true,
                ),
                "development build",
            ),
            (
                detect(
                    "/home/x/.local/bin/osier",
                    &[HOME, ("CARGO_MANIFEST_DIR", "/x")],
                    false,
                    None,
                    true,
                ),
                "development build",
            ),
            (
                detect("/app/bin/osier", &[HOME], true, None, false),
                "flatpak update",
            ),
            (
                detect(
                    "/usr/bin/osier",
                    &[HOME],
                    false,
                    Some(("pacman", "osier-bin")),
                    false,
                ),
                "AUR helper",
            ),
            (
                detect(
                    "/usr/bin/osier",
                    &[HOME],
                    false,
                    Some(("dpkg", "osier")),
                    false,
                ),
                "apt",
            ),
            (
                detect("/home/x/.cargo/bin/osier", &[HOME], false, None, true),
                "cargo install",
            ),
            (
                detect(
                    "/srv/c/bin/osier",
                    &[HOME, ("CARGO_HOME", "/srv/c")],
                    false,
                    None,
                    true,
                ),
                "cargo install",
            ),
            (
                detect(
                    "/nix/store/abc-osier/bin/osier",
                    &[HOME],
                    false,
                    None,
                    false,
                ),
                "Nix",
            ),
            (
                detect(
                    "/snap/osier/1/bin/osier",
                    &[HOME, ("SNAP", "/snap/osier/1")],
                    false,
                    None,
                    false,
                ),
                "snap refresh",
            ),
            (
                detect(
                    "/tmp/.mount/usr/bin/osier",
                    &[HOME, ("APPIMAGE", "/home/x/Osier.AppImage")],
                    false,
                    None,
                    false,
                ),
                "AppImage",
            ),
            (
                detect("/usr/local/bin/osier", &[HOME], false, None, false),
                "not writable",
            ),
            (
                detect("/home/x/Downloads/osier", &[HOME], false, None, true),
                "not in a bin folder",
            ),
        ];
        for (route, advice) in cases {
            let refusal = route
                .update_refusal()
                .unwrap_or_else(|| panic!("{route:?} should be refused"));
            assert!(refusal.contains(advice), "{route:?}: {refusal}");
            assert!(!route.label().is_empty());
        }
    }

    #[test]
    fn labels_name_the_route() {
        assert_eq!(
            detect(
                "/usr/bin/osier",
                &[HOME],
                false,
                Some(("pacman", "osier-bin")),
                false
            )
            .label(),
            "pacman (osier-bin)"
        );
        assert_eq!(
            detect("/app/bin/osier", &[HOME], true, None, false).label(),
            "Flatpak"
        );
        assert_eq!(
            detect("/home/x/target/release/osier", &[HOME], false, None, true).label(),
            "development build"
        );
    }
}
