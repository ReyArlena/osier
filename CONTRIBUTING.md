# Contributing to Osier

Thanks for considering a contribution. Osier is a small project with a clear scope: a fast,
native git GUI for Linux that mirrors the JetBrains git workflow. Bug reports, parity gaps
with that workflow and polish are all welcome.

## Reporting bugs

Include the output of `osier --version --verbose`, what you did, what happened, and the
relevant lines from the git command log (title bar, terminal icon). Never paste tokens; the
log never contains them, but check anything you copy from a terminal.

## Development setup

Native build dependencies on Debian/Ubuntu:

```
pkg-config libxkbcommon-dev libxkbcommon-x11-dev libxcb-xkb-dev libwayland-dev libvulkan-dev libfontconfig1-dev libssl-dev
```

Arch: `pkgconf libxkbcommon libxkbcommon-x11 wayland vulkan-headers fontconfig openssl`.
Then:

```sh
cargo build
cargo run -p osier -- /path/to/some/repo
cargo test --workspace            # no network needed
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
cargo deny check                  # cargo install --locked cargo-deny
```

The workspace is split into `osier-git` (git layer, no UI), `osier-config`, `osier-theme`,
`osier-auth` and the `osier` binary. Keep the git layer free of UI types so it stays testable.

## Conventions

- Rust 2024, `cargo fmt` defaults, clippy clean with `-D warnings`.
- Every git invocation goes through `GitRunner`; the UI never builds argv.
- Parse git output with porcelain v2, `-z` or explicit `--format` separators; fail closed on
  anything unexpected. Add a test with captured output for each parser change.
- Every mutating operation has a function in `osier-git::ops`, tests on `TempRepo` for a
  success and a failure path, a confirmation when data could be lost, and a history entry.
- No `unwrap`/`expect` in `osier-git` outside tests. Errors are typed (`GitError`).
- Long work runs off the UI thread. Results come back through entity updates.
- Settings go in `~/.config/osier`, state in `~/.local/state/osier`. Never write elsewhere
  except inside the repository being operated on.
- Never log credentials. The credential helper and askpass modes are the only places tokens
  or passphrases exist, and only in memory.
- Use a plain `-` instead of an em dash in strings and docs.

## UI changes

Headless UI tests live in `crates/osier/src/app.rs` (`mod tests`) and run with plain
`cargo test`. Add one for every new flow that can be driven without a pointer. For visual
checks, `scripts/dev-screenshot.sh [seconds] [repo]` builds, launches, captures with `grim`
(Wayland) and prints the log. Screenshots for the README show only this repository or a
throwaway one.

## Pull requests

1. Fork and branch from `main`.
2. Keep commits atomic with a single short subject line, no body, no co-author trailers.
3. Run fmt, clippy, tests and `cargo deny check` before pushing.
4. Describe the user-visible change and add a line under `Unreleased` in `CHANGELOG.md`.
5. Screenshots for UI changes help review.

## Testing against real remotes

The unit and integration tests use temporary repositories and local bare remotes only. Live
tests (`OSIER_LIVE_TESTS=1 cargo test -p osier --test live -- --ignored`) need the
maintainer's GitHub accounts and a private testing repository; they are not expected to pass
elsewhere.
