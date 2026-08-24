//! Git command log: every command the app runs, shown in a bottom panel.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

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
