# Changelog

All notable changes to Osier are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Changes tab with visual staging of files, hunks and selected lines, discard to trash for
  untracked files, commit box with amend, sign-off, commit and push, and recent messages.
- Log tab with a commit graph, branch and tag labels, reflog, compare between refs, file
  history and blame.
- Branches tab covering local and remote branches, tags, remotes, worktrees and submodules.
- Stashes tab with push, apply, pop and drop.
- Files tab with a code viewer, rendered markdown (with remote images, optional) and images
  at the working tree or any revision; edit files in place.
- Fetch, pull and push with streamed progress and cancel, upstream prompts, per-repository
  pull strategy, divergence and rejection dialogs, `--force-with-lease`.
- Merge, rebase, cherry-pick, revert, interactive rebase and in-place conflict resolution.
- Patches: copy as patch, apply from clipboard or file with a dry run and three-way fallback,
  a patch clipboard with shelve and unshelve.
- Multi-account GitHub authentication through the gh CLI with a credential helper mode, an
  askpass window for ssh and git prompts, per-project account memory.
- Themes: generated Catppuccin Mocha and Latte, user theme files with hot reload, Dux Shell
  colour sync, font settings; a theme menu in the title bar.
- Settings sheet (appearance, diff, git, editors, accounts, projects, about), with
  `config.toml` reloaded on external change and `osier --version --verbose` runtime info.
- Command palette, git command log panel, operation history, project sidebar with scan
  directories, filter, recents, pins and drag reordering.
- Right-click a git command log row to copy the command or its full details.
- Project filter qualifiers `is:changed`, `is:ahead` and `is:behind`, with a suggestion
  dropdown under the filter box.
- The Changes tab shows how many files have changes.
- Open on GitHub in the project and commit context menus.
- Sidebar badges: ahead/behind arrows and an uncommitted-changes marker (icon colour or
  dot), optionally also for projects that are not open (cached, watched or checked on an
  interval).
- Packaging: install script, desktop entry and icon, GitHub Actions CI and release
  workflows, Flatpak manifest draft and AUR PKGBUILDs.
