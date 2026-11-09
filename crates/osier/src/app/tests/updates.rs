//! Update checks and installs (`update_ui.rs`).

use super::*;

/// GitHub releases and the installer as the update flow sees them, offline.
#[derive(Default)]
struct FakeUpdates {
    latest: std::sync::Mutex<Option<String>>,
    checks: std::sync::atomic::AtomicUsize,
    installs: std::sync::Mutex<Vec<(std::path::PathBuf, String)>>,
}

impl crate::update_ui::UpdateSource for FakeUpdates {
    fn latest(&self) -> Result<String, String> {
        self.checks
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.latest
            .lock()
            .expect("lock")
            .clone()
            .ok_or_else(|| "offline".to_string())
    }
    fn install(&self, bin_dir: &std::path::Path, tag: &str) -> Result<String, String> {
        self.installs
            .lock()
            .expect("lock")
            .push((bin_dir.to_path_buf(), tag.to_string()));
        Ok("Installed osier".into())
    }
    fn restart(&self, _: &std::path::Path) -> Result<(), String> {
        Err("not in tests".into())
    }
}

/// A harness on release v0.1.0, installed by the script, with `latest` published.
fn updates_harness(
    cx: &mut TestAppContext,
    latest: Option<&str>,
    route: crate::install_route::InstallRoute,
) -> (Harness, TempRepo, Arc<FakeUpdates>) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    let h = Harness::open(cx, &repo);
    let fake = Arc::new(FakeUpdates::default());
    *fake.latest.lock().expect("lock") = latest.map(String::from);
    let source = fake.clone();
    h.act(cx, move |o, _, _| {
        o.updates.source = source;
        o.updates.current = "0.1.0".into();
        o.updates.route = Some(route);
        o.updates.check = crate::update_ui::CheckState::NotChecked;
        o.state.update_check = Default::default();
    });
    (h, repo, fake)
}

fn script_route() -> crate::install_route::InstallRoute {
    crate::install_route::InstallRoute::Script {
        bin_dir: "/home/x/.local/bin".into(),
    }
}

#[gpui::test]
fn ui_update_badge_opens_the_updates_section(cx: &mut TestAppContext) {
    let (h, _repo, fake) = updates_harness(cx, Some("v0.2.0"), script_route());
    assert!(
        h.bounds(cx, "update-badge").is_none(),
        "nothing checked yet"
    );
    h.act(cx, |o, _, cx| o.startup_update_check(cx));
    assert_eq!(fake.checks.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(h.bounds(cx, "update-badge").is_some());
    h.read(cx, |o, _| {
        assert_eq!(o.state.update_check.latest.as_deref(), Some("v0.2.0"));
        assert!(o.state.update_check.checked_at.is_some());
    });
    h.click(cx, "update-badge");
    assert!(h.has_settings(cx));
    assert_eq!(
        h.read(cx, |o, _| o.settings_highlight.map(|(t, _)| t)),
        Some(crate::settings_panel::SettingsTarget::UPDATES)
    );
    assert!(
        h.bounds(cx, "updates-section").is_some(),
        "About > Updates shown"
    );
    assert!(h.bounds(cx, "update-release-notes").is_some());
}

#[gpui::test]
fn startup_reuses_a_recent_check_and_respects_the_setting(cx: &mut TestAppContext) {
    let (h, _repo, fake) = updates_harness(cx, Some("v0.3.0"), script_route());
    let calls = |fake: &FakeUpdates| fake.checks.load(std::sync::atomic::Ordering::SeqCst);
    // Checked an hour ago: no request, the cached result shows.
    h.act(cx, |o, _, cx| {
        o.state.update_check.checked_at = Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_secs() as i64
                - 3600,
        );
        o.state.update_check.latest = Some("v0.2.0".into());
        o.startup_update_check(cx);
    });
    assert_eq!(calls(&fake), 0);
    assert_eq!(
        h.read(cx, |o, _| o.update_badge().map(String::from))
            .as_deref(),
        Some("v0.2.0")
    );
    // Two days old: asks again.
    h.act(cx, |o, _, cx| {
        o.state.update_check.checked_at = Some(0);
        o.startup_update_check(cx);
    });
    assert_eq!(calls(&fake), 1);
    assert_eq!(
        h.read(cx, |o, _| o.update_badge().map(String::from))
            .as_deref(),
        Some("v0.3.0")
    );
    // Turned off: never asks.
    h.act(cx, |o, _, cx| {
        o.settings.updates.check_on_startup = false;
        o.state.update_check.checked_at = Some(0);
        o.startup_update_check(cx);
    });
    assert_eq!(calls(&fake), 1);
    // Up to date or a development build: no badge.
    h.act(cx, |o, _, cx| {
        *fake.latest.lock().expect("lock") = Some("v0.1.0".into());
        o.check_for_updates(cx);
    });
    assert!(h.read(cx, |o, _| o.update_badge().is_none()));
    h.act(cx, |o, _, cx| {
        o.updates.current = "0.1.0+3.gabcdef0".into();
        *fake.latest.lock().expect("lock") = Some("v0.2.0".into());
        o.check_for_updates(cx);
    });
    assert!(h.read(cx, |o, _| o.update_badge().is_none()));
}

#[gpui::test]
fn ui_update_installs_into_the_script_prefix_and_offers_a_restart(cx: &mut TestAppContext) {
    let (h, _repo, fake) = updates_harness(cx, Some("v0.2.0"), script_route());
    h.act(cx, |o, _, cx| o.check_for_updates(cx));
    h.act(cx, |o, w, cx| {
        o.open_settings_at(crate::settings_panel::SettingsTarget::UPDATES, w, cx)
    });
    assert!(h.bounds(cx, "update-restart").is_none());
    h.click(cx, "update-install");
    assert_eq!(
        fake.installs.lock().expect("lock").clone(),
        [("/home/x/.local/bin".into(), "v0.2.0".to_string())]
    );
    assert!(h.bounds(cx, "update-restart").is_some());
    assert!(
        h.bounds(cx, "update-badge").is_none(),
        "installed: no more badge"
    );
}

#[gpui::test]
fn ui_package_installs_get_the_badge_but_no_update_button(cx: &mut TestAppContext) {
    let route = crate::install_route::InstallRoute::Package {
        manager: "pacman",
        name: "osier-bin".into(),
    };
    let (h, _repo, fake) = updates_harness(cx, Some("v0.2.0"), route);
    h.act(cx, |o, _, cx| o.check_for_updates(cx));
    assert!(
        h.bounds(cx, "update-badge").is_some(),
        "the news is shown anyway"
    );
    h.click(cx, "update-badge");
    h.click(cx, "update-install");
    assert!(
        fake.installs.lock().expect("lock").is_empty(),
        "the button is disabled"
    );
    h.read(cx, |o, _| {
        assert_eq!(o.updates.install, crate::update_ui::InstallState::Idle)
    });
}
