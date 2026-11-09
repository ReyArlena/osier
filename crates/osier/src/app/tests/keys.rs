//! Key bindings: the keymap page and overrides, slash and question mark search, prefix keys,
//! plain keys in text fields, menu keys.

use super::*;

#[gpui::test]
fn ui_slash_searches_projects_and_escape_clears(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    let h = Harness::open(cx, &repo);
    h.keys(cx, "/");
    h.type_text(cx, "lan");
    assert_eq!(
        input_text(&h, cx, |o| &o.sidebar_filter),
        "lan",
        "no stray slash"
    );
    h.keys(cx, "escape");
    assert_eq!(
        input_text(&h, cx, |o| &o.sidebar_filter),
        "",
        "escape clears"
    );
    assert!(focused_is(&h, cx, |o, cx| {
        gpui_kit::Focusable::focus_handle(o.sidebar_filter.read(cx), cx)
    }));
    h.keys(cx, "escape");
    assert!(
        focused_is(&h, cx, |o, _| o.focus_handle.clone()),
        "second escape leaves the box"
    );
    // The shortcut works again afterwards.
    h.keys(cx, "/");
    h.type_text(cx, "x");
    assert_eq!(input_text(&h, cx, |o| &o.sidebar_filter), "x");
}

#[gpui::test]
fn ui_question_mark_searches_files(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    for keys in ["?", "shift-/"] {
        let h = Harness::open(cx, &repo);
        h.keys(cx, keys);
        h.read(cx, |o, _| assert_eq!(o.tab, Tab::Files, "{keys}"));
        h.type_text(cx, "a.t");
        assert_eq!(input_text(&h, cx, |o| &o.files_filter), "a.t", "{keys}");
        h.keys(cx, "escape");
        assert_eq!(input_text(&h, cx, |o| &o.files_filter), "");
    }
}

#[gpui::test]
fn ui_plain_keys_type_into_text_fields(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    repo.commit_file("b.txt", "b\n", "second");
    repo.write("a.txt", "A\n");
    repo.write("b.txt", "B\n");
    let h = Harness::open(cx, &repo);
    let selection = h.read(cx, |o, _| o.active_project().unwrap().selection.clone());
    h.click(cx, "commit-message");
    h.keys(cx, "j k space / ?");
    let text = h.read(cx, |o, cx| o.commit_message_text(cx));
    assert_eq!(text, "jk /?", "every key typed as text");
    h.read(cx, |o, _| {
        assert_eq!(
            o.active_project().unwrap().selection,
            selection,
            "j/k did not move"
        );
        assert_eq!(o.tab, Tab::Changes, "? did not switch tabs");
    });
    assert_eq!(
        repo.git(["diff", "--cached", "--name-only"]).trim(),
        "",
        "space did not stage"
    );
    assert_eq!(input_text(&h, cx, |o| &o.sidebar_filter), "");
}

#[gpui::test]
fn ui_slash_in_settings_searches_settings(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    let h = Harness::open(cx, &repo);
    h.keys(cx, "ctrl-,");
    assert!(h.has_settings(cx));
    h.keys(cx, "/");
    let (inside, on_container) = {
        let view = h.view.clone();
        cx.update_window(h.window, |_, window, cx| {
            let f = view.read(cx).settings_focus.clone();
            (f.contains_focused(window, cx), f.is_focused(window))
        })
        .expect("window")
    };
    assert!(
        inside && !on_container,
        "focus moved into the settings search box"
    );
    assert!(!focused_is(&h, cx, |o, cx| {
        gpui_kit::Focusable::focus_handle(o.sidebar_filter.read(cx), cx)
    }));
    h.type_text(cx, "diff");
    assert!(
        h.has_settings(cx),
        "typing in the search box keeps settings open"
    );
    assert_eq!(
        input_text(&h, cx, |o| &o.sidebar_filter),
        "",
        "nothing reached the sidebar"
    );
}

#[gpui::test]
fn keymap_overrides_apply_live_and_keep_input_keys(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    let h = Harness::open(cx, &repo);
    // A clean repository opens on Log (`projects.empty_changes_tab`); start from Changes.
    h.act(cx, |o, _, cx| o.set_tab(Tab::Changes, cx));
    let file = h.read(cx, |o, _| o.paths.keymap_file());
    std::fs::create_dir_all(file.parent().expect("dir")).expect("mkdir");
    std::fs::write(
        &file,
        "[bindings]\nfocus_project_search = [\"ctrl-f\"]\ntab_log = []\nrefresh = [\"ctrl-??\"]\nnope = [\"x\"]\n",
    )
    .expect("write keymap");
    let mut reported = Vec::new();
    h.act(cx, |osier, _, cx| reported = osier.load_keymap(cx));
    assert!(
        reported
            .iter()
            .any(|p| p.contains("unknown action \"nope\"")),
        "{reported:?}"
    );
    assert!(
        reported.iter().any(|p| p.contains("ctrl-??")),
        "{reported:?}"
    );
    h.read(cx, |_, cx| {
        let k = crate::keymap::Keymap::global(cx);
        assert_eq!(k.keys_for("focus_project_search"), ["ctrl-f"]);
        assert!(k.keys_for("tab_log").is_empty(), "unbound");
        assert!(k.keys_for("refresh").is_empty(), "invalid key dropped");
        assert_eq!(k.keys_for("tab_files"), ["ctrl-5"], "defaults untouched");
    });
    // The old default no longer fires; the new key does.
    h.keys(cx, "/");
    assert_eq!(input_text(&h, cx, |o| &o.sidebar_filter), "");
    assert!(!focused_is(&h, cx, |o, cx| {
        gpui_kit::Focusable::focus_handle(o.sidebar_filter.read(cx), cx)
    }));
    h.keys(cx, "ctrl-f");
    h.type_text(cx, "abc");
    assert_eq!(input_text(&h, cx, |o| &o.sidebar_filter), "abc");
    // gpui-kit's own bindings survived the reload: backspace edits, escape clears.
    h.keys(cx, "backspace");
    assert_eq!(input_text(&h, cx, |o| &o.sidebar_filter), "ab");
    h.keys(cx, "escape");
    assert_eq!(input_text(&h, cx, |o| &o.sidebar_filter), "");
    h.keys(cx, "ctrl-2");
    h.read(cx, |o, _| {
        assert_eq!(o.tab, Tab::Changes, "unbound action stays unbound")
    });
}

#[gpui::test]
fn ui_keymap_page_records_removes_and_resets(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    let h = Harness::open(cx, &repo);
    let keys_for = |h: &Harness, cx: &mut TestAppContext, id: &str| {
        let id = id.to_string();
        h.read(cx, move |_, cx| {
            crate::keymap::Keymap::global(cx).keys_for(&id).to_vec()
        })
    };
    // Find Refresh through the settings search, then record a new key with the +.
    h.keys(cx, "ctrl-,");
    h.keys(cx, "/");
    h.type_text(cx, "Refresh");
    h.click(cx, "keymap-add:refresh");
    h.read(cx, |o, _| assert!(o.keymap_recording.is_some()));
    h.keys(cx, "ctrl-shift-r");
    assert_eq!(
        keys_for(&h, cx, "refresh"),
        ["ctrl-r", "f5", "ctrl-shift-r"]
    );
    let file = h.read(cx, |o, _| o.paths.keymap_file());
    let text = std::fs::read_to_string(&file).expect("keymap.toml written");
    assert!(text.contains("refresh = ["), "{text}");
    // Remove one key with its x.
    h.click(cx, "keymap-remove:refresh:f5");
    assert_eq!(keys_for(&h, cx, "refresh"), ["ctrl-r", "ctrl-shift-r"]);
    // Reset brings back the defaults and the file goes away (nothing overridden).
    h.click(cx, "keymap-reset:refresh");
    assert_eq!(keys_for(&h, cx, "refresh"), ["ctrl-r", "f5"]);
    assert!(!file.exists());
}

#[gpui::test]
fn ui_keymap_mode_buttons_and_refused_clashes(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    let h = Harness::open(cx, &repo);
    let keys_for = |h: &Harness, cx: &mut TestAppContext, id: &str| {
        let id = id.to_string();
        h.read(cx, move |_, cx| {
            crate::keymap::Keymap::global(cx).keys_for(&id).to_vec()
        })
    };
    h.keys(cx, "ctrl-,");
    h.keys(cx, "/");
    h.type_text(cx, "Search projects");
    // The three buttons switch the key's mode.
    h.click(cx, "keymode:focus_project_search:/:prefix");
    assert_eq!(keys_for(&h, cx, "focus_project_search"), ["prefix /"]);
    h.click(cx, "keymode:focus_project_search:prefix /:either");
    assert_eq!(keys_for(&h, cx, "focus_project_search"), ["prefix? /"]);
    h.click(cx, "keymode:focus_project_search:prefix? /:plain");
    assert_eq!(keys_for(&h, cx, "focus_project_search"), ["/"]);

    // A context-menu key may not take a key another binding uses.
    h.act(cx, |o, _, cx| {
        o.start_key_recording("row_add_to_gitignore", cx)
    });
    h.keys(cx, "/");
    assert!(
        keys_for(&h, cx, "row_add_to_gitignore").is_empty(),
        "refused"
    );
    h.act(cx, |o, _, cx| {
        o.start_key_recording("row_add_to_gitignore", cx)
    });
    h.keys(cx, "i");
    assert_eq!(keys_for(&h, cx, "row_add_to_gitignore"), ["i"]);
    // ...and a global cannot take it afterwards either.
    h.act(cx, |o, _, cx| o.start_key_recording("refresh", cx));
    h.keys(cx, "i");
    assert_eq!(keys_for(&h, cx, "refresh"), ["ctrl-r", "f5"]);
    // Prefixed keys live in their own namespace: `prefix i` is fine for a global.
    h.act(cx, |o, w, cx| {
        o.set_key_mode(
            "row_add_to_gitignore",
            "i",
            crate::keymap::KeyMode::Prefix,
            w,
            cx,
        )
    });
    h.act(cx, |o, _, cx| o.start_key_recording("refresh", cx));
    h.keys(cx, "i");
    assert_eq!(keys_for(&h, cx, "refresh"), ["ctrl-r", "f5", "i"]);
    // Switching the menu key back to plain would clash now: refused.
    h.act(cx, |o, w, cx| {
        o.set_key_mode(
            "row_add_to_gitignore",
            "prefix i",
            crate::keymap::KeyMode::Plain,
            w,
            cx,
        )
    });
    assert_eq!(keys_for(&h, cx, "row_add_to_gitignore"), ["prefix i"]);
}

#[gpui::test]
fn recording_captures_keys_instead_of_running_them(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    let h = Harness::open(cx, &repo);
    // A clean repository opens on Log (`projects.empty_changes_tab`); start from Changes.
    h.act(cx, |o, _, cx| o.set_tab(Tab::Changes, cx));
    h.act(cx, |o, _, cx| o.start_key_recording("tab_log", cx));
    h.keys(cx, "ctrl-3");
    h.read(cx, |o, cx| {
        assert_eq!(o.tab, Tab::Changes, "ctrl-3 was captured, not run");
        let k = crate::keymap::Keymap::global(cx);
        assert_eq!(k.keys_for("tab_log"), ["ctrl-2", "ctrl-3"]);
        assert_eq!(
            crate::keymap::conflicts(k, "tab_log", "ctrl-3"),
            ["Branches tab"]
        );
        assert!(o.keymap_recording.is_none());
    });
    // A shifted symbol is stored as the symbol; escape cancels without changes.
    h.act(cx, |o, _, cx| {
        o.start_key_recording("focus_project_search", cx)
    });
    h.keys(cx, "?");
    h.act(cx, |o, _, cx| {
        o.start_key_recording("focus_project_search", cx)
    });
    h.keys(cx, "escape");
    h.read(cx, |o, cx| {
        assert_eq!(
            crate::keymap::Keymap::global(cx).keys_for("focus_project_search"),
            ["/", "?"]
        );
        assert!(o.keymap_recording.is_none());
    });
}

#[gpui::test]
fn keymap_changes_update_the_dms_cheatsheet(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    let h = Harness::open(cx, &repo);
    let dir = tempfile::tempdir().expect("tempdir");
    let dms = dir.path().join("dms");
    std::fs::create_dir(&dms).expect("dms dir");
    h.act(cx, |o, window, cx| {
        o.dms_dir = Some(dms.clone());
        o.set_action_keys("quit", vec!["ctrl-shift-q".into()], window, cx);
    });
    let sheet = std::fs::read_to_string(dms.join("cheatsheets/osier.json")).expect("written");
    assert!(sheet.contains("\"Ctrl+Shift+Q\""), "{sheet}");
    // An outside edit of keymap.toml is picked up and reaches DMS too.
    let file = h.read(cx, |o, _| o.paths.keymap_file());
    std::fs::write(&file, "[bindings]\nquit = [\"ctrl-alt-q\"]\n").expect("edit");
    h.act(cx, |o, window, cx| o.reload_keymap(window, cx));
    let sheet = std::fs::read_to_string(dms.join("cheatsheets/osier.json")).expect("written");
    assert!(sheet.contains("\"Ctrl+Alt+Q\""), "{sheet}");
    h.read(cx, |_, cx| {
        assert_eq!(
            crate::keymap::Keymap::global(cx).keys_for("quit"),
            ["ctrl-alt-q"]
        )
    });
}

#[gpui::test]
fn ui_open_repository_moved_to_ctrl_shift_o(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "one\n", "first");
    let h = Harness::open(cx, &repo);
    h.read(cx, |_, cx| {
        let keymap = crate::keymap::Keymap::global(cx);
        assert_eq!(
            keymap.keys_for("open_repository"),
            vec!["ctrl-shift-o".to_string()]
        );
        assert_eq!(
            keymap.keys_for("switch_project"),
            vec!["ctrl-o".to_string()]
        );
    });
}

fn bind(cx: &mut TestAppContext, pairs: &[(&str, &[&str])]) {
    let mut file = osier_config::keymap::KeymapFile::default();
    for (id, keys) in pairs {
        file.bindings
            .insert(id.to_string(), keys.iter().map(|k| k.to_string()).collect());
    }
    cx.update(|cx| crate::keymap::apply(&file, cx));
}

fn filter_focused(h: &Harness, cx: &mut TestAppContext) -> bool {
    focused_is(h, cx, |o, cx| {
        gpui_kit::Focusable::focus_handle(o.sidebar_filter.read(cx), cx)
    })
}

#[gpui::test]
fn ui_prefix_key_waits_shows_a_badge_and_runs_the_next_key(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    let h = Harness::open(cx, &repo);
    bind(
        cx,
        &[
            ("prefix_key", &["ctrl-h"]),
            ("focus_project_search", &["prefix /"]),
            ("focus_file_search", &["prefix? ?"]),
        ],
    );
    let badge = |h: &Harness, cx: &mut TestAppContext| h.bounds(cx, "prefix-badge").is_some();

    // Prefix only: `/` alone does nothing, prefix then `/` searches.
    h.keys(cx, "/");
    assert!(
        !filter_focused(&h, cx),
        "prefix-only key ignored on its own"
    );
    h.keys(cx, "ctrl-h");
    assert!(badge(&h, cx), "badge while waiting");
    h.keys(cx, "/");
    assert!(!badge(&h, cx));
    assert!(filter_focused(&h, cx));

    // While typing in a text field: the prefixed key runs instead of typing.
    h.keys(cx, "ctrl-h ?");
    assert_eq!(
        input_text(&h, cx, |o| &o.sidebar_filter),
        "",
        "nothing typed"
    );
    h.read(cx, |o, _| assert_eq!(o.tab, Tab::Files));

    // Either: works alone too (from outside a text field).
    h.act(cx, |o, w, cx| {
        o.set_tab(Tab::Changes, cx);
        o.focus_handle.focus(w, cx);
    });
    h.keys(cx, "?");
    h.read(cx, |o, _| assert_eq!(o.tab, Tab::Files));
}

#[gpui::test]
fn ui_prefix_cancels_on_escape_click_unknown_key_and_timeout(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    let h = Harness::open(cx, &repo);
    bind(
        cx,
        &[
            ("prefix_key", &["ctrl-h"]),
            ("focus_project_search", &["prefix /"]),
        ],
    );
    let waiting = |h: &Harness, cx: &mut TestAppContext| h.read(cx, |o, _| o.prefix_waiting());

    h.keys(cx, "ctrl-h escape");
    assert!(!waiting(&h, cx), "escape");
    h.keys(cx, "/");
    assert!(!filter_focused(&h, cx), "the prefix was really cancelled");

    h.keys(cx, "ctrl-h");
    h.click(cx, "tab-log");
    assert!(!waiting(&h, cx), "click");

    h.keys(cx, "ctrl-h z");
    assert!(!waiting(&h, cx), "unbound key");

    h.keys(cx, "ctrl-h");
    assert!(waiting(&h, cx));
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(4));
    cx.run_until_parked();
    assert!(waiting(&h, cx), "still waiting at 4 s");
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(2));
    cx.run_until_parked();
    assert!(!waiting(&h, cx), "timed out after 5 s");
}

#[gpui::test]
fn ui_context_menu_keys_fire_on_the_selected_row(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    repo.write("notes.tmp", "x\n");
    let h = Harness::open(cx, &repo);
    bind(cx, &[("row_add_to_gitignore", &["i"])]);
    h.click(cx, "change-row:untracked:notes.tmp");
    h.keys(cx, "i");
    let gitignore = std::fs::read_to_string(repo.path().join(".gitignore")).unwrap_or_default();
    assert_eq!(gitignore, "/notes.tmp\n");
}

#[gpui::test]
fn ui_branch_tag_and_stash_rows_select_and_take_menu_keys(cx: &mut TestAppContext) {
    use crate::app::row_actions::RefRow;
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    repo.git(["branch", "feature"]);
    repo.git(["tag", "v1"]);
    repo.write("a.txt", "changed\n");
    repo.git(["stash", "-q"]);
    let h = Harness::open(cx, &repo);
    bind(
        cx,
        &[
            ("row_checkout", &["c"]),
            ("row_delete", &["x"]),
            ("row_copy_name", &["n"]),
        ],
    );
    let head = |h: &Harness, cx: &mut TestAppContext| {
        h.read(cx, |o, _| {
            o.active_project()
                .and_then(|p| p.snapshot.as_ref())
                .and_then(|s| s.status.branch.head.clone())
        })
    };

    // Click selects; the key runs the entry on that row.
    h.click(cx, "tab-branches");
    h.click(cx, "branch-row:feature");
    h.read(cx, |o, _| {
        assert_eq!(
            o.ref_selected,
            Some(RefRow::Branch {
                name: "feature".into(),
                remote: false
            })
        )
    });
    h.keys(cx, "c");
    assert_eq!(head(&h, cx).as_deref(), Some("feature"));

    // Arrow keys walk branches (feature, main), then tags.
    h.keys(cx, "down");
    h.read(cx, |o, _| {
        assert!(matches!(
            &o.ref_selected,
            Some(RefRow::Branch { remote: false, .. })
        ))
    });
    h.keys(cx, "down");
    h.read(cx, |o, _| {
        assert_eq!(o.ref_selected, Some(RefRow::Tag("v1".into())))
    });
    h.keys(cx, "n");
    assert_eq!(
        cx.read_from_clipboard().and_then(|c| c.text()).as_deref(),
        Some("v1")
    );

    // Stashes: select, then Delete / Drop asks first.
    h.click(cx, "tab-stashes");
    h.click(cx, "stash-row:0");
    h.keys(cx, "x");
    assert!(h.has_settings(cx), "drop asks for confirmation");
    h.click(cx, "dialog-ok");
    assert!(
        repo.git(["stash", "list"]).trim().is_empty(),
        "stash dropped"
    );
}

#[gpui::test]
fn menu_entries_show_their_key(cx: &mut TestAppContext) {
    use crate::app::row_actions::{RowKind, key_hint};
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    let _h = Harness::open(cx, &repo);
    assert_eq!(cx.update(|cx| key_hint(RowKind::AddToGitignore, cx)), None);
    bind(
        cx,
        &[
            ("row_add_to_gitignore", &["i"]),
            ("row_untrack", &["prefix u"]),
        ],
    );
    assert_eq!(
        cx.update(|cx| key_hint(RowKind::AddToGitignore, cx))
            .as_deref(),
        Some("I")
    );
    assert_eq!(
        cx.update(|cx| key_hint(RowKind::Untrack, cx)).as_deref(),
        Some("Prefix U")
    );
}

#[gpui::test]
fn ui_shortcuts_survive_hiding_the_focused_filter(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    let h = Harness::open(cx, &repo);
    // Focus the sidebar filter, then hide the sidebar under it.
    h.keys(cx, "/");
    h.type_text(cx, "a");
    h.keys(cx, "ctrl-b");
    h.read(cx, |o, _| {
        assert!(!o.sidebar_visible, "ctrl-b hid the sidebar")
    });
    h.keys(cx, "ctrl-2");
    h.read(cx, |o, _| {
        assert_eq!(o.tab, Tab::Log, "shortcuts still work with the filter gone")
    });
    h.keys(cx, "ctrl-b");
    h.read(cx, |o, _| {
        assert!(o.sidebar_visible, "and ctrl-b brings it back")
    });

    // The same with the Files tab's filter and a tab switch.
    h.keys(cx, "?");
    h.type_text(cx, "a");
    h.keys(cx, "ctrl-1");
    h.read(cx, |o, _| assert_eq!(o.tab, Tab::Changes));
    h.keys(cx, "ctrl-2");
    h.read(cx, |o, _| {
        assert_eq!(o.tab, Tab::Log, "shortcuts work after leaving Files")
    });
}

#[gpui::test]
fn ui_quit_reaches_the_app_while_a_dialog_is_open(cx: &mut TestAppContext) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static QUIT: AtomicBool = AtomicBool::new(false);
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    let h = Harness::open(cx, &repo);
    // A second global listener, after the one `init` registers: the test platform's `quit`
    // does nothing, so observe the action reaching the app level instead.
    cx.update(|cx| {
        cx.on_action(|_: &crate::app::Quit, _| QUIT.store(true, Ordering::SeqCst));
    });
    h.keys(cx, "ctrl-p");
    assert!(h.has_settings(cx), "the palette is open");
    h.keys(cx, "ctrl-q");
    assert!(
        QUIT.load(Ordering::SeqCst),
        "ctrl-q is handled at the app level, not only by the root view"
    );
}

#[gpui::test]
fn ui_shortcuts_close_an_open_dropdown_menu(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    let h = Harness::open(cx, &repo);
    h.click(cx, "branch");
    assert!(h.menu_open(cx), "the branch button opens its menu");
    // gpui-kit's menu closes itself only on Escape or a click outside. A shortcut that moves
    // on (a tab switch here) must not leave it hanging over the new tab.
    h.keys(cx, "ctrl-2");
    h.read(cx, |o, _| assert_eq!(o.tab, Tab::Log));
    assert!(!h.menu_open(cx), "the shortcut closed the menu");
    // The same when the shortcut moves focus itself (the sidebar filter).
    h.click(cx, "branch");
    assert!(h.menu_open(cx));
    h.keys(cx, "/");
    assert!(!h.menu_open(cx), "focusing the filter closed the menu");
    assert!(
        focused_is(&h, cx, |o, cx| {
            gpui_kit::Focusable::focus_handle(o.sidebar_filter.read(cx), cx)
        }),
        "and the filter has focus"
    );
    // Escape still works as before.
    h.click(cx, "branch");
    assert!(h.menu_open(cx));
    h.keys(cx, "escape");
    assert!(!h.menu_open(cx), "escape closes the menu");
}
