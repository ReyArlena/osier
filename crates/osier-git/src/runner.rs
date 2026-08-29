//! Runs `git` and records every invocation.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use wait_timeout::ChildExt;

use crate::error::{GitError, Result};

/// Record of one git invocation. Shown in the command log and attached to errors.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CommandRecord {
    pub id: u64,
    /// `git` plus every argument, exactly as executed.
    pub argv: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub started: SystemTime,
    pub duration: Duration,
    /// Exit status. `None` when the process was killed (timeout).
    pub status: Option<i32>,
    /// Whether the caller considers `status` a success (some commands signal results through
    /// non-zero exits, e.g. `diff --no-index` or `config --get` on a missing key).
    pub expected: bool,
    pub stdout_len: usize,
    /// Stderr, lossy utf-8, capped to 64 KiB.
    pub stderr: String,
}

impl CommandRecord {
    /// Shell-escaped rendering for "copy as shell".
    pub fn as_shell(&self) -> String {
        self.argv
            .iter()
            .map(|a| shell_quote(a))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn shell_quote(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./=:@%+,".contains(c))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// Receives command records. Implemented by the UI command log.
pub trait CommandSink: Send + Sync {
    fn record(&self, record: &CommandRecord);
}

/// Discards records.
#[derive(Debug, Default)]
pub struct NoopSink;

impl CommandSink for NoopSink {
    fn record(&self, _record: &CommandRecord) {}
}

/// Keeps records in memory. Used by tests and by the command log panel.
#[derive(Debug, Default)]
pub struct MemorySink {
    records: Mutex<Vec<CommandRecord>>,
}

impl MemorySink {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn records(&self) -> Vec<CommandRecord> {
        self.records.lock().map(|r| r.clone()).unwrap_or_default()
    }

    pub fn clear(&self) {
        if let Ok(mut r) = self.records.lock() {
            r.clear();
        }
    }
}

impl CommandSink for MemorySink {
    fn record(&self, record: &CommandRecord) {
        if let Ok(mut r) = self.records.lock() {
            r.push(record.clone());
        }
    }
}

/// Raw result of a finished git process.
#[derive(Debug, Clone)]
pub struct Output {
    pub record: CommandRecord,
    pub stdout: Vec<u8>,
}

impl Output {
    pub fn stdout_str(&self) -> Result<&str> {
        std::str::from_utf8(&self.stdout).map_err(|_| GitError::Utf8("stdout"))
    }
}

/// Executes git with a controlled environment.
///
/// Cheap to clone: the sink is shared, everything else is small.
#[derive(Clone)]
pub struct GitRunner {
    git: OsString,
    env: BTreeMap<OsString, OsString>,
    env_remove: Vec<OsString>,
    timeout: Duration,
    sink: Arc<dyn CommandSink>,
    next_id: Arc<AtomicU64>,
}

impl std::fmt::Debug for GitRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitRunner")
            .field("git", &self.git)
            .field("env", &self.env)
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

impl Default for GitRunner {
    fn default() -> Self {
        Self::new(Arc::new(NoopSink))
    }
}

/// Stderr kept in a record, in bytes.
const STDERR_CAP: usize = 64 * 1024;

impl GitRunner {
    pub fn new(sink: Arc<dyn CommandSink>) -> Self {
        let mut env = BTreeMap::new();
        // Never hang on a hidden prompt.
        env.insert("GIT_TERMINAL_PROMPT".into(), "0".into());
        // Stable, parseable messages.
        env.insert("LC_ALL".into(), "C".into());
        env.insert("LANG".into(), "C".into());
        // Reads (status, diff) must never write the index: that would wake our own file watcher
        // and loop. Mandatory locks for mutations are unaffected.
        env.insert("GIT_OPTIONAL_LOCKS".into(), "0".into());
        Self {
            git: std::env::var_os("OSIER_GIT").unwrap_or_else(|| "git".into()),
            env,
            env_remove: vec![
                "GIT_DIR".into(),
                "GIT_WORK_TREE".into(),
                "GIT_INDEX_FILE".into(),
            ],
            timeout: Duration::from_secs(120),
            sink,
            next_id: Arc::new(AtomicU64::new(1)),
        }
    }

    pub fn with_git(mut self, git: impl Into<OsString>) -> Self {
        self.git = git.into();
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    pub fn git_path(&self) -> &OsStr {
        &self.git
    }

    /// Run git with `-C repo` (when given) and the arguments. Succeeds only on exit 0.
    pub fn run<I, S>(&self, repo: Option<&Path>, args: I) -> Result<Output>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = self.execute(repo, args, None, &[0])?;
        match output.record.status {
            Some(0) => Ok(output),
            Some(status) => {
                let stdout_head =
                    String::from_utf8_lossy(&output.stdout[..output.stdout.len().min(STDERR_CAP)])
                        .into_owned();
                Err(classify(status, output.record, &stdout_head))
            }
            None => Err(GitError::timeout(self.timeout.as_secs(), output.record)),
        }
    }

    /// Run git and return the output whatever the exit status (killed on timeout).
    pub fn run_unchecked<I, S>(
        &self,
        repo: Option<&Path>,
        args: I,
        stdin: Option<&[u8]>,
    ) -> Result<Output>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.execute(repo, args, stdin, &[0])
    }

    /// Like [`run_unchecked`](Self::run_unchecked), but exit codes in `allowed` are recorded as
    /// expected so the command log does not flag them.
    pub fn run_allowing<I, S>(
        &self,
        repo: Option<&Path>,
        args: I,
        allowed: &[i32],
    ) -> Result<Output>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.execute(repo, args, None, allowed)
    }

    fn execute<I, S>(
        &self,
        repo: Option<&Path>,
        args: I,
        stdin: Option<&[u8]>,
        allowed: &[i32],
    ) -> Result<Output>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut argv: Vec<String> = vec![self.git.to_string_lossy().into_owned()];
        let mut cmd = Command::new(&self.git);
        if let Some(repo) = repo {
            cmd.arg("-C").arg(repo);
            argv.push("-C".into());
            argv.push(repo.to_string_lossy().into_owned());
        }
        cmd.arg("--no-pager");
        argv.push("--no-pager".into());
        for pair in ["color.ui=never", "core.quotePath=false"] {
            cmd.arg("-c").arg(pair);
            argv.push("-c".into());
            argv.push(pair.into());
        }
        for a in args {
            let a = a.as_ref();
            argv.push(a.to_string_lossy().into_owned());
            cmd.arg(a);
        }
        for key in &self.env_remove {
            cmd.env_remove(key);
        }
        cmd.envs(&self.env);
        cmd.stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let started = SystemTime::now();
        let clock = Instant::now();
        tracing::debug!(id, argv = %argv.join(" "), "git");

        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                GitError::GitNotFound(self.git.to_string_lossy().into_owned())
            } else {
                GitError::Io(e)
            }
        })?;

        if let (Some(data), Some(mut pipe)) = (stdin, child.stdin.take()) {
            use std::io::Write;
            // A closed pipe just means git did not want the input.
            let _ = pipe.write_all(data);
        }

        // Drain pipes on threads so a chatty git never deadlocks on a full pipe.
        let mut stdout_pipe = child.stdout.take();
        let mut stderr_pipe = child.stderr.take();
        let stdout_thread = std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(p) = stdout_pipe.as_mut() {
                let _ = p.read_to_end(&mut buf);
            }
            buf
        });
        let stderr_thread = std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(p) = stderr_pipe.as_mut() {
                let _ = p.read_to_end(&mut buf);
            }
            buf
        });

        let status = match child.wait_timeout(self.timeout)? {
            Some(status) => status.code(),
            None => {
                let _ = child.kill();
                let _ = child.wait();
                None
            }
        };
        let stdout = stdout_thread.join().unwrap_or_default();
        let mut stderr = stderr_thread.join().unwrap_or_default();
        stderr.truncate(STDERR_CAP);

        let record = CommandRecord {
            id,
            argv,
            cwd: repo.map(Path::to_path_buf),
            started,
            duration: clock.elapsed(),
            status,
            expected: status.is_some_and(|s| allowed.contains(&s)),
            stdout_len: stdout.len(),
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
        };
        self.sink.record(&record);
        if !record.expected {
            tracing::debug!(id, ?status, stderr = %record.stderr.trim(), "git failed");
        }
        Ok(Output { record, stdout })
    }
}

/// Map a non-zero exit into the most specific error we can recognise from stderr (and stdout,
/// where git prints `CONFLICT` lines).
fn classify(status: i32, record: CommandRecord, stdout: &str) -> GitError {
    let err = record.stderr.as_str();
    let has = |needle: &str| err.contains(needle) || stdout.contains(needle);
    if has("index.lock") || has("Unable to create") && has(".lock") {
        GitError::Locked(Box::new(record))
    } else if has("Authentication failed")
        || has("could not read Username")
        || has("could not read Password")
        || has("Permission denied (publickey")
        || has("terminal prompts disabled")
    {
        GitError::AuthRequired(Box::new(record))
    } else if has("non-fast-forward") || has("fetch first") || has("stale info") {
        GitError::NonFastForward(Box::new(record))
    } else if has("CONFLICT (")
        || has("Automatic merge failed")
        || has("could not apply")
        || has("could not revert")
    {
        GitError::Conflict(Box::new(record))
    } else if has("would be overwritten by") || has("commit your changes or stash them") {
        GitError::Dirty(Box::new(record))
    } else if has("is not fully merged") {
        GitError::BranchNotMerged(Box::new(record))
    } else if has("not a git repository") {
        GitError::NotARepo(record.cwd.clone().unwrap_or_default())
    } else {
        GitError::failed(status, record)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_rendering_quotes_only_when_needed() {
        let rec = CommandRecord {
            id: 1,
            argv: vec![
                "git".into(),
                "commit".into(),
                "-m".into(),
                "it's done".into(),
            ],
            cwd: None,
            started: SystemTime::UNIX_EPOCH,
            duration: Duration::ZERO,
            status: Some(0),
            expected: true,
            stdout_len: 0,
            stderr: String::new(),
        };
        assert_eq!(rec.as_shell(), "git commit -m 'it'\\''s done'");
    }

    #[test]
    fn version_runs_without_a_repo() {
        let sink = MemorySink::new();
        let runner = GitRunner::new(sink.clone());
        let out = runner.run(None, ["--version"]).expect("git --version");
        assert!(out.stdout_str().unwrap().starts_with("git version"));
        assert_eq!(sink.records().len(), 1);
        assert_eq!(sink.records()[0].status, Some(0));
    }

    #[test]
    fn missing_binary_is_git_not_found() {
        let runner = GitRunner::default().with_git("/nonexistent/git-binary");
        match runner.run(None, ["--version"]) {
            Err(GitError::GitNotFound(_)) => {}
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn classifies_not_a_repo() {
        let dir = tempfile::tempdir().unwrap();
        let runner = GitRunner::default();
        match runner.run(Some(dir.path()), ["status"]) {
            Err(GitError::NotARepo(_)) => {}
            other => panic!("unexpected: {other:?}"),
        }
    }
}
