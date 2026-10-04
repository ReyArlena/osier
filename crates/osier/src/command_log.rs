//! Git command log: every command the app runs, shown in a bottom panel.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use osier_git::{CommandRecord, CommandSink};

const CAP: usize = 1000;

/// Thread-safe sink shared by every runner. The panel polls it after tasks finish.
#[derive(Debug, Default)]
pub struct UiCommandSink {
    records: Mutex<Vec<CommandRecord>>,
    version: AtomicU64,
}

impl UiCommandSink {
    pub fn records(&self) -> Vec<CommandRecord> {
        self.records.lock().map(|r| r.clone()).unwrap_or_default()
    }

    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Relaxed)
    }

    pub fn clear(&self) {
        if let Ok(mut r) = self.records.lock() {
            r.clear();
        }
        self.version.fetch_add(1, Ordering::Relaxed);
    }
}

impl CommandSink for UiCommandSink {
    fn record(&self, record: &CommandRecord) {
        if let Ok(mut r) = self.records.lock() {
            if r.len() >= CAP {
                let drop = r.len() - CAP + 1;
                r.drain(..drop);
            }
            r.push(record.clone());
        }
        self.version.fetch_add(1, Ordering::Relaxed);
    }
}

/// Passes on only commands that failed unexpectedly. Background checks (status of projects
/// that are not open) run every few minutes across many repositories; logging every success
/// would bury the user's own commands, but a failure is worth seeing.
#[derive(Debug)]
pub struct FailuresOnly(pub Arc<UiCommandSink>);

impl CommandSink for FailuresOnly {
    fn record(&self, record: &CommandRecord) {
        if !record.expected {
            self.0.record(record);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use osier_git::GitRunner;

    use super::{FailuresOnly, UiCommandSink};

    #[test]
    fn only_unexpected_failures_reach_the_log() {
        let ui = Arc::new(UiCommandSink::default());
        let runner = GitRunner::new(Arc::new(FailuresOnly(ui.clone())));
        let dir = tempfile::tempdir().expect("tempdir");
        let _ = runner.run(None, ["--version"]);
        // Exit 128 allowed by the caller: expected, so not logged.
        let _ = runner.run_allowing(Some(dir.path()), ["rev-parse", "HEAD"], &[0, 128]);
        assert!(
            ui.records().is_empty(),
            "successes and expected exits stay out"
        );
        let _ = runner.run(Some(dir.path()), ["status"]);
        let records = ui.records();
        assert_eq!(records.len(), 1);
        assert!(!records[0].expected);
    }
}
