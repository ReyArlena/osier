# Changelog

All notable changes to Osier are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Builds between releases say exactly which commit they are in Settings > About and
  `osier --version` (`v0.1.0+12.g4a1b2c3`: 12 commits after v0.1.0, `-dirty` with
  uncommitted changes); release builds show the plain version. Versions show with a leading v.
- Show background tasks (Settings > Appearance, off by default): a spinner and label in the
  status bar while Osier refreshes, checks project status, scans directories, loads GitHub
  lists or looks up diffs, blame and branches.
- GitHub API calls made for clone and create appear in the git command log (the helper
  command, status, time and GitHub's error message, such as a rate limit; never a token).

### Changed

- Higher contrast also strengthens the ref badges in the Log and the palette's submenu badge.
- Status bar: the branch and running jobs in full-strength text (the rest a touch lighter
  grey than before), and running jobs (fetch, pull, push, clone, create)
  show a larger spinner in the theme's main colour.
- Create new repository finds templates with one GitHub search over the account and its
  organisations instead of reading every repository it can reach.
- The clone and create submenus wait for the gh accounts when opened right after startup.

- Opening a project (fresh, from the recent list, `osier <path>` or after a clone) that has
  changes shows the Changes tab. Switching between open projects keeps its existing rules.
- Clone repository… (command palette, bindable, no default key): pick a gh account and one of
  the repositories it can reach (its own, its organisations', the ones it collaborates on),
  or "Other…" for any HTTPS or SSH address. The dialog asks for the directory (next to the
  open project by default, or "Choose directory…"), runs with `--recurse-submodules` unless
  unticked, shows progress with cancel, opens the clone and remembers the account.
- Create new repository… (command palette, bindable, no default key): on GitHub for a gh
  account (owner or organisation, name, description, visibility, template, README,
  .gitignore and licence, as on github.com), then cloned and opened; or "No remote" for a
  local `git init` with the same starter files left uncommitted.
- `osier <path>` while Osier is running opens the repository in that window and brings it to the front (a socket in `$XDG_RUNTIME_DIR/osier`); `osier --new-instance` starts a separate window.
- Commit details under the Log (`ctrl-shift-d` or the header button): the message on the left; hash with copy, author, dates, committer when different, parents, refs and the branches containing the commit on the right. Resizable; shown or hidden and its height are remembered.
- Higher contrast (Settings > Appearance > Theme, `appearance.high_contrast`, off by default): selections, hovers and the sidebar highlight in a stronger tint of the theme's main colour, for every theme.
- The Log graph marks commits that are not on any remote yet: dimmed by default, or drawn
  in another colour (Settings > Git > Log, `git.log_unpushed = "dim" | "colour"`).
- Multi-select in the Log: shift-click picks a range, ctrl-click adds or removes single
  commits. Picks follow their commits when new ones arrive.
- Squash commits from the Log: pick them, right-click, Squash selected commits. Neighbouring
  commits combine with exactly the same content and staged work untouched; separated ones
  move together first (aborted, with nothing changed, if that conflicts). Refused for
  commits off the branch's own line or with merges in the way; the dialog says when the
  commits are already pushed or other branches and tags still contain them.
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

- The right pane follows the tab: each tab keeps its own selection (and open editor or
  blame) and shows it again when you come back. Changes starts on the first change, Log on
  the latest commit, other tabs show what to pick. Clicking a stash shows its changes. A
  merge, rebase or cherry-pick that stops on conflicts switches to Changes and shows the
  first conflicted file.
- Tabs keep their diff's scroll position, expanded sections and selected lines; coming back
  reuses the view without running git unless its content changed.
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

- Files tab folders with changes are marked like sidebar projects (tinted icon or dot, in the same colour, per Settings > Appearance > Sidebar) instead of a grey tag; hovering the marker says how many changed files are inside, and status letters say what they mean.
- Paths under the home folder show as `~/...` however the home is spelled (symlinked homes such as `/var/home`, `$HOME` with a trailing slash or unset), and `/home/reynolds` is no longer shortened as if it were under `/home/rey`.
- The diff now refreshes when the file it shows is edited again while already modified, and no longer reloads (losing its scroll) when an unrelated file changes.
- A shift-click pair in the Log stays on the same commits when new commits arrive.
- Startup shows the window at once however many projects were open: only the active project opens before the first frame, the rest load in the background in their saved order (446 projects: 1.35 s to 0.14 s).
- The centre panel keeps its saved width when the window is resized, including right after a tiling compositor opens it.
- The per-hunk actions button is a boxed plus or minus icon, so it no longer reads as a second diff sign.
- Counts read "1 change" and "1 file" instead of "1 changes" and "1 files".
- Startup reopens the project that was active last session, not the last one in the open list.
- Confirmation dialogs now have OK and Cancel buttons (they only answered to Enter and
  Escape before).
- install.sh: source builds now install the desktop entry and icons, and the entry
  launches the installed binary by its full path.
- install.sh: when no prebuilt binary fits (or the download fails), the source build uses
  the latest release tag instead of main; main only while there is no release, or with
  `OSIER_VERSION=main`.
