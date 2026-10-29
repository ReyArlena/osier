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
- Switching to or opening a project with nothing to commit while on the Changes tab shows the
  Log tab instead; switching to a project with changes brings Changes back. Other tabs are
  kept as before. Pick the tab or turn it off in Settings > Projects
  (`projects.empty_changes_tab`).
- Command palette submenus. "Switch project…" (`ctrl-o` opens it directly) searches the open,
  pinned and recent projects, shown with the same changes, ahead/behind and pin markers as the
  sidebar, and switches to or opens the chosen one. Backspace on an empty query or Esc goes
  back to the commands.
- Right-click any commit, reflog entry, branch, tag, remote, worktree, stash or patch row for
  its menu. The `...` row buttons are hidden by default (Settings > Appearance > Lists).

### Changed

- New application icon.
- Log: branch and tag badges can sit before the subject (default) or at the end of the line,
  where the subject fades out under them like sidebar names (Settings > Git > Log,
  `git.log_badges`); branch badges other than the checked-out one are tinted with the
  primary colour.
- Times in the Log, reflog, Branches, Stashes, Patches and blame show the full local date and
  time on hover, after the configured tooltip delay; "12 h ago" is now "12h ago".
- The askpass window shows the Osier icon (a hidden desktop entry matches its app id).
- The command palette opens with `ctrl-p` (and still `F1`).
- Open repository moved from `ctrl-o` to `ctrl-shift-o`; `ctrl-o` now switches project.
- New installs follow the desktop's light/dark preference (System mode), read from the XDG
  settings portal. With no stated preference or no portal, Osier uses the dark theme (set in
  Appearance > When the desktop has no preference).
- Themes: One Dark, One Light, Monokai, Dracula, Nord, Gruvbox Dark and Light, Solarized Dark
  and Light, Tokyo Night.
- Theme picker with preview cards for the dark and light themes. Following Dux Shell colours
  is the first Theme setting, on by default when Dux Shell is detected; turning it on sets
  the mode to System and turns the pickers off while it is on.
- Prefix (leader) key: bind one key, then set each binding to No prefix, Prefix only or
  Either. A status bar badge shows while it waits; Escape, a click, an unbound key or the
  timeout (5 seconds, Settings > Keymap) cancel it. Works in text fields.
- Keys for context-menu entries, run on the selected row of Changes, Files, Log, Branches
  (branches, tags, remotes), Stashes or Patches (Settings > Keymap > Context menus). They
  cannot share a key with another binding, and menus show each entry's key.
- Branch, tag, remote and stash rows can be selected with a click or the arrow keys.
- New .gitignore lines go next to related lines (same folders, or other `*.ext` patterns)
  instead of the end, unless a later rule would override them.
- Add to .gitignore, ignore only on this machine (`.git/info/exclude`) and Untrack from the
  Files and Changes context menus. Tracked paths get a confirmation that offers to stop
  tracking them (`git rm --cached`, the file stays on disk).
- Tooltip delay setting (Settings > Appearance > Tooltips), shown at once by default (was 500 ms).
- The tab list menu only appears when the tabs do not fit, and cut-off tabs fade out at the
  edges.
- File watching skips ignored folders (build output) and `.git/objects`, which cuts
  inotify watches by orders of magnitude on projects with build directories. Ignored files
  are still watched while the Files tab shows them.
- Sidebar badges: ahead/behind arrows and an uncommitted-changes marker (icon colour or
  dot), optionally also for projects that are not open (cached, watched or checked on an
  interval).
- Packaging: install script, desktop entry and icon, GitHub Actions CI and release
  workflows, Flatpak manifest draft and AUR PKGBUILDs.

### Fixed

- The per-hunk actions button is a boxed plus or minus icon, so it no longer reads as a second diff sign.
- Counts read "1 change" and "1 file" instead of "1 changes" and "1 files".
- Startup reopens the project that was active last session, not the last one in the open list.
- Confirmation dialogs now have OK and Cancel buttons (they only answered to Enter and
  Escape before).
- install.sh: source builds now install the desktop entry and icons, and the entry
  launches the installed binary by its full path.
