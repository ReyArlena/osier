//! Lists that are slow to fetch (GitHub's repositories, organisations, templates, `.gitignore`
//! and licence lists), kept so screens can show them at once.
//!
//! Each list lives in memory and on disk (`<cache>/lists/<kind>/<key>.json`). A screen calls
//! [`Osier::use_list`]: the first use reads the disk copy in the background and shows it, then
//! the list is refreshed in the background when it is older than allowed (or always, when the
//! screen wants to "check for new ones"). Screens watch [`Cached::revision`] (and
//! [`Cached::refreshing`] for a "Checking for new …" note) and redraw when it moves.
//!
//! Nothing here runs on the UI thread: disk reads, the fetch and the disk write all happen in
//! background tasks. Refreshes on a timer ([`Osier::refresh_lists_on_timer`]) go one at a
//! time, so slow GitHub calls never hold more than one background thread.
//!
//! To cache another list: add a [`CachedLists`] field (to `GhLists` or next to it), call
//! `use_list` where the list is shown, and list it in the timer if it should stay fresh.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui_kit::Context;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::app::Osier;

/// Fetches a list (blocking: a `gh api` call). Shared, so a refresh can run it again later.
pub(crate) type Fetch<T> = Arc<dyn Fn() -> Result<T, String> + Send + Sync>;

/// One cached list.
pub(crate) struct Cached<T> {
    /// What screens show; `None` until the disk copy or the first fetch is in.
    pub data: Option<Arc<T>>,
    /// When `data` was fetched (seconds since the epoch).
    pub fetched_at: Option<u64>,
    /// A refresh is running: screens can say "Checking for new …".
    pub refreshing: bool,
    /// Why the last refresh failed, if it did. `data` (if any) is from before.
    pub error: Option<String>,
    /// Moves whenever `data` changes, for screens that copy it into their own state.
    pub revision: u64,
    disk: Disk,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Disk {
    NotRead,
    Reading,
    Read,
}

impl<T> Default for Cached<T> {
    fn default() -> Self {
        Self {
            data: None,
            fetched_at: None,
            refreshing: false,
            error: None,
            revision: 0,
            disk: Disk::NotRead,
        }
    }
}

/// One kind of list, one entry per key (a GitHub account).
pub(crate) struct CachedLists<T> {
    kind: &'static str,
    /// Status bar label while a refresh runs (background-tasks setting).
    label: &'static str,
    entries: HashMap<String, Cached<T>>,
}

impl<T> CachedLists<T> {
    /// `kind` names the cache folder; `label` is shown in the status bar while refreshing.
    pub(crate) fn new(kind: &'static str, label: &'static str) -> Self {
        Self {
            kind,
            label,
            entries: HashMap::new(),
        }
    }

    pub(crate) fn get(&self, key: &str) -> Option<&Cached<T>> {
        self.entries.get(key)
    }

    /// Drop the in-memory copy so the next use fetches again (after creating a repository).
    /// The disk copy is replaced by that fetch.
    pub(crate) fn mark_stale(&mut self, key: &str) {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.fetched_at = None;
        }
    }

    /// Whether any entry is refreshing (the timer waits for it).
    pub(crate) fn any_refreshing(&self) -> bool {
        self.entries.values().any(|e| e.refreshing)
    }
}

/// When to refresh in [`Osier::use_list`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Freshness {
    /// Always check now (a screen showing the list was just opened).
    CheckNow,
    /// Only when older than this (the timer).
    OlderThan(Duration),
}

#[derive(serde::Deserialize, Serialize)]
struct OnDisk<T> {
    fetched_at: u64,
    data: T,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A key as a file name: letters, digits, `.`, `-` and `_` kept, anything else `_`.
fn file_name(key: &str) -> String {
    let safe: String = key
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{safe}.json")
}

impl Osier {
    fn list_file(&self, kind: &str, key: &str) -> PathBuf {
        self.paths
            .cache_dir()
            .join("lists")
            .join(kind)
            .join(file_name(key))
    }

    /// Show the cached list for `key` and refresh it in the background as `freshness` asks.
    /// `slot` finds the [`CachedLists`] in `Osier`. Every change to the entry notifies the app
    /// view; watch the entry's `revision` and `refreshing`.
    pub(crate) fn use_list<T>(
        &mut self,
        slot: fn(&mut Osier) -> &mut CachedLists<T>,
        key: &str,
        freshness: Freshness,
        fetch: Fetch<T>,
        cx: &mut Context<Self>,
    ) where
        T: Serialize + DeserializeOwned + Send + Sync + 'static,
    {
        let (kind, label) = (slot(self).kind, slot(self).label);
        let file = self.list_file(kind, key);
        let entry = slot(self).entries.entry(key.to_string()).or_default();
        match entry.disk {
            Disk::NotRead => {
                // First use: the disk copy first (quick), then decide about a refresh.
                entry.disk = Disk::Reading;
                let key = key.to_string();
                self.in_background_quiet(
                    move || {
                        let text = std::fs::read_to_string(&file).ok()?;
                        serde_json::from_str::<OnDisk<T>>(&text).ok()
                    },
                    move |this, read, cx| {
                        let entry = slot(this).entries.entry(key.clone()).or_default();
                        entry.disk = Disk::Read;
                        if let Some(read) = read
                            && entry.data.is_none()
                        {
                            entry.data = Some(Arc::new(read.data));
                            entry.fetched_at = Some(read.fetched_at);
                            entry.revision += 1;
                            cx.notify();
                        }
                        this.use_list(slot, &key, freshness, fetch, cx);
                    },
                    cx,
                );
                return;
            }
            // The disk read calls back here when it is done.
            Disk::Reading => return,
            Disk::Read => {}
        }
        if entry.refreshing {
            return;
        }
        let stale = match freshness {
            Freshness::CheckNow => true,
            Freshness::OlderThan(age) => entry
                .fetched_at
                .is_none_or(|at| now().saturating_sub(at) >= age.as_secs()),
        };
        if !stale {
            return;
        }
        entry.refreshing = true;
        cx.notify();
        let key = key.to_string();
        self.in_background_cx(
            label,
            move || {
                let fetched = fetch()?;
                let at = now();
                // Saved here, still off the UI thread. A failed write only costs the next
                // start its quick first look.
                let on_disk = OnDisk {
                    fetched_at: at,
                    data: &fetched,
                };
                if let Some(dir) = file.parent()
                    && std::fs::create_dir_all(dir).is_ok()
                    && let Ok(text) = serde_json::to_string(&on_disk)
                {
                    let tmp = file.with_extension("json.tmp");
                    if std::fs::write(&tmp, text).is_ok() {
                        let _ = std::fs::rename(&tmp, &file);
                    }
                }
                Ok((fetched, at))
            },
            move |this, result: Result<(T, u64), String>, cx| {
                let entry = slot(this).entries.entry(key).or_default();
                entry.refreshing = false;
                match result {
                    Ok((data, at)) => {
                        entry.data = Some(Arc::new(data));
                        entry.fetched_at = Some(at);
                        entry.error = None;
                        entry.revision += 1;
                    }
                    Err(e) => {
                        tracing::info!(error = %e, "list refresh failed");
                        entry.error = Some(e);
                    }
                }
                cx.notify();
            },
            cx,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::file_name;

    #[test]
    fn keys_become_safe_file_names() {
        assert_eq!(file_name("alice@github.com"), "alice_github.com.json");
        assert_eq!(file_name("../x/y"), ".._x_y.json");
    }
}
