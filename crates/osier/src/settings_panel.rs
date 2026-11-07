//! The settings sheet, built on gpui-kit's `Settings` component. Fields read from and write
//! to `Osier::settings` through the entity; every write saves `config.toml` and re-applies.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::diff::DiffMode;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::setting::{
    NumberFieldOptions, SelectIndex, SettingField, SettingGroup, SettingItem, SettingPage,
    Settings as SettingsView,
};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Selectable as _, Sizable as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use osier_config::Settings;
use osier_config::settings::ThemeMode;

use crate::app::Osier;
use crate::runtime_info::RuntimeInfo;
use crate::test_ids::TestId as _;
use crate::themes::ThemeStore;

fn get<T: 'static>(
    view: &Entity<Osier>,
    f: impl Fn(&Osier) -> T + 'static,
) -> impl Fn(&App) -> T + 'static {
    let view = view.clone();
    move |cx: &App| f(view.read(cx))
}

/// A setter that saves and re-applies after the change.
fn set<T: 'static>(
    view: &Entity<Osier>,
    f: impl Fn(&mut Osier, T, &mut Context<Osier>) + 'static,
) -> impl Fn(T, &mut App) + 'static {
    let view = view.clone();
    move |val: T, cx: &mut App| {
        view.update(cx, |o, cx| {
            f(o, val, cx);
            o.settings_changed(cx);
        })
    }
}

fn opt_string(s: SharedString) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

fn kv(label: &str, value: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    h_flex()
        .gap_3()
        .w_full()
        .child(
            div()
                .w(px(96.))
                .flex_none()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(label.to_string()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_sm()
                .font_family(cx.theme().mono_font_family.clone())
                .child(value.into()),
        )
}

impl Osier {
    /// Open settings as a large centered dialog. Account and ssh details are refreshed first.
    ///
    /// Not a sheet: gpui-kit shrinks the toast layer by an open sheet's width, so with a
    /// wide sheet any toast raised from settings (Scan now, Clear recents) lands off-screen.
    /// Dialogs leave toasts where they are.
    pub(crate) fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_settings_with(None, window, cx);
    }

    /// Open settings on `target`'s page, scrolled to its group, and flash the group so the
    /// eye finds it. For links from elsewhere in the app ("change this in settings").
    pub(crate) fn open_settings_at(
        &mut self,
        target: SettingsTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings_highlight = Some((target, Instant::now()));
        self.settings_scroll_nudge = true;
        self.open_settings_with(Some(target), window, cx);
        // Repaint while the highlight fades, then drop it.
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(33))
                    .await;
                let done = this
                    .update_in(cx, |this, window, _| {
                        this.settings_scroll_nudge = false;
                        let done = this
                            .settings_highlight
                            .is_none_or(|(_, since)| highlight_alpha(since.elapsed()) <= 0.);
                        if done {
                            this.settings_highlight = None;
                        }
                        window.refresh();
                        done
                    })
                    .unwrap_or(true);
                if done {
                    break;
                }
            }
        })
        .detach();
    }

    fn open_settings_with(
        &mut self,
        target: Option<SettingsTarget>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings_target = target;
        self.load_gh_accounts(cx);
        self.ssh_status = Some(osier_auth::ssh::status());
        if self.git_version.is_none() {
            self.git_version = osier_git::GitVersion::detect(&self.runner)
                .ok()
                .map(|v| v.to_string());
        }
        let view = cx.entity();
        let focus = self.settings_focus.clone();
        let root_focus = self.focus_handle.clone();
        window.open_dialog(cx, move |dialog, window, _| {
            // Sized from the window each time the layer is built, so it follows resizes.
            let viewport = window.viewport_size();
            let width = (viewport.width * 0.9).min(px(1100.));
            let height = (viewport.height * 0.8).min(px(820.));
            let root_focus = root_focus.clone();
            let (view, focus) = (view.clone(), focus.clone());
            dialog
                .title("Settings")
                .w(width)
                .centered()
                .overlay_closable(true)
                // Put focus back on the app so shortcuts keep working after closing.
                .on_close(move |_, window, cx| root_focus.focus(window, cx))
                .content(move |content, window, cx| {
                    let pages = pages(&view, window, cx);
                    content.child(
                        div()
                            .w_full()
                            .h(height)
                            .track_focus(&focus)
                            .key_context("OsierSettings")
                            .on_action({
                                let focus = focus.clone();
                                // `/`: the settings page's own search box. It lives inside
                                // gpui-kit's component, out of reach, but it is the first tab
                                // stop after this container.
                                move |_: &crate::app::FocusSettingsSearch, window, cx| {
                                    focus.focus(window, cx);
                                    window.focus_next(cx);
                                }
                            })
                            .test_id("settings-dialog")
                            .child(match target {
                                Some(t) => SettingsView::new("osier-settings")
                                    .pages(pages)
                                    .default_selected_index(SelectIndex {
                                        page_ix: t.page,
                                        group_ix: Some(t.group),
                                    }),
                                None => SettingsView::new("osier-settings").pages(pages),
                            }),
                    )
                })
        });
        // The button that opened settings takes focus back after its click handler ran.
        // The dialog only receives Cancel (its close button, Escape) while focus is inside
        // it, so move focus in once the layer exists.
        let focus = self.settings_focus.clone();
        window.defer(cx, move |window, cx| focus.focus(window, cx));
    }

    /// Save and re-apply after any settings change (panel, config reload).
    pub(crate) fn settings_changed(&mut self, cx: &mut Context<Self>) {
        self.save_settings();
        self.apply_appearance(cx);
        self.apply_recent_status_mode(cx);
        self.rescan_projects(cx);
        let mode = match self.settings.diff.mode {
            osier_config::settings::DiffMode::Split => DiffMode::Split,
            osier_config::settings::DiffMode::Unified => DiffMode::Unified,
        };
        self.diff.update(cx, |state, cx| state.set_mode(mode, cx));
        cx.notify();
    }

    /// Re-read `config.toml` after it changed on disk. Returns true when settings differ.
    pub(crate) fn reload_settings_from_disk(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match Settings::load(&self.paths) {
            Ok(fresh) if fresh != self.settings => {
                self.settings = fresh;
                self.apply_recent_status_mode(cx);
                let minutes = self.settings.projects.recent_status_minutes.clamp(1, 60) as f32;
                self.recent_interval
                    .update(cx, |s, cx| s.set_value(minutes, window, cx));
                let tooltip_ms = self
                    .settings
                    .appearance
                    .tooltip_delay_ms
                    .min(crate::app::TOOLTIP_DELAY_MAX_MS) as f32;
                self.tooltip_delay
                    .update(cx, |s, cx| s.set_value(tooltip_ms, window, cx));
                self.apply_appearance(cx);
                self.rescan_projects(cx);
                self.reload_selection(cx);
                window.push_notification("Settings reloaded from config.toml", cx);
                cx.notify();
                true
            }
            Ok(_) => false,
            Err(error) => {
                window.push_notification(
                    Notification::error("config.toml has an error").message(error.to_string()),
                    cx,
                );
                false
            }
        }
    }

    pub(crate) fn poll_config(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match &self.config_watch {
            Some(w) if w.take_dirty() => self.reload_settings_from_disk(window, cx),
            _ => false,
        }
    }

    pub(crate) fn add_scan_dir_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: true,
            prompt: Some("Scan for repositories".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else {
                return;
            };
            let _ = this.update_in(cx, |this, _, cx| {
                for p in paths {
                    if !this.settings.projects.scan_dirs.contains(&p) {
                        this.settings.projects.scan_dirs.push(p);
                    }
                }
                this.settings_changed(cx);
            });
        })
        .detach();
    }

    pub(crate) fn remove_scan_dir(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        self.settings.projects.scan_dirs.retain(|d| d != &dir);
        self.settings_changed(cx);
    }

    pub(crate) fn confirm_clear_recents(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let n = self.state.recent_not_open().count();
        self.confirm(
            "Clear all recents?",
            format!(
                "{n} recent repositories are forgotten. Open projects and scanned directories stay."
            ),
            "Clear",
            true,
            |this, window, cx| {
                let open: Vec<PathBuf> = this.projects.iter().map(|p| p.path.clone()).collect();
                this.state.recent.retain(|r| open.contains(r));
                this.save_state();
                window.push_notification("Recents cleared", cx);
                cx.notify();
            },
            window,
            cx,
        );
    }

    pub(crate) fn clear_account_override(&mut self, repo: String, cx: &mut Context<Self>) {
        self.state.accounts.remove(&repo);
        self.session_only_accounts.remove(&repo);
        self.save_state();
        cx.notify();
    }

    pub(crate) fn clear_all_account_overrides(&mut self, cx: &mut Context<Self>) {
        self.state.accounts.clear();
        self.session_only_accounts.clear();
        self.save_state();
        cx.notify();
    }

    pub(crate) fn runtime_info(&self) -> RuntimeInfo {
        RuntimeInfo::collect(Some(&self.paths), self.git_version.clone())
    }
}

/// A settings group to open at: page and group positions as built by `pages`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SettingsTarget {
    page: usize,
    group: usize,
}

impl SettingsTarget {
    const PROJECTS: usize = 5;
    /// Projects > Status of projects that are not open.
    pub(crate) const RECENT_STATUS: Self = Self {
        page: Self::PROJECTS,
        group: 1,
    };
}

const HIGHLIGHT_HOLD: Duration = Duration::from_millis(900);
const HIGHLIGHT_FADE: Duration = Duration::from_millis(900);

/// Highlight strength: full while held, then a linear fade to 0.
fn highlight_alpha(elapsed: Duration) -> f32 {
    if elapsed <= HIGHLIGHT_HOLD {
        return 1.;
    }
    let fade = (elapsed - HIGHLIGHT_HOLD).as_secs_f32() / HIGHLIGHT_FADE.as_secs_f32();
    (1. - fade).max(0.)
}

/// A group that can be flashed by `open_settings_at`. Padding with a matching negative
/// margin gives the tint room around the text without moving anything.
fn highlightable(
    group: SettingGroup,
    target: SettingsTarget,
    view: &Entity<Osier>,
    cx: &App,
) -> SettingGroup {
    let alpha = view
        .read(cx)
        .settings_highlight
        .filter(|(t, _)| *t == target)
        .map_or(0., |(_, since)| highlight_alpha(since.elapsed()));
    let pad = px(10.);
    let group = group.p(pad).m(-pad).rounded(cx.theme().radius);
    if alpha > 0. {
        group.bg(cx.theme().primary.opacity(0.14 * alpha))
    } else {
        group
    }
}

fn pages(view: &Entity<Osier>, window: &mut Window, cx: &mut App) -> Vec<SettingPage> {
    let pages = all_pages(view, window, cx);
    let (target, nudge) = {
        let o = view.read(cx);
        (o.settings_target, o.settings_scroll_nudge)
    };
    let Some(target) = target else {
        return pages;
    };
    pages
        .into_iter()
        .enumerate()
        .map(|(ix, page)| {
            let page = page.default_open(ix == target.page);
            // gpui-kit scrolls to the selected group only when a page's group list changes,
            // so the first frame carries an extra empty group and the next one drops it.
            if nudge && ix == target.page {
                page.group(SettingGroup::new().item(SettingItem::render(|_, _, _| div())))
            } else {
                page
            }
        })
        .collect()
}

fn all_pages(view: &Entity<Osier>, _window: &mut Window, cx: &mut App) -> Vec<SettingPage> {
    vec![
        appearance_page(view, cx),
        diff_page(view),
        git_page(view),
        editors_page(view),
        accounts_page(view, cx),
        projects_page(view, cx),
        keymap_page(view),
        about_page(view, cx),
    ]
}

/// "Dark theme" / "Light theme" heading over its card picker.
fn theme_section(title: &'static str, view: &Entity<Osier>, dark: bool, cx: &App) -> AnyElement {
    v_flex()
        .w_full()
        .gap_2()
        .child(div().text_sm().child(title))
        .child(crate::theme_cards::theme_picker(view, dark, cx))
        .into_any_element()
}

fn appearance_page(view: &Entity<Osier>, cx: &App) -> SettingPage {
    let store = ThemeStore::global(cx);
    let dux_available = store.dux_available();
    SettingPage::new("Appearance")
        .icon(Icon::new(Lucide::Palette))
        .default_open(true)
        .resettable(true)
        .group(SettingGroup::new().title("Theme").items(vec![
                SettingItem::new(
                    "Follow Dux Shell colours",
                    SettingField::switch(
                        {
                            let v = view.clone();
                            move |cx: &App| v.read(cx).dux_sync_on(cx)
                        },
                        {
                            let v = view.clone();
                            move |on: bool, cx: &mut App| {
                                v.update(cx, |o, cx| o.set_dux_sync(on, cx))
                            }
                        },
                    )
                    .default_value(dux_available),
                )
                .description(if dux_available {
                    "Take the theme from DankMaterialShell's colours and follow them live. \
                     Turning it on sets the mode to System; the theme pickers below are off \
                     while it is on."
                } else {
                    "Not detected: no dms-colors.json in the cache directory."
                })
                .disabled(!dux_available),
                SettingItem::new(
                    "Higher contrast",
                    SettingField::switch(
                        get(view, |o| o.settings.appearance.high_contrast),
                        set(view, |o, v: bool, _| o.settings.appearance.high_contrast = v),
                    )
                    .default_value(false),
                )
                .description(
                    "Selections, hovers and the sidebar highlight in a stronger tint of the \
                     theme's main colour, like the switches.",
                ),
                SettingItem::new(
                    "Show background tasks",
                    SettingField::switch(
                        get(view, |o| o.settings.appearance.show_background_tasks),
                        set(view, |o, v: bool, _| {
                            o.settings.appearance.show_background_tasks = v
                        }),
                    )
                    .default_value(false),
                )
                .description(
                    "A spinner in the status bar while Osier works quietly: re-reading a project after \
                     a change, checking project status, loading GitHub lists, diffs and branch \
                     lookups. Fetch, pull, push, clone and create always show.",
                ),
                SettingItem::new(
                    "Mode",
                    SettingField::dropdown(
                        vec![
                            ("dark".into(), "Dark".into()),
                            ("light".into(), "Light".into()),
                            ("system".into(), "System".into()),
                        ],
                        get(view, |o| {
                            match o.settings.appearance.mode {
                                ThemeMode::Dark => "dark",
                                ThemeMode::Light => "light",
                                ThemeMode::System => "system",
                            }
                            .into()
                        }),
                        set(view, |o, v: SharedString, _| {
                            o.settings.appearance.mode = match v.as_ref() {
                                "light" => ThemeMode::Light,
                                "system" => ThemeMode::System,
                                _ => ThemeMode::Dark,
                            }
                        }),
                    )
                    .default_value("dark".to_string()),
                )
                .description("System follows the desktop, or Dux Shell when its sync is on."),
                SettingItem::new(
                    "When the desktop has no preference",
                    SettingField::dropdown(
                        vec![("dark".into(), "Dark".into()), ("light".into(), "Light".into())],
                        get(view, |o| {
                            match o.settings.appearance.system_fallback {
                                osier_config::settings::SystemFallback::Dark => "dark",
                                osier_config::settings::SystemFallback::Light => "light",
                            }
                            .into()
                        }),
                        set(view, |o, v: SharedString, _| {
                            o.settings.appearance.system_fallback = if v.as_ref() == "light" {
                                osier_config::settings::SystemFallback::Light
                            } else {
                                osier_config::settings::SystemFallback::Dark
                            }
                        }),
                    )
                    .default_value("dark".to_string()),
                )
                .description(
                    "System mode uses this when the desktop states no light or dark preference, \
                     or has no settings portal.",
                ),
                SettingItem::render({
                    let v = view.clone();
                    move |_, _, cx| theme_section("Dark theme", &v, true, cx)
                }),
                SettingItem::render({
                    let v = view.clone();
                    move |_, _, cx| theme_section("Light theme", &v, false, cx)
                }),
            ]))
        .group(
            SettingGroup::new().title("Sidebar").item(
                SettingItem::new(
                    "Uncommitted changes",
                    SettingField::dropdown(
                        vec![
                            ("icon".into(), "Icon colour".into()),
                            ("dot".into(), "Dot".into()),
                        ],
                        get(view, |o| {
                            match o.settings.appearance.changes_indicator {
                                osier_config::settings::ChangesIndicator::Icon => "icon",
                                osier_config::settings::ChangesIndicator::Dot => "dot",
                            }
                            .into()
                        }),
                        set(view, |o, v: SharedString, _| {
                            o.settings.appearance.changes_indicator = if v.as_ref() == "dot" {
                                osier_config::settings::ChangesIndicator::Dot
                            } else {
                                osier_config::settings::ChangesIndicator::Icon
                            }
                        }),
                    )
                    .default_value("icon".to_string()),
                )
                .description("How a project with uncommitted changes is marked in the sidebar."),
            ),
        )
        .group(
            SettingGroup::new()
                .title("Tooltips")
                .item(SettingItem::render({
                    let v = view.clone();
                    move |options, _, cx| {
                        let o = v.read(cx);
                        let ms = o
                            .settings
                            .appearance
                            .tooltip_delay_ms
                            .min(crate::app::TOOLTIP_DELAY_MAX_MS);
                        let slider = o.tooltip_delay.clone();
                        h_flex()
                            .w_full()
                            .gap_3()
                            .items_center()
                            .child(div().text_sm().flex_none().child("Show after"))
                            .child(
                                div().flex_1().child(
                                    gpui_kit::component::slider::Slider::new(&slider)
                                        .disabled(options.is_disabled()),
                                ),
                            )
                            .child(div().text_sm().w(px(64.)).flex_none().child(if ms == 0 {
                                "at once".to_string()
                            } else {
                                format!("{ms} ms")
                            }))
                    }
                }))
                .description(
                    "How long the pointer rests on a button or control before its tooltip \
                     appears. Default: at once.",
                ),
        )
        .group(
            SettingGroup::new().title("Lists").item(
                SettingItem::new(
                    "Show menu buttons on rows",
                    SettingField::switch(
                        get(view, |o| o.settings.appearance.row_menu_buttons),
                        set(view, |o, v: bool, _| {
                            o.settings.appearance.row_menu_buttons = v
                        }),
                    )
                    .default_value(false),
                )
                .description(
                    "Add a ... button to the end of commit, branch, tag, remote, worktree and \
                     patch rows. Right-clicking a row opens the same menu whether this is on or \
                     off.",
                ),
            ),
        )
        .group(
            SettingGroup::new()
                .title("Fonts")
                .description("Leave empty to use the theme's fonts.")
                .items(vec![
                    SettingItem::new(
                        "UI font",
                        SettingField::input(
                            get(view, |o| {
                                o.settings
                                    .appearance
                                    .ui_font
                                    .clone()
                                    .unwrap_or_default()
                                    .into()
                            }),
                            set(view, |o, v: SharedString, _| {
                                o.settings.appearance.ui_font = opt_string(v)
                            }),
                        )
                        .default_value(""),
                    ),
                    SettingItem::new(
                        "Monospace font",
                        SettingField::input(
                            get(view, |o| {
                                o.settings
                                    .appearance
                                    .mono_font
                                    .clone()
                                    .unwrap_or_default()
                                    .into()
                            }),
                            set(view, |o, v: SharedString, _| {
                                o.settings.appearance.mono_font = opt_string(v)
                            }),
                        )
                        .default_value(""),
                    ),
                    SettingItem::new(
                        "Font size",
                        SettingField::number_input(
                            NumberFieldOptions {
                                min: 0.0,
                                max: 40.0,
                                step: 1.0,
                            },
                            get(view, |o| {
                                f64::from(o.settings.appearance.font_size.unwrap_or(0.0))
                            }),
                            set(view, |o, v: f64, _| {
                                o.settings.appearance.font_size = (v > 0.0).then_some(v as f32)
                            }),
                        )
                        .default_value(0.0),
                    )
                    .description("0 uses the theme's size."),
                ]),
        )
}

fn diff_page(view: &Entity<Osier>) -> SettingPage {
    let d = Settings::default().diff;
    SettingPage::new("Diff")
        .icon(Icon::new(Lucide::FileDiff))
        .resettable(true)
        .group(SettingGroup::new().title("Diff pane").items(vec![
            SettingItem::new(
                "Layout",
                SettingField::dropdown(
                    vec![
                        ("unified".into(), "Unified".into()),
                        ("split".into(), "Split".into()),
                    ],
                    get(view, |o| match o.settings.diff.mode {
                        osier_config::settings::DiffMode::Split => "split".into(),
                        osier_config::settings::DiffMode::Unified => "unified".into(),
                    }),
                    set(view, |o, v: SharedString, _| {
                        o.settings.diff.mode = if v.as_ref() == "split" {
                            osier_config::settings::DiffMode::Split
                        } else {
                            osier_config::settings::DiffMode::Unified
                        }
                    }),
                )
                .default_value("unified".to_string()),
            ),
            SettingItem::new(
                "Context lines",
                SettingField::number_input(
                    NumberFieldOptions {
                        min: 0.0,
                        max: 30.0,
                        step: 1.0,
                    },
                    get(view, |o| f64::from(o.settings.diff.context_lines)),
                    set(view, |o, v: f64, cx| {
                        o.settings.diff.context_lines = v.max(0.0) as u32;
                        o.reload_selection(cx);
                    }),
                )
                .default_value(f64::from(d.context_lines)),
            ),
            SettingItem::new(
                "Line numbers",
                SettingField::switch(
                    get(view, |o| o.settings.diff.line_numbers),
                    set(view, |o, v: bool, _| o.settings.diff.line_numbers = v),
                )
                .default_value(d.line_numbers),
            ),
            SettingItem::new(
                "Soft wrap",
                SettingField::switch(
                    get(view, |o| o.settings.diff.soft_wrap),
                    set(view, |o, v: bool, _| o.settings.diff.soft_wrap = v),
                )
                .default_value(d.soft_wrap),
            )
            .description("Wrap long lines instead of scrolling sideways."),
            SettingItem::new(
                "Large file limit (MB)",
                SettingField::number_input(
                    NumberFieldOptions {
                        min: 1.0,
                        max: 500.0,
                        step: 1.0,
                    },
                    get(view, |o| {
                        o.settings.diff.large_file_bytes as f64 / (1024.0 * 1024.0)
                    }),
                    set(view, |o, v: f64, _| {
                        o.settings.diff.large_file_bytes =
                            (v.max(1.0) * 1024.0 * 1024.0) as u64
                    }),
                )
                .default_value(d.large_file_bytes as f64 / (1024.0 * 1024.0)),
            )
            .description("Files above this size ask before rendering."),
        ]))
}

fn git_page(view: &Entity<Osier>) -> SettingPage {
    let defaults = Settings::default();
    SettingPage::new("Git")
        .icon(Icon::new(Lucide::GitBranch))
        .resettable(true)
        .group(SettingGroup::new().title("Log").items(vec![
            SettingItem::new(
                "Commits per page",
                SettingField::number_input(
                    NumberFieldOptions {
                        min: 50.0,
                        max: 10000.0,
                        step: 50.0,
                    },
                    get(view, |o| o.settings.git.log_page_size as f64),
                    set(view, |o, v: f64, _| {
                        o.settings.git.log_page_size = v.max(50.0) as usize
                    }),
                )
                .default_value(defaults.git.log_page_size as f64),
            )
            .description("Applies to repositories opened from now on."),
            SettingItem::new(
                "Show command log on startup",
                SettingField::switch(
                    get(view, |o| o.settings.git.show_command_log),
                    set(view, |o, v: bool, _| o.settings.git.show_command_log = v),
                )
                .default_value(defaults.git.show_command_log),
            ),
            SettingItem::new(
                "Branch and tag badges",
                SettingField::dropdown(
                    vec![
                        ("left".into(), "Before the subject".into()),
                        ("right".into(), "At the end of the line".into()),
                    ],
                    get(view, |o| {
                        match o.settings.git.log_badges {
                            osier_config::settings::LogBadges::Left => "left",
                            osier_config::settings::LogBadges::Right => "right",
                        }
                        .into()
                    }),
                    set(view, |o, v: SharedString, _| {
                        o.settings.git.log_badges = if v.as_ref() == "right" {
                            osier_config::settings::LogBadges::Right
                        } else {
                            osier_config::settings::LogBadges::Left
                        }
                    }),
                )
                .default_value("left".to_string()),
            ),
            SettingItem::new(
                "Commits not pushed yet",
                SettingField::dropdown(
                    vec![
                        ("dim".into(), "Dim".into()),
                        ("colour".into(), "Change colour".into()),
                    ],
                    get(view, |o| {
                        match o.settings.git.log_unpushed {
                            osier_config::settings::UnpushedStyle::Dim => "dim",
                            osier_config::settings::UnpushedStyle::Colour => "colour",
                        }
                        .into()
                    }),
                    set(view, |o, v: SharedString, _| {
                        o.settings.git.log_unpushed = if v.as_ref() == "colour" {
                            osier_config::settings::UnpushedStyle::Colour
                        } else {
                            osier_config::settings::UnpushedStyle::Dim
                        }
                    }),
                )
                .default_value("dim".to_string()),
            )
            .description(
                "How the Log graph shows commits that are on a local branch but no remote yet.",
            ),
        ]))
        .group(SettingGroup::new().title("Prompts and viewer").items(vec![
            SettingItem::new(
                "Osier askpass window",
                SettingField::switch(
                    get(view, |o| o.settings.auth.askpass),
                    set(view, |o, v: bool, _| o.settings.auth.askpass = v),
                )
                .default_value(defaults.auth.askpass),
            )
            .description(
                "Route ssh passphrase and git password prompts to Osier's own window. Off uses your ssh agent and git credential helpers only.",
            ),
            SettingItem::new(
                "Remote images in markdown",
                SettingField::switch(
                    get(view, |o| o.settings.viewer.remote_images),
                    set(view, |o, v: bool, _| o.settings.viewer.remote_images = v),
                )
                .default_value(defaults.viewer.remote_images),
            )
            .description("Fetch http(s) images such as README badges. Off shows repository files only."),
            SettingItem::new(
                "Markdown files open as",
                SettingField::dropdown(
                    vec![
                        ("rendered".into(), "Rendered".into()),
                        ("code".into(), "Code".into()),
                    ],
                    get(view, |o| match o.settings.viewer.markdown_default {
                        osier_config::settings::MarkdownMode::Rendered => "rendered".into(),
                        osier_config::settings::MarkdownMode::Code => "code".into(),
                    }),
                    set(view, |o, v: SharedString, _| {
                        o.settings.viewer.markdown_default = if v.as_ref() == "code" {
                            osier_config::settings::MarkdownMode::Code
                        } else {
                            osier_config::settings::MarkdownMode::Rendered
                        };
                        // A new default makes per-type overrides that equal it redundant.
                        let d = o.settings.viewer.markdown_default;
                        o.settings.viewer.markdown_modes.retain(|_, m| *m != d);
                    }),
                )
                .default_value("rendered".to_string()),
            )
            .description("The Rendered/Code toggle remembers its choice per file type on top of this."),
        ]))
}

fn editors_page(view: &Entity<Osier>) -> SettingPage {
    let mut options: Vec<(SharedString, SharedString)> =
        vec![("".into(), "System default (xdg-open)".into())];
    for (name, template) in Osier::detected_editors() {
        options.push((template.into(), name.into()));
    }
    options.push(("custom".into(), "Custom command (below)".into()));
    SettingPage::new("Editors")
        .icon(Icon::new(Lucide::SquarePen))
        .resettable(true)
        .group(
            SettingGroup::new()
                .title("Open in editor")
                .description(
                    "Placeholders: {path} is the repository, {file} the file and {line} the line.",
                )
                .items(vec![
                    SettingItem::new(
                        "Editor",
                        SettingField::dropdown(
                            options.clone(),
                            {
                                let options = options.clone();
                                get(view, move |o| {
                                    let current =
                                        o.settings.editor.command.clone().unwrap_or_default();
                                    if options.iter().any(|(v, _)| v.as_ref() == current) {
                                        current.into()
                                    } else {
                                        "custom".into()
                                    }
                                })
                            },
                            set(view, |o, v: SharedString, _| {
                                if v.as_ref() != "custom" {
                                    o.settings.editor.command = opt_string(v);
                                }
                            }),
                        )
                        .default_value(""),
                    )
                    .description("Editors found on PATH."),
                    SettingItem::new(
                        "Editor command",
                        SettingField::input(
                            get(view, |o| {
                                o.settings.editor.command.clone().unwrap_or_default().into()
                            }),
                            set(view, |o, v: SharedString, _| {
                                o.settings.editor.command = opt_string(v)
                            }),
                        )
                        .default_value(""),
                    )
                    .layout(Axis::Vertical)
                    .description("For example: zed {file}:{line}"),
                    SettingItem::new(
                        "Terminal command",
                        SettingField::input(
                            get(view, |o| {
                                o.settings
                                    .editor
                                    .terminal
                                    .clone()
                                    .unwrap_or_default()
                                    .into()
                            }),
                            set(view, |o, v: SharedString, _| {
                                o.settings.editor.terminal = opt_string(v)
                            }),
                        )
                        .default_value(""),
                    )
                    .layout(Axis::Vertical)
                    .description("Empty tries common terminals. {path} is the repository."),
                ]),
        )
}

fn accounts_page(view: &Entity<Osier>, cx: &App) -> SettingPage {
    let osier = view.read(cx);
    let mut hosts: Vec<String> = osier.gh_accounts.iter().map(|a| a.host.clone()).collect();
    hosts.sort();
    hosts.dedup();
    let mut host_items = Vec::new();
    for host in hosts {
        let mut options: Vec<(SharedString, SharedString)> =
            vec![("".into(), "Ask when needed".into())];
        for a in osier.gh_accounts.iter().filter(|a| a.host == host) {
            options.push((a.login.clone().into(), a.login.clone().into()));
        }
        let (h1, h2) = (host.clone(), host.clone());
        host_items.push(
            SettingItem::new(
                host.clone(),
                SettingField::dropdown(
                    options,
                    get(view, move |o| {
                        o.settings
                            .auth
                            .default_accounts
                            .get(&h1)
                            .cloned()
                            .unwrap_or_default()
                            .into()
                    }),
                    set(view, move |o, v: SharedString, _| {
                        if v.is_empty() {
                            o.settings.auth.default_accounts.remove(&h2);
                        } else {
                            o.settings
                                .auth
                                .default_accounts
                                .insert(h2.clone(), v.to_string());
                        }
                    }),
                )
                .default_value(""),
            )
            .description("Used for https remotes on this host when the project has no choice."),
        );
    }
    let v1 = view.clone();
    let v2 = view.clone();
    let v3 = view.clone();
    let mut page = SettingPage::new("Accounts")
        .icon(Icon::new(Lucide::KeyRound))
        .group(
            SettingGroup::new()
                .title("GitHub CLI accounts")
                .description("Detected with `gh auth status`. Tokens are read only inside Osier's credential helper, never shown.")
                .item(SettingItem::render(move |options, _, cx| {
                    let o = v1.read(cx);
                    let muted = cx.theme().muted_foreground;
                    let mut col = v_flex().gap_1().w_full();
                    if o.gh_accounts.is_empty() {
                        col = col.child(
                            div().text_sm().text_color(muted).child(
                                "No accounts. Run `gh auth login` in a terminal, then refresh.",
                            ),
                        );
                    }
                    for a in &o.gh_accounts {
                        col = col.child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(Icon::new(Lucide::Github).xsmall())
                                .child(div().text_sm().child(a.login.clone()))
                                .child(div().text_xs().text_color(muted).child(a.host.clone()))
                                .child(div().text_xs().text_color(muted).child(a.git_protocol.clone()))
                                .when(a.active, |this| {
                                    this.child(Tag::info().xsmall().child("gh default"))
                                })
                                .when(a.state != "ok" && !a.state.is_empty(), |this| {
                                    this.child(Tag::warning().xsmall().child(a.state.clone()))
                                }),
                        );
                    }
                    let v = v1.clone();
                    col.child(
                        h_flex().pt_1().child(
                            Button::new("refresh-accounts").test_id("refresh-accounts")
                                .outline()
                                .with_size(options.size())
                                .icon(Lucide::RefreshCw)
                                .label("Refresh")
                                .on_click(move |_, _, cx| {
                                    v.update(cx, |o, cx| o.load_gh_accounts(cx));
                                }),
                        ),
                    )
                })),
        );
    if !host_items.is_empty() {
        page = page.group(
            SettingGroup::new()
                .title("Default account per host")
                .items(host_items),
        );
    }
    page.group(
            SettingGroup::new()
                .title("Per-project choices")
                .description("Accounts chosen with \"always use for this project\". Session-only choices are forgotten on exit.")
                .item(SettingItem::render(move |options, _, cx| {
                    let o = v2.read(cx);
                    let muted = cx.theme().muted_foreground;
                    let mut col = v_flex().gap_1().w_full();
                    if o.state.accounts.is_empty() {
                        col = col.child(div().text_sm().text_color(muted).child("None yet."));
                    }
                    for (ix, (repo, login)) in o.state.accounts.iter().enumerate() {
                        let session = o.session_only_accounts.contains(repo);
                        let v = v2.clone();
                        let key = repo.clone();
                        col = col.child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_sm()
                                        .child(crate::format::tilde(std::path::Path::new(repo))),
                                )
                                .child(div().text_sm().child(login.clone()))
                                .when(session, |this| {
                                    this.child(Tag::secondary().xsmall().child("this session"))
                                })
                                .child(
                                    Button::new(("reset-account", ix))
                                        .ghost()
                                        .with_size(options.size())
                                        .label("Reset")
                                        .on_click(move |_, _, cx| {
                                            v.update(cx, |o, cx| {
                                                o.clear_account_override(key.clone(), cx)
                                            });
                                        }),
                                ),
                        );
                    }
                    let v = v2.clone();
                    col.when(!o.state.accounts.is_empty(), |this| {
                        this.child(
                            h_flex().pt_1().child(
                                Button::new("reset-all-accounts").test_id("reset-all-accounts")
                                    .outline()
                                    .with_size(options.size())
                                    .label("Reset all")
                                    .on_click(move |_, _, cx| {
                                        v.update(cx, |o, cx| o.clear_all_account_overrides(cx));
                                    }),
                            ),
                        )
                    })
                })),
        )
        .group(
            SettingGroup::new()
                .title("SSH")
                .description("Key names only; private keys are never read.")
                .item(SettingItem::render(move |_, _, cx| {
                    let o = v3.read(cx);
                    let muted = cx.theme().muted_foreground;
                    let status = o.ssh_status.clone().unwrap_or_default();
                    let agent = if status.agent_socket {
                        format!(
                            "running, {} {} loaded",
                            status.loaded_identities.len(),
                            if status.loaded_identities.len() == 1 { "identity" } else { "identities" }
                        )
                    } else {
                        "no agent socket (SSH_AUTH_SOCK unset)".to_string()
                    };
                    let keys = if status.key_files.is_empty() {
                        "none found in ~/.ssh".to_string()
                    } else {
                        status
                            .key_files
                            .iter()
                            .filter_map(|p| p.file_name())
                            .map(|n| n.to_string_lossy().into_owned())
                            .collect::<Vec<_>>()
                            .join(", ")
                    };
                    v_flex()
                        .gap_1()
                        .w_full()
                        .child(kv("Agent", agent, cx))
                        .when(!status.loaded_identities.is_empty(), |this| {
                            this.child(kv("Loaded", status.loaded_identities.join(", "), cx))
                        })
                        .child(kv("Keys", keys, cx))
                        .child(
                            div().text_xs().text_color(muted).child(
                                "ssh remotes use these directly; the gh accounts above only apply to https remotes.",
                            ),
                        )
                })),
        )
}

fn projects_page(view: &Entity<Osier>, cx: &App) -> SettingPage {
    let v1 = view.clone();
    let v2 = view.clone();
    SettingPage::new("Projects")
        .icon(Icon::new(Lucide::FolderGit2))
        .group(
            SettingGroup::new()
                .title("Scan directories")
                .description("Repositories found directly inside these directories are listed under Recents and cannot be removed individually.")
                .item(SettingItem::render(move |options, _, cx| {
                    let o = v1.read(cx);
                    let muted = cx.theme().muted_foreground;
                    let mut col = v_flex().gap_1().w_full();
                    if o.settings.projects.scan_dirs.is_empty() {
                        col = col.child(div().text_sm().text_color(muted).child("None."));
                    }
                    for (ix, dir) in o.settings.projects.scan_dirs.iter().enumerate() {
                        let v = v1.clone();
                        let d = dir.clone();
                        col = col.child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(Icon::new(Lucide::Folder).xsmall())
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_sm()
                                        .child(crate::format::tilde(dir)),
                                )
                                .child(
                                    Button::new(("remove-scan-dir", ix))
                                        .ghost()
                                        .with_size(options.size())
                                        .icon(Lucide::X)
                                        .tooltip("Remove")
                                        .on_click(move |_, _, cx| {
                                            v.update(cx, |o, cx| o.remove_scan_dir(d.clone(), cx));
                                        }),
                                ),
                        );
                    }
                    let v = v1.clone();
                    col.child(
                        h_flex().pt_1().child(
                            Button::new("add-scan-dir").test_id("add-scan-dir")
                                .outline()
                                .with_size(options.size())
                                .icon(Lucide::FolderPlus)
                                .label("Add directory…")
                                .on_click(move |_, window, cx| {
                                    v.update(cx, |o, cx| o.add_scan_dir_prompt(window, cx));
                                }),
                        ),
                    )
                }))
                .item(
                    SettingItem::new(
                        "Scan depth",
                        SettingField::number_input(
                            NumberFieldOptions {
                                min: 1.0,
                                max: 3.0,
                                step: 1.0,
                            },
                            get(view, |o| f64::from(o.settings.projects.scan_depth.unwrap_or(1))),
                            set(view, |o, v: f64, _| {
                                o.settings.projects.scan_depth = Some(v.clamp(1.0, 3.0) as u8)
                            }),
                        )
                        .default_value(1.0),
                    )
                    .description("1 looks at direct children only."),
                ),
        )
        .group(
            SettingGroup::new().title("Switching projects").item(
                SettingItem::new(
                    "When Changes is empty",
                    SettingField::dropdown(
                        vec![
                            ("stay".into(), "Don't switch".into()),
                            ("log".into(), "Log".into()),
                            ("branches".into(), "Branches".into()),
                            ("stashes".into(), "Stashes".into()),
                            ("files".into(), "Files".into()),
                            ("patches".into(), "Patches".into()),
                        ],
                        get(view, |o| {
                            use osier_config::settings::EmptyChangesTab as E;
                            match o.settings.projects.empty_changes_tab {
                                E::Stay => "stay",
                                E::Log => "log",
                                E::Branches => "branches",
                                E::Stashes => "stashes",
                                E::Files => "files",
                                E::Patches => "patches",
                            }
                            .into()
                        }),
                        set(view, |o, v: SharedString, _| {
                            use osier_config::settings::EmptyChangesTab as E;
                            o.settings.projects.empty_changes_tab = match v.as_ref() {
                                "stay" => E::Stay,
                                "branches" => E::Branches,
                                "stashes" => E::Stashes,
                                "files" => E::Files,
                                "patches" => E::Patches,
                                _ => E::Log,
                            };
                        }),
                    )
                    .default_value("log".to_string()),
                )
                .description(
                    "When you switch to or open a project while on the Changes tab and it has \
                     nothing to commit, show this tab instead. Switching to a project with \
                     changes brings Changes back.",
                ),
            ),
        )
        .group(
            highlightable(SettingGroup::new(), SettingsTarget::RECENT_STATUS, view, cx)
                .title("Status of projects that are not open")
                .description("Ahead/behind badges and the changes marker on pinned, recent and scanned projects.")
                .item(
                    SettingItem::new(
                        "Show status",
                        SettingField::dropdown(
                            vec![
                                ("never".into(), "Never".into()),
                                ("cached".into(), "From when it was last open".into()),
                                ("watch".into(), "Watch git (ahead/behind only)".into()),
                                ("check".into(), "Watch git and check every N minutes".into()),
                            ],
                            get(view, |o| {
                                use osier_config::settings::RecentStatusMode as M;
                                match o.settings.projects.recent_status {
                                    M::Never => "never",
                                    M::Cached => "cached",
                                    M::Watch => "watch",
                                    M::Check => "check",
                                }
                                .into()
                            }),
                            set(view, |o, v: SharedString, _| {
                                use osier_config::settings::RecentStatusMode as M;
                                o.settings.projects.recent_status = match v.as_ref() {
                                    "cached" => M::Cached,
                                    "watch" => M::Watch,
                                    "check" => M::Check,
                                    _ => M::Never,
                                };
                            }),
                        )
                        .default_value("never".to_string()),
                    )
                    .description("Never runs nothing in the background. Watch modes use a few file watches per repository and one git process at a time."),
                )
                .item(SettingItem::render({
                    let v = view.clone();
                    move |options, _, cx| {
                        let o = v.read(cx);
                        let checking = o.settings.projects.recent_status
                            == osier_config::settings::RecentStatusMode::Check;
                        let minutes = o.settings.projects.recent_status_minutes.clamp(1, 60);
                        let slider = o.recent_interval.clone();
                        h_flex()
                            .w_full()
                            .gap_3()
                            .items_center()
                            .when(!checking, |this| this.opacity(0.5))
                            .child(div().text_sm().flex_none().child("Check every"))
                            .child(
                                div().flex_1().child(
                                    gpui_kit::component::slider::Slider::new(&slider)
                                        .disabled(!checking || options.is_disabled()),
                                ),
                            )
                            .child(div().text_sm().w(px(64.)).flex_none().child(if minutes == 1 {
                                "1 minute".to_string()
                            } else {
                                format!("{minutes} minutes")
                            }))
                    }
                }))
                .item(SettingItem::render({
                    let v = view.clone();
                    move |options, _, cx| {
                        use osier_config::settings::RecentStatusMode as M;
                        let mode = v.read(cx).settings.projects.recent_status;
                        let hint = match mode {
                            M::Never => "Turn the status on to scan.",
                            M::Cached => "One full check now; the results are cached.",
                            M::Watch => "Re-read ahead/behind of every project now.",
                            M::Check => "Full check of every project now; the timer restarts.",
                        };
                        let v = v.clone();
                        h_flex()
                            .w_full()
                            .gap_3()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(hint),
                            )
                            .child(
                                Button::new("scan-recent-now")
                                    .test_id("scan-recent-now")
                                    .outline()
                                    .with_size(options.size())
                                    .icon(Lucide::RefreshCw)
                                    .label("Scan now")
                                    .disabled(mode == M::Never || options.is_disabled())
                                    .on_click(move |_, window, cx| {
                                        v.update(cx, |o, cx| o.scan_recent_now(window, cx));
                                    }),
                            )
                    }
                })),
        )
        .group(
            SettingGroup::new()
                .title("Recents")
                .item(SettingItem::render(move |options, _, cx| {
                    let o = v2.read(cx);
                    let n = o.state.recent_not_open().count();
                    let v = v2.clone();
                    h_flex()
                        .w_full()
                        .justify_between()
                        .items_center()
                        .gap_3()
                        .child(div().text_sm().child(format!(
                            "{n} recent {} not currently open",
                            if n == 1 { "repository" } else { "repositories" }
                        )))
                        .child(
                            Button::new("clear-recents").test_id("clear-recents")
                                .outline()
                                .with_size(options.size())
                                .label("Clear all recents")
                                .disabled(n == 0)
                                .on_click(move |_, window, cx| {
                                    v.update(cx, |o, cx| o.confirm_clear_recents(window, cx));
                                }),
                        )
                })),
        )
}

fn about_page(view: &Entity<Osier>, cx: &App) -> SettingPage {
    let info = view.read(cx).runtime_info();
    let text = info.to_string();
    let config_file = view.read(cx).paths.settings_file();
    let config_dir = view.read(cx).paths.config_dir().to_path_buf();
    let v1 = view.clone();
    let v2 = view.clone();
    SettingPage::new("About")
        .icon(Icon::new(Lucide::Info))
        .group(
            SettingGroup::new()
                .title("Runtime")
                .description("Include this block in bug reports (`osier --version --verbose` prints the same).")
                .item(SettingItem::render(move |options, _, cx| {
                    let text = text.clone();
                    let mut col = v_flex().gap_1().w_full();
                    for (label, value) in info.rows() {
                        col = col.child(kv(label, value, cx));
                    }
                    col.child(
                        h_flex().pt_1().child(
                            Button::new("copy-runtime").test_id("copy-runtime")
                                .outline()
                                .with_size(options.size())
                                .icon(Lucide::Copy)
                                .label("Copy")
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                                }),
                        ),
                    )
                })),
        )
        .group(
            SettingGroup::new()
                .title("Configuration")
                .description("config.toml is re-read when it changes on disk, so it is safe to manage with dotfiles.")
                .item(SettingItem::render(move |options, _, _| {
                    let (f, d) = (config_file.clone(), config_dir.clone());
                    let (va, vb) = (v1.clone(), v1.clone());
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("edit-config").test_id("edit-config")
                                .outline()
                                .with_size(options.size())
                                .icon(Lucide::SquarePen)
                                .label("Edit config.toml")
                                .on_click(move |_, window, cx| {
                                    let f = f.clone();
                                    va.update(cx, |o, cx| o.open_externally(&f, window, cx));
                                }),
                        )
                        .child(
                            Button::new("open-config-dir").test_id("open-config-dir")
                                .outline()
                                .with_size(options.size())
                                .icon(Lucide::FolderOpen)
                                .label("Open folder")
                                .on_click(move |_, window, cx| {
                                    let d = d.clone();
                                    vb.update(cx, |o, cx| o.open_externally(&d, window, cx));
                                }),
                        )
                })),
        )
        .group(
            SettingGroup::new().title("Osier").item(SettingItem::render(move |options, _, cx| {
                let _ = &v2;
                v_flex()
                    .gap_2()
                    .w_full()
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("A native git GUI for Linux. MIT licensed."),
                    )
                    .child(
                        h_flex().gap_2().child(
                            Button::new("repo-link").test_id("repo-link")
                                .outline()
                                .with_size(options.size())
                                .icon(Lucide::Github)
                                .label("github.com/ReyArlena/osier")
                                .on_click(|_, _, cx| cx.open_url("https://github.com/ReyArlena/osier")),
                        ),
                    )
            })),
        )
}

fn keymap_page(view: &Entity<Osier>) -> SettingPage {
    use crate::keymap::{Scope, actions};
    let specs = actions();
    let mut categories: Vec<&'static str> = Vec::new();
    for s in &specs {
        if !categories.contains(&s.category) {
            categories.push(s.category);
        }
    }
    let v_top = view.clone();
    let mut page = SettingPage::new("Keymap")
        .icon(Icon::new(Lucide::Keyboard))
        .group(
            SettingGroup::new()
                .title("Key bindings")
                .description(
                    "Click + and press the keys to add a binding, x to remove one. Plain keys (letters, /, space) never fire while typing in a text field.",
                )
                .item(SettingItem::render(move |options, _, _| {
                    let (va, vb) = (v_top.clone(), v_top.clone());
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("keymap-edit-file")
                                .test_id("keymap-edit-file")
                                .outline()
                                .with_size(options.size())
                                .icon(Lucide::SquarePen)
                                .label("Edit keymap.toml")
                                .on_click(move |_, window, cx| {
                                    va.update(cx, |o, cx| o.edit_keymap_file(window, cx));
                                }),
                        )
                        .child(
                            Button::new("keymap-reset-all")
                                .test_id("keymap-reset-all")
                                .outline()
                                .with_size(options.size())
                                .label("Reset all to defaults")
                                .on_click(move |_, window, cx| {
                                    vb.update(cx, |o, cx| o.reset_keymap(window, cx));
                                }),
                        )
                })),
        );
    for category in categories {
        let mut group = SettingGroup::new().title(category);
        if category == "Prefix" {
            group = group.description(
                "Press the prefix key, then a binding's key. While it waits, the status bar shows \
                 \"Prefix key pressed\"; Escape, a click, an unbound key or the timeout cancel \
                 it. Each key below picks how it fires with its three buttons: No prefix (the \
                 key alone), Prefix only (the prefix key, then the key) or Either (both work). \
                 Keys after the prefix also work while typing in a text field.",
            );
        }
        if category == "Context menus" {
            group = group.description(
                "Run a right-click menu entry on the selected row with a key. One key covers \
                 every row that offers the entry; others ignore it. These keys cannot share a \
                 key with any other binding.",
            );
        }
        for spec in specs.iter().filter(|s| s.category == category) {
            let (id, scope) = (spec.id, spec.scope);
            let v = view.clone();
            let mut item = SettingItem::new(
                spec.name,
                SettingField::render(move |options, _, cx| key_row(&v, id, options, cx)),
            );
            match scope {
                Scope::List => item = item.description("In the Changes, Log and other lists"),
                Scope::Row => {
                    item = item.description(format!("On the selected row: {}", spec.detail))
                }
                Scope::Settings => item = item.description("While settings is open"),
                Scope::App => {}
            }
            group = group.item(item);
        }
        if category == "Prefix" {
            let v = view.clone();
            group = group.item(SettingItem::render(move |options, _, cx| {
                let o = v.read(cx);
                let secs = o
                    .settings
                    .keys
                    .prefix_timeout_secs
                    .min(crate::app::PREFIX_TIMEOUT_MAX_SECS);
                let slider = o.prefix_timeout.clone();
                h_flex()
                    .w_full()
                    .gap_3()
                    .items_center()
                    .child(div().text_sm().flex_none().child("Stop waiting after"))
                    .child(
                        div().flex_1().child(
                            gpui_kit::component::slider::Slider::new(&slider)
                                .disabled(options.is_disabled()),
                        ),
                    )
                    .child(div().text_sm().w(px(110.)).flex_none().child(match secs {
                        0 => "never (Esc or a key)".to_string(),
                        1 => "1 second".to_string(),
                        n => format!("{n} seconds"),
                    }))
            }));
        }
        page = page.group(group);
    }
    page
}

/// No prefix / Prefix only / Either for one key, as three joined icon buttons.
fn mode_buttons(
    view: &Entity<Osier>,
    id: &'static str,
    stored: &str,
    current: crate::keymap::KeyMode,
) -> AnyElement {
    use crate::keymap::{KeyMode, pretty_key, split_mode};
    use gpui_kit::component::button::ButtonGroup;
    let bare = pretty_key(split_mode(stored).1);
    let info = |mode: KeyMode| match mode {
        KeyMode::Plain => ("plain", Lucide::Key, format!("No prefix: {bare} alone")),
        KeyMode::Prefix => (
            "prefix",
            Lucide::ChevronsRight,
            format!("Prefix only: the prefix key, then {bare}"),
        ),
        KeyMode::Either => (
            "either",
            Lucide::Split,
            format!("Either: {bare} alone, or the prefix key then {bare}"),
        ),
    };
    let mut group = ButtonGroup::new(SharedString::from(format!("keymode-{id}-{stored}")))
        .compact()
        .xsmall();
    for mode in KeyMode::ALL {
        let (name, icon, tip) = info(mode);
        group = group.child(
            Button::new(SharedString::from(format!("keymode-{id}-{stored}-{name}")))
                .test_id(format!("keymode:{id}:{stored}:{name}"))
                .icon(icon)
                .tooltip(tip)
                // The chosen mode is filled so it reads at a glance; the others are outlined.
                .map(|b| {
                    if mode == current {
                        b.primary()
                    } else {
                        b.outline()
                    }
                })
                .selected(mode == current),
        );
    }
    let v = view.clone();
    let stored = stored.to_string();
    group
        .on_click(move |picked: &Vec<usize>, window, cx| {
            if let Some(&ix) = picked.first()
                && let Some(&mode) = KeyMode::ALL.get(ix)
            {
                v.update(cx, |o, cx| o.set_key_mode(id, &stored, mode, window, cx));
            }
        })
        .into_any_element()
}

/// One action's keys: removable chips, a recorder, Reset when changed, and conflicts.
fn key_row(
    view: &Entity<Osier>,
    id: &'static str,
    options: &gpui_kit::component::setting::RenderOptions,
    cx: &App,
) -> AnyElement {
    use crate::keymap::{Keymap, conflicts};
    let o = view.read(cx);
    let keymap = Keymap::global(cx);
    let keys = keymap.keys_for(id).to_vec();
    let recording = o.keymap_recording.as_ref().is_some_and(|(r, _)| *r == id);
    let overridden = o.keymap_applied.bindings.contains_key(id);
    let clashes: Vec<&'static str> = keys.iter().flat_map(|k| conflicts(keymap, id, k)).collect();
    let muted = cx.theme().muted_foreground;
    let mut row = h_flex().gap_1().items_center().flex_wrap().justify_end();
    if keys.is_empty() && !recording {
        row = row.child(div().text_sm().text_color(muted).child("Unbound"));
    }
    for k in keys {
        let v = view.clone();
        let key = k.clone();
        let (mode, bare) = crate::keymap::split_mode(&k);
        let modes = (id != crate::keymap::PREFIX_KEY).then(|| mode_buttons(view, id, &k, mode));
        row = row.child(
            h_flex()
                .gap_0p5()
                .items_center()
                .child(
                    Tag::secondary()
                        .xsmall()
                        .child(crate::keymap::pretty_key(bare)),
                )
                .when_some(modes, |this, m| this.child(m))
                .child(
                    Button::new(SharedString::from(format!("keymap-remove-{id}-{k}")))
                        .test_id(format!("keymap-remove:{id}:{k}"))
                        .ghost()
                        .xsmall()
                        .icon(Lucide::X)
                        .tooltip("Remove")
                        .on_click(move |_, window, cx| {
                            v.update(cx, |o, cx| o.remove_action_key(id, &key, window, cx));
                        }),
                ),
        );
    }
    let v = view.clone();
    row = if recording {
        row.child(Tag::warning().xsmall().child("Press keys… (Esc cancels)"))
    } else {
        row.child(
            Button::new(SharedString::from(format!("keymap-add-{id}")))
                .test_id(format!("keymap-add:{id}"))
                .ghost()
                .with_size(options.size())
                .icon(Lucide::Plus)
                .tooltip("Add a key binding")
                .on_click(move |_, _, cx| {
                    v.update(cx, |o, cx| o.start_key_recording(id, cx));
                }),
        )
    };
    let v = view.clone();
    v_flex()
        .items_end()
        .gap_1()
        .child(row.when(overridden, |row| {
            row.child(
                Button::new(SharedString::from(format!("keymap-reset-{id}")))
                    .test_id(format!("keymap-reset:{id}"))
                    .ghost()
                    .with_size(options.size())
                    .label("Reset")
                    .on_click(move |_, window, cx| {
                        let defaults: Vec<String> = crate::keymap::actions()
                            .iter()
                            .find(|a| a.id == id)
                            .map(|a| a.defaults.iter().map(|k| k.to_string()).collect())
                            .unwrap_or_default();
                        v.update(cx, |o, cx| o.set_action_keys(id, defaults, window, cx));
                    }),
            )
        }))
        .when(!clashes.is_empty(), |this| {
            this.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().warning)
                    .child(format!("Also bound to: {}", clashes.join(", "))),
            )
        })
        .into_any_element()
}
