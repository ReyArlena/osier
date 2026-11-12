//! The Changes tab: staging, the selection after staging, the commit box, discard, ignore and
//! untrack.

use super::*;

#[gpui::test]
fn ui_stage_with_the_checkbox_and_commit_from_the_keyboard(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "one\n", "first");
    repo.write("a.txt", "two\n");
    let h = Harness::open(cx, &repo);
    h.click(cx, "change-stage:changes:a.txt");
    assert_eq!(
        repo.git(["diff", "--cached", "--name-only"]).trim(),
        "a.txt"
    );
    assert!(
        h.bounds(cx, "change-row:changes:a.txt").is_some(),
        "the row moved to Staged"
    );
    h.click(cx, "commit-message");
    h.type_text(cx, "Typed in the commit box");
    h.keys(cx, "ctrl-enter");
    assert_eq!(
        repo.git(["log", "-1", "--format=%s"]).trim(),
        "Typed in the commit box"
    );
    assert!(repo.git(["status", "--porcelain"]).trim().is_empty());
}

#[gpui::test]
fn ui_list_keys_select_and_stage(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    repo.commit_file("b.txt", "b\n", "second");
    repo.write("a.txt", "A\n");
    repo.write("b.txt", "B\n");
    let h = Harness::open(cx, &repo);
    h.click(cx, "change-row:changes:a.txt");
    h.keys(cx, "down");
    h.read(cx, |osier, _| {
        assert_eq!(
            osier.active_project().unwrap().selection,
            Selection::File {
                path: "b.txt".into(),
                area: Area::Changed
            }
        )
    });
    h.keys(cx, "space");
    assert_eq!(
        repo.git(["diff", "--cached", "--name-only"]).trim(),
        "b.txt"
    );
    h.keys(cx, "j");
    h.keys(cx, "delete");
    assert!(
        cx.update_window(h.window, |_, window, cx| {
            use gpui_kit::component::WindowExt as _;
            window.has_active_dialog(cx)
        })
        .unwrap(),
        "delete asks first"
    );
    h.keys(cx, "escape");
    assert_eq!(repo.read("a.txt"), "A\n");
}

fn selected(h: &Harness, cx: &mut TestAppContext) -> Selection {
    h.read(cx, |o, _| o.active_project().unwrap().selection.clone())
}

/// Three changed files `a`, `b`, `c` in the Unstaged list.
fn three_unstaged() -> TempRepo {
    let repo = TempRepo::new();
    for f in ["a.txt", "b.txt", "c.txt"] {
        repo.commit_file(f, "x\n", f);
    }
    for f in ["a.txt", "b.txt", "c.txt"] {
        repo.write(f, "changed\n");
    }
    repo
}

fn check_of(h: &Harness, cx: &mut TestAppContext, path: &str) -> Option<osier_git::staging::Check> {
    h.read(cx, |o, _| o.change_check(path))
}

#[gpui::test]
fn ui_ticking_a_file_keeps_it_selected_in_the_one_list(cx: &mut TestAppContext) {
    use osier_git::staging::Check;
    let repo = three_unstaged();
    let h = Harness::open(cx, &repo);
    h.click(cx, "change-row:changes:b.txt");
    h.click(cx, "change-stage:changes:b.txt");
    // One list: the file stays where it is, ticked, and stays selected with its diff.
    assert_eq!(selected(&h, cx), file("b.txt", Area::Changed));
    assert_eq!(check_of(&h, cx, "b.txt"), Some(Check::Staged));
    assert_eq!(h.read(cx, |o, _| o.diff_files), 1);
    assert_eq!(
        repo.git(["diff", "--cached", "--name-only"]).trim(),
        "b.txt"
    );
    // A second click clears it.
    h.click(cx, "change-stage:changes:b.txt");
    assert_eq!(check_of(&h, cx, "b.txt"), Some(Check::Unstaged));
    assert_eq!(selected(&h, cx), file("b.txt", Area::Changed));
}

#[gpui::test]
fn ui_ticking_another_file_keeps_the_selection(cx: &mut TestAppContext) {
    use osier_git::staging::Check;
    let repo = three_unstaged();
    let h = Harness::open(cx, &repo);
    h.click(cx, "change-row:changes:a.txt");
    h.click(cx, "change-stage:changes:c.txt");
    assert_eq!(selected(&h, cx), file("a.txt", Area::Changed));
    h.act(cx, |o, w, cx| o.stage_all(w, cx));
    assert_eq!(selected(&h, cx), file("a.txt", Area::Changed));
    for f in ["a.txt", "b.txt", "c.txt"] {
        assert_eq!(check_of(&h, cx, f), Some(Check::Staged), "{f}");
    }
}

#[gpui::test]
fn ui_staging_a_file_open_in_the_editor_keeps_it(cx: &mut TestAppContext) {
    let repo = three_unstaged();
    let h = Harness::open(cx, &repo);
    h.click(cx, "change-row:changes:b.txt");
    h.act(cx, |o, w, cx| o.open_editor(w, cx));
    h.click(cx, "change-stage:changes:b.txt");
    assert_eq!(selected(&h, cx), file("b.txt", Area::Changed));
    assert!(h.read(cx, |o, _| o.editor.is_some()));
}

#[gpui::test]
fn ui_selection_stays_on_a_staged_file_and_moves_on_when_it_is_committed(cx: &mut TestAppContext) {
    let repo = three_unstaged();
    let h = Harness::open(cx, &repo);
    h.click(cx, "change-row:changes:b.txt");
    h.act(cx, |o, _, cx| o.set_tab(Tab::Log, cx));
    repo.git(["add", "b.txt"]);
    h.act(cx, |o, _, cx| o.refresh_active(cx));
    h.act(cx, |o, _, cx| o.set_tab(Tab::Changes, cx));
    assert_eq!(selected(&h, cx), file("b.txt", Area::Changed));
    // Committed, it leaves the list: the selection moves to the next file.
    repo.git(["commit", "-q", "-m", "b"]);
    h.act(cx, |o, _, cx| o.refresh_active(cx));
    assert_eq!(selected(&h, cx), file("c.txt", Area::Changed));
}

#[gpui::test]
fn ui_an_added_unversioned_file_is_followed_into_changes(cx: &mut TestAppContext) {
    use osier_git::staging::Check;
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    repo.write("new.txt", "new\n");
    let h = Harness::open(cx, &repo);
    h.click(cx, "change-row:unversioned:new.txt");
    h.click(cx, "change-stage:unversioned:new.txt");
    assert_eq!(selected(&h, cx), file("new.txt", Area::Changed));
    assert_eq!(check_of(&h, cx, "new.txt"), Some(Check::Staged));
    // Unticking a newly added file makes it unversioned again.
    h.click(cx, "change-stage:changes:new.txt");
    assert_eq!(selected(&h, cx), file("new.txt", Area::Untracked));
    assert!(h.bounds(cx, "change-row:unversioned:new.txt").is_some());
}

#[gpui::test]
fn ui_dropping_the_selected_stash_moves_to_the_next(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    for n in 0..3 {
        repo.write("a.txt", &format!("{n}\n"));
        repo.git(["stash", "-q"]);
    }
    let h = Harness::open(cx, &repo);
    h.act(cx, |o, _, cx| o.set_tab(Tab::Stashes, cx));
    let oids: Vec<String> = h.read(cx, |o, _| {
        let s = o.active_project().unwrap().snapshot.as_ref().unwrap();
        s.stashes.iter().map(|s| s.oid.clone()).collect()
    });
    h.act(cx, |o, _, cx| o.select_stash(oids[1].clone(), cx));
    repo.git(["stash", "drop", "-q", "stash@{1}"]);
    h.act(cx, |o, _, cx| o.refresh_active(cx));
    let to = |oid: &str| Selection::Compare {
        from: format!("{oid}^1"),
        to: oid.into(),
        since_merge_base: false,
    };
    assert_eq!(selected(&h, cx), to(&oids[2]));
    assert_eq!(
        h.read(cx, |o, _| o.stash_selected.clone()),
        Some(oids[2].clone())
    );
}

#[gpui::test]
fn ui_changes_tab_shows_the_number_of_changes(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    let clean = Harness::open(cx, &repo);
    assert!(
        clean.bounds(cx, "tab-changes-count").is_none(),
        "no badge when clean"
    );

    let dirty = TempRepo::new();
    dirty.commit_file("a.txt", "a\n", "first");
    dirty.write("a.txt", "b\n");
    dirty.write("new.txt", "x\n");
    let h = Harness::open(cx, &dirty);
    assert!(h.bounds(cx, "tab-changes-count").is_some());
    h.read(cx, |o, _| {
        let n = o
            .active_project()
            .and_then(|p| p.snapshot.as_ref())
            .map(|s| s.status.entries.len());
        assert_eq!(n, Some(2));
    });
}

fn status_has(h: &Harness, cx: &mut TestAppContext, line: &str) -> bool {
    h.read(cx, |o, _| {
        o.active_project()
            .and_then(|p| p.snapshot.as_ref())
            .is_some_and(|s| s.status.entries.iter().any(|e| e.path == line))
    })
}

#[gpui::test]
fn ignoring_an_untracked_file_takes_effect_at_once_and_twice_is_a_no_op(cx: &mut TestAppContext) {
    use crate::app::ignore_ops::{PatternKind, RowState};
    use osier_git::ops::ignore::IgnoreFile;
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    repo.write("debug.log", "x\n");
    let h = Harness::open(cx, &repo);
    assert!(status_has(&h, cx, "debug.log"));
    let row = RowState {
        is_dir: false,
        tracked: false,
    };
    h.act(cx, |o, w, cx| {
        o.ignore_path(
            "debug.log".into(),
            row,
            PatternKind::Exact,
            IgnoreFile::Gitignore,
            w,
            cx,
        )
    });
    let gitignore = || std::fs::read_to_string(repo.path().join(".gitignore")).unwrap_or_default();
    assert_eq!(gitignore(), "/debug.log\n");
    assert!(
        !status_has(&h, cx, "debug.log"),
        "gone from Changes straight away"
    );
    assert!(!h.has_settings(cx), "no dialog for an untracked file");

    // Again: already ignored, nothing written, no dialog.
    h.act(cx, |o, w, cx| {
        o.ignore_path(
            "debug.log".into(),
            row,
            PatternKind::Exact,
            IgnoreFile::Gitignore,
            w,
            cx,
        )
    });
    assert_eq!(gitignore(), "/debug.log\n");
    assert!(!h.has_settings(cx));
}

#[gpui::test]
fn ignoring_places_the_pattern_with_related_lines(cx: &mut TestAppContext) {
    use crate::app::ignore_ops::{PatternKind, RowState};
    use osier_git::ops::ignore::IgnoreFile;
    let repo = TempRepo::new();
    repo.commit_file(
        ".gitignore",
        "# local binaries\nlocal-bin/.local/bin/zed\nlocal-bin/.local/bin/btop\n\n# other\n*.log\n",
        "ignore",
    );
    repo.write("local-bin/.local/bin/new-app", "x\n");
    let h = Harness::open(cx, &repo);
    let row = RowState {
        is_dir: false,
        tracked: false,
    };
    h.act(cx, |o, w, cx| {
        o.ignore_path(
            "local-bin/.local/bin/new-app".into(),
            row,
            PatternKind::Exact,
            IgnoreFile::Gitignore,
            w,
            cx,
        )
    });
    let text = std::fs::read_to_string(repo.path().join(".gitignore")).unwrap();
    assert_eq!(
        text,
        "# local binaries\nlocal-bin/.local/bin/zed\nlocal-bin/.local/bin/btop\n\
         local-bin/.local/bin/new-app\n\n# other\n*.log\n"
    );
    assert!(!status_has(&h, cx, "local-bin/.local/bin/new-app"));
}

#[gpui::test]
fn ignoring_a_tracked_file_asks_and_untracks_by_default(cx: &mut TestAppContext) {
    use crate::app::ignore_ops::{PatternKind, RowState};
    use osier_git::ops::ignore::IgnoreFile;
    let repo = TempRepo::new();
    repo.commit_file("config/local.php", "secret\n", "tracked");
    repo.commit_file("keep.txt", "k\n", "tracked too");
    let h = Harness::open(cx, &repo);
    let row = RowState {
        is_dir: false,
        tracked: true,
    };
    let tracked = |path: &str| !repo.git(["ls-files", "--", path]).trim().is_empty();

    // Enter accepts with the checkbox on: ignored and untracked, the file stays.
    h.act(cx, |o, w, cx| {
        o.ignore_path(
            "config/local.php".into(),
            row,
            PatternKind::Exact,
            IgnoreFile::Gitignore,
            w,
            cx,
        )
    });
    assert!(h.has_settings(cx), "a dialog for a tracked file");
    assert!(h.bounds(cx, "tracked-dialog-check").is_some());
    h.keys(cx, "enter");
    assert!(!h.has_settings(cx));
    assert!(!tracked("config/local.php"));
    assert!(repo.path().join("config/local.php").exists());
    let gitignore = std::fs::read_to_string(repo.path().join(".gitignore")).unwrap();
    assert!(gitignore.contains("/config/local.php"));

    // Escape cancels.
    h.act(cx, |o, w, cx| {
        o.ignore_path(
            "keep.txt".into(),
            row,
            PatternKind::Exact,
            IgnoreFile::Gitignore,
            w,
            cx,
        )
    });
    assert!(h.has_settings(cx));
    h.keys(cx, "escape");
    assert!(!h.has_settings(cx));
    // The Cancel button does the same.
    h.act(cx, |o, w, cx| {
        o.ignore_path(
            "keep.txt".into(),
            row,
            PatternKind::Exact,
            IgnoreFile::Gitignore,
            w,
            cx,
        )
    });
    h.click(cx, "dialog-cancel");
    assert!(!h.has_settings(cx), "Cancel closes the dialog");
    assert!(tracked("keep.txt"));
    assert!(
        !std::fs::read_to_string(repo.path().join(".gitignore"))
            .unwrap()
            .contains("keep.txt")
    );

    // Unchecking the box only adds the pattern.
    h.act(cx, |o, w, cx| {
        o.ignore_path(
            "keep.txt".into(),
            row,
            PatternKind::Exact,
            IgnoreFile::Gitignore,
            w,
            cx,
        )
    });
    h.click(cx, "tracked-dialog-check");
    h.click(cx, "dialog-ok");
    assert!(!h.has_settings(cx), "the OK button confirms");
    assert!(tracked("keep.txt"), "still tracked");
    assert!(
        std::fs::read_to_string(repo.path().join(".gitignore"))
            .unwrap()
            .contains("/keep.txt")
    );
}

#[gpui::test]
fn ui_add_to_gitignore_from_the_changes_menu(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    repo.write("notes.tmp", "x\n");
    let h = Harness::open(cx, &repo);
    h.right_click(cx, "change-row:unversioned:notes.tmp");
    // Open in editor, Open in file manager, Copy path, Blame, History, Copy as patch,
    // Save to Patches, Shelve, then Add to .gitignore.
    h.keys(cx, "down down down down down down down down down enter");
    let gitignore = std::fs::read_to_string(repo.path().join(".gitignore")).unwrap_or_default();
    assert_eq!(gitignore, "/notes.tmp\n");
    assert!(!status_has(&h, cx, "notes.tmp"));
}

#[gpui::test]
fn ignoring_the_selected_file_selects_its_neighbour(cx: &mut TestAppContext) {
    use crate::app::row_actions::RowKind;
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    for f in ["one.tmp", "two.tmp", "three.tmp"] {
        repo.write(f, "x\n");
    }
    let h = Harness::open(cx, &repo);
    let selected = |h: &Harness, cx: &mut TestAppContext| {
        h.read(cx, |o, _| match &o.active_project().unwrap().selection {
            Selection::File { path, area } => Some((path.clone(), *area)),
            _ => None,
        })
    };
    // Untracked files are listed by name: one, three, two. Ignoring the middle one moves to
    // the next, and ignoring the last moves back to the one before it.
    for (pick, next) in [("three.tmp", "two.tmp"), ("two.tmp", "one.tmp")] {
        h.act(cx, |o, _, cx| {
            o.select_file(pick.into(), Area::Untracked, cx)
        });
        h.act(cx, |o, w, cx| {
            o.run_row_action(RowKind::AddToGitignore, w, cx)
        });
        assert!(!status_has(&h, cx, pick), "{pick} ignored");
        assert_eq!(
            selected(&h, cx),
            Some((next.to_string(), Area::Untracked)),
            "after ignoring {pick}"
        );
    }
}

#[gpui::test]
fn ui_discard_from_the_changes_menu(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    repo.write("a.txt", "changed\n");
    let h = Harness::open(cx, &repo);
    h.right_click(cx, "change-row:changes:a.txt");
    // Open in editor, Open in file manager, Copy path, Blame, History, Copy as patch,
    // Save to Patches, Shelve, Add to .gitignore, Add all *.txt files, Ignore only on this
    // machine, Untrack, then Discard changes.
    h.keys(cx, &["down"; 13].join(" "));
    h.keys(cx, "enter");
    assert!(h.has_settings(cx), "discard asks first");
    assert_eq!(repo.read("a.txt"), "changed\n", "nothing discarded yet");
    h.click(cx, "dialog-ok");
    assert_eq!(repo.read("a.txt"), "a\n");
    assert!(!status_has(&h, cx, "a.txt"));
}

#[gpui::test]
fn ui_confirm_dialogs_have_working_buttons(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    let h = Harness::open(cx, &repo);
    let hits = std::rc::Rc::new(std::cell::Cell::new(0));
    let open = |h: &Harness, cx: &mut TestAppContext, hits: std::rc::Rc<std::cell::Cell<i32>>| {
        h.act(cx, move |o, w, cx| {
            o.confirm(
                "Title",
                "Body",
                "Do it",
                false,
                move |_, _, _| hits.set(hits.get() + 1),
                w,
                cx,
            )
        });
    };
    open(&h, cx, hits.clone());
    assert!(h.bounds(cx, "dialog-ok").is_some() && h.bounds(cx, "dialog-cancel").is_some());
    h.click(cx, "dialog-cancel");
    assert!(!h.has_settings(cx));
    assert_eq!(hits.get(), 0);
    open(&h, cx, hits.clone());
    h.click(cx, "dialog-ok");
    assert!(!h.has_settings(cx));
    assert_eq!(hits.get(), 1);
}

#[gpui::test]
fn ui_clicking_the_option_words_toggles_the_checkbox(cx: &mut TestAppContext) {
    use crate::app::ignore_ops::RowState;
    let repo = TempRepo::new();
    repo.commit_file("notes.txt", "n\n", "tracked");
    let h = Harness::open(cx, &repo);
    let row = RowState {
        is_dir: false,
        tracked: true,
    };
    h.act(cx, |o, w, cx| {
        o.untrack_prompt("notes.txt".into(), row, w, cx)
    });
    h.click(cx, "tracked-dialog-option");
    h.click(cx, "dialog-ok");
    assert!(
        repo.git(["ls-files", "--", "notes.txt"]).trim().is_empty(),
        "untracked"
    );
    assert!(
        !repo.path().join(".gitignore").exists(),
        "the unticked option did not add a pattern"
    );
}

#[gpui::test]
fn untrack_offers_to_ignore_too(cx: &mut TestAppContext) {
    use crate::app::ignore_ops::RowState;
    let repo = TempRepo::new();
    repo.commit_file("build/out.bin", "b\n", "tracked build output");
    let h = Harness::open(cx, &repo);
    let row = RowState {
        is_dir: true,
        tracked: true,
    };
    h.act(cx, |o, w, cx| o.untrack_prompt("build".into(), row, w, cx));
    assert!(h.has_settings(cx));
    h.keys(cx, "enter");
    assert!(
        repo.git(["ls-files", "--", "build"]).trim().is_empty(),
        "folder untracked"
    );
    assert!(repo.path().join("build/out.bin").exists());
    let gitignore = std::fs::read_to_string(repo.path().join(".gitignore")).unwrap();
    assert_eq!(gitignore, "/build/\n");
    assert!(
        status_has(&h, cx, "build/out.bin"),
        "the staged removal shows at once"
    );
}

#[gpui::test]
fn commit_message_history_and_keyboard_staging(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "one\n", "first");
    repo.write("a.txt", "two\n");
    let h = Harness::open(cx, &repo);

    // Space on the selected file stages it, a second Space unstages it.
    h.act(cx, |osier, _, cx| {
        osier.select_file("a.txt".into(), Area::Changed, cx)
    });
    h.act(cx, |osier, window, cx| {
        osier.toggle_stage_selected(window, cx)
    });
    cx.run_until_parked();
    assert_eq!(
        repo.git(["diff", "--cached", "--name-only"]).trim(),
        "a.txt"
    );
    h.act(cx, |osier, _, cx| {
        osier.select_file("a.txt".into(), Area::Changed, cx)
    });
    h.act(cx, |osier, window, cx| {
        osier.toggle_stage_selected(window, cx)
    });
    cx.run_until_parked();
    assert_eq!(repo.git(["diff", "--cached", "--name-only"]).trim(), "");

    // Delete asks before discarding.
    h.act(cx, |osier, _, cx| {
        osier.select_file("a.txt".into(), Area::Changed, cx)
    });
    h.act(cx, |osier, window, cx| osier.discard_selected(window, cx));
    let asked = cx
        .update_window(h.window, |_, window, cx| {
            use gpui_kit::component::WindowExt as _;
            window.has_active_dialog(cx)
        })
        .expect("window");
    assert!(asked, "discard opens a confirmation");
    assert_eq!(repo.read("a.txt"), "two\n", "nothing discarded yet");

    // Committed messages come back in the history, newest first, and fill the box.
    h.act(cx, |osier, window, cx| {
        use gpui_kit::component::WindowExt as _;
        window.close_dialog(cx);
        osier.stage_paths(vec!["a.txt".into()], window, cx);
    });
    cx.run_until_parked();
    h.act(cx, |osier, window, cx| {
        osier.set_message("Second commit message", window, cx);
        osier.commit(window, cx);
    });
    cx.run_until_parked();
    let history = h.read(cx, |osier, _| osier.recent_commit_messages());
    assert_eq!(
        history.first().map(String::as_str),
        Some("Second commit message")
    );
    h.act(cx, |osier, window, cx| {
        osier.set_message(&history[0], window, cx)
    });
    h.read(cx, |osier, cx| {
        assert_eq!(osier.commit_message_text(cx), "Second commit message")
    });
}

#[gpui::test]
fn staging_and_committing_through_the_app(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "one\n", "first");
    repo.write("a.txt", "two\n");
    let h = Harness::open(cx, &repo);

    h.act(cx, |osier, window, cx| {
        osier.stage_paths(vec!["a.txt".into()], window, cx)
    });
    h.read(cx, |osier, _| {
        let snapshot = osier.active_project().unwrap().snapshot.as_ref().unwrap();
        assert_eq!(
            snapshot.status.staged().count(),
            1,
            "file staged via the app"
        );
        assert_eq!(snapshot.status.unstaged().count(), 0);
    });

    h.act(cx, |osier, window, cx| {
        osier.set_message("From the headless test", window, cx);
        assert!(osier.can_commit(cx));
        osier.commit(window, cx);
    });
    h.read(cx, |osier, cx| {
        let snapshot = osier.active_project().unwrap().snapshot.as_ref().unwrap();
        assert_eq!(snapshot.commits.len(), 2);
        assert_eq!(snapshot.commits[0].subject, "From the headless test");
        assert!(snapshot.status.is_clean());
        assert!(
            osier.commit_message_text(cx).is_empty(),
            "message cleared after commit"
        );
    });
    assert_eq!(repo.git(["log", "--oneline"]).lines().count(), 2);
}

#[gpui::test]
fn hunk_staging_and_discard_from_the_diff_pane(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    let base = "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n";
    repo.commit_file("f.txt", base, "base");
    repo.write(
        "f.txt",
        "ONE\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nTEN\n",
    );
    let h = Harness::open(cx, &repo);
    h.read(cx, |osier, _| {
        assert_eq!(osier.diff_model[0].hunks.len(), 2)
    });

    h.act(cx, |osier, window, cx| {
        osier.hunk_action(
            "f.txt".into(),
            1,
            crate::app::hunk_ops::HunkAction::Stage,
            window,
            cx,
        )
    });
    h.read(cx, |osier, _| {
        let snapshot = osier.active_project().unwrap().snapshot.as_ref().unwrap();
        let f = snapshot
            .status
            .entries
            .iter()
            .find(|e| e.path == "f.txt")
            .unwrap();
        assert!(f.is_staged() && f.is_unstaged(), "partially staged");
    });
    // One diff (HEAD against the working tree) still shows both hunks; only the second is
    // staged, and the index has exactly its change.
    use osier_git::staging::Check;
    h.read(cx, |osier, _| {
        assert_eq!(osier.diff_model[0].hunks.len(), 2);
        let staging = osier.diff_staging.as_ref().expect("staging");
        assert_eq!(staging.hunk_check(0), Check::Unstaged);
        assert_eq!(staging.hunk_check(1), Check::Staged);
    });
    let index = repo.git(["show", ":f.txt"]);
    assert!(index.contains("TEN") && !index.contains("ONE"));
    assert_eq!(check_of(&h, cx, "f.txt"), Some(Check::Partial));
}

#[gpui::test]
fn ui_commit_buttons_wrap_together_in_a_narrow_pane(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "one\n", "first");
    repo.write("a.txt", "two\n");
    // (centre pane width, whether the buttons fit next to the checkboxes)
    // The last one is the narrowest the pane goes, too narrow for both buttons side by side.
    for (center, fits) in [(700., true), (440., false), (260., false)] {
        let mut state = State::default();
        state.layout.center_width = Some(center);
        let h = Harness::open_with(cx, state, Some(repo.path().to_path_buf()));
        let amend = h.bounds(cx, "amend").expect("amend checkbox");
        let push = h.bounds(cx, "commit-push").expect("commit and push");
        let commit = h.bounds(cx, "commit").expect("commit");
        let pane = h.bounds(cx, "center-pane").expect("center pane");
        let below = push.origin.y > amend.origin.y + amend.size.height / 2.;
        assert_eq!(below, !fits, "at {center}: buttons under the checkboxes");
        if center >= 440. {
            assert_eq!(
                push.origin.y, commit.origin.y,
                "at {center}: both buttons on one line"
            );
            assert!(push.origin.x < commit.origin.x, "at {center}: order kept");
        }
        for b in [push, commit] {
            assert!(
                b.origin.x >= pane.origin.x
                    && b.origin.x + b.size.width <= pane.origin.x + pane.size.width,
                "at {center}: nothing runs past the pane"
            );
        }
    }
}

/// Select lines `start..=end` of the new side of `path` in the diff pane, as dragging over
/// them does.
fn select_new_lines(h: &Harness, cx: &mut TestAppContext, path: &'static str, start: usize) {
    use gpui_kit::component::diff::{DiffLineRange, DiffSide};
    h.act(cx, |osier, _, cx| {
        osier.diff.update(cx, |d, cx| {
            let range = DiffLineRange::new(path, DiffSide::Modified, start, start);
            d.set_selected_lines(Some(range), cx)
        });
        cx.notify();
    });
}

#[gpui::test]
fn ui_unstage_lines_takes_only_the_selected_lines(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("f.txt", "a\nb\nc\n", "base");
    repo.write("f.txt", "a\nB\nc\nd\n");
    repo.git(["add", "f.txt"]);
    let h = Harness::open(cx, &repo);
    h.click(cx, "change-row:changes:f.txt");
    assert!(
        h.bounds(cx, "lines-unstage").is_none(),
        "no button without a selection"
    );
    // The added "d" is line 4 of the new side; the b -> B change shares its hunk.
    select_new_lines(&h, cx, "f.txt", 4);
    h.click(cx, "lines-unstage");
    assert_eq!(
        repo.git(["diff", "--cached", "-U0"])
            .lines()
            .filter(|l| l.starts_with(['+', '-']) && !l.starts_with("+++") && !l.starts_with("---"))
            .collect::<Vec<_>>(),
        ["-b", "+B"],
        "the other change of the hunk stays staged"
    );
    assert_eq!(
        repo.git(["diff", "-U0"])
            .lines()
            .filter(|l| l.starts_with('+') && !l.starts_with("+++"))
            .collect::<Vec<_>>(),
        ["+d"],
        "the unstaged line is back under Unstaged"
    );
}

#[gpui::test]
fn ui_stage_lines_takes_only_the_selected_lines(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("f.txt", "a\nb\nc\n", "base");
    repo.write("f.txt", "a\nB\nc\nd\n");
    let h = Harness::open(cx, &repo);
    h.click(cx, "change-row:changes:f.txt");
    select_new_lines(&h, cx, "f.txt", 4);
    h.click(cx, "lines-stage");
    assert_eq!(
        repo.git(["diff", "--cached", "-U0"])
            .lines()
            .filter(|l| l.starts_with('+') && !l.starts_with("+++"))
            .collect::<Vec<_>>(),
        ["+d"]
    );
    assert!(
        h.bounds(cx, "lines-stage").is_none(),
        "the buttons go once the lines are staged"
    );
}

#[gpui::test]
fn ui_line_buttons_go_when_the_press_is_released_elsewhere(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("f.txt", "a\nb\nc\n", "base");
    repo.write("f.txt", "a\nB\nc\nd\n");
    let h = Harness::open(cx, &repo);
    h.click(cx, "change-row:changes:f.txt");
    select_new_lines(&h, cx, "f.txt", 4);
    let button = h.bounds(cx, "lines-stage").unwrap().center();
    let elsewhere = h.bounds(cx, "commit-message").unwrap().center();
    let mut v = h.visual(cx);
    v.simulate_mouse_down(button, gpui::MouseButton::Left, gpui::Modifiers::default());
    v.run_until_parked();
    v.simulate_mouse_up(
        elsewhere,
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    v.run_until_parked();
    assert!(h.bounds(cx, "lines-stage").is_none());
    assert!(
        repo.git(["diff", "--cached", "--name-only"])
            .trim()
            .is_empty()
    );
}

#[gpui::test]
fn ui_discard_lines_takes_only_the_selected_lines(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("f.txt", "a\nb\nc\n", "base");
    repo.write("f.txt", "a\nB\nc\nd\n");
    let h = Harness::open(cx, &repo);
    h.click(cx, "change-row:changes:f.txt");
    select_new_lines(&h, cx, "f.txt", 4);
    h.click(cx, "lines-discard");
    h.click(cx, "dialog-ok");
    assert_eq!(repo.read("f.txt"), "a\nB\nc\n");
}

#[gpui::test]
fn ui_palette_only_commands_can_be_bound(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "one\n", "first");
    repo.write("a.txt", "two\n");
    repo.write("b.txt", "new\n");
    let h = Harness::open(cx, &repo);
    h.act(cx, |o, _, cx| o.start_key_recording("stage_all", cx));
    h.keys(cx, "ctrl-alt-a");
    h.keys(cx, "ctrl-alt-a");
    let staged = repo.git(["diff", "--cached", "--name-only"]);
    assert_eq!(staged.lines().collect::<Vec<_>>(), ["a.txt", "b.txt"]);
}

/// One file with two changes far apart (two hunks).
fn two_hunk_file() -> TempRepo {
    let repo = TempRepo::new();
    let base: String = (1..=30).map(|i| format!("line {i}\n")).collect();
    repo.commit_file("f.txt", &base, "base");
    repo.write(
        "f.txt",
        &base
            .replace("line 2\n", "line two\n")
            .replace("line 28\n", "line twenty-eight\n"),
    );
    repo
}

fn has_dialog(h: &Harness, cx: &mut TestAppContext) -> bool {
    cx.update_window(h.window, |_, window, cx| {
        use gpui_kit::component::WindowExt as _;
        window.has_active_dialog(cx)
    })
    .unwrap()
}

#[gpui::test]
fn ui_group_checkbox_ticks_all_and_a_dash_ticks_too(cx: &mut TestAppContext) {
    use osier_git::staging::Check;
    let repo = three_unstaged();
    let h = Harness::open(cx, &repo);
    // One file ticked: the group shows a dash, and clicking it ticks the rest.
    h.click(cx, "change-stage:changes:a.txt");
    h.click(cx, "change-check-group:changes");
    for f in ["a.txt", "b.txt", "c.txt"] {
        assert_eq!(check_of(&h, cx, f), Some(Check::Staged), "{f}");
    }
    // All ticked: clicking clears them all.
    h.click(cx, "change-check-group:changes");
    for f in ["a.txt", "b.txt", "c.txt"] {
        assert_eq!(check_of(&h, cx, f), Some(Check::Unstaged), "{f}");
    }
}

#[gpui::test]
fn ui_adding_many_unversioned_files_asks_first(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    let many = crate::views::changes::ADD_ALL_CONFIRM + 1;
    for i in 0..many {
        repo.write(&format!("new-{i:03}.txt"), "n\n");
    }
    let h = Harness::open(cx, &repo);
    h.click(cx, "change-check-group:unversioned");
    assert!(has_dialog(&h, cx), "asks before adding {many} files");
    assert!(
        repo.git(["diff", "--cached", "--name-only"])
            .trim()
            .is_empty()
    );
    h.click(cx, "dialog-ok");
    assert_eq!(
        repo.git(["diff", "--cached", "--name-only"])
            .lines()
            .count(),
        many
    );
}

#[gpui::test]
fn ui_a_few_unversioned_files_are_added_at_once(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    repo.write("one.txt", "1\n");
    repo.write("two.txt", "2\n");
    let h = Harness::open(cx, &repo);
    h.click(cx, "change-check-group:unversioned");
    assert!(!has_dialog(&h, cx));
    assert_eq!(
        repo.git(["diff", "--cached", "--name-only"])
            .lines()
            .count(),
        2
    );
    assert!(h.bounds(cx, "change-row:changes:one.txt").is_some());
}

#[gpui::test]
fn ui_a_partly_staged_file_says_how_many_changes(cx: &mut TestAppContext) {
    use osier_git::staging::Check;
    let repo = two_hunk_file();
    let h = Harness::open(cx, &repo);
    h.click(cx, "change-row:changes:f.txt");
    h.click(cx, "hunk-check:0");
    assert_eq!(check_of(&h, cx, "f.txt"), Some(Check::Partial));
    let counts = h.read(cx, |o, _| {
        o.active_project()
            .and_then(|p| p.snapshot.as_ref())
            .and_then(|s| s.partial_changes.get("f.txt").copied())
    });
    assert_eq!(counts, Some((1, 2)));
    assert!(h.bounds(cx, "change-partial:f.txt").is_some());
    // Space on the partly staged file (back in the list) ticks all of it.
    h.click(cx, "change-row:changes:f.txt");
    h.keys(cx, "space");
    assert_eq!(check_of(&h, cx, "f.txt"), Some(Check::Staged));
    assert!(h.bounds(cx, "change-partial:f.txt").is_none());
}

#[gpui::test]
fn ui_hunk_checkbox_and_roll_back_in_the_gutter(cx: &mut TestAppContext) {
    use osier_git::staging::Check;
    let repo = two_hunk_file();
    let h = Harness::open(cx, &repo);
    h.click(cx, "change-row:changes:f.txt");
    // Like PhpStorm: the roll-back arrow on the change's removed (red) line, the checkbox on
    // its added (green) line just below it.
    let revert = h.bounds(cx, "hunk-revert:0").expect("revert arrow");
    let check = h.bounds(cx, "hunk-check:0").expect("checkbox");
    assert!(
        revert.origin.y < check.origin.y,
        "red line above green line"
    );
    h.click(cx, "hunk-check:1");
    h.read(cx, |o, _| {
        let s = o.diff_staging.as_ref().expect("staging");
        assert_eq!(s.hunk_check(0), Check::Unstaged);
        assert_eq!(s.hunk_check(1), Check::Staged);
    });
    // Clicking a ticked hunk clears it.
    h.click(cx, "hunk-check:1");
    h.read(cx, |o, _| {
        assert_eq!(
            o.diff_staging.as_ref().unwrap().hunk_check(1),
            Check::Unstaged
        )
    });
    // Roll back the first hunk: asks, then puts the line back in the file and the index.
    h.click(cx, "hunk-check:0");
    h.click(cx, "hunk-revert:0");
    assert!(has_dialog(&h, cx), "roll back asks first");
    h.click(cx, "dialog-ok");
    let work = repo.read("f.txt");
    assert!(work.contains("line 2\n") && work.contains("line twenty-eight"));
    let index = repo.git(["show", ":f.txt"]);
    assert!(index.contains("line 2\n") && !index.contains("line two"));
    h.read(cx, |o, _| {
        assert_eq!(o.diff_staging.as_ref().unwrap().shown.hunks.len(), 1)
    });
}

#[gpui::test]
fn ui_roll_back_sits_above_the_checkbox_for_one_kind_changes(cx: &mut TestAppContext) {
    // Hunks far enough apart to stay separate: three lines added, one added, one removed,
    // three removed.
    let base: String = (1..=60).map(|n| format!("line {n}\n")).collect();
    let mut work: Vec<String> = base.lines().map(str::to_owned).collect();
    work.drain(49..52); // lines 50-52 removed
    work.remove(39); // line 40 removed
    work.insert(25, "one added".into()); // after line 25
    for (ix, text) in ["added a", "added b", "added c"].into_iter().enumerate() {
        work.insert(5 + ix, text.into()); // after line 5
    }
    let repo = TempRepo::new();
    repo.commit_file("f.txt", &base, "base");
    repo.write("f.txt", &(work.join("\n") + "\n"));
    repo.write("new.txt", "unversioned\n");
    let h = Harness::open(cx, &repo);
    h.click(cx, "change-row:changes:f.txt");
    for hix in 0..4 {
        let revert = h
            .bounds(cx, &format!("hunk-revert:{hix}"))
            .unwrap_or_else(|| panic!("hunk {hix}: revert arrow"));
        let check = h
            .bounds(cx, &format!("hunk-check:{hix}"))
            .unwrap_or_else(|| panic!("hunk {hix}: checkbox"));
        assert!(
            revert.origin.y < check.origin.y,
            "hunk {hix}: arrow above the checkbox"
        );
    }
    // An unversioned file: no hunk controls, its checkbox adds it.
    h.click(cx, "change-row:unversioned:new.txt");
    assert!(h.bounds(cx, "hunk-check:0").is_none());
    assert!(h.bounds(cx, "hunk-revert:0").is_none());
}

#[gpui::test]
fn ui_reloading_the_same_file_keeps_its_diff_on_screen(cx: &mut TestAppContext) {
    let repo = two_hunk_file();
    repo.write("other.txt", "other\n");
    let h = Harness::open(cx, &repo);
    h.click(cx, "change-row:changes:f.txt");
    // Reload without letting it finish: what a tick or a file change does. The diff stays up
    // (no "Reading diff" placeholder for a frame: the flicker).
    let reload = |cx: &mut TestAppContext| {
        let view = h.view.clone();
        cx.update_window(h.window, |_, _, cx| {
            view.update(cx, |o, cx| o.reload_selection(cx))
        })
        .unwrap();
    };
    reload(cx);
    h.read(cx, |o, _| assert!(!o.diff_loading && o.diff_refreshing));
    assert!(
        h.bounds(cx, "hunk-check:0").is_some(),
        "the diff is still drawn"
    );
    cx.run_until_parked();
    h.read(cx, |o, _| assert!(!o.diff_loading && !o.diff_refreshing));
    // Another file still shows the placeholder while it loads.
    h.act(cx, |o, _, cx| {
        if let Some(p) = o.active_project_mut() {
            p.selection = crate::project::Selection::File {
                path: "other.txt".into(),
                area: crate::project::Area::Untracked,
            };
        }
        let _ = cx;
    });
    reload(cx);
    h.read(cx, |o, _| assert!(o.diff_loading && !o.diff_refreshing));
}

#[gpui::test]
fn ui_rolling_back_a_new_file_asks_to_trash_it(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    repo.write("new.txt", "new\n");
    repo.git(["add", "new.txt"]);
    let h = Harness::open(cx, &repo);
    h.click(cx, "change-row:changes:new.txt");
    h.keys(cx, "delete");
    assert!(has_dialog(&h, cx));
    // Cancel: the system trash is not touched in tests.
    h.click(cx, "dialog-cancel");
    assert_eq!(repo.read("new.txt"), "new\n");
}

#[gpui::test]
fn ui_group_checkbox_copes_with_staged_deletions_and_renames(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("keep.txt", "k\n", "keep");
    repo.commit_file("gone.txt", "g\n", "gone");
    repo.commit_file("old.txt", "o\n", "old");
    repo.write("keep.txt", "changed\n");
    // A deletion that is already staged (no file left to `git add`) and a staged rename.
    repo.git(["rm", "-q", "gone.txt"]);
    repo.git(["mv", "old.txt", "new.txt"]);
    let h = Harness::open(cx, &repo);
    h.click(cx, "change-check-group:changes");
    assert!(
        repo.git(["diff", "--name-only"]).trim().is_empty(),
        "everything staged"
    );
    h.click(cx, "change-check-group:changes");
    assert!(
        repo.git(["diff", "--cached", "--name-only"])
            .trim()
            .is_empty(),
        "nothing staged, the rename's old path included"
    );
}

fn unchanged(h: &Harness, cx: &mut TestAppContext) -> Option<osier_git::staging::Unchanged> {
    h.read(cx, |osier, _| osier.diff_unchanged.clone())
}

#[gpui::test]
fn ui_a_staged_change_undone_in_the_file_says_why_it_is_listed(cx: &mut TestAppContext) {
    use osier_git::staging::Unchanged;
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "one\n", "first");
    repo.commit_file("b.txt", "b\n", "second");
    repo.write("a.txt", "two\n");
    repo.git(["add", "a.txt"]);
    repo.write("a.txt", "one\n");
    repo.write("b.txt", "B\n");
    let h = Harness::open(cx, &repo);
    h.click(cx, "change-row:changes:a.txt");
    assert_eq!(unchanged(&h, cx), Some(Unchanged::StagedOnly));
    h.read(cx, |osier, _| assert_eq!(osier.diff_files, 0));
    // A file with a real diff has no such note.
    h.click(cx, "change-row:changes:b.txt");
    assert_eq!(unchanged(&h, cx), None);
    h.read(cx, |osier, _| assert_eq!(osier.diff_files, 1));
    // What the note promises: the checkbox stages the file as it is and the row goes.
    h.click(cx, "change-row:changes:a.txt");
    h.click(cx, "change-stage:changes:a.txt");
    assert_eq!(repo.git(["status", "--porcelain"]).trim(), "M b.txt");
    assert!(h.bounds(cx, "change-row:changes:a.txt").is_none());
}

#[cfg(unix)]
#[gpui::test]
fn ui_a_permissions_only_change_says_so(cx: &mut TestAppContext) {
    use osier_git::staging::Unchanged;
    use std::os::unix::fs::PermissionsExt as _;
    let repo = TempRepo::new();
    repo.commit_file("run.sh", "echo hi\n", "first");
    std::fs::set_permissions(
        repo.path().join("run.sh"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    repo.git(["add", "run.sh"]);
    let h = Harness::open(cx, &repo);
    h.click(cx, "change-row:changes:run.sh");
    let reason = unchanged(&h, cx).expect("a reason");
    assert_eq!(
        reason,
        Unchanged::Mode {
            old: "100644".into(),
            new: "100755".into()
        }
    );
    let (title, hint) = crate::views::diff_pane::unchanged_text(&reason);
    assert_eq!(title, "Only the permissions changed");
    assert!(hint.starts_with("This file was made executable."), "{hint}");
}

/// A repo whose `a.txt` has a staged change that was undone in the file: on disk it matches
/// the last commit.
fn staged_then_undone() -> TempRepo {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "one\n", "first");
    repo.write("a.txt", "two\n");
    repo.git(["add", "a.txt"]);
    repo.write("a.txt", "one\n");
    repo
}

fn roll_back(h: &Harness, cx: &mut TestAppContext, path: &'static str) {
    h.click(
        cx,
        Box::leak(format!("change-row:changes:{path}").into_boxed_str()),
    );
    h.keys(cx, "delete");
    h.click(cx, "dialog-ok");
}

#[cfg(unix)]
#[gpui::test]
fn ui_roll_back_leaves_a_matching_file_untouched_when_rewriting_is_off(cx: &mut TestAppContext) {
    use std::os::unix::fs::MetadataExt as _;
    let repo = staged_then_undone();
    let inode = || std::fs::metadata(repo.path().join("a.txt")).unwrap().ino();
    let before = inode();
    let h = Harness::open(cx, &repo);
    h.act(cx, |osier, _, _| {
        osier.settings.git.rollback_always_rewrite = false
    });
    roll_back(&h, cx, "a.txt");
    assert!(repo.git(["status", "--porcelain"]).trim().is_empty());
    assert_eq!(inode(), before, "the file on disk was not replaced");
    assert_eq!(repo.read("a.txt"), "one\n");
}

#[gpui::test]
fn ui_roll_back_clears_a_staged_only_change_by_default(cx: &mut TestAppContext) {
    let repo = staged_then_undone();
    let h = Harness::open(cx, &repo);
    h.read(cx, |osier, _| {
        assert!(osier.settings.git.rollback_always_rewrite)
    });
    roll_back(&h, cx, "a.txt");
    assert!(repo.git(["status", "--porcelain"]).trim().is_empty());
    assert_eq!(repo.read("a.txt"), "one\n");
}

#[gpui::test]
fn ui_a_small_renamed_and_edited_file_shows_as_one_diff(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("tiny.txt", "Short note.\nTwo lines.\n", "first");
    repo.git(["mv", "tiny.txt", "small.txt"]);
    repo.write("small.txt", "Short note!\nTwo lines.\n");
    let h = Harness::open(cx, &repo);
    h.click(cx, "change-row:changes:small.txt");
    h.read(cx, |osier, _| {
        assert_eq!(
            osier.diff_files, 1,
            "one file, not one added and one deleted"
        );
        let file = &osier.diff_model[0];
        assert_eq!(file.status, osier_git::diff::FileStatus::Renamed);
        assert_eq!(file.old_path.as_deref(), Some("tiny.txt"));
        assert_eq!(file.hunks.len(), 1);
        assert!(osier.diff_unchanged.is_none());
    });
    // Rolling the change back from the diff leaves the plain move.
    h.click(cx, "hunk-revert:0");
    h.click(cx, "dialog-ok");
    assert_eq!(repo.read("small.txt"), "Short note.\nTwo lines.\n");
    assert_eq!(
        h.read(cx, |osier, _| osier.diff_unchanged.clone()),
        Some(osier_git::staging::Unchanged::Renamed {
            from: "tiny.txt".into()
        })
    );
}
