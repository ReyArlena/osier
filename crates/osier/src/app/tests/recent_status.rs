//! Status badges of projects that are not open: cached, check and watch modes.

use super::*;

#[gpui::test]
fn recent_status_never_does_nothing(cx: &mut TestAppContext) {
    let repo = repo_with_upstream(2, true);
    let other = repo_with_upstream(1, false);
    let h = Harness::open(cx, &repo);
    h.act(cx, |osier, _, _| {
        osier.state.recent.push(other.path().to_path_buf())
    });
    drain_recent(&h, cx);
    h.read(cx, |osier, _| {
        assert!(osier.recent_badge(other.path()).is_none());
        assert!(osier.recent_badge(repo.path()).is_none());
        let cache = osier_config::status_cache::StatusCache::file(&osier.paths);
        assert!(!cache.exists(), "nothing written in never mode");
    });
}

#[gpui::test]
fn recent_status_cached_keeps_values_from_the_last_open(cx: &mut TestAppContext) {
    use osier_config::settings::RecentStatusMode;
    let repo = repo_with_upstream(2, true);
    let h = Harness::open(cx, &repo);
    set_recent_mode(&h, cx, RecentStatusMode::Cached);
    h.act(cx, |osier, window, cx| osier.close_project(0, window, cx));
    drain_recent(&h, cx);
    h.read(cx, |osier, _| {
        let b = osier.recent_badge(repo.path()).expect("cached badge");
        assert_eq!((b.dirty, b.ahead, b.behind), (Some(true), 2, 0));
        let cache = osier_config::status_cache::StatusCache::load(&osier.paths);
        assert_eq!(
            cache.get(repo.path()).map(|b| b.ahead),
            Some(2),
            "written to disk"
        );
        assert_eq!(
            osier.recent_status_queue_len(),
            0,
            "cached mode runs no git"
        );
    });
    // Switching to never hides everything again.
    set_recent_mode(&h, cx, RecentStatusMode::Never);
    h.read(cx, |osier, _| {
        assert!(osier.recent_badge(repo.path()).is_none())
    });
}

#[gpui::test]
fn recent_status_check_mode_reads_projects_that_were_never_opened(cx: &mut TestAppContext) {
    use osier_config::settings::RecentStatusMode;
    let repo = repo_with_upstream(0, false);
    let other = repo_with_upstream(3, true);
    let h = Harness::open(cx, &repo);
    h.act(cx, |osier, _, _| {
        osier.state.recent.push(other.path().to_path_buf())
    });
    set_recent_mode(&h, cx, RecentStatusMode::Check);
    drain_recent(&h, cx);
    h.read(cx, |osier, _| {
        let b = osier.recent_badge(other.path()).expect("checked");
        assert_eq!((b.dirty, b.ahead, b.behind), (Some(true), 3, 0));
    });
    // Focus regained: everything is queued again.
    h.act(cx, |osier, _, _| osier.recent_status_on_focus());
    h.read(cx, |osier, _| assert!(osier.recent_status_queue_len() >= 1));
}

#[gpui::test]
fn scan_now_follows_the_mode(cx: &mut TestAppContext) {
    use osier_config::settings::RecentStatusMode;
    let repo = repo_with_upstream(0, false);
    let a = repo_with_upstream(1, true);
    let b = repo_with_upstream(0, false);
    let h = Harness::open(cx, &repo);
    h.act(cx, |osier, _, _| {
        osier.state.recent.push(a.path().to_path_buf());
        osier.state.recent.push(b.path().to_path_buf());
    });
    // Never: nothing queued, nothing shown.
    h.act(cx, |osier, window, cx| osier.scan_recent_now(window, cx));
    h.read(cx, |o, _| assert_eq!(o.recent_status_queue_len(), 0));
    // Cached: a one-off full check, so the change marker becomes known.
    set_recent_mode(&h, cx, RecentStatusMode::Cached);
    h.act(cx, |osier, window, cx| osier.scan_recent_now(window, cx));
    drain_recent(&h, cx);
    h.read(cx, |o, _| {
        let badge = o.recent_badge(a.path()).expect("scanned");
        assert_eq!((badge.dirty, badge.ahead), (Some(true), 1));
        assert!(o.recent_badge(b.path()).is_some());
    });
    // Watch: refs only; the change marker from the earlier full check is kept.
    set_recent_mode(&h, cx, RecentStatusMode::Watch);
    drain_recent(&h, cx);
    a.git(["commit", "-q", "--allow-empty", "-m", "more"]);
    h.act(cx, |osier, window, cx| osier.scan_recent_now(window, cx));
    drain_recent(&h, cx);
    h.read(cx, |o, _| {
        let badge = o.recent_badge(a.path()).unwrap();
        assert_eq!(badge.ahead, 2);
        assert_eq!(
            badge.dirty,
            Some(true),
            "refs-only scan keeps the last marker"
        );
    });
}

#[gpui::test]
fn failed_background_checks_reach_the_command_log(cx: &mut TestAppContext) {
    use osier_config::settings::RecentStatusMode;
    let repo = repo_with_upstream(0, false);
    let healthy = repo_with_upstream(1, false);
    // A repository whose .git points nowhere: git fails on it.
    let broken = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        broken.path().join(".git"),
        "gitdir: /nonexistent/osier-test\n",
    )
    .expect("write .git");
    let h = Harness::open(cx, &repo);
    let before = h.read(cx, |o, _| o.sink.records().len());
    h.act(cx, |osier, _, _| {
        osier.state.recent.push(healthy.path().to_path_buf());
        osier.state.recent.push(broken.path().to_path_buf());
    });
    set_recent_mode(&h, cx, RecentStatusMode::Check);
    drain_recent(&h, cx);
    h.read(cx, |o, _| {
        let new: Vec<_> = o.sink.records().into_iter().skip(before).collect();
        assert!(!new.is_empty(), "the broken check is logged");
        assert!(new.iter().all(|r| !r.expected), "only failures are logged");
        assert!(
            new.iter().all(|r| r.cwd.as_deref() == Some(broken.path())),
            "the healthy repository's checks stay out of the log"
        );
        assert!(o.recent_badge(healthy.path()).is_some());
    });
}

#[gpui::test]
fn recent_status_watch_mode_follows_commits_without_a_change_marker(cx: &mut TestAppContext) {
    use osier_config::settings::RecentStatusMode;
    let repo = repo_with_upstream(0, false);
    let other = repo_with_upstream(1, true);
    let h = Harness::open(cx, &repo);
    h.act(cx, |osier, _, _| {
        osier.state.recent.push(other.path().to_path_buf())
    });
    set_recent_mode(&h, cx, RecentStatusMode::Watch);
    drain_recent(&h, cx);
    h.read(cx, |osier, _| {
        let b = osier.recent_badge(other.path()).expect("checked");
        assert_eq!((b.ahead, b.dirty), (1, None), "refs only in watch mode");
        assert!(!osier.recent_status.shows_changes());
    });
    // A commit made by another tool reaches us through the .git watcher.
    other.git(["add", "new.txt"]);
    other.git(["commit", "-qm", "outside"]);
    let mut ahead = 1;
    for _ in 0..50 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        drain_recent(&h, cx);
        ahead = h.read(cx, |o, _| {
            o.recent_badge(other.path()).map(|b| b.ahead).unwrap_or(0)
        });
        if ahead == 2 {
            break;
        }
    }
    assert_eq!(ahead, 2, "the watcher saw the commit");
}
