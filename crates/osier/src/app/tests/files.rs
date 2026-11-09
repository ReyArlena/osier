//! The Files tab: the tree, ignored files, folder state and the viewer.

use super::*;

/// Hover the element with test id `id` until its tooltip shows; the tooltip's text.
fn hover_text(h: &Harness, cx: &mut TestAppContext, id: &str, tooltip: &'static str) -> bool {
    let at = h
        .bounds(cx, id)
        .unwrap_or_else(|| panic!("no {id}"))
        .center();
    h.visual(cx)
        .simulate_mouse_move(at, None, gpui::Modifiers::none());
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(2));
    cx.run_until_parked();
    h.bounds(cx, tooltip).is_some()
}

#[gpui::test]
fn ui_files_folder_marker_follows_the_sidebar_style_and_explains_itself(cx: &mut TestAppContext) {
    use osier_config::settings::ChangesIndicator;
    let repo = TempRepo::new();
    repo.commit_file("dir/sub/a.txt", "a\n", "first");
    repo.commit_file("other/c.txt", "c\n", "second");
    repo.write("dir/sub/a.txt", "changed\n");
    repo.write("dir/b.txt", "new\n");
    let h = Harness::open(cx, &repo);
    h.click(cx, "tab-files");
    // Icon style (the default): the folder icon carries the marker.
    assert!(h.bounds(cx, "files-dirty:dir").is_some());
    assert!(
        h.bounds(cx, "files-dirty:other").is_none(),
        "clean folder unmarked"
    );
    assert!(hover_text(
        &h,
        cx,
        "files-dirty:dir",
        "files-change-tooltip"
    ));
    h.read(cx, |o, _| {
        assert_eq!(
            o.settings.appearance.changes_indicator,
            ChangesIndicator::Icon
        )
    });
    // Dot style: a dot in the sidebar's colour, same tooltip.
    h.act(cx, |o, _, cx| {
        o.settings.appearance.changes_indicator = ChangesIndicator::Dot;
        cx.notify();
    });
    h.visual(cx).simulate_mouse_move(
        gpui::point(gpui::px(1.), gpui::px(1.)),
        None,
        gpui::Modifiers::none(),
    );
    cx.run_until_parked();
    assert!(hover_text(
        &h,
        cx,
        "files-dirty:dir",
        "files-change-tooltip"
    ));
    let count = h.read(cx, |o, _| {
        let s = o.active_project().unwrap().snapshot.as_ref().unwrap();
        s.status
            .entries
            .iter()
            .filter(|e| e.path.starts_with("dir/"))
            .count()
    });
    assert_eq!(count, 2, "two changed files inside dir");
}

/// The Files tree's cursor (its highlighted row) and the file the viewer shows.
fn files_cursor(h: &Harness, cx: &mut TestAppContext) -> (Option<String>, Option<String>) {
    h.read(cx, |o, cx| {
        let cursor = o
            .files_tree
            .read(cx)
            .selected_item()
            .map(|i| i.id.to_string());
        let viewed = match &o.active_project().unwrap().selection {
            Selection::WorkFile { path, rev: None } => Some(path.clone()),
            _ => None,
        };
        (cursor, viewed)
    })
}

#[gpui::test]
fn ui_files_tab_keeps_its_row_when_the_tree_changes(cx: &mut TestAppContext) {
    use crate::app::row_actions::RowKind;
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    for f in ["one.tmp", "two.tmp", "three.tmp"] {
        repo.write(f, "x\n");
    }
    let h = Harness::open(cx, &repo);
    h.act(cx, |o, _, cx| o.set_tab(Tab::Files, cx));
    let at = |f: &str| (Some(f.to_string()), Some(f.to_string()));
    h.click(cx, "files-row:three.tmp");
    assert_eq!(files_cursor(&h, cx), at("three.tmp"));

    // A file appearing elsewhere leaves the row alone.
    repo.write("new.txt", "n\n");
    h.act(cx, |o, _, cx| o.refresh_project(0, cx));
    assert_eq!(files_cursor(&h, cx), at("three.tmp"));

    // Ignoring the selected file moves to the next row (files are listed by name: one,
    // three, two), and from the last row back to the one before it.
    h.act(cx, |o, w, cx| {
        o.run_row_action(RowKind::AddToGitignore, w, cx)
    });
    assert_eq!(files_cursor(&h, cx), at("two.tmp"));
    h.act(cx, |o, w, cx| {
        o.run_row_action(RowKind::AddToGitignore, w, cx)
    });
    assert_eq!(files_cursor(&h, cx), at("one.tmp"));
    h.keys(cx, "down");
    assert_eq!(
        files_cursor(&h, cx).0.as_deref(),
        Some(".gitignore"),
        "wraps to the top"
    );
    h.keys(cx, "up");
    assert_eq!(files_cursor(&h, cx).0.as_deref(), Some("one.tmp"));
}

#[gpui::test]
fn ui_files_tab_reads_ignored_files_once_not_per_frame(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file(".gitignore", "*.log\n", "first");
    repo.commit_file("a.txt", "a\n", "second");
    repo.write("debug.log", "x\n");
    let h = Harness::open(cx, &repo);
    h.act(cx, |o, _, cx| o.set_tab(Tab::Files, cx));
    let listings = |h: &Harness, cx: &mut TestAppContext| {
        h.read(cx, |o, _| {
            o.sink
                .records()
                .iter()
                .filter(|r| r.argv.iter().any(|a| a == "--ignored"))
                .count()
        })
    };
    assert!(h.bounds(cx, "files-row:debug.log").is_none());
    h.click(cx, "show-ignored");
    assert!(
        h.bounds(cx, "files-row:debug.log").is_some(),
        "listed once loaded"
    );
    let after_toggle = listings(&h, cx);
    assert_eq!(after_toggle, 1);
    // Redraws (hovering rows) reuse the list.
    for i in 0..6 {
        let row = ["files-row:a.txt", "files-row:debug.log"][i % 2];
        let at = h.bounds(cx, row).unwrap().center();
        h.visual(cx)
            .simulate_mouse_move(at, None, gpui::Modifiers::none());
        cx.run_until_parked();
    }
    assert_eq!(listings(&h, cx), after_toggle);
    h.click(cx, "show-ignored");
    assert!(h.bounds(cx, "files-row:debug.log").is_none());
}

#[gpui::test]
fn ui_files_tab_shows_why_a_file_is_ignored(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file(".gitignore", "/build\n*.log\n", "first");
    repo.write("debug.log", "x\n");
    repo.write("build/out.o", "x\n");
    let h = Harness::open(cx, &repo);
    h.act(cx, |o, _, cx| o.set_tab(Tab::Files, cx));
    h.click(cx, "show-ignored");
    assert!(h.bounds(cx, "files-ignored:debug.log").is_some());
    assert!(
        h.bounds(cx, "files-ignored:build").is_some(),
        "wholly ignored folder"
    );
    assert!(h.bounds(cx, "files-ignored:.gitignore").is_none());
    // Hovering the badge explains the rule.
    let at = h.bounds(cx, "files-ignored:debug.log").unwrap().center();
    h.visual(cx)
        .simulate_mouse_move(at, None, gpui::Modifiers::none());
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(2000));
    cx.run_until_parked();
    assert!(
        h.bounds(cx, "files-ignored-tooltip").is_some(),
        "tooltip shown"
    );
}

#[gpui::test]
fn ui_files_tab_viewer_follows_the_arrow_keys(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("src/lib.rs", "l\n", "first");
    repo.commit_file("a.txt", "a\n", "second");
    repo.commit_file("b.txt", "b\n", "third");
    let h = Harness::open(cx, &repo);
    h.act(cx, |o, _, cx| o.set_tab(Tab::Files, cx));
    // Folders first: src, src/lib.rs, a.txt, b.txt.
    h.click(cx, "files-row:a.txt");
    let at = |f: &str| (Some(f.to_string()), Some(f.to_string()));
    h.keys(cx, "down");
    assert_eq!(files_cursor(&h, cx), at("b.txt"));
    h.keys(cx, "up up");
    assert_eq!(files_cursor(&h, cx), at("src/lib.rs"));
    // A folder row leaves the viewer on the last file.
    h.keys(cx, "up");
    assert_eq!(
        files_cursor(&h, cx),
        (Some("src".to_string()), Some("src/lib.rs".to_string()))
    );
}

#[gpui::test]
fn ui_files_tab_keeps_open_folders_open(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    // Four top-level folders: all start closed.
    for f in ["docs/d.md", "src/lib.rs", "tests/t.rs", "web/w.js"] {
        repo.commit_file(f, "x\n", f);
    }
    let h = Harness::open(cx, &repo);
    h.act(cx, |o, _, cx| o.set_tab(Tab::Files, cx));
    let shown = |h: &Harness, cx: &mut TestAppContext, id: &str| {
        h.bounds(cx, &format!("files-row:{id}")).is_some()
    };
    assert!(!shown(&h, cx, "src/lib.rs"));
    h.click(cx, "files-row:src");
    assert!(shown(&h, cx, "src/lib.rs"));

    // A new file rebuilds the tree: src stays open, the others stay closed.
    repo.write("new.txt", "n\n");
    h.act(cx, |o, _, cx| o.refresh_project(0, cx));
    assert!(shown(&h, cx, "new.txt"));
    assert!(shown(&h, cx, "src/lib.rs"));
    assert!(!shown(&h, cx, "docs/d.md"));

    // Filtering opens every folder; clearing it brings the old layout back.
    let set_filter = |h: &Harness, cx: &mut TestAppContext, text: &'static str| {
        h.act(cx, move |o, w, cx| {
            o.files_filter.update(cx, |s, cx| s.set_value(text, w, cx))
        });
    };
    set_filter(&h, cx, ".");
    assert!(shown(&h, cx, "docs/d.md"));
    set_filter(&h, cx, "");
    assert!(shown(&h, cx, "src/lib.rs"));
    assert!(!shown(&h, cx, "docs/d.md"));

    // A folder that leaves the tree for a while (eg a build folder being rebuilt, or
    // ignored files hidden) comes back the way it was.
    repo.write("gen/out.txt", "g\n");
    h.act(cx, |o, _, cx| o.refresh_project(0, cx));
    h.click(cx, "files-row:gen");
    assert!(shown(&h, cx, "gen/out.txt"));
    std::fs::remove_dir_all(repo.path().join("gen")).unwrap();
    h.act(cx, |o, _, cx| o.refresh_project(0, cx));
    assert!(!shown(&h, cx, "gen"));
    repo.write("gen/out.txt", "g\n");
    h.act(cx, |o, _, cx| o.refresh_project(0, cx));
    assert!(shown(&h, cx, "gen/out.txt"), "gen came back closed");
}

#[gpui::test]
fn ui_files_tab_with_ignored_shown_stays_on_an_ignored_file(cx: &mut TestAppContext) {
    use crate::app::row_actions::RowKind;
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    repo.write("notes.tmp", "x\n");
    repo.write("z.tmp", "x\n");
    let h = Harness::open(cx, &repo);
    h.act(cx, |o, _, cx| o.set_tab(Tab::Files, cx));
    h.click(cx, "show-ignored");
    h.click(cx, "files-row:notes.tmp");
    h.act(cx, |o, w, cx| {
        o.run_row_action(RowKind::AddToGitignore, w, cx)
    });
    let still = Some("notes.tmp".to_string());
    assert_eq!(files_cursor(&h, cx), (still.clone(), still));
}

#[gpui::test]
fn markdown_mode_is_remembered_per_file_type(cx: &mut TestAppContext) {
    use osier_config::settings::MarkdownMode;
    let repo = TempRepo::new();
    repo.commit_file("README.md", "# Title\n", "first");
    repo.commit_file("notes.md", "# Notes\n", "second");
    repo.commit_file("page.mdx", "# Page\n", "third");
    let h = Harness::open(cx, &repo);
    h.act(cx, |osier, _, cx| {
        osier.select_work_file("README.md".into(), None, cx)
    });
    h.read(cx, |osier, _| {
        assert!(!osier.markdown_as_code, "rendered by default")
    });
    h.act(cx, |osier, _, cx| osier.toggle_markdown_code(cx));
    h.read(cx, |osier, _| {
        assert!(osier.markdown_as_code);
        assert_eq!(
            osier.settings.viewer.markdown_modes.get("md"),
            Some(&MarkdownMode::Code)
        );
    });
    // Another .md opens as code; .mdx keeps the default.
    h.act(cx, |osier, _, cx| {
        osier.select_work_file("notes.md".into(), None, cx)
    });
    h.read(cx, |osier, _| assert!(osier.markdown_as_code));
    h.act(cx, |osier, _, cx| {
        osier.select_work_file("page.mdx".into(), None, cx)
    });
    h.read(cx, |osier, _| assert!(!osier.markdown_as_code));
    // Toggling back to the default drops the override instead of storing it.
    h.act(cx, |osier, _, cx| {
        osier.select_work_file("notes.md".into(), None, cx)
    });
    h.act(cx, |osier, _, cx| osier.toggle_markdown_code(cx));
    h.read(cx, |osier, _| {
        assert!(osier.settings.viewer.markdown_modes.is_empty())
    });
}

#[gpui::test]
fn files_tab_viewer_picks_markdown_code_and_binary(cx: &mut TestAppContext) {
    use crate::viewer::Viewer;
    let repo = TempRepo::new();
    repo.commit_file("README.md", "# Title\n\nhello\n", "docs");
    repo.commit_file("src/main.rs", "fn main() {}\n", "code");
    repo.write("blob.bin", "ab\0cd");
    let h = Harness::open(cx, &repo);
    h.read(cx, |osier, _| {
        let files = &osier
            .active_project()
            .unwrap()
            .snapshot
            .as_ref()
            .unwrap()
            .files;
        assert_eq!(files.as_slice(), &["README.md", "blob.bin", "src/main.rs"]);
    });
    h.act(cx, |osier, _, cx| {
        osier.select_work_file("README.md".into(), None, cx)
    });
    h.read(cx, |osier, _| {
        assert!(matches!(osier.viewer, Some(Viewer::Markdown { .. })))
    });
    h.act(cx, |osier, _, cx| {
        osier.select_work_file("src/main.rs".into(), None, cx)
    });
    h.read(cx, |osier, _| {
        assert!(matches!(
            &osier.viewer,
            Some(Viewer::Code {
                language: "rust",
                ..
            })
        ));
    });
    h.act(cx, |osier, _, cx| {
        osier.select_work_file("blob.bin".into(), None, cx)
    });
    h.read(cx, |osier, _| {
        assert!(matches!(osier.viewer, Some(Viewer::Binary { .. })))
    });
    // At a revision: the committed README is served from the object store.
    h.act(cx, |osier, _, cx| {
        osier.select_work_file("README.md".into(), Some("HEAD".into()), cx)
    });
    h.read(cx, |osier, _| {
        assert!(matches!(osier.viewer, Some(Viewer::Markdown { .. })))
    });
    h.act(cx, |osier, _, cx| {
        osier.select_work_file("nope.txt".into(), Some("HEAD".into()), cx)
    });
    h.read(cx, |osier, _| {
        assert!(matches!(osier.viewer, Some(Viewer::Missing)))
    });
}
