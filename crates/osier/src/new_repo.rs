//! "Clone repository…" and "Create new repository…".
//!
//! Both start in the palette (`PaletteMode::CloneAccount`, `CreateAccount`): pick a gh account
//! (or "Other…" / "No remote"), for a clone then a repository. A dialog then asks where it
//! goes: a directory field that starts at the parent of the open project, plus "Choose
//! directory…" for the portal folder picker. The new working tree is always opened.
//!
//! GitHub clones go over HTTPS with Osier's credential helper for the chosen account, and that
//! account is remembered for the project. GitHub calls go through [`Hub`] so tests never reach
//! gh or the network.

use crate::app_dialog::AppDialogExt as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};

use crate::checkbox::Checkbox;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::Button;
use gpui_kit::component::dialog::DialogButtonProps;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::radio::{Radio, RadioGroup};
use gpui_kit::component::searchable_list::SearchableVec;
use gpui_kit::component::select::{Select, SelectState};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IndexPath, Sizable as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use osier_auth::GhAccount;
use osier_auth::hub::{CreatedRepo, GhRepo, License, NewRepo};
use osier_config::history::HistoryEntry;
use osier_git::ops::remote::{self, CloneOptions, RemoteContext};
use osier_git::{CancelToken, GitError, Progress};

use crate::app::Osier;
use crate::cached_list::{CachedLists, Freshness};
use crate::notices::{Notice, WindowNoticeExt as _};
use crate::palette::{Choice, ChoicePick};
use crate::test_ids::TestId as _;

// ----- GitHub access -----

/// What the clone and create flows ask GitHub for. Errors are already user-facing text.
pub(crate) trait Hub: Send + Sync {
    fn repos(&self, account: &GhAccount) -> Result<Vec<GhRepo>, String>;
    fn orgs(&self, account: &GhAccount) -> Result<Vec<String>, String>;
    /// Template repositories of `owners` (the account first, then its organisations).
    fn templates(&self, account: &GhAccount, owners: &[String]) -> Result<Vec<String>, String>;
    fn gitignore_templates(&self, account: &GhAccount) -> Result<Vec<String>, String>;
    fn licenses(&self, account: &GhAccount) -> Result<Vec<License>, String>;
    fn gitignore_template(&self, account: &GhAccount, name: &str) -> Result<String, String>;
    fn license_text(
        &self,
        account: &GhAccount,
        key: &str,
        year: i32,
        holder: &str,
    ) -> Result<String, String>;
    fn create(&self, account: &GhAccount, new: &NewRepo) -> Result<CreatedRepo, String>;
}

/// The real one: `gh api` as the chosen account, through `osier --gh-api`. Every call goes
/// to the git command log like a git command (the helper's argv, never a token).
#[cfg_attr(test, allow(dead_code))]
struct GhHub {
    log: Arc<dyn osier_git::CommandSink>,
}

#[cfg_attr(test, allow(dead_code))]
impl GhHub {
    fn with<T>(
        &self,
        a: &GhAccount,
        call: impl FnOnce(&osier_auth::hub::Gh) -> osier_auth::Result<T>,
    ) -> Result<T, String> {
        let exe = crate::runtime_info::own_binary()
            .map_err(|e| format!("cannot find the Osier binary: {e}"))?;
        let observe = |c: &osier_auth::hub::ApiCall| {
            // Shown like the git rows: the program by name, not its full path.
            let mut argv = c.argv.clone();
            if let Some(first) = argv.first_mut() {
                *first = "osier".into();
            }
            self.log.record(&osier_git::CommandRecord {
                id: 0,
                argv,
                cwd: None,
                started: c.started,
                duration: c.duration,
                status: c.status,
                expected: c.status == Some(0),
                stdout_len: c.stdout_len,
                stderr: c.stderr.clone(),
            })
        };
        let gh = osier_auth::hub::Gh {
            exe: &exe,
            login: &a.login,
            host: &a.host,
            observer: Some(&observe),
        };
        call(&gh).map_err(|e| e.to_string())
    }
}

impl Hub for GhHub {
    fn repos(&self, a: &GhAccount) -> Result<Vec<GhRepo>, String> {
        self.with(a, osier_auth::hub::repos)
    }
    fn orgs(&self, a: &GhAccount) -> Result<Vec<String>, String> {
        self.with(a, osier_auth::hub::orgs)
    }
    fn templates(&self, a: &GhAccount, owners: &[String]) -> Result<Vec<String>, String> {
        self.with(a, |gh| osier_auth::hub::templates(gh, owners))
    }
    fn gitignore_templates(&self, a: &GhAccount) -> Result<Vec<String>, String> {
        self.with(a, osier_auth::hub::gitignore_templates)
    }
    fn licenses(&self, a: &GhAccount) -> Result<Vec<License>, String> {
        self.with(a, osier_auth::hub::licenses)
    }
    fn gitignore_template(&self, a: &GhAccount, name: &str) -> Result<String, String> {
        self.with(a, |gh| osier_auth::hub::gitignore_template(gh, name))
    }
    fn license_text(
        &self,
        a: &GhAccount,
        key: &str,
        year: i32,
        holder: &str,
    ) -> Result<String, String> {
        self.with(a, |gh| osier_auth::hub::license_text(gh, key, year, holder))
    }
    fn create(&self, a: &GhAccount, new: &NewRepo) -> Result<CreatedRepo, String> {
        self.with(a, |gh| osier_auth::hub::create(gh, new))
    }
}

/// Tests start with a hub that refuses everything; the ones that need GitHub set a fake.
#[cfg(test)]
struct NoHub;

#[cfg(test)]
impl Hub for NoHub {
    fn repos(&self, _: &GhAccount) -> Result<Vec<GhRepo>, String> {
        Err("no GitHub in tests".into())
    }
    fn orgs(&self, _: &GhAccount) -> Result<Vec<String>, String> {
        Err("no GitHub in tests".into())
    }
    fn templates(&self, _: &GhAccount, _: &[String]) -> Result<Vec<String>, String> {
        Err("no GitHub in tests".into())
    }
    fn gitignore_templates(&self, _: &GhAccount) -> Result<Vec<String>, String> {
        Err("no GitHub in tests".into())
    }
    fn licenses(&self, _: &GhAccount) -> Result<Vec<License>, String> {
        Err("no GitHub in tests".into())
    }
    fn gitignore_template(&self, _: &GhAccount, _: &str) -> Result<String, String> {
        Err("no GitHub in tests".into())
    }
    fn license_text(&self, _: &GhAccount, _: &str, _: i32, _: &str) -> Result<String, String> {
        Err("no GitHub in tests".into())
    }
    fn create(&self, _: &GhAccount, _: &NewRepo) -> Result<CreatedRepo, String> {
        Err("no GitHub in tests".into())
    }
}

pub(crate) fn default_hub(log: Arc<dyn osier_git::CommandSink>) -> Arc<dyn Hub> {
    #[cfg(test)]
    {
        let _ = log;
        Arc::new(NoHub)
    }
    #[cfg(not(test))]
    {
        Arc::new(GhHub { log })
    }
}

impl Osier {
    /// The repositories `account` can clone (`CheckNow` when the clone list opens).
    pub(crate) fn use_gh_repos(
        &mut self,
        account: &GhAccount,
        freshness: Freshness,
        cx: &mut Context<Self>,
    ) {
        let (hub, a) = (self.hub.clone(), account.clone());
        self.use_list(
            |o| &mut o.gh_lists.repos,
            &account_key(account),
            freshness,
            Arc::new(move || hub.repos(&a)),
            cx,
        );
    }

    /// The Create dialog's lists: owners and templates for `account` (creating on GitHub),
    /// `.gitignore` templates and licences from `source` (any logged-in account).
    pub(crate) fn use_gh_create_lists(
        &mut self,
        account: Option<&GhAccount>,
        source: &GhAccount,
        freshness: Freshness,
        cx: &mut Context<Self>,
    ) {
        if let Some(account) = account {
            let key = account_key(account);
            let (hub, a) = (self.hub.clone(), account.clone());
            self.use_list(
                |o| &mut o.gh_lists.orgs,
                &key,
                freshness,
                Arc::new(move || hub.orgs(&a)),
                cx,
            );
            // One search over the account and its organisations, not a read of every
            // repository it can reach (hundreds for some accounts).
            let (hub, a) = (self.hub.clone(), account.clone());
            self.use_list(
                |o| &mut o.gh_lists.templates,
                &key,
                freshness,
                Arc::new(move || {
                    let mut owners = vec![a.login.clone()];
                    owners.extend(hub.orgs(&a).unwrap_or_default());
                    hub.templates(&a, &owners)
                }),
                cx,
            );
        }
        let key = account_key(source);
        let (hub, a) = (self.hub.clone(), source.clone());
        self.use_list(
            |o| &mut o.gh_lists.gitignores,
            &key,
            freshness,
            Arc::new(move || hub.gitignore_templates(&a)),
            cx,
        );
        let (hub, a) = (self.hub.clone(), source.clone());
        self.use_list(
            |o| &mut o.gh_lists.licenses,
            &key,
            freshness,
            Arc::new(move || hub.licenses(&a)),
            cx,
        );
    }

    /// Make every account's repository list ready (disk copy, then a refresh when older than
    /// the timer's interval, or 5 minutes with the timer off): entering the clone or create
    /// submenus, so the list is there by the time an account is picked.
    pub(crate) fn warm_gh_repos(&mut self, cx: &mut Context<Self>) {
        let minutes = match self.settings.github.list_refresh_minutes {
            0 => 5,
            m => m,
        };
        let freshness =
            Freshness::OlderThan(std::time::Duration::from_secs(u64::from(minutes) * 60));
        for account in self.gh_accounts.clone() {
            self.use_gh_repos(&account, freshness, cx);
        }
    }

    /// Settings > Accounts > GitHub lists > Refresh now: every account's lists, at once.
    pub(crate) fn refresh_gh_lists_now(&mut self, cx: &mut Context<Self>) {
        for account in self.gh_accounts.clone() {
            self.use_gh_repos(&account, Freshness::CheckNow, cx);
            self.use_gh_create_lists(Some(&account), &account, Freshness::CheckNow, cx);
        }
    }

    /// Whether any GitHub list is being refreshed.
    pub(crate) fn gh_lists_refreshing(&self) -> bool {
        self.gh_lists.any_refreshing()
    }

    /// The tick loop's refresh of the GitHub lists (`github.list_refresh_minutes`, 0 = off):
    /// a list older than that is fetched again, one at a time, starting a little after
    /// launch so startup has the machine to itself.
    pub(crate) fn refresh_lists_on_timer(&mut self, cx: &mut Context<Self>) {
        const AFTER_LAUNCH: std::time::Duration = std::time::Duration::from_secs(20);
        let minutes = self.settings.github.list_refresh_minutes;
        if minutes == 0
            || !self.gh_accounts_loaded
            || self.started.elapsed() < AFTER_LAUNCH
            || self.gh_lists.any_refreshing()
        {
            return;
        }
        let freshness =
            Freshness::OlderThan(std::time::Duration::from_secs(u64::from(minutes) * 60));
        let accounts = self.gh_accounts.clone();
        for account in &accounts {
            self.use_gh_repos(account, freshness, cx);
            if self.gh_lists.any_refreshing() {
                return;
            }
            self.use_gh_create_lists(Some(account), account, freshness, cx);
            if self.gh_lists.any_refreshing() {
                return;
            }
        }
    }
}

/// The GitHub lists Osier keeps ready (`cached_list.rs`), by account.
pub(crate) struct GhLists {
    pub(crate) repos: CachedLists<Vec<GhRepo>>,
    pub(crate) orgs: CachedLists<Vec<String>>,
    pub(crate) templates: CachedLists<Vec<String>>,
    pub(crate) gitignores: CachedLists<Vec<String>>,
    pub(crate) licenses: CachedLists<Vec<License>>,
}

impl Default for GhLists {
    fn default() -> Self {
        Self {
            repos: CachedLists::new("github-repos", crate::background::GH_REPOS),
            orgs: CachedLists::new("github-orgs", crate::background::GH_LISTS),
            templates: CachedLists::new("github-templates", crate::background::GH_LISTS),
            gitignores: CachedLists::new("github-gitignores", crate::background::GH_LISTS),
            licenses: CachedLists::new("github-licenses", crate::background::GH_LISTS),
        }
    }
}

impl GhLists {
    fn any_refreshing(&self) -> bool {
        self.repos.any_refreshing()
            || self.orgs.any_refreshing()
            || self.templates.any_refreshing()
            || self.gitignores.any_refreshing()
            || self.licenses.any_refreshing()
    }
}

/// Key of an account in the GitHub lists (`GhLists`).
pub(crate) fn account_key(a: &GhAccount) -> String {
    format!("{}@{}", a.login, a.host)
}

// ----- palette rows -----

/// The account rows of "Clone repository…" (`clone`) or "Create new repository…".
pub(crate) fn account_choices(osier: &Osier, clone: bool) -> Vec<Choice> {
    let section: SharedString = "GitHub accounts".into();
    let mut rows: Vec<Choice> = osier
        .gh_accounts
        .iter()
        .map(|a| Choice {
            section: section.clone(),
            label: a.login.clone().into(),
            detail: Some(
                if a.active {
                    format!("{} (gh's active account)", a.host)
                } else {
                    a.host.clone()
                }
                .into(),
            ),
            icon: Lucide::Github,
            pick: if clone {
                ChoicePick::CloneAccount(a.clone())
            } else {
                ChoicePick::CreateAccount(a.clone())
            },
            action: None,
        })
        .collect();
    let other: SharedString = "Other".into();
    rows.push(if clone {
        Choice {
            section: other,
            label: "Other…".into(),
            detail: Some("Any HTTPS or SSH clone address".into()),
            icon: Lucide::Link,
            pick: ChoicePick::CloneOther,
            action: None,
        }
    } else {
        Choice {
            section: other,
            label: "No remote".into(),
            detail: Some("A local repository (git init)".into()),
            icon: Lucide::HardDrive,
            pick: ChoicePick::CreateLocal,
            action: None,
        }
    });
    rows
}

/// The repositories of `account`, grouped by owner: its own first, then the rest by name.
pub(crate) fn repo_choices(account: &GhAccount, repos: &[GhRepo]) -> Vec<Choice> {
    let mut sorted: Vec<&GhRepo> = repos.iter().collect();
    sorted.sort_by(|a, b| {
        let own = |r: &GhRepo| !r.owner.eq_ignore_ascii_case(&account.login);
        own(a)
            .cmp(&own(b))
            .then_with(|| a.owner.to_lowercase().cmp(&b.owner.to_lowercase()))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    sorted
        .into_iter()
        .map(|r| {
            let mut notes: Vec<String> = Vec::new();
            if r.private {
                notes.push("private".into());
            }
            if r.fork {
                notes.push("fork".into());
            }
            if r.archived {
                notes.push("archived".into());
            }
            if r.is_template {
                notes.push("template".into());
            }
            if let Some(d) = r.description.as_deref().filter(|d| !d.trim().is_empty()) {
                notes.push(d.trim().to_string());
            }
            Choice {
                section: r.owner.clone().into(),
                label: r.name.clone().into(),
                detail: (!notes.is_empty()).then(|| notes.join(" · ").into()),
                icon: if r.private {
                    Lucide::Lock
                } else {
                    Lucide::BookMarked
                },
                pick: ChoicePick::CloneRepo(account.clone(), r.clone()),
                action: None,
            }
        })
        .collect()
}

// ----- shared form pieces -----

/// The directory a new repository goes in by default: next to the open project, else the
/// first scan directory, else home.
fn default_parent(osier: &Osier) -> PathBuf {
    if let Some(parent) = osier.active_project().and_then(|p| p.path.parent()) {
        return parent.to_path_buf();
    }
    if let Some(dir) = osier.settings.projects.scan_dirs.first() {
        return osier_config::scan::expand_home(dir);
    }
    osier_config::paths::home_dirs()
        .into_iter()
        .next()
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// The directory field's text as an absolute path (`~` expanded).
fn parse_dir(text: &str) -> Result<PathBuf, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("Pick a directory.".into());
    }
    let path = osier_config::scan::expand_home(Path::new(text));
    if !path.is_absolute() {
        return Err(format!("{text} is not an absolute path."));
    }
    Ok(path)
}

/// The folder name a clone gets: the address's last part without `.git`.
pub(crate) fn name_from_url(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches('/');
    let last = url.rsplit(['/', ':']).next()?;
    let name = last.strip_suffix(".git").unwrap_or(last);
    (!name.is_empty() && name != "." && name != "..").then(|| name.to_string())
}

/// Characters GitHub keeps in a repository name; the folder gets the same name.
fn check_repo_name(name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Name the repository.".into());
    }
    if name == "." || name == ".." {
        return Err(format!("{name} cannot be a repository name."));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
    {
        return Err("Use letters, digits, '-', '_' and '.' only.".into());
    }
    Ok(())
}

/// The target must be missing or an empty directory (checked again by the git layer).
fn check_target(target: &Path) -> Result<(), String> {
    // Shown under the form: the path as everywhere else in Osier (`~/...`).
    remote::check_empty_target(target).map_err(|e| {
        e.to_string()
            .replace(&target.display().to_string(), &crate::format::tilde(target))
    })
}

/// Whether a form's target folder can take the new repository (missing or empty), checked in
/// the background once per target path: listing a folder on every frame would be file work on
/// the UI thread, and it would also see the clone's own files appear while it runs.
#[derive(Default)]
struct TargetCheck {
    target: Option<PathBuf>,
    result: Option<Result<(), String>>,
    _task: Option<Task<()>>,
}

impl TargetCheck {
    /// The result for `target`, `None` while it is being checked. A new target starts a
    /// check; `slot` finds this check in the form when the result comes back.
    fn status<V: 'static>(
        &mut self,
        target: &Path,
        slot: fn(&mut V) -> &mut TargetCheck,
        cx: &mut Context<V>,
    ) -> Option<Result<(), String>> {
        if self.target.as_deref() != Some(target) {
            self.target = Some(target.to_path_buf());
            self.result = None;
            let checked = target.to_path_buf();
            self._task = Some(cx.spawn(async move |this, cx| {
                let path = checked.clone();
                let result = cx
                    .background_spawn(async move { check_target(&path) })
                    .await;
                let _ = this.update(cx, |form, cx| {
                    let check = slot(form);
                    if check.target.as_deref() == Some(checked.as_path()) {
                        check.result = Some(result);
                        cx.notify();
                    }
                });
            }));
        }
        self.result.clone()
    }
}

/// Directory field plus "Choose directory…" (portal folder picker).
fn directory_row(dir: &Entity<InputState>, prefix: &'static str) -> impl IntoElement {
    let target = dir.clone();
    h_flex()
        .w_full()
        .gap_2()
        .child(
            div()
                .test_id(|| format!("{prefix}-dir"))
                .flex_1()
                .min_w_0()
                .child(Input::new(dir)),
        )
        .child(
            Button::new(SharedString::from(format!("{prefix}-choose-dir")))
                .test_id(|| format!("{prefix}-choose-dir"))
                .outline()
                .icon(Lucide::FolderOpen)
                .label("Choose directory…")
                .on_click(move |_, window, cx| choose_directory(target.clone(), window, cx)),
        )
}

fn choose_directory(dir: Entity<InputState>, window: &mut Window, cx: &mut App) {
    let rx = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some("Choose directory".into()),
    });
    window
        .spawn(cx, async move |cx| {
            let Ok(Ok(Some(paths))) = rx.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = cx.update(|window, cx| {
                dir.update(cx, |d, cx| {
                    d.set_value(crate::format::tilde(&path), window, cx)
                });
            });
        })
        .detach();
}

fn field_label(text: &'static str, cx: &App) -> Div {
    div()
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .text_color(cx.theme().muted_foreground)
        .child(text)
}

/// "Clones into ~/Work/app" under the form, or what is wrong as an error notice.
fn outcome_line(text: Result<String, String>, cx: &App) -> AnyElement {
    match text {
        Ok(t) => div()
            .test_id("new-repo-target")
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(t)
            .into_any_element(),
        Err(e) => crate::dialog_notice::DialogNotice::error(e)
            .test_id("new-repo-error")
            .into_any_element(),
    }
}

// ----- clone -----

/// What a clone needs once the dialog is confirmed.
#[derive(Debug, Clone)]
pub(crate) struct ClonePlan {
    pub url: String,
    pub target: PathBuf,
    pub recurse_submodules: bool,
    /// The gh account picked in the palette: used for the credentials and remembered.
    pub account: Option<GhAccount>,
    /// "owner/name" or the folder name, for messages.
    pub label: String,
}

/// The clone dialog's form.
pub(crate) struct CloneForm {
    /// The repository picked in the palette; `None` for "Other…" (an address field instead).
    source: Option<(GhAccount, GhRepo)>,
    url: Entity<InputState>,
    dir: Entity<InputState>,
    recurse: bool,
    target_check: TargetCheck,
    /// Everything checked and fine: Clone is enabled.
    ready: bool,
    /// Confirmed: what the form says while the dialog closes (nothing is checked any more).
    submitted: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl CloneForm {
    fn new(
        source: Option<(GhAccount, GhRepo)>,
        parent: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let url = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("https://github.com/owner/repo.git or git@github.com:owner/repo.git")
        });
        let dir =
            cx.new(|cx| InputState::new(window, cx).default_value(crate::format::tilde(parent)));
        let watch = |cx: &mut Context<Self>, input: &Entity<InputState>| {
            cx.subscribe(input, |_: &mut Self, _, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Change) {
                    cx.notify();
                }
            })
        };
        let _subscriptions = vec![watch(cx, &url), watch(cx, &dir)];
        Self {
            source,
            url,
            dir,
            recurse: true,
            target_check: TargetCheck::default(),
            ready: false,
            submitted: None,
            _subscriptions,
        }
    }

    fn plan(&self, cx: &App) -> Result<ClonePlan, String> {
        let (url, account, label) = match &self.source {
            Some((account, repo)) => (
                repo.clone_url.clone(),
                Some(account.clone()),
                repo.full_name.clone(),
            ),
            None => {
                let url = self.url.read(cx).value().trim().to_string();
                if url.is_empty() {
                    return Err("Enter the repository's clone address.".into());
                }
                let label = name_from_url(&url).unwrap_or_else(|| url.clone());
                (url, None, label)
            }
        };
        let name = match &self.source {
            Some((_, repo)) => repo.name.clone(),
            None => name_from_url(&url)
                .ok_or_else(|| format!("Cannot tell the folder name from {url}."))?,
        };
        let target = parse_dir(&self.dir.read(cx).value())?.join(name);
        Ok(ClonePlan {
            url,
            target,
            recurse_submodules: self.recurse,
            account,
            label,
        })
    }
}

impl CloneForm {
    /// The line under the form (muted text, or an error), and whether Clone may go ahead.
    fn outcome(&mut self, cx: &mut Context<Self>) -> Result<String, String> {
        if let Some(text) = &self.submitted {
            return Ok(text.clone());
        }
        let (ready, outcome) = match self.plan(cx) {
            // Before anything is typed, say what is needed without alarm.
            Err(e) if self.source.is_none() && self.url.read(cx).value().is_empty() => {
                (false, Ok(e))
            }
            Err(e) => (false, Err(e)),
            Ok(plan) => {
                let target = crate::format::tilde(&plan.target);
                match self
                    .target_check
                    .status(&plan.target, |f: &mut Self| &mut f.target_check, cx)
                {
                    None => (false, Ok(format!("Checking {target}…"))),
                    Some(Ok(())) => (true, Ok(format!("Clones into {target}"))),
                    Some(Err(e)) => (false, Err(e)),
                }
            }
        };
        if self.ready != ready {
            // The dialog reads `ready` for its button before this form redraws in the same
            // frame, so ask for one more frame. Deferred: gpui drops a notify made while the
            // window is drawing.
            self.ready = ready;
            let form = cx.entity();
            cx.defer(move |cx| form.update(cx, |_, cx| cx.notify()));
        }
        outcome
    }

    /// The plan to run when Clone is pressed, `None` while checks are not done or failed.
    /// Freezes the form: it stops checking (the clone fills the folder it checked).
    fn confirm(&mut self, cx: &mut Context<Self>) -> Option<ClonePlan> {
        let ready = self.outcome(cx).is_ok() && self.ready;
        let plan = self.plan(cx).ok().filter(|_| ready)?;
        self.submitted = Some(format!(
            "Cloning into {}…",
            crate::format::tilde(&plan.target)
        ));
        cx.notify();
        Some(plan)
    }
}

impl Render for CloneForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let outcome = self.outcome(cx);
        v_flex()
            .test_id("clone-form")
            .w_full()
            .gap_3()
            .child(match &self.source {
                Some((account, repo)) => div()
                    .text_sm()
                    .text_color(muted)
                    .child(format!(
                        "{} over HTTPS as {}. Osier remembers the account for this project.",
                        repo.full_name, account.login
                    ))
                    .into_any_element(),
                None => v_flex()
                    .gap_1()
                    .child(field_label("Repository address", cx))
                    .child(div().test_id("clone-url").child(Input::new(&self.url)))
                    .into_any_element(),
            })
            .child(
                v_flex()
                    .gap_1()
                    .child(field_label("Directory", cx))
                    .child(directory_row(&self.dir, "clone")),
            )
            .child(
                Checkbox::new("clone-recurse")
                    .test_id("clone-recurse")
                    .small()
                    .label("Also clone submodules")
                    .checked(self.recurse)
                    .on_click(cx.listener(|this, v: &bool, _, cx| {
                        this.recurse = *v;
                        cx.notify();
                    }))
                    .child(crate::dialog_hint::hint(
                        "Submodules are other repositories this one includes. Without this, their \
                         folders stay empty until you fetch them.",
                        cx,
                    )),
            )
            .child(outcome_line(outcome, cx))
    }
}

impl Osier {
    /// The clone dialog for a palette pick, or with an address field for "Other…".
    pub(crate) fn prompt_clone(
        &mut self,
        source: Option<(GhAccount, GhRepo)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let parent = default_parent(self);
        let title: SharedString = match &source {
            Some((_, repo)) => format!("Clone {}", repo.full_name).into(),
            None => "Clone repository".into(),
        };
        let has_url = source.is_none();
        let form = cx.new(|cx| CloneForm::new(source, &parent, window, cx));
        let weak = cx.entity().downgrade();
        let focus = form.clone();
        window.open_app_dialog(cx, move |dialog, _, cx| {
            let (form, weak) = (form.clone(), weak.clone());
            let ready = form.read(cx).ready;
            dialog
                .title(title.clone())
                .w(px(560.))
                .child(form.clone())
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Clone")
                        .show_cancel(true),
                )
                // Disabled until every check (the target folder in the background) passed.
                .footer(crate::dialog_buttons::DialogButtons::new("Clone").disabled(!ready))
                .on_ok(
                    move |_, window, cx| match form.update(cx, |f, cx| f.confirm(cx)) {
                        Some(plan) => {
                            let _ = weak.update(cx, |this, cx| this.start_clone(plan, window, cx));
                            true
                        }
                        // Enter before the checks are done, or with a problem shown: stay.
                        None => false,
                    },
                )
        });
        focus.update(cx, |f, cx| {
            let input = if has_url { &f.url } else { &f.dir };
            input.update(cx, |i, cx| i.focus(window, cx));
        });
    }

    /// Clone in the background with progress in the status bar, then open the result.
    pub(crate) fn start_clone(
        &mut self,
        plan: ClonePlan,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(job) = &self.remote_job {
            window.push_notice(
                Notice::warning(format!("{} is still running", job.label)),
                cx,
            );
            return;
        }
        let ctx = self.clone_context(&plan);
        let (tx, rx) = mpsc::channel();
        let label = format!("Clone {}", plan.label);
        self.remote_job = Some(crate::app::remote_ops::RemoteJob::new(
            label.clone(),
            ctx.cancel.clone().unwrap_or_default(),
            rx,
        ));
        cx.notify();
        let runner = self.runner.clone();
        let options = CloneOptions {
            url: plan.url.clone(),
            target: plan.target.clone(),
            recurse_submodules: plan.recurse_submodules,
        };
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    remote::clone(&runner, &options, &ctx, move |p: Progress| {
                        let _ = tx.send(p);
                    })
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.remote_job = None;
                this.finish_new_repo(
                    &plan.target,
                    &label,
                    plan.account.as_ref(),
                    result,
                    window,
                    cx,
                );
            });
        })
        .detach();
    }

    /// Credentials for a clone: the picked account's helper over HTTPS; for an address typed
    /// in, the gh account for that host if there is exactly one obvious choice.
    fn clone_context(&self, plan: &ClonePlan) -> RemoteContext {
        let mut ctx = RemoteContext {
            cancel: Some(CancelToken::new()),
            ..Default::default()
        };
        let Ok(exe) = crate::runtime_info::own_binary() else {
            return ctx;
        };
        if self.settings.auth.askpass {
            ctx.env = osier_auth::askpass_env(&exe);
        }
        if let osier_auth::RemoteKind::Https { host } = osier_auth::classify_url(&plan.url) {
            let login = match &plan.account {
                Some(a) => Some(a.login.clone()),
                None => {
                    let on_host: Vec<&GhAccount> =
                        self.gh_accounts.iter().filter(|a| a.host == host).collect();
                    self.settings
                        .auth
                        .default_accounts
                        .get(&host)
                        .filter(|l| on_host.iter().any(|a| &&a.login == l))
                        .cloned()
                        .or_else(|| (on_host.len() == 1).then(|| on_host[0].login.clone()))
                }
            };
            if let Some(login) = login {
                ctx.config = osier_auth::credential_config(&exe, &login, &host);
            }
        }
        ctx
    }

    /// After a clone or create: record it, open the new project and remember the account.
    fn finish_new_repo(
        &mut self,
        target: &Path,
        label: &str,
        account: Option<&GhAccount>,
        result: Result<String, GitError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut entry = HistoryEntry::now(target, label.to_string());
        if let Err(e) = &result {
            entry.ok = false;
            entry.error = Some(crate::app::ops::describe(e));
        }
        self.record_history(entry, cx);
        match result {
            Ok(_) => {
                window.push_notice(
                    Notice::success(format!("In {}", crate::format::tilde(target)))
                        .title(label.to_string()),
                    cx,
                );
                let target_dir = target.to_path_buf();
                let login = account.map(|a| a.login.clone());
                self.open_project_then(
                    target,
                    move |this, ix, _, _| {
                        if let Some(login) = login
                            && let Some(project) = this.projects.get(ix)
                            && project.path.starts_with(&target_dir)
                        {
                            let key = project.path.to_string_lossy().into_owned();
                            this.set_project_account(&key, &login, true);
                        }
                    },
                    window,
                    cx,
                );
            }
            Err(GitError::Cancelled(_)) => {
                window.push_notice(format!("{label}: cancelled"), cx);
            }
            Err(e) => window.push_notice(
                Notice::warning(crate::app::ops::describe(&e)).title(format!("{label} failed")),
                cx,
            ),
        }
        cx.notify();
    }
}

// ----- create -----

type Pick = Entity<SelectState<SearchableVec<SharedString>>>;

const NONE: &str = "None";
const NO_TEMPLATE: &str = "No template";

/// What a create needs once the dialog is confirmed.
#[derive(Debug, Clone)]
pub(crate) struct CreatePlan {
    /// `None`: a local repository.
    pub account: Option<GhAccount>,
    /// Account used to fetch .gitignore and licence texts for a local repository.
    pub lookup: Option<GhAccount>,
    pub new: NewRepo,
    pub target: PathBuf,
}

/// What the create form starts from (read from the main view before it is built).
struct FormSeed {
    account: Option<GhAccount>,
    lookup: Option<GhAccount>,
    /// The app view, whose cached GitHub lists (`gh_lists`) fill the form.
    app: Entity<Osier>,
    parent: PathBuf,
}

/// One cached list as the Create form sees it.
struct ListState<T> {
    /// The form uses this list (owners and templates only when creating on GitHub).
    wanted: bool,
    revision: u64,
    data: Option<Arc<T>>,
    checking: bool,
    error: Option<String>,
}

impl<T> ListState<T> {
    /// For a list the form uses: (has data, being checked, last error).
    fn summary(&self) -> Option<(bool, bool, Option<String>)> {
        self.wanted
            .then(|| (self.data.is_some(), self.checking, self.error.clone()))
    }

    fn read(lists: &CachedLists<T>, key: Option<String>) -> Self {
        let Some(key) = key else {
            return Self {
                wanted: false,
                revision: 0,
                data: None,
                checking: false,
                error: None,
            };
        };
        match lists.get(&key) {
            Some(e) => Self {
                wanted: true,
                revision: e.revision,
                data: e.data.clone(),
                checking: e.refreshing,
                error: e.error.clone(),
            },
            // Its disk copy is still being read.
            None => Self {
                wanted: true,
                revision: 0,
                data: None,
                checking: true,
                error: None,
            },
        }
    }
}

/// The Create form's four lists, read from the app's cache.
struct CreateLists {
    orgs: ListState<Vec<String>>,
    templates: ListState<Vec<String>>,
    gitignores: ListState<Vec<String>>,
    licenses: ListState<Vec<License>>,
}

impl CreateLists {
    fn read(app: &Osier, account: Option<&GhAccount>, source: Option<&GhAccount>) -> Self {
        let account = account.map(account_key);
        let source = source.map(account_key);
        Self {
            orgs: ListState::read(&app.gh_lists.orgs, account.clone()),
            templates: ListState::read(&app.gh_lists.templates, account),
            gitignores: ListState::read(&app.gh_lists.gitignores, source.clone()),
            licenses: ListState::read(&app.gh_lists.licenses, source),
        }
    }
}

/// The "Create new repository" dialog's form, after GitHub's own.
pub(crate) struct CreateForm {
    account: Option<GhAccount>,
    /// Any gh account, for the .gitignore and licence lists of a local repository.
    lookup: Option<GhAccount>,
    owner: Pick,
    name: Entity<InputState>,
    description: Entity<InputState>,
    private: bool,
    template: Pick,
    readme: bool,
    gitignore: Pick,
    license: Pick,
    licenses: Vec<License>,
    dir: Entity<InputState>,
    /// Lists still loading, or why they could not be.
    status: Option<String>,
    target_check: TargetCheck,
    /// Everything checked and fine: Create is enabled.
    ready: bool,
    /// Confirmed: what the form says while the dialog closes (nothing is checked any more).
    submitted: Option<String>,
    /// `status` is a problem (lists that could not load), shown as a warning notice.
    status_warning: bool,
    /// Revisions of the cached lists last copied into the form: owners, templates,
    /// `.gitignore`s, licences.
    seen: [u64; 4],
    _subscriptions: Vec<Subscription>,
}

impl CreateForm {
    fn new(seed: FormSeed, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let FormSeed {
            account,
            lookup,
            app,
            parent,
        } = seed;
        let parent = parent.as_path();
        let pick = |items: Vec<SharedString>, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| {
                SelectState::new(
                    SearchableVec::new(items),
                    Some(IndexPath::default()),
                    window,
                    cx,
                )
                .searchable(true)
            })
        };
        let owner_items: Vec<SharedString> = account
            .iter()
            .map(|a| SharedString::from(a.login.clone()))
            .collect();
        let owner = pick(owner_items, window, cx);
        let template = pick(vec![NO_TEMPLATE.into()], window, cx);
        let gitignore = pick(vec![NONE.into()], window, cx);
        let license = pick(vec![NONE.into()], window, cx);
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("my-project"));
        let description = cx.new(|cx| InputState::new(window, cx).placeholder("Optional"));
        let dir =
            cx.new(|cx| InputState::new(window, cx).default_value(crate::format::tilde(parent)));
        let watch = |cx: &mut Context<Self>, input: &Entity<InputState>| {
            cx.subscribe(input, |_: &mut Self, _, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Change) {
                    cx.notify();
                }
            })
        };
        let mut _subscriptions = vec![watch(cx, &name), watch(cx, &dir)];
        // The lists come from the app's cache: shown at once, refreshed in the background,
        // copied in again whenever one changes.
        _subscriptions.push(
            cx.observe_in(&app, window, |this: &mut Self, app, window, cx| {
                let view = CreateLists::read(app.read(cx), this.account.as_ref(), this.source());
                this.apply_lists(view, window, cx);
            }),
        );
        let source = account.clone().or_else(|| lookup.clone());
        let mut this = Self {
            account: account.clone(),
            lookup,
            owner,
            name,
            description,
            private: true,
            template,
            readme: true,
            gitignore,
            license,
            licenses: Vec::new(),
            dir,
            status: source
                .is_some()
                .then(|| "Loading the lists from GitHub…".to_string()),
            target_check: TargetCheck::default(),
            ready: false,
            submitted: None,
            status_warning: false,
            seen: [0; 4],
            _subscriptions,
        };
        if source.is_none() {
            this.status = Some(
                "No gh account is logged in, so the .gitignore and licence lists are not \
                 available."
                    .into(),
            );
        }
        this
    }

    /// The account the `.gitignore` and licence lists come from.
    fn source(&self) -> Option<&GhAccount> {
        self.account.as_ref().or(self.lookup.as_ref())
    }

    /// Copy the cached lists in where they changed, keeping each dropdown's pick, and say
    /// whether GitHub is being checked or a list could not load.
    fn apply_lists(&mut self, view: CreateLists, window: &mut Window, cx: &mut Context<Self>) {
        if self.source().is_none() {
            return;
        }
        let fill =
            |pick: &Pick, first: &str, rest: &[String], window: &mut Window, cx: &mut App| {
                // Keep what the user picked if it is still there.
                let keep = pick.read(cx).selected_value().map(|v| v.to_string());
                let mut items: Vec<SharedString> = vec![SharedString::from(first.to_string())];
                items.extend(rest.iter().cloned().map(SharedString::from));
                let ix = keep
                    .and_then(|k| items.iter().position(|i| i.as_ref() == k))
                    .unwrap_or(0);
                pick.update(cx, |s, cx| {
                    s.set_items(SearchableVec::new(items), window, cx);
                    s.set_selected_index(Some(IndexPath::new(ix)), window, cx);
                });
            };
        let mut changed = false;
        if view.orgs.revision != self.seen[0]
            && let (Some(orgs), Some(a)) = (&view.orgs.data, &self.account)
        {
            let login = a.login.clone();
            fill(&self.owner, &login, orgs, window, cx);
            self.seen[0] = view.orgs.revision;
            changed = true;
        }
        if view.templates.revision != self.seen[1]
            && let Some(templates) = &view.templates.data
        {
            fill(&self.template, NO_TEMPLATE, templates, window, cx);
            self.seen[1] = view.templates.revision;
            changed = true;
        }
        if view.gitignores.revision != self.seen[2]
            && let Some(names) = &view.gitignores.data
        {
            fill(&self.gitignore, NONE, names, window, cx);
            self.seen[2] = view.gitignores.revision;
            changed = true;
        }
        if view.licenses.revision != self.seen[3]
            && let Some(licenses) = &view.licenses.data
        {
            let names: Vec<String> = licenses.iter().map(|l| l.name.clone()).collect();
            fill(&self.license, NONE, &names, window, cx);
            self.licenses = licenses.as_ref().clone();
            self.seen[3] = view.licenses.revision;
            changed = true;
        }
        let lists: Vec<(bool, bool, Option<String>)> = [
            view.orgs.summary(),
            view.templates.summary(),
            view.gitignores.summary(),
            view.licenses.summary(),
        ]
        .into_iter()
        .flatten()
        .collect();
        let missing = lists.iter().any(|(has, _, _)| !has);
        let checking = lists.iter().any(|(_, checking, _)| *checking);
        let errors: Vec<String> = lists
            .iter()
            .filter(|(has, _, _)| !has)
            .filter_map(|(_, _, e)| e.clone())
            .collect();
        let (status, warning) = if missing && checking {
            (
                Some(
                    "Loading the lists from GitHub… (templates take longer for accounts with \
                     many repositories)"
                        .to_string(),
                ),
                false,
            )
        } else if checking {
            (
                Some("Checking GitHub for new templates and lists…".to_string()),
                false,
            )
        } else if !errors.is_empty() {
            (
                Some(format!("Some lists could not load: {}", errors.join("; "))),
                true,
            )
        } else {
            (None, false)
        };
        if changed || status != self.status || warning != self.status_warning {
            self.status = status;
            self.status_warning = warning;
            cx.notify();
        }
    }

    fn picked(pick: &Pick, none: &str, cx: &App) -> Option<String> {
        pick.read(cx)
            .selected_value()
            .map(|v| v.to_string())
            .filter(|v| v != none)
    }

    fn template(&self, cx: &App) -> Option<String> {
        self.account
            .as_ref()
            .and(Self::picked(&self.template, NO_TEMPLATE, cx))
    }

    fn plan(&self, cx: &App) -> Result<CreatePlan, String> {
        let name = self.name.read(cx).value().trim().to_string();
        check_repo_name(&name)?;
        let target = parse_dir(&self.dir.read(cx).value())?.join(&name);
        let template = self.template(cx);
        let starters = template.is_none();
        let license_name = Self::picked(&self.license, NONE, cx);
        let license = license_name.and_then(|n| {
            self.licenses
                .iter()
                .find(|l| l.name == n)
                .map(|l| l.key.clone())
        });
        let owner = match &self.account {
            Some(a) => Self::picked(&self.owner, "", cx).unwrap_or_else(|| a.login.clone()),
            None => String::new(),
        };
        Ok(CreatePlan {
            account: self.account.clone(),
            lookup: self.lookup.clone(),
            new: NewRepo {
                owner,
                name,
                description: self.description.read(cx).value().trim().to_string(),
                private: self.private,
                template,
                readme: starters && self.readme,
                gitignore: Self::picked(&self.gitignore, NONE, cx).filter(|_| starters),
                license: license.filter(|_| starters),
            },
            target,
        })
    }
}

impl CreateForm {
    /// The line under the form (muted text, or an error), and whether Create may go ahead.
    fn outcome(&mut self, cx: &mut Context<Self>) -> Result<String, String> {
        if let Some(text) = &self.submitted {
            return Ok(text.clone());
        }
        let (ready, outcome) = match self.plan(cx) {
            Err(e) if self.name.read(cx).value().is_empty() => (false, Ok(e)),
            Err(e) => (false, Err(e)),
            Ok(plan) => {
                let target = crate::format::tilde(&plan.target);
                match self
                    .target_check
                    .status(&plan.target, |f: &mut Self| &mut f.target_check, cx)
                {
                    None => (false, Ok(format!("Checking {target}…"))),
                    Some(Ok(())) => (
                        true,
                        Ok(match &plan.account {
                            Some(_) => format!(
                                "Creates {}/{} on GitHub and clones it into {target}",
                                plan.new.owner, plan.new.name
                            ),
                            None => format!("Creates a local repository in {target}"),
                        }),
                    ),
                    Some(Err(e)) => (false, Err(e)),
                }
            }
        };
        if self.ready != ready {
            // The dialog reads `ready` for its button before this form redraws in the same
            // frame, so ask for one more frame. Deferred: gpui drops a notify made while the
            // window is drawing.
            self.ready = ready;
            let form = cx.entity();
            cx.defer(move |cx| form.update(cx, |_, cx| cx.notify()));
        }
        outcome
    }

    /// The plan to run when Create is pressed, `None` while checks are not done or failed.
    /// Freezes the form: it stops checking (creating fills the folder it checked).
    fn confirm(&mut self, cx: &mut Context<Self>) -> Option<CreatePlan> {
        let ready = self.outcome(cx).is_ok() && self.ready;
        let plan = self.plan(cx).ok().filter(|_| ready)?;
        self.submitted = Some(format!(
            "Creating in {}…",
            crate::format::tilde(&plan.target)
        ));
        cx.notify();
        Some(plan)
    }
}

impl Render for CreateForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let remote = self.account.is_some();
        let templated = self.template(cx).is_some();
        let outcome = self.outcome(cx);
        let row = |label: &'static str, child: AnyElement, cx: &App| {
            v_flex().gap_1().child(field_label(label, cx)).child(child)
        };
        v_flex()
            .test_id("create-form")
            .w_full()
            .gap_3()
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_end()
                    .when(remote, |this| {
                        this.child(
                            div().w(px(180.)).flex_none().child(row(
                                "Owner",
                                div()
                                    .test_id("create-owner")
                                    .child(Select::new(&self.owner))
                                    .into_any_element(),
                                cx,
                            )),
                        )
                        .child(div().pb_2().text_color(muted).child("/"))
                    })
                    .child(
                        div().flex_1().min_w_0().child(row(
                            "Repository name",
                            div()
                                .test_id("create-name")
                                .child(Input::new(&self.name))
                                .into_any_element(),
                            cx,
                        )),
                    ),
            )
            .child(row(
                "Description",
                div()
                    .test_id("create-description")
                    .child(Input::new(&self.description))
                    .into_any_element(),
                cx,
            ))
            .when(remote, |this| {
                let private = self.private;
                this.child(row(
                    "Visibility",
                    RadioGroup::horizontal("create-visibility")
                        .selected_index(Some(if private { 1 } else { 0 }))
                        .on_click(cx.listener(|this, ix: &usize, _, cx| {
                            this.private = *ix == 1;
                            cx.notify();
                        }))
                        .child(
                            Radio::new("create-public")
                                .test_id("create-public")
                                .label("Public"),
                        )
                        .child(
                            Radio::new("create-private")
                                .test_id("create-private")
                                .label("Private"),
                        )
                        .into_any_element(),
                    cx,
                ))
                .child(row(
                    "Template",
                    div()
                        .test_id("create-template")
                        .child(Select::new(&self.template))
                        .into_any_element(),
                    cx,
                ))
            })
            .child(
                Checkbox::new("create-readme")
                    .test_id("create-readme")
                    .small()
                    .label("Add a README file")
                    .checked(self.readme && !templated)
                    .disabled(templated)
                    .on_click(cx.listener(|this, v: &bool, _, cx| {
                        this.readme = *v;
                        cx.notify();
                    })),
            )
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .child(
                        div().flex_1().min_w_0().child(row(
                            ".gitignore template",
                            div()
                                .test_id("create-gitignore")
                                .child(Select::new(&self.gitignore).disabled(templated))
                                .into_any_element(),
                            cx,
                        )),
                    )
                    .child(
                        div().flex_1().min_w_0().child(row(
                            "Licence",
                            div()
                                .test_id("create-license")
                                .child(Select::new(&self.license).disabled(templated))
                                .into_any_element(),
                            cx,
                        )),
                    ),
            )
            .when(templated, |this| {
                this.child(
                    div().text_xs().text_color(muted).child(
                        "The template's own files are used; GitHub skips the starter files.",
                    ),
                )
            })
            .when(remote && !self.readme && !templated, |this| {
                this.child(div().text_xs().text_color(muted).child(
                    "Without a README GitHub makes an empty repository, and the .gitignore \
                     and licence are skipped.",
                ))
            })
            .when(!remote, |this| {
                this.child(div().text_xs().text_color(muted).child(
                    "The files are written but not committed, so you can review them in \
                     Changes first.",
                ))
            })
            .child(
                v_flex()
                    .gap_1()
                    .child(field_label("Directory", cx))
                    .child(directory_row(&self.dir, "create")),
            )
            .when_some(self.status.clone(), |this, s| {
                // Lists that could not load are worth a warning; loading is just a note.
                this.child(if self.status_warning {
                    crate::dialog_notice::DialogNotice::warning(s)
                        .test_id("create-status")
                        .into_any_element()
                } else {
                    div()
                        .test_id("create-status")
                        .text_xs()
                        .text_color(muted)
                        .child(s)
                        .into_any_element()
                })
            })
            .child(outcome_line(outcome, cx))
    }
}

impl Osier {
    /// The create dialog: on GitHub for `account`, else a local repository.
    pub(crate) fn prompt_create(
        &mut self,
        account: Option<GhAccount>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let parent = default_parent(self);
        let lookup = self
            .gh_accounts
            .iter()
            .find(|a| a.active)
            .or_else(|| self.gh_accounts.first())
            .cloned();
        let title: SharedString = match &account {
            Some(a) => format!("Create a repository as {}", a.login).into(),
            None => "Create a local repository".into(),
        };
        // Cached lists at once, a check for new ones in the background.
        let source = account.clone().or_else(|| lookup.clone());
        if let Some(source) = &source {
            self.use_gh_create_lists(account.as_ref(), source, Freshness::CheckNow, cx);
        }
        let view = CreateLists::read(self, account.as_ref(), source.as_ref());
        let app = cx.entity();
        let form = cx.new(|cx| {
            let seed = FormSeed {
                account,
                lookup,
                app,
                parent,
            };
            CreateForm::new(seed, window, cx)
        });
        form.update(cx, |f, cx| f.apply_lists(view, window, cx));
        let weak = cx.entity().downgrade();
        let focus = form.clone();
        window.open_app_dialog(cx, move |dialog, _, cx| {
            let (form, weak) = (form.clone(), weak.clone());
            let ready = form.read(cx).ready;
            dialog
                .title(title.clone())
                .w(px(600.))
                .child(form.clone())
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Create")
                        .show_cancel(true),
                )
                // Disabled until every check (the target folder in the background) passed.
                .footer(crate::dialog_buttons::DialogButtons::new("Create").disabled(!ready))
                .on_ok(
                    move |_, window, cx| match form.update(cx, |f, cx| f.confirm(cx)) {
                        Some(plan) => {
                            let _ = weak.update(cx, |this, cx| this.start_create(plan, window, cx));
                            true
                        }
                        // Enter before the checks are done, or with a problem shown: stay.
                        None => false,
                    },
                )
        });
        focus.update(cx, |f, cx| f.name.update(cx, |i, cx| i.focus(window, cx)));
    }

    /// Create on GitHub then clone, or `git init` with the starter files.
    pub(crate) fn start_create(
        &mut self,
        plan: CreatePlan,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(job) = &self.remote_job {
            window.push_notice(
                Notice::warning(format!("{} is still running", job.label)),
                cx,
            );
            return;
        }
        let hub = self.hub.clone();
        match plan.account.clone() {
            Some(account) => {
                let label = format!("Create {}/{}", plan.new.owner, plan.new.name);
                let (_tx, rx) = mpsc::channel();
                self.remote_job = Some(crate::app::remote_ops::RemoteJob::new(
                    label.clone(),
                    CancelToken::new(),
                    rx,
                ));
                cx.notify();
                let new = plan.new.clone();
                let acc = account.clone();
                cx.spawn_in(window, async move |this, cx| {
                    let created = cx
                        .background_spawn(async move { hub.create(&acc, &new) })
                        .await;
                    let _ = this.update_in(cx, |this, window, cx| {
                        this.remote_job = None;
                        match created {
                            Ok(repo) => {
                                this.gh_lists.repos.mark_stale(&account_key(&account));
                                this.start_clone(
                                    ClonePlan {
                                        url: repo.clone_url,
                                        target: plan.target.clone(),
                                        recurse_submodules: false,
                                        account: Some(account),
                                        label: repo.full_name,
                                    },
                                    window,
                                    cx,
                                );
                            }
                            Err(e) => {
                                window.push_notice(
                                    Notice::warning(e).title(format!("{label} failed")),
                                    cx,
                                );
                                cx.notify();
                            }
                        }
                    });
                })
                .detach();
            }
            None => {
                let runner = self.runner.clone();
                let label = format!("Create {}", plan.new.name);
                let target = plan.target.clone();
                cx.spawn_in(window, async move |this, cx| {
                    let result = cx
                        .background_spawn(async move { create_local(&*hub, &runner, &plan) })
                        .await;
                    let _ = this.update_in(cx, |this, window, cx| {
                        this.finish_new_repo(&target, &label, None, result, window, cx);
                    });
                })
                .detach();
            }
        }
    }
}

/// `git init` plus the starter files, fetched from GitHub's lists when picked.
fn create_local(
    hub: &dyn Hub,
    runner: &osier_git::GitRunner,
    plan: &CreatePlan,
) -> Result<String, GitError> {
    let new = &plan.new;
    let lookup = plan.lookup.as_ref();
    let fetch =
        |what: &str, r: Option<Result<String, String>>| -> Result<Option<String>, GitError> {
            match r {
                None => Ok(None),
                Some(Ok(text)) => Ok(Some(text)),
                Some(Err(e)) => Err(GitError::Refused(format!(
                    "Could not fetch the {what}: {e}"
                ))),
            }
        };
    let gitignore = fetch(
        ".gitignore template",
        new.gitignore
            .as_ref()
            .zip(lookup)
            .map(|(name, a)| hub.gitignore_template(a, name)),
    )?;
    let license = fetch(
        "licence",
        new.license.as_ref().zip(lookup).map(|(key, a)| {
            let holder = runner
                .run(None, ["config", "user.name"])
                .ok()
                .and_then(|o| o.stdout_str().ok().map(|s| s.trim().to_string()))
                .unwrap_or_default();
            let year = chrono::Datelike::year(&chrono::Local::now());
            hub.license_text(a, key, year, &holder)
        }),
    )?;
    let readme = new.readme.then(|| {
        let mut text = format!("# {}\n", new.name);
        if !new.description.is_empty() {
            text.push_str(&format!("\n{}\n", new.description));
        }
        text
    });
    let scaffold = osier_git::ops::init::Scaffold {
        readme,
        gitignore,
        license,
    };
    osier_git::ops::init::init(runner, &plan.target, &scaffold)?;
    Ok(String::new())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{check_repo_name, name_from_url, parse_dir, repo_choices};
    use osier_auth::GhAccount;
    use osier_auth::hub::GhRepo;

    #[test]
    fn folder_names_come_from_clone_addresses() {
        assert_eq!(
            name_from_url("https://github.com/a/app.git").as_deref(),
            Some("app")
        );
        assert_eq!(
            name_from_url("git@github.com:a/app.git").as_deref(),
            Some("app")
        );
        assert_eq!(
            name_from_url("ssh://host/srv/repo/").as_deref(),
            Some("repo")
        );
        assert_eq!(name_from_url("/srv/git/thing").as_deref(), Some("thing"));
        assert_eq!(name_from_url("  "), None);
        assert_eq!(name_from_url("https://host/.git"), None);
    }

    #[test]
    fn names_and_directories_are_checked() {
        assert!(check_repo_name("my-app_2.0").is_ok());
        assert!(check_repo_name("").is_err());
        assert!(check_repo_name("a/b").is_err());
        assert!(check_repo_name("..").is_err());
        assert!(parse_dir("relative").is_err());
        assert!(parse_dir("").is_err());
        assert!(parse_dir("/srv").is_ok());
    }

    pub(crate) fn repo(owner: &str, name: &str) -> GhRepo {
        GhRepo {
            full_name: format!("{owner}/{name}"),
            name: name.into(),
            owner: owner.into(),
            clone_url: format!("https://github.com/{owner}/{name}.git"),
            ssh_url: format!("git@github.com:{owner}/{name}.git"),
            private: false,
            description: None,
            is_template: false,
            archived: false,
            fork: false,
        }
    }

    #[test]
    fn the_accounts_own_repositories_come_first() {
        let account = GhAccount {
            host: "github.com".into(),
            login: "zed".into(),
            active: true,
            git_protocol: "https".into(),
            state: "success".into(),
        };
        let rows = repo_choices(
            &account,
            &[repo("acme", "b"), repo("zed", "x"), repo("acme", "a")],
        );
        let order: Vec<String> = rows
            .iter()
            .map(|c| format!("{}/{}", c.section, c.label))
            .collect();
        assert_eq!(order, ["zed/x", "acme/a", "acme/b"]);
    }
}
