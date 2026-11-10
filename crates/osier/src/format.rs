//! Small display helpers.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// "5 min ago", "12h ago", "yesterday", "2024-01-05"; times ahead of the clock read the
/// other way: "in 5 min", "in 3h", "tomorrow".
pub fn relative_time(unix: i64) -> String {
    relative_to(now_secs(), unix)
}

/// Whether [`relative_time`] shows `unix` as ahead of the clock ("in 3h") rather than past
/// or "just now", e.g. a commit made on a machine whose clock runs fast.
pub fn is_future(unix: i64) -> bool {
    unix - now_secs() >= JUST_NOW_SECS
}

/// Within this many seconds either side of now reads as "just now".
const JUST_NOW_SECS: i64 = 45;

fn now_secs() -> i64 {
    unix_secs(SystemTime::now())
}

fn relative_to(now: i64, unix: i64) -> String {
    let delta = now - unix;
    let future = delta < 0;
    let delta = delta.abs();
    let minutes = delta / 60;
    let hours = minutes / 60;
    let days = hours / 24;
    let span = match () {
        _ if delta < JUST_NOW_SECS => return "just now".into(),
        _ if minutes < 60 => format!("{} min", minutes.max(1)),
        _ if hours < 24 => format!("{hours}h"),
        _ if days == 1 => return if future { "tomorrow" } else { "yesterday" }.into(),
        _ if days < 14 => format!("{days} days"),
        _ if days < 60 => format!("{} weeks", days / 7),
        _ if days < 365 => format!("{} months", days / 30),
        _ => return iso_date(unix),
    };
    if future {
        format!("in {span}")
    } else {
        format!("{span} ago")
    }
}

/// Seconds since the epoch, 0 for times before it.
pub fn unix_secs(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// The local wall-clock time of day with seconds: "14:03:27".
pub fn clock_time(unix: i64) -> String {
    use chrono::TimeZone as _;
    match chrono::Local.timestamp_opt(unix, 0).single() {
        Some(t) => t.format("%H:%M:%S").to_string(),
        None => "--:--:--".into(),
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

/// [`full_time`] with seconds: "Monday 1 January 2024, 12:00:05". For times that may sit
/// seconds apart, such as a commit's author and committer dates.
pub fn full_time_seconds(unix: i64) -> String {
    use chrono::TimeZone as _;
    match chrono::Local.timestamp_opt(unix, 0).single() {
        Some(t) => t.format("%A %-d %B %Y, %H:%M:%S").to_string(),
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

/// What tells apart paths whose folder names are the same: for each of them, the fewest
/// parent folders that differ from all the others' (`~/Work/app` and `~/Projects/app` give
/// `Work` and `Projects`). Paths with a name of their own are left out.
pub fn name_hints<'a>(
    paths: impl IntoIterator<Item = &'a Path>,
) -> std::collections::HashMap<std::path::PathBuf, String> {
    use std::collections::HashMap;
    use std::path::Component;
    let mut groups: HashMap<&std::ffi::OsStr, Vec<&Path>> = HashMap::new();
    for path in paths {
        if let Some(name) = path.file_name() {
            let group = groups.entry(name).or_default();
            if !group.contains(&path) {
                group.push(path);
            }
        }
    }
    let mut out = HashMap::new();
    for group in groups.into_values().filter(|g| g.len() > 1) {
        // Each path's parent folders, nearest first.
        let parents: Vec<Vec<String>> = group
            .iter()
            .map(|path| {
                let mut names: Vec<String> = path
                    .parent()
                    .into_iter()
                    .flat_map(Path::components)
                    .filter_map(|c| match c {
                        Component::Normal(n) => Some(n.to_string_lossy().into_owned()),
                        _ => None,
                    })
                    .collect();
                names.reverse();
                names
            })
            .collect();
        let hint = |names: &[String], depth: usize| {
            let mut shown: Vec<&str> = names.iter().take(depth).map(String::as_str).collect();
            shown.reverse();
            if shown.is_empty() {
                "/".to_string()
            } else {
                shown.join("/")
            }
        };
        let deepest = parents.iter().map(Vec::len).max().unwrap_or(0).max(1);
        for (ix, (path, names)) in group.iter().zip(&parents).enumerate() {
            // The fewest folders at which this path differs from every other one.
            let depth = (1..=deepest)
                .find(|depth| {
                    let own = hint(names, *depth);
                    parents
                        .iter()
                        .enumerate()
                        .all(|(other, names)| other == ix || hint(names, *depth) != own)
                })
                .unwrap_or(deepest);
            out.insert(path.to_path_buf(), hint(names, depth));
        }
    }
    out
}

/// A project's name as lists show it: with its [`name_hints`] folder when it has one,
/// `app (Work)`.
pub fn name_with_hint(name: &str, hint: Option<&String>) -> String {
    match hint {
        Some(hint) => format!("{name} ({hint})"),
        None => name.to_string(),
    }
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
    fn name_hints_cover_only_shared_names_with_the_fewest_folders() {
        let paths = [
            "/home/x/Work/app",
            "/home/x/Projects/app",
            "/home/x/Projects/api-server",
            "/home/x/Work/client/site",
            "/home/x/Projects/client/site",
            "/srv/client/site",
            "/home/x/Work/app",
        ];
        let hints = name_hints(paths.iter().map(Path::new));
        let hint = |p: &str| hints.get(Path::new(p)).map(String::as_str);
        assert_eq!(hint("/home/x/Work/app"), Some("Work"));
        assert_eq!(hint("/home/x/Projects/app"), Some("Projects"));
        assert_eq!(
            hint("/home/x/Projects/api-server"),
            None,
            "a name of its own"
        );
        // One folder up is `client` for all three, so every one of them shows two.
        assert_eq!(hint("/home/x/Work/client/site"), Some("Work/client"));
        assert_eq!(
            hint("/home/x/Projects/client/site"),
            Some("Projects/client")
        );
        assert_eq!(hint("/srv/client/site"), Some("srv/client"));
        assert_eq!(
            name_with_hint("app", hints.get(Path::new("/home/x/Work/app"))),
            "app (Work)"
        );
        assert_eq!(name_with_hint("api-server", None), "api-server");
    }

    #[test]
    fn name_hints_show_each_path_only_what_it_needs() {
        let hints = name_hints(
            [
                "/h/Projects/notes",
                "/h/Work/client/notes",
                "/h/Old/client/notes",
            ]
            .map(Path::new),
        );
        let hint = |p: &str| hints.get(Path::new(p)).map(String::as_str);
        assert_eq!(hint("/h/Projects/notes"), Some("Projects"));
        assert_eq!(hint("/h/Work/client/notes"), Some("Work/client"));
        assert_eq!(hint("/h/Old/client/notes"), Some("Old/client"));
    }

    #[test]
    fn name_hints_handle_a_shorter_path_and_the_root() {
        let hints = name_hints(["/app", "/srv/app"].map(Path::new));
        assert_eq!(hints.get(Path::new("/app")).map(String::as_str), Some("/"));
        assert_eq!(
            hints.get(Path::new("/srv/app")).map(String::as_str),
            Some("srv")
        );
        assert!(name_hints(["/srv/app"].map(Path::new)).is_empty());
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
    fn future_times_read_forwards() {
        let now = 1_800_000_000;
        assert_eq!(relative_to(now, now + 10), "just now");
        assert_eq!(relative_to(now, now + 3 * 60), "in 3 min");
        assert_eq!(relative_to(now, now + 3 * 3600), "in 3h");
        assert_eq!(relative_to(now, now + 36 * 3600), "tomorrow");
        assert_eq!(relative_to(now, now + 5 * 86_400), "in 5 days");
        assert_eq!(relative_to(now, now + 21 * 86_400), "in 3 weeks");
        assert_eq!(relative_to(now, now + 90 * 86_400), "in 3 months");
    }

    #[test]
    fn full_time_names_the_day_and_month() {
        // Noon UTC is the same calendar day in every time zone from -11 to +11.
        let t = full_time(1_704_110_400); // 2024-01-01 12:00 UTC
        assert!(t.starts_with("Monday 1 January 2024, "), "{t}");
    }

    #[test]
    fn full_time_seconds_adds_the_seconds() {
        let t = super::full_time_seconds(1_704_110_405); // 2024-01-01 12:00:05 UTC
        assert!(t.starts_with("Monday 1 January 2024, "), "{t}");
        assert!(t.ends_with(":05"), "{t}");
        assert!(t.starts_with(&super::full_time(1_704_110_405)), "{t}");
    }
}
