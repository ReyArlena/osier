//! Bake the exact build into the version Osier reports (About, `--version`, the log), so a
//! build of main is never mistaken for the release it follows.
//!
//! `OSIER_BUILD_VERSION` is the Cargo version, plus build metadata from git when the build
//! is not exactly a clean release tag:
//! - on the tag `v0.1.0`, clean: `0.1.0`
//! - 12 commits after it: `0.1.0+12.g4a1b2c3`
//! - before any release tag, or past a tag for another version (the Cargo version was not
//!   bumped yet): `0.1.0+g4a1b2c3`; pre-release tags (`v0.2.0-rc1`) do not count
//! - with uncommitted changes: `-dirty` on the end
//! - no git (source tarball, crates.io): `0.1.0`
//!
//! The dirty marker is decided when this script runs, which cargo repeats when HEAD, the
//! index or the tags change; editing a file without staging it does not re-run it.

use std::path::Path;
use std::process::Command;

fn git(dir: &str, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn main() {
    let base = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());
    let version = describe(&dir, &base).unwrap_or_else(|| base.clone());
    println!("cargo:rustc-env=OSIER_BUILD_VERSION={version}");

    // Re-run when the commit, the staged state or the tags change.
    if let Some(git_dir) = git(&dir, &["rev-parse", "--absolute-git-dir"]) {
        for f in ["HEAD", "index", "packed-refs", "refs/tags"] {
            let p = Path::new(&git_dir).join(f);
            if p.exists() {
                println!("cargo:rerun-if-changed={}", p.display());
            }
        }
        if let Some(head_ref) = git(&dir, &["symbolic-ref", "-q", "HEAD"]) {
            let p = Path::new(&git_dir).join(head_ref);
            if p.exists() {
                println!("cargo:rerun-if-changed={}", p.display());
            }
        }
    }
    println!("cargo:rerun-if-changed=build.rs");
}

/// The version with git build metadata, or `None` outside a git checkout.
fn describe(dir: &str, base: &str) -> Option<String> {
    let hash = git(dir, &["rev-parse", "--short=7", "HEAD"])?;
    let dirty =
        git(dir, &["status", "--porcelain", "--untracked-files=no"]).is_some_and(|s| !s.is_empty());
    // The newest release tag reachable from HEAD (stable `vX.Y.Z` only, no pre-releases) and
    // how many commits HEAD is past it.
    let since_tag = git(
        dir,
        &[
            "describe",
            "--tags",
            "--long",
            "--match",
            "v[0-9]*.[0-9]*.[0-9]*",
            "--exclude",
            "v*-*",
        ],
    )
    .and_then(|d| {
        // v0.1.0-12-g4a1b2c3
        let mut parts = d.rsplitn(3, '-');
        let _hash = parts.next()?;
        let n = parts.next()?.parse::<u32>().ok()?;
        Some((parts.next()?.to_string(), n))
    });
    let mut meta = match since_tag {
        // Exactly the release this Cargo version names, nothing changed.
        Some((tag, 0)) if !dirty && tag == format!("v{base}") => return Some(base.to_string()),
        Some((tag, n)) if n > 0 && tag == format!("v{base}") => format!("{n}.g{hash}"),
        _ => format!("g{hash}"),
    };
    if dirty {
        meta.push_str("-dirty");
    }
    Some(format!("{base}+{meta}"))
}
