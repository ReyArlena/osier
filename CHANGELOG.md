# Changelog

All notable changes to Osier are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- GitHub lists are kept ready:
  - Covered: repositories to clone, organisations, templates, `.gitignore` and licence lists.
  - The clone list and the Create dialog show the last known lists at once (saved in the
    cache folder), then check GitHub for new ones in the background. The palette says
    "Checking for new repositories…" while it does, and new ones appear when they are in.
  - The lists are also refreshed in the background every 30 minutes, one at a time.
    Settings > Accounts > GitHub lists sets how often, or Off (`github.list_refresh_minutes`);
    "Refresh now" checks at once.
- In the Files tab, moving with the arrow keys opens each file in the viewer, as clicking
  does. Folder rows leave the viewer on the last file.
- With "Ignored" on, the Files tab dims ignored files and shows where the rule comes from
  as a badge (eg `.gitignore`, `site/.gitignore`, `.git/info/exclude` or your global
  excludes file). The tooltip names the pattern and its line. Folders whose files are all
  ignored are dimmed too, with a badge per rule file.
- The right-click menu in Changes has "Discard changes…" (or "Move to trash…" for untracked
  files), the same as the row's hover button.
- Pull request links from what git already knows, no GitHub login needed. A Log commit that
  merges or squashes a pull request ("Merge pull request #93 ...", "... (#93)") gets "View
  pull request #93 on GitHub" in its right-click menu, and a commit at the tip of a remote
  branch gets "Pull requests for origin/... on GitHub", which opens GitHub's pull request list
  filtered to that branch. Branch rows (remote ones, and local ones with an upstream) have
  the same entry. Bindable as "Pull requests on GitHub" under Context menus in the keymap.
- The command log shows when each command started (time of day on the row, the full date
  when expanded, and in "Copy details").
- Syntax colours that sit too close to the editor background are lifted until they read,
  whatever the theme: comments above all, which most themes draw in a border grey (1.2:1
  inside a changed word on Catppuccin Mocha). Hue stays, so the theme still looks like
  itself. On by default; "Readable syntax colours" under Settings > Appearance > Contrast turns
  it off (`syntax_contrast` in config.toml).
- An Animations setting under Settings > Appearance > Motion (`animations` in config.toml):
  follow the desktop (the default), on or off. Off shows dialogs, menus and toasts at once
  and keeps spinners still. Following the desktop uses its reduced-motion setting, or
  GNOME's "Animations" switch where the desktop has no reduced-motion setting.
- An Animation length slider under Settings > Appearance > Motion (`animation_ms`, 60 to
  400ms, default 180): how long a dialog takes to fade in. Switches, tabs and collapsing
  sections speed up or slow down with it; menus, tooltips and toasts keep their own timing.
- The command palette, project switcher and settings shortcuts close what they opened when
  pressed again, as Escape does. With the palette open, the other one of `ctrl-p` and
  `ctrl-o` switches between commands and projects.
- A Notifications panel: every toast the app showed this session, newest first, with its
  time and, for git failures, the same "View command log" button, so a message that slid
  away can still be read. It shares the bottom panel with the git command log as a second
  tab. Open it with the bell in the title bar (which counts unseen notifications),
  `ctrl-shift-n`, or "Toggle notifications" in the command palette. Session only, nothing
  is saved.

### Changed

- The sidebar lists the projects found in the scan directories at once, from the last scan
  (kept on disk); the fresh scan replaces them when it is done.
- Finding the branches that contain a commit (commit details) is named in the status bar
  when it takes a moment.
- Settings > Editors finds the editors on PATH in the background (it looked on every redraw)
  and fills the list in once they are found. It also knows more:
  - Commands under other names, such as Zed as `zeditor` on Arch, `code-oss` and `sublime_text`.
  - JetBrains Toolbox scripts and Flatpak apps.
  - Cursor, Windsurf, GNOME Text Editor, WebStorm, PyCharm, RustRover, GoLand and CLion.
  A note says only editors Osier can start from the command line are listed.
- Osier no longer waits on git, file reads or other programs while you use it. These run in
  the background, with the status bar's background-tasks label while they do:
  - opening a project
  - viewing a file
  - the HEAD lookups around every git operation
  - saving, reading and listing patches
  - the interactive rebase list
  - Update project's first choice
  - turning on Amend
  - the inline and merge editors
  - theme and `config.toml` reloads
  - project status watches
  - settings and state saves (finished before Osier quits)
- Work you start and wait for (opening a project, a file, an editor, the rebase list, a
  patch) is named in the status bar when it takes longer than a moment, even with "Show
  background tasks" off. Quick ones show nothing, so nothing flashes.
- The Changes list builds only the rows on screen, so thousands of changes stay smooth. The
  Files tab, Log and command log no longer copy their data on every frame.
- Dialogs fade in where they sit instead of sliding down from the top, and finish sooner
  (180ms instead of 250ms).
- Error toasts put their "View command log" button under the message instead of beside it,
  so the message gets the full width.
- Settings has a cleaner layout: each group sits on its own card, every page has a short
  description, the header is a separate bar and the sidebar runs to the dialog's edges.
  Theme cards fill the width in an even grid, and Appearance is split into Theme, Dark
  theme, Light theme, Contrast and Status bar.

### Removed

- The `ctrl-q` Quit hint in the status bar. The shortcut is unchanged and listed in the
  command palette and the keymap.

### Fixed

- The clone and create dialogs flashed a red "already exists and is not empty" error just
  before closing, about the clone's own folder. The target folder is now checked in the
  background when it changes (not on every frame), Clone and Create stay disabled until the
  check passes, and a confirmed dialog no longer checks again. Problems show as an error
  notice (icon, tinted box) instead of a red line, and paths read `~/...`.
- Programs opened from Osier (editor, terminal, file manager, `xdg-open`) run detached in
  their own systemd scope where there is one, and no longer linger as `<defunct>` processes
  until Osier quits.
- Settings opens without waiting for `ssh-add -l`: the ssh status loads in the background
  and the Accounts page shows "checking…" until it is back.
- The command palette redrew the whole window twice while opening; now once.
- Frame drops while the settings dialog or the command palette opens. The main window was
  laid out and drawn again on every frame of the dialog's animation although nothing in it
  changed; it is now reused until something in it does.
- Controls on the right of settings rows all end at the same edge: the highlighted groups
  (Updates, project status, per-project choices), the slider values and the Keymap + buttons
  sat further in than the rest.
- Git commands that take a long list of paths no longer hang when git answers faster than
  Osier can send the list. This left the Files tab without ignored files in repositories
  with many of them (eg `target/`), and could stop the check for files a pull removed after
  they were gitignored.
- Folders you open or close in the Files tab stay that way when files are added or removed,
  instead of every folder going back to its default, also when a folder is gone for a while
  (eg a build folder being rebuilt, or ignored files hidden). Clearing the filter brings
  back the folders you had open before filtering.
- With "Ignored" on, the Files tab no longer runs git each time it redraws (every hover and
  key press). The ignored list is read in the background when files change, and the rows
  are only rebuilt when the list or the filter changes, so the tab stays smooth in
  repositories with many ignored files (eg `target/` or `node_modules`).
- The Files tab keeps its highlighted row when the file list changes, instead of losing it
  so the arrow keys start again from the top. When the highlighted file leaves the list (eg
  after adding it to .gitignore), the row and the viewer move to the next file.
- Removing a scan directory (in Settings or config.toml) also takes its repositories out of
  recents, instead of leaving behind the ones you had opened. Open projects stay.
- Opening a file or folder inside a repository that is already open (for example
  `osier src/main.rs`) switches to that project instead of adding it to the sidebar again.
- Keyboard shortcuts no longer go dead after hiding the panel that holds the focused filter
  box (`ctrl-b` while typing in the project filter, or switching tabs while typing in the
  Files filter). Focus moves back to the app instead of staying on a box that is not shown.
- `ctrl-q` quits while the command palette, settings or a prompt is open, instead of only
  from the main window.
- A dropdown or context menu opened from the keyboard closes when a shortcut moves on (a tab
  switch, the command palette, a filter box), instead of staying on screen until clicked away.

## [0.2.0] - 2026-11-09

### Added

- After a fetch or pull, Osier offers to delete local branches whose branch on the remote
  was deleted (after a prune) and whose commits are all on the remote's default branch, such
  as merged pull requests. It asks once per session; branches with work of their own are
  never offered.
- Three-way merge tool: a **3-way** layout next to Unified and Split shows conflicted files
  as your version, the result and their version side by side, like PhpStorm's merge dialog.
  Apply (`>>` / `<<`) or ignore (`X`) each change, apply every non-conflicting change from
  one side or both, type in the result, accept a whole side, and Save or Apply (marks the
  file resolved). Changes are highlighted word by word, joined across the panes, and the
  panes scroll together. During a rebase your commit stays on the left.
- Conflicted files open in the merge tool by default; Unified shows the inline conflict
  blocks instead, and the choice is remembered (`diff.conflicts`), separately from the
  unified / split choice for other files. A conflicted file without markers left says so and
  offers 3-way or Mark resolved instead of an empty diff.
- Unified and Split have the merge tool's bar: previous / next change (`F7`), file arrows
  while expanded, a change count and a gear menu. Its settings (hide whitespace, word
  highlights, synchronised scrolling, wrapping) are shared with the merge tool and saved.
- The merge tool's panes stay lined up when one has many more lines above or below the
  others: the shorter pane gets blank space instead of stopping at its first or last line.
- **Expand** in the diff pane header (or `ctrl-shift-m`, `Esc` to go back) hides the sidebar
  and file list in every layout and stays on while switching. In the merge tool it adds
  previous / next conflicted file, and Apply moves on to the next file.
- `osier merge <left> [<base>] <right> <output>` and `osier diff <left> <right>` open the
  merge tool or the diff view in a window of their own, for any files and as git's
  `mergetool` / `difftool` (setup in the README). Without a base every difference is a
  conflict.
- "Update project" button, left of Fetch: fetches, then merges or rebases with your
  uncommitted changes set aside and put back, like PhpStorm's Update Project. It asks which
  method to use each time, preselecting your last choice (or the repository's `pull.rebase`
  the first time), unless you tick "Remember my choice for this repository". This choice is
  kept apart from the diverged-pull one. Shortcut: `ctrl-t`, as in PhpStorm (Settings >
  Keymap to change it).
- Settings > Projects > Per-project choices lists what Osier remembers for each project
  (GitHub account, diverged-pull method, Update project method) as pills; the X on a pill
  forgets that choice so Osier asks again. The Accounts page points to it.
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
- When a pull deletes files that someone stopped tracking and added to .gitignore, Osier
  offers to put your copies back, untracked and ignored. Settings > Git > Pull turns the
  offer off.
- A Git config panel, from a project's right-click menu in the sidebar, lists the git
  settings that change how the repository behaves (pull, merge, rebase, push, identity,
  signing, line endings, remotes, credentials and more), with the value git uses, where it
  comes from (this repository, global, system, an included file or the environment) and
  what it overrides, plus hooks, ignore files and `GIT_` variables. Read only for now.
  Settings > Git > Git config tab also shows it as a tab.

### Changed

- A conflicted file's "Resolved" button is now "Mark resolved", for files fixed in another
  editor. It is disabled while the file still has conflict markers, with a tooltip saying
  why, so the markers can no longer be recorded as the resolved file. The editor's Save
  button says "Save and resolve" when saving will also mark the conflict resolved.
- "Always use this account for this project" in the account picker starts unticked, so
  remembering an account is always your choice.
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
- The untrack and ignore dialogs say the file stays on your disk, and that other people
  will most likely have it deleted when they pull the commit.
- A force push that git stops because someone pushed since your last fetch says so,
  instead of reporting a rejected non fast-forward update.
- The Log lists commits from all branches by date (git's date order, as PhpStorm does),
  instead of listing each branch's commits together.
- In the commit graph each branch keeps its own column and colour from its tip down to
  the commit it started from, and its line joins that commit there. Before, a branch moved
  into a neighbouring column straight away and took that column's colour.
- Shift-click in the Log picks only the commits on the line between the two clicked
  commits, so commits of other branches listed between them are left out and the range
  can still be squashed.

### Fixed

- Pulling (or Update project) on a branch whose upstream was deleted on GitHub, usually after
  its pull request was merged, explains that instead of showing git's error, and offers to
  switch to the default branch and pull it. It also offers to delete the local branch when
  all of its commits are already on the default branch.
- Sign-in (git's credential helper, the askpass prompt, GitHub calls) kept working only until
  the Osier binary was replaced while it ran (an update or a rebuild): Osier then handed git
  a path ending in " (deleted)". It now uses the new binary at the original path.
- Starting git retries for a moment when the executable is briefly busy ("Text file busy"),
  which made a test fail now and then on CI.
- Pulling a diverged branch now closes its dialog and asks which GitHub account to use when
  the project has none chosen, instead of leaving the dialog open and doing nothing.
- Keyboard shortcuts work again straight after a dialog closes, instead of doing nothing
  until you click somewhere in the window.
- The sidebar filter's link to the project status setting selects "Status of projects that
  are not open" in Settings, instead of "Switching projects".
- Pulling in a repository whose git config makes pulls rebase (`pull.rebase`) with
  uncommitted changes offers "Stash and pull", instead of showing git's "cannot pull with
  rebase" error.
- In the Log graph, a line from an unpushed commit to its parent stays dimmed all the way
  down, instead of lighting up where it runs past commits that are already pushed.
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

[Unreleased]: https://github.com/ReyArlena/osier/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/ReyArlena/osier/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/ReyArlena/osier/releases/tag/v0.1.0
