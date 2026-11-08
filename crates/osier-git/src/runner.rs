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

    /// Same settings, different command sink (e.g. one that records only failures).
    pub fn with_sink(mut self, sink: Arc<dyn CommandSink>) -> Self {
        self.sink = sink;
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

    /// [`run_allowing`](Self::run_allowing) with `stdin` fed to git.
    pub fn run_allowing_with_stdin<I, S>(
        &self,
        repo: Option<&Path>,
        args: I,
        stdin: &[u8],
        allowed: &[i32],
    ) -> Result<Output>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.execute(repo, args, Some(stdin), allowed)
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
    } else if has("non-fast-forward")
        || has("fetch first")
        || has("stale info")
        || has("Not possible to fast-forward")
        || has("cannot fast-forward")
    {
        GitError::NonFastForward(Box::new(record))
    } else if has("has no upstream branch") || has("no tracking information") {
        GitError::NoUpstream(Box::new(record))
    } else if has("Could not resolve host")
        || has("Connection refused")
        || has("Connection timed out")
        || has("unable to access")
        || has("Could not read from remote repository")
        || has("Network is unreachable")
    {
        GitError::Network(Box::new(record))
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

// ----- streaming execution with progress and cancellation -----

/// Shared flag a long operation polls; set it to kill the git process.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<std::sync::atomic::AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// One line of progress from git's stderr (`--progress`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    /// `Receiving objects`, `Compressing objects`, `remote: Counting objects`, ...
    pub phase: String,
    pub percent: Option<u8>,
    pub line: String,
}

impl Progress {
    /// Parse a stderr line. Returns `None` for lines that are not progress.
    pub fn parse(line: &str) -> Option<Self> {
        let line = line.trim();
        let (phase, rest) = line.rsplit_once(':')?;
        let rest = rest.trim_start();
        let percent = rest
            .split('%')
            .next()
            .and_then(|p| p.trim().parse::<u8>().ok());
        if percent.is_none() && !rest.contains("done") {
            return None;
        }
        Some(Self {
            phase: phase.trim().to_string(),
            percent,
            line: line.to_string(),
        })
    }
}

impl GitRunner {
    /// Run git, forwarding stderr lines to `on_line` as they arrive (progress lives there),
    /// and killing the process when `cancel` fires. Succeeds only on exit 0.
    pub fn run_streaming<I, S>(
        &self,
        repo: Option<&Path>,
        args: I,
        cancel: Option<CancelToken>,
        mut on_line: impl FnMut(&str) + Send + 'static,
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
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let started = SystemTime::now();
        let clock = Instant::now();
        tracing::debug!(id, argv = %redact(&argv).join(" "), "git (streaming)");

        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                GitError::GitNotFound(self.git.to_string_lossy().into_owned())
            } else {
                GitError::Io(e)
            }
        })?;

        // Reader threads write into shared buffers and are never joined after a kill: an
        // orphaned child (ssh, a remote helper) can keep the pipe open for a while.
        let stdout_buf: Arc<Mutex<Vec<u8>>> = Arc::default();
        let stderr_buf: Arc<Mutex<String>> = Arc::default();
        let mut stdout_pipe = child.stdout.take();
        let stdout_thread = {
            let buf = stdout_buf.clone();
            std::thread::spawn(move || {
                let mut local = Vec::new();
                if let Some(p) = stdout_pipe.as_mut() {
                    let _ = p.read_to_end(&mut local);
                }
                if let Ok(mut b) = buf.lock() {
                    *b = local;
                }
            })
        };
        let stderr_pipe = child.stderr.take();
        let stderr_thread = {
            let buf = stderr_buf.clone();
            std::thread::spawn(move || {
                // Progress updates are separated by `\r`, final lines by `\n`.
                let Some(mut pipe) = stderr_pipe else {
                    return;
                };
                let mut current = Vec::new();
                let mut byte = [0u8; 1];
                let mut flush = |current: &mut Vec<u8>| {
                    if current.is_empty() {
                        return;
                    }
                    let line = String::from_utf8_lossy(current).into_owned();
                    on_line(&line);
                    if let Ok(mut all) = buf.lock()
                        && all.len() < STDERR_CAP
                    {
                        all.push_str(&line);
                        all.push('\n');
                    }
                    current.clear();
                };
                loop {
                    match pipe.read(&mut byte) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => match byte[0] {
                            b'\r' | b'\n' => flush(&mut current),
                            b => current.push(b),
                        },
                    }
                }
                flush(&mut current);
            })
        };

        let deadline = Instant::now() + self.timeout;
        let mut cancelled = false;
        let status = loop {
            if let Some(s) = child.try_wait()? {
                break s.code();
            }
            if cancel.as_ref().is_some_and(CancelToken::is_cancelled) {
                cancelled = true;
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            std::thread::sleep(Duration::from_millis(30));
        };
        if status.is_some() {
            let _ = stdout_thread.join();
            let _ = stderr_thread.join();
        }
        let stdout = stdout_buf.lock().map(|b| b.clone()).unwrap_or_default();
        let stderr = stderr_buf.lock().map(|b| b.clone()).unwrap_or_default();

        let record = CommandRecord {
            id,
            argv: redact(&argv),
            cwd: repo.map(Path::to_path_buf),
            started,
            duration: clock.elapsed(),
            status,
            expected: status == Some(0),
            stdout_len: stdout.len(),
            stderr,
        };
        self.sink.record(&record);
        match status {
            Some(0) => Ok(Output { record, stdout }),
            Some(s) => Err(classify_with_stdout(s, record, &stdout)),
            None if cancelled => Err(GitError::Cancelled(Box::new(record))),
            None => Err(GitError::timeout(self.timeout.as_secs(), record)),
        }
    }
}

fn classify_with_stdout(status: i32, record: CommandRecord, stdout: &[u8]) -> GitError {
    let head = String::from_utf8_lossy(&stdout[..stdout.len().min(STDERR_CAP)]).into_owned();
    classify(status, record, &head)
}

/// Hide credential material that may appear in argv (`credential.helper=... --token ...`).
fn redact(argv: &[String]) -> Vec<String> {
    argv.iter()
        .map(|a| {
            if let Some((k, _)) = a.split_once('=')
                && (k.ends_with("password") || k.ends_with("token") || k.contains("askpass"))
            {
                format!("{k}=<redacted>")
            } else {
                a.clone()
            }
        })
        .collect()
}

#[cfg(test)]
mod streaming_tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn parses_progress_lines() {
        let p = Progress::parse("Receiving objects:  45% (10/22), 1.2 MiB | 3 MiB/s").unwrap();
        assert_eq!(p.phase, "Receiving objects");
        assert_eq!(p.percent, Some(45));
        let p = Progress::parse("remote: Counting objects: 100% (4/4), done.").unwrap();
        assert_eq!(p.phase, "remote: Counting objects");
        assert_eq!(p.percent, Some(100));
        assert!(Progress::parse("From /tmp/bare.git").is_none());
        assert!(Progress::parse(" * [new branch]      main -> origin/main").is_none());
        assert_eq!(
            Progress::parse("remote: Enumerating objects: 4, done.").map(|p| p.percent),
            Some(None)
        );
    }

    #[test]
    fn streaming_delivers_stderr_lines_and_succeeds() {
        let sink = MemorySink::new();
        let runner = GitRunner::new(sink.clone());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s2 = seen.clone();
        // `git --version` prints nothing on stderr; a bad option prints usage there.
        let out = runner.run_streaming(None, ["--version"], None, move |l| {
            s2.lock().unwrap().push(l.to_string());
        });
        assert!(out.is_ok());
        assert!(seen.lock().unwrap().is_empty());
        let err = runner.run_streaming(None, ["--definitely-not-an-option"], None, move |l| {
            seen.lock().unwrap().push(l.to_string());
        });
        assert!(matches!(err, Err(GitError::Failed { .. })));
    }

    #[test]
    fn cancel_kills_the_process() {
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("git");
        std::fs::write(&fake, "#!/bin/sh\nsleep 10\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let runner = GitRunner::default().with_git(&fake);
        let cancel = CancelToken::new();
        let c2 = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            c2.cancel();
        });
        let started = Instant::now();
        let r = runner.run_streaming(None, ["fetch"], Some(cancel), |_| {});
        assert!(matches!(r, Err(GitError::Cancelled(_))), "{r:?}");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn argv_redaction_hides_secrets() {
        let argv = vec![
            "git".to_string(),
            "-c".into(),
            "http.extraHeader=x".into(),
            "-c".into(),
            "credential.password=abc".into(),
        ];
        let r = redact(&argv);
        assert_eq!(r[4], "credential.password=<redacted>");
        assert_eq!(r[2], "http.extraHeader=x");
    }
}
