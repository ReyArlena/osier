//! Updates in the app: a daily check at startup, the "Update available" status bar badge and
//! Settings > About > Updates (status, Check now, Update, Restart now). The steps are the
//! ones `osier --update` uses (`update.rs`); [`UpdateSource`] lets tests fake them.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui_kit::*;

use crate::app::Osier;
use crate::install_route::InstallRoute;
use crate::notices::WindowNoticeExt as _;
use crate::update::{self, Decision};

/// Startup checks are skipped when the last one is younger than this.
const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// Network and process steps, faked in tests.
pub(crate) trait UpdateSource: Send + Sync {
    fn latest(&self) -> Result<String, String>;
    fn install(&self, bin_dir: &Path, tag: &str) -> Result<String, String>;
    fn restart(&self, binary: &Path) -> Result<(), String>;
}

#[cfg_attr(test, allow(dead_code))]
struct Real;

impl UpdateSource for Real {
    fn latest(&self) -> Result<String, String> {
        update::latest_release()
    }
    fn install(&self, bin_dir: &Path, tag: &str) -> Result<String, String> {
        update::install(bin_dir, tag, true)
    }
    fn restart(&self, binary: &Path) -> Result<(), String> {
        update::restart(binary, std::process::id()).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
struct Offline;

#[cfg(test)]
impl UpdateSource for Offline {
    fn latest(&self) -> Result<String, String> {
        Err("no network in tests".into())
    }
    fn install(&self, _: &Path, _: &str) -> Result<String, String> {
        Err("no installs in tests".into())
    }
    fn restart(&self, _: &Path) -> Result<(), String> {
        Err("no restarts in tests".into())
    }
}

pub(crate) fn default_source() -> Arc<dyn UpdateSource> {
    #[cfg(test)]
    {
        Arc::new(Offline)
    }
    #[cfg(not(test))]
    {
        Arc::new(Real)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum CheckState {
    #[default]
    NotChecked,
    Checking,
    /// The latest release and how this build compares with it.
    Done {
        latest: String,
        decision: Decision,
    },
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum InstallState {
    #[default]
    Idle,
    Installing(String),
    /// Installed this tag; a restart runs it.
    Installed(String),
    Failed(String),
}

/// Everything the Updates section and the badge show.
pub(crate) struct Updates {
    pub source: Arc<dyn UpdateSource>,
    /// This build's version (`runtime_info::VERSION`; tests set a release version).
    pub current: String,
    pub check: CheckState,
    pub install: InstallState,
    /// How Osier was installed; looked up when first needed (it asks the package managers).
    pub route: Option<InstallRoute>,
}

impl Default for Updates {
    fn default() -> Self {
        Self {
            source: default_source(),
            current: crate::runtime_info::VERSION.to_string(),
            check: CheckState::NotChecked,
            install: InstallState::Idle,
            route: None,
        }
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl Osier {
    /// At startup: reuse a check younger than a day, else ask GitHub in the background.
    pub(crate) fn startup_update_check(&mut self, cx: &mut Context<Self>) {
        if !self.settings.updates.check_on_startup {
            return;
        }
        let cached = &self.state.update_check;
        let fresh = cached
            .checked_at
            .is_some_and(|t| now() - t < CHECK_EVERY.as_secs() as i64);
        if fresh && let Some(latest) = cached.latest.clone() {
            self.apply_latest(latest);
            return;
        }
        self.check_for_updates(cx);
    }

    fn apply_latest(&mut self, latest: String) {
        let decision = update::decide(&self.updates.current, &latest).unwrap_or(Decision::UpToDate);
        self.updates.check = CheckState::Done { latest, decision };
    }

    /// "Check now", and the startup check when the cached one is stale.
    pub(crate) fn check_for_updates(&mut self, cx: &mut Context<Self>) {
        if self.updates.check == CheckState::Checking {
            return;
        }
        self.updates.check = CheckState::Checking;
        cx.notify();
        let source = self.updates.source.clone();
        let busy = self.background.begin(crate::background::UPDATE_CHECK);
        cx.spawn(async move |this, cx| {
            let _busy = busy;
            let result = cx.background_spawn(async move { source.latest() }).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(latest) => {
                        this.state.update_check.checked_at = Some(now());
                        this.state.update_check.latest = Some(latest.clone());
                        this.save_state();
                        this.apply_latest(latest);
                    }
                    Err(e) => this.updates.check = CheckState::Failed(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The release to show in the status bar badge, if there is one.
    pub(crate) fn update_badge(&self) -> Option<&str> {
        match &self.updates.check {
            CheckState::Done {
                latest,
                decision: Decision::Update,
            } if !matches!(self.updates.install, InstallState::Installed(_)) => Some(latest),
            _ => None,
        }
    }

    /// The install route, looked up once.
    pub(crate) fn install_route(&mut self) -> &InstallRoute {
        self.updates.route.get_or_insert_with(InstallRoute::detect)
    }

    /// "Update to vX.Y.Z": run the installer in the background.
    pub(crate) fn install_update(&mut self, cx: &mut Context<Self>) {
        let Some(tag) = self.update_badge().map(str::to_string) else {
            return;
        };
        let InstallRoute::Script { bin_dir } = self.install_route().clone() else {
            return;
        };
        if matches!(self.updates.install, InstallState::Installing(_)) {
            return;
        }
        self.updates.install = InstallState::Installing(tag.clone());
        cx.notify();
        let source = self.updates.source.clone();
        let busy = self.background.begin(crate::background::UPDATE_INSTALL);
        cx.spawn(async move |this, cx| {
            let _busy = busy;
            let t = tag.clone();
            let result = cx
                .background_spawn(async move { source.install(&bin_dir, &t) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.updates.install = match result {
                    Ok(output) => {
                        tracing::info!(%tag, "updated osier: {}", output.trim());
                        InstallState::Installed(tag)
                    }
                    Err(e) => InstallState::Failed(e),
                };
                cx.notify();
            });
        })
        .detach();
    }

    /// "Restart now" after an update: start the new binary once this one has exited.
    pub(crate) fn restart_after_update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(InstallRoute::Script { bin_dir }) = self.updates.route.clone() else {
            return;
        };
        let binary: PathBuf = bin_dir.join("osier");
        self.save_state();
        match self.updates.source.restart(&binary) {
            Ok(()) => cx.quit(),
            Err(e) => window.push_notice(
                crate::notices::Notice::warning(format!("{e}. Close Osier and start it again."))
                    .title("Could not restart"),
                cx,
            ),
        }
    }
}
