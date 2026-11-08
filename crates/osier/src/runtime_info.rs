//! What Osier is running as: versions, display backend, sandbox, paths. Shown in Settings >
//! About and printed by `osier --version --verbose` for bug reports.

use std::fmt;

use osier_config::Paths;

/// The exact build: the Cargo version, plus git build metadata when this is not a clean
/// release tag (see `build.rs`), e.g. `0.1.0` or `0.1.0+12.g4a1b2c3`.
pub const VERSION: &str = env!("OSIER_BUILD_VERSION");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeInfo {
    pub osier_version: String,
    pub git_version: Option<String>,
    /// How this binary was installed (`install_route.rs`), when it was looked up.
    pub install: Option<String>,
    /// `Wayland` or `X11`, from the environment the process was started with.
    pub display_backend: &'static str,
    pub session_type: Option<String>,
    pub desktop: Option<String>,
    pub flatpak: bool,
    pub config_dir: Option<String>,
    pub state_dir: Option<String>,
    pub cache_dir: Option<String>,
}

impl RuntimeInfo {
    pub fn collect(
        paths: Option<&Paths>,
        git_version: Option<String>,
        install: Option<String>,
    ) -> Self {
        let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let display_backend = if env("WAYLAND_DISPLAY").is_some() {
            "Wayland"
        } else if env("DISPLAY").is_some() {
            "X11"
        } else {
            "none"
        };
        Self {
            osier_version: VERSION.to_string(),
            git_version,
            install,
            display_backend,
            session_type: env("XDG_SESSION_TYPE"),
            desktop: env("XDG_CURRENT_DESKTOP"),
            flatpak: std::path::Path::new("/.flatpak-info").exists(),
            config_dir: paths.map(|p| p.config_dir().display().to_string()),
            state_dir: paths.map(|p| p.state_dir().display().to_string()),
            cache_dir: paths.map(|p| p.cache_dir().display().to_string()),
        }
    }

    /// `(label, value)` rows, in display order.
    pub fn rows(&self) -> Vec<(&'static str, String)> {
        let or_unknown = |v: &Option<String>| v.clone().unwrap_or_else(|| "unknown".into());
        vec![
            ("Osier", format!("v{}", self.osier_version)),
            ("Installed", or_unknown(&self.install)),
            (
                "Git",
                self.git_version
                    .as_ref()
                    .map_or_else(|| "unknown".into(), |v| format!("v{v}")),
            ),
            ("Display", self.display_backend.to_string()),
            ("Session", or_unknown(&self.session_type)),
            ("Desktop", or_unknown(&self.desktop)),
            ("Flatpak", if self.flatpak { "yes" } else { "no" }.into()),
            ("Config", or_unknown(&self.config_dir)),
            ("State", or_unknown(&self.state_dir)),
            ("Cache", or_unknown(&self.cache_dir)),
        ]
    }
}

impl fmt::Display for RuntimeInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (label, value) in self.rows() {
            writeln!(f, "{label:<8} {value}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::RuntimeInfo;

    #[test]
    fn prints_every_row() {
        let info = RuntimeInfo::collect(
            None,
            Some("2.50.0".into()),
            Some("install script (~/.local/bin)".into()),
        );
        let text = info.to_string();
        assert!(text.contains("Git      v2.50.0"), "{text}");
        assert!(
            text.contains(&format!("Osier    v{}", super::VERSION)),
            "{text}"
        );
        assert!(text.contains("Flatpak"));
        assert_eq!(text.lines().count(), 10);
        assert!(
            text.contains("Installed install script (~/.local/bin)"),
            "{text}"
        );
        let unknown = RuntimeInfo::collect(None, None, None).to_string();
        assert!(unknown.contains("Git      unknown"), "{unknown}");
    }

    #[test]
    fn build_version_starts_with_the_cargo_version() {
        let v = super::VERSION;
        assert!(v.starts_with(env!("CARGO_PKG_VERSION")), "{v}");
        let meta = v
            .strip_prefix(env!("CARGO_PKG_VERSION"))
            .unwrap_or_default();
        // Either the plain release version or semver build metadata.
        assert!(meta.is_empty() || meta.starts_with('+'), "{v}");
        assert!(
            meta.chars()
                .all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c)),
            "{v}"
        );
    }
}
