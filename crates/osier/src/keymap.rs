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
}

pub(crate) struct ActionSpec {
    /// Stable id used in `keymap.toml`.
    pub id: &'static str,
    // Read by the settings Keymap page and the DMS cheatsheet writer.
    #[allow(dead_code)]
    pub name: &'static str,
    #[allow(dead_code)]
    pub category: &'static str,
    pub defaults: &'static [&'static str],
    pub scope: Scope,
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
            make: || Box::new($action),
        }
    };
}

/// The registry, in display order.
pub(crate) fn actions() -> Vec<ActionSpec> {
    vec![
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
            ["ctrl-o"],
            App,
            OpenRepository
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
            ["ctrl-shift-a", "f1"],
            App,
            OpenCommandPalette
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
        spec!("fetch", "Fetch", "Remote", ["ctrl-shift-f"], App, Fetch),
        spec!("pull", "Pull", "Remote", ["ctrl-shift-l"], App, Pull),
        spec!("push", "Push", "Remote", ["ctrl-shift-p"], App, Push),
    ]
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
        (Scope::List, true) => Some("OsierList"),
        (Scope::List, false) => Some("OsierList && !Input"),
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

/// Default bindings for every action.
pub(crate) fn default_bindings(cx: &App) -> Vec<KeyBinding> {
    let specs = actions();
    let pairs: Vec<(&ActionSpec, &str)> = specs
        .iter()
        .flat_map(|s| s.defaults.iter().map(move |k| (s, *k)))
        .collect();
    bindings(pairs, cx)
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
    fn ids_are_unique_and_every_action_has_a_default() {
        let all = actions();
        let mut ids: Vec<&str> = all.iter().map(|a| a.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), all.len());
        assert!(all.iter().all(|a| !a.defaults.is_empty()));
    }
}
