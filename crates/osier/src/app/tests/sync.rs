//! Fetch, pull and push, update project, stash prompts and the offers that follow a fetch.

use super::*;

#[gpui::test]
fn open_on_github_uses_the_github_remote(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    repo.git(["remote", "add", "origin", "git@github.com:octo/repo.git"]);
    let other = TempRepo::new();
    other.commit_file("b.txt", "b\n", "first");
    other.git([
        "remote",
        "add",
        "origin",
        "https://github.com/octo/other.git",
    ]);
    let h = Harness::open(cx, &repo);

    let hash = h.read(cx, |o, _| {
        o.active_project()
            .and_then(|p| p.snapshot.as_ref())
            .expect("snapshot")
            .commits[0]
            .hash
            .clone()
    });
    h.act(cx, |o, window, cx| {
        o.open_commit_on_github(hash.clone(), window, cx)
    });
    assert_eq!(
        cx.opened_url(),
        Some(format!("https://github.com/octo/repo/commit/{hash}"))
    );

    // A project that is not open reads its remotes first.
    let path = other.path().to_path_buf();
    h.act(cx, |o, window, cx| {
        o.open_project_on_github(&path, window, cx)
    });
    assert_eq!(
        cx.opened_url().as_deref(),
        Some("https://github.com/octo/other")
    );
}

#[gpui::test]
fn switching_over_conflicting_changes_offers_to_save_them(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "base");
    repo.checkout_new("other");
    repo.commit_file("a.txt", "other\n", "other");
    repo.checkout("main");
    repo.write("a.txt", "mine\n");
    let h = Harness::open(cx, &repo);
    h.act(cx, |o, w, cx| {
        o.checkout_branch("other".into(), false, w, cx)
    });
    assert_eq!(repo.git(["branch", "--show-current"]).trim(), "main");
    h.click(cx, "dialog-ok");
    assert_eq!(repo.git(["branch", "--show-current"]).trim(), "other");
    assert!(
        repo.git(["stash", "list"])
            .contains("Saved before switching to other"),
        "the edit is saved, not brought along"
    );
    assert_eq!(repo.read("a.txt"), "other\n");
}

#[gpui::test]
fn push_fetch_and_pull_through_the_app(cx: &mut TestAppContext) {
    use crate::remote_ops::RemoteOp;
    use osier_git::ops::remote::PushOptions;

    let repo = TempRepo::new();
    repo.commit_file("a.txt", "one\n", "first");
    let bare = repo.add_bare_origin();
    let h = Harness::open(cx, &repo);

    // Push and set upstream.
    h.act(cx, |osier, window, cx| {
        osier.start_remote(
            RemoteOp::Push(PushOptions {
                remote: Some("origin".into()),
                set_upstream: true,
                ..Default::default()
            }),
            window,
            cx,
        )
    });
    h.read(cx, |osier, _| {
        assert!(osier.remote_job.is_none(), "job finished");
        let snapshot = osier.active_project().unwrap().snapshot.as_ref().unwrap();
        assert_eq!(
            snapshot.status.branch.upstream.as_deref(),
            Some("origin/main")
        );
        assert_eq!(snapshot.remotes.len(), 1);
    });

    // Someone else pushes; fetch shows us behind; pull fast-forwards.
    let other = TempRepo::new();
    other.git(["remote", "add", "origin", bare.to_string_lossy().as_ref()]);
    other.git(["fetch", "-q", "origin"]);
    other.git(["checkout", "-q", "-b", "main", "origin/main"]);
    other.commit_file("b.txt", "two\n", "second");
    other.git(["push", "-q", "origin", "main"]);

    h.act(cx, |osier, window, cx| {
        osier.start_remote(RemoteOp::fetch_all(), window, cx)
    });
    h.read(cx, |osier, _| {
        let snapshot = osier.active_project().unwrap().snapshot.as_ref().unwrap();
        assert_eq!(snapshot.status.branch.behind, 1);
    });
    h.act(cx, |osier, window, cx| {
        osier.start_remote(RemoteOp::pull_default(), window, cx)
    });
    h.read(cx, |osier, _| {
        let snapshot = osier.active_project().unwrap().snapshot.as_ref().unwrap();
        assert_eq!(snapshot.status.branch.behind, 0);
        assert_eq!(snapshot.commits.len(), 2);
    });
    assert!(repo.path().join("b.txt").exists());

    // Session-only account choices never reach state.toml.
    h.act(cx, |osier, _, _| {
        osier.set_project_account("/tmp/x", "alice", false);
        osier.save_state();
    });
    h.read(cx, |osier, _| {
        let saved = osier_config::State::load(&osier.paths).unwrap();
        assert!(!saved.accounts.contains_key("/tmp/x"));
        assert_eq!(
            osier.state.accounts.get("/tmp/x").map(String::as_str),
            Some("alice")
        );
    });
}

/// A repository with a pushed `config.env`, and a second clone that stops tracking it,
/// ignores it and pushes that.
fn pushed_then_untracked_elsewhere() -> (TempRepo, TempRepo) {
    let repo = TempRepo::new();
    repo.commit_file("config.env", "secret=1\n", "add config");
    let bare = repo.add_bare_origin();
    repo.git(["push", "-q", "-u", "origin", "HEAD"]);
    let other = TempRepo::new();
    other.git(["remote", "add", "origin", bare.to_string_lossy().as_ref()]);
    other.git(["fetch", "-q", "origin"]);
    other.git(["checkout", "-q", "-b", "main", "origin/main"]);
    other.write(".gitignore", "/config.env\n");
    other.git(["rm", "-q", "--cached", "config.env"]);
    other.git(["add", ".gitignore"]);
    other.commit("untrack config");
    other.git(["push", "-q", "origin", "main"]);
    (repo, other)
}

#[gpui::test]
fn pull_offers_to_keep_newly_ignored_files(cx: &mut TestAppContext) {
    use crate::remote_ops::RemoteOp;

    let (repo, _other) = pushed_then_untracked_elsewhere();
    let h = Harness::open(cx, &repo);
    h.act(cx, |osier, window, cx| {
        osier.start_remote(RemoteOp::pull_default(), window, cx)
    });
    assert!(!repo.path().join("config.env").exists(), "git removed it");
    assert!(h.bounds(cx, "keep-ignored-dialog").is_some(), "offer shown");
    h.click(cx, "dialog-ok");
    assert_eq!(repo.read("config.env"), "secret=1\n");
    assert!(
        repo.git(["status", "--porcelain"]).is_empty(),
        "restored copy is untracked and ignored"
    );
}

#[gpui::test]
fn ui_pull_in_a_rebase_configured_repo_offers_stash_and_pull(cx: &mut TestAppContext) {
    use crate::remote_ops::RemoteOp;

    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    let bare = repo.add_bare_origin();
    repo.git(["push", "-q", "-u", "origin", "HEAD"]);
    let other = TempRepo::new();
    other.git(["remote", "add", "origin", bare.to_string_lossy().as_ref()]);
    other.git(["fetch", "-q", "origin"]);
    other.git(["checkout", "-q", "-b", "main", "origin/main"]);
    other.commit_file("b.txt", "b\n", "remote work");
    other.git(["push", "-q", "origin", "main"]);
    repo.git(["config", "pull.rebase", "true"]);
    repo.write("a.txt", "edited\n");

    let h = Harness::open(cx, &repo);
    h.act(cx, |osier, window, cx| {
        osier.start_remote(RemoteOp::pull_default(), window, cx)
    });
    assert!(!repo.path().join("b.txt").exists(), "git refused");
    h.click(cx, "dialog-ok");
    assert!(repo.path().join("b.txt").exists(), "pulled");
    assert_eq!(repo.read("a.txt"), "edited\n", "change put back");
    assert!(repo.git(["stash", "list"]).is_empty());
}

/// A pushed repository whose remote got a commit from elsewhere while it made one of its
/// own (diverged), plus an uncommitted edit.
fn diverged_with_local_edit() -> (TempRepo, TempRepo) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    let bare = repo.add_bare_origin();
    repo.git(["push", "-q", "-u", "origin", "HEAD"]);
    let other = TempRepo::new();
    other.git(["remote", "add", "origin", bare.to_string_lossy().as_ref()]);
    other.git(["fetch", "-q", "origin"]);
    other.git(["checkout", "-q", "-b", "main", "origin/main"]);
    other.commit_file("remote.txt", "r\n", "remote work");
    other.git(["push", "-q", "origin", "main"]);
    repo.commit_file("local.txt", "l\n", "local work");
    repo.write("a.txt", "edited\n");
    (repo, other)
}

#[gpui::test]
fn ui_update_project_asks_then_remembers(cx: &mut TestAppContext) {
    let (repo, other) = diverged_with_local_edit();
    let h = Harness::open(cx, &repo);
    let key = repo.path().to_string_lossy().into_owned();

    h.click(cx, "update-project");
    assert!(
        h.bounds(cx, "update-dialog").is_some(),
        "asks the first time"
    );
    h.click(cx, "update-remember");
    h.click(cx, "dialog-ok");
    assert!(repo.path().join("remote.txt").exists(), "merged in");
    assert_eq!(repo.read("a.txt"), "edited\n", "local edit put back");
    assert!(repo.git(["stash", "list"]).is_empty());
    h.read(cx, |o, _| {
        let choice = o.state.update_strategies.get(&key).expect("saved");
        assert_eq!(choice.strategy, "merge");
        assert!(choice.remember);
        assert!(o.state.pull_strategies.is_empty(), "kept apart from pulls");
    });

    // Remembered: the next update runs straight away.
    other.commit_file("more.txt", "m\n", "more remote work");
    other.git(["push", "-q", "origin", "main"]);
    h.click(cx, "update-project");
    assert!(h.bounds(cx, "update-dialog").is_none(), "no dialog");
    assert!(repo.path().join("more.txt").exists());
}

#[gpui::test]
fn ui_update_project_preselects_and_does_not_remember_by_default(cx: &mut TestAppContext) {
    let (repo, _other) = diverged_with_local_edit();
    repo.git(["config", "pull.rebase", "true"]);
    let h = Harness::open(cx, &repo);
    let key = repo.path().to_string_lossy().into_owned();
    h.click(cx, "update-project");
    // Preselected from pull.rebase; OK without touching anything rebases.
    h.click(cx, "dialog-ok");
    assert_eq!(
        repo.git(["rev-list", "--count", "--merges", "HEAD"]).trim(),
        "0",
        "rebased, no merge commit"
    );
    h.read(cx, |o, _| {
        let choice = o
            .state
            .update_strategies
            .get(&key)
            .expect("last choice kept");
        assert_eq!(choice.strategy, "rebase");
        assert!(!choice.remember, "unticked by default");
    });
    // ctrl-t, listed on the Keymap page like the other shortcuts.
    let spec = crate::keymap::actions()
        .into_iter()
        .find(|s| s.id == "update_project")
        .expect("in the keymap");
    assert_eq!(spec.defaults, ["ctrl-t"]);

    h.keys(cx, "ctrl-t");
    assert!(h.bounds(cx, "update-dialog").is_some(), "still asks");
    // Shortcuts keep working after a dialog closes, by Escape or by its button.
    h.keys(cx, "escape");
    assert!(h.bounds(cx, "update-dialog").is_none());
    h.keys(cx, "ctrl-t");
    assert!(h.bounds(cx, "update-dialog").is_some(), "after Escape");
    h.click(cx, "dialog-cancel");
    h.keys(cx, "ctrl-t");
    assert!(h.bounds(cx, "update-dialog").is_some(), "after Cancel");
}

#[gpui::test]
fn ui_per_project_choices_show_as_pills_and_forget(cx: &mut TestAppContext) {
    use crate::settings_panel::SettingsTarget;

    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    let h = Harness::open(cx, &repo);
    let key = repo.path().to_string_lossy().into_owned();
    h.act(cx, |o, _, _| {
        o.state.pull_strategies.insert(key.clone(), "rebase".into());
        o.state.update_strategies.insert(
            key.clone(),
            osier_config::state::UpdateChoice {
                strategy: "merge".into(),
                remember: true,
            },
        );
    });

    // The Accounts page's card opens Projects > Per-project choices.
    h.act(cx, |o, w, cx| {
        o.open_settings_at(SettingsTarget::ACCOUNTS, w, cx)
    });
    h.click(cx, "accounts-choices-link");
    // The scroll to the group happens a frame later, on a timer.
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(100));
    cx.run_until_parked();
    assert!(h.has_settings(cx));
    assert_eq!(
        h.read(cx, |o, _| o.settings_target),
        Some(SettingsTarget::PER_PROJECT_CHOICES)
    );
    assert!(h.bounds(cx, "choice:pull:0").is_some());
    assert!(h.bounds(cx, "choice:update:0").is_some());

    h.click(cx, "choice-forget:update:0");
    h.read(cx, |o, _| {
        assert!(o.state.update_strategies.is_empty(), "forgotten");
        assert_eq!(o.state.pull_strategies.len(), 1, "the other stays");
    });
    assert!(h.bounds(cx, "choice:update:0").is_none());
    h.click(cx, "reset-all-choices");
    h.read(cx, |o, _| assert!(o.state.pull_strategies.is_empty()));
    assert!(h.bounds(cx, "project-choice-row:0").is_none());
}

#[gpui::test]
fn pull_skips_the_keep_offer_when_turned_off(cx: &mut TestAppContext) {
    use crate::remote_ops::RemoteOp;

    let (repo, _other) = pushed_then_untracked_elsewhere();
    let h = Harness::open(cx, &repo);
    h.act(cx, |osier, window, cx| {
        osier.settings.git.offer_keep_ignored = false;
        osier.start_remote(RemoteOp::pull_default(), window, cx)
    });
    assert!(!repo.path().join("config.env").exists());
    assert!(h.bounds(cx, "keep-ignored-dialog").is_none());
}

#[gpui::test]
fn ui_pulling_a_branch_deleted_after_merge_offers_main(cx: &mut TestAppContext) {
    let t = TempRepo::new();
    t.commit_file("a.txt", "a\n", "first");
    let bare = t.add_bare_origin();
    t.git(["push", "-q", "-u", "origin", "HEAD"]);
    t.git(["remote", "set-head", "origin", "main"]);
    t.checkout_new("feature");
    t.commit_file("b.txt", "b\n", "feature work");
    t.git(["push", "-q", "-u", "origin", "feature"]);
    // Merged on GitHub and the branch deleted there.
    t.git(["push", "-q", "origin", "feature:main"]);
    t.git([
        "--git-dir",
        bare.to_str().unwrap(),
        "branch",
        "-D",
        "feature",
    ]);
    let h = Harness::open(cx, &t);
    h.act(cx, |o, window, cx| {
        o.start_remote(crate::remote_ops::RemoteOp::pull_default(), window, cx)
    });
    assert!(h.has_settings(cx), "a dialog explains and offers main");
    h.click(cx, "dialog-ok");
    assert_eq!(t.git(["branch", "--show-current"]).trim(), "main");
    assert!(
        t.try_git(["rev-parse", "--verify", "--quiet", "refs/heads/feature"])
            .is_err(),
        "the merged branch is deleted"
    );
    assert_eq!(t.read("b.txt"), "b\n", "main was pulled");
}

#[gpui::test]
fn ui_fetch_and_prune_offers_to_delete_merged_branches_once(cx: &mut TestAppContext) {
    let t = TempRepo::new();
    t.commit_file("a.txt", "a\n", "first");
    let bare = t.add_bare_origin();
    t.git(["push", "-q", "-u", "origin", "main"]);
    t.git(["remote", "set-head", "origin", "main"]);
    for (b, merge) in [("one", true), ("two", true), ("wip", false)] {
        t.checkout("main");
        t.git(["merge", "-q", "--ff-only", "origin/main"]);
        t.checkout_new(b);
        t.commit_file(&format!("{b}.txt"), "x\n", b);
        t.git(["push", "-q", "-u", "origin", b]);
        if merge {
            t.git(["push", "-q", "origin", &format!("{b}:main")]);
            t.git(["fetch", "-q", "origin"]);
        }
    }
    t.checkout("main");
    for b in ["one", "two", "wip"] {
        t.git(["--git-dir", bare.to_str().unwrap(), "branch", "-D", b]);
    }
    let h = Harness::open(cx, &t);
    let prune = || {
        crate::remote_ops::RemoteOp::Fetch(osier_git::ops::remote::FetchOptions {
            remote: None,
            prune: true,
            tags: false,
        })
    };
    h.act(cx, |o, window, cx| o.start_remote(prune(), window, cx));
    assert!(h.has_settings(cx), "offers the merged branches");
    h.click(cx, "dialog-cancel");
    // Declined: the next fetch does not ask about the same branches again.
    h.act(cx, |o, window, cx| o.start_remote(prune(), window, cx));
    assert!(!h.has_settings(cx));
    // Offered again in a later session: delete them.
    h.act(cx, |o, _, _| o.gone_offered.clear());
    h.act(cx, |o, window, cx| o.start_remote(prune(), window, cx));
    assert!(h.has_settings(cx));
    h.click(cx, "dialog-ok");
    let branches = t.git(["branch", "--format=%(refname:short)"]);
    let mut branches: Vec<&str> = branches.lines().collect();
    branches.sort();
    assert_eq!(branches, ["main", "wip"], "unmerged work is kept");
}
