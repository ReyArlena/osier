//! Stable element ids for headless UI tests.
//!
//! `test_id` sets gpui's debug selector. gpui records selectors only in test builds (debug
//! assertions plus its `test-support` feature, which only the dev-dependency enables), so this
//! costs nothing in the real app. Tests look elements up with
//! `VisualTestContext::debug_bounds(id)` and click or type at their position, which exercises
//! the real event routing (focus, key contexts, overlays) that calling methods directly skips.
//!
//! Naming convention, kebab-case, `area-thing`, with a `:`-separated key for repeated rows:
//! - buttons with a literal element id reuse it (`Button::new("settings")` is `settings`)
//! - tabs `tab-<label>`; sidebar rows `sidebar-open:<name>`, `sidebar-pinned:<name>`,
//!   `sidebar-recent:<name>`
//! - Changes rows `change-row:<area>:<path>` and their checkbox `change-stage:<area>:<path>`
//!   (`<area>` is `conflicts`, `staged`, `unstaged` or `untracked`)
//! - Log rows `log-row:<index>`, Branches `branch-row:<name>`, `tag-row:<name>`,
//!   branch menu buttons `branch-menu:<name>`, Stashes `stash-row:<index>`, Patches `patch-row:<id>`, command log `command-row:<record id>`
//! - inputs by their container: `commit-message`, `files-filter`, `sidebar-filter`
//! - the filter dropdown `filter-suggestions`, its rows `filter-suggestion:<term>` and the
//!   settings link `filter-open-settings`
//! - the settings dialog content `settings-dialog`
//! - command palette: query field `palette-query`, submenu badge `palette-badge`, rows
//!   `palette-command:<label>` and `palette-project:<name>`

use gpui_kit::InteractiveElement;

pub(crate) trait TestId: InteractiveElement + Sized {
    fn test_id(self, id: impl Into<String>) -> Self {
        let id = id.into();
        self.debug_selector(move || id)
    }
}

impl<T: InteractiveElement> TestId for T {}
