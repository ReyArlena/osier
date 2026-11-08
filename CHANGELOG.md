# Changelog

All notable changes to Osier are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- The command log has a "Failures only" view, and error notifications open it on the
  command that failed.
- Merge, rebase, interactive rebase and pulling a diverged branch offer to stash your
  uncommitted changes while they run (git's autostash), and say so when the changes come
  back with conflicts.
- Cherry-pick, revert and hard reset offer to save uncommitted changes to the Stashes tab
  first. Those are not put back automatically.
- Squash offers the same stash option when commits have to be replayed over uncommitted
  changes, instead of refusing.
- When uncommitted changes stop a pull, Osier offers to stash them, pull and put them back.
  When they stop a branch switch, it offers to save them to the Stashes tab and switch.

### Changed

- Common git refusals (unmerged branch, worktree with changes, missing remote branch,
  local changes in the way) are explained in plain words, and error titles say what could
  not be done, such as "Could not delete branch x".
- Dialog options have short labels with a plain explanation underneath, instead of
  bracketed text. "Push rejected" now says "Fetch now" and explains that pulling and
  pushing again are separate steps you take afterwards.
- "Branch has diverged" always asks how to pull, with your last choice for the repository
  preselected, instead of offering to remember it and never asking again.
- "Remove worktree?" says what is deleted and what is kept, and no longer shows the path in
  an editable text field.
- Clearer wording in the pull, force push, upstream, abort, branch delete, reset, detached
  checkout, stash, tag, worktree, clone and untrack dialogs, written for people new to git.
- A force push that git stops because someone pushed since your last fetch says so,
  instead of reporting a rejected non fast-forward update.

### Fixed

- Error notifications show git's error line instead of progress output such as
  "From <url>".
- The "Also add ... to .gitignore" option in the untrack dialog wraps like normal text.
- Notifications show their title again. A title such as "Push failed" was replaced by the
  message underneath it.
- Choosing force push in the "Push rejected" dialog opens its confirmation instead of
  doing nothing, and the second step of "New worktree" opens too.
- Text prompts such as "New branch" and "Stash changes" focus their text field when they
  open.
- The hard reset dialog no longer says the reflog can bring back uncommitted work.
- The conflict notification points at the Continue and Abort buttons above the tabs, not
  the status bar.
- Staging or unstaging the selected file moves the selection to the next file in its list
  (or the one above, when it was the last), instead of leaving the file's name over an
  empty "No changes" pane. When the list empties, the selection follows the file. An open
  editor keeps its file selected, and a dropped stash moves on to the next stash the same
  way.
- Ticking a file's checkbox in Changes no longer selects that file.

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
