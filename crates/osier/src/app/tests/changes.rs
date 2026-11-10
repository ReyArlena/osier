//! The Changes tab: staging, the selection after staging, the commit box, discard, ignore and
//! untrack.

use super::*;

#[gpui::test]
fn ui_stage_with_the_checkbox_and_commit_from_the_keyboard(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "one\n", "first");
    repo.write("a.txt", "two\n");
    let h = Harness::open(cx, &repo);
    h.click(cx, "change-stage:unstaged:a.txt");
    assert_eq!(
        repo.git(["diff", "--cached", "--name-only"]).trim(),
        "a.txt"
    );
    assert!(
        h.bounds(cx, "change-row:staged:a.txt").is_some(),
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
    h.click(cx, "change-row:unstaged:a.txt");
    h.keys(cx, "down");
    h.read(cx, |osier, _| {
        assert_eq!(
            osier.active_project().unwrap().selection,
            Selection::File {
                path: "b.txt".into(),
                area: Area::Unstaged
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

#[gpui::test]
fn ui_staging_the_selected_file_moves_to_the_next(cx: &mut TestAppContext) {
    let repo = three_unstaged();
    let h = Harness::open(cx, &repo);
    h.click(cx, "change-row:unstaged:b.txt");
    h.click(cx, "change-stage:unstaged:b.txt");
    assert_eq!(selected(&h, cx), file("c.txt", Area::Unstaged));
    // The pane shows the new selection's diff, not an empty one.
    assert_eq!(h.read(cx, |o, _| o.diff_files), 1);
    // Last in the list: the one above.
    h.click(cx, "change-stage:unstaged:c.txt");
    assert_eq!(selected(&h, cx), file("a.txt", Area::Unstaged));
    // Only one left: follow it into Staged.
    h.click(cx, "change-stage:unstaged:a.txt");
    assert_eq!(selected(&h, cx), file("a.txt", Area::Staged));
    assert_eq!(h.read(cx, |o, _| o.diff_files), 1);
}

#[gpui::test]
fn ui_staging_another_file_keeps_the_selection(cx: &mut TestAppContext) {
    let repo = three_unstaged();
    let h = Harness::open(cx, &repo);
    h.click(cx, "change-row:unstaged:a.txt");
    h.click(cx, "change-stage:unstaged:c.txt");
    assert_eq!(selected(&h, cx), file("a.txt", Area::Unstaged));
    // Stage all follows the selected file; unstaging moves on in the Staged list.
    h.act(cx, |o, w, cx| o.stage_all(w, cx));
    assert_eq!(selected(&h, cx), file("a.txt", Area::Staged));
    h.click(cx, "change-stage:staged:a.txt");
    assert_eq!(selected(&h, cx), file("b.txt", Area::Staged));
}

#[gpui::test]
fn ui_staging_a_file_open_in_the_editor_keeps_it(cx: &mut TestAppContext) {
    let repo = three_unstaged();
    let h = Harness::open(cx, &repo);
    h.click(cx, "change-row:unstaged:b.txt");
    h.act(cx, |o, w, cx| o.open_editor(w, cx));
    h.click(cx, "change-stage:unstaged:b.txt");
    assert_eq!(selected(&h, cx), file("b.txt", Area::Staged));
    assert!(h.read(cx, |o, _| o.editor.is_some()));
}

#[gpui::test]
fn ui_selection_on_an_away_tab_follows_too(cx: &mut TestAppContext) {
    let repo = three_unstaged();
    let h = Harness::open(cx, &repo);
    h.click(cx, "change-row:unstaged:b.txt");
    h.act(cx, |o, _, cx| o.set_tab(Tab::Log, cx));
    repo.git(["add", "b.txt"]);
    h.act(cx, |o, _, cx| o.refresh_active(cx));
    h.act(cx, |o, _, cx| o.set_tab(Tab::Changes, cx));
    assert_eq!(selected(&h, cx), file("c.txt", Area::Unstaged));
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
    h.right_click(cx, "change-row:untracked:notes.tmp");
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
    h.right_click(cx, "change-row:unstaged:a.txt");
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
        osier.select_file("a.txt".into(), Area::Unstaged, cx)
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
        osier.select_file("a.txt".into(), Area::Staged, cx)
    });
    h.act(cx, |osier, window, cx| {
        osier.toggle_stage_selected(window, cx)
    });
    cx.run_until_parked();
    assert_eq!(repo.git(["diff", "--cached", "--name-only"]).trim(), "");

    // Delete asks before discarding.
    h.act(cx, |osier, _, cx| {
        osier.select_file("a.txt".into(), Area::Unstaged, cx)
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
    // Staged side shows exactly the second hunk.
    h.act(cx, |osier, _, cx| {
        osier.select_file("f.txt".into(), Area::Staged, cx)
    });
    h.read(cx, |osier, _| {
        let hunks = &osier.diff_model[0].hunks;
        assert_eq!(hunks.len(), 1);
        let added: Vec<&str> = hunks[0]
            .lines
            .iter()
            .filter(|l| l.kind == osier_git::diff::LineKind::Added)
            .map(|l| l.text.as_str())
            .collect();
        assert_eq!(added, ["TEN"], "only the second hunk is staged");
    });
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
