# Changelog

All notable changes to Osier are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- Notifications show their title again. A title such as "Push failed" was replaced by the
  message underneath it.

## [0.1.0] - 2026-11-08

First release.

### Highlights

- Visual staging of files, hunks and selected lines, with discard to trash for untracked
  files and a commit box with amend, sign-off, commit and push.
- Log with a commit graph, ref badges, reflog, multi-select, squash, file history, blame and
  a resizable commit details panel.
- Branches, tags, remotes, worktrees, submodules and stashes in one place.
- Fetch, pull and push with progress and cancel, upstream prompts and `--force-with-lease`.
- Merge, rebase, interactive rebase, cherry-pick and revert, with conflicts resolved in place.
- Patches: copy as patch, apply with a dry run and three-way fallback, shelve and unshelve.
- Clone and create repositories from the command palette, including on GitHub.
- Multiple GitHub accounts through the gh CLI, chosen per project; tokens never reach the
  command line or a log.
- Files tab with code, markdown and image viewing at any revision.
- Themes: Catppuccin, classic themes, your own theme files and live Dux Shell colours,
  with a higher contrast option.
- Command palette, rebindable keys with an optional prefix key, and a log of every git
  command Osier runs.
- Install script, desktop entry and release builds for x86_64 and ARM64 Linux; a daily
  update check with an "Update available" badge, and updates from Settings or
  `osier --update` for script installs.

[Unreleased]: https://github.com/ReyArlena/osier/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/ReyArlena/osier/releases/tag/v0.1.0
