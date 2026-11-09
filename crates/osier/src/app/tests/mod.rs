//! Headless UI tests: the `Harness` opens the app in a test window and drives it
//! through the same methods the buttons call, or through real clicks and keystrokes.
//!
//! The tests live in one child module per area and reach the harness and the shared
//! helpers below through `use super::*`. Keep this module free of glob imports:
//! `gpui_kit::*` exports a `gpui` module (ambiguous with the `gpui` crate the
//! `#[gpui::test]` macro's paths resolve to, published by `gpui-pre`) and a `test`
//! attribute macro that would shadow `#[test]`.

use std::sync::Arc;

use super::{Osier, TOOLTIP_DELAY_MAX_MS, Tab, init};
use crate::command_log::UiCommandSink;
use crate::project::{Area, Selection};
use gpui::TestAppContext;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, WindowOptions};
use osier_config::{Paths, Settings, State};
use osier_git::GitRunner;
use osier_git::testing::TempRepo;

mod changes;
mod files;
mod keys;
mod log;
mod merge;
mod new_repo;
mod panes;
mod patches;
mod projects;
mod recent_status;
mod settings;
mod sync;
mod updates;

fn isolated(sink: Arc<UiCommandSink>) -> GitRunner {
    GitRunner::new(sink)
        .with_env("GIT_CONFIG_GLOBAL", "/dev/null")
        .with_env("GIT_CONFIG_NOSYSTEM", "1")
        .with_env("HOME", std::env::temp_dir())
}

struct Harness {
    _dirs: tempfile::TempDir,
    window: AnyWindowHandle,
    view: Entity<Osier>,
}

impl Harness {
    fn open(cx: &mut TestAppContext, repo: &TempRepo) -> Self {
        Self::open_with(cx, State::default(), Some(repo.path().to_path_buf()))
    }

    /// Start like a later session: `state` as saved, `open` as the command-line path.
    fn open_with(cx: &mut TestAppContext, state: State, open: Option<std::path::PathBuf>) -> Self {
        cx.update(gpui_kit::init);
        cx.update(init);
        let dirs = tempfile::tempdir().expect("tempdir");
        let paths = Paths::under(dirs.path());
        let sink = Arc::new(UiCommandSink::default());
        let runner = isolated(sink.clone());
        let (handle, view) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    Osier::new(
                        paths,
                        Settings::default(),
                        state,
                        runner,
                        sink,
                        open,
                        None,
                        window,
                        cx,
                    )
                })
            })
            .expect("window")
        });
        cx.run_until_parked();
        Self {
            _dirs: dirs,
            window: handle,
            view,
        }
    }

    /// Run `f` with the window, like a click handler would.
    fn act(
        &self,
        cx: &mut TestAppContext,
        f: impl FnOnce(&mut Osier, &mut gpui::Window, &mut gpui::Context<Osier>),
    ) {
        let view = self.view.clone();
        cx.update_window(self.window, |_, window, cx| {
            view.update(cx, |osier, cx| f(osier, window, cx));
        })
        .expect("window alive");
        cx.run_until_parked();
    }

    fn read<R>(&self, cx: &mut TestAppContext, f: impl FnOnce(&Osier, &gpui::App) -> R) -> R {
        cx.update(|cx| f(self.view.read(cx), cx))
    }

    // ----- simulated input (see `test_ids.rs` for the id convention) -----

    fn visual(&self, cx: &TestAppContext) -> gpui::VisualTestContext {
        gpui::VisualTestContext::from_window(self.window, cx)
    }

    /// Bounds of the element with test id `id` in the last rendered frame.
    fn bounds(&self, cx: &mut TestAppContext, id: &str) -> Option<gpui::Bounds<gpui::Pixels>> {
        // gpui keys the map by `&'static str`; leaking a few test strings is fine.
        let id: &'static str = Box::leak(id.to_string().into_boxed_str());
        self.visual(cx).debug_bounds(id)
    }

    /// Click the centre of the element with test id `id`, as a user would.
    fn click(&self, cx: &mut TestAppContext, id: &str) {
        self.click_with(cx, id, gpui::Modifiers::none());
    }

    fn click_with(&self, cx: &mut TestAppContext, id: &str, modifiers: gpui::Modifiers) {
        let bounds = self
            .bounds(cx, id)
            .unwrap_or_else(|| panic!("no element with test id {id:?} in the last frame"));
        self.visual(cx).simulate_click(bounds.center(), modifiers);
        cx.run_until_parked();
    }

    /// Right-click the centre of the element with test id `id` (opens context menus).
    fn right_click(&self, cx: &mut TestAppContext, id: &str) {
        let bounds = self
            .bounds(cx, id)
            .unwrap_or_else(|| panic!("no element with test id {id:?} in the last frame"));
        let mut v = self.visual(cx);
        let at = bounds.center();
        v.simulate_mouse_move(at, None, gpui::Modifiers::none());
        v.simulate_mouse_down(at, gpui::MouseButton::Right, gpui::Modifiers::none());
        v.simulate_mouse_up(at, gpui::MouseButton::Right, gpui::Modifiers::none());
        cx.run_until_parked();
    }

    /// Keystrokes in gpui notation, space separated: `"ctrl-, escape"`.
    fn keys(&self, cx: &mut TestAppContext, keys: &str) {
        cx.simulate_keystrokes(self.window, keys);
        cx.run_until_parked();
    }

    /// Text typed into whatever has focus.
    fn type_text(&self, cx: &mut TestAppContext, text: &str) {
        cx.simulate_input(self.window, text);
        cx.run_until_parked();
    }

    /// Whether a popup menu (a dropdown or a context menu) has the keyboard.
    fn menu_open(&self, cx: &mut TestAppContext) -> bool {
        cx.update_window(self.window, |_, window, _| {
            window
                .context_stack()
                .iter()
                .any(|context| context.contains("PopupMenu"))
        })
        .expect("window")
    }

    fn has_settings(&self, cx: &mut TestAppContext) -> bool {
        use gpui_kit::component::WindowExt as _;
        cx.update_window(self.window, |_, window, cx| window.has_active_dialog(cx))
            .expect("window")
    }
}

/// A repository with an upstream, `ahead` local commits and optionally a dirty tree.
fn repo_with_upstream(ahead: usize, dirty: bool) -> TempRepo {
    let t = TempRepo::new();
    t.commit_file("a.txt", "a\n", "first");
    t.add_bare_origin();
    t.git(["push", "-q", "-u", "origin", "HEAD"]);
    for n in 0..ahead {
        t.commit_file("a.txt", &format!("{n}\n"), &format!("local {n}"));
    }
    if dirty {
        t.write("new.txt", "x\n");
    }
    t
}

fn set_recent_mode(
    h: &Harness,
    cx: &mut TestAppContext,
    mode: osier_config::settings::RecentStatusMode,
) {
    h.act(cx, |osier, _, cx| {
        osier.settings.projects.recent_status = mode;
        osier.apply_recent_status_mode(cx);
    });
}

/// Run the tick-loop step until the queue is empty (the tick loop's timer does not run in
/// tests).
fn drain_recent(h: &Harness, cx: &mut TestAppContext) {
    for _ in 0..100 {
        h.act(cx, |osier, _, cx| osier.pump_recent_status(cx));
        cx.run_until_parked();
        let idle = h.read(cx, |o, _| {
            o.recent_status_queue_len() == 0 && !o.recent_status_busy()
        });
        if idle {
            h.act(cx, |osier, _, cx| osier.pump_recent_status(cx));
            return;
        }
    }
    panic!("recent status queue did not drain");
}

fn file(path: &str, area: Area) -> Selection {
    Selection::File {
        path: path.into(),
        area,
    }
}

fn input_text(
    h: &Harness,
    cx: &mut TestAppContext,
    pick: fn(&Osier) -> &Entity<gpui_kit::component::input::InputState>,
) -> String {
    h.read(cx, |o, cx| pick(o).read(cx).value().to_string())
}

fn focused_is(
    h: &Harness,
    cx: &mut TestAppContext,
    pick: fn(&Osier, &gpui::App) -> gpui::FocusHandle,
) -> bool {
    let view = h.view.clone();
    cx.update_window(h.window, |_, window, cx| {
        let handle = pick(view.read(cx), cx);
        handle.is_focused(window)
    })
    .expect("window")
}

/// Two open repositories, the second one active. Returns the harness and both names.
fn two_projects(cx: &mut TestAppContext) -> (Harness, TempRepo, TempRepo, String, String) {
    let a = TempRepo::new();
    a.commit_file("a.txt", "a\n", "first");
    let b = TempRepo::new();
    b.commit_file("b.txt", "b\n", "first");
    let h = Harness::open(cx, &a);
    let path = b.path().to_path_buf();
    h.act(cx, |o, w, cx| o.open_project_notify(&path, w, cx));
    let names = h.read(cx, |o, _| {
        assert_eq!(o.active, Some(1));
        (o.projects[0].name.clone(), o.projects[1].name.clone())
    });
    (h, a, b, names.0, names.1)
}

fn active_name(h: &Harness, cx: &mut TestAppContext) -> String {
    h.read(cx, |o, _| {
        o.active_project()
            .map(|p| p.name.clone())
            .unwrap_or_default()
    })
}

fn tab(h: &Harness, cx: &mut TestAppContext) -> Tab {
    h.read(cx, |o, _| o.tab)
}
