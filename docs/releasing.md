# Releasing

A release is a `vX.Y.Z` tag on `main`. Pushing the tag starts
`.github/workflows/release.yml`, which builds x86_64 and ARM64 binaries, checks that each one
reports the tag's version and publishes them as a GitHub release. `install.sh` and
`osier --update` pick the new release up from there.

Versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html): `fix` commits
since the last tag make a patch release, `feat` a minor one, a breaking change (`!`) a major
one (a minor one before 1.0).

```sh
git log --oneline "$(git describe --tags --abbrev=0)"..HEAD   # what is going out
```

## 1. Bump the version

| File | Change |
|---|---|
| `Cargo.toml` | `version` under `[workspace.package]` |
| `Cargo.lock` | refreshed by `cargo build`, never edited by hand |
| `CHANGELOG.md` | rename `[Unreleased]` to `[X.Y.Z] - YYYY-MM-DD`, add an empty `[Unreleased]` above it, update the compare links at the bottom |
| `packaging/com.reyarlena.osier.metainfo.xml` | a `<release version="X.Y.Z" date="YYYY-MM-DD" />` line at the top of `<releases>` (the date of the tag) |
| `packaging/aur/osier/PKGBUILD`, `packaging/aur/osier-bin/PKGBUILD` | `pkgver=X.Y.Z`, `pkgrel=1`, then `makepkg --printsrcinfo > .SRCINFO` in each folder |

`git grep -nF "<current version>"` finds anything this list misses. The version the app shows
comes from `Cargo.toml` through `crates/osier/build.rs`, so nothing in the source needs
editing.

The release workflow fails when the binary's `--version` is not exactly `osier vX.Y.Z`, so
the bump has to be committed before the tag. Pre-release tags (`v1.2.3-rc.1`) fail that
check as it stands.

## 2. Check

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check
cargo build --release -p osier && target/release/osier --version --verbose
shellcheck install.sh
appstreamcli validate --no-net packaging/com.reyarlena.osier.metainfo.xml
desktop-file-validate packaging/com.reyarlena.osier.desktop
```

Read the new CHANGELOG section and the README install instructions once more: they should
be true for this version.

## 3. Commit, tag and push

On `main`, up to date, with everything for the release merged:

```sh
git commit -am "chore(release): X.Y.Z"
git status --short                  # prints nothing
git tag -a vX.Y.Z -m "vX.Y.Z"
git push origin main                # the release commit first
git push origin vX.Y.Z              # the tag starts the release workflow
gh run watch
```

A tag that was made but not pushed can be deleted and made again (`git tag -d vX.Y.Z`). Once
a tag is pushed, prefer a new patch release over moving it: anyone who fetched it keeps the
old one.

## 4. After the workflow finishes

```sh
gh release view vX.Y.Z
```

The release should hold, for each of `x86_64` and `aarch64`:

- `osier-X.Y.Z-<arch>-unknown-linux-gnu.tar.gz` and its `.sha256` (binary, desktop entries,
  metainfo and icons; what `install.sh` downloads)
- `osier-linux-<arch>` and its `.sha256` (the bare binary, also reachable as
  `releases/latest/download/osier-linux-<arch>`)

Then:

- Download the assets and run `sha256sum -c` on each `.sha256`.
- Install into a throwaway prefix and check the version:
  `OSIER_PREFIX=/tmp/osier-release XDG_DATA_HOME=/tmp/osier-release/share sh install.sh`,
  then `/tmp/osier-release/bin/osier --version`.
- From an older script install, `osier --update --check` reports the new version.
- The release notes list every commit by type (`scripts/release-notes.sh`), then the pull
  requests. Add the CHANGELOG section on GitHub if they need more.
- If `site/` changed, the Pages workflow has deployed the website.

## Notes

- Release binaries are built on `ubuntu-22.04` so they run on glibc 2.35 and newer.
  `MIN_GLIBC` in `install.sh` has to match the runner image, so change both together.
- The Rust version for CI and releases is the one pinned in `rust-toolchain.toml`.
- The AUR packages and the Flatpak are not published yet. The PKGBUILDs carry
  `sha256sums=('SKIP')` until they are; the sections about them in the README and on the
  website are commented out and come back in the release that publishes them.
