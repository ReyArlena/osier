//! Opening, switching and closing projects, startup, the sidebar filter, pins, scan directories
//! and the git config panel.

use super::*;

#[gpui::test]
fn ui_filter_suggestions_insert_is_terms_and_filter_by_state(cx: &mut TestAppContext) {
    use osier_config::settings::RecentStatusMode;
    let name = |t: &TempRepo| {
        t.path()
            .file_name()
            .expect("name")
            .to_string_lossy()
            .into_owned()
    };
    let ahead = repo_with_upstream(1, false);
    let clean = repo_with_upstream(0, false);
    let recent_ahead = repo_with_upstream(2, false);
    let h = Harness::open(cx, &ahead);
    h.act(cx, |osier, _, cx| {
        osier.open_project(clean.path(), false, cx).expect("open");
        osier.state.recent.push(recent_ahead.path().to_path_buf());
    });
    let shown = |h: &Harness, cx: &mut TestAppContext, id: String| h.bounds(cx, &id).is_some();

    // Focus shows every qualifier, plus the note while recent status is off.
    h.keys(cx, "/");
    assert!(shown(&h, cx, "filter-suggestions".into()));
    for term in ["is:changed", "is:ahead", "is:behind"] {
        assert!(shown(&h, cx, format!("filter-suggestion:{term}")), "{term}");
    }
    assert!(shown(&h, cx, "filter-open-settings".into()));

    // Clicking a suggestion inserts it, keeps focus, and filters open projects by state.
    h.click(cx, "filter-suggestion:is:ahead");
    assert_eq!(input_text(&h, cx, |o| &o.sidebar_filter), "is:ahead ");
    assert!(focused_is(&h, cx, |o, cx| {
        gpui_kit::Focusable::focus_handle(o.sidebar_filter.read(cx), cx)
    }));
    assert!(shown(&h, cx, format!("sidebar-open:{}", name(&ahead))));
    assert!(!shown(&h, cx, format!("sidebar-open:{}", name(&clean))));
    assert!(
        !shown(&h, cx, format!("sidebar-recent:{}", name(&recent_ahead))),
        "recent projects are unknown while recent status is off"
    );
    assert!(
        !shown(&h, cx, "filter-suggestion:is:ahead".into()),
        "used terms drop out"
    );

    // The note's link opens settings on the right page, even after a normal open.
    h.keys(cx, "escape escape ctrl-,");
    assert!(h.has_settings(cx));
    assert!(
        !shown(&h, cx, "scan-recent-now".into()),
        "normal open starts elsewhere"
    );
    h.keys(cx, "escape /");
    h.click(cx, "filter-open-settings");
    assert!(h.has_settings(cx));
    assert!(
        shown(&h, cx, "scan-recent-now".into()),
        "settings open on the Projects page"
    );
    assert_eq!(
        h.read(cx, |o, _| o.settings_highlight.map(|(t, _)| t)),
        Some(crate::settings_panel::SettingsTarget::RECENT_STATUS)
    );
    h.keys(cx, "escape");
    h.keys(cx, "/");
    h.type_text(cx, "is:ahead ");

    // With status checks on, recent projects match by their badges and the note goes.
    set_recent_mode(&h, cx, RecentStatusMode::Check);
    drain_recent(&h, cx);
    assert!(shown(
        &h,
        cx,
        format!("sidebar-recent:{}", name(&recent_ahead))
    ));
    assert!(!shown(&h, cx, "filter-open-settings".into()));

    // Enter completes a half-typed qualifier; is:behind matches nothing here.
    h.type_text(cx, "is:b");
    h.keys(cx, "enter");
    assert_eq!(
        input_text(&h, cx, |o| &o.sidebar_filter),
        "is:ahead is:behind "
    );
    assert!(!shown(&h, cx, format!("sidebar-open:{}", name(&ahead))));

    // Leaving the box hides the dropdown.
    h.keys(cx, "escape escape");
    assert!(!shown(&h, cx, "filter-suggestions".into()));
}

#[gpui::test]
fn opening_a_file_or_folder_inside_an_open_repo_focuses_it(cx: &mut TestAppContext) {
    let (h, a, _b, _, _) = two_projects(cx);
    a.commit_file("sub/c.txt", "c\n", "second");
    for inside in [a.path().join("sub"), a.path().join("sub/c.txt")] {
        h.act(cx, |o, w, cx| o.activate(1, w, cx));
        assert_eq!(h.read(cx, |o, _| o.active), Some(1));
        h.act(cx, |o, w, cx| o.open_project_notify(&inside, w, cx));
        h.read(cx, |o, _| {
            assert_eq!(o.projects.len(), 2, "{} added a copy", inside.display());
            assert_eq!(
                o.active,
                Some(0),
                "{} focuses its repository",
                inside.display()
            );
        });
    }
}

fn in_palette(h: &Harness, cx: &mut TestAppContext) -> bool {
    h.has_settings(cx)
}

#[gpui::test]
fn ui_switch_project_shortcut_flips_to_the_other_project(cx: &mut TestAppContext) {
    let (h, _a, _b, first, second) = two_projects(cx);
    h.keys(cx, "ctrl-o");
    assert!(h.bounds(cx, "palette-badge").is_some(), "submenu badge");
    assert!(h.bounds(cx, &format!("palette-project:{first}")).is_some());
    assert!(h.bounds(cx, &format!("palette-project:{second}")).is_some());
    // The current project is skipped by the initial highlight.
    h.keys(cx, "enter");
    assert!(!in_palette(&h, cx));
    assert_eq!(active_name(&h, cx), first);
    // Now the first is current: the highlight starts on the second.
    h.keys(cx, "ctrl-o enter");
    assert_eq!(active_name(&h, cx), second);
}

#[gpui::test]
fn ui_palette_shortcuts_close_their_own_mode_and_switch_to_the_other(cx: &mut TestAppContext) {
    let (h, _a, _b, _first, _second) = two_projects(cx);
    let switcher = |h: &Harness, cx: &mut TestAppContext| h.bounds(cx, "palette-badge").is_some();
    h.keys(cx, "ctrl-p");
    assert!(in_palette(&h, cx) && !switcher(&h, cx), "commands");
    h.keys(cx, "ctrl-p");
    assert!(!in_palette(&h, cx), "the palette key again closes it");
    h.keys(cx, "ctrl-o");
    assert!(switcher(&h, cx));
    h.keys(cx, "ctrl-o");
    assert!(!in_palette(&h, cx), "the switcher key again closes it");
    // The other key switches mode instead of doing nothing.
    h.keys(cx, "ctrl-o ctrl-p");
    assert!(in_palette(&h, cx) && !switcher(&h, cx));
    h.keys(cx, "ctrl-o");
    assert!(switcher(&h, cx));
    // Text in the field does not get in the way.
    h.type_text(cx, "x");
    h.keys(cx, "ctrl-o");
    assert!(!in_palette(&h, cx));
    // Focus is back on the app: its shortcuts work.
    h.keys(cx, "ctrl-p");
    assert!(in_palette(&h, cx));
}

#[gpui::test]
fn ui_switch_project_highlights_the_first_row_when_it_is_not_current(cx: &mut TestAppContext) {
    let (h, _a, _b, first, second) = two_projects(cx);
    // A recent project below the open ones must not steal the highlight.
    h.act(cx, |o, w, cx| o.activate(0, w, cx));
    let c = TempRepo::new();
    c.commit_file("c.txt", "c\n", "first");
    let path = c.path().to_path_buf();
    // Opening finishes in the background: let it land before closing the new project.
    h.act(cx, |o, w, cx| o.open_project_notify(&path, w, cx));
    h.act(cx, |o, w, cx| {
        o.close_project(2, w, cx);
        o.activate(1, w, cx);
    });
    assert_eq!(active_name(&h, cx), second);
    h.keys(cx, "ctrl-o enter");
    assert_eq!(active_name(&h, cx), first);
}

#[gpui::test]
fn ui_switch_project_filters_and_moves_with_arrows(cx: &mut TestAppContext) {
    let (h, _a, _b, first, second) = two_projects(cx);
    h.keys(cx, "ctrl-o");
    h.type_text(cx, &second);
    assert!(h.bounds(cx, &format!("palette-project:{first}")).is_none());
    h.keys(cx, "enter");
    assert_eq!(active_name(&h, cx), second);
    // Arrows reach the list through the custom query field.
    h.keys(cx, "ctrl-o down up enter");
    assert_eq!(active_name(&h, cx), first);
}

#[gpui::test]
fn ui_switch_project_opens_a_recent_project(cx: &mut TestAppContext) {
    let (h, _a, b, first, second) = two_projects(cx);
    h.act(cx, |o, w, cx| o.close_project(1, w, cx));
    assert_eq!(active_name(&h, cx), first);
    h.keys(cx, "ctrl-o");
    h.type_text(cx, &second);
    assert!(h.bounds(cx, &format!("palette-project:{second}")).is_some());
    h.keys(cx, "enter");
    assert_eq!(active_name(&h, cx), second);
    h.read(cx, |o, _| {
        assert!(o.projects.iter().any(|p| p.path == b.path()));
    });
}

#[gpui::test]
fn ui_palette_submenu_back_and_escape(cx: &mut TestAppContext) {
    let (h, _a, _b, first, _second) = two_projects(cx);
    // Into the submenu from the commands, by typing and by clicking.
    h.keys(cx, "ctrl-p");
    assert!(h.bounds(cx, "palette-badge").is_none());
    h.type_text(cx, "switch proj");
    h.keys(cx, "enter");
    assert!(h.bounds(cx, "palette-badge").is_some());
    // Backspace with an empty query goes back to the commands; the palette stays.
    h.keys(cx, "backspace");
    assert!(h.bounds(cx, "palette-badge").is_none());
    assert!(in_palette(&h, cx));
    h.click(cx, "palette-command:Switch project…");
    assert!(h.bounds(cx, "palette-badge").is_some());
    // Backspace that deletes text stays in the submenu.
    h.type_text(cx, "x");
    h.keys(cx, "backspace");
    assert!(h.bounds(cx, "palette-badge").is_some());
    // Escape goes back too, then closes the commands as before.
    h.keys(cx, "escape");
    assert!(h.bounds(cx, "palette-badge").is_none());
    assert!(in_palette(&h, cx));
    h.keys(cx, "escape");
    assert!(!in_palette(&h, cx));
    // Opened straight into the submenu, Escape still lands on the commands.
    h.keys(cx, "ctrl-o escape");
    assert!(in_palette(&h, cx));
    assert!(h.bounds(cx, "palette-command:Switch project…").is_some());
    h.keys(cx, "escape");
    assert!(!in_palette(&h, cx));
    assert_ne!(active_name(&h, cx), first);
}

#[gpui::test]
fn switch_project_rows_carry_the_sidebar_markers(cx: &mut TestAppContext) {
    use crate::app::switch_project::Section;
    use osier_config::settings::ChangesIndicator;
    let repo = repo_with_upstream(2, true);
    let h = Harness::open(cx, &repo);
    let path = repo.path().to_path_buf();
    let row = |h: &Harness, cx: &mut TestAppContext| {
        h.read(cx, |o, _| o.switch_project_entries().remove(0))
    };
    let e = row(&h, cx);
    assert_eq!(e.section, Section::Open);
    assert!(e.active && e.dirty && !e.dot && !e.pinned);
    assert_eq!(e.arrows.as_deref(), Some("↑2"));
    assert!(e.dir.ends_with(&e.name));
    h.act(cx, |o, _, cx| {
        o.settings.appearance.changes_indicator = ChangesIndicator::Dot;
        o.toggle_pin(&path, cx);
    });
    let e = row(&h, cx);
    assert!(e.dirty && e.dot && e.pinned);
    // Closed and pinned: listed under Pinned with the pin as its icon.
    h.act(cx, |o, w, cx| o.close_project(0, w, cx));
    let e = row(&h, cx);
    assert_eq!(e.section, Section::Pinned);
    assert!(!e.active);
}

#[gpui::test]
fn empty_changes_tab_on_startup_and_when_switching(cx: &mut TestAppContext) {
    // Startup on a clean project: Changes would be empty, so Log shows.
    let (h, a, _b, first, second) = two_projects(cx);
    assert_eq!(tab(&h, cx), Tab::Log);
    // Changes appear in the first project; going back there returns to Changes.
    a.write("new.txt", "x\n");
    h.act(cx, |o, _, cx| o.refresh_project(0, cx));
    h.keys(cx, "ctrl-o enter");
    assert_eq!(active_name(&h, cx), first);
    assert_eq!(tab(&h, cx), Tab::Changes);
    // And to the clean one: Log again.
    h.keys(cx, "ctrl-o enter");
    assert_eq!(active_name(&h, cx), second);
    assert_eq!(tab(&h, cx), Tab::Log);
}

#[gpui::test]
fn ui_other_tabs_and_manual_choices_are_kept(cx: &mut TestAppContext) {
    let (h, a, _b, first, _second) = two_projects(cx);
    a.write("new.txt", "x\n");
    h.act(cx, |o, _, cx| o.refresh_project(0, cx));
    // On Branches, switching never moves the tab.
    h.click(cx, "tab-branches");
    h.keys(cx, "ctrl-o enter");
    assert_eq!(active_name(&h, cx), first);
    assert_eq!(tab(&h, cx), Tab::Branches);
    h.keys(cx, "ctrl-o enter");
    assert_eq!(tab(&h, cx), Tab::Branches);
    // Moved to Log automatically, then picked Log by hand: it stays Log on a project
    // with changes.
    h.click(cx, "tab-changes");
    h.keys(cx, "ctrl-o enter");
    assert_eq!(tab(&h, cx), Tab::Changes, "first has changes");
    h.keys(cx, "ctrl-o enter");
    assert_eq!(tab(&h, cx), Tab::Log);
    h.click(cx, "tab-log");
    h.keys(cx, "ctrl-o enter");
    assert_eq!(active_name(&h, cx), first);
    assert_eq!(tab(&h, cx), Tab::Log);
}

#[gpui::test]
fn empty_changes_tab_setting_picks_the_tab_or_stays(cx: &mut TestAppContext) {
    use osier_config::settings::EmptyChangesTab;
    let (h, a, _b, _first, _second) = two_projects(cx);
    a.write("new.txt", "x\n");
    h.act(cx, |o, _, cx| o.refresh_project(0, cx));
    h.act(cx, |o, w, cx| {
        o.settings.projects.empty_changes_tab = EmptyChangesTab::Stashes;
        o.activate(0, w, cx);
    });
    assert_eq!(tab(&h, cx), Tab::Changes);
    h.act(cx, |o, w, cx| o.activate(1, w, cx));
    assert_eq!(tab(&h, cx), Tab::Stashes);
    h.act(cx, |o, w, cx| {
        o.activate(0, w, cx);
        o.settings.projects.empty_changes_tab = EmptyChangesTab::Stay;
        o.activate(1, w, cx);
    });
    assert_eq!(tab(&h, cx), Tab::Changes);
    // A project opened fresh is judged once its first status arrives.
    let c = TempRepo::new();
    c.commit_file("c.txt", "c\n", "first");
    let path = c.path().to_path_buf();
    h.act(cx, |o, w, cx| {
        o.settings.projects.empty_changes_tab = EmptyChangesTab::Files;
        o.activate(0, w, cx);
        o.open_project_notify(&path, w, cx);
    });
    assert_eq!(tab(&h, cx), Tab::Files);
}

#[gpui::test]
fn startup_restores_the_project_that_was_active(cx: &mut TestAppContext) {
    let (a, b, c) = (TempRepo::new(), TempRepo::new(), TempRepo::new());
    for r in [&a, &b, &c] {
        r.commit_file("a.txt", "a\n", "first");
    }
    let state = State {
        open_projects: vec![a.path().into(), b.path().into(), c.path().into()],
        active_project: Some(b.path().into()),
        ..State::default()
    };
    let h = Harness::open_with(cx, state, None);
    h.read(cx, |o, _| {
        assert_eq!(o.projects.len(), 3);
        assert_eq!(
            o.active,
            Some(1),
            "the saved active project, not the last opened"
        );
        let order: Vec<&std::path::Path> = o.projects.iter().map(|p| p.path.as_path()).collect();
        assert_eq!(order, [a.path(), b.path(), c.path()], "saved order kept");
    });
}

#[gpui::test]
fn startup_opens_the_active_project_first_and_the_rest_in_the_background(cx: &mut TestAppContext) {
    let repos: Vec<TempRepo> = (0..20)
        .map(|n| {
            let r = TempRepo::new();
            r.commit_file("a.txt", &format!("{n}\n"), "first");
            r
        })
        .collect();
    let gone = std::env::temp_dir().join("osier-test-no-such-repo");
    let mut open_projects: Vec<std::path::PathBuf> =
        repos.iter().map(|r| r.path().to_path_buf()).collect();
    open_projects.insert(5, gone.clone());
    let state = State {
        open_projects,
        active_project: Some(repos[12].path().into()),
        ..State::default()
    };
    let h = Harness::open_with(cx, state, None);
    h.read(cx, |o, _| {
        let paths: Vec<&std::path::Path> = o.projects.iter().map(|p| p.path.as_path()).collect();
        let want: Vec<&std::path::Path> = repos.iter().map(|r| r.path()).collect();
        assert_eq!(
            paths, want,
            "all twenty, in saved order, missing one dropped"
        );
        assert_eq!(o.active, Some(12));
        assert!(
            !o.state.open_projects.contains(&gone),
            "missing repo forgotten"
        );
        let error = o.global_error.clone().unwrap_or_default();
        assert!(error.starts_with("Could not reopen a project"), "{error}");
        assert!(
            o.projects.iter().all(|p| p.snapshot.is_some()),
            "all loaded"
        );
    });
}

#[gpui::test]
fn a_repository_deleted_while_open_shows_an_error(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "one\n", "first");
    let h = Harness::open(cx, &repo);
    h.read(cx, |osier, _| {
        assert!(osier.active_project().expect("project").error.is_none());
    });
    std::fs::remove_dir_all(repo.path()).expect("remove repo");
    h.act(cx, |osier, _, cx| osier.refresh_active(cx));
    cx.run_until_parked();
    h.read(cx, |osier, _| {
        let project = osier.active_project().expect("project still listed");
        assert!(
            project.error.is_some(),
            "refresh of a missing repo reports an error"
        );
        assert!(!project.loading);
    });
}

#[gpui::test]
fn pins_and_drag_reorder(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "one\n", "first");
    let other = TempRepo::new();
    other.commit_file("b.txt", "b\n", "first");
    let third = TempRepo::new();
    third.commit_file("c.txt", "c\n", "first");
    let h = Harness::open(cx, &repo);
    h.act(cx, |osier, window, cx| {
        osier.open_project_notify(other.path(), window, cx);
        osier.open_project_notify(third.path(), window, cx);
    });
    cx.run_until_parked();
    let names = |h: &Harness, cx: &mut TestAppContext| {
        h.read(cx, |osier, _| {
            osier
                .projects
                .iter()
                .map(|p| p.path.clone())
                .collect::<Vec<_>>()
        })
    };
    let before = names(&h, cx);
    assert_eq!(before.len(), 3);
    // Drag the last project to the top.
    h.act(cx, |osier, _, cx| osier.move_project_to(2, 0, cx));
    let after = names(&h, cx);
    assert_eq!(after[0], before[2]);
    assert_eq!(after[1], before[0]);
    h.read(cx, |osier, _| {
        assert_eq!(osier.state.open_projects, after, "order is persisted");
        assert_eq!(
            osier.active_project().unwrap().path,
            before[2],
            "the active project follows its row"
        );
    });

    // Pin the second repository, close it: it moves to Pinned, not Recent.
    let pinned_path = before[1].clone();
    h.act(cx, |osier, _, cx| osier.toggle_pin(&pinned_path, cx));
    h.act(cx, |osier, window, cx| {
        let ix = osier
            .projects
            .iter()
            .position(|p| p.path == pinned_path)
            .unwrap();
        osier.close_project(ix, window, cx);
    });
    h.read(cx, |osier, cx| {
        assert_eq!(osier.pinned_entries(cx), vec![pinned_path.clone()]);
        assert!(
            !osier
                .recent_entries(cx)
                .iter()
                .any(|(p, _)| p == &pinned_path)
        );
    });
    h.act(cx, |osier, _, cx| osier.toggle_pin(&pinned_path, cx));
    h.read(cx, |osier, cx| {
        assert!(osier.pinned_entries(cx).is_empty());
        assert!(
            osier
                .recent_entries(cx)
                .iter()
                .any(|(p, _)| p == &pinned_path)
        );
    });
}

#[gpui::test]
fn opens_a_repository_and_selects_the_first_change(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "one\n", "first");
    repo.write("a.txt", "two\n");
    repo.write("new.txt", "new\n");
    let h = Harness::open(cx, &repo);

    h.read(cx, |osier, _| {
        let project = osier.active_project().expect("active project");
        let snapshot = project.snapshot.as_ref().expect("snapshot loaded");
        assert_eq!(snapshot.status.unstaged().count(), 1);
        assert_eq!(snapshot.status.untracked().count(), 1);
        assert_eq!(snapshot.commits.len(), 1);
        assert_eq!(
            project.selection,
            Selection::File {
                path: "a.txt".into(),
                area: Area::Unstaged
            }
        );
        assert_eq!(osier.diff_files, 1, "diff pane shows the selected file");
        assert_eq!(osier.diff_model.len(), 1, "hunk model parsed");
        assert!(osier.state.open_projects.contains(&project.path));
    });

    // Keyboard navigation moves to the next change and reloads the diff.
    h.act(cx, |osier, _, cx| osier.move_selection(1, cx));
    h.read(cx, |osier, _| {
        assert_eq!(
            osier.active_project().expect("project").selection,
            Selection::File {
                path: "new.txt".into(),
                area: Area::Untracked
            }
        );
        assert_eq!(osier.diff_files, 1);
    });
}

#[gpui::test]
fn closing_projects_and_recents(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "one\n", "first");
    let other = TempRepo::new();
    other.commit_file("b.txt", "two\n", "first");
    let h = Harness::open(cx, &repo);

    // Open a second project, close the first: the second stays active, the first is recent.
    let other_path = other.path().to_path_buf();
    h.act(cx, |osier, window, cx| {
        osier.open_project_notify(&other_path, window, cx)
    });
    h.read(cx, |osier, _| assert_eq!(osier.projects.len(), 2));
    h.act(cx, |osier, window, cx| osier.close_project(0, window, cx));
    h.read(cx, |osier, _| {
        assert_eq!(osier.projects.len(), 1);
        assert_eq!(osier.active_project().unwrap().path, other.path());
        assert_eq!(osier.state.recent_not_open().count(), 1);
    });

    // Forget the recent entry.
    let gone = repo.path().to_path_buf();
    h.act(cx, |osier, window, cx| {
        osier.forget_recent(&gone, window, cx)
    });
    h.read(cx, |osier, _| {
        assert_eq!(osier.state.recent_not_open().count(), 0)
    });

    // A path that is not a repository fails cleanly and is not kept anywhere.
    let not_repo = tempfile::tempdir().unwrap();
    let p = not_repo.path().to_path_buf();
    h.act(cx, |osier, _, cx| {
        assert!(osier.open_project(&p, true, cx).is_err());
    });
    h.read(cx, |osier, _| {
        assert_eq!(osier.projects.len(), 1);
        assert!(!osier.state.recent.contains(&p));
    });
}

#[gpui::test]
fn ui_sidebar_menu_opens_git_config(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    repo.git(["config", "pull.rebase", "true"]);
    let mut state = State::default();
    state.layout.sidebar_visible = true;
    let h = Harness::open_with(cx, state, Some(repo.path().to_path_buf()));
    let name = h.read(cx, |o, _| o.active_project().unwrap().name.clone());
    h.right_click(cx, &format!("sidebar-open:{name}"));
    // Open in editor, Open in terminal, Open in file manager, Git config.
    h.keys(cx, "down down down down enter");
    assert!(h.bounds(cx, "repo-config-dialog").is_some(), "dialog open");
    h.read(cx, |o, _| {
        let view = o.repo_config.as_ref().expect("loaded");
        let config = view.data.as_ref().unwrap().as_ref().unwrap();
        let rebase = config
            .entries
            .iter()
            .find(|e| e.key == "pull.rebase")
            .unwrap();
        assert_eq!(rebase.scope, osier_git::config::Scope::Local);
    });
    h.keys(cx, "escape");
    assert!(h.bounds(cx, "repo-config-dialog").is_none(), "closed");
}

#[gpui::test]
fn ui_git_config_tab_follows_its_setting(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    // Wide enough for every tab, so the click lands on the tab itself.
    let mut state = State::default();
    state.layout.center_width = Some(1000.);
    let h = Harness::open_with(cx, state, Some(repo.path().to_path_buf()));
    assert!(h.bounds(cx, "tab-git config").is_none(), "off by default");
    h.act(cx, |o, _, cx| {
        o.set_tab(Tab::Config, cx);
        assert_eq!(o.tab, Tab::Log, "hidden tab cannot be picked");
        o.settings.git.config_tab = true;
        o.settings_changed(cx);
    });
    h.click(cx, "tab-git config");
    h.read(cx, |o, _| assert_eq!(o.tab, Tab::Config));
    assert!(h.bounds(cx, "repo-config-tab").is_some());
    assert!(
        h.bounds(cx, "repo-config-reload").is_some(),
        "config loaded"
    );
    h.act(cx, |o, _, cx| {
        o.settings.git.config_tab = false;
        o.settings_changed(cx);
    });
    h.read(cx, |o, _| {
        assert_eq!(o.tab, Tab::Log, "turned off: back to Log")
    });
    assert!(h.bounds(cx, "tab-git config").is_none());
}

#[gpui::test]
fn removing_a_scan_dir_forgets_its_recents(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    let scan_root = tempfile::tempdir().unwrap();
    for name in ["alpha", "beta"] {
        let r = scan_root.path().join(name);
        std::fs::create_dir_all(&r).unwrap();
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(&r)
            .status()
            .unwrap();
    }
    let h = Harness::open(cx, &repo);
    h.act(cx, |o, _, cx| {
        o.settings.projects.scan_dirs = vec![scan_root.path().to_path_buf()];
        o.rescan_projects(cx);
    });
    // Open both scanned repositories (which makes them recents), then close beta.
    for name in ["alpha", "beta"] {
        let p = scan_root.path().join(name);
        h.act(cx, |o, w, cx| o.open_project_notify(&p, w, cx));
    }
    h.act(cx, |o, w, cx| o.close_project(2, w, cx));
    let names = |h: &Harness, cx: &mut TestAppContext| {
        h.read(cx, |o, cx| {
            let recent: Vec<String> = o
                .state
                .recent
                .iter()
                .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
                .collect();
            let sidebar: Vec<String> = o
                .recent_entries(cx)
                .iter()
                .map(|(p, _)| p.file_name().unwrap().to_string_lossy().into_owned())
                .collect();
            (recent, sidebar)
        })
    };
    let (recent, sidebar) = names(&h, cx);
    assert!(recent.contains(&"beta".to_string()), "{recent:?}");
    assert_eq!(sidebar, ["beta"]);

    let dir = scan_root.path().to_path_buf();
    h.act(cx, |o, _, cx| o.remove_scan_dir(dir, cx));
    let (recent, sidebar) = names(&h, cx);
    let first = repo.path().file_name().unwrap().to_string_lossy();
    assert!(!recent.contains(&"beta".to_string()), "{recent:?}");
    assert!(
        recent.contains(&"alpha".to_string()),
        "open projects stay: {recent:?}"
    );
    assert!(
        recent.iter().any(|r| *r == first),
        "others stay: {recent:?}"
    );
    assert!(sidebar.is_empty(), "{sidebar:?}");
}

#[gpui::test]
fn scan_dirs_feed_recents_and_filter_and_reorder(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    let second = TempRepo::new();
    second.commit_file("b.txt", "b\n", "first");
    // A scan directory holding two repositories and one plain folder.
    let scan_root = tempfile::tempdir().unwrap();
    for name in ["alpha", "beta"] {
        let r = scan_root.path().join(name);
        std::fs::create_dir_all(&r).unwrap();
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(&r)
            .status()
            .unwrap();
    }
    std::fs::create_dir_all(scan_root.path().join("plain")).unwrap();

    let h = Harness::open(cx, &repo);
    h.act(cx, |osier, _, cx| {
        osier.settings.projects.scan_dirs = vec![scan_root.path().to_path_buf()];
        osier.rescan_projects(cx);
    });
    h.read(cx, |osier, cx| {
        assert_eq!(
            osier.scanned.len(),
            2,
            "alpha and beta found, plain skipped"
        );
        let recents = osier.recent_entries(cx);
        assert_eq!(recents.len(), 2);
        assert!(recents.iter().all(|(_, provider)| provider.is_some()));
    });

    // Filtering narrows recents; Enter opens the first match.
    h.act(cx, |osier, window, cx| {
        osier
            .sidebar_filter
            .update(cx, |s, cx| s.set_value("beta", window, cx))
    });
    h.read(cx, |osier, cx| {
        assert_eq!(osier.recent_entries(cx).len(), 1)
    });
    h.act(cx, |osier, window, cx| {
        osier.open_first_filtered(window, cx)
    });
    h.read(cx, |osier, _| {
        assert_eq!(osier.projects.len(), 2);
        assert!(osier.active_project().unwrap().path.ends_with("beta"));
    });

    // A scanned entry cannot be forgotten: recents are unchanged after the attempt.
    h.act(cx, |osier, window, cx| {
        osier
            .sidebar_filter
            .update(cx, |s, cx| s.set_value("", window, cx))
    });
    let alpha = scan_root.path().join("alpha");
    h.act(cx, |osier, window, cx| {
        osier.forget_recent(&alpha, window, cx)
    });
    h.read(cx, |osier, cx| {
        assert!(
            osier
                .recent_entries(cx)
                .iter()
                .any(|(p, _)| p.ends_with("alpha"))
        )
    });

    // Reorder open projects.
    let second_path = second.path().to_path_buf();
    h.act(cx, |osier, window, cx| {
        osier.open_project_notify(&second_path, window, cx)
    });
    h.read(cx, |osier, _| assert_eq!(osier.projects.len(), 3));
    h.act(cx, |osier, _, cx| osier.move_project(2, -2, cx));
    h.read(cx, |osier, _| {
        assert_eq!(osier.projects[0].path, second.path());
        assert_eq!(osier.state.open_projects[0], second.path());
        assert_eq!(
            osier.active_project().unwrap().path,
            second.path(),
            "active follows the moved project"
        );
    });
}
