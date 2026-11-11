//! Merge conflicts: the inline conflict view and the three-way merge tool.

use super::*;

#[gpui::test]
fn merge_conflict_resolve_and_continue(cx: &mut TestAppContext) {
    use osier_git::ops::integrate::{ConflictSide, MergeMode};
    use osier_git::repo::InProgress;

    let repo = TempRepo::new();
    repo.commit_file("c.txt", "base\n", "base");
    repo.checkout_new("feature");
    repo.commit_file("c.txt", "theirs\n", "theirs");
    repo.checkout("main");
    repo.commit_file("c.txt", "ours\n", "ours");
    let h = Harness::open(cx, &repo);

    h.act(cx, |osier, window, cx| {
        osier.merge_branch("feature".into(), MergeMode::Default, window, cx)
    });
    h.read(cx, |osier, _| {
        assert_eq!(osier.in_progress(), Some(InProgress::Merge));
        let snapshot = osier.active_project().unwrap().snapshot.as_ref().unwrap();
        assert_eq!(snapshot.status.conflicted().count(), 1);
        assert_eq!(
            osier.active_project().unwrap().selection,
            Selection::File {
                path: "c.txt".into(),
                area: Area::Conflicted
            }
        );
        assert_eq!(
            osier.diff_files, 1,
            "conflict view loaded from the working file"
        );
    });

    h.act(cx, |osier, window, cx| {
        osier.resolve_conflict_with("c.txt".into(), ConflictSide::Theirs, window, cx)
    });
    h.read(cx, |osier, _| {
        let snapshot = osier.active_project().unwrap().snapshot.as_ref().unwrap();
        assert_eq!(snapshot.status.conflicted().count(), 0);
        assert_eq!(osier.in_progress(), Some(InProgress::Merge));
    });
    h.act(cx, |osier, window, cx| {
        osier.continue_in_progress(window, cx)
    });
    h.read(cx, |osier, _| {
        assert_eq!(osier.in_progress(), None);
        let snapshot = osier.active_project().unwrap().snapshot.as_ref().unwrap();
        assert!(snapshot.status.is_clean());
        assert!(snapshot.commits[0].is_merge());
    });
    assert_eq!(repo.read("c.txt"), "theirs\n");

    // Compare the merged branch with its parent: one file.
    h.act(cx, |osier, _, cx| {
        osier.select_compare("main~1".into(), "main".into(), false, cx)
    });
    h.read(cx, |osier, _| assert_eq!(osier.diff_files, 1));
    h.read(cx, |osier, _| {
        assert!(
            osier
                .active_project()
                .unwrap()
                .snapshot
                .as_ref()
                .unwrap()
                .reflog
                .len()
                >= 3
        )
    });
}

#[gpui::test]
fn ui_mark_resolved_waits_for_markers_and_save_says_when_it_resolves(cx: &mut TestAppContext) {
    use osier_git::ops::integrate::MergeMode;

    let repo = TempRepo::new();
    let both = |content: &str, message: &str| {
        repo.write("c.txt", content);
        repo.write("d.txt", content);
        repo.git(["add", "c.txt", "d.txt"]);
        repo.commit(message);
    };
    both("base\n", "base");
    repo.checkout_new("feature");
    both("theirs\n", "theirs");
    repo.checkout("main");
    both("ours\n", "ours");
    let h = Harness::open(cx, &repo);
    h.act(cx, |o, window, cx| {
        o.merge_branch("feature".into(), MergeMode::Default, window, cx)
    });
    let conflicted = |h: &Harness, cx: &mut TestAppContext| {
        h.read(cx, |o, _| {
            o.active_project()
                .unwrap()
                .snapshot
                .as_ref()
                .unwrap()
                .status
                .conflicted()
                .map(|e| e.path.clone())
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(conflicted(&h, cx), ["c.txt", "d.txt"]);

    // Markers left: the button is disabled, a click records nothing.
    h.click(cx, "mark-resolved:c.txt");
    assert_eq!(conflicted(&h, cx).len(), 2);
    assert!(repo.read("c.txt").contains("<<<<<<<"), "markers untouched");
    // Called directly (the file may change after the frame was drawn), it refuses too.
    h.act(cx, |o, window, cx| {
        o.mark_resolved("c.txt".into(), window, cx)
    });
    assert_eq!(conflicted(&h, cx).len(), 2);
    // Its error toast (kept open by its Command log button) would sit over the editor's
    // buttons below.
    h.act(cx, |_, window, cx| {
        use gpui_kit::component::WindowExt as _;
        window.clear_notifications(cx)
    });
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(5));
    cx.run_until_parked();

    // Fixed in another editor: after the refresh the button marks it resolved.
    repo.write("d.txt", "fixed elsewhere\n");
    h.act(cx, |o, _, cx| o.refresh_active(cx));
    h.click(cx, "mark-resolved:d.txt");
    assert_eq!(conflicted(&h, cx), ["c.txt"]);

    // Osier's editor: "Save" while markers remain, "Save and resolve" once they are gone.
    h.act(cx, |o, _, cx| {
        o.select_file("c.txt".into(), Area::Conflicted, cx)
    });
    h.act(cx, |o, window, cx| o.open_editor(window, cx));
    assert_eq!(h.read(cx, |o, cx| o.save_button_text(cx).0), "Save");
    h.act(cx, |o, window, cx| {
        let state = o.editor.as_ref().unwrap().state.clone();
        state.update(cx, |s, cx| s.set_value("merged\n", window, cx));
    });
    assert_eq!(
        h.read(cx, |o, cx| o.save_button_text(cx).0),
        "Save and resolve"
    );
    h.click(cx, "editor-save");
    assert!(conflicted(&h, cx).is_empty());
    assert_eq!(repo.read("c.txt"), "merged\n");
}

/// A merge stopped on conflicts in `c.txt` (line 2 changed on both sides, line 6 only on
/// theirs) and, with `two`, in `d.txt` too. The diff pane is switched to 3-way through its
/// button and `c.txt` is selected.
fn three_way_merge(cx: &mut TestAppContext, two: bool) -> (TempRepo, Harness) {
    use osier_git::ops::integrate::MergeMode;
    let repo = TempRepo::new();
    let commit = |c: &str, d: &str, message: &str| {
        repo.write("c.txt", c);
        repo.write("d.txt", d);
        repo.git(["add", "c.txt", "d.txt"]);
        repo.commit(message);
    };
    commit("a\nb\nc\nd\ne\nf\n", "base\n", "base");
    repo.checkout_new("feature");
    commit(
        "a\nB-theirs\nc\nd\ne\nF\n",
        if two { "theirs\n" } else { "base\n" },
        "theirs",
    );
    repo.checkout("main");
    commit(
        "a\nB-ours\nc\nd\ne\nf\n",
        if two { "ours\n" } else { "base\n" },
        "ours",
    );
    let h = Harness::open(cx, &repo);
    h.act(cx, |o, window, cx| {
        o.merge_branch("feature".into(), MergeMode::Default, window, cx)
    });
    h.act(cx, |o, _, cx| {
        o.select_file("c.txt".into(), Area::Conflicted, cx)
    });
    h.click(cx, "three-way");
    // The versions load in the background; the gutter buttons appear once the panes
    // have been laid out.
    h.act(cx, |_, _, cx| cx.notify());
    h.act(cx, |_, _, cx| cx.notify());
    (repo, h)
}

fn merge_view(h: &Harness, cx: &mut TestAppContext) -> Entity<crate::merge_view::MergeView> {
    h.read(cx, |o, _| {
        o.merges
            .views
            .values()
            .next()
            .expect("a merge view")
            .view
            .clone()
    })
}

fn conflicted_files(h: &Harness, cx: &mut TestAppContext) -> Vec<String> {
    h.read(cx, |o, _| {
        o.active_project()
            .unwrap()
            .snapshot
            .as_ref()
            .unwrap()
            .status
            .conflicted()
            .map(|e| e.path.clone())
            .collect()
    })
}

#[gpui::test]
fn ui_three_way_apply_ignore_and_resolve(cx: &mut TestAppContext) {
    let (repo, h) = three_way_merge(cx, false);
    assert!(
        h.bounds(cx, "merge-result").is_some(),
        "the merge tool shows"
    );
    assert!(
        h.bounds(cx, "apply-resolution").is_none(),
        "inline controls are hidden"
    );
    let view = merge_view(&h, cx);
    assert_eq!(
        h.read(cx, |_, cx| view.read(cx).merge().pending_counts()),
        (1, 1)
    );
    // The result starts as the base.
    assert_eq!(
        h.read(cx, |_, cx| view.read(cx).result_text(cx)),
        "a\nb\nc\nd\ne\nf\n"
    );

    h.click(cx, "merge-apply-all");
    assert_eq!(
        h.read(cx, |_, cx| view.read(cx).result_text(cx)),
        "a\nb\nc\nd\ne\nF\n"
    );
    assert!(
        h.bounds(cx, "merge-chunk-apply:left:0").is_some(),
        "gutter buttons drawn"
    );
    h.click(cx, "merge-chunk-apply:left:0");
    h.click(cx, "merge-chunk-ignore:right:0");
    assert!(h.read(cx, |_, cx| view.read(cx).merge().is_resolved()));
    assert!(
        h.bounds(cx, "merge-chunk-apply:left:0").is_none(),
        "decided chunks lose them"
    );

    h.click(cx, "merge-apply");
    assert!(!h.has_settings(cx), "nothing undecided: no question");
    assert!(conflicted_files(&h, cx).is_empty());
    assert_eq!(repo.read("c.txt"), "a\nB-ours\nc\nd\ne\nF\n");
}

#[gpui::test]
fn ui_three_way_append_the_second_side_and_revert(cx: &mut TestAppContext) {
    let (_repo, h) = three_way_merge(cx, false);
    let view = merge_view(&h, cx);
    h.click(cx, "merge-chunk-apply:right:0");
    h.act(cx, |_, _, cx| cx.notify());
    h.click(cx, "merge-chunk-apply:left:0");
    assert_eq!(
        h.read(cx, |_, cx| view.read(cx).result_text(cx)),
        "a\nB-theirs\nB-ours\nc\nd\ne\nf\n",
        "the second side goes below the first"
    );
    h.click(cx, "merge-revert");
    h.click(cx, "dialog-ok");
    assert_eq!(
        h.read(cx, |_, cx| view.read(cx).result_text(cx)),
        "a\nb\nc\nd\ne\nf\n"
    );
    assert_eq!(
        h.read(cx, |_, cx| view.read(cx).merge().pending_counts()),
        (1, 1)
    );
}

#[gpui::test]
fn ui_three_way_apply_with_undecided_changes_asks_first(cx: &mut TestAppContext) {
    let (repo, h) = three_way_merge(cx, false);
    h.click(cx, "merge-apply");
    assert!(h.has_settings(cx), "asks before saving undecided changes");
    h.click(cx, "dialog-cancel");
    assert_eq!(conflicted_files(&h, cx), ["c.txt"]);
    // ctrl-enter in the result is Apply too.
    h.click(cx, "merge-result");
    h.keys(cx, "ctrl-enter");
    assert!(h.has_settings(cx), "ctrl-enter asks the same question");
    h.click(cx, "dialog-ok");
    assert!(conflicted_files(&h, cx).is_empty());
    assert_eq!(repo.read("c.txt"), "a\nb\nc\nd\ne\nf\n", "saved as shown");
}

#[gpui::test]
fn ui_three_way_accept_a_whole_side_and_typed_edits_count(cx: &mut TestAppContext) {
    let (repo, h) = three_way_merge(cx, false);
    let view = merge_view(&h, cx);
    // Typing into the result moves the chunks with it.
    h.act(cx, |_, window, cx| {
        let editor = view.read(cx).result_editor().clone();
        editor.update(cx, |s, cx| {
            s.set_selected_range(0..0, cx);
            s.replace("top\n", window, cx);
        });
    });
    h.read(cx, |_, cx| {
        let v = view.read(cx);
        assert_eq!(v.merge().chunks()[0].result, 2..3);
        assert!(v.is_modified());
    });
    h.click(cx, "merge-accept-right");
    assert!(conflicted_files(&h, cx).is_empty());
    assert_eq!(repo.read("c.txt"), "a\nB-theirs\nc\nd\ne\nF\n");
}

#[gpui::test]
fn ui_three_way_expand_and_walk_the_conflicted_files(cx: &mut TestAppContext) {
    let (_repo, h) = three_way_merge(cx, true);
    assert!(h.bounds(cx, "center-pane").is_some());
    h.click(cx, "diff-expand");
    assert!(h.bounds(cx, "center-pane").is_none(), "file list hidden");
    assert!(h.bounds(cx, "merge-result").is_some());
    h.click(cx, "merge-next-file");
    h.act(cx, |_, _, cx| cx.notify());
    let selected = h.read(cx, |o, _| o.active_project().unwrap().selection.clone());
    assert_eq!(
        selected,
        Selection::File {
            path: "d.txt".into(),
            area: Area::Conflicted
        }
    );
    assert!(
        h.bounds(cx, "merge-result").is_some(),
        "still the merge tool, still expanded"
    );
    assert!(h.bounds(cx, "center-pane").is_none());
    // Resolving the last but one file moves on to the next; the progress made on another
    // file is kept meanwhile.
    h.click(cx, "merge-accept-left");
    h.act(cx, |_, _, cx| cx.notify());
    assert_eq!(conflicted_files(&h, cx), ["c.txt"]);
    let selected = h.read(cx, |o, _| o.active_project().unwrap().selection.clone());
    assert_eq!(
        selected,
        Selection::File {
            path: "c.txt".into(),
            area: Area::Conflicted
        }
    );
    // Escape from the result editor goes back to the normal layout.
    h.click(cx, "merge-result");
    h.keys(cx, "escape");
    assert!(
        h.bounds(cx, "center-pane").is_some(),
        "back to the normal layout"
    );
}

#[gpui::test]
fn ui_conflicts_default_to_three_way_and_other_files_keep_their_layout(cx: &mut TestAppContext) {
    use osier_config::settings::{ConflictView, DiffMode as Mode};
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "one\n", "init");
    repo.write("a.txt", "two\n");
    let h = Harness::open(cx, &repo);
    h.read(cx, |o, _| {
        assert_eq!(o.settings.diff.conflicts, ConflictView::ThreeWay);
        assert_eq!(o.settings.diff.mode, Mode::Unified);
    });
    h.act(cx, |o, _, cx| {
        o.select_file("a.txt".into(), Area::Changed, cx)
    });
    // 3-way is only for conflicts: disabled here, so a click changes nothing.
    h.click(cx, "three-way");
    h.click(cx, "split");
    h.read(cx, |o, cx| {
        assert_eq!(o.settings.diff.mode, Mode::Split);
        assert_eq!(o.settings.diff.conflicts, ConflictView::ThreeWay);
        assert_eq!(
            o.diff.read(cx).mode(),
            gpui_kit::component::diff::DiffMode::Split
        );
        assert!(o.three_way_target().is_none());
    });
    assert!(h.bounds(cx, "merge-result").is_none());
}

#[gpui::test]
fn ui_conflicts_inline_split_is_off_and_expand_survives_switching(cx: &mut TestAppContext) {
    use osier_config::settings::{ConflictView, DiffMode as Mode};
    let (_repo, h) = three_way_merge(cx, false);
    h.click(cx, "diff-expand");
    assert!(h.bounds(cx, "center-pane").is_none());
    // Unified on a conflicted file switches to the inline blocks, still expanded.
    h.click(cx, "unified");
    h.read(cx, |o, _| {
        assert_eq!(o.settings.diff.conflicts, ConflictView::Inline);
        assert_eq!(
            o.settings.diff.mode,
            Mode::Unified,
            "normal files untouched"
        );
    });
    assert!(h.bounds(cx, "merge-result").is_none());
    assert!(
        h.bounds(cx, "apply-resolution").is_some(),
        "inline controls"
    );
    assert!(h.bounds(cx, "center-pane").is_none(), "still expanded");
    // Split has no conflict view: disabled.
    h.click(cx, "split");
    h.read(cx, |o, _| assert_eq!(o.settings.diff.mode, Mode::Unified));
    h.click(cx, "three-way");
    assert!(h.bounds(cx, "merge-result").is_some());
    assert!(h.bounds(cx, "center-pane").is_none(), "still expanded");
    h.click(cx, "diff-expand");
    assert!(h.bounds(cx, "center-pane").is_some());
}

#[gpui::test]
fn ui_a_conflicted_file_without_markers_says_so(cx: &mut TestAppContext) {
    let (repo, h) = three_way_merge(cx, false);
    // Saved from the merge tool: the file has no markers but is still conflicted.
    h.click(cx, "merge-save");
    assert!(!repo.read("c.txt").contains("<<<<<<<"));
    assert_eq!(conflicted_files(&h, cx), ["c.txt"]);
    h.click(cx, "unified");
    assert!(h.read(cx, |o, _| o.diff_no_markers));
    assert!(h.bounds(cx, "no-markers-three-way").is_some());
    h.click(cx, "no-markers-resolve");
    assert!(conflicted_files(&h, cx).is_empty());
}

#[gpui::test]
fn ui_three_way_wraps_with_the_wrap_button(cx: &mut TestAppContext) {
    let (_repo, h) = three_way_merge(cx, false);
    let view = merge_view(&h, cx);
    let wrap = h.read(cx, |o, _| o.settings.diff.soft_wrap);
    assert_eq!(h.read(cx, |_, cx| view.read(cx).soft_wrap()), wrap);
    h.click(cx, "wrap");
    assert_eq!(h.read(cx, |_, cx| view.read(cx).soft_wrap()), !wrap);
    // The gutter buttons are still drawn with wrapping changed.
    h.act(cx, |_, _, cx| cx.notify());
    assert!(h.bounds(cx, "merge-chunk-apply:left:0").is_some());
}

#[gpui::test]
fn ui_diff_settings_are_shared_with_the_merge_tool(cx: &mut TestAppContext) {
    let (_repo, h) = three_way_merge(cx, false);
    let view = merge_view(&h, cx);
    h.act(cx, |o, window, cx| {
        let mut opts = o.diff_options();
        opts.highlight_words = false;
        opts.sync_scroll = false;
        o.set_diff_options(opts, window, cx)
    });
    h.read(cx, |o, cx| {
        let v = view.read(cx).options();
        assert!(!v.highlight_words && !v.sync_scroll);
        assert!(!o.settings.diff.word_highlights);
        assert_eq!(o.diff.read(cx).inline_unit(), None);
    });
}

#[gpui::test]
fn ui_three_way_scrolling_one_pane_is_not_pulled_back_by_a_focused_one(cx: &mut TestAppContext) {
    use osier_git::ops::integrate::MergeMode;
    // Both sides dropped a long header the base still has: the result is much longer
    // above, so the side panes get a gap while the result scrolls.
    let repo = TempRepo::new();
    let body: String = (1..=30).map(|i| format!("line {i}\n")).collect();
    let header: String = (1..=60).map(|i| format!("header {i}\n")).collect();
    repo.commit_file("c.txt", &format!("{header}{body}"), "base");
    repo.checkout_new("feature");
    repo.commit_file("c.txt", &body.replace("line 20\n", "theirs\n"), "theirs");
    repo.checkout("main");
    repo.commit_file("c.txt", &body.replace("line 20\n", "ours\n"), "ours");
    let h = Harness::open(cx, &repo);
    h.act(cx, |o, window, cx| {
        o.merge_branch("feature".into(), MergeMode::Default, window, cx)
    });
    h.act(cx, |o, _, cx| {
        o.select_file("c.txt".into(), Area::Conflicted, cx)
    });
    h.act(cx, |_, _, cx| cx.notify());
    h.act(cx, |_, _, cx| cx.notify());
    let view = merge_view(&h, cx);
    // The caret sits in the right pane, as after clicking into it.
    h.click(cx, "merge-right");
    let focused = cx
        .update_window(h.window, |_, window, cx| {
            view.read(cx).editors_focus(window, cx)
        })
        .unwrap();
    assert_eq!(focused, [false, false, true]);
    let result = h.bounds(cx, "merge-result").expect("result pane");
    let scroll = |h: &Harness, cx: &mut TestAppContext, dy: f32| {
        let mut v = h.visual(cx);
        v.simulate_event(gpui::ScrollEvent {
            position: result.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(dy))),
            ..Default::default()
        });
        cx.run_until_parked();
        h.act(cx, |_, _, cx| cx.notify());
        // Let the focused pane's caret blink: it redraws on its own.
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(1200));
        cx.run_until_parked();
        h.act(cx, |_, _, cx| cx.notify());
    };
    let result_y = |h: &Harness, cx: &mut TestAppContext| {
        h.read(cx, |_, cx| {
            f32::from(view.read(cx).result_editor().read(cx).scroll_offset().y)
        })
    };
    scroll(&h, cx, -400.);
    let down = result_y(&h, cx);
    assert!(down < -300., "scrolled down: {down}");
    scroll(&h, cx, 150.);
    let up = result_y(&h, cx);
    assert!(
        (up - (down + 150.)).abs() < 2.,
        "scrolling up stays where the user put it: {down} then {up}"
    );
}
