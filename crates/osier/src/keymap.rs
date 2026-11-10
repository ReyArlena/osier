//! Every keyboard action Osier offers, with its default keys and where it applies.
//!
//! One registry feeds the key bindings, the command palette's shortcut hints, the settings
//! Keymap page, `keymap.toml` overrides and the DMS cheatsheet, so they cannot drift apart.
//!
//! Scope rules (the reason this exists): a binding never fires inside a text field unless it
//! uses a modifier (ctrl, alt, super) or is a function key. gpui-kit gives every text field the
//! key context `Input`, so plain keys get `!Input` in their context: typing `/`, `j` or a space
//! into the commit message, a filter or the editor types the character.

use gpui_kit::*;

use crate::app::*;

/// Where an action applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scope {
    /// Anywhere in the main window.
    App,
    /// The centre lists (Changes, Log, ...): context `OsierList`.
    List,
    /// The settings dialog: context `OsierSettings`.
    Settings,
    /// A context-menu action on the selected row of a centre list (Changes, Files, Log,
    /// Patches). Same context as `List`; its keys may not clash with any other binding.
    Row,
}

/// How a key relates to the prefix (leader) key. Stored in `keymap.toml` as a leading token:
/// `"/"` (no prefix), `"prefix /"` (prefix only), `"prefix? /"` (either).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeyMode {
    /// Fires on the key alone.
    Plain,
    /// Fires only after the prefix key.
    Prefix,
    /// Fires on the key alone or after the prefix key.
    Either,
}

impl KeyMode {
    pub(crate) const ALL: [KeyMode; 3] = [KeyMode::Plain, KeyMode::Prefix, KeyMode::Either];

    /// The key alone fires it.
    pub(crate) fn plain(self) -> bool {
        self != KeyMode::Prefix
    }

    /// The key after the prefix fires it.
    pub(crate) fn prefixed(self) -> bool {
        self != KeyMode::Plain
    }
}

/// `"prefix /"` as `(Prefix, "/")`.
pub(crate) fn split_mode(keys: &str) -> (KeyMode, &str) {
    let t = keys.trim();
    if let Some(rest) = t.strip_prefix("prefix? ") {
        (KeyMode::Either, rest.trim())
    } else if let Some(rest) = t.strip_prefix("prefix ") {
        (KeyMode::Prefix, rest.trim())
    } else {
        (KeyMode::Plain, t)
    }
}

/// The stored form of `key` with `mode`.
pub(crate) fn with_mode(mode: KeyMode, key: &str) -> String {
    match mode {
        KeyMode::Plain => key.to_string(),
        KeyMode::Prefix => format!("prefix {key}"),
        KeyMode::Either => format!("prefix? {key}"),
    }
}

/// Id of the prefix (leader) key's own action.
pub(crate) const PREFIX_KEY: &str = "prefix_key";

pub(crate) struct ActionSpec {
    /// Stable id used in `keymap.toml`.
    pub id: &'static str,
    pub name: &'static str,
    pub category: &'static str,
    pub defaults: &'static [&'static str],
    pub scope: Scope,
    /// Where it applies, shown under the name on the Keymap page ("" for none).
    pub detail: &'static str,
    pub make: fn() -> Box<dyn Action>,
}

macro_rules! spec {
    ($id:literal, $name:literal, $cat:literal, [$($key:literal),*], $scope:ident, $action:expr) => {
        ActionSpec {
            id: $id,
            name: $name,
            category: $cat,
            defaults: &[$($key),*],
            scope: Scope::$scope,
            detail: "",
            make: || Box::new($action),
        }
    };
}

/// The registry, in display order.
pub(crate) fn actions() -> Vec<ActionSpec> {
    let mut all = vec![
        spec!("prefix_key", "Prefix key", "Prefix", [], App, PrefixKey),
        spec!("quit", "Quit", "General", ["ctrl-q"], App, Quit),
        spec!(
            "refresh",
            "Refresh",
            "General",
            ["ctrl-r", "f5"],
            App,
            Refresh
        ),
        spec!(
            "open_repository",
            "Open repository",
            "General",
            ["ctrl-shift-o"],
            App,
            OpenRepository
        ),
        spec!(
            "clone_repository",
            "Clone repository",
            "General",
            [],
            App,
            CloneRepository
        ),
        spec!(
            "create_repository",
            "Create new repository",
            "General",
            [],
            App,
            CreateRepository
        ),
        spec!(
            "close_project",
            "Close project",
            "General",
            ["ctrl-w"],
            App,
            CloseProject
        ),
        spec!(
            "command_palette",
            "Command palette",
            "General",
            ["ctrl-p", "f1"],
            App,
            OpenCommandPalette
        ),
        spec!(
            "switch_project",
            "Switch project",
            "General",
            ["ctrl-o"],
            App,
            SwitchProject
        ),
        spec!(
            "open_settings",
            "Settings",
            "General",
            ["ctrl-,"],
            App,
            OpenSettings
        ),
        spec!(
            "toggle_sidebar",
            "Toggle sidebar",
            "View",
            ["ctrl-b"],
            App,
            ToggleSidebar
        ),
        spec!(
            "toggle_command_log",
            "Toggle git command log",
            "View",
            ["ctrl-`"],
            App,
            ToggleCommandLog
        ),
        spec!(
            "toggle_notifications",
            "Toggle notifications",
            "View",
            ["ctrl-shift-n"],
            App,
            ToggleNotifications
        ),
        spec!(
            "toggle_commit_details",
            "Toggle commit details",
            "View",
            ["ctrl-shift-d"],
            App,
            ToggleCommitDetails
        ),
        spec!(
            "toggle_diff_expanded",
            "Expand the diff pane",
            "View",
            ["ctrl-shift-m"],
            App,
            ToggleDiffExpanded
        ),
        spec!(
            "toggle_theme",
            "Toggle dark / light",
            "View",
            ["ctrl-shift-t"],
            App,
            ToggleTheme
        ),
        spec!(
            "focus_project_search",
            "Search projects",
            "Search",
            ["/"],
            App,
            FocusProjectSearch
        ),
        spec!(
            "focus_file_search",
            "Search files",
            "Search",
            ["?", "shift-/"],
            App,
            FocusFileSearch
        ),
        spec!(
            "focus_settings_search",
            "Search settings",
            "Search",
            ["/"],
            Settings,
            FocusSettingsSearch
        ),
        spec!(
            "tab_changes",
            "Changes tab",
            "Tabs",
            ["ctrl-1"],
            App,
            TabChanges
        ),
        spec!("tab_log", "Log tab", "Tabs", ["ctrl-2"], App, TabLog),
        spec!(
            "tab_branches",
            "Branches tab",
            "Tabs",
            ["ctrl-3"],
            App,
            TabBranches
        ),
        spec!(
            "tab_stashes",
            "Stashes tab",
            "Tabs",
            ["ctrl-4"],
            App,
            TabStashes
        ),
        spec!("tab_files", "Files tab", "Tabs", ["ctrl-5"], App, TabFiles),
        spec!(
            "tab_patches",
            "Patches tab",
            "Tabs",
            ["ctrl-6"],
            App,
            TabPatches
        ),
        spec!(
            "commit",
            "Commit staged",
            "Changes",
            ["ctrl-enter"],
            App,
            CommitStaged
        ),
        spec!(
            "toggle_stage",
            "Stage / unstage selected file",
            "Changes",
            ["space"],
            List,
            ToggleStageSelected
        ),
        spec!(
            "discard",
            "Discard selected file",
            "Changes",
            ["delete"],
            List,
            DiscardSelected
        ),
        spec!(
            "select_next",
            "Next item",
            "Lists",
            ["down", "j"],
            List,
            SelectNext
        ),
        spec!(
            "select_prev",
            "Previous item",
            "Lists",
            ["up", "k"],
            List,
            SelectPrev
        ),
        spec!(
            "update_branch",
            "Update branch",
            "Remote",
            ["ctrl-t"],
            App,
            UpdateBranch
        ),
        spec!("fetch", "Fetch", "Remote", ["ctrl-shift-f"], App, Fetch),
        spec!("pull", "Pull", "Remote", ["ctrl-shift-l"], App, Pull),
        spec!("push", "Push", "Remote", ["ctrl-shift-p"], App, Push),
    ];
    all.extend(crate::app::row_actions::specs());
    all
}

/// A keystroke that cannot type into a text field: a modifier chord or a function key.
pub(crate) fn works_in_text_fields(keys: &str) -> bool {
    let first = keys.split_whitespace().next().unwrap_or("");
    let lower = first.to_ascii_lowercase();
    ["ctrl-", "alt-", "super-", "cmd-", "platform-", "fn-"]
        .iter()
        .any(|m| lower.contains(m))
        || lower.rsplit('-').next().is_some_and(|k| {
            k.len() > 1 && k.starts_with('f') && k[1..].chars().all(|c| c.is_ascii_digit())
        })
}

/// The key context predicate a binding gets.
pub(crate) fn context_for(scope: Scope, keys: &str) -> Option<&'static str> {
    let anywhere = works_in_text_fields(keys);
    match (scope, anywhere) {
        (Scope::App, true) => None,
        (Scope::App, false) => Some("Osier && !Input"),
        (Scope::List | Scope::Row, true) => Some("OsierList"),
        (Scope::List | Scope::Row, false) => Some("OsierList && !Input"),
        (Scope::Settings, true) => Some("OsierSettings"),
        (Scope::Settings, false) => Some("OsierSettings && !Input"),
    }
}

/// gpui key bindings for `(spec, keys)` pairs. Invalid keystrokes are skipped with a warning.
pub(crate) fn bindings<'a>(
    pairs: impl IntoIterator<Item = (&'a ActionSpec, &'a str)>,
    cx: &App,
) -> Vec<KeyBinding> {
    let mut out = Vec::new();
    for (spec, keys) in pairs {
        let predicate = context_for(spec.scope, keys)
            .map(|c| KeyBindingContextPredicate::parse(c).map(std::rc::Rc::new));
        let predicate = match predicate.transpose() {
            Ok(p) => p,
            Err(error) => {
                tracing::warn!(%error, action = spec.id, "bad key context");
                continue;
            }
        };
        match KeyBinding::load(keys, (spec.make)(), predicate, false, None, cx) {
            Ok(b) => out.push(b),
            Err(error) => tracing::warn!(%error, action = spec.id, keys, "invalid keystroke"),
        }
    }
    out
}

/// gpui-kit's own bindings (inputs, menus, dialogs), saved before ours are added so a keymap
/// reload can clear everything and restore them.
struct KitBindings(Vec<KeyBinding>);
impl Global for KitBindings {}

/// The keys each action has right now (defaults plus `keymap.toml`), and problems found.
#[derive(Debug, Default, Clone)]
pub(crate) struct Keymap {
    /// Stored forms, mode token included (`"prefix /"`).
    pub keys: std::collections::BTreeMap<&'static str, Vec<String>>,
    /// Keys that fire after the prefix key, with their action ids.
    pub prefixed: Vec<(String, &'static str)>,
    pub problems: Vec<String>,
}
impl Global for Keymap {}

impl Keymap {
    pub fn global(cx: &App) -> &Self {
        cx.global::<Self>()
    }

    /// Keys for an action id, empty when unbound or unknown.
    pub fn keys_for(&self, id: &str) -> &[String] {
        self.keys.get(id).map(Vec::as_slice).unwrap_or(&[])
    }
}

/// First-time setup: remember gpui-kit's bindings, then apply the defaults.
pub(crate) fn init(cx: &mut App) {
    if !cx.has_global::<KitBindings>() {
        let kit: Vec<KeyBinding> = cx.key_bindings().borrow().bindings().cloned().collect();
        cx.set_global(KitBindings(kit));
    }
    apply(&osier_config::keymap::KeymapFile::default(), cx);
}

/// Apply defaults with `overrides` on top. Unknown action ids and invalid keys are reported in
/// `Keymap::problems` and skipped; everything valid still applies.
pub(crate) fn apply(overrides: &osier_config::keymap::KeymapFile, cx: &mut App) {
    let specs = actions();
    let mut keymap = Keymap::default();
    for id in overrides.bindings.keys() {
        if !specs.iter().any(|s| s.id == id) {
            keymap.problems.push(format!("unknown action \"{id}\""));
        }
    }
    for spec in &specs {
        let keys: Vec<String> = match overrides.bindings.get(spec.id) {
            Some(k) => k
                .iter()
                .map(|k| k.trim().to_string())
                .filter(|k| !k.is_empty())
                .collect(),
            None => spec.defaults.iter().map(|k| k.to_string()).collect(),
        };
        let mut valid = Vec::new();
        for k in keys {
            let (mode, key) = split_mode(&k);
            // The prefix key itself is always a plain key.
            let k = if spec.id == PREFIX_KEY {
                key.to_string()
            } else {
                k.clone()
            };
            if !plausible(key) || bindings([(spec, key)], cx).is_empty() {
                keymap
                    .problems
                    .push(format!("invalid key \"{k}\" for {}", spec.id));
            } else {
                if mode.prefixed() && spec.id != PREFIX_KEY {
                    keymap.prefixed.push((key.to_ascii_lowercase(), spec.id));
                }
                valid.push(k);
            }
        }
        keymap.keys.insert(spec.id, valid);
    }
    let ours: Vec<KeyBinding> = {
        let pairs: Vec<(&ActionSpec, &str)> = specs
            .iter()
            .flat_map(|s| {
                keymap.keys_for(s.id).iter().filter_map(move |k| {
                    let (mode, key) = split_mode(k);
                    (mode.plain() || s.id == PREFIX_KEY).then_some((s, key))
                })
            })
            .collect();
        bindings(pairs, cx)
    };
    let kit = cx
        .try_global::<KitBindings>()
        .map(|k| k.0.clone())
        .unwrap_or_default();
    cx.clear_key_bindings();
    cx.bind_keys(kit);
    cx.bind_keys(ours);
    cx.set_global(keymap);
    // Menus and tooltips in the cached main view show these keys: redraw past the cache.
    cx.refresh_windows();
}

/// A recorded keystroke in gpui notation (`ctrl-shift-p`, `?`, `f5`). `None` for a modifier
/// pressed on its own. A shifted symbol keeps its symbol (`?`, not `shift-/`).
pub(crate) fn keystroke_string(k: &Keystroke) -> Option<String> {
    let key = k.key.to_ascii_lowercase();
    if matches!(
        key.as_str(),
        "control" | "ctrl" | "shift" | "alt" | "platform" | "super" | "cmd" | "function" | "fn"
    ) || key.is_empty()
    {
        return None;
    }
    let m = k.modifiers;
    let mut s = String::new();
    if m.control {
        s.push_str("ctrl-");
    }
    if m.alt {
        s.push_str("alt-");
    }
    if m.platform {
        s.push_str("super-");
    }
    if m.function {
        s.push_str("fn-");
    }
    let symbol = key.chars().count() == 1 && !key.chars().all(|c| c.is_ascii_alphanumeric());
    if m.shift && !symbol {
        s.push_str("shift-");
    }
    s.push_str(&key);
    Some(s)
}

/// Other actions sharing `keys` (stored form, mode included) with `id` where both can fire:
/// overlapping window areas and a shared namespace (alone, or after the prefix). The prefix
/// key counts as a plain key.
pub(crate) fn conflicts(keymap: &Keymap, id: &str, keys: &str) -> Vec<&'static str> {
    conflicting(keymap, id, keys)
        .into_iter()
        .map(|s| s.name)
        .collect()
}

/// Like [`conflicts`], but only the clashes Osier refuses: any involving a context-menu
/// action or the prefix key. (Clashes between other actions are allowed with a warning.)
pub(crate) fn refused(keymap: &Keymap, id: &str, keys: &str) -> Vec<&'static str> {
    let specs = actions();
    let strict = |s: &ActionSpec| s.scope == Scope::Row || s.id == PREFIX_KEY;
    let mine_strict = specs.iter().find(|s| s.id == id).is_some_and(strict);
    conflicting(keymap, id, keys)
        .into_iter()
        .filter(|s| mine_strict || strict(s))
        .map(|s| s.name)
        .collect()
}

fn conflicting(keymap: &Keymap, id: &str, keys: &str) -> Vec<ActionSpec> {
    let specs = actions();
    let mode_of = |id: &str, k: &str| {
        if id == PREFIX_KEY {
            (KeyMode::Plain, split_mode(k).1.to_ascii_lowercase())
        } else {
            let (m, key) = split_mode(k);
            (m, key.to_ascii_lowercase())
        }
    };
    let Some(mine) = specs.iter().find(|s| s.id == id).map(|s| s.scope) else {
        return Vec::new();
    };
    let overlaps = |a: Scope, b: Scope| {
        let main = |s| matches!(s, Scope::App | Scope::List | Scope::Row);
        (main(a) && main(b)) || (a == Scope::Settings && b == Scope::Settings)
    };
    let (mode, key) = mode_of(id, keys);
    specs
        .into_iter()
        .filter(|s| s.id != id && overlaps(mine, s.scope))
        .filter(|s| {
            keymap.keys_for(s.id).iter().any(|k| {
                let (m2, k2) = mode_of(s.id, k);
                k2 == key && ((mode.plain() && m2.plain()) || (mode.prefixed() && m2.prefixed()))
            })
        })
        .collect()
}

/// gpui accepts any key name, so a typo such as `ctrl-shft-p` would load and never fire.
/// Every stroke must be modifiers followed by one character or a known key name.
pub(crate) fn plausible(keys: &str) -> bool {
    const NAMED: &[&str] = &[
        "escape",
        "enter",
        "tab",
        "space",
        "backspace",
        "delete",
        "insert",
        "home",
        "end",
        "pageup",
        "pagedown",
        "up",
        "down",
        "left",
        "right",
    ];
    let strokes: Vec<&str> = split_mode(keys).1.split_whitespace().collect();
    !strokes.is_empty()
        && strokes.iter().all(|stroke| {
            let mut rest = *stroke;
            for m in [
                "ctrl-",
                "alt-",
                "shift-",
                "super-",
                "cmd-",
                "platform-",
                "fn-",
            ] {
                while rest.len() > m.len() && rest.to_ascii_lowercase().starts_with(m) {
                    rest = &rest[m.len()..];
                }
            }
            let key = rest.to_ascii_lowercase();
            rest.chars().count() == 1
                || NAMED.contains(&key.as_str())
                || (key.len() > 1
                    && key.len() <= 3
                    && key.starts_with('f')
                    && key[1..].chars().all(|c| c.is_ascii_digit()))
        })
}

/// The first key bound to action `id`, pretty-printed for hints and tooltips; `None` when
/// unbound.
pub(crate) fn hint(id: &str, cx: &App) -> Option<String> {
    Keymap::global(cx).keys_for(id).first().map(|k| pretty(k))
}

/// `ctrl-shift-p` as `Ctrl+Shift+P`, `escape` as `Esc`, sequences joined by spaces. The form
/// shown in the palette, the Keymap page and the DMS cheatsheet.
pub(crate) fn pretty(keys: &str) -> String {
    let (mode, key) = split_mode(keys);
    let plain = pretty_key(key);
    match mode {
        KeyMode::Plain => plain,
        KeyMode::Prefix => format!("Prefix {plain}"),
        KeyMode::Either => format!("{plain} or Prefix {plain}"),
    }
}

/// The key itself, without its prefix mode.
pub(crate) fn pretty_key(key: &str) -> String {
    key.split_whitespace()
        .map(pretty_stroke)
        .collect::<Vec<_>>()
        .join(" ")
}

fn pretty_stroke(stroke: &str) -> String {
    let mut rest = stroke;
    let mut parts: Vec<String> = Vec::new();
    loop {
        let lower = rest.to_ascii_lowercase();
        let found = [
            ("ctrl-", "Ctrl"),
            ("alt-", "Alt"),
            ("shift-", "Shift"),
            ("super-", "Super"),
            ("cmd-", "Super"),
            ("platform-", "Super"),
            ("fn-", "Fn"),
        ]
        .iter()
        .find(|(p, _)| lower.starts_with(p) && rest.len() > p.len());
        match found {
            Some((p, name)) => {
                parts.push(name.to_string());
                rest = &rest[p.len()..];
            }
            None => break,
        }
    }
    let key = match rest.to_ascii_lowercase().as_str() {
        "escape" => "Esc".to_string(),
        "delete" => "Del".to_string(),
        "backspace" => "Backspace".to_string(),
        "enter" => "Enter".to_string(),
        "space" => "Space".to_string(),
        "tab" => "Tab".to_string(),
        "up" => "Up".to_string(),
        "down" => "Down".to_string(),
        "left" => "Left".to_string(),
        "right" => "Right".to_string(),
        "pageup" => "PgUp".to_string(),
        "pagedown" => "PgDn".to_string(),
        "home" => "Home".to_string(),
        "end" => "End".to_string(),
        k if k.len() > 1 && k.starts_with('f') && k[1..].chars().all(|c| c.is_ascii_digit()) => {
            k.to_uppercase()
        }
        _ if rest.chars().count() == 1 => rest.to_uppercase(),
        _ => rest.to_string(),
    };
    parts.push(key);
    parts.join("+")
}

#[cfg(test)]
mod tests {
    use super::{Scope, actions, context_for, works_in_text_fields};

    #[test]
    fn plain_keys_never_apply_in_text_fields() {
        assert!(works_in_text_fields("ctrl-r"));
        assert!(works_in_text_fields("f5"));
        assert!(works_in_text_fields("ctrl-shift-a"));
        assert!(!works_in_text_fields("/"));
        assert!(!works_in_text_fields("shift-/"));
        assert!(!works_in_text_fields("space"));
        assert!(!works_in_text_fields("delete"));
        assert!(!works_in_text_fields("f"), "a plain f is a letter");
        assert_eq!(context_for(Scope::App, "/"), Some("Osier && !Input"));
        assert_eq!(context_for(Scope::App, "ctrl-r"), None);
        assert_eq!(context_for(Scope::List, "j"), Some("OsierList && !Input"));
    }

    #[test]
    fn plausible_keys() {
        use super::plausible;
        for ok in [
            "ctrl-shift-p",
            "/",
            "shift-/",
            "?",
            "f5",
            "f12",
            "ctrl-`",
            "ctrl--",
            "space",
            "ctrl-k ctrl-s",
            "alt-enter",
        ] {
            assert!(plausible(ok), "{ok}");
        }
        for bad in ["ctrl-??", "ctrl-shft-p", "", "ctrl-", "f99x", "escpe"] {
            assert!(!plausible(bad), "{bad}");
        }
    }

    #[test]
    fn pretty_keys() {
        use super::pretty;
        assert_eq!(pretty("ctrl-shift-p"), "Ctrl+Shift+P");
        assert_eq!(pretty("escape"), "Esc");
        assert_eq!(pretty("shift-/"), "Shift+/");
        assert_eq!(pretty("ctrl-`"), "Ctrl+`");
        assert_eq!(pretty("ctrl--"), "Ctrl+-");
        assert_eq!(pretty("f5"), "F5");
        assert_eq!(pretty("j"), "J");
        assert_eq!(pretty("ctrl-k ctrl-s"), "Ctrl+K Ctrl+S");
        assert_eq!(pretty("prefix /"), "Prefix /");
        assert_eq!(pretty("prefix? i"), "I or Prefix I");
    }

    #[test]
    fn key_modes_round_trip() {
        use super::{KeyMode, split_mode, with_mode};
        for mode in KeyMode::ALL {
            let stored = with_mode(mode, "ctrl-x");
            assert_eq!(split_mode(&stored), (mode, "ctrl-x"));
        }
        assert!(super::plausible("prefix /"));
        assert!(super::plausible("prefix? ctrl-k"));
        assert!(!super::plausible("prefix ctrl-shft-x"));
    }

    #[test]
    fn ids_are_unique_and_every_action_has_a_default() {
        let all = actions();
        let mut ids: Vec<&str> = all.iter().map(|a| a.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), all.len());
        // Context-menu actions, the prefix key, clone and create start unbound; everything
        // else has keys.
        let unbound = [super::PREFIX_KEY, "clone_repository", "create_repository"];
        assert!(
            all.iter()
                .filter(|a| a.scope != Scope::Row && !unbound.contains(&a.id))
                .all(|a| !a.defaults.is_empty())
        );
        assert!(
            all.iter()
                .filter(|a| unbound[1..].contains(&a.id))
                .all(|a| a.defaults.is_empty())
        );
    }
}
