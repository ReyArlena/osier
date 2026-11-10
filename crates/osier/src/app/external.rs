//! Launching other programs: editor, terminal, file manager.

use std::path::Path;
use std::process::{Command, Stdio};

use gpui_kit::*;

use crate::app::Osier;
use crate::notices::{Notice, WindowNoticeExt as _};

/// Editors we can detect on PATH, with the command template that opens a file at a line.
pub const KNOWN_EDITORS: &[(&str, &str, &str)] = &[
    ("zed", "Zed", "zed {file}:{line}"),
    ("code", "VS Code", "code --goto {file}:{line}"),
    ("codium", "VSCodium", "codium --goto {file}:{line}"),
    ("phpstorm", "PhpStorm", "phpstorm --line {line} {file}"),
    ("idea", "IntelliJ IDEA", "idea --line {line} {file}"),
    ("subl", "Sublime Text", "subl {file}:{line}"),
    ("gedit", "gedit", "gedit {file}"),
    ("kate", "Kate", "kate --line {line} {file}"),
];

pub fn on_path(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(bin).is_file()))
        .unwrap_or(false)
}

/// Split a command template on whitespace and substitute placeholders.
fn expand(template: &str, path: &Path, file: Option<(&Path, Option<u32>)>) -> Vec<String> {
    let line = file.and_then(|(_, l)| l).unwrap_or(1).to_string();
    let file_s = file
        .map(|(f, _)| f.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    template
        .split_whitespace()
        .map(|part| {
            part.replace("{path}", &path.to_string_lossy())
                .replace("{file}", &file_s)
                .replace("{line}", &line)
        })
        .collect()
}

/// Start a program detached from Osier.
///
/// - Under a systemd user session it runs through `systemd-run --user --scope`, so it gets its
///   own scope (cgroup): quitting Osier, or the out-of-memory killer stopping Osier's scope,
///   leaves it alone. Without systemd it is started directly.
/// - Its exit is collected on a background thread, so a finished program does not linger as a
///   zombie process for as long as Osier runs.
fn spawn(argv: &[String], cwd: &Path) -> std::io::Result<()> {
    let argv = launch_argv(argv, systemd_user_session());
    let (program, args) = argv
        .split_first()
        .ok_or_else(|| std::io::Error::other("empty command"))?;
    let mut child = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    std::thread::Builder::new()
        .name("osier-launch".into())
        .spawn(move || {
            let _ = child.wait();
        })?;
    Ok(())
}

/// `argv`, wrapped in `systemd-run --user --scope` when `systemd` says a user manager is there.
fn launch_argv(argv: &[String], systemd: bool) -> Vec<String> {
    if !systemd || argv.is_empty() {
        return argv.to_vec();
    }
    [
        "systemd-run",
        "--user",
        "--scope",
        "--collect",
        "--quiet",
        "--",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain(argv.iter().cloned())
    .collect()
}

/// A systemd user manager we can ask for a scope: `systemd-run` on PATH and the manager's
/// socket present. Not inside Flatpak, where the sandbox has its own rules.
fn systemd_user_session() -> bool {
    if Path::new("/.flatpak-info").exists() || !on_path("systemd-run") {
        return false;
    }
    std::env::var_os("XDG_RUNTIME_DIR")
        .is_some_and(|dir| Path::new(&dir).join("systemd/private").exists())
}

impl Osier {
    /// Open the project (or one file at a line) in the configured editor, else `xdg-open`.
    pub(crate) fn open_in_editor(
        &mut self,
        file: Option<(String, Option<u32>)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.active_project() else {
            return;
        };
        let root = project.path.clone();
        let file_path = file.as_ref().map(|(f, l)| (root.join(f), *l));
        let argv = match &self.settings.editor.command {
            Some(template) if !template.trim().is_empty() => expand(
                template,
                &root,
                file_path.as_ref().map(|(p, l)| (p.as_path(), *l)),
            ),
            _ => {
                // No editor configured: let the desktop decide.
                let target = file_path
                    .as_ref()
                    .map(|(p, _)| p.clone())
                    .unwrap_or_else(|| root.clone());
                vec!["xdg-open".into(), target.to_string_lossy().into_owned()]
            }
        };
        match spawn(&argv, &root) {
            Ok(()) => {}
            Err(e) => window.push_notice(
                Notice::error(format!("{}: {e}", argv.join(" "))).title("Could not open editor"),
                cx,
            ),
        }
    }

    pub(crate) fn open_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.active_project() else {
            return;
        };
        let root = project.path.clone();
        let argv: Vec<String> = match &self.settings.editor.terminal {
            Some(t) if !t.trim().is_empty() => expand(t, &root, None),
            _ => match std::env::var("TERMINAL") {
                Ok(t) if !t.is_empty() => vec![t],
                _ => [
                    "ghostty",
                    "kitty",
                    "alacritty",
                    "foot",
                    "wezterm",
                    "konsole",
                    "gnome-terminal",
                    "xterm",
                ]
                .iter()
                .find(|b| on_path(b))
                .map(|b| vec![b.to_string()])
                .unwrap_or_default(),
            },
        };
        if argv.is_empty() {
            window.push_notice(
                Notice::warning("Set editor.terminal in config.toml or export TERMINAL.")
                    .title("No terminal found"),
                cx,
            );
            return;
        }
        if let Err(e) = spawn(&argv, &root) {
            window.push_notice(
                Notice::error(e.to_string()).title("Could not open terminal"),
                cx,
            );
        }
    }

    pub(crate) fn reveal_in_file_manager(
        &mut self,
        path: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.active_project() else {
            return;
        };
        let root = project.path.clone();
        let target = match path {
            Some(p) => {
                let full = root.join(&p);
                if full.is_dir() {
                    full
                } else {
                    full.parent().map(Path::to_path_buf).unwrap_or(root.clone())
                }
            }
            None => root.clone(),
        };
        let argv = vec![
            "xdg-open".to_string(),
            target.to_string_lossy().into_owned(),
        ];
        if let Err(e) = spawn(&argv, &root) {
            window.push_notice(
                Notice::error(e.to_string()).title("Could not open file manager"),
                cx,
            );
        }
    }

    /// Open any path with the desktop default handler.
    pub(crate) fn open_externally(
        &mut self,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let argv = vec!["xdg-open".to_string(), path.to_string_lossy().into_owned()];
        let cwd = path.parent().unwrap_or(path);
        if let Err(e) = spawn(&argv, cwd) {
            window.push_notice(Notice::error(e.to_string()).title("Could not open"), cx);
        }
    }

    /// Editors found on PATH, for the settings panel.
    #[allow(dead_code)]
    /// Find the known editors on PATH (a stat per PATH folder and editor), once, in the
    /// background; settings shows them when they are in (`detected_editors`).
    pub(crate) fn detect_editors(&mut self, cx: &mut Context<Self>) {
        if self.detected_editors.is_some() || self.detecting_editors {
            return;
        }
        self.detecting_editors = true;
        self.in_background_quiet(
            || {
                KNOWN_EDITORS
                    .iter()
                    .filter(|(bin, _, _)| on_path(bin))
                    .map(|(_, name, template)| (*name, *template))
                    .collect::<Vec<_>>()
            },
            |this, found, cx| {
                this.detecting_editors = false;
                this.detected_editors = Some(found);
                this.redraw_settings(cx);
            },
            cx,
        );
    }
}

#[cfg(test)]
mod tests {
    // No glob import: `gpui_kit::*` exports a `test` attribute that would shadow `#[test]`.
    use std::path::Path;

    use super::expand;

    #[test]
    fn expands_placeholders() {
        let argv = expand(
            "zed {file}:{line}",
            Path::new("/r"),
            Some((Path::new("/r/a.rs"), Some(7))),
        );
        assert_eq!(argv, ["zed", "/r/a.rs:7"]);
        let argv = expand("code {path}", Path::new("/r"), None);
        assert_eq!(argv, ["code", "/r"]);
        let argv = expand(
            "kate --line {line} {file}",
            Path::new("/r"),
            Some((Path::new("/r/x"), None)),
        );
        assert_eq!(argv, ["kate", "--line", "1", "/r/x"]);
    }
}

#[cfg(test)]
mod launch_tests {
    use super::launch_argv;

    #[test]
    fn systemd_wraps_the_command_in_its_own_scope() {
        let argv = vec!["zed".to_string(), "/x/a.rs:3".to_string()];
        assert_eq!(launch_argv(&argv, false), argv);
        assert_eq!(
            launch_argv(&argv, true),
            [
                "systemd-run",
                "--user",
                "--scope",
                "--collect",
                "--quiet",
                "--",
                "zed",
                "/x/a.rs:3"
            ]
        );
        assert!(launch_argv(&[], true).is_empty());
    }
}
