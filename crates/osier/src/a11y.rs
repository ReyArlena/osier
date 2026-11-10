//! Names for assistive technology: screen readers, voice control, and the AT-SPI tools that
//! find Osier's widgets by name in live tests.
//!
//! gpui reports an element only when it has an id *and* a role, and a role alone gives it no
//! name. gpui-kit's components cover themselves when they show a label (a button with text, a
//! checkbox with text, a tab, a sidebar row). What Osier builds itself is not covered, and
//! each needs one of these:
//!
//! - a list row built from a `div` or a `ListItem`: [`A11yRow::a11y_row`] when it can be
//!   selected, [`A11yRow::a11y_item`] when it cannot
//! - a button that shows only an icon: `.tooltip(..)` (read as its description) or
//!   `.accessibility_label("..")`, and [`A11yName::a11y_name`] when the name is built per row
//! - a checkbox without a visible label: [`A11yName::a11y_name`]
//! - buttons that repeat on every row with the same text ("Checkout"): [`A11yName::a11y_name`]
//!   with what they act on ("Checkout main")
//!
//! Names built per row cost a `String` each, on every render. They are only built while
//! accessibility is active ([`active`]), which gpui tells us per frame; turning it on forces
//! a redraw, so the names are there for the first tree that is sent.

use std::sync::atomic::{AtomicBool, Ordering};

use gpui_kit::component::button::Button;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::{Role, StatefulInteractiveElement, Window};

static ACTIVE: AtomicBool = AtomicBool::new(false);

/// Record whether assistive technology is connected for this frame. Called at the top of the
/// main view's render, before any row is built.
pub(crate) fn sync(window: &Window) {
    ACTIVE.store(window.is_a11y_active(), Ordering::Relaxed);
}

/// Whether names built per row are wanted (see the module docs).
pub(crate) fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

/// Rows of Osier's own lists. The element needs an `.id(..)`, as every clickable row has.
pub(crate) trait A11yRow: StatefulInteractiveElement + Sized {
    /// A row that can be selected (a file, a commit, a branch): an option of its list, with
    /// `label` as what is read out and whether it is the selected one.
    fn a11y_row(self, label: impl FnOnce() -> String, selected: bool) -> Self {
        if !active() {
            return self;
        }
        self.role(Role::ListBoxOption)
            .aria_label(label())
            .aria_selected(selected)
    }

    /// A row that is only listed (a worktree, a notification).
    fn a11y_item(self, label: impl FnOnce() -> String) -> Self {
        if !active() {
            return self;
        }
        self.role(Role::ListItem).aria_label(label())
    }
}

impl<T: StatefulInteractiveElement> A11yRow for T {}

/// A name built per row for a gpui-kit control that has none of its own, or one that every
/// row repeats.
pub(crate) trait A11yName: Sized {
    fn a11y_name(self, name: impl FnOnce() -> String) -> Self;
}

impl A11yName for Button {
    fn a11y_name(self, name: impl FnOnce() -> String) -> Self {
        if active() {
            self.accessibility_label(name())
        } else {
            self
        }
    }
}

impl A11yName for Checkbox {
    fn a11y_name(self, name: impl FnOnce() -> String) -> Self {
        if active() {
            self.accessibility_label(name())
        } else {
            self
        }
    }
}

/// Source checks: the accessibility tree cannot be built in a headless test (gpui's test
/// platform never activates it), so these read the code instead. They catch the element that
/// was added without a name; the live audit (`a11y.py <pid> audit`) is the proof.
#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    /// Every source file of the app, without its `#[cfg(test)]` tail and without the UI tests.
    fn sources() -> Vec<(PathBuf, String)> {
        fn walk(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if path.file_name().is_some_and(|n| n != "tests") {
                        walk(&path, out);
                    }
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let text = std::fs::read_to_string(&path).unwrap();
                    let code = text.split("#[cfg(test)]").next().unwrap_or("").to_string();
                    out.push((path, code));
                }
            }
        }
        let mut out = Vec::new();
        walk(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut out);
        out.sort();
        out
    }

    /// Index just past the `)` that closes the `(` at `open`, skipping strings, character
    /// literals and line comments.
    fn close_paren(src: &[u8], open: usize) -> usize {
        let (mut i, mut depth) = (open, 0usize);
        while i < src.len() {
            match src[i] {
                b'"' => {
                    i += 1;
                    while i < src.len() && src[i] != b'"' {
                        i += if src[i] == b'\\' { 2 } else { 1 };
                    }
                }
                b'\'' if src.get(i + 2) == Some(&b'\'') => i += 2,
                b'/' if src.get(i + 1) == Some(&b'/') => {
                    while i < src.len() && src[i] != b'\n' {
                        i += 1;
                    }
                }
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return i + 1;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        src.len()
    }

    /// The methods called directly on the value built at `start` (`Button::new(..)` and what
    /// is chained onto it), not the ones inside their arguments.
    fn chain_methods(code: &str, start: usize) -> Vec<&str> {
        let src = code.as_bytes();
        let Some(open) = code[start..].find('(') else {
            return Vec::new();
        };
        let mut i = close_paren(src, start + open);
        let mut methods = Vec::new();
        loop {
            // Whitespace and comments between the links of the chain.
            let mut j = i;
            loop {
                while j < src.len() && src[j].is_ascii_whitespace() {
                    j += 1;
                }
                if src.get(j) == Some(&b'/') && src.get(j + 1) == Some(&b'/') {
                    while j < src.len() && src[j] != b'\n' {
                        j += 1;
                    }
                } else {
                    break;
                }
            }
            if src.get(j) != Some(&b'.') {
                return methods;
            }
            let name_start = j + 1;
            let mut k = name_start;
            while k < src.len() && (src[k].is_ascii_alphanumeric() || src[k] == b'_') {
                k += 1;
            }
            if k == name_start {
                return methods;
            }
            methods.push(&code[name_start..k]);
            // `.method::<T>(..)`, `.method(..)` or a field / `.await`.
            let mut call = k;
            if code[call..].starts_with("::") {
                call += code[call..].find('(').unwrap_or(0);
            }
            i = if src.get(call) == Some(&b'(') {
                close_paren(src, call)
            } else {
                k
            };
        }
    }

    fn line_of(code: &str, at: usize) -> usize {
        code[..at].matches('\n').count() + 1
    }

    /// An exception says why on one of the three lines above: `// a11y: named by the tree`.
    fn excused(code: &str, at: usize) -> bool {
        code[..at]
            .rsplit('\n')
            .take(4)
            .any(|line| line.trim_start().starts_with("// a11y:"))
    }

    fn report(problems: Vec<String>, what: &str) {
        assert!(
            problems.is_empty(),
            "{what} (see the docs at the top of crates/osier/src/a11y.rs; a real exception takes \
             a `// a11y: <why>` comment on the line above):\n  {}",
            problems.join("\n  ")
        );
    }

    #[test]
    fn icon_only_buttons_have_a_name() {
        let named = ["label", "tooltip", "accessibility_label", "a11y_name"];
        let mut problems = Vec::new();
        for (path, code) in sources() {
            for (at, _) in code.match_indices("Button::new(") {
                let methods = chain_methods(&code, at);
                if methods.contains(&"icon")
                    && !methods.iter().any(|m| named.contains(m))
                    && !excused(&code, at)
                {
                    problems.push(format!("{}:{}", path.display(), line_of(&code, at)));
                }
            }
        }
        report(
            problems,
            "buttons that show only an icon need `.tooltip(..)`, `.accessibility_label(..)` or \
             `.a11y_name(..)`",
        );
    }

    #[test]
    fn checkboxes_have_a_name() {
        let named = ["label", "accessibility_label", "a11y_name"];
        let mut problems = Vec::new();
        for (path, code) in sources() {
            for (at, _) in code.match_indices("Checkbox::new(") {
                let methods = chain_methods(&code, at);
                if !methods.iter().any(|m| named.contains(m)) && !excused(&code, at) {
                    problems.push(format!("{}:{}", path.display(), line_of(&code, at)));
                }
            }
        }
        report(
            problems,
            "checkboxes without a `.label(..)` need `.accessibility_label(..)` or `.a11y_name(..)`",
        );
    }

    #[test]
    fn list_rows_are_exposed() {
        let mut problems = Vec::new();
        for (path, code) in sources() {
            for (at, _) in code.match_indices("-row:") {
                // Only test ids: `.test_id(|| format!("log-row:{ix}"))`.
                let before = &code[at.saturating_sub(60)..at];
                if !before.contains(".test_id(") {
                    continue;
                }
                let after = &code[at..(at + 400).min(code.len())];
                if !after.contains(".a11y_row(")
                    && !after.contains(".a11y_item(")
                    && !excused(&code, at)
                {
                    problems.push(format!("{}:{}", path.display(), line_of(&code, at)));
                }
            }
        }
        report(
            problems,
            "rows need `.a11y_row(..)` or `.a11y_item(..)` right after their `.test_id(..)`",
        );
    }

    #[test]
    fn the_chain_reader_sees_only_top_level_methods() {
        let code = r#"let b = Button::new(("x", 1)) // id
            .ghost()
            .icon(Lucide::X)
            .when(open, |b| b.label("inner ) \" paren"))
            .on_click(cx.listener(|this, _, _, cx| this.tooltip(cx)));
        other.label("no")"#;
        assert_eq!(
            chain_methods(code, code.find("Button::new(").unwrap()),
            ["ghost", "icon", "when", "on_click"]
        );
    }
}
