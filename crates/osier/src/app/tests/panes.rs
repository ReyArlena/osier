//! Tabs, the right pane per tab, cached diff views, the editor, the command log panel and the
//! diff toolbar.

use super::*;

#[gpui::test]
fn ui_command_log_rows_copy_from_the_context_menu(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    let h = Harness::open(cx, &repo);
    h.act(cx, |osier, _, cx| {
        if !osier.command_log_visible {
            osier.toggle_command_log(cx);
        }
    });
    let rec = h.read(cx, |o, _| {
        o.sink.records().last().cloned().expect("a record")
    });
    let row = format!("command-row:{}", rec.id);

    h.right_click(cx, &row);
    h.keys(cx, "down enter");
    let copied = cx.read_from_clipboard().and_then(|c| c.text());
    assert_eq!(copied.as_deref(), Some(rec.as_shell().as_str()));

    h.right_click(cx, &row);
    h.keys(cx, "down down enter");
    let copied = cx
        .read_from_clipboard()
        .and_then(|c| c.text())
        .unwrap_or_default();
    assert_eq!(copied, crate::views::command_log::command_details(&rec));
    assert!(copied.starts_with("$ git "), "{copied}");
    assert!(copied.contains("\nexit: "), "{copied}");
    assert!(copied.contains("\nstdout: "), "{copied}");
}

#[gpui::test]
fn ui_tabs_and_command_palette(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "one\n", "first");
    let h = Harness::open(cx, &repo);
    h.click(cx, "tab-log");
    h.read(cx, |osier, _| assert_eq!(osier.tab, Tab::Log));
    h.keys(cx, "f1");
    h.type_text(cx, "Patches tab");
    h.keys(cx, "enter");
    h.read(cx, |osier, _| assert_eq!(osier.tab, Tab::Patches));
    h.keys(cx, "ctrl-1");
    h.read(cx, |osier, _| assert_eq!(osier.tab, Tab::Changes));
    h.keys(cx, "ctrl-p");
    h.type_text(cx, "Log tab");
    h.keys(cx, "enter");
    h.read(cx, |osier, _| assert_eq!(osier.tab, Tab::Log));
}

#[gpui::test]
fn center_pane_keeps_its_saved_width_when_the_window_resizes(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    let mut state = State::default();
    state.layout.sidebar_visible = true;
    state.layout.center_width = Some(320.);
    let h = Harness::open_with(cx, state, Some(repo.path().to_path_buf()));
    let width = |h: &Harness, cx: &mut TestAppContext| {
        f32::from(h.bounds(cx, "center-pane").expect("center pane").size.width)
    };
    assert!(
        (width(&h, cx) - 320.).abs() < 2.,
        "restored: {}",
        width(&h, cx)
    );
    for size in [
        gpui::size(gpui::px(1600.), gpui::px(900.)),
        gpui::size(gpui::px(1100.), gpui::px(700.)),
    ] {
        h.visual(cx).simulate_resize(size);
        cx.run_until_parked();
        assert!(
            (width(&h, cx) - 320.).abs() < 2.,
            "after resize: {}",
            width(&h, cx)
        );
    }
}

/// Press at `from`, move to `to` in two steps and release, as dragging an edge does.
fn drag(
    h: &Harness,
    cx: &mut TestAppContext,
    from: gpui::Point<gpui::Pixels>,
    to: gpui::Point<gpui::Pixels>,
) {
    let (left, none) = (gpui::MouseButton::Left, gpui::Modifiers::none());
    let mut v = h.visual(cx);
    v.simulate_mouse_move(from, None, none);
    v.simulate_mouse_down(from, left, none);
    let half = gpui::point((from.x + to.x) / 2., (from.y + to.y) / 2.);
    v.simulate_mouse_move(half, Some(left), none);
    v.simulate_mouse_move(to, Some(left), none);
    v.simulate_mouse_up(to, left, none);
    cx.run_until_parked();
}

#[gpui::test]
fn ui_sidebar_edge_drags_to_resize_and_the_width_is_kept(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    let mut state = State::default();
    state.layout.sidebar_visible = true;
    let h = Harness::open_with(cx, state, Some(repo.path().to_path_buf()));
    h.visual(cx)
        .simulate_resize(gpui::size(gpui::px(1400.), gpui::px(900.)));
    cx.run_until_parked();
    let pane =
        |h: &Harness, cx: &mut TestAppContext| h.bounds(cx, "sidebar-pane").expect("sidebar");
    let width = |h: &Harness, cx: &mut TestAppContext| f32::from(pane(h, cx).size.width);
    assert!(
        (width(&h, cx) - 240.).abs() < 1.,
        "default: {}",
        width(&h, cx)
    );

    let edge = |h: &Harness, cx: &mut TestAppContext| {
        let b = pane(h, cx);
        gpui::point(b.origin.x + b.size.width - gpui::px(2.), b.center().y)
    };
    let from = edge(&h, cx);
    drag(&h, cx, from, gpui::point(gpui::px(330.), from.y));
    assert!(
        (width(&h, cx) - 330.).abs() < 3.,
        "dragged wider: {}",
        width(&h, cx)
    );
    h.read(cx, |o, _| {
        let saved = o.state.layout.sidebar_width.expect("width saved");
        assert!((saved - 330.).abs() < 3., "saved: {saved}");
    });

    // Never narrower than its smallest size, never past half the window.
    let from = edge(&h, cx);
    drag(&h, cx, from, gpui::point(gpui::px(20.), from.y));
    assert!(
        (width(&h, cx) - 160.).abs() < 1.,
        "smallest: {}",
        width(&h, cx)
    );
    let from = edge(&h, cx);
    drag(&h, cx, from, gpui::point(gpui::px(1300.), from.y));
    assert!(
        (width(&h, cx) - 600.).abs() < 1.,
        "largest: {}",
        width(&h, cx)
    );
}

#[gpui::test]
fn ui_sidebar_width_is_restored(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    let mut state = State::default();
    state.layout.sidebar_visible = true;
    state.layout.sidebar_width = Some(300.);
    let h = Harness::open_with(cx, state, Some(repo.path().to_path_buf()));
    let width = f32::from(h.bounds(cx, "sidebar-pane").expect("sidebar").size.width);
    assert!((width - 300.).abs() < 1., "restored: {width}");
}

#[gpui::test]
fn ui_bottom_panel_edge_drags_to_resize_and_the_height_is_kept(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    let mut state = State::default();
    state.layout.command_log_visible = true;
    let h = Harness::open_with(cx, state, Some(repo.path().to_path_buf()));
    h.visual(cx)
        .simulate_resize(gpui::size(gpui::px(1400.), gpui::px(1000.)));
    cx.run_until_parked();
    let panel =
        |h: &Harness, cx: &mut TestAppContext| h.bounds(cx, "bottom-panel").expect("bottom panel");
    let height = |h: &Harness, cx: &mut TestAppContext| f32::from(panel(h, cx).size.height);
    assert!(
        (height(&h, cx) - 220.).abs() < 1.,
        "default: {}",
        height(&h, cx)
    );

    let edge = |h: &Harness, cx: &mut TestAppContext| {
        let b = panel(h, cx);
        gpui::point(b.center().x, b.origin.y + gpui::px(2.))
    };
    let from = edge(&h, cx);
    let bottom = panel(&h, cx).origin.y + panel(&h, cx).size.height;
    drag(&h, cx, from, gpui::point(from.x, bottom - gpui::px(400.)));
    assert!(
        (height(&h, cx) - 400.).abs() < 3.,
        "dragged taller: {}",
        height(&h, cx)
    );
    h.read(cx, |o, _| {
        let saved = o.state.layout.command_log_height.expect("height saved");
        assert!((saved - 400.).abs() < 3., "saved: {saved}");
    });
    assert!(
        h.bounds(cx, "close-log").is_some(),
        "the panel's own buttons are still there"
    );

    // Never shorter than its smallest size, never more than 60% of the window.
    let from = edge(&h, cx);
    drag(&h, cx, from, gpui::point(from.x, bottom - gpui::px(10.)));
    assert!(
        (height(&h, cx) - 100.).abs() < 1.,
        "smallest: {}",
        height(&h, cx)
    );
    let from = edge(&h, cx);
    drag(&h, cx, from, gpui::point(from.x, gpui::px(5.)));
    assert!(
        (height(&h, cx) - 600.).abs() < 1.,
        "largest: {}",
        height(&h, cx)
    );
}

fn selection(h: &Harness, cx: &mut TestAppContext) -> Selection {
    h.read(cx, |o, _| o.active_project().unwrap().selection.clone())
}

#[gpui::test]
fn ui_each_tab_keeps_its_own_right_pane(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    repo.commit_file("b.txt", "b\n", "second");
    repo.write("a.txt", "changed\n");
    repo.write("b.txt", "changed too\n");
    let h = Harness::open(cx, &repo);
    assert_eq!(h.read(cx, |o, _| o.tab), Tab::Changes);
    // Changes starts on its first change; pick the second.
    h.click(cx, "change-row:changes:b.txt");
    let b = Selection::File {
        path: "b.txt".into(),
        area: Area::Changed,
    };
    assert_eq!(selection(&h, cx), b);
    // Log starts on the latest commit, not on Changes' file.
    h.click(cx, "tab-log");
    let latest = h.read(cx, |o, _| {
        o.active_project()
            .unwrap()
            .snapshot
            .as_ref()
            .unwrap()
            .commits[0]
            .hash
            .clone()
    });
    assert_eq!(selection(&h, cx), Selection::Commit { hash: latest });
    h.click(cx, "log-row:1");
    let older = h.read(cx, |o, _| {
        o.active_project()
            .unwrap()
            .commit_at(1)
            .unwrap()
            .hash
            .clone()
    });
    // Tabs with nothing picked show nothing.
    h.click(cx, "tab-stashes");
    assert_eq!(selection(&h, cx), Selection::None);
    // Each tab comes back as it was left.
    h.click(cx, "tab-changes");
    assert_eq!(selection(&h, cx), b);
    h.click(cx, "tab-log");
    assert_eq!(
        selection(&h, cx),
        Selection::Commit {
            hash: older.clone()
        }
    );
    h.read(cx, |o, _| {
        let p = o.active_project().unwrap();
        assert_eq!(p.selected_commit, Some(1));
        assert_eq!(p.picked, [older]);
    });
}

#[gpui::test]
fn an_open_editor_survives_switching_tabs(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    repo.write("a.txt", "changed\n");
    let h = Harness::open(cx, &repo);
    h.act(cx, |o, w, cx| o.open_editor(w, cx));
    h.act(cx, |o, w, cx| {
        let editor = o.editor.as_ref().expect("editor open");
        editor
            .state
            .update(cx, |s, cx| s.set_value("typed but not saved\n", w, cx));
    });
    h.click(cx, "tab-log");
    h.read(cx, |o, _| {
        assert!(o.editor.is_none(), "Log shows its commit")
    });
    h.click(cx, "tab-changes");
    h.read(cx, |o, cx| {
        let editor = o.editor.as_ref().expect("editor back");
        assert_eq!(
            editor.state.read(cx).value().to_string(),
            "typed but not saved\n"
        );
    });
}

#[gpui::test]
fn an_open_editor_comes_back_with_its_project(cx: &mut TestAppContext) {
    let (h, a, b, _first, _second) = two_projects(cx);
    a.write("a.txt", "a changed\n");
    b.write("b.txt", "b changed\n");
    h.act(cx, |o, _, cx| {
        o.refresh_project(0, cx);
        o.refresh_project(1, cx);
    });
    h.act(cx, |o, w, cx| {
        o.activate(0, w, cx);
        o.set_tab(Tab::Changes, cx);
    });
    h.act(cx, |o, w, cx| o.open_editor(w, cx));
    h.read(cx, |o, _| {
        assert_eq!(o.editor.as_ref().unwrap().path, "a.txt")
    });
    h.act(cx, |o, w, cx| o.activate(1, w, cx));
    h.read(cx, |o, _| {
        assert!(o.editor.is_none(), "not on the other project")
    });
    h.act(cx, |o, w, cx| o.activate(0, w, cx));
    h.read(cx, |o, _| {
        assert_eq!(o.editor.as_ref().map(|e| e.path.as_str()), Some("a.txt"))
    });
}

#[gpui::test]
fn a_tab_gets_its_own_diff_view_back_unless_the_content_moved(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    repo.commit_file("b.txt", "b\n", "second");
    repo.write("a.txt", "changed\n");
    let h = Harness::open(cx, &repo);
    let view = |h: &Harness, cx: &mut TestAppContext| h.read(cx, |o, _| o.diff.entity_id());
    let changes_view = view(&h, cx);
    h.click(cx, "tab-log");
    let log_view = view(&h, cx);
    assert_ne!(log_view, changes_view, "each tab has its own view");
    // Back and forth with nothing changed: the same views, scroll and all.
    h.click(cx, "tab-changes");
    assert_eq!(view(&h, cx), changes_view);
    assert_eq!(h.read(cx, |o, _| o.diff_files), 1);
    h.click(cx, "tab-log");
    assert_eq!(view(&h, cx), log_view);
    // The working tree moves while Changes is away: its diff is loaded fresh.
    repo.write("a.txt", "changed again\n");
    h.act(cx, |o, _, cx| o.refresh_project(0, cx));
    assert_eq!(view(&h, cx), log_view, "a fixed commit stays as it was");
    h.click(cx, "tab-changes");
    assert_ne!(view(&h, cx), changes_view, "reloaded");
    assert_eq!(h.read(cx, |o, _| o.diff_files), 1);
}

#[gpui::test]
fn the_shown_diff_follows_edits_to_its_file_and_ignores_other_files(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    repo.commit_file("b.txt", "b\n", "second");
    repo.write("a.txt", "one\n");
    repo.write("b.txt", "other\n");
    let h = Harness::open(cx, &repo);
    h.click(cx, "change-row:changes:a.txt");
    let loads = |h: &Harness, cx: &mut TestAppContext| h.read(cx, |o, _| o.diff_revision);
    let added = |h: &Harness, cx: &mut TestAppContext| {
        h.read(cx, |o, _| {
            o.diff_model.iter().map(|f| f.added()).sum::<usize>()
        })
    };
    assert_eq!(added(&h, cx), 1);
    // Same status (a.txt stays modified), new content: the diff follows.
    let before = loads(&h, cx);
    repo.write("a.txt", "one\ntwo\nthree\n");
    h.act(cx, |o, _, cx| o.refresh_project(0, cx));
    assert!(loads(&h, cx) > before, "reloaded");
    assert_eq!(added(&h, cx), 3);
    // Another file changes: the shown diff (and its scroll) is left alone.
    let before = loads(&h, cx);
    repo.write("b.txt", "other, edited\n");
    h.act(cx, |o, _, cx| o.refresh_project(0, cx));
    assert_eq!(loads(&h, cx), before, "not reloaded");
}

#[gpui::test]
fn switching_project_drops_the_cached_diff_views(cx: &mut TestAppContext) {
    let (h, _a, _b, _first, _second) = two_projects(cx);
    h.act(cx, |o, w, cx| o.activate(0, w, cx));
    h.click(cx, "tab-log");
    h.click(cx, "tab-branches");
    h.read(cx, |o, _| {
        let p = o.active_project().unwrap();
        assert!(p.panes.values().any(|pane| pane.diff.is_some()));
    });
    h.act(cx, |o, w, cx| o.activate(1, w, cx));
    h.read(cx, |o, _| {
        assert!(o.projects[0].panes.values().all(|pane| pane.diff.is_none()));
    });
}

#[gpui::test]
fn clicking_a_stash_shows_its_changes(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    repo.write("a.txt", "stashed\n");
    repo.git(["stash", "push", "-q", "-m", "wip"]);
    let h = Harness::open(cx, &repo);
    h.click(cx, "tab-stashes");
    assert_eq!(selection(&h, cx), Selection::None);
    let oid = h.read(cx, |o, _| {
        o.active_project()
            .unwrap()
            .snapshot
            .as_ref()
            .unwrap()
            .stashes[0]
            .oid
            .clone()
    });
    h.act(cx, |o, _, cx| o.select_stash(oid.clone(), cx));
    assert_eq!(
        selection(&h, cx),
        Selection::Compare {
            from: format!("{oid}^1"),
            to: oid,
            since_merge_base: false
        }
    );
    h.read(cx, |o, _| assert_eq!(o.diff_files, 1, "the stashed file"));
}

#[gpui::test]
fn tab_overflow_is_measured_and_settles(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "one\n", "first");
    let h = Harness::open(cx, &repo);
    // The test layout's centre panel is about 440px: six tabs do not fit, so the strip
    // scrolls and the menu shows. Redrawing settles (a render loop would never park).
    h.act(cx, |_, _, cx| cx.notify());
    h.act(cx, |_, _, cx| cx.notify());
    h.read(cx, |o, _| {
        assert!(o.tab_scroll.bounds().size.width > gpui::px(0.));
        assert!(o.tab_scroll.max_offset().x > gpui::px(0.5));
    });
}

#[gpui::test]
fn ui_diff_toolbar_walks_files_and_hides_whitespace_in_commits(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.write("a.txt", "one\n");
    repo.write("b.txt", "x  y\n");
    repo.write("c.txt", "three\n");
    repo.git(["add", "."]);
    repo.commit("init");
    repo.write("a.txt", "ONE\n");
    repo.write("b.txt", "x y\n");
    repo.write("c.txt", "THREE\n");
    repo.git(["add", "."]);
    repo.commit("change");
    let h = Harness::open(cx, &repo);
    h.act(cx, |o, _, cx| o.set_tab(Tab::Log, cx));
    h.act(cx, |o, _, cx| o.select_commit(0, cx));
    assert!(
        h.bounds(cx, "diff-next").is_some(),
        "toolbar on a two-way diff"
    );
    assert!(
        h.bounds(cx, "diff-prev-file").is_none(),
        "file arrows only while expanded"
    );
    assert_eq!(h.read(cx, |o, _| o.diff_files), 3);
    h.click(cx, "diff-expand");
    h.click(cx, "diff-next-file");
    assert_eq!(h.read(cx, |o, _| o.diff_file_ix), 1);
    h.click(cx, "diff-prev-file");
    h.click(cx, "diff-prev-file");
    assert_eq!(h.read(cx, |o, _| o.diff_file_ix), 2, "wraps");
    // Hiding whitespace drops the file that only changed its spacing.
    h.act(cx, |o, window, cx| {
        let mut opts = o.diff_options();
        opts.whitespace = osier_git::merge3::Whitespace::Ignore;
        o.set_diff_options(opts, window, cx)
    });
    assert_eq!(h.read(cx, |o, _| o.diff_files), 2);
    h.read(cx, |o, _| {
        assert_eq!(
            o.settings.diff.whitespace,
            osier_config::settings::DiffWhitespace::All
        )
    });
    h.click(cx, "diff-next");
    assert!(h.bounds(cx, "center-pane").is_none(), "still expanded");
}

#[gpui::test]
fn ui_changes_never_hide_whitespace_and_file_arrows_walk_the_list(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a  b\n", "init");
    repo.commit_file("z.txt", "z\n", "z");
    repo.write("a.txt", "a b\n");
    repo.write("z.txt", "Z\n");
    let h = Harness::open(cx, &repo);
    h.act(cx, |o, window, cx| {
        let mut opts = o.diff_options();
        opts.whitespace = osier_git::merge3::Whitespace::Ignore;
        o.set_diff_options(opts, window, cx)
    });
    h.act(cx, |o, _, cx| {
        o.select_file("a.txt".into(), Area::Changed, cx)
    });
    assert_eq!(
        h.read(cx, |o, _| o.diff_files),
        1,
        "stageable diffs stay exact"
    );
    h.click(cx, "diff-expand");
    h.click(cx, "diff-next-file");
    let selected = h.read(cx, |o, _| o.active_project().unwrap().selection.clone());
    assert_eq!(
        selected,
        Selection::File {
            path: "z.txt".into(),
            area: Area::Changed
        }
    );
}

#[gpui::test]
fn ui_notifications_panel_keeps_every_toast(cx: &mut TestAppContext) {
    use crate::app::BottomTab;
    use crate::notices::{Notice, NoticeLog, WindowNoticeExt as _};
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    let h = Harness::open(cx, &repo);
    // A failing command, so the error toast has a command to open the log on.
    let error = h.read(cx, |o, _| {
        o.runner
            .run(Some(repo.path()), ["rev-parse", "--verify", "nope"])
            .expect_err("rev-parse of a missing ref fails")
    });
    let record = error.record().map(|r| r.id);
    assert!(record.is_some(), "the error carries its command record");
    h.act(cx, |_, window, cx| {
        window.push_notice("Recents cleared", cx);
        window.push_notice(
            Notice::error("boom")
                .title("Could not pull")
                .command_log(&error),
            cx,
        );
    });
    h.read(cx, |o, cx| {
        assert_eq!(
            o.unread_notices(cx),
            2,
            "both count as unread until the panel shows them"
        );
        assert!(!o.command_log_visible);
    });
    // The error toast stays up and would cover the title bar's bell in this small window.
    h.act(cx, |_, window, cx| {
        use gpui_kit::component::WindowExt as _;
        window.clear_notifications(cx)
    });
    assert!(
        h.bounds(cx, "notifications").is_some(),
        "the title bar has the bell"
    );

    // The shortcut opens the panel on the Notifications tab and marks them seen.
    h.keys(cx, "ctrl-shift-n");
    h.read(cx, |o, cx| {
        assert!(o.command_log_visible);
        assert_eq!(o.bottom_tab, BottomTab::Notifications);
        assert_eq!(o.unread_notices(cx), 0);
    });
    assert!(h.bounds(cx, "notice-row:1").is_some());
    assert!(h.bounds(cx, "notice-row:2").is_some());
    assert!(
        h.bounds(cx, "notice-command-log:1").is_none(),
        "a plain notice has no command log button"
    );

    // "View command log" on the error switches to the commands tab, failures only, expanded.
    h.click(cx, "notice-command-log:2");
    h.read(cx, |o, _| {
        assert_eq!(o.bottom_tab, BottomTab::Commands);
        assert!(o.command_log_failures_only);
        assert_eq!(o.expanded_command, record);
    });
    assert!(
        h.bounds(cx, &format!("command-row:{}", record.unwrap_or(0)))
            .is_some()
    );

    // Back to notifications by its tab button; Clear empties the list.
    h.click(cx, "panel-notifications");
    h.read(cx, |o, _| {
        assert_eq!(o.bottom_tab, BottomTab::Notifications)
    });
    h.click(cx, "clear-notices");
    h.read(cx, |_, cx| assert!(NoticeLog::notices(cx).is_empty()));
    assert!(h.bounds(cx, "notice-row:1").is_none());

    // The shortcut on its own tab hides the panel; ctrl-` brings it back on commands.
    h.keys(cx, "ctrl-shift-n");
    h.read(cx, |o, _| assert!(!o.command_log_visible));
    h.keys(cx, "ctrl-`");
    h.read(cx, |o, _| {
        assert!(o.command_log_visible);
        assert_eq!(o.bottom_tab, BottomTab::Commands);
    });
    // The bell toggles the notifications tab too.
    h.click(cx, "notifications");
    h.read(cx, |o, _| {
        assert_eq!(o.bottom_tab, BottomTab::Notifications)
    });
    h.click(cx, "notifications");
    h.read(cx, |o, _| assert!(!o.command_log_visible));
}

#[gpui::test]
fn ui_palette_categories_show_only_when_asked(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "one\n", "first");
    let h = Harness::open(cx, &repo);
    h.keys(cx, "ctrl-p");
    h.type_text(cx, "Log tab");
    assert!(h.bounds(cx, "palette-command:Log tab").is_some());
    assert!(
        h.bounds(cx, "palette-category:Log tab").is_none(),
        "hidden by default"
    );
    h.keys(cx, "escape escape");
    h.act(cx, |o, _, _| o.settings.palette.show_categories = true);
    h.keys(cx, "ctrl-p");
    h.type_text(cx, "Log tab");
    assert!(h.bounds(cx, "palette-category:Log tab").is_some());
}
