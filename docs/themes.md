# Themes

Osier themes are [gpui-kit](https://github.com/longbridge/gpui-kit) theme files: one JSON
document holding one or more themes. Drop a file into `~/.config/osier/themes/` and it appears in
the theme menu (the sun/moon button in the title bar) straight away; edits to the file are
applied live.

## Picking themes

The theme menu has four parts:

- **Mode**: Dark, Light or System. System follows the desktop (and, with Dux Shell sync on,
  Dux Shell's own mode).
- **Dark theme** and **Light theme**: one choice each, so toggling mode keeps your picks.
  `ctrl-shift-t` toggles between the two.
- **Follow Dux Shell colours**: generate the theme from DankMaterialShell's colour cache
  (see below). Picking an explicit theme turns this off again.
- **Open themes folder**.

The same choices live in `~/.config/osier/config.toml`:

```toml
[appearance]
mode = "system"           # dark | light | system (default)
system_fallback = "dark"  # system mode with no stated desktop preference
dark_theme = "Catppuccin Mocha"
light_theme = "Catppuccin Latte"
# dux_shell_sync = true  # unset: on when Dux Shell is detected
ui_font = "Inter"         # optional, overrides the theme's font
mono_font = "JetBrains Mono"
font_size = 14.0          # optional, UI and mono size
```

## Bundled themes

- `Catppuccin Mocha` and `Catppuccin Latte` are generated from the official Catppuccin palette
  by the `osier-theme` crate (`themes/osier-catppuccin.json`). They are the defaults and the
  fallback when a configured theme is missing.
- `Catppuccin Frappe` and `Catppuccin Macchiato` come from gpui-kit's own Catppuccin file.

A user file with the same theme name as a bundled theme replaces it.

## Writing a theme

Start from `themes/template.json`, or copy `themes/osier-catppuccin.json` for every token.
Omitted colour tokens fall back to gpui-kit's defaults for the theme's `mode`, so a minimal
theme only needs the tokens it wants to change.

```json
{
  "name": "My themes",
  "author": "me",
  "themes": [
    {
      "name": "My Dark",
      "mode": "dark",
      "font.family": "Inter",
      "mono_font.family": "JetBrains Mono",
      "colors": { "background": "#1e1e2e", "...": "..." },
      "highlight": { "created": "#a6e3a1", "deleted": "#f38ba8", "...": "..." }
    }
  ]
}
```

Colours are `#rrggbb` or `#rrggbbaa`.

### Colour tokens Osier uses

| Token | Where |
|-------|-------|
| `background`, `foreground` | Content panes, body text |
| `title_bar.background`, `title_bar.border` | Title bar |
| `sidebar.background`, `sidebar.foreground`, `sidebar.border`, `sidebar.accent.background` | Project sidebar and its active row |
| `tab_bar.background`, `tab.foreground`, `tab.active.background`, `tab.active.foreground` | Center tabs |
| `status_bar.background`, `status_bar.border` | Status bar |
| `panel.background`, `popover.background`, `popover.foreground` | Command log, menus, dialogs |
| `border`, `input.border`, `ring` | Separators, inputs, focus ring |
| `muted.background`, `muted.foreground` | Counts, hints, secondary text |
| `primary.*`, `secondary.*`, `danger.*`, `success.*`, `warning.*`, `info.*` | Buttons and tags (`.background`, `.foreground`, `.hover.background`, `.active.background`) |
| `link.foreground`, `link.hover.foreground` | Links in markdown |
| `list.background`, `list.even.background`, `list.head.background`, `list.hover.background`, `list.active.background`, `list.active.border` | Changes, log, branches lists |
| `scrollbar.background`, `scrollbar.thumb.background`, `scrollbar.thumb.hover.background` | Scrollbars |
| `selection.background`, `caret` | Text selection and cursor |
| `base.red`, `base.green`, `base.yellow`, `base.blue`, `base.cyan`, `base.magenta` (+ `.light`) | Status badges, graph lanes |
| `chart.1` .. `chart.5` | Commit graph branch colours |
| `window.border` | Window border |

The full list, including `button.*`, `switch.*`, `table.*` and the rest of the component
tokens, is gpui-kit's schema: `crates/component/src/theme/schema.rs` in the gpui-kit source.

### Highlight tokens (diff and editor)

| Token | Where |
|-------|-------|
| `created`, `deleted`, `modified`, `renamed`, `conflict` | Diff line backgrounds (at reduced alpha), status letters, conflict blocks |
| `editor.background`, `editor.foreground`, `editor.line_number`, `editor.active_line_number`, `editor.active_line.background`, `editor.invisible` | Diff pane and file viewer |
| `error`, `warning`, `info`, `hint`, `success` | Inline messages |
| `syntax.<scope>` | Syntax colours: `{"color": "#..", "font_style": "italic", "font_weight": 700}` for `keyword`, `string`, `comment`, `function`, `type`, `number`, `property`, `punctuation`, `variable`, ... |

## Dux Shell sync

Sync is on by default when Dux Shell is detected (unless `dux_shell_sync = false`). Turning it
on in Settings sets the mode to System, so the shell's light or dark decides; you can still pick
Dark or Light afterwards. While it is on, the theme pickers are off. With sync on Osier reads `$XDG_CACHE_HOME/dms/dms-colors.json` (falling back to
the older `DankMaterialShell/` directory) and `dms-typography.json` next to it, and builds two
themes, `Dux Shell Dark` and `Dux Shell Light`. The files are watched, so a wallpaper change in
Dux Shell recolours Osier within half a second. If the files are missing the toggle is shown
as "not detected" and the configured Catppuccin themes are used.

Mapping (Material token on the left):

| Dux Shell | Osier |
|-----------|-------|
| `background` | `background`, `list.background`, `tab.active.background` |
| `surface_container_low` (or `surface`) | sidebar, title bar, status bar, tab bar |
| `surface_container_lowest` (or `surface_dim`) | `window.border` |
| `surface_container` | panels, popovers, inputs, `muted.background` |
| `surface_container_high` / `_highest` | `border`, hover and active surfaces |
| `on_surface`, `on_surface_variant`, `outline` | text, muted text, line numbers |
| `primary`, `on_primary` | primary buttons, selection, focus ring |
| `tertiary` | links |
| `dank16` color1/2/3/4/5/6/9 (else `error`, `tertiary`, `secondary`) | red, green, yellow, blue, magenta, cyan, peach/maroon |
| `fontFamily`, `monoFontFamily` | UI and mono font, unless set in `config.toml` |
