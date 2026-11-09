//! The settings sheet, built on gpui-kit's `Settings` component. Fields read from and write
//! to `Osier::settings` through the entity; every write saves `config.toml` and re-applies.
//! One child module per page; this file holds the sheet, the shared helpers and `SettingsTarget`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::group_box::GroupBoxVariant;
use gpui_kit::component::setting::{
    SelectIndex, SettingGroup, SettingItem, SettingPage, Settings as SettingsView,
};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::*;
use osier_config::Settings;

use crate::app::Osier;
use crate::notices::{Notice, WindowNoticeExt as _};
use crate::runtime_info::RuntimeInfo;
use crate::test_ids::TestId as _;

mod about;
mod accounts;
mod appearance;
mod diff;
mod editors;
mod git;
mod keymap;
mod projects;

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

/// The settings title bar: the app's chrome colour (like the main title bar), the page
/// area below stays the content colour, so the two read as separate surfaces.
fn header(radius: Pixels, root_focus: FocusHandle, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    h_flex()
        .flex_none()
        .h(px(48.))
        .pl_4()
        .pr_2()
        .gap_2()
        .items_center()
        .bg(theme.title_bar)
        .border_b_1()
        .border_color(theme.border)
        .rounded_t(radius)
        .child(
            Icon::new(Lucide::Settings)
                .small()
                .text_color(theme.muted_foreground),
        )
        .child(div().flex_1().font_semibold().child("Settings"))
        .child(
            Button::new("settings-close")
                .test_id("settings-close")
                .ghost()
                .small()
                .icon(Lucide::X)
                .tooltip("Close")
                .on_click(move |_, window, cx| {
                    window.close_dialog(cx);
                    root_focus.focus(window, cx);
                }),
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
        // Asks the package managers who owns the binary; once per run is enough.
        let _ = self.install_route();
        let view = cx.entity();
        let focus = self.settings_focus.clone();
        let root_focus = self.focus_handle.clone();
        window.open_dialog(cx, move |dialog, window, _| {
            // Sized from the window each time the layer is built, so it follows resizes.
            let viewport = window.viewport_size();
            let width = (viewport.width * 0.9).min(px(1100.));
            let height = (viewport.height * 0.8).min(px(820.));
            let (view, focus) = (view.clone(), focus.clone());
            let close_focus = root_focus.clone();
            let root_focus = root_focus.clone();
            dialog
                .w(width)
                .centered()
                // No padding and no stock title: the header bar and the settings sidebar run
                // to the dialog's edges, like the title bar and sidebar of the main window.
                .p_0()
                .close_button(false)
                .overlay_closable(true)
                // Put focus back on the app so shortcuts keep working after closing.
                .on_close(move |_, window, cx| root_focus.focus(window, cx))
                .content(move |content, window, cx| {
                    let pages = pages(&view, window, cx);
                    // Inside the dialog's 1px border, so one pixel less than its radius.
                    let radius = (cx.theme().radius_lg - px(1.)).max(px(0.));
                    let mut sidebar_style = StyleRefinement::default();
                    sidebar_style = sidebar_style.rounded_bl(radius);
                    let settings = SettingsView::new("osier-settings")
                        .with_group_variant(GroupBoxVariant::Fill)
                        .sidebar_style(&sidebar_style)
                        .pages(pages);
                    let settings = match target {
                        Some(t) => settings.default_selected_index(SelectIndex {
                            page_ix: t.page,
                            group_ix: Some(t.group),
                        }),
                        None => settings,
                    };
                    content.child(
                        v_flex()
                            .w_full()
                            .h(height)
                            .test_id("settings-dialog")
                            .child(header(radius, close_focus.clone(), cx))
                            .child(
                                // The header's close button stays outside this container, so
                                // the search box is still the first tab stop after it.
                                div()
                                    .flex_1()
                                    .min_h_0()
                                    .track_focus(&focus)
                                    .key_context("OsierSettings")
                                    .on_action({
                                        let focus = focus.clone();
                                        // `/`: the settings page's own search box. It lives
                                        // inside gpui-kit's component, out of reach, but it
                                        // is the first tab stop after this container.
                                        move |_: &crate::app::FocusSettingsSearch, window, cx| {
                                            focus.focus(window, cx);
                                            window.focus_next(cx);
                                        }
                                    })
                                    .child(settings),
                            ),
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
        if self.tab == crate::app::Tab::Config && !self.config_tab_enabled() {
            self.set_tab(crate::app::Tab::Log, cx);
        }
        self.apply_appearance(cx);
        self.apply_recent_status_mode(cx);
        self.rescan_projects(cx);
        let mode = crate::app::kit_mode(self.settings.diff.mode);
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
                let old_scan_dirs = std::mem::replace(&mut self.settings, fresh)
                    .projects
                    .scan_dirs;
                self.forget_recents_of_removed_scan_dirs(&old_scan_dirs);
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
                window.push_notice("Settings reloaded from config.toml", cx);
                cx.notify();
                true
            }
            Ok(_) => false,
            Err(error) => {
                window.push_notice(
                    Notice::error(error.to_string()).title("config.toml has an error"),
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
        let old = self.settings.projects.scan_dirs.clone();
        self.settings.projects.scan_dirs.retain(|d| d != &dir);
        self.forget_recents_of_removed_scan_dirs(&old);
        self.settings_changed(cx);
    }

    /// Drop the recents inside scan directories that `old` had and the settings no longer
    /// do, so removing a scan directory takes all of its repositories out of the sidebar.
    /// Open projects and repositories another scan directory still provides are kept.
    fn forget_recents_of_removed_scan_dirs(&mut self, old: &[PathBuf]) {
        use osier_config::scan::provider;
        let dirs = &self.settings.projects.scan_dirs;
        let removed: Vec<PathBuf> = old.iter().filter(|d| !dirs.contains(d)).cloned().collect();
        if removed.is_empty() {
            return;
        }
        let before = self.state.recent.len();
        let projects = &self.projects;
        self.state.recent.retain(|r| {
            projects.iter().any(|p| &p.path == r)
                || provider(&removed, r).is_none()
                || provider(dirs, r).is_some()
        });
        if self.state.recent.len() != before {
            self.save_state();
        }
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
                window.push_notice("Recents cleared", cx);
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
        RuntimeInfo::collect(
            Some(&self.paths),
            self.git_version.clone(),
            self.updates.route.as_ref().map(|r| r.label()),
        )
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
    const ABOUT: usize = 7;
    /// About > Updates (the status bar's "Update available" badge opens it).
    pub(crate) const UPDATES: Self = Self {
        page: Self::ABOUT,
        group: 0,
    };
    /// Projects > Status of projects that are not open.
    pub(crate) const RECENT_STATUS: Self = Self {
        page: Self::PROJECTS,
        group: 2,
    };
    /// The Accounts page, for tests of its link.
    #[cfg(test)]
    pub(crate) const ACCOUNTS: Self = Self { page: 4, group: 0 };
    /// Projects > Per-project choices (linked from the Accounts page).
    pub(crate) const PER_PROJECT_CHOICES: Self = Self {
        page: Self::PROJECTS,
        group: 4,
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

/// A group that can be flashed by `open_settings_at`. Side padding with a matching negative
/// margin gives the tint room around the card without moving anything.
pub(crate) fn highlightable(
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
    // Sideways only: the page already gives every group its vertical padding, and
    // overriding it would move this group closer to its neighbours. `w_auto` instead of
    // the group's full width, so the negative margins widen it on both sides; with full
    // width they only shift it left and its controls end short of the other groups'.
    let pad = px(10.);
    let group = group
        .px(pad)
        .mx(-pad)
        .w_auto()
        .rounded(cx.theme().radius_lg);
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
    // A larger page title with its description close under it.
    let mut header = StyleRefinement::default();
    header = header.text_lg().gap_1().pt_5().pb_4();
    let pages = vec![
        appearance::appearance_page(view, cx),
        diff::diff_page(view),
        git::git_page(view),
        editors::editors_page(view),
        accounts::accounts_page(view, cx),
        projects::projects_page(view, cx),
        keymap::keymap_page(view),
        about::about_page(view, cx),
    ];
    pages
        .into_iter()
        .map(|page| page.header_style(&header))
        .collect()
}
