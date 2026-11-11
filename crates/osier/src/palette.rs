//! The command palette (ctrl-p or F1): every action reachable by typing its name.
//!
//! Commands run through their own closures rather than dispatched actions, so the dialog
//! layer never has to route key events back to the main view.
//!
//! Some entries open a submenu (a [`PaletteMode`]): the list is replaced and the query field
//! shows the submenu's name in a badge. Backspace on an empty query or Escape go back to the
//! commands; Escape there closes the palette. A submenu can also be opened directly by its
//! own shortcut (ctrl-o for "Switch project", ctrl-shift-b for "Switch branch"). The Fetch,
//! Pull, Push and Update branch submenus list what the title bar's dropdowns offer
//! (`palette_menus.rs`).
//!
//! gpui-kit's `Command` draws its own query field with a fixed search icon, so the palette
//! hides it (`searchable(false)`), draws an `Input` with the badge in the `header` slot and
//! filters the items itself. The header sits inside the `Command` key context, so Up, Down
//! and Enter still reach the list.

use crate::app_dialog::AppDialogExt as _;
use std::rc::Rc;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::actions::Cancel;
use gpui_kit::component::command::{Command, CommandGroup, CommandItem, CommandState};
use gpui_kit::component::diff::DiffMode;
use gpui_kit::component::input::{Backspace, Input, InputEvent, InputState};
use gpui_kit::component::{
    ActiveTheme as _, Icon, IndexPath, Sizable as _, WindowExt as _, h_flex, v_flex,
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
    /// Local branches, then remote ones not checked out yet.
    SwitchBranch,
    Fetch,
    Pull,
    Push,
    UpdateBranch,
}

impl PaletteMode {
    /// The remote submenus, in the order the commands list them.
    pub(crate) const REMOTE: [PaletteMode; 4] = [
        PaletteMode::Fetch,
        PaletteMode::UpdateBranch,
        PaletteMode::Pull,
        PaletteMode::Push,
    ];

    /// Submenus about the open project: nothing to list without one.
    fn needs_project(self) -> bool {
        matches!(self, PaletteMode::SwitchBranch) || Self::REMOTE.contains(&self)
    }

    /// The badge in the query field; `None` for the commands themselves.
    pub(crate) fn badge(self) -> Option<&'static str> {
        match self {
            PaletteMode::Root => None,
            PaletteMode::SwitchProject => Some("Switch project"),
            PaletteMode::CloneAccount | PaletteMode::CloneRepo => Some("Clone repository"),
            PaletteMode::CreateAccount => Some("New repository"),
            PaletteMode::SwitchBranch => Some("Switch branch"),
            PaletteMode::Fetch => Some("Fetch"),
            PaletteMode::Pull => Some("Pull"),
            PaletteMode::Push => Some("Push"),
            PaletteMode::UpdateBranch => Some("Update branch"),
        }
    }

    fn placeholder(self) -> &'static str {
        match self {
            PaletteMode::Root => "Type a command…",
            PaletteMode::SwitchProject => "Search open and recent projects…",
            PaletteMode::CloneAccount => "Pick a GitHub account, or Other… for any address",
            PaletteMode::CloneRepo => "Search repositories…",
            PaletteMode::CreateAccount => "Pick a GitHub account, or No remote",
            PaletteMode::SwitchBranch => "Search branches…",
            PaletteMode::Fetch => "How to fetch…",
            PaletteMode::Pull => "How to pull…",
            PaletteMode::Push => "How to push…",
            PaletteMode::UpdateBranch => "How to update the branch…",
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

/// A row of a submenu other than "Switch project".
#[derive(Clone)]
pub(crate) struct Choice {
    /// Group heading; rows with an empty one are listed without a heading.
    pub section: SharedString,
    pub label: SharedString,
    pub detail: Option<SharedString>,
    pub icon: Lucide,
    pub pick: ChoicePick,
    /// Registry action id whose keys are shown as the row's hint.
    pub action: Option<&'static str>,
}

#[derive(Clone)]
pub(crate) enum ChoicePick {
    CloneAccount(osier_auth::GhAccount),
    CloneOther,
    CloneRepo(osier_auth::GhAccount, osier_auth::hub::GhRepo),
    CreateAccount(osier_auth::GhAccount),
    CreateLocal,
    Remote(crate::app::remote_ops::RemoteOp),
    ForcePush,
    /// Update branch: `None` asks (or uses the remembered method), like the button.
    Update(Option<osier_git::ops::remote::PullStrategy>),
    ForgetUpdateMethod,
    Checkout {
        name: String,
        remote: bool,
    },
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

/// A row's shortcut as a quiet `<kbd>`-like pill: outlined, muted, monospace.
fn key_pill(keys: String, cx: &App) -> Div {
    let theme = cx.theme();
    div()
        .flex_none()
        .px_1p5()
        .rounded(theme.radius.half())
        .border_1()
        // Faded text colour, not the border token: that one matches the highlighted row's
        // background and the pill vanished there.
        .border_color(theme.muted_foreground.opacity(0.35))
        .text_color(theme.muted_foreground)
        .font_family(theme.mono_font_family.clone())
        .text_xs()
        .line_height(relative(1.5))
        .child(keys)
}

/// Case-insensitive substring match on a row's name; an empty query matches everything.
fn label_matches(label: &str, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    q.is_empty() || label.to_lowercase().contains(&q)
}

/// The key-bindable form of a palette command that has no action of its own (Stage all,
/// Split layout…): runs that command's closure, so the palette and the key do the same.
/// The ids and keymap names are listed in `keymap::palette_specs`.
#[derive(Debug, Clone, PartialEq, Action)]
#[action(namespace = osier, no_json)]
pub(crate) struct PaletteAction {
    pub id: &'static str,
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
    needs_project: bool,
    mode: PaletteMode,
) -> PaletteCommand {
    PaletteCommand {
        group,
        label,
        icon,
        action,
        needs_project,
        target: Target::Submenu(mode),
    }
}

/// Every palette command, in display order.
pub(crate) fn commands() -> Vec<PaletteCommand> {
    vec![
        sub(
            "General",
            "Switch project…",
            Lucide::ArrowLeftRight,
            Some("switch_project"),
            false,
            PaletteMode::SwitchProject,
        ),
        sub(
            "General",
            "Switch branch…",
            Lucide::GitBranch,
            Some("switch_branch"),
            true,
            PaletteMode::SwitchBranch,
        ),
        sub(
            "General",
            "Clone repository…",
            Lucide::CloudDownload,
            Some("clone_repository"),
            false,
            PaletteMode::CloneAccount,
        ),
        sub(
            "General",
            "Create new repository…",
            Lucide::FolderPlus,
            Some("create_repository"),
            false,
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
            Some("open_themes_folder"),
            false,
            |o, w, cx| o.open_themes_dir(w, cx),
        ),
        cmd(
            "General",
            "Rescan project directories",
            Lucide::FolderSearch,
            Some("rescan_projects"),
            false,
            |o, _, cx| o.rescan_projects(cx),
        ),
        cmd(
            "General",
            "Scan project status now",
            Lucide::RefreshCw,
            Some("scan_project_status"),
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
            Some("commit_and_push"),
            true,
            |o, w, cx| o.commit_and_push(w, cx),
        ),
        cmd(
            "Changes",
            "Stage all",
            Lucide::ListChecks,
            Some("stage_all"),
            true,
            |o, w, cx| o.stage_all(w, cx),
        ),
        cmd(
            "Changes",
            "Unstage all",
            Lucide::ListX,
            Some("unstage_all"),
            true,
            |o, w, cx| o.unstage_all(w, cx),
        ),
        cmd(
            "Changes",
            "Stash changes…",
            Lucide::Archive,
            Some("stash_changes"),
            true,
            |o, w, cx| o.prompt_stash(w, cx),
        ),
        cmd(
            "Changes",
            "Shelve all changes…",
            Lucide::Package,
            Some("shelve_all_changes"),
            true,
            |o, w, cx| o.shelve_prompt(Vec::new(), w, cx),
        ),
        sub(
            "Remote",
            "Fetch…",
            Lucide::CloudDownload,
            Some("fetch"),
            true,
            PaletteMode::Fetch,
        ),
        sub(
            "Remote",
            "Update branch…",
            Lucide::FolderSync,
            Some("update_branch"),
            true,
            PaletteMode::UpdateBranch,
        ),
        sub(
            "Remote",
            "Pull…",
            Lucide::ArrowDown,
            Some("pull"),
            true,
            PaletteMode::Pull,
        ),
        sub(
            "Remote",
            "Push…",
            Lucide::ArrowUp,
            Some("push"),
            true,
            PaletteMode::Push,
        ),
        cmd(
            "Remote",
            "Choose GitHub account for this project…",
            Lucide::Github,
            Some("choose_github_account"),
            true,
            |o, w, cx| o.choose_account(w, cx),
        ),
        cmd(
            "Remote",
            "Forget GitHub account choice for this project",
            Lucide::Github,
            Some("forget_github_account"),
            true,
            |o, w, cx| o.clear_project_account(w, cx),
        ),
        cmd(
            "Branches",
            "New branch…",
            Lucide::GitBranch,
            Some("new_branch"),
            true,
            |o, w, cx| o.prompt_new_branch(None, w, cx),
        ),
        cmd(
            "Patches",
            "Apply patch from clipboard…",
            Lucide::ClipboardPaste,
            Some("apply_patch_from_clipboard"),
            true,
            |o, w, cx| o.apply_patch_from_clipboard(w, cx),
        ),
        cmd(
            "Patches",
            "Apply patch from file…",
            Lucide::FileInput,
            Some("apply_patch_from_file"),
            true,
            |o, w, cx| o.apply_patch_from_file(w, cx),
        ),
        cmd(
            "Patches",
            "Copy all changes as patch",
            Lucide::ClipboardCopy,
            Some("copy_all_changes_as_patch"),
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
            Some("diff_unified"),
            false,
            |o, _, cx| o.set_diff_mode(DiffMode::Unified, cx),
        ),
        cmd(
            "Diff",
            "Split layout",
            Lucide::Columns2,
            Some("diff_split"),
            false,
            |o, _, cx| o.set_diff_mode(DiffMode::Split, cx),
        ),
        cmd(
            "Diff",
            "Conflicts in the 3-way merge tool",
            Lucide::Columns3,
            Some("conflicts_three_way"),
            false,
            |o, _, cx| o.set_conflict_view(osier_config::settings::ConflictView::ThreeWay, cx),
        ),
        cmd(
            "Diff",
            "Conflicts inline",
            Lucide::FileDiff,
            Some("conflicts_inline"),
            false,
            |o, _, cx| o.set_conflict_view(osier_config::settings::ConflictView::Inline, cx),
        ),
        cmd(
            "Diff",
            "Expand the diff pane",
            Lucide::Maximize2,
            Some("toggle_diff_expanded"),
            false,
            |o, _, cx| o.toggle_diff_expanded(cx),
        ),
        cmd(
            "Diff",
            "Toggle soft wrap",
            Lucide::FileDiff,
            Some("toggle_soft_wrap"),
            false,
            |o, window, cx| o.toggle_soft_wrap(window, cx),
        ),
        cmd(
            "Open",
            "Open project in editor",
            Lucide::SquareArrowOutUpRight,
            Some("open_in_editor"),
            true,
            |o, w, cx| o.open_in_editor(None, w, cx),
        ),
        cmd(
            "Open",
            "Open terminal here",
            Lucide::Terminal,
            Some("open_terminal"),
            true,
            |o, w, cx| o.open_terminal(w, cx),
        ),
        cmd(
            "Open",
            "Open in file manager",
            Lucide::FolderOpen,
            Some("open_in_file_manager"),
            true,
            |o, w, cx| o.reveal_in_file_manager(None, w, cx),
        ),
    ]
}

impl Osier {
    /// Run the palette command whose keymap action is `id`, as choosing it would.
    pub(crate) fn run_palette_command(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(command) = commands().into_iter().find(|c| c.action == Some(id)) else {
            return;
        };
        if command.needs_project && self.active_project().is_none() {
            return;
        }
        match command.target {
            Target::Run(run) => run(self, window, cx),
            Target::Submenu(mode) => self.open_palette_in(mode, window, cx),
        }
    }

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
        if mode.needs_project() && !has_project {
            return;
        }
        let commands: Rc<Vec<PaletteCommand>> = Rc::new(
            commands()
                .into_iter()
                .filter(|c| has_project || !c.needs_project)
                .collect(),
        );
        if matches!(mode, PaletteMode::CloneAccount | PaletteMode::CreateAccount) {
            self.warm_gh_repos(cx);
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
        let query = palette.read(cx).query.clone();
        window.open_app_dialog(cx, move |dialog, _, _| {
            let palette = palette.clone();
            dialog
                .close_button(false)
                .p_0()
                .content(move |content, _, _| {
                    // The palette's own shortcuts while it is open: the dialog is drawn outside
                    // the app view, so they never reach the app's handlers. The key for the
                    // mode on screen closes the palette, the other one switches to its mode.
                    let (p1, p2, p3) = (palette.clone(), palette.clone(), palette.clone());
                    let (p4, p5, p6, p7) = (
                        palette.clone(),
                        palette.clone(),
                        palette.clone(),
                        palette.clone(),
                    );
                    content.child(
                        v_flex()
                            .w_full()
                            .on_action(move |_: &crate::app::OpenCommandPalette, window, cx| {
                                p1.update(cx, |p, cx| p.toggle(PaletteMode::Root, window, cx))
                            })
                            .on_action(move |_: &crate::app::SwitchProject, window, cx| {
                                p2.update(cx, |p, cx| {
                                    p.toggle(PaletteMode::SwitchProject, window, cx)
                                })
                            })
                            .on_action(move |_: &crate::app::SwitchBranch, window, cx| {
                                p3.update(cx, |p, cx| {
                                    p.toggle(PaletteMode::SwitchBranch, window, cx)
                                })
                            })
                            .on_action(move |_: &crate::app::Fetch, window, cx| {
                                p4.update(cx, |p, cx| p.toggle(PaletteMode::Fetch, window, cx))
                            })
                            .on_action(move |_: &crate::app::Pull, window, cx| {
                                p5.update(cx, |p, cx| p.toggle(PaletteMode::Pull, window, cx))
                            })
                            .on_action(move |_: &crate::app::Push, window, cx| {
                                p6.update(cx, |p, cx| p.toggle(PaletteMode::Push, window, cx))
                            })
                            .on_action(move |_: &crate::app::UpdateBranch, window, cx| {
                                p7.update(cx, |p, cx| {
                                    p.toggle(PaletteMode::UpdateBranch, window, cx)
                                })
                            })
                            .child(palette.clone()),
                    )
                })
        });
        // Focus the field now, before the dialog's first frame. Moving focus redraws the
        // whole window (main view included, past its cache), so doing it from the first
        // frame instead cost a second full redraw while the palette animates in.
        window.defer(cx, move |window, cx| {
            query.update(cx, |q, cx| q.focus(window, cx))
        });
    }

    /// The rows a submenu lists, read when it is entered.
    fn entries_for(&self, mode: PaletteMode) -> Rc<Vec<ProjectEntry>> {
        Rc::new(match mode {
            PaletteMode::Root if self.settings.palette.search_submenus => {
                self.switch_project_entries()
            }
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
            PaletteMode::SwitchBranch => self.branch_choices(),
            PaletteMode::Root => self.searchable_sub_choices(),
            _ => self.remote_choices(mode),
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
    /// Watches the cached repository list while "Clone from" shows it.
    _repos_watch: Option<Subscription>,
    /// The cached list's revision shown in the rows.
    repos_seen: u64,
    /// "Checking for new repositories…" under the search field.
    note: Option<SharedString>,
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
            _repos_watch: None,
            repos_seen: 0,
            note: None,
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
            owner.update(cx, |o, cx| o.warm_gh_repos(cx));
        }
        self.projects = owner.read(cx).entries_for(mode);
        self.choices = Rc::new(owner.read(cx).choices_for(mode));
        if Self::lists_accounts(mode) && !owner.read(cx).gh_accounts_loaded {
            self.wait_for_accounts(window, cx);
        }
        self._repos_watch = None;
        self.note = None;
        if mode != PaletteMode::CloneRepo {
            return;
        }
        let Some(account) = self.account.clone() else {
            return;
        };
        // The cached list shows at once (from disk on the first open of a session); a
        // check for new repositories runs in the background, and its result replaces the
        // rows when it lands.
        self.repos_seen = 0;
        owner.update(cx, |o, cx| {
            o.use_gh_repos(&account, crate::cached_list::Freshness::CheckNow, cx)
        });
        self._repos_watch = Some(cx.observe_in(&owner, window, |this, _, window, cx| {
            this.sync_repos(window, cx)
        }));
        self.sync_repos(window, cx);
    }

    /// Show the cached repository list of the account being cloned from: rows when its
    /// revision moved, the note while a refresh runs, a status when there is nothing to show.
    fn sync_repos(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode != PaletteMode::CloneRepo {
            return;
        }
        let (Some(account), Some(owner)) = (self.account.clone(), self.owner.upgrade()) else {
            return;
        };
        let (revision, data, checking, error) = {
            let o = owner.read(cx);
            match o
                .gh_lists
                .repos
                .get(&crate::new_repo::account_key(&account))
            {
                Some(e) => (e.revision, e.data.clone(), e.refreshing, e.error.clone()),
                // Its disk copy is still being read.
                None => (0, None, true, None),
            }
        };
        let status: Option<SharedString> = match &data {
            None if checking => Some("Loading repositories…".into()),
            None => error.map(|e| format!("Could not list repositories: {e}").into()),
            Some(repos) if repos.is_empty() && !checking => {
                Some(format!("{} has no repositories.", account.login).into())
            }
            Some(_) => None,
        };
        let note: Option<SharedString> =
            (checking && data.is_some()).then(|| "Checking for new repositories…".into());
        let rows_changed = revision != self.repos_seen;
        if !rows_changed && status == self.status && note == self.note {
            return;
        }
        self.status = status;
        self.note = note;
        if rows_changed {
            self.repos_seen = revision;
            self.choices = Rc::new(
                data.map(|repos| crate::new_repo::repo_choices(&account, &repos))
                    .unwrap_or_default(),
            );
            self.rebuild_list(window, cx);
        } else {
            cx.notify();
        }
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
                        owner.update(cx, |o, cx| o.warm_gh_repos(cx));
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
        let root = self.mode == PaletteMode::Root;
        let mut out: Vec<(SharedString, Vec<usize>)> = Vec::new();
        for (ix, c) in self.choices.iter().enumerate() {
            // From the commands list only the row's own name counts: a branch's last commit
            // or a submenu's name would bring in rows nobody searched for.
            let hit = if root {
                label_matches(&c.label, &query)
            } else {
                c.matches(&query)
            };
            if !hit {
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

    /// The shortcut for `mode` pressed while the palette is open: close it when it already
    /// shows `mode`, else switch to it.
    fn toggle(&mut self, mode: PaletteMode, window: &mut Window, cx: &mut Context<Self>) {
        let has_project = self
            .owner
            .upgrade()
            .is_some_and(|o| o.read(cx).active_project().is_some());
        if mode.needs_project() && !has_project {
            return;
        }
        if self.mode == mode {
            window.close_dialog(cx);
        } else {
            self.enter(mode, window, cx);
        }
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
                let _ = self
                    .owner
                    .update(cx, |o, cx| o.run_choice(pick, window, cx));
            }
            Pick::Nothing => {}
        }
    }

    fn choice_item(&self, ix: usize, cx: &App) -> CommandItem {
        let c = self.choices[ix].clone();
        let muted = cx.theme().muted_foreground;
        let keys = c.action.and_then(|id| crate::keymap::hint(id, cx));
        let id = format!("palette-choice:{}", c.label);
        CommandItem::new()
            .label(c.label.clone())
            .icon(c.icon)
            .child(move |_, cx| {
                h_flex()
                    .test_id(|| id.clone())
                    .w_full()
                    .gap_3()
                    .items_center()
                    .child(Icon::new(c.icon).xsmall().flex_none())
                    // The name, with what it does or where it is underneath.
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().truncate().child(c.label.clone()))
                            .when_some(c.detail.clone(), |this, detail| {
                                this.child(
                                    div().text_xs().text_color(muted).truncate().child(detail),
                                )
                            }),
                    )
                    .when_some(keys.clone(), |this, k| this.child(key_pill(k, cx)))
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
        let keys = c.action.and_then(|id| crate::keymap::hint(id, cx));
        let (label, icon, group) = (c.label, c.icon, c.group);
        let submenu = matches!(c.target, Target::Submenu(_));
        let show_group = self
            .owner
            .upgrade()
            .is_some_and(|o| o.read(cx).settings.palette.show_categories);
        CommandItem::new()
            .label(label)
            .icon(icon)
            .child(move |_, cx| {
                h_flex()
                    .test_id(|| format!("palette-command:{label}"))
                    .w_full()
                    .gap_3()
                    .items_center()
                    .child(Icon::new(icon).xsmall())
                    .child(div().flex_1().min_w_0().truncate().child(label))
                    .when(show_group, |this| {
                        this.child(
                            div()
                                .test_id(|| format!("palette-category:{label}"))
                                .text_xs()
                                .text_color(muted)
                                .child(group),
                        )
                    })
                    .when_some(keys.clone(), |this, k| this.child(key_pill(k, cx)))
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
    note: Option<SharedString>,
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
        // While a cached list is checked for new entries ("Checking for new repositories…").
        .when_some(note, |this, note| {
            this.child(
                h_flex()
                    .test_id("palette-note")
                    .pb_2()
                    .gap_2()
                    .items_center()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(gpui_kit::component::spinner::Spinner::new().xsmall())
                    .child(note),
            )
        })
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
                command = command.group(
                    CommandGroup::new().items(rows.iter().map(|&ix| self.command_item(ix, cx))),
                );
                let mut picks: Vec<Vec<Pick>> = vec![rows.into_iter().map(Pick::Command).collect()];
                // `palette.search_submenus`: matching submenu rows, after every command.
                if !query.trim().is_empty() {
                    let projects: Vec<usize> = (0..self.projects.len())
                        .filter(|&ix| label_matches(&self.projects[ix].name, &query))
                        .collect();
                    if !projects.is_empty() {
                        command = command.group(
                            CommandGroup::new()
                                .label("Switch project")
                                .items(projects.iter().map(|&ix| self.project_item(ix))),
                        );
                        picks.push(projects.into_iter().map(Pick::Project).collect());
                    }
                    for (section, rows) in self.choice_sections(cx) {
                        command = command.group(
                            CommandGroup::new()
                                .label(section)
                                .items(rows.iter().map(|&ix| self.choice_item(ix, cx))),
                        );
                        picks.push(rows.into_iter().map(Pick::Choice).collect());
                    }
                }
                picks
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
                    let mut group = CommandGroup::new();
                    if !section.is_empty() {
                        group = group.label(section.clone());
                    }
                    command =
                        command.group(group.items(rows.iter().map(|&ix| self.choice_item(ix, cx))));
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
        let note = self.note.clone();
        command
            .header(move |_, _, cx| {
                render_query(badge.clone(), note.clone(), &query, &header_owner, cx)
            })
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
        let specs = crate::keymap::actions();
        for c in &all {
            // Every command can be given a key: its own action, or a `PaletteAction`.
            let id = c
                .action
                .unwrap_or_else(|| panic!("{:?} has no keymap action", c.label));
            let spec = specs
                .iter()
                .find(|s| s.id == id)
                .unwrap_or_else(|| panic!("palette points at unknown action {id}"));
            // Same name in Settings > Keymap, "…" included: it says a dialog or a choice
            // follows.
            assert_eq!(spec.name, c.label, "keymap name of {id}");
        }
        // Each palette-only action belongs to exactly one command, under the same name.
        for spec in crate::keymap::palette_specs() {
            let owners: Vec<&super::PaletteCommand> =
                all.iter().filter(|c| c.action == Some(spec.id)).collect();
            assert_eq!(owners.len(), 1, "{} is used by one command", spec.id);
            assert_eq!(owners[0].label, spec.name);
            assert!(matches!(
                (spec.make)().as_any().downcast_ref::<super::PaletteAction>(),
                Some(a) if a.id == spec.id
            ));
        }
    }
}
