//! The command palette (ctrl-p or F1): every action reachable by typing its name.
//!
//! Commands run through their own closures rather than dispatched actions, so the dialog
//! layer never has to route key events back to the main view.
//!
//! Some entries open a submenu (a [`PaletteMode`]): the list is replaced and the query field
//! shows the submenu's name in a badge. Backspace on an empty query or Escape go back to the
//! commands; Escape there closes the palette. A submenu can also be opened directly by its
//! own shortcut (ctrl-o for "Switch project").
//!
//! gpui-kit's `Command` draws its own query field with a fixed search icon, so the palette
//! hides it (`searchable(false)`), draws an `Input` with the badge in the `header` slot and
//! filters the items itself. The header sits inside the `Command` key context, so Up, Down
//! and Enter still reach the list.

use crate::app_dialog::AppDialogExt as _;
use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::actions::Cancel;
use gpui_kit::component::command::{Command, CommandGroup, CommandItem, CommandState};
use gpui_kit::component::diff::DiffMode;
use gpui_kit::component::input::{Backspace, Input, InputEvent, InputState};
use gpui_kit::component::{
    ActiveTheme as _, Icon, IndexPath, Sizable as _, WindowExt as _, h_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app::switch_project::{ProjectEntry, Section};
use crate::app::{Osier, Tab};
use crate::test_ids::TestId as _;

type Run = Rc<dyn Fn(&mut Osier, &mut Window, &mut Context<Osier>)>;

/// What the palette lists: the commands, or one of their submenus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PaletteMode {
    Root,
    SwitchProject,
    /// "Clone repository…": a gh account, or "Other…" for any address.
    CloneAccount,
    /// The repositories of the account picked in `CloneAccount`.
    CloneRepo,
    /// "Create new repository…": a gh account, or "No remote".
    CreateAccount,
}

impl PaletteMode {
    /// The badge in the query field; `None` for the commands themselves.
    fn badge(self) -> Option<&'static str> {
        match self {
            PaletteMode::Root => None,
            PaletteMode::SwitchProject => Some("Switch project"),
            PaletteMode::CloneAccount | PaletteMode::CloneRepo => Some("Clone repository"),
            PaletteMode::CreateAccount => Some("New repository"),
        }
    }

    fn placeholder(self) -> &'static str {
        match self {
            PaletteMode::Root => "Type a command…",
            PaletteMode::SwitchProject => "Search open and recent projects…",
            PaletteMode::CloneAccount => "Pick a GitHub account, or Other… for any address",
            PaletteMode::CloneRepo => "Search repositories…",
            PaletteMode::CreateAccount => "Pick a GitHub account, or No remote",
        }
    }

    /// Where Escape or Backspace on an empty query go back to.
    fn parent(self) -> PaletteMode {
        match self {
            PaletteMode::CloneRepo => PaletteMode::CloneAccount,
            _ => PaletteMode::Root,
        }
    }
}

/// A row of the clone and create submenus.
#[derive(Clone)]
pub(crate) struct Choice {
    pub section: SharedString,
    pub label: SharedString,
    pub detail: Option<SharedString>,
    pub icon: Lucide,
    pub pick: ChoicePick,
}

#[derive(Clone)]
pub(crate) enum ChoicePick {
    CloneAccount(osier_auth::GhAccount),
    CloneOther,
    CloneRepo(osier_auth::GhAccount, osier_auth::hub::GhRepo),
    CreateAccount(osier_auth::GhAccount),
    CreateLocal,
}

impl Choice {
    fn matches(&self, query: &str) -> bool {
        let q = query.trim().to_lowercase();
        q.is_empty()
            || self.label.to_lowercase().contains(&q)
            || self.section.to_lowercase().contains(&q)
            || self
                .detail
                .as_ref()
                .is_some_and(|d| d.to_lowercase().contains(&q))
    }
}

/// What choosing a command does.
pub(crate) enum Target {
    Run(Run),
    Submenu(PaletteMode),
}

pub(crate) struct PaletteCommand {
    pub group: &'static str,
    pub label: &'static str,
    pub icon: Lucide,
    /// Registry action id (`keymap.rs`) whose current keys are shown as the hint.
    pub action: Option<&'static str>,
    /// Hidden when no repository is open.
    pub needs_project: bool,
    pub target: Target,
}

impl PaletteCommand {
    fn matches(&self, query: &str) -> bool {
        let q = query.trim().to_lowercase();
        q.is_empty()
            || self.label.to_lowercase().contains(&q)
            || self.group.to_lowercase().contains(&q)
    }
}

fn cmd(
    group: &'static str,
    label: &'static str,
    icon: Lucide,
    action: Option<&'static str>,
    needs_project: bool,
    run: impl Fn(&mut Osier, &mut Window, &mut Context<Osier>) + 'static,
) -> PaletteCommand {
    PaletteCommand {
        group,
        label,
        icon,
        action,
        needs_project,
        target: Target::Run(Rc::new(run)),
    }
}

/// A command that opens a submenu.
fn sub(
    group: &'static str,
    label: &'static str,
    icon: Lucide,
    action: Option<&'static str>,
    mode: PaletteMode,
) -> PaletteCommand {
    PaletteCommand {
        group,
        label,
        icon,
        action,
        needs_project: false,
        target: Target::Submenu(mode),
    }
}

/// Every palette command, in display order.
pub(crate) fn commands() -> Vec<PaletteCommand> {
    use crate::app::remote_ops::RemoteOp;
    vec![
        sub(
            "General",
            "Switch project…",
            Lucide::ArrowLeftRight,
            Some("switch_project"),
            PaletteMode::SwitchProject,
        ),
        sub(
            "General",
            "Clone repository…",
            Lucide::CloudDownload,
            Some("clone_repository"),
            PaletteMode::CloneAccount,
        ),
        sub(
            "General",
            "Create new repository…",
            Lucide::FolderPlus,
            Some("create_repository"),
            PaletteMode::CreateAccount,
        ),
        cmd(
            "General",
            "Open repository…",
            Lucide::FolderOpen,
            Some("open_repository"),
            false,
            |o, w, cx| o.prompt_open_repository(w, cx),
        ),
        cmd(
            "General",
            "Close project",
            Lucide::X,
            Some("close_project"),
            true,
            |o, w, cx| {
                if let Some(ix) = o.active {
                    o.close_project(ix, w, cx)
                }
            },
        ),
        cmd(
            "General",
            "Refresh",
            Lucide::RefreshCw,
            Some("refresh"),
            true,
            |o, _, cx| o.refresh_active(cx),
        ),
        cmd(
            "General",
            "Settings…",
            Lucide::Settings,
            Some("open_settings"),
            false,
            |o, w, cx| o.open_settings(w, cx),
        ),
        cmd(
            "General",
            "Toggle sidebar",
            Lucide::PanelLeft,
            Some("toggle_sidebar"),
            false,
            |o, _, cx| o.toggle_sidebar(cx),
        ),
        cmd(
            "General",
            "Toggle git command log",
            Lucide::SquareTerminal,
            Some("toggle_command_log"),
            false,
            |o, _, cx| o.toggle_command_log(cx),
        ),
        cmd(
            "General",
            "Toggle notifications",
            Lucide::Bell,
            Some("toggle_notifications"),
            false,
            |o, _, cx| o.toggle_notifications(cx),
        ),
        cmd(
            "General",
            "Toggle commit details",
            Lucide::PanelBottom,
            Some("toggle_commit_details"),
            true,
            |o, _, cx| o.toggle_commit_details(cx),
        ),
        cmd(
            "General",
            "Toggle dark / light",
            Lucide::SunMoon,
            Some("toggle_theme"),
            false,
            |o, _, cx| o.toggle_theme(cx),
        ),
        cmd(
            "General",
            "Open themes folder",
            Lucide::Palette,
            None,
            false,
            |o, w, cx| o.open_themes_dir(w, cx),
        ),
        cmd(
            "General",
            "Rescan project directories",
            Lucide::FolderSearch,
            None,
            false,
            |o, _, cx| o.rescan_projects(cx),
        ),
        cmd(
            "General",
            "Scan project status now",
            Lucide::RefreshCw,
            None,
            false,
            |o, w, cx| o.scan_recent_now(w, cx),
        ),
        cmd(
            "General",
            "Quit",
            Lucide::LogOut,
            Some("quit"),
            false,
            |_, _, cx| cx.quit(),
        ),
        cmd(
            "Tabs",
            "Changes tab",
            Lucide::FileDiff,
            Some("tab_changes"),
            true,
            |o, _, cx| o.set_tab(Tab::Changes, cx),
        ),
        cmd(
            "Tabs",
            "Log tab",
            Lucide::GitCommitHorizontal,
            Some("tab_log"),
            true,
            |o, _, cx| o.set_tab(Tab::Log, cx),
        ),
        cmd(
            "Tabs",
            "Branches tab",
            Lucide::GitBranch,
            Some("tab_branches"),
            true,
            |o, _, cx| o.set_tab(Tab::Branches, cx),
        ),
        cmd(
            "Tabs",
            "Stashes tab",
            Lucide::Archive,
            Some("tab_stashes"),
            true,
            |o, _, cx| o.set_tab(Tab::Stashes, cx),
        ),
        cmd(
            "Tabs",
            "Files tab",
            Lucide::Folder,
            Some("tab_files"),
            true,
            |o, _, cx| o.set_tab(Tab::Files, cx),
        ),
        cmd(
            "Tabs",
            "Patches tab",
            Lucide::Package,
            Some("tab_patches"),
            true,
            |o, _, cx| o.set_tab(Tab::Patches, cx),
        ),
        cmd(
            "Changes",
            "Commit staged",
            Lucide::GitCommitHorizontal,
            Some("commit"),
            true,
            |o, w, cx| o.commit(w, cx),
        ),
        cmd(
            "Changes",
            "Commit and push",
            Lucide::ArrowUp,
            None,
            true,
            |o, w, cx| o.commit_and_push(w, cx),
        ),
        cmd(
            "Changes",
            "Stage all",
            Lucide::ListChecks,
            None,
            true,
            |o, w, cx| o.stage_all(w, cx),
        ),
        cmd(
            "Changes",
            "Unstage all",
            Lucide::ListX,
            None,
            true,
            |o, w, cx| o.unstage_all(w, cx),
        ),
        cmd(
            "Changes",
            "Stash changes…",
            Lucide::Archive,
            None,
            true,
            |o, w, cx| o.prompt_stash(w, cx),
        ),
        cmd(
            "Changes",
            "Shelve all changes…",
            Lucide::Package,
            None,
            true,
            |o, w, cx| o.shelve_prompt(Vec::new(), w, cx),
        ),
        cmd(
            "Remote",
            "Fetch all remotes",
            Lucide::CloudDownload,
            Some("fetch"),
            true,
            |o, w, cx| o.start_remote(RemoteOp::fetch_all(), w, cx),
        ),
        cmd(
            "Remote",
            "Update project",
            Lucide::FolderSync,
            Some("update_project"),
            true,
            |o, w, cx| o.update_project(w, cx),
        ),
        cmd(
            "Remote",
            "Pull",
            Lucide::ArrowDown,
            Some("pull"),
            true,
            |o, w, cx| o.start_remote(RemoteOp::pull_default(), w, cx),
        ),
        cmd(
            "Remote",
            "Push",
            Lucide::ArrowUp,
            Some("push"),
            true,
            |o, w, cx| o.start_remote(RemoteOp::push_default(), w, cx),
        ),
        cmd(
            "Remote",
            "Choose GitHub account for this project…",
            Lucide::Github,
            None,
            true,
            |o, w, cx| o.choose_account(w, cx),
        ),
        cmd(
            "Branches",
            "New branch…",
            Lucide::GitBranch,
            None,
            true,
            |o, w, cx| o.prompt_new_branch(None, w, cx),
        ),
        cmd(
            "Patches",
            "Apply patch from clipboard…",
            Lucide::ClipboardPaste,
            None,
            true,
            |o, w, cx| o.apply_patch_from_clipboard(w, cx),
        ),
        cmd(
            "Patches",
            "Apply patch from file…",
            Lucide::FileInput,
            None,
            true,
            |o, w, cx| o.apply_patch_from_file(w, cx),
        ),
        cmd(
            "Patches",
            "Copy all changes as patch",
            Lucide::ClipboardCopy,
            None,
            true,
            |o, w, cx| {
                if let Some(s) = o.all_changes_source() {
                    o.copy_patch(s, w, cx)
                }
            },
        ),
        cmd(
            "Diff",
            "Unified layout",
            Lucide::FileDiff,
            None,
            false,
            |o, _, cx| o.set_diff_mode(DiffMode::Unified, cx),
        ),
        cmd(
            "Diff",
            "Split layout",
            Lucide::Columns2,
            None,
            false,
            |o, _, cx| o.set_diff_mode(DiffMode::Split, cx),
        ),
        cmd(
            "Diff",
            "Conflicts in the 3-way merge tool",
            Lucide::Columns3,
            None,
            false,
            |o, _, cx| o.set_conflict_view(osier_config::settings::ConflictView::ThreeWay, cx),
        ),
        cmd(
            "Diff",
            "Conflicts inline",
            Lucide::FileDiff,
            None,
            false,
            |o, _, cx| o.set_conflict_view(osier_config::settings::ConflictView::Inline, cx),
        ),
        cmd(
            "Diff",
            "Expand the diff pane",
            Lucide::Maximize2,
            None,
            false,
            |o, _, cx| o.toggle_diff_expanded(cx),
        ),
        cmd(
            "Diff",
            "Toggle soft wrap",
            Lucide::FileDiff,
            None,
            false,
            |o, window, cx| o.toggle_soft_wrap(window, cx),
        ),
        cmd(
            "Open",
            "Open project in editor",
            Lucide::SquareArrowOutUpRight,
            None,
            true,
            |o, w, cx| o.open_in_editor(None, w, cx),
        ),
        cmd(
            "Open",
            "Open terminal here",
            Lucide::Terminal,
            None,
            true,
            |o, w, cx| o.open_terminal(w, cx),
        ),
        cmd(
            "Open",
            "Open in file manager",
            Lucide::FolderOpen,
            None,
            true,
            |o, w, cx| o.reveal_in_file_manager(None, w, cx),
        ),
    ]
}

impl Osier {
    pub(crate) fn open_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_palette_in(PaletteMode::Root, window, cx);
    }

    /// Open the palette on `mode`: the commands, or straight into one of their submenus.
    pub(crate) fn open_palette_in(
        &mut self,
        mode: PaletteMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let has_project = self.active_project().is_some();
        let commands: Rc<Vec<PaletteCommand>> = Rc::new(
            commands()
                .into_iter()
                .filter(|c| has_project || !c.needs_project)
                .collect(),
        );
        if matches!(mode, PaletteMode::CloneAccount | PaletteMode::CreateAccount) {
            self.prefetch_hub_repos(cx);
        }
        let owner = cx.weak_entity();
        // Read here: the palette cannot read this view while it is being updated.
        let projects = self.entries_for(mode);
        let choices = Rc::new(self.choices_for(mode));
        let palette =
            cx.new(|cx| Palette::new(owner, commands, mode, projects, choices, window, cx));
        if Palette::lists_accounts(mode) && !self.gh_accounts_loaded {
            palette.update(cx, |p, cx| p.wait_for_accounts(window, cx));
        }
        let focus_on_mount = Rc::new(Cell::new(true));
        window.open_app_dialog(cx, move |dialog, _, _| {
            let palette = palette.clone();
            let focus_on_mount = focus_on_mount.clone();
            dialog
                .close_button(false)
                .p_0()
                .content(move |content, window, cx| {
                    if focus_on_mount.replace(false) {
                        let query = palette.read(cx).query.clone();
                        window.defer(cx, move |window, cx| {
                            query.update(cx, |q, cx| q.focus(window, cx));
                        });
                    }
                    content.child(palette.clone())
                })
        });
    }

    /// The rows a submenu lists, read when it is entered.
    fn entries_for(&self, mode: PaletteMode) -> Rc<Vec<ProjectEntry>> {
        Rc::new(match mode {
            PaletteMode::Root => Vec::new(),
            PaletteMode::SwitchProject => self.switch_project_entries(),
            _ => Vec::new(),
        })
    }

    /// The account rows of the clone and create submenus (the repositories load later).
    fn choices_for(&self, mode: PaletteMode) -> Vec<Choice> {
        match mode {
            PaletteMode::CloneAccount | PaletteMode::CreateAccount => {
                crate::new_repo::account_choices(self, mode == PaletteMode::CloneAccount)
            }
            _ => Vec::new(),
        }
    }
}

/// The open palette: its mode, query field and list.
pub(crate) struct Palette {
    owner: WeakEntity<Osier>,
    mode: PaletteMode,
    query: Entity<InputState>,
    /// Recreated whenever the rows change, so the first row is highlighted again.
    list: Entity<CommandState>,
    commands: Rc<Vec<PaletteCommand>>,
    projects: Rc<Vec<ProjectEntry>>,
    /// Rows of the clone and create submenus.
    choices: Rc<Vec<Choice>>,
    /// The account whose repositories `CloneRepo` lists.
    account: Option<osier_auth::GhAccount>,
    /// Shown instead of the rows while they load, or when there are none.
    status: Option<SharedString>,
    _load: Option<Task<()>>,
    /// A row to highlight once the new list has been drawn (see `rebuild_list`).
    pending_select: Option<IndexPath>,
    _subscription: Subscription,
}

/// What a list row stands for, by its position in the rendered model.
#[derive(Clone, Copy)]
enum Pick {
    Command(usize),
    Project(usize),
    Choice(usize),
    /// The status row (loading, nothing found): nothing happens.
    Nothing,
}

impl Palette {
    fn new(
        owner: WeakEntity<Osier>,
        commands: Rc<Vec<PaletteCommand>>,
        mode: PaletteMode,
        projects: Rc<Vec<ProjectEntry>>,
        choices: Rc<Vec<Choice>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder(mode.placeholder()));
        let _subscription =
            cx.subscribe_in(&query, window, |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change) {
                    this.rebuild_list(window, cx);
                }
            });
        let list = cx.new(|cx| CommandState::new(window, cx));
        let mut this = Self {
            owner,
            mode,
            query,
            list,
            commands,
            projects,
            choices,
            account: None,
            status: None,
            _load: None,
            pending_select: None,
            _subscription,
        };
        this.rebuild_list(window, cx);
        this
    }

    /// Fill the rows for `mode`: projects, accounts, or (in the background) repositories.
    fn load(&mut self, mode: PaletteMode, window: &mut Window, cx: &mut Context<Self>) {
        self._load = None;
        self.status = None;
        let Some(owner) = self.owner.upgrade() else {
            return;
        };
        if matches!(mode, PaletteMode::CloneAccount | PaletteMode::CreateAccount) {
            owner.update(cx, |o, cx| o.prefetch_hub_repos(cx));
        }
        self.projects = owner.read(cx).entries_for(mode);
        self.choices = Rc::new(owner.read(cx).choices_for(mode));
        if Self::lists_accounts(mode) && !owner.read(cx).gh_accounts_loaded {
            self.wait_for_accounts(window, cx);
        }
        if mode != PaletteMode::CloneRepo {
            return;
        }
        let Some(account) = self.account.clone() else {
            return;
        };
        let (hub, cached) = {
            let o = owner.read(cx);
            (
                o.hub.clone(),
                o.hub_repos
                    .get(&crate::new_repo::account_key(&account))
                    .cloned(),
            )
        };
        match cached {
            Some(repos) => self.choices = Rc::new(crate::new_repo::repo_choices(&account, &repos)),
            None => self.status = Some("Loading repositories…".into()),
        }
        // Always refresh: the cached list shows at once, the new one replaces it.
        let busy = owner.read(cx).background.begin(crate::background::GH_REPOS);
        self._load = Some(cx.spawn_in(window, async move |this, cx| {
            let _busy = busy;
            let acc = account.clone();
            let result = cx.background_spawn(async move { hub.repos(&acc) }).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.mode != PaletteMode::CloneRepo || this.account.as_ref() != Some(&account) {
                    return;
                }
                match result {
                    Ok(repos) => {
                        let _ = this.owner.update(cx, |o, _| {
                            o.hub_repos
                                .insert(crate::new_repo::account_key(&account), repos.clone());
                        });
                        let rows = crate::new_repo::repo_choices(&account, &repos);
                        this.status = rows
                            .is_empty()
                            .then(|| format!("{} has no repositories.", account.login).into());
                        this.choices = Rc::new(rows);
                    }
                    Err(e) if this.choices.is_empty() => {
                        this.status = Some(format!("Could not list repositories: {e}").into());
                    }
                    Err(e) => tracing::info!(error = %e, "repository list refresh failed"),
                }
                this.rebuild_list(window, cx);
            });
        }));
    }

    fn lists_accounts(mode: PaletteMode) -> bool {
        matches!(mode, PaletteMode::CloneAccount | PaletteMode::CreateAccount)
    }

    /// Opened before gh answered at startup: say so, and fill the accounts in when it has.
    fn wait_for_accounts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.status = Some("Loading GitHub accounts…".into());
        let mode = self.mode;
        self._load = Some(cx.spawn_in(window, async move |this, cx| {
            for _ in 0..200 {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(150))
                    .await;
                let done = this
                    .update_in(cx, |this, window, cx| {
                        if this.mode != mode {
                            return true;
                        }
                        let Some(owner) = this.owner.upgrade() else {
                            return true;
                        };
                        if !owner.read(cx).gh_accounts_loaded {
                            return false;
                        }
                        this.status = None;
                        this.choices = Rc::new(owner.read(cx).choices_for(mode));
                        owner.update(cx, |o, cx| o.prefetch_hub_repos(cx));
                        this.rebuild_list(window, cx);
                        true
                    })
                    .unwrap_or(true);
                if done {
                    return;
                }
            }
        }));
        cx.notify();
    }

    /// The badge, with the account once one is picked.
    fn badge(&self) -> Option<SharedString> {
        match (self.mode, &self.account) {
            (PaletteMode::CloneRepo, Some(a)) => Some(format!("Clone from {}", a.login).into()),
            (mode, _) => mode.badge().map(SharedString::from),
        }
    }

    /// Matching choices grouped by section, in their listed order.
    fn choice_sections(&self, cx: &App) -> Vec<(SharedString, Vec<usize>)> {
        let query = self.query_text(cx);
        let mut out: Vec<(SharedString, Vec<usize>)> = Vec::new();
        for (ix, c) in self.choices.iter().enumerate() {
            if !c.matches(&query) {
                continue;
            }
            match out.iter_mut().find(|(s, _)| *s == c.section) {
                Some((_, rows)) => rows.push(ix),
                None => out.push((c.section.clone(), vec![ix])),
            }
        }
        out
    }

    fn query_text(&self, cx: &App) -> String {
        self.query.read(cx).value().to_string()
    }

    /// Switch to `mode` with an empty query.
    fn enter(&mut self, mode: PaletteMode, window: &mut Window, cx: &mut Context<Self>) {
        self.mode = mode;
        self.load(mode, window, cx);
        // A programmatic write emits no change event, so the list is rebuilt here. Focus
        // returns to the field: a click on the row that led here moved it to the list.
        self.query.update(cx, |q, cx| {
            q.set_value("", window, cx);
            q.set_placeholder(mode.placeholder(), window, cx);
            q.focus(window, cx);
        });
        self.rebuild_list(window, cx);
    }

    /// Start a fresh list for the current mode and query: highlight and scroll reset. With no
    /// query, "Switch project" highlights the first project that is not the current one, so
    /// ctrl-o enter flips between two projects.
    fn rebuild_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.list = cx.new(|cx| CommandState::new(window, cx));
        self.pending_select = None;
        if self.mode == PaletteMode::SwitchProject && self.query_text(cx).trim().is_empty() {
            let picks = self.project_sections(cx);
            // The list highlights its first row by itself; only a later one needs setting.
            self.pending_select = picks
                .iter()
                .enumerate()
                .find_map(|(section, (_, rows))| {
                    let row = rows.iter().position(|&ix| !self.projects[ix].active)?;
                    Some((section, row))
                })
                .filter(|&first| first != (0, 0))
                .map(|(section, row)| IndexPath::new(row).section(section));
        }
        cx.notify();
    }

    /// Matching projects, grouped like the sidebar; empty groups are left out.
    fn project_sections(&self, cx: &App) -> Vec<(Section, Vec<usize>)> {
        let query = crate::sidebar_query::Query::parse(&self.query_text(cx));
        Section::ALL
            .iter()
            .map(|&section| {
                let rows = self
                    .projects
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| e.section == section && e.matches(&query))
                    .map(|(ix, _)| ix)
                    .collect::<Vec<_>>();
                (section, rows)
            })
            .filter(|(_, rows)| !rows.is_empty())
            .collect()
    }

    /// Escape: leave a submenu, else clear the query, else let the dialog close.
    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode != PaletteMode::Root {
            self.enter(self.mode.parent(), window, cx);
        } else if !self.query_text(cx).is_empty() {
            self.query.update(cx, |q, cx| q.set_value("", window, cx));
            self.rebuild_list(window, cx);
        } else {
            cx.propagate();
        }
    }

    /// Backspace that had nothing to delete: leave a submenu when the query is empty.
    fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode != PaletteMode::Root && self.query_text(cx).is_empty() {
            self.enter(self.mode.parent(), window, cx);
        } else {
            cx.propagate();
        }
    }

    fn confirm(&mut self, pick: Pick, window: &mut Window, cx: &mut Context<Self>) {
        match pick {
            Pick::Command(ix) => match &self.commands[ix].target {
                Target::Submenu(mode) => self.enter(*mode, window, cx),
                Target::Run(run) => {
                    let run = run.clone();
                    window.close_dialog(cx);
                    let _ = self.owner.update(cx, |o, cx| run(o, window, cx));
                }
            },
            Pick::Project(ix) => {
                let path = self.projects[ix].path.clone();
                window.close_dialog(cx);
                let _ = self
                    .owner
                    .update(cx, |o, cx| o.switch_to_project(&path, window, cx));
            }
            Pick::Choice(ix) => {
                let pick = self.choices[ix].pick.clone();
                if let ChoicePick::CloneAccount(account) = pick {
                    self.account = Some(account);
                    self.enter(PaletteMode::CloneRepo, window, cx);
                    return;
                }
                window.close_dialog(cx);
                let _ = self.owner.update(cx, |o, cx| match pick {
                    ChoicePick::CloneOther => o.prompt_clone(None, window, cx),
                    ChoicePick::CloneRepo(account, repo) => {
                        o.prompt_clone(Some((account, repo)), window, cx)
                    }
                    ChoicePick::CreateAccount(account) => {
                        o.prompt_create(Some(account), window, cx)
                    }
                    ChoicePick::CreateLocal => o.prompt_create(None, window, cx),
                    ChoicePick::CloneAccount(_) => {}
                });
            }
            Pick::Nothing => {}
        }
    }

    fn choice_item(&self, ix: usize, cx: &App) -> CommandItem {
        let c = self.choices[ix].clone();
        let muted = cx.theme().muted_foreground;
        let id = format!("palette-choice:{}", c.label);
        CommandItem::new()
            .label(c.label.clone())
            .icon(c.icon)
            .child(move |_, _| {
                h_flex()
                    .test_id(|| id.clone())
                    .w_full()
                    .gap_3()
                    .items_center()
                    .child(Icon::new(c.icon).xsmall())
                    .child(div().flex_none().child(c.label.clone()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(muted)
                            .children(c.detail.clone()),
                    )
                    .when(matches!(c.pick, ChoicePick::CloneAccount(_)), |this| {
                        this.child(Icon::new(Lucide::ChevronRight).xsmall().text_color(muted))
                    })
            })
    }

    fn status_item(text: SharedString, cx: &App) -> CommandItem {
        let muted = cx.theme().muted_foreground;
        CommandItem::new().label(text.clone()).child(move |_, _| {
            div()
                .test_id("palette-status")
                .text_sm()
                .text_color(muted)
                .child(text.clone())
        })
    }

    fn command_item(&self, ix: usize, cx: &App) -> CommandItem {
        let c = &self.commands[ix];
        let muted = cx.theme().muted_foreground;
        let mono = cx.theme().mono_font_family.clone();
        let keys = c.action.and_then(|id| crate::keymap::hint(id, cx));
        let (label, icon, group) = (c.label, c.icon, c.group);
        let submenu = matches!(c.target, Target::Submenu(_));
        CommandItem::new()
            .label(label)
            .icon(icon)
            .child(move |_, _| {
                h_flex()
                    .test_id(|| format!("palette-command:{label}"))
                    .w_full()
                    .gap_3()
                    .items_center()
                    .child(Icon::new(icon).xsmall())
                    .child(div().flex_1().min_w_0().truncate().child(label))
                    .child(div().text_xs().text_color(muted).child(group))
                    .when_some(keys.clone(), |this, k| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(muted)
                                .font_family(mono.clone())
                                .child(k),
                        )
                    })
                    .when(submenu, |this| {
                        this.child(Icon::new(Lucide::ChevronRight).xsmall().text_color(muted))
                    })
            })
    }

    fn project_item(&self, ix: usize) -> CommandItem {
        let entry = self.projects[ix].clone();
        CommandItem::new()
            .label(entry.name.clone())
            .child(move |_, cx| entry.render(cx))
    }
}

/// The query field: search icon, or the submenu's badge, then the text. Drawn by the list
/// (its `header` slot) on each of its renders, so it holds no state of its own.
fn render_query(
    badge: Option<SharedString>,
    query: &Entity<InputState>,
    palette: &WeakEntity<Palette>,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let prefix = match badge {
        None => Icon::new(Lucide::Search)
            .text_color(theme.muted_foreground)
            .into_any_element(),
        Some(badge) => div()
            .test_id("palette-badge")
            .flex_none()
            .px_2()
            .py_0p5()
            .rounded(theme.radius)
            .bg(crate::themes::badge_tint(cx).0)
            .text_color(if crate::themes::high_contrast_on(cx) {
                theme.foreground
            } else {
                theme.primary
            })
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .child(badge)
            .into_any_element(),
    };
    let (on_cancel, on_back) = (palette.clone(), palette.clone());
    div()
        .test_id("palette-query")
        .flex_none()
        .px_3()
        .border_b_1()
        .border_color(theme.border)
        .on_action(move |_: &Cancel, window, cx| match on_cancel.upgrade() {
            Some(p) => p.update(cx, |p, cx| p.cancel(window, cx)),
            None => cx.propagate(),
        })
        .on_action(move |_: &Backspace, window, cx| match on_back.upgrade() {
            Some(p) => p.update(cx, |p, cx| p.back(window, cx)),
            None => cx.propagate(),
        })
        .child(Input::new(query).prefix(prefix).appearance(false).p_0())
        .into_any_element()
}

impl Render for Palette {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(ix) = self.pending_select.take() {
            // The list installs its rows while this frame is drawn; select after that.
            let list = self.list.clone();
            window.defer(cx, move |window, cx| {
                list.update(cx, |l, cx| l.set_selected_index(Some(ix), window, cx));
            });
        }
        let mut command = Command::new(&self.list)
            .searchable(false)
            .bordered(false)
            .rounded(cx.theme().radius_lg);
        let picks: Vec<Vec<Pick>> = match self.mode {
            PaletteMode::Root => {
                let query = self.query_text(cx);
                let rows: Vec<usize> = (0..self.commands.len())
                    .filter(|&ix| self.commands[ix].matches(&query))
                    .collect();
                for &ix in &rows {
                    command = command.item(self.command_item(ix, cx));
                }
                vec![rows.into_iter().map(Pick::Command).collect()]
            }
            PaletteMode::SwitchProject => {
                let sections = self.project_sections(cx);
                for (section, rows) in &sections {
                    command = command.group(
                        CommandGroup::new()
                            .label(section.label())
                            .items(rows.iter().map(|&ix| self.project_item(ix))),
                    );
                }
                sections
                    .into_iter()
                    .map(|(_, rows)| rows.into_iter().map(Pick::Project).collect())
                    .collect()
            }
            _ => {
                let sections = self.choice_sections(cx);
                let mut picks: Vec<Vec<Pick>> = Vec::new();
                for (section, rows) in &sections {
                    command = command.group(
                        CommandGroup::new()
                            .label(section.clone())
                            .items(rows.iter().map(|&ix| self.choice_item(ix, cx))),
                    );
                    picks.push(rows.iter().map(|&ix| Pick::Choice(ix)).collect());
                }
                if let Some(status) = self.status.clone() {
                    command =
                        command.group(CommandGroup::new().item(Self::status_item(status, cx)));
                    picks.push(vec![Pick::Nothing]);
                }
                picks
            }
        };
        let this = cx.weak_entity();
        let (badge, query, header_owner) = (self.badge(), self.query.clone(), this.clone());
        command
            .header(move |_, _, cx| render_query(badge.clone(), &query, &header_owner, cx))
            .on_confirm(move |ix: IndexPath, window, cx| {
                let Some(pick) = picks.get(ix.section).and_then(|s| s.get(ix.row)).copied() else {
                    return;
                };
                let _ = this.update(cx, |p, cx| p.confirm(pick, window, cx));
            })
    }
}

#[cfg(test)]
mod tests {
    use super::commands;

    #[test]
    fn labels_are_unique_and_shortcuts_match_the_keymap() {
        let all = commands();
        let mut labels: Vec<&str> = all.iter().map(|c| c.label).collect();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), all.len(), "duplicate palette label");
        for c in &all {
            assert!(!c.group.is_empty() && !c.label.is_empty());
        }
        let keyed = all.iter().filter(|c| c.action.is_some()).count();
        let ids: Vec<&str> = crate::keymap::actions().iter().map(|a| a.id).collect();
        for c in &all {
            if let Some(id) = c.action {
                assert!(ids.contains(&id), "palette points at unknown action {id}");
            }
        }
        assert!(keyed >= 15, "{keyed} commands carry a shortcut hint");
    }
}
