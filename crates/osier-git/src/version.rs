//! Git version detection and the minimum we support.

use crate::error::{GitError, Result};
use crate::runner::GitRunner;

/// Oldest git version Osier supports.
pub const MIN_VERSION: GitVersion = GitVersion {
    major: 2,
    minor: 30,
    patch: 0,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct GitVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl std::fmt::Display for GitVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl GitVersion {
    /// Parse the output of `git --version`, e.g. `git version 2.56.0` or
    /// `git version 2.39.5 (Apple Git-154)`.
    pub fn parse(text: &str) -> Result<Self> {
        let rest = text
            .trim()
            .strip_prefix("git version ")
            .ok_or_else(|| GitError::parse("git --version", text.trim().to_string()))?;
        let numeric: String = rest
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        let mut parts = numeric.split('.').map(|p| p.parse::<u32>());
        let mut next = || parts.next().transpose().ok().flatten();
        let major = next().ok_or_else(|| GitError::parse("git --version", text.to_string()))?;
        let minor = next().unwrap_or(0);
        let patch = next().unwrap_or(0);
        Ok(Self {
            major,
            minor,
            patch,
        })
    }

    /// Query the git binary.
    pub fn detect(runner: &GitRunner) -> Result<Self> {
        let out = runner.run(None, ["--version"])?;
        Self::parse(out.stdout_str()?)
    }

    /// Detect and fail when older than [`MIN_VERSION`].
    pub fn check(runner: &GitRunner) -> Result<Self> {
        let found = Self::detect(runner)?;
        if found < MIN_VERSION {
            return Err(GitError::GitTooOld {
                found: found.to_string(),
                required: MIN_VERSION.to_string(),
            });
        }
        Ok(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_and_vendor_suffixed() {
        assert_eq!(
            GitVersion::parse("git version 2.56.0\n").unwrap(),
            GitVersion {
                major: 2,
                minor: 56,
                patch: 0
            }
        );
        assert_eq!(
            GitVersion::parse("git version 2.39.5 (Apple Git-154)").unwrap(),
            GitVersion {
                major: 2,
                minor: 39,
                patch: 5
            }
        );
        assert_eq!(
            GitVersion::parse("git version 2.47.1.windows.1").unwrap(),
            GitVersion {
                major: 2,
                minor: 47,
                patch: 1
            }
        );
    }

    #[test]
    fn rejects_garbage() {
        assert!(GitVersion::parse("gti version").is_err());
    }

    #[test]
    fn ordering_matches_semver() {
        assert!(
            GitVersion {
                major: 2,
                minor: 29,
                patch: 9
            } < MIN_VERSION
        );
        assert!(
            GitVersion {
                major: 2,
                minor: 30,
                patch: 0
            } >= MIN_VERSION
        );
        assert!(
            GitVersion {
                major: 3,
                minor: 0,
                patch: 0
            } > MIN_VERSION
        );
    }

    #[test]
    fn local_git_passes_the_check() {
        GitVersion::check(&GitRunner::default()).expect("local git new enough");
    }
}
