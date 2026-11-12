//! The Log tab: shift-click ranges, squash, commit details, row menus, blame and file history,
//! tags.

use super::*;

#[gpui::test]
fn ui_log_time_shows_the_full_date_on_hover(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    let h = Harness::open(cx, &repo);
    h.act(cx, |o, _, cx| o.set_tab(Tab::Log, cx));
    let at = h
        .bounds(cx, "commit-time:0")
        .expect("time in the log row")
        .center();
    assert!(h.bounds(cx, "time-tooltip").is_none());
    // The configured delay applies, not gpui's fixed 500 ms.
    h.act(cx, |o, _, cx| {
        o.settings.appearance.tooltip_delay_ms = 1000;
        o.apply_tooltip_delay(cx);
        cx.notify();
    });
    h.visual(cx)
        .simulate_mouse_move(at, None, gpui::Modifiers::none());
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(700));
    cx.run_until_parked();
    assert!(
        h.bounds(cx, "time-tooltip").is_none(),
        "still waiting at 700 ms"
    );
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(400));
    cx.run_until_parked();
    assert!(
        h.bounds(cx, "time-tooltip").is_some(),
        "tooltip after the delay"
    );
    let committed = h.read(cx, |o, _| {
        o.active_project()
            .and_then(|p| p.snapshot.as_ref())
            .map(|s| s.commits[0].committed_at)
            .unwrap_or_default()
    });
    assert!(crate::format::full_time(committed).contains(", "));
}

#[gpui::test]
fn log_badges_sit_before_the_subject_or_at_the_end(cx: &mut TestAppContext) {
    use osier_config::settings::LogBadges;
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    repo.git(["tag", "v1"]);
    let h = Harness::open(cx, &repo);
    h.act(cx, |o, _, cx| o.set_tab(Tab::Log, cx));
    let badge = |h: &Harness, cx: &mut TestAppContext| {
        h.bounds(cx, "log-ref:0:v1").expect("tag badge").origin.x
    };
    let subject = |h: &Harness, cx: &mut TestAppContext| {
        h.bounds(cx, "log-subject:0").expect("subject").origin.x
    };
    assert!(badge(&h, cx) < subject(&h, cx), "left by default");
    h.act(cx, |o, _, cx| {
        o.settings.git.log_badges = LogBadges::Right;
        cx.notify();
    });
    assert!(badge(&h, cx) > subject(&h, cx), "right when set");
}

#[gpui::test]
fn a_long_subject_stays_inside_its_row_in_every_text_ending(cx: &mut TestAppContext) {
    use osier_config::settings::{LogBadges, TextOverflow};
    let repo = TempRepo::new();
    let long = "a commit subject far too long for the narrow center column ".repeat(4);
    repo.commit_file("a.txt", "a\n", long.trim());
    repo.git(["tag", "v1"]);
    repo.commit_file("b.txt", "b\n", long.trim());
    let mut state = State::default();
    state.layout.center_width = Some(440.);
    let h = Harness::open_with(cx, state, Some(repo.path().to_path_buf()));
    h.act(cx, |o, _, cx| o.set_tab(Tab::Log, cx));
    for style in [
        TextOverflow::Fade,
        TextOverflow::Ellipsis,
        TextOverflow::Both,
    ] {
        for badges in [LogBadges::Left, LogBadges::Right] {
            h.act(cx, |o, _, cx| {
                o.settings.appearance.text_overflow = style;
                o.settings.git.log_badges = badges;
                cx.notify();
            });
            let pane = h.bounds(cx, "center-pane").expect("center pane");
            // Row 0 has no badge, row 1 the tag.
            for row in [0, 1] {
                let id: &'static str = Box::leak(format!("log-subject:{row}").into_boxed_str());
                let subject = h.bounds(cx, id).expect("subject");
                assert!(
                    subject.right() <= pane.right() && subject.size.width > gpui::px(40.),
                    "{style:?}, badges {badges:?}, row {row}: subject {subject:?} in {pane:?}"
                );
            }
        }
    }
}

#[gpui::test]
fn ui_log_badges_put_local_branches_first_and_count_the_rest(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    repo.add_bare_origin();
    repo.git(["fetch", "-q", "origin"]);
    repo.git(["remote", "set-head", "origin", "main"]);
    repo.branch("feat/a");
    repo.branch("feat/b");
    repo.git(["tag", "v1"]);
    let h = Harness::open(cx, &repo);
    h.act(cx, |o, _, cx| o.set_tab(Tab::Log, cx));
    // Local branches first, the checked-out one leading, the others by name.
    let x = |h: &Harness, cx: &mut TestAppContext, id: &'static str| {
        h.bounds(cx, id).unwrap_or_else(|| panic!("{id}")).origin.x
    };
    let (main, a, b) = (
        x(&h, cx, "log-ref:0:main"),
        x(&h, cx, "log-ref:0:feat/a"),
        x(&h, cx, "log-ref:0:feat/b"),
    );
    assert!(main < a && a < b, "{main:?} {a:?} {b:?}");
    // The tag and origin/main do not fit; origin/HEAD says nothing origin/main does not.
    for id in [
        "log-ref:0:v1",
        "log-ref:0:origin/main",
        "log-ref:0:origin/HEAD",
    ] {
        assert!(h.bounds(cx, id).is_none(), "{id}");
    }
    // Its visible left edge: in this narrow window the row's end fade lies over the rest.
    let more = h.bounds(cx, "log-ref-more:0").expect("+2 badge");
    let more = gpui::point(more.origin.x + gpui::px(3.), more.center().y);
    h.visual(cx)
        .simulate_mouse_move(more, None, gpui::Modifiers::none());
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(2));
    cx.run_until_parked();
    assert!(h.bounds(cx, "log-ref-more-tooltip").is_some());
}

fn log_subjects(h: &Harness, cx: &mut TestAppContext) -> Vec<String> {
    h.read(cx, |o, _| {
        o.active_project()
            .and_then(|p| p.snapshot.as_ref())
            .map(|s| s.commits.iter().map(|c| c.subject.clone()).collect())
            .unwrap_or_default()
    })
}

#[gpui::test]
fn shift_click_pair_stays_on_its_commits_when_new_ones_arrive(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    for s in ["a", "b", "c", "d"] {
        repo.commit_file(&format!("{s}.txt"), "x\n", s);
    }
    let h = Harness::open(cx, &repo);
    h.act(cx, |o, _, cx| o.set_tab(Tab::Log, cx));
    h.click(cx, "log-row:1");
    h.click_with(cx, "log-row:2", gpui::Modifiers::shift());
    let picked = |h: &Harness, cx: &mut TestAppContext| {
        h.read(cx, |o, _| {
            let p = o.active_project().unwrap();
            let hash = |ix: Option<usize>| p.commit_at(ix.unwrap()).unwrap().subject.clone();
            (hash(p.selected_commit), hash(p.compare_commit))
        })
    };
    assert_eq!(picked(&h, cx), ("c".to_string(), "b".to_string()));
    // A commit made outside Osier lands on top.
    repo.commit_file("e.txt", "x\n", "e");
    h.act(cx, |o, _, cx| o.refresh_project(0, cx));
    assert_eq!(log_subjects(&h, cx)[0], "e");
    assert_eq!(
        picked(&h, cx),
        ("c".to_string(), "b".to_string()),
        "same commits"
    );
}

#[gpui::test]
fn shift_click_range_of_several_stays_on_its_commits(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    for s in ["a", "b", "c", "d", "e"] {
        repo.commit_file(&format!("{s}.txt"), "x\n", s);
    }
    let h = Harness::open(cx, &repo);
    h.act(cx, |o, _, cx| o.set_tab(Tab::Log, cx));
    // Rows 1 to 3: d, c, b.
    h.click(cx, "log-row:1");
    h.click_with(cx, "log-row:3", gpui::Modifiers::shift());
    let run = |h: &Harness, cx: &mut TestAppContext| {
        h.read(cx, |o, _| {
            let p = o.active_project().unwrap();
            let mut rows: Vec<usize> = p.picked_rows().into_iter().collect();
            rows.sort_unstable();
            rows.into_iter()
                .map(|ix| p.commit_at(ix).unwrap().subject.clone())
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(run(&h, cx), ["d", "c", "b"]);
    for s in ["f", "g"] {
        repo.commit_file(&format!("{s}.txt"), "x\n", s);
    }
    h.act(cx, |o, _, cx| o.refresh_project(0, cx));
    assert_eq!(log_subjects(&h, cx)[..2], ["g", "f"]);
    assert_eq!(
        run(&h, cx),
        ["d", "c", "b"],
        "the same three commits, two rows lower"
    );
}

#[gpui::test]
fn ui_squash_a_range_with_another_branch_between(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "x\n", "base");
    // Alternate commits on main and side, so date order interleaves them.
    repo.git(["branch", "side"]);
    for (branch, s) in [
        ("side", "s1"),
        ("main", "m1"),
        ("side", "s2"),
        ("main", "m2"),
    ] {
        repo.git(["switch", "-q", branch]);
        repo.commit_file(&format!("{s}.txt"), "x\n", s);
    }
    repo.git(["switch", "-q", "main"]);
    let h = Harness::open(cx, &repo);
    h.act(cx, |o, _, cx| o.set_tab(Tab::Log, cx));
    assert_eq!(log_subjects(&h, cx), ["m2", "s2", "m1", "s1", "base"]);
    h.click(cx, "log-row:0");
    h.click_with(cx, "log-row:2", gpui::Modifiers::shift());
    let picked = h.read(cx, |o, _| {
        let p = o.active_project().unwrap();
        let mut rows: Vec<usize> = p.picked_rows().into_iter().collect();
        rows.sort_unstable();
        rows
    });
    assert_eq!(picked, [0, 2], "s2 between them is not picked");
    h.act(cx, |o, w, cx| o.confirm_squash(w, cx));
    assert!(h.has_settings(cx), "squash dialog open");
    h.click(cx, "dialog-ok");
    assert_eq!(repo.git(["rev-list", "--count", "main"]).trim(), "2");
    assert_eq!(repo.git(["rev-list", "--count", "side"]).trim(), "3");
}

#[gpui::test]
fn ui_squash_a_shift_click_range(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    for s in ["a", "b", "c", "d"] {
        repo.commit_file(&format!("{s}.txt"), "x\n", s);
    }
    let h = Harness::open(cx, &repo);
    h.act(cx, |o, _, cx| o.set_tab(Tab::Log, cx));
    // Newest to oldest: d, c, b.
    h.click(cx, "log-row:0");
    h.click_with(cx, "log-row:2", gpui::Modifiers::shift());
    h.read(cx, |o, _| {
        assert_eq!(o.active_project().unwrap().picked.len(), 3)
    });
    h.act(cx, |o, w, cx| o.confirm_squash(w, cx));
    assert!(h.has_settings(cx), "squash dialog open");
    assert!(h.bounds(cx, "squash-message").is_some());
    h.click(cx, "dialog-ok");
    assert_eq!(
        log_subjects(&h, cx),
        ["b\n\nc\n\nd".lines().next().unwrap(), "a"]
    );
    h.read(cx, |o, _| {
        let p = o.active_project().unwrap();
        let head = &p.snapshot.as_ref().unwrap().commits[0];
        assert_eq!(head.body.trim(), "c\n\nd", "messages kept, oldest first");
        assert_eq!(
            p.selected_commit,
            Some(0),
            "the squashed commit is selected"
        );
        assert_eq!(p.compare_commit, None);
    });
}

#[gpui::test]
fn squash_of_commits_on_different_lines_is_refused(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "x\n", "a");
    repo.git(["checkout", "-qb", "side"]);
    repo.commit_file("s.txt", "x\n", "side");
    repo.git(["checkout", "-q", "main"]);
    repo.commit_file("b.txt", "x\n", "b");
    let h = Harness::open(cx, &repo);
    h.act(cx, |o, _, cx| o.set_tab(Tab::Log, cx));
    // b and side: rows 0 and 1, picked with ctrl-click; side is on another branch.
    h.click(cx, "log-row:0");
    h.click_with(cx, "log-row:1", gpui::Modifiers::control());
    h.read(cx, |o, _| {
        assert_eq!(o.active_project().unwrap().picked.len(), 2)
    });
    h.act(cx, |o, w, cx| o.confirm_squash(w, cx));
    assert!(!h.has_settings(cx), "no dialog");
    assert_eq!(log_subjects(&h, cx).len(), 3, "nothing changed");
}

#[gpui::test]
fn ui_ctrl_click_picks_separate_commits_and_squashes_them(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    for s in ["a", "b", "c", "d", "e"] {
        repo.commit_file(&format!("{s}.txt"), "x\n", s);
    }
    let h = Harness::open(cx, &repo);
    h.act(cx, |o, _, cx| o.set_tab(Tab::Log, cx));
    // Rows: e d c b a. Pick d and b (c between), then toggle a in and out again.
    h.click(cx, "log-row:1");
    h.click_with(cx, "log-row:3", gpui::Modifiers::control());
    h.click_with(cx, "log-row:4", gpui::Modifiers::control());
    h.click_with(cx, "log-row:4", gpui::Modifiers::control());
    h.read(cx, |o, _| {
        let p = o.active_project().unwrap();
        let mut rows: Vec<usize> = p.picked_rows().into_iter().collect();
        rows.sort_unstable();
        assert_eq!(rows, [1, 3], "d and b, not c between them");
    });
    // New commits do not move the picks.
    repo.commit_file("f.txt", "x\n", "f");
    h.act(cx, |o, _, cx| o.refresh_project(0, cx));
    h.read(cx, |o, _| {
        let p = o.active_project().unwrap();
        let mut rows: Vec<usize> = p.picked_rows().into_iter().collect();
        rows.sort_unstable();
        assert_eq!(rows, [2, 4]);
    });
    h.act(cx, |o, w, cx| o.confirm_squash(w, cx));
    assert!(h.has_settings(cx), "squash dialog open");
    h.click(cx, "dialog-ok");
    assert_eq!(log_subjects(&h, cx), ["f", "e", "c", "b", "a"]);
    h.read(cx, |o, _| {
        let s = o.active_project().unwrap().snapshot.as_ref().unwrap();
        assert_eq!(
            s.commits[3].body.trim(),
            "d",
            "b's subject, d's message below"
        );
    });
}

#[gpui::test]
fn snapshot_knows_which_commits_are_not_pushed(cx: &mut TestAppContext) {
    let repo = repo_with_upstream(2, false);
    let h = Harness::open(cx, &repo);
    h.read(cx, |o, _| {
        let s = o.active_project().unwrap().snapshot.as_ref().unwrap();
        let marked: Vec<&str> = s
            .commits
            .iter()
            .filter(|c| s.unpushed.contains(&c.hash))
            .map(|c| c.subject.as_str())
            .collect();
        assert_eq!(marked, ["local 1", "local 0"]);
    });
}

#[gpui::test]
fn ui_commit_details_panel_shows_toggles_and_remembers(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    repo.commit_file("b.txt", "b\n", "second");
    let h = Harness::open(cx, &repo);
    h.click(cx, "tab-log");
    assert!(h.bounds(cx, "commit-details").is_some(), "shown by default");
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
    h.read(cx, |o, _| {
        assert_eq!(
            o.commit_branches.get(&latest).map(Vec::as_slice),
            Some(&["main".to_string()][..])
        )
    });
    // The header button hides it, the key brings it back; the choice is saved.
    h.click(cx, "toggle-commit-details");
    assert!(h.bounds(cx, "commit-details").is_none());
    h.read(cx, |o, _| assert!(o.state.layout.log_details_hidden));
    h.keys(cx, "ctrl-shift-d");
    assert!(h.bounds(cx, "commit-details").is_some());
    h.read(cx, |o, _| assert!(!o.state.layout.log_details_hidden));
}

#[gpui::test]
fn ui_commit_details_list_both_dates_only_when_they_differ(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    let h = Harness::open(cx, &repo);
    h.click(cx, "tab-log");
    // A plain commit: one "Date".
    assert!(h.bounds(cx, "commit-date:Date").is_some());
    assert!(h.bounds(cx, "commit-date:Authored").is_none());
    assert!(h.bounds(cx, "commit-date:Committed").is_none());

    // Amended a few seconds later: the committer date moves, the author date stays.
    repo.runner()
        .with_env("GIT_COMMITTER_DATE", "2024-01-02T12:00:03+00:00")
        .run(Some(repo.path()), ["commit", "-q", "--amend", "--no-edit"])
        .expect("amend");
    h.act(cx, |o, _, cx| o.refresh_active(cx));
    h.read(cx, |o, _| {
        let c = &o
            .active_project()
            .unwrap()
            .snapshot
            .as_ref()
            .unwrap()
            .commits[0];
        assert_eq!(c.committed_at - c.authored_at, 3);
    });
    // The amend replaced the selected commit: pick the new one.
    h.click(cx, "log-row:0");
    assert!(h.bounds(cx, "commit-date:Date").is_none());
    assert!(h.bounds(cx, "commit-date:Authored").is_some());
    assert!(h.bounds(cx, "commit-date:Committed").is_some());
}

#[gpui::test]
fn squash_over_uncommitted_changes_stashes_them_while_it_runs(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    for f in ["a", "b", "c", "d"] {
        repo.commit_file(&format!("{f}.txt"), &format!("{f}\n"), f);
    }
    repo.write("a.txt", "edited\n");
    let h = Harness::open(cx, &repo);
    // Squashing HEAD~2 and HEAD~1 replays HEAD over the uncommitted edit.
    let picked: Vec<String> = ["HEAD~1", "HEAD~2"]
        .iter()
        .map(|r| repo.git(["rev-parse", r]).trim().to_string())
        .collect();
    h.act(cx, move |o, w, cx| {
        o.active_project_mut().expect("project").picked = picked;
        o.confirm_squash(w, cx);
    });
    h.click(cx, "dialog-ok");
    assert_eq!(repo.git(["rev-list", "--count", "HEAD"]).trim(), "3");
    assert_eq!(repo.read("a.txt"), "edited\n", "the edit is put back");
    assert!(repo.git(["stash", "list"]).trim().is_empty());
}

#[gpui::test]
fn ui_right_click_opens_the_row_menu_and_buttons_follow_the_setting(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "one\n", "first");
    repo.git(["branch", "feature"]);
    let h = Harness::open(cx, &repo);
    h.click(cx, "tab-branches");
    let button = "branch-menu:feature";
    // The buttons live in the row's hover group, so hover the row before looking.
    let visible = |h: &Harness, cx: &mut TestAppContext| {
        let row = h.bounds(cx, "branch-row:feature").expect("row");
        h.visual(cx)
            .simulate_mouse_move(row.center(), None, gpui::Modifiers::none());
        cx.run_until_parked();
        h.bounds(cx, button)
            .is_some_and(|b| b.size.width > gpui::px(0.))
    };
    assert!(!visible(&h, cx), "menu buttons are off by default");

    // Right-click, first entry: Checkout (the hover button's action).
    h.right_click(cx, "branch-row:feature");
    h.keys(cx, "down enter");
    h.read(cx, |o, _| {
        let head = o
            .active_project()
            .and_then(|p| p.snapshot.as_ref())
            .and_then(|s| s.status.branch.head.clone());
        assert_eq!(head.as_deref(), Some("feature"));
    });

    h.act(cx, |o, _, cx| {
        o.settings.appearance.row_menu_buttons = true;
        cx.notify();
    });
    assert!(visible(&h, cx), "the setting shows them");
}

#[gpui::test]
fn ui_shift_click_compares_two_commits(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "one\n", "first");
    repo.commit_file("a.txt", "two\n", "second");
    let h = Harness::open(cx, &repo);
    h.click(cx, "tab-log");
    h.click(cx, "log-row:0");
    h.click_with(
        cx,
        "log-row:1",
        gpui::Modifiers {
            shift: true,
            ..Default::default()
        },
    );
    h.read(cx, |osier, _| {
        assert!(matches!(
            osier.active_project().unwrap().selection,
            Selection::Compare { .. }
        ));
    });
}

#[gpui::test]
fn blame_and_file_history(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "one\n", "first");
    repo.commit_file("b.txt", "bee\n", "second");
    repo.commit_file("a.txt", "one\ntwo\n", "third");
    let h = Harness::open(cx, &repo);

    h.act(cx, |osier, _, cx| {
        osier.show_blame("a.txt".into(), None, cx)
    });
    cx.run_until_parked();
    h.read(cx, |osier, _| {
        let view = osier.blame.as_ref().expect("blame view");
        let lines = view.lines.as_ref().expect("blame loaded");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].summary, "first");
        assert_eq!(lines[1].summary, "third");
        assert_ne!(lines[0].hash, lines[1].hash);
    });
    // Selecting something else closes the blame.
    h.act(cx, |osier, _, cx| osier.select_commit(0, cx));
    h.read(cx, |osier, _| assert!(osier.blame.is_none()));

    h.act(cx, |osier, _, cx| {
        osier.show_file_history("b.txt".into(), cx)
    });
    cx.run_until_parked();
    h.read(cx, |osier, _| {
        assert_eq!(osier.tab, Tab::Log);
        let project = osier.active_project().expect("project");
        assert_eq!(project.log_path.as_deref(), Some("b.txt"));
        let commits = &project.snapshot.as_ref().expect("snapshot").commits;
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].subject, "second");
    });
    h.act(cx, |osier, _, cx| osier.clear_file_history(cx));
    cx.run_until_parked();
    h.read(cx, |osier, _| {
        let project = osier.active_project().expect("project");
        assert!(project.log_path.is_none());
        assert_eq!(
            project.snapshot.as_ref().expect("snapshot").commits.len(),
            3
        );
    });
}

#[gpui::test]
fn tags_compare_picks_and_view_at_head(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "one\n", "first");
    repo.commit_file("a.txt", "two\n", "second");
    repo.git(["tag", "-a", "-m", "release", "v1"]);
    repo.write("a.txt", "three\n");
    let h = Harness::open(cx, &repo);

    h.read(cx, |osier, _| {
        let tags = &osier
            .active_project()
            .unwrap()
            .snapshot
            .as_ref()
            .unwrap()
            .tags;
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].name, "v1");
        assert!(tags[0].annotated);
    });

    // Select the newest commit, then compare with the older one: older is the base.
    h.act(cx, |osier, _, cx| osier.select_commit(0, cx));
    h.act(cx, |osier, _, cx| osier.compare_with_selected(1, cx));
    h.read(cx, |osier, _| {
        let p = osier.active_project().unwrap();
        let commits = &p.snapshot.as_ref().unwrap().commits;
        assert_eq!(p.compare_commit, Some(1));
        assert_eq!(
            p.selection,
            Selection::Compare {
                from: commits[1].hash.clone(),
                to: commits[0].hash.clone(),
                since_merge_base: false,
            }
        );
    });
    // A plain click clears the compare partner.
    h.act(cx, |osier, _, cx| osier.select_commit(1, cx));
    h.read(cx, |osier, _| {
        assert_eq!(osier.active_project().unwrap().compare_commit, None)
    });

    // View at HEAD shows the committed text, not the working tree.
    h.act(cx, |osier, _, cx| {
        osier.select_work_file("a.txt".into(), Some("HEAD".into()), cx)
    });
    h.read(cx, |osier, _| match &osier.viewer {
        Some(crate::viewer::Viewer::Code { text, .. }) => assert_eq!(text.as_ref(), "two\n"),
        other => panic!("unexpected viewer {other:?}"),
    });
}

#[gpui::test]
fn ui_reflog_lists_entries_and_builds_only_the_rows_on_screen(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    for i in 0..120 {
        repo.commit_file("a.txt", &format!("{i}\n"), &format!("commit {i}"));
    }
    let h = Harness::open(cx, &repo);
    h.click(cx, "tab-log");
    h.click(cx, "toggle-reflog");
    assert!(h.bounds(cx, "reflog-row:0").is_some(), "newest entry drawn");
    assert!(
        h.bounds(cx, "reflog-row:119").is_none(),
        "entries below the fold are not built"
    );
    h.click(cx, "close-reflog");
    assert!(h.bounds(cx, "reflog-row:0").is_none());
    assert!(h.bounds(cx, "log-row:0").is_some());
}
