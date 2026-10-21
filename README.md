# Osier

A fast, native git GUI for Linux. Built in Rust on [gpui](https://www.gpui.rs/) through
[gpui-kit](https://github.com/longbridge/gpui-kit), with the git workflow of a JetBrains IDE and
none of the IDE.

![Changes tab with a diff](docs/screenshots/changes.png)

## Features

- **Visual staging**: stage, unstage and discard whole files, single hunks or selected lines
  from the diff; untracked files go to the trash, never `rm`.
- **Commit log with graph**, tags and branch labels, reflog, compare any two refs, file
  history (`log --follow`) and blame with a click through to the commit.
- **Branches, tags, remotes, worktrees, submodules, stashes** in one place, with the usual
  create/rename/delete/checkout actions and confirmations where data could be lost.
- **Fetch, pull, push** with progress and cancel, upstream prompts, pull strategy memory per
  repository, `--force-with-lease` only (never plain `--force`).
- **Merge, rebase, cherry-pick, revert, interactive rebase** with conflicts resolved in place:
  accept current / incoming / both per block, or edit the result directly.
- **Patches**: copy files or commits as a patch, apply from clipboard or file with a dry run
  and a three-way fallback, and a patch clipboard that shelves changes and carries them
  between projects.
- **Multiple GitHub accounts** through the [gh CLI](https://cli.github.com/): pick an account
  per project, remember it, and never let a token touch the command line or a log.
- **Files tab** with code, markdown (rendered or source) and image viewing at any revision.
- **Themes**: Catppuccin Mocha and Latte, your own gpui-kit theme files, and optional live
  colours from [Dux Shell](https://github.com/AvengeMedia/DankMaterialShell).
- **Everything is visible**: a log of every git command Osier runs, an operation history,
  and a command palette for every action.

![Log tab with the commit graph](docs/screenshots/log.png)

## Install

Osier needs Linux (Wayland, X11 fallback), `git` 2.30 or newer and a GPU driver with Vulkan.

**Script** (prebuilt binary, falls back to a cargo build):

```sh
curl -fsSL https://raw.githubusercontent.com/ReyArlena/osier/main/install.sh | sh
```

**Arch Linux**: PKGBUILDs for `osier` and `osier-bin` live in `packaging/aur/` until the
packages are on the AUR.

**From source**:

```sh
# Debian/Ubuntu build deps:
#   pkg-config libxkbcommon-dev libxkbcommon-x11-dev libxcb-xkb-dev libwayland-dev \
#   libvulkan-dev libfontconfig1-dev libssl-dev
# Arch: pkgconf libxkbcommon libxkbcommon-x11 wayland vulkan-headers fontconfig openssl
cargo install --locked --git https://github.com/ReyArlena/osier osier
```

A Flatpak manifest draft is in `packaging/flatpak/`.

## Usage

```
osier [path]      open a repository (or start with the last session)
osier --version --verbose
```

Open repositories from the sidebar (`ctrl-o`), or let Osier scan directories for them
(Settings > Projects). Useful keys: `ctrl-1`..`ctrl-6` switch tabs, `ctrl-enter` commits,
`ctrl-shift-f/l/p` fetch/pull/push, `` ctrl-` `` shows the git command log, `ctrl-p` or
`F1` opens the command palette, `ctrl-,` opens settings, `/` searches projects, `?` searches
files (Esc clears a search). Plain keys like `/`, `j` or space never fire while you type in a
text field.

The project filter also takes `is:changed` (uncommitted changes), `is:ahead` (unpushed
commits) and `is:behind` (new commits on the remote), alone or with a name: `is:ahead api`.
Clicking the filter lists them. Projects that are not open only match when Settings >
Projects > Show status is on.

Every shortcut can be changed in Settings > Keymap or in `~/.config/osier/keymap.toml`, which
holds only your changes and is reloaded when it changes:

```toml
[bindings]
refresh = ["f5"]                      # replaces ctrl-r and f5
focus_project_search = ["/", "ctrl-f"]
open_settings = []                    # unbound
prefix_key = ["ctrl-h"]               # a leader key (unset by default)
focus_file_search = ["prefix ?"]      # only after the prefix: ctrl-h, then ?
tab_log = ["prefix? l"]               # either: l alone, or ctrl-h then l
row_add_to_gitignore = ["i"]          # context menu entry on the selected row
```

**Prefix key.** Press it, then a binding's key. While it waits the status bar shows "Prefix
key pressed"; Escape, a click, an unbound key or the timeout (5 seconds by default) cancel it.
Each key is set to No prefix, Prefix only or Either with three buttons on the Keymap page, and
keys after the prefix also work while typing in a text field.

**Context menus.** Every right-click entry (Add to .gitignore, Delete / Drop, Cherry-pick, ...)
can have a key that runs it on the selected row of Changes, Files, Log or Patches. These keys
start unbound and cannot share a key with any other binding.

If you use Dux Shell (DankMaterialShell), Osier writes its shortcuts to
`~/.config/dms/cheatsheets/osier.json`, so `dms keybinds show osier` and the DMS keybinds
overlay list them.

![Settings sheet](docs/screenshots/settings.png)

## Configuration

Settings live in `~/.config/osier/config.toml` and are safe to keep in dotfiles: the file is
re-read when it changes. Every key is optional.

```toml
[appearance]
mode = "dark"                 # dark | light | system
dark_theme = "Catppuccin Mocha"
light_theme = "Catppuccin Latte"
dux_shell_sync = false        # follow Dux Shell colours when its cache exists
# ui_font = "Inter"
# mono_font = "JetBrains Mono"
# font_size = 14.0
changes_indicator = "icon"    # icon | dot: how the sidebar marks uncommitted changes
row_menu_buttons = false      # show a ... menu button on list rows (right-click always works)
tooltip_delay_ms = 0          # how long to hover before a tooltip shows (0 to 1000)

[keys]
prefix_timeout_secs = 5       # how long the prefix key waits; 0 waits until a key

[editor]
# command = "zed {file}:{line}"   # {path} = repository, {file}, {line}; empty = xdg-open
# terminal = "kitty --directory {path}"

[projects]
scan_dirs = ["~/Projects", "~/Work"]   # children listed under Recents
scan_depth = 1
recent_status = "never"       # badges on projects that are not open:
                              # never | cached | watch | check
recent_status_minutes = 2     # check mode sweep interval, 1 to 60

[diff]
context_lines = 3
line_numbers = true
soft_wrap = true
mode = "unified"              # unified | split
large_file_bytes = 2097152

[git]
log_page_size = 500
show_command_log = false

[auth]
askpass = true                # route ssh/git prompts to Osier's own window
[auth.default_accounts]
# "github.com" = "your-login"

[viewer]
remote_images = true          # fetch http(s) images in markdown
```

State (open projects, recents, window layout, per-project account choices) is in
`~/.local/state/osier/`, the patch clipboard in `~/.local/state/osier/patches/`.

## Themes

Drop any gpui-kit theme JSON into `~/.config/osier/themes/` and pick it from the theme menu.
`themes/template.json` is a starting point and [docs/themes.md](docs/themes.md) lists every
token Osier uses and how Dux Shell colours are mapped.

## FAQ

**Which git does it use?** The `git` on your `PATH`, with your config, hooks, aliases and
credential helpers. Osier never bundles or replaces git, and every command it runs is shown in
the command log.

**How do multiple GitHub accounts work?** Osier reads the accounts `gh auth status` knows
about. For an https remote it asks which one to use (or uses the one you saved for that
project) and hands git a credential helper that fetches the token from `gh` inside a helper
process. Tokens never appear in argv, logs or files. ssh remotes use your keys as usual.

**Why does a push ask about `--force-with-lease`?** Osier never runs plain `--force`. When a
push is rejected it offers a lease-protected force, which fails if the remote moved since you
last fetched.

**Does it work on X11?** gpui supports X11, and Osier runs there, but development and testing
happen on Wayland (Hyprland).

**Where is the three-pane merge tool?** Conflicts are resolved inside the diff pane: each
conflict block has accept-current / accept-incoming / accept-both controls, and the result can
be edited in place before it is marked resolved.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Bug reports are most useful with the output of
`osier --version --verbose` and the relevant lines from the command log.

## License

MIT. Catppuccin palette by [Catppuccin](https://github.com/catppuccin/catppuccin) (MIT); icons
from [Lucide](https://lucide.dev/) via gpui-kit (ISC).
