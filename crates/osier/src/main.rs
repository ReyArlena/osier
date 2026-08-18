//! Osier: fast native git GUI.
//!
//! The layout with placeholder data until the git layer is wired in.

use gpui_kit::assets::{AllAssets, IconName as Lucide};
use gpui_kit::component::{
    ActiveTheme as _, Icon, Selectable as _, Sizable as _, StyledExt as _, Theme, ThemeRegistry,
    TitleBar,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex,
    input::{Textarea, TextareaState},
    kbd::Kbd,
    resizable::{h_resizable, resizable_panel},
    separator::Separator,
    sidebar::{Sidebar, SidebarFooter, SidebarGroup, SidebarHeader, SidebarMenu, SidebarMenuItem},
    status_bar::StatusBar,
    tab::{Tab, TabBar},
    tag::Tag,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

gpui_kit::actions!(osier, [Quit]);

const APP_ID: &str = "com.reyarlena.osier";
const BUNDLED_THEMES: &[&str] = &[include_str!("../../../themes/catppuccin.json")];
const DARK_THEME: &str = "Catppuccin Mocha";
const LIGHT_THEME: &str = "Catppuccin Latte";

/// Load bundled themes into the registry and apply the named one.
fn init_themes(cx: &mut App) {
    let registry = ThemeRegistry::global_mut(cx);
    for source in BUNDLED_THEMES {
        if let Err(error) = registry.load_themes_from_str(source) {
            tracing::warn!(%error, "failed to load bundled theme");
        }
    }
    apply_theme(DARK_THEME, cx);
}

fn apply_theme(name: &str, cx: &mut App) {
    let Some(config) = ThemeRegistry::global(cx).themes().get(name).cloned() else {
        tracing::warn!(name, "theme not found in registry");
        return;
    };
    Theme::update(cx, |theme| theme.apply_config(&config));
}

// ---------- fake data ----------

struct Project {
    name: &'static str,
    path: &'static str,
    changes: usize,
}

const PROJECTS: &[Project] = &[
    Project {
        name: "osier",
        path: "~/Projects/osier",
        changes: 4,
    },
    Project {
        name: "dotfiles",
        path: "~/Projects/dotfiles",
        changes: 0,
    },
    Project {
        name: "website",
        path: "~/Projects/website",
        changes: 1,
    },
    Project {
        name: "api-server",
        path: "~/Work/api-server",
        changes: 2,
    },
];

#[derive(Clone, Copy, PartialEq)]
enum FileStatus {
    Modified,
    Added,
    Deleted,
}

struct ChangedFile {
    path: &'static str,
    status: FileStatus,
    staged: bool,
}

struct FakeCommit {
    hash: &'static str,
    subject: &'static str,
    author: &'static str,
    when: &'static str,
}

const COMMITS: &[FakeCommit] = &[
    FakeCommit {
        hash: "6d15b59",
        subject: "Scaffold workspace",
        author: "dev",
        when: "2 min ago",
    },
    FakeCommit {
        hash: "a3f9c01",
        subject: "Add queue worker",
        author: "dev",
        when: "yesterday",
    },
    FakeCommit {
        hash: "77be210",
        subject: "Fix retention off-by-one",
        author: "dev",
        when: "2 days ago",
    },
    FakeCommit {
        hash: "c0ffee1",
        subject: "Merge branch 'feature/snapshots'",
        author: "dev",
        when: "3 days ago",
    },
    FakeCommit {
        hash: "deadb33",
        subject: "Initial commit",
        author: "dev",
        when: "2 weeks ago",
    },
];

const BRANCHES: &[(&str, bool)] = &[
    ("main", true),
    ("feature/snapshots", false),
    ("fix/retention", false),
    ("origin/main", false),
    ("origin/feature/snapshots", false),
];

// ---------- app ----------

struct Osier {
    focus_handle: FocusHandle,
    active_project: usize,
    active_tab: usize,
    files: Vec<ChangedFile>,
    selected_file: usize,
    amend: bool,
    commit_message: Entity<TextareaState>,
}

impl Osier {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let commit_message = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(4)
                .placeholder("Commit message")
        });

        Self {
            focus_handle: cx.focus_handle(),
            active_project: 0,
            active_tab: 0,
            files: vec![
                ChangedFile {
                    path: "src/main.rs",
                    status: FileStatus::Modified,
                    staged: true,
                },
                ChangedFile {
                    path: "Cargo.toml",
                    status: FileStatus::Modified,
                    staged: true,
                },
                ChangedFile {
                    path: "src/git/repo.rs",
                    status: FileStatus::Added,
                    staged: false,
                },
                ChangedFile {
                    path: "README.md",
                    status: FileStatus::Deleted,
                    staged: false,
                },
            ],
            selected_file: 0,
            amend: false,
            commit_message,
        }
    }

    fn toolbar_button(
        id: &'static str,
        icon: Lucide,
        label: &'static str,
        tip: &'static str,
    ) -> Button {
        Button::new(id)
            .ghost()
            .small()
            .icon(icon)
            .label(label)
            .tooltip(tip)
    }

    // ----- title bar -----

    fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let project = &PROJECTS[self.active_project];
        TitleBar::new().child(
            h_flex()
                .size_full()
                .px_2()
                .pr_3()
                .gap_2()
                .child(Icon::new(Lucide::FolderGit2))
                .child(div().text_sm().font_medium().child(project.name))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(project.path),
                )
                .child(Separator::vertical().h_4().mx_1())
                .child(
                    Button::new("branch")
                        .ghost()
                        .small()
                        .icon(Lucide::GitBranch)
                        .label("main")
                        .tooltip("Switch branch"),
                )
                .child(div().flex_1())
                .child(Self::toolbar_button(
                    "fetch",
                    Lucide::RefreshCw,
                    "Fetch",
                    "Fetch all remotes",
                ))
                .child(Self::toolbar_button(
                    "pull",
                    Lucide::ArrowDown,
                    "Pull",
                    "Pull current branch",
                ))
                .child(Self::toolbar_button(
                    "push",
                    Lucide::ArrowUp,
                    "Push",
                    "Push current branch",
                ))
                .child(Separator::vertical().h_4().mx_1())
                .child(
                    Button::new("stash")
                        .ghost()
                        .small()
                        .icon(Lucide::Archive)
                        .tooltip("Stash changes"),
                )
                .child(
                    Button::new("patch")
                        .ghost()
                        .small()
                        .icon(Lucide::ClipboardCopy)
                        .tooltip("Copy as patch"),
                )
                .child(
                    Button::new("theme")
                        .ghost()
                        .small()
                        .icon(if cx.theme().mode.is_dark() {
                            Lucide::Sun
                        } else {
                            Lucide::Moon
                        })
                        .tooltip("Toggle theme")
                        .on_click(cx.listener(|_, _, _window, cx| {
                            let next = if cx.theme().mode.is_dark() {
                                LIGHT_THEME
                            } else {
                                DARK_THEME
                            };
                            apply_theme(next, cx);
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("settings")
                        .ghost()
                        .small()
                        .icon(Lucide::Settings)
                        .tooltip("Settings"),
                ),
        )
    }

    // ----- sidebar -----

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut menu = SidebarMenu::new();
        for (ix, project) in PROJECTS.iter().enumerate() {
            let mut item = SidebarMenuItem::new(project.name)
                .icon(Lucide::FolderGit2)
                .active(ix == self.active_project)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.active_project = ix;
                    cx.notify();
                }));
            if project.changes > 0 {
                let count = project.changes;
                item = item.suffix(move |_, _| Tag::secondary().xsmall().child(count.to_string()));
            }
            menu = menu.child(item);
        }

        let recent = SidebarMenu::new().children([
            SidebarMenuItem::new("notes").icon(Lucide::Folder),
            SidebarMenuItem::new("scripts").icon(Lucide::Folder),
        ]);

        Sidebar::new("projects")
            .w(px(240.))
            .header(
                SidebarHeader::new().child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_center()
                                .size_8()
                                .rounded(cx.theme().radius)
                                .bg(cx.theme().sidebar_primary)
                                .text_color(cx.theme().sidebar_primary_foreground)
                                .child(Icon::new(Lucide::GitGraph)),
                        )
                        .child(
                            v_flex()
                                .child(div().font_medium().child("Open Projects"))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(format!("{} repositories", PROJECTS.len())),
                                ),
                        ),
                ),
            )
            .child(SidebarGroup::new("Open").child(menu))
            .child(SidebarGroup::new("Recent").child(recent))
            .footer(
                SidebarFooter::new().child(
                    Button::new("open-repo")
                        .outline()
                        .small()
                        .w_full()
                        .icon(Lucide::FolderPlus)
                        .label("Open repository"),
                ),
            )
    }

    // ----- center: tabs -----

    fn render_center(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .min_w_0()
            .child(
                TabBar::new("main-tabs")
                    .w_full()
                    .small()
                    .selected_index(self.active_tab)
                    .on_click(cx.listener(|this, ix: &usize, _, cx| {
                        this.active_tab = *ix;
                        cx.notify();
                    }))
                    .child(Tab::new().label("Changes"))
                    .child(Tab::new().label("Log"))
                    .child(Tab::new().label("Branches"))
                    .child(Tab::new().label("Stashes").disabled(true)),
            )
            .child(div().flex_1().min_h_0().child(match self.active_tab {
                0 => self.render_changes(cx).into_any_element(),
                1 => self.render_log(cx).into_any_element(),
                _ => self.render_branches(cx).into_any_element(),
            }))
    }

    fn status_tag(status: FileStatus) -> Tag {
        match status {
            FileStatus::Modified => Tag::warning().xsmall().child("M"),
            FileStatus::Added => Tag::success().xsmall().child("A"),
            FileStatus::Deleted => Tag::danger().xsmall().child("D"),
        }
    }

    fn render_changes(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let staged = self.files.iter().filter(|f| f.staged).count();

        let mut list = v_flex().flex_1().min_h_0().overflow_hidden();
        for (ix, file) in self.files.iter().enumerate() {
            let selected = ix == self.selected_file;
            list = list.child(
                h_flex()
                    .id(("file", ix))
                    .px_3()
                    .py_1()
                    .gap_2()
                    .items_center()
                    .cursor_pointer()
                    .when(selected, |this| this.bg(cx.theme().accent))
                    .hover(|this| this.bg(cx.theme().accent))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected_file = ix;
                        cx.notify();
                    }))
                    .child(
                        Checkbox::new(("stage", ix))
                            .small()
                            .checked(file.staged)
                            .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                this.files[ix].staged = *checked;
                                cx.notify();
                            })),
                    )
                    .child(Self::status_tag(file.status))
                    .child(div().text_sm().flex_1().truncate().child(file.path)),
            );
        }

        v_flex()
            .size_full()
            .child(
                h_flex()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(div().text_sm().font_medium().child("Changes"))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("{staged} of {} staged", self.files.len())),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("stage-all")
                            .ghost()
                            .xsmall()
                            .icon(Lucide::ListChecks)
                            .tooltip("Stage all"),
                    )
                    .child(
                        Button::new("refresh")
                            .ghost()
                            .xsmall()
                            .icon(Lucide::RefreshCw)
                            .tooltip("Refresh"),
                    ),
            )
            .child(list)
            .child(Separator::horizontal())
            .child(
                v_flex()
                    .p_3()
                    .gap_2()
                    .child(Textarea::new(&self.commit_message))
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                Checkbox::new("amend")
                                    .small()
                                    .label("Amend")
                                    .checked(self.amend)
                                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                        this.amend = *checked;
                                        cx.notify();
                                    })),
                            )
                            .child(div().flex_1())
                            .child(
                                Button::new("commit-push")
                                    .outline()
                                    .small()
                                    .label("Commit and Push"),
                            )
                            .child(
                                Button::new("commit")
                                    .primary()
                                    .small()
                                    .icon(Lucide::GitCommitHorizontal)
                                    .label(format!("Commit {staged} files")),
                            ),
                    ),
            )
    }

    fn render_log(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut list = v_flex().size_full().overflow_hidden();
        for (ix, commit) in COMMITS.iter().enumerate() {
            list = list.child(
                h_flex()
                    .id(("commit", ix))
                    .px_3()
                    .py_2()
                    .gap_3()
                    .items_center()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .hover(|this| this.bg(cx.theme().accent))
                    .child(
                        Icon::new(Lucide::GitCommitVertical)
                            .text_color(cx.theme().muted_foreground),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_sm().truncate().child(commit.subject))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!("{} committed {}", commit.author, commit.when)),
                            ),
                    )
                    .when(ix == 0, |this| {
                        this.child(Tag::primary().xsmall().child("HEAD"))
                    })
                    .when(ix == 0, |this| {
                        this.child(Tag::secondary().xsmall().child("origin/main"))
                    })
                    .child(
                        div()
                            .font_family(cx.theme().mono_font_family.clone())
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(commit.hash),
                    ),
            );
        }
        list
    }

    fn render_branches(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut list = v_flex().size_full().overflow_hidden();
        for (ix, (name, current)) in BRANCHES.iter().enumerate() {
            let remote = name.starts_with("origin/");
            list = list.child(
                h_flex()
                    .id(("branch", ix))
                    .px_3()
                    .py_1p5()
                    .gap_2()
                    .items_center()
                    .hover(|this| this.bg(cx.theme().accent))
                    .child(Icon::new(if remote {
                        Lucide::Cloud
                    } else {
                        Lucide::GitBranch
                    }))
                    .child(div().text_sm().flex_1().child(*name))
                    .when(*current, |this| {
                        this.child(Tag::primary().xsmall().child("current"))
                    })
                    .when(!current && !remote, |this| {
                        this.child(
                            Button::new(("checkout", ix))
                                .ghost()
                                .xsmall()
                                .label("Checkout"),
                        )
                    }),
            );
        }
        list
    }

    // ----- right: diff -----

    fn render_diff(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let file = &self.files[self.selected_file];
        let lines: &[(&str, &str)] = &[
            (" ", "fn main() {"),
            (" ", "    gpui_kit::application().run(move |cx| {"),
            ("-", "        gpui::App::new().run(cx);"),
            ("+", "        gpui_kit::init(cx);"),
            (
                "+",
                "        gpui_kit::open_window(options, cx, |window, cx| {",
            ),
            ("+", "            cx.new(|cx| Osier::new(window, cx))"),
            ("+", "        })"),
            ("+", "        .expect(\"Failed to open window\");"),
            (" ", "    });"),
            (" ", "}"),
        ];

        let mut body = v_flex()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .font_family(cx.theme().mono_font_family.clone())
            .text_xs();
        for (ix, (kind, text)) in lines.iter().enumerate() {
            let (bg, fg) = match *kind {
                "+" => (Some(cx.theme().success.opacity(0.15)), cx.theme().success),
                "-" => (Some(cx.theme().danger.opacity(0.15)), cx.theme().danger),
                _ => (None, cx.theme().foreground),
            };
            body = body.child(
                h_flex()
                    .px_3()
                    .gap_3()
                    .when_some(bg, |this, bg| this.bg(bg))
                    .child(
                        div()
                            .w_8()
                            .text_right()
                            .text_color(cx.theme().muted_foreground)
                            .child((ix + 12).to_string()),
                    )
                    .child(div().w_3().text_color(fg).child(*kind))
                    .child(div().text_color(fg).child(*text)),
            );
        }

        v_flex()
            .size_full()
            .child(
                h_flex()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(Icon::new(Lucide::FileDiff))
                    .child(div().text_sm().font_medium().child(file.path))
                    .child(Self::status_tag(file.status))
                    .child(div().flex_1())
                    .child(
                        Button::new("unified")
                            .ghost()
                            .xsmall()
                            .label("Unified")
                            .selected(true),
                    )
                    .child(Button::new("split").ghost().xsmall().label("Split"))
                    .child(Separator::vertical().h_4())
                    .child(
                        Button::new("revert")
                            .ghost()
                            .xsmall()
                            .icon(Lucide::Undo2)
                            .tooltip("Revert file"),
                    ),
            )
            .child(
                h_flex()
                    .px_3()
                    .py_1()
                    .gap_2()
                    .bg(cx.theme().muted)
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("@@ -12,4 +12,8 @@ fn main()"),
            )
            .child(body)
    }

    // ----- status bar -----

    fn render_status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        StatusBar::new()
            .left(
                h_flex()
                    .gap_1()
                    .text_xs()
                    .child(Icon::new(Lucide::GitBranch).xsmall())
                    .child("main")
                    .child(div().text_color(cx.theme().muted_foreground).child("↑1 ↓2")),
            )
            .left(
                h_flex()
                    .gap_1()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(Icon::new(Lucide::CircleCheck).xsmall())
                    .child("Up to date with origin"),
            )
            .right(
                h_flex()
                    .gap_3()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("{} changes", self.files.len()))
                    .child(
                        h_flex()
                            .gap_1()
                            .children(Keystroke::parse("ctrl-q").ok().map(Kbd::new))
                            .child("Quit"),
                    ),
            )
    }
}

impl Render for Osier {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .track_focus(&self.focus_handle)
            .on_action(|_: &Quit, _, cx| cx.quit())
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.render_title_bar(cx))
            .child(
                div().flex_1().min_h_0().child(
                    h_flex().size_full().child(self.render_sidebar(cx)).child(
                        h_resizable("panes")
                            .child(
                                resizable_panel()
                                    .size(px(420.))
                                    .size_range(px(300.)..px(700.))
                                    .child(self.render_center(cx)),
                            )
                            .child(resizable_panel().child(self.render_diff(cx))),
                    ),
                ),
            )
            .child(self.render_status_bar(cx))
    }
}

fn init_logging() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_env("OSIER_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}

fn main() {
    init_logging();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "starting osier");

    gpui_kit::application()
        .with_assets(AllAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            init_themes(cx);
            cx.bind_keys([KeyBinding::new("ctrl-q", Quit, None)]);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1400.), px(860.)), cx)),
                window_min_size: Some(size(px(1000.), px(600.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Osier".into()),
                    ..Default::default()
                }),
                app_id: Some(APP_ID.into()),
                ..TitleBar::window_options()
            };
            let opened = gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| {
                    let app = Osier::new(window, cx);
                    app.focus_handle.focus(window, cx);
                    tracing::debug!("main window opened");
                    app
                })
            });
            if let Err(error) = opened {
                tracing::error!(%error, "failed to open main window");
                cx.quit();
                return;
            }
            cx.activate(true);
        });
}
