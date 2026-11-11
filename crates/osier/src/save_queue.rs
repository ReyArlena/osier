//! Ordered file writes off the UI thread: settings, state, the status cache, keymap files.
//!
//! One thread runs the jobs in the order they were queued, so a later save of a file can
//! never be overtaken by an earlier one. Serialising stays with the caller (cheap); the write
//! (temp file and rename) happens here. Osier flushes the queue when it quits, so the last
//! change is on disk before the process exits.
//!
//! Tests run each job at once, so they can read the file straight after the action.

use std::sync::mpsc;

type Job = Box<dyn FnOnce() + Send>;

enum Message {
    Job(Job),
    Flush(mpsc::Sender<()>),
}

pub(crate) struct SaveQueue {
    tx: Option<mpsc::Sender<Message>>,
}

impl SaveQueue {
    pub(crate) fn new() -> Self {
        if cfg!(test) {
            return Self { tx: None };
        }
        let (tx, rx) = mpsc::channel::<Message>();
        let spawned = std::thread::Builder::new()
            .name("osier-save".into())
            .spawn(move || {
                for message in rx {
                    match message {
                        Message::Job(job) => job(),
                        Message::Flush(done) => {
                            let _ = done.send(());
                        }
                    }
                }
            });
        match spawned {
            Ok(_) => Self { tx: Some(tx) },
            Err(error) => {
                tracing::warn!(%error, "no save thread, saving on the UI thread");
                Self { tx: None }
            }
        }
    }

    /// Queue a write. Runs at once when there is no thread (tests, or it could not start).
    pub(crate) fn push(&self, job: impl FnOnce() + Send + 'static) {
        match &self.tx {
            Some(tx) => {
                if let Err(mpsc::SendError(Message::Job(job))) =
                    tx.send(Message::Job(Box::new(job)))
                {
                    job();
                }
            }
            None => job(),
        }
    }

    /// Wait until every queued write is done (quitting). Bounded, so a stuck disk cannot
    /// hang the exit for long.
    pub(crate) fn flush(&self) {
        let Some(tx) = &self.tx else {
            return;
        };
        let (done_tx, done_rx) = mpsc::channel();
        if tx.send(Message::Flush(done_tx)).is_ok() {
            let _ = done_rx.recv_timeout(std::time::Duration::from_secs(3));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SaveQueue;
    use std::sync::{Arc, Mutex};

    #[test]
    fn jobs_run_in_order() {
        let queue = SaveQueue::new();
        let log = Arc::new(Mutex::new(Vec::new()));
        for n in 0..5 {
            let log = log.clone();
            queue.push(move || log.lock().unwrap().push(n));
        }
        queue.flush();
        assert_eq!(*log.lock().unwrap(), [0, 1, 2, 3, 4]);
    }
}
