//! Stable element ids for headless UI tests.
//!
//! `test_id` sets gpui's debug selector. gpui records selectors only in test builds (debug
//! assertions plus its `test-support` feature, which only the dev-dependency enables), and
//! `test_id` itself is a no-op outside `cfg(test)`, so the real app never builds the id
//! string: pass a literal, or a closure (`|| format!(..)`) for ids built per row. Tests look
//! elements up with `VisualTestContext::debug_bounds(id)` and click or type at their position,
//! which exercises the real event routing (focus, key contexts, overlays) that calling methods
//! directly skips.
//!
//! Naming convention, kebab-case, `area-thing`, with a `:`-separated key for repeated rows:
//! - buttons with a literal element id reuse it (`Button::new("settings")` is `settings`)
//! - tabs `tab-<label>`; sidebar rows `sidebar-open:<name>`, `sidebar-pinned:<name>`,
//!   `sidebar-recent:<name>`
//! - Changes rows `change-row:<area>:<path>` and their checkbox `change-stage:<area>:<path>`
//!   (`<area>` is `conflicts`, `staged`, `unstaged` or `untracked`)
//! - Log rows `log-row:<index>`, reflog rows `reflog-row:<index>`, Branches `branch-row:<name>`, `tag-row:<name>`,
//!   branch menu buttons `branch-menu:<name>`, Stashes `stash-row:<index>`, Patches `patch-row:<id>`, command log `command-row:<record id>`
//! - inputs by their container: `commit-message`, `files-filter`, `sidebar-filter`
//! - the filter dropdown `filter-suggestions`, its rows `filter-suggestion:<term>` and the
//!   settings link `filter-open-settings`
//! - the settings dialog content `settings-dialog`
//! - command palette: query field `palette-query`, submenu badge `palette-badge`, rows
//!   `palette-command:<label>` and `palette-project:<name>`

use gpui_kit::InteractiveElement;

/// What `test_id` takes: a literal, an owned string, or a closure that builds the id only
/// when ids are recorded (the `Fn` traits are fundamental, so the impls do not overlap).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) trait TestIdSource {
    fn build(self) -> String;
}

impl TestIdSource for &'static str {
    fn build(self) -> String {
        self.to_string()
    }
}

impl TestIdSource for String {
    fn build(self) -> String {
        self
    }
}

impl<F: FnOnce() -> String> TestIdSource for F {
    fn build(self) -> String {
        self()
    }
}

pub(crate) trait TestId: InteractiveElement + Sized {
    #[cfg(test)]
    fn test_id(self, id: impl TestIdSource) -> Self {
        self.debug_selector(|| id.build())
    }

    #[cfg(not(test))]
    #[inline]
    fn test_id(self, _id: impl TestIdSource) -> Self {
        self
    }
}

impl<T: InteractiveElement> TestId for T {}
