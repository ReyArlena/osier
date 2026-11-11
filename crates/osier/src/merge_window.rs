//! `osier merge` and `osier diff`: the merge tool and the diff view in a window of their own,
//! with no repository open. git's `mergetool` and `difftool` run them and wait for the process
//! to exit, so each runs its own short-lived app (like the askpass window) instead of handing
//! the files to a running Osier. The exit code tells git whether the merge was done.

use std::path::{Path, PathBuf};

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::diff::{Diff, DiffFile, DiffMode, DiffState};
use gpui_kit::component::{
    ActiveTheme as _, Selectable as _, Sizable as _, StyledExt as _, h_flex, v_flex,
};
use gpui_kit::*;

use crate::merge_view::{MergeEvent, MergeHost, MergeInput, MergeView};
use crate::test_ids::TestId as _;

/// Read a file for merging or comparing: text only.
fn read_text(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if bytes.iter().take(8000).any(|b| *b == 0) {
        return Err(format!("{} is a binary file", path.display()));
    }
    String::from_utf8(bytes).map_err(|_| format!("{} is not UTF-8 text", path.display()))
}

/// A pane label. git's mergetool hands over temporary copies named `<name>_LOCAL_<pid>.<ext>`
/// and `<name>_REMOTE_<pid>.<ext>`; any other file is labelled with its path.
fn side_label(path: &Path) -> String {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if name.contains("_LOCAL_") {
        "Yours (local)".into()
    } else if name.contains("_REMOTE_") {
        "Theirs (remote)".into()
    } else {
        path.display().to_string()
    }
}

fn window_options(title: String, cx: &mut App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::centered(size(px(1400.), px(900.)), cx)),
        window_min_size: Some(size(px(800.), px(400.))),
        titlebar: Some(TitlebarOptions {
            title: Some(title.into()),
            ..Default::default()
        }),
        app_id: Some(crate::APP_ID.into()),
        ..Default::default()
    }
}

/// Start the app, apply the saved theme and open one window. The process exits from the
/// window's own handlers; closing the window exits with `closed_code`.
fn run_window<V: Render + 'static>(
    title: String,
    closed_code: i32,
    build: impl FnOnce(&mut Window, &mut Context<V>) -> V + 'static,
) {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            crate::themes::apply_saved(cx);
            let options = window_options(title, cx);
            let opened = gpui_kit::open_window(options, cx, move |window, cx| {
                cx.new(|cx| build(window, cx))
            });
            let _window = match opened {
                Ok((window, _)) => window,
                Err(error) => {
                    eprintln!("osier: cannot open a window: {error}");
                    std::process::exit(2);
                }
            };
            #[cfg(feature = "profiler")]
            crate::profiling::start(_window, cx);
            cx.on_window_closed(move |cx, _| {
                if cx.windows().is_empty() {
                    std::process::exit(closed_code);
                }
            })
            .detach();
            cx.activate(true);
        });
}

/// The merge tool window around a [`MergeView`].
struct MergeWindow {
    view: Entity<MergeView>,
    _events: Subscription,
}

impl Render for MergeWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.view.clone())
    }
}

/// Write the result and leave: 0 when written, 2 when the file could not be written.
fn finish(output: &Path, text: &str) -> ! {
    match std::fs::write(output, text) {
        Ok(()) => std::process::exit(0),
        Err(e) => {
            eprintln!("osier: cannot write {}: {e}", output.display());
            std::process::exit(2);
        }
    }
}

/// `osier merge`. Exit code for the process (only returned on errors before the window).
pub fn run_merge(left: PathBuf, base: Option<PathBuf>, right: PathBuf, output: PathBuf) -> i32 {
    let read = |p: &Path| match read_text(p) {
        Ok(t) => Ok(t),
        Err(e) => {
            eprintln!("osier: {e}");
            Err(2)
        }
    };
    let (left_text, right_text) = match (read(&left), read(&right)) {
        (Ok(l), Ok(r)) => (l, r),
        _ => return 2,
    };
    let base_text = match &base {
        // git creates an empty base file when the versions share no ancestor.
        Some(p) if p.exists() => match read(p) {
            Ok(t) => Some(t),
            Err(code) => return code,
        },
        _ => None,
    };
    let input = MergeInput {
        path: output.display().to_string(),
        base: base_text,
        left: left_text,
        right: right_text,
        left_label: side_label(&left),
        right_label: side_label(&right),
    };
    let name = output
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    run_window(format!("Merge {name} - Osier"), 1, move |window, cx| {
        let view = cx.new(|cx| MergeView::new(input, MergeHost::Window, window, cx));
        // Focus the result once the window exists (nothing is focused during construction),
        // so typing and ctrl-enter reach it.
        let editor = view.read(cx).result_editor().clone();
        window.defer(cx, move |window, cx| {
            let handle = editor.read(cx).focus_handle(cx);
            handle.focus(window, cx);
        });
        let output = output.clone();
        let events = cx.subscribe(&view, move |_, _, ev: &MergeEvent, _| match ev {
            MergeEvent::Apply(text) | MergeEvent::Save(text) => finish(&output, text),
            MergeEvent::Cancel => std::process::exit(1),
            _ => {}
        });
        MergeWindow {
            view,
            _events: events,
        }
    });
    0
}

/// The read-only diff window of `osier diff`.
struct DiffWindow {
    state: Entity<DiffState>,
    title: String,
    identical: bool,
}

impl DiffWindow {
    fn set_mode(&mut self, mode: DiffMode, cx: &mut Context<Self>) {
        self.state.update(cx, |s, cx| s.set_mode(mode, cx));
        cx.notify();
    }
}

impl Render for DiffWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mode = self.state.read(cx).mode();
        let muted = cx.theme().muted_foreground;
        let body: AnyElement = if self.identical {
            v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .child(div().font_medium().child("No differences"))
                .child(
                    div()
                        .text_sm()
                        .text_color(muted)
                        .child("The two files have the same content."),
                )
                .into_any_element()
        } else {
            Diff::new(&self.state).line_number(true).into_any_element()
        };
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                h_flex()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .text_sm()
                            .font_medium()
                            .truncate()
                            .min_w_0()
                            .child(self.title.clone()),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("unified")
                            .test_id("unified")
                            .ghost()
                            .xsmall()
                            .label("Unified")
                            .selected(mode == DiffMode::Unified)
                            .on_click(
                                cx.listener(|this, _, _, cx| this.set_mode(DiffMode::Unified, cx)),
                            ),
                    )
                    .child(
                        Button::new("split")
                            .test_id("split")
                            .ghost()
                            .xsmall()
                            .label("Split")
                            .selected(mode == DiffMode::Split)
                            .on_click(
                                cx.listener(|this, _, _, cx| this.set_mode(DiffMode::Split, cx)),
                            ),
                    )
                    .child(
                        Button::new("diff-close")
                            .test_id("diff-close")
                            .outline()
                            .xsmall()
                            .label("Close")
                            .on_click(|_, _, _| std::process::exit(0)),
                    ),
            )
            .child(div().flex_1().min_h_0().child(body))
    }
}

/// The patch the diff window shows, and whether the files are the same.
fn diff_files(
    left: &Path,
    right: &Path,
    left_text: &str,
    right_text: &str,
) -> (Vec<DiffFile>, bool) {
    let name = |p: &Path| {
        p.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| p.display().to_string())
    };
    let patch =
        osier_git::merge3::unified_patch(&name(left), &name(right), left_text, right_text, 3);
    if patch.is_empty() {
        return (Vec::new(), true);
    }
    (DiffFile::parse(&patch).unwrap_or_default(), false)
}

/// `osier diff`. Exit code for the process (only returned on errors before the window).
pub fn run_diff(left: PathBuf, right: PathBuf) -> i32 {
    let (left_text, right_text) = match (read_text(&left), read_text(&right)) {
        (Ok(l), Ok(r)) => (l, r),
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("osier: {e}");
            return 2;
        }
    };
    let (files, identical) = diff_files(&left, &right, &left_text, &right_text);
    let title = format!("{} → {}", left.display(), right.display());
    run_window("Diff - Osier".to_string(), 0, move |_, cx| {
        let state = cx.new(|cx| {
            let mut s = DiffState::new(files, cx);
            s.set_mode(DiffMode::Split, cx);
            s
        });
        DiffWindow {
            state,
            title,
            identical,
        }
    });
    0
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{read_text, side_label};

    #[test]
    fn labels_from_git_temp_names() {
        assert_eq!(
            side_label(Path::new("./app_LOCAL_1234.json")),
            "Yours (local)"
        );
        assert_eq!(
            side_label(Path::new("./app_REMOTE_1234.json")),
            "Theirs (remote)"
        );
        assert_eq!(side_label(Path::new("/x/a.txt")), "/x/a.txt");
    }

    #[test]
    fn refuses_binary_files() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("b");
        std::fs::write(&bin, [1u8, 0, 2]).unwrap();
        assert!(read_text(&bin).unwrap_err().contains("binary"));
        let txt = dir.path().join("t");
        std::fs::write(&txt, "hi\n").unwrap();
        assert_eq!(read_text(&txt).unwrap(), "hi\n");
        assert!(read_text(&dir.path().join("missing")).is_err());
    }
}
