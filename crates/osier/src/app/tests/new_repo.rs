//! Clone and create (`new_repo.rs`).

use super::*;

/// GitHub as the clone and create flows see it, with nothing on the network.
#[derive(Default)]
struct FakeHub {
    repos: Vec<osier_auth::hub::GhRepo>,
    /// Where a created repository can be cloned from (a local bare repo).
    created_url: String,
    created: std::sync::Mutex<Option<osier_auth::hub::NewRepo>>,
}

impl crate::new_repo::Hub for FakeHub {
    fn repos(&self, _: &osier_auth::GhAccount) -> Result<Vec<osier_auth::hub::GhRepo>, String> {
        Ok(self.repos.clone())
    }
    fn orgs(&self, _: &osier_auth::GhAccount) -> Result<Vec<String>, String> {
        Ok(vec!["acme".into()])
    }
    fn templates(
        &self,
        _: &osier_auth::GhAccount,
        owners: &[String],
    ) -> Result<Vec<String>, String> {
        Ok(owners.iter().map(|o| format!("{o}/starter")).collect())
    }
    fn gitignore_templates(&self, _: &osier_auth::GhAccount) -> Result<Vec<String>, String> {
        Ok(vec!["Rust".into(), "Node".into()])
    }
    fn licenses(&self, _: &osier_auth::GhAccount) -> Result<Vec<osier_auth::hub::License>, String> {
        Ok(vec![osier_auth::hub::License {
            key: "mit".into(),
            name: "MIT License".into(),
        }])
    }
    fn gitignore_template(&self, _: &osier_auth::GhAccount, name: &str) -> Result<String, String> {
        Ok(format!("# {name}\n/target\n"))
    }
    fn license_text(
        &self,
        _: &osier_auth::GhAccount,
        _: &str,
        year: i32,
        holder: &str,
    ) -> Result<String, String> {
        Ok(format!("MIT {year} {holder}\n"))
    }
    fn create(
        &self,
        _: &osier_auth::GhAccount,
        new: &osier_auth::hub::NewRepo,
    ) -> Result<osier_auth::hub::CreatedRepo, String> {
        *self.created.lock().expect("lock") = Some(new.clone());
        Ok(osier_auth::hub::CreatedRepo {
            full_name: format!("{}/{}", new.owner, new.name),
            clone_url: self.created_url.clone(),
            html_url: String::new(),
        })
    }
}

fn gh_account(login: &str) -> osier_auth::GhAccount {
    osier_auth::GhAccount {
        host: "github.com".into(),
        login: login.into(),
        active: true,
        git_protocol: "https".into(),
        state: "success".into(),
    }
}

fn with_hub(h: &Harness, cx: &mut TestAppContext, hub: Arc<FakeHub>) {
    h.act(cx, move |o, _, _| {
        o.gh_accounts = vec![gh_account("alice")];
        o.gh_accounts_loaded = true;
        o.hub = hub;
    });
}

/// Replace the text of the input with test id `id`.
fn retype(h: &Harness, cx: &mut TestAppContext, id: &str, text: &str) {
    h.click(cx, id);
    h.keys(cx, "ctrl-a backspace");
    h.type_text(cx, text);
}

fn active_path(h: &Harness, cx: &mut TestAppContext) -> Option<std::path::PathBuf> {
    h.read(cx, |o, _| o.active_project().map(|p| p.path.clone()))
}

fn same_dir(a: &std::path::Path, b: &std::path::Path) -> bool {
    std::fs::canonicalize(a).ok() == std::fs::canonicalize(b).ok()
}

#[gpui::test]
fn ui_clone_a_repository_of_an_account(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    let source = TempRepo::new();
    source.commit_file("hello.txt", "hi\n", "hello");
    let bare = source.bare_clone();
    let mut gh_repo = crate::new_repo::tests::repo("alice", "hello");
    // A local address: no credentials are involved.
    gh_repo.clone_url = bare.to_string_lossy().into_owned();
    let hub = Arc::new(FakeHub {
        repos: vec![gh_repo, crate::new_repo::tests::repo("acme", "other")],
        ..Default::default()
    });
    let h = Harness::open(cx, &repo);
    with_hub(&h, cx, hub);
    // The palette entry and its account submenu.
    h.keys(cx, "ctrl-p");
    h.type_text(cx, "clone");
    h.click(cx, "palette-command:Clone repository…");
    assert!(h.bounds(cx, "palette-choice:alice").is_some());
    assert!(h.bounds(cx, "palette-choice:Other…").is_some());
    h.click(cx, "palette-choice:alice");
    assert!(h.bounds(cx, "palette-choice:hello").is_some());
    assert!(h.bounds(cx, "palette-choice:other").is_some());
    // Backspace goes back to the accounts, not the commands.
    h.keys(cx, "backspace");
    assert!(h.bounds(cx, "palette-choice:alice").is_some());
    h.click(cx, "palette-choice:alice");
    h.type_text(cx, "hel");
    assert!(h.bounds(cx, "palette-choice:other").is_none());
    h.keys(cx, "enter");
    // The directory dialog starts next to the open project.
    assert!(h.bounds(cx, "clone-form").is_some());
    assert!(h.bounds(cx, "clone-url").is_none());
    assert!(h.bounds(cx, "clone-choose-dir").is_some());
    let dest = tempfile::tempdir().expect("tempdir");
    retype(&h, cx, "clone-dir", &dest.path().to_string_lossy());
    h.click(cx, "dialog-ok");
    let target = dest.path().join("hello");
    assert!(target.join("hello.txt").exists(), "cloned");
    let path = active_path(&h, cx).expect("opened");
    assert!(same_dir(&path, &target));
    let key = path.to_string_lossy().into_owned();
    h.read(cx, |o, _| {
        assert_eq!(
            o.state.accounts.get(&key).map(String::as_str),
            Some("alice")
        );
        assert!(o.remote_job.is_none());
    });
}

#[gpui::test]
fn ui_clone_any_address_and_refuse_a_full_directory(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    let source = TempRepo::new();
    source.commit_file("x.txt", "x\n", "x");
    let bare = source.bare_clone();
    let h = Harness::open(cx, &repo);
    with_hub(&h, cx, Arc::new(FakeHub::default()));
    h.act(cx, |o, w, cx| {
        o.open_palette_in(crate::palette::PaletteMode::CloneAccount, w, cx)
    });
    h.click(cx, "palette-choice:Other…");
    assert!(h.bounds(cx, "clone-url").is_some());
    // The address field has focus.
    h.type_text(cx, &bare.to_string_lossy());
    let dest = tempfile::tempdir().expect("tempdir");
    let name = bare
        .file_name()
        .map(|n| n.to_string_lossy().trim_end_matches(".git").to_string())
        .expect("name");
    std::fs::create_dir(dest.path().join(&name)).expect("mkdir");
    std::fs::write(dest.path().join(&name).join("keep.txt"), "mine").expect("write");
    retype(&h, cx, "clone-dir", &dest.path().to_string_lossy());
    assert!(
        h.bounds(cx, "new-repo-error").is_some(),
        "full target flagged"
    );
    h.click(cx, "dialog-ok");
    assert!(h.has_settings(cx), "the dialog stays open");
    assert_eq!(
        std::fs::read_to_string(dest.path().join(&name).join("keep.txt")).expect("kept"),
        "mine"
    );
    // Into a fresh directory, without submodules.
    let fresh = dest.path().join("fresh");
    std::fs::create_dir(&fresh).expect("mkdir");
    retype(&h, cx, "clone-dir", &fresh.to_string_lossy());
    h.click(cx, "clone-recurse");
    h.click(cx, "dialog-ok");
    assert!(!h.has_settings(cx));
    let target = fresh.join(&name);
    assert!(target.join("x.txt").exists());
    assert!(same_dir(&active_path(&h, cx).expect("opened"), &target));
    // No account was picked, so none is remembered.
    h.read(cx, |o, _| assert!(o.state.accounts.is_empty()));
}

#[gpui::test]
fn ui_create_a_local_repository(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    let h = Harness::open(cx, &repo);
    with_hub(&h, cx, Arc::new(FakeHub::default()));
    h.keys(cx, "ctrl-p");
    h.type_text(cx, "create new");
    h.keys(cx, "enter");
    assert!(h.bounds(cx, "palette-choice:alice").is_some());
    h.click(cx, "palette-choice:No remote");
    assert!(h.bounds(cx, "create-form").is_some());
    // GitHub-only fields are hidden.
    assert!(h.bounds(cx, "create-owner").is_none());
    assert!(h.bounds(cx, "create-template").is_none());
    assert!(h.bounds(cx, "create-gitignore").is_some());
    // The name field has focus; a bad name is refused.
    h.type_text(cx, "bad/name");
    h.click(cx, "dialog-ok");
    assert!(h.has_settings(cx));
    assert!(h.bounds(cx, "new-repo-error").is_some());
    retype(&h, cx, "create-name", "fresh");
    retype(&h, cx, "create-description", "A new thing");
    let dest = tempfile::tempdir().expect("tempdir");
    retype(&h, cx, "create-dir", &dest.path().to_string_lossy());
    h.click(cx, "dialog-ok");
    assert!(!h.has_settings(cx));
    let target = dest.path().join("fresh");
    assert!(target.join(".git").is_dir());
    assert_eq!(
        std::fs::read_to_string(target.join("README.md")).expect("readme"),
        "# fresh\n\nA new thing\n"
    );
    assert!(!target.join(".gitignore").exists(), "None was picked");
    assert!(same_dir(&active_path(&h, cx).expect("opened"), &target));
}

#[gpui::test]
fn ui_create_on_github_then_clone(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    // What GitHub would have made: a repository with a README.
    let made = TempRepo::new();
    made.commit_file("README.md", "# shiny\n", "Initial commit");
    let bare = made.bare_clone();
    let hub = Arc::new(FakeHub {
        created_url: bare.to_string_lossy().into_owned(),
        ..Default::default()
    });
    let h = Harness::open(cx, &repo);
    with_hub(&h, cx, hub.clone());
    h.act(cx, |o, w, cx| {
        o.open_palette_in(crate::palette::PaletteMode::CreateAccount, w, cx)
    });
    h.click(cx, "palette-choice:alice");
    assert!(h.bounds(cx, "create-owner").is_some());
    assert!(h.bounds(cx, "create-template").is_some());
    assert!(h.bounds(cx, "create-status").is_none(), "lists loaded");
    h.type_text(cx, "shiny");
    let dest = tempfile::tempdir().expect("tempdir");
    retype(&h, cx, "create-dir", &dest.path().to_string_lossy());
    h.click(cx, "create-public");
    h.click(cx, "dialog-ok");
    let sent = hub.created.lock().expect("lock").clone().expect("created");
    assert_eq!(sent.owner, "alice");
    assert_eq!(sent.name, "shiny");
    assert!(!sent.private && sent.readme && sent.template.is_none());
    let target = dest.path().join("shiny");
    assert!(target.join("README.md").exists(), "cloned after creating");
    let path = active_path(&h, cx).expect("opened");
    assert!(same_dir(&path, &target));
    let key = path.to_string_lossy().into_owned();
    h.read(cx, |o, _| {
        assert_eq!(
            o.state.accounts.get(&key).map(String::as_str),
            Some("alice")
        )
    });
}

#[gpui::test]
fn a_local_repository_gets_the_picked_gitignore_and_licence(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    let h = Harness::open(cx, &repo);
    with_hub(&h, cx, Arc::new(FakeHub::default()));
    let dest = tempfile::tempdir().expect("tempdir");
    let target = dest.path().join("tidy");
    let plan = crate::new_repo::CreatePlan {
        account: None,
        lookup: Some(gh_account("alice")),
        new: osier_auth::hub::NewRepo {
            name: "tidy".into(),
            readme: false,
            gitignore: Some("Rust".into()),
            license: Some("mit".into()),
            ..Default::default()
        },
        target: target.clone(),
    };
    h.act(cx, move |o, w, cx| o.start_create(plan, w, cx));
    assert_eq!(
        std::fs::read_to_string(target.join(".gitignore")).expect("gitignore"),
        "# Rust\n/target\n"
    );
    let licence = std::fs::read_to_string(target.join("LICENSE")).expect("licence");
    assert!(licence.starts_with("MIT 20"), "{licence}");
    assert!(!target.join("README.md").exists());
    // Left uncommitted, for review in Changes.
    h.read(cx, |o, _| {
        let p = o.active_project().expect("opened");
        assert!(p.path.ends_with("tidy"));
        assert!(
            p.snapshot
                .as_ref()
                .is_some_and(|s| s.status.entries.len() == 2)
        );
    });
}

#[gpui::test]
fn opening_a_project_with_changes_shows_them(cx: &mut TestAppContext) {
    let (h, a, _b, first, _second) = two_projects(cx);
    h.click(cx, "tab-branches");
    // Opened fresh with changes: Changes, whatever tab was showing.
    let dirty = TempRepo::new();
    dirty.commit_file("d.txt", "d\n", "first");
    dirty.write("d.txt", "changed\n");
    let dirty_path = dirty.path().to_path_buf();
    h.act(cx, move |o, w, cx| {
        o.open_project_notify(&dirty_path, w, cx)
    });
    assert_eq!(tab(&h, cx), Tab::Changes);
    // Opened fresh and clean: the tab stays (Branches is never moved).
    h.click(cx, "tab-branches");
    let clean = TempRepo::new();
    clean.commit_file("c.txt", "c\n", "first");
    let clean_path = clean.path().to_path_buf();
    h.act(cx, move |o, w, cx| {
        o.open_project_notify(&clean_path, w, cx)
    });
    assert_eq!(tab(&h, cx), Tab::Branches);
    // Switching back to an open project with changes keeps the old behaviour: Branches.
    a.write("new.txt", "x\n");
    h.act(cx, |o, _, cx| o.refresh_project(0, cx));
    let a_path = a.path().to_path_buf();
    h.act(cx, move |o, w, cx| o.switch_to_project(&a_path, w, cx));
    assert_eq!(active_name(&h, cx), first);
    assert_eq!(tab(&h, cx), Tab::Branches);
    // From the recent list (closed, then reopened) it counts as opening again.
    h.click(cx, "tab-log");
    h.act(cx, |o, w, cx| o.close_project(0, w, cx));
    let a_path = a.path().to_path_buf();
    h.act(cx, move |o, w, cx| o.switch_to_project(&a_path, w, cx));
    assert_eq!(active_name(&h, cx), first);
    assert_eq!(tab(&h, cx), Tab::Changes);
}

#[gpui::test]
fn background_tasks_show_only_when_asked(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    let h = Harness::open(cx, &repo);
    let guard = h.read(cx, |o, _| o.background.begin(crate::background::GH_LISTS));
    h.act(cx, |_, _, cx| cx.notify());
    assert!(h.bounds(cx, "background-tasks").is_none(), "off by default");
    h.act(cx, |o, _, cx| {
        o.settings.appearance.show_background_tasks = true;
        cx.notify();
    });
    assert!(h.bounds(cx, "background-tasks").is_some());
    h.read(cx, |o, _| {
        assert!(
            o.background_labels()
                .contains(&crate::background::GH_LISTS.to_string())
        )
    });
    // Dropping the guard (the work finished or its dialog closed) counts it out.
    drop(guard);
    h.act(cx, |_, _, cx| cx.notify());
    assert!(h.bounds(cx, "background-tasks").is_none());
}

#[gpui::test]
fn the_clone_submenu_waits_for_the_gh_accounts(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    let h = Harness::open(cx, &repo);
    h.act(cx, |o, w, cx| {
        o.gh_accounts.clear();
        o.gh_accounts_loaded = false;
        o.hub = Arc::new(FakeHub::default());
        o.open_palette_in(crate::palette::PaletteMode::CloneAccount, w, cx)
    });
    assert!(h.bounds(cx, "palette-status").is_some());
    assert!(h.bounds(cx, "palette-choice:alice").is_none());
    h.act(cx, |o, _, _| {
        o.gh_accounts = vec![gh_account("alice")];
        o.gh_accounts_loaded = true;
    });
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(400));
    cx.run_until_parked();
    assert!(h.bounds(cx, "palette-status").is_none());
    assert!(h.bounds(cx, "palette-choice:alice").is_some());
}

#[gpui::test]
fn ui_clone_waits_for_the_target_check_and_refuses_a_full_folder(cx: &mut TestAppContext) {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "a\n", "first");
    let source = TempRepo::new();
    source.commit_file("hello.txt", "hi\n", "hello");
    let bare = source.bare_clone();
    let mut gh_repo = crate::new_repo::tests::repo("alice", "hello");
    gh_repo.clone_url = bare.to_string_lossy().into_owned();
    let hub = Arc::new(FakeHub {
        repos: vec![gh_repo],
        ..Default::default()
    });
    let h = Harness::open(cx, &repo);
    with_hub(&h, cx, hub);
    h.keys(cx, "ctrl-p");
    h.type_text(cx, "clone");
    h.click(cx, "palette-command:Clone repository…");
    h.click(cx, "palette-choice:alice");
    h.click(cx, "palette-choice:hello");
    // A target folder that already has files: an error notice, and Clone does nothing.
    let dest = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(dest.path().join("hello")).expect("mkdir");
    std::fs::write(dest.path().join("hello/keep.txt"), "mine").expect("write");
    retype(&h, cx, "clone-dir", &dest.path().to_string_lossy());
    assert!(
        h.bounds(cx, "new-repo-error").is_some(),
        "the folder check failed"
    );
    h.click(cx, "dialog-ok");
    assert!(h.has_settings(cx), "the dialog stays open");
    assert!(
        !dest.path().join("hello/hello.txt").exists(),
        "nothing was cloned"
    );
    // An empty folder passes, and Clone goes ahead.
    let other = tempfile::tempdir().expect("tempdir");
    retype(&h, cx, "clone-dir", &other.path().to_string_lossy());
    assert!(h.bounds(cx, "new-repo-error").is_none());
    assert!(h.bounds(cx, "new-repo-target").is_some());
    h.click(cx, "dialog-ok");
    assert!(other.path().join("hello/hello.txt").exists(), "cloned");
}
