//! One Osier per user: `osier <path>` hands the path to a running instance, which opens it
//! and comes to the front, instead of starting a second app. `--new-instance` skips this.
//!
//! The running instance listens on a Unix socket in `$XDG_RUNTIME_DIR/osier/` (a directory
//! only the user can reach). The socket name is keyed by the state directory, so instances
//! run with their own `OSIER_STATE_DIR` (tests, demos) never hand off to the user's app.
//! Requests are one line: `open <absolute path>` or `raise`; the answer is `ok`.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

/// What a second `osier` asked the running one to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Request {
    /// Open (or switch to) this repository and come to the front.
    Open(PathBuf),
    /// Just come to the front.
    Raise,
}

static INBOX: Mutex<Option<Receiver<Request>>> = Mutex::new(None);

/// The socket for this user and state directory; `None` without `XDG_RUNTIME_DIR`.
pub(crate) fn socket_path(state_dir: &Path) -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty())?;
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    state_dir.hash(&mut h);
    Some(
        PathBuf::from(runtime)
            .join("osier")
            .join(format!("instance-{:016x}.sock", h.finish())),
    )
}

/// Hand `path` (or a raise) to a running instance. True when one took it.
pub(crate) fn hand_off(socket: &Path, path: Option<&Path>) -> bool {
    let Ok(mut stream) = UnixStream::connect(socket) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let line = match path {
        Some(p) => format!("open {}\n", p.display()),
        None => "raise\n".to_string(),
    };
    if stream.write_all(line.as_bytes()).is_err() {
        return false;
    }
    let mut answer = String::new();
    BufReader::new(stream).read_line(&mut answer).is_ok() && answer.trim() == "ok"
}

/// Listen for other `osier` invocations. A stale socket (no one answering) is replaced; a
/// live one (another instance started with `--new-instance`) is left alone.
pub(crate) fn serve(socket: &Path) -> std::io::Result<()> {
    if let Some(dir) = socket.parent() {
        std::fs::create_dir_all(dir)?;
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    if socket.exists() {
        if UnixStream::connect(socket).is_ok() {
            return Ok(());
        }
        std::fs::remove_file(socket)?;
    }
    let listener = UnixListener::bind(socket)?;
    let (tx, rx) = channel();
    if let Ok(mut inbox) = INBOX.lock() {
        *inbox = Some(rx);
    }
    std::thread::Builder::new()
        .name("osier-instance".into())
        .spawn(move || accept(listener, tx))?;
    Ok(())
}

fn accept(listener: UnixListener, tx: Sender<Request>) {
    for stream in listener.incoming().flatten() {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
        let mut reader = BufReader::new(&stream);
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() {
            continue;
        }
        let request = match parse(&line) {
            Some(r) => r,
            None => continue,
        };
        if tx.send(request).is_err() {
            return;
        }
        let _ = (&stream).write_all(b"ok\n");
    }
}

fn parse(line: &str) -> Option<Request> {
    let line = line.trim_end_matches(['\n', '\r']);
    if line == "raise" {
        return Some(Request::Raise);
    }
    let path = line.strip_prefix("open ")?;
    let path = PathBuf::from(path);
    path.is_absolute().then_some(Request::Open(path))
}

/// Requests that arrived since the last call (the UI's tick loop drains them).
pub(crate) fn take_requests() -> Vec<Request> {
    match INBOX.lock() {
        Ok(inbox) => inbox
            .as_ref()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::{Request, accept, hand_off, parse};
    use std::os::unix::net::UnixListener;

    #[test]
    fn parses_requests_and_refuses_relative_paths() {
        assert_eq!(parse("raise\n"), Some(Request::Raise));
        assert_eq!(
            parse("open /home/x/repo\n"),
            Some(Request::Open("/home/x/repo".into()))
        );
        assert_eq!(parse("open relative/repo\n"), None);
        assert_eq!(parse("rm -rf /\n"), None);
    }

    #[test]
    fn a_second_invocation_hands_its_path_to_the_first() {
        let dir = tempfile::tempdir().expect("tempdir");
        let socket = dir.path().join("instance.sock");
        assert!(!hand_off(&socket, None), "nobody listening yet");
        let listener = UnixListener::bind(&socket).expect("bind");
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || accept(listener, tx));
        assert!(hand_off(
            &socket,
            Some(std::path::Path::new("/tmp/some/repo"))
        ));
        assert!(hand_off(&socket, None));
        let got: Vec<Request> = rx.try_iter().collect();
        assert_eq!(
            got,
            [Request::Open("/tmp/some/repo".into()), Request::Raise]
        );
    }
}
