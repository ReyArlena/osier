//! Launching other programs: editor, terminal, file manager.

use std::path::Path;
use std::process::{Command, Stdio};

use gpui_kit::*;

use crate::app::Osier;
use crate::notices::{Notice, WindowNoticeExt as _};

/// An editor Osier knows how to start at a file and line.
pub struct KnownEditor {
    pub name: &'static str,
    /// Command names it is installed under, most common first (distributions rename them:
    /// Zed is `zeditor` on Arch).
    pub bins: &'static [&'static str],
    /// Flatpak application IDs (`flatpak run <id>`).
    pub flatpaks: &'static [&'static str],
    /// JetBrains Toolbox script name (`~/.local/share/JetBrains/Toolbox/scripts/<name>`).
    pub toolbox: Option<&'static str>,
    /// Arguments after the program: `{file}`, `{line}`, `{path}`.
    pub args: &'static str,
}

const fn editor(
    name: &'static str,
    bins: &'static [&'static str],
    flatpaks: &'static [&'static str],
    toolbox: Option<&'static str>,
    args: &'static str,
) -> KnownEditor {
    KnownEditor {
        name,
        bins,
        flatpaks,
        toolbox,
        args,
    }
}

const JETBRAINS_ARGS: &str = "--line {line} {file}";

pub const KNOWN_EDITORS: &[KnownEditor] = &[
    editor(
        "Zed",
        &["zed", "zeditor", "zedit"],
        &["dev.zed.Zed"],
        None,
        "{file}:{line}",
    ),
    editor(
        "VS Code",
        &["code", "code-oss", "visual-studio-code"],
        &["com.visualstudio.code"],
        None,
        "--goto {file}:{line}",
    ),
    editor(
        "VSCodium",
        &["codium", "vscodium"],
        &["com.vscodium.codium"],
        None,
        "--goto {file}:{line}",
    ),
    editor("Cursor", &["cursor"], &[], None, "--goto {file}:{line}"),
    editor("Windsurf", &["windsurf"], &[], None, "--goto {file}:{line}"),
    editor(
        "Sublime Text",
        &["subl", "sublime_text"],
        &["com.sublimetext.three"],
        None,
        "{file}:{line}",
    ),
    editor(
        "PhpStorm",
        &["phpstorm", "phpstorm.sh"],
        &["com.jetbrains.PhpStorm"],
        Some("phpstorm"),
        JETBRAINS_ARGS,
    ),
    editor(
        "IntelliJ IDEA",
        &[
            "idea",
            "idea.sh",
            "intellij-idea-ultimate-edition",
            "intellij-idea-community-edition",
        ],
        &[
            "com.jetbrains.IntelliJ-IDEA-Ultimate",
            "com.jetbrains.IntelliJ-IDEA-Community",
        ],
        Some("idea"),
        JETBRAINS_ARGS,
    ),
    editor(
        "WebStorm",
        &["webstorm", "webstorm.sh"],
        &["com.jetbrains.WebStorm"],
        Some("webstorm"),
        JETBRAINS_ARGS,
    ),
    editor(
        "PyCharm",
        &[
            "pycharm",
            "pycharm.sh",
            "pycharm-professional",
            "pycharm-community",
        ],
        &[
            "com.jetbrains.PyCharm-Professional",
            "com.jetbrains.PyCharm-Community",
        ],
        Some("pycharm"),
        JETBRAINS_ARGS,
    ),
    editor(
        "RustRover",
        &["rustrover", "rustrover.sh"],
        &["com.jetbrains.RustRover"],
        Some("rustrover"),
        JETBRAINS_ARGS,
    ),
    editor(
        "GoLand",
        &["goland", "goland.sh"],
        &["com.jetbrains.GoLand"],
        Some("goland"),
        JETBRAINS_ARGS,
    ),
    editor(
        "CLion",
        &["clion", "clion.sh"],
        &["com.jetbrains.CLion"],
        Some("clion"),
        JETBRAINS_ARGS,
    ),
    editor(
        "Kate",
        &["kate"],
        &["org.kde.kate"],
        None,
        "--line {line} {file}",
    ),
    editor(
        "gedit",
        &["gedit"],
        &["org.gnome.gedit"],
        None,
        "+{line} {file}",
    ),
    editor(
        "GNOME Text Editor",
        &["gnome-text-editor"],
        &["org.gnome.TextEditor"],
        None,
        "{file}",
    ),
];

/// Where to look besides PATH, so tests can fake an install.
pub(crate) struct EditorProbe<'a> {
    pub on_path: &'a dyn Fn(&str) -> bool,
    /// Whether a Flatpak app with this ID is installed.
    pub flatpak: &'a dyn Fn(&str) -> bool,
    /// The JetBrains Toolbox script with this name, if there is one.
    pub toolbox: &'a dyn Fn(&str) -> Option<std::path::PathBuf>,
}

/// The known editors that are installed, as (name, command template): a command on PATH
/// first, then a JetBrains Toolbox script, then a Flatpak app. Blocking (file checks).
pub(crate) fn find_editors(probe: &EditorProbe) -> Vec<(String, String)> {
    KNOWN_EDITORS
        .iter()
        .filter_map(|e| {
            let program = e
                .bins
                .iter()
                .find(|b| (probe.on_path)(b))
                .map(|b| b.to_string())
                .or_else(|| {
                    e.toolbox
                        .and_then(|t| (probe.toolbox)(t))
                        .map(|p| p.to_string_lossy().into_owned())
                })
                .or_else(|| {
                    e.flatpaks
                        .iter()
                        .find(|id| (probe.flatpak)(id))
                        .map(|id| format!("flatpak run {id}"))
                })?;
            Some((e.name.to_string(), format!("{program} {}", e.args)))
        })
        .collect()
}

/// The real places: PATH, the Flatpak exports (system and user), the Toolbox scripts folder.
fn find_editors_here() -> Vec<(String, String)> {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let flatpak_dirs: Vec<std::path::PathBuf> =
        std::iter::once(std::path::PathBuf::from("/var/lib/flatpak/exports/bin"))
            .chain(
                home.iter()
                    .map(|h| h.join(".local/share/flatpak/exports/bin")),
            )
            .collect();
    let toolbox_dir = home
        .as_ref()
        .map(|h| h.join(".local/share/JetBrains/Toolbox/scripts"));
    find_editors(&EditorProbe {
        on_path: &on_path,
        flatpak: &|id| flatpak_dirs.iter().any(|d| d.join(id).is_file()),
        toolbox: &|name| {
            toolbox_dir
                .as_ref()
                .map(|d| d.join(name))
                .filter(|p| p.is_file())
        },
    })
}

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

    /// Find the known editors (PATH, Toolbox scripts, Flatpak apps), once, in the
    /// background; settings shows them when they are in (`detected_editors`).
    pub(crate) fn detect_editors(&mut self, cx: &mut Context<Self>) {
        if self.detected_editors.is_some() || self.detecting_editors {
            return;
        }
        self.detecting_editors = true;
        self.in_background_quiet(
            find_editors_here,
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

#[cfg(test)]
mod editor_tests {
    use super::{EditorProbe, find_editors};

    #[test]
    fn renamed_toolbox_and_flatpak_installs_are_found() {
        let found = find_editors(&EditorProbe {
            // Arch's name for Zed, and VS Code.
            on_path: &|b| matches!(b, "zeditor" | "code"),
            flatpak: &|id| id == "org.kde.kate",
            toolbox: &|name| {
                (name == "rustrover")
                    .then(|| "/home/x/.local/share/JetBrains/Toolbox/scripts/rustrover".into())
            },
        });
        let get = |name: &str| {
            found
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, t)| t.as_str())
        };
        assert_eq!(get("Zed"), Some("zeditor {file}:{line}"));
        assert_eq!(get("VS Code"), Some("code --goto {file}:{line}"));
        assert_eq!(
            get("RustRover"),
            Some("/home/x/.local/share/JetBrains/Toolbox/scripts/rustrover --line {line} {file}")
        );
        assert_eq!(
            get("Kate"),
            Some("flatpak run org.kde.kate --line {line} {file}")
        );
        assert_eq!(get("PhpStorm"), None);
    }
}
