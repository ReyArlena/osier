//! Sidebar filter queries: name text plus GitHub-style `is:` qualifiers.
//!
//! `is:changed` (uncommitted changes), `is:ahead` (unpushed commits) and `is:behind` (the
//! upstream has commits we do not) combine with each other and with name words; every term
//! must match. A half-typed qualifier (`is:`, `is:ch`) is ignored so the list does not empty
//! while the user picks from the suggestions. Unknown qualifiers are plain name text.

use std::path::Path;

/// Known qualifiers with the description shown in the suggestion dropdown.
pub(crate) const QUALIFIERS: [(&str, &str); 3] = [
    ("is:changed", "Uncommitted changes"),
    ("is:ahead", "Unpushed commits"),
    ("is:behind", "New commits on the remote"),
];

/// What the sidebar knows about a project's state, from a snapshot or a recent-status badge.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct RowStatus {
    /// `None` when only refs are watched (no working tree check).
    pub dirty: Option<bool>,
    pub ahead: u32,
    pub behind: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Query {
    pub words: Vec<String>,
    pub changed: bool,
    pub ahead: bool,
    pub behind: bool,
}

impl Query {
    pub(crate) fn parse(filter: &str) -> Self {
        let mut q = Query::default();
        for token in filter.split_whitespace() {
            let token = token.to_lowercase();
            match token.as_str() {
                "is:changed" => q.changed = true,
                "is:ahead" => q.ahead = true,
                "is:behind" => q.behind = true,
                t if is_partial_qualifier(t) => {}
                _ => q.words.push(token),
            }
        }
        q
    }

    pub(crate) fn has_qualifiers(&self) -> bool {
        self.changed || self.ahead || self.behind
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.words.is_empty() && !self.has_qualifiers()
    }

    /// `status` is `None` when the project's state is unknown; it then fails every qualifier.
    pub(crate) fn matches(&self, path: &Path, status: Option<RowStatus>) -> bool {
        let haystack = path.to_string_lossy().to_lowercase();
        if !self.words.iter().all(|w| haystack.contains(w.as_str())) {
            return false;
        }
        if !self.has_qualifiers() {
            return true;
        }
        let Some(s) = status else {
            return false;
        };
        (!self.changed || s.dirty == Some(true))
            && (!self.ahead || s.ahead > 0)
            && (!self.behind || s.behind > 0)
    }
}

/// `is:` or the start of a known qualifier, while the user is still typing it.
fn is_partial_qualifier(token: &str) -> bool {
    token.starts_with("is:") && QUALIFIERS.iter().any(|(q, _)| q.starts_with(token))
}

/// Suggestions for the word under the cursor (the last one): all qualifiers not already in
/// the filter whose text starts with that word. Shown for an empty word too.
pub(crate) fn suggestions(filter: &str) -> Vec<(&'static str, &'static str)> {
    let current = current_word(filter).to_lowercase();
    let triggers = current.is_empty() || current == "is" || current.starts_with("is:");
    if !triggers {
        return Vec::new();
    }
    let used: Vec<String> = filter.split_whitespace().map(str::to_lowercase).collect();
    QUALIFIERS
        .iter()
        .copied()
        .filter(|(q, _)| q.starts_with(current.as_str()) && !used.iter().any(|u| u == q))
        .collect()
}

/// The word being typed: whatever follows the last space (empty after a space).
fn current_word(filter: &str) -> &str {
    filter.rsplit(char::is_whitespace).next().unwrap_or("")
}

/// The filter with the word being typed replaced by `term` and a trailing space.
pub(crate) fn insert(filter: &str, term: &str) -> String {
    let current = current_word(filter);
    let head = &filter[..filter.len() - current.len()];
    format!("{head}{term} ")
}

#[cfg(test)]
mod tests {
    use super::{Query, RowStatus, insert, suggestions};
    use std::path::Path;

    fn st(dirty: Option<bool>, ahead: u32, behind: u32) -> Option<RowStatus> {
        Some(RowStatus {
            dirty,
            ahead,
            behind,
        })
    }

    #[test]
    fn words_and_qualifiers_all_have_to_match() {
        let p = Path::new("/code/osier");
        assert!(Query::parse("").matches(p, None));
        assert!(Query::parse("OSI").matches(p, None));
        assert!(Query::parse("code osier").matches(p, None));
        assert!(!Query::parse("osier other").matches(p, None));

        let q = Query::parse("is:ahead osier");
        assert!(q.matches(p, st(Some(false), 2, 0)));
        assert!(!q.matches(p, st(Some(true), 0, 3)));
        assert!(
            !q.matches(p, None),
            "unknown state never matches a qualifier"
        );
        assert!(!q.matches(Path::new("/code/other"), st(None, 2, 0)));

        let q = Query::parse("is:changed is:behind");
        assert!(q.matches(p, st(Some(true), 0, 1)));
        assert!(!q.matches(p, st(Some(true), 0, 0)));
        assert!(
            !q.matches(p, st(None, 0, 1)),
            "refs-only watching cannot answer is:changed"
        );
    }

    #[test]
    fn half_typed_qualifiers_are_ignored_and_unknown_ones_are_text() {
        assert!(Query::parse("is:").is_empty());
        assert!(Query::parse("is:ch").is_empty());
        let q = Query::parse("is:open");
        assert_eq!(q.words, vec!["is:open".to_string()]);
        assert!(!q.has_qualifiers());
        assert_eq!(Query::parse("is").words, vec!["is".to_string()]);
    }

    #[test]
    fn suggestions_follow_the_word_being_typed() {
        let all = |f: &str| {
            suggestions(f)
                .into_iter()
                .map(|(q, _)| q)
                .collect::<Vec<_>>()
        };
        assert_eq!(all(""), ["is:changed", "is:ahead", "is:behind"]);
        assert_eq!(all("is"), ["is:changed", "is:ahead", "is:behind"]);
        assert_eq!(all("is:a"), ["is:ahead"]);
        assert_eq!(all("osier "), ["is:changed", "is:ahead", "is:behind"]);
        assert_eq!(
            all("is:ahead "),
            ["is:changed", "is:behind"],
            "used terms drop out"
        );
        assert!(
            all("osier").is_empty(),
            "plain words do not trigger suggestions"
        );
        assert!(all("i").is_empty());
        assert!(all("ix").is_empty());
    }

    #[test]
    fn inserting_replaces_the_word_being_typed() {
        assert_eq!(insert("", "is:ahead"), "is:ahead ");
        assert_eq!(insert("osier is:a", "is:ahead"), "osier is:ahead ");
        assert_eq!(insert("osier ", "is:behind"), "osier is:behind ");
    }
}
