//! Small display helpers.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// "5 min ago", "12h ago", "yesterday", "2024-01-05".
pub fn relative_time(unix: i64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    relative_to(now, unix)
}

fn relative_to(now: i64, unix: i64) -> String {
    let delta = now - unix;
    if delta < 0 {
        return "in the future".into();
    }
    let minutes = delta / 60;
    let hours = minutes / 60;
    let days = hours / 24;
    match () {
        _ if delta < 45 => "just now".into(),
        _ if minutes < 60 => format!("{} min ago", minutes.max(1)),
        _ if hours < 24 => format!("{hours}h ago"),
        _ if days == 1 => "yesterday".into(),
        _ if days < 14 => format!("{days} days ago"),
        _ if days < 60 => format!("{} weeks ago", days / 7),
        _ if days < 365 => format!("{} months ago", days / 30),
        _ => iso_date(unix),
    }
}

/// The full local date and time, for tooltips: "Monday 1 January 2024, 12:00".
pub fn full_time(unix: i64) -> String {
    use chrono::TimeZone as _;
    match chrono::Local.timestamp_opt(unix, 0).single() {
        Some(t) => t.format("%A %-d %B %Y, %H:%M").to_string(),
        None => iso_date(unix),
    }
}

/// `YYYY-MM-DD` without a date crate (proleptic Gregorian, UTC).
pub fn iso_date(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    // Algorithm from Howard Hinnant's "days from civil".
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// The path with the home directory shown as `~`: `/home/rey/Work/app` is `~/Work/app`.
/// Whole path components only (`/home/reynolds` is not under `/home/rey`). The home is
/// looked up once, every way it can be spelled (`osier_config::paths::home_dirs`); a path
/// that only matches once its own symlinks are resolved is resolved once and remembered.
pub fn tilde(path: &Path) -> String {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{Mutex, OnceLock};
    static HOMES: OnceLock<Vec<PathBuf>> = OnceLock::new();
    static RESOLVED: OnceLock<Mutex<HashMap<PathBuf, String>>> = OnceLock::new();
    let homes = HOMES.get_or_init(osier_config::paths::home_dirs);
    if let Some(short) = tilde_with(path, homes) {
        return short;
    }
    let cache = RESOLVED.get_or_init(Default::default);
    if let Ok(mut cache) = cache.lock() {
        if let Some(hit) = cache.get(path) {
            return hit.clone();
        }
        let shown = std::fs::canonicalize(path)
            .ok()
            .and_then(|real| tilde_with(&real, homes))
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        if cache.len() < 4096 {
            cache.insert(path.to_path_buf(), shown.clone());
        }
        return shown;
    }
    path.to_string_lossy().into_owned()
}

fn tilde_with(path: &Path, homes: &[std::path::PathBuf]) -> Option<String> {
    homes.iter().find_map(|home| {
        let rest = path.strip_prefix(home).ok()?;
        Some(if rest.as_os_str().is_empty() {
            "~".to_string()
        } else {
            format!("~/{}", rest.to_string_lossy())
        })
    })
}

/// "1 change", "3 changes": a count with its noun, singular for one.
pub fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

/// "a", "a and b", "a, b and c".
pub fn list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// `↑a ↓b` for commits ahead of and behind the upstream, only the non-zero parts; `None`
/// when in sync.
pub fn ahead_behind(ahead: u32, behind: u32) -> Option<String> {
    match (ahead, behind) {
        (0, 0) => None,
        (a, 0) => Some(format!("↑{a}")),
        (0, b) => Some(format!("↓{b}")),
        (a, b) => Some(format!("↑{a} ↓{b}")),
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn lists_read_like_a_sentence() {
        let l = |v: &[&str]| super::list(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(l(&[]), "");
        assert_eq!(l(&["a"]), "a");
        assert_eq!(l(&["a", "b"]), "a and b");
        assert_eq!(l(&["a", "b", "c"]), "a, b and c");
    }

    use super::*;

    #[test]
    fn iso_dates() {
        assert_eq!(iso_date(0), "1970-01-01");
        assert_eq!(iso_date(1_704_067_200), "2024-01-01");
        assert_eq!(iso_date(951_782_400), "2000-02-29");
    }

    #[test]
    fn tilde_matches_whole_components_and_any_spelling_of_home() {
        use std::path::PathBuf;
        let homes = [PathBuf::from("/home/rey"), PathBuf::from("/var/home/rey")];
        let t = |p: &str| tilde_with(Path::new(p), &homes);
        assert_eq!(
            t("/home/rey/Work/api-server").as_deref(),
            Some("~/Work/api-server")
        );
        assert_eq!(t("/var/home/rey/Work/app").as_deref(), Some("~/Work/app"));
        assert_eq!(t("/home/rey").as_deref(), Some("~"));
        assert_eq!(t("/home/rey/").as_deref(), Some("~"));
        assert_eq!(t("/home/reynolds/x"), None, "not a prefix of a longer name");
        assert_eq!(t("/opt/app"), None);
    }

    #[test]
    fn counts_are_singular_for_one() {
        assert_eq!(count(0, "change"), "0 changes");
        assert_eq!(count(1, "change"), "1 change");
        assert_eq!(count(2, "file"), "2 files");
    }

    #[test]
    fn relative_times_have_no_space_before_h() {
        let now = 1_800_000_000;
        assert_eq!(relative_to(now, now - 10), "just now");
        assert_eq!(relative_to(now, now - 5 * 60), "5 min ago");
        assert_eq!(relative_to(now, now - 12 * 3600), "12h ago");
        assert_eq!(relative_to(now, now - 36 * 3600), "yesterday");
    }

    #[test]
    fn full_time_names_the_day_and_month() {
        // Noon UTC is the same calendar day in every time zone from -11 to +11.
        let t = full_time(1_704_110_400); // 2024-01-01 12:00 UTC
        assert!(t.starts_with("Monday 1 January 2024, "), "{t}");
    }
}
