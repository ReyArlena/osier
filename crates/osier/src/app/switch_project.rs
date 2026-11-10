//! "Switch project" in the command palette (ctrl-o): open, pinned and recent projects with the
//! same markers as their sidebar rows (changes icon or dot, ahead/behind, pin, loading).

use std::path::{Path, PathBuf};

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use osier_config::settings::ChangesIndicator;

use crate::app::Osier;
use crate::format::{ahead_behind, tilde};
use crate::sidebar_query::{Query, RowStatus};
use crate::test_ids::TestId as _;

/// The sidebar group a project comes from, used as the palette's group heading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Section {
    Open,
    Pinned,
    Recent,
}

impl Section {
    pub(crate) const ALL: [Section; 3] = [Section::Open, Section::Pinned, Section::Recent];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Section::Open => "Open",
            Section::Pinned => "Pinned",
            Section::Recent => "Recent",
        }
    }
}

/// One row of the "Switch project" list.
#[derive(Clone)]
pub(crate) struct ProjectEntry {
    pub section: Section,
    pub path: PathBuf,
    pub name: String,
    /// The path with `~` for the home directory, shown under the name.
    pub dir: String,
    pub icon: Lucide,
    /// The project shown right now.
    pub active: bool,
    /// Open and pinned: a pin after the name, like the sidebar. Pinned projects that are not
    /// open carry the pin as their icon instead.
    pub pinned: bool,
    pub loading: bool,
    /// Uncommitted changes, drawn as a coloured icon or a dot per `appearance.changes_indicator`.
    pub dirty: bool,
    pub dot: bool,
    pub arrows: Option<String>,
    /// For `is:` qualifiers in the query.
    pub status: Option<RowStatus>,
}

impl ProjectEntry {
    pub(crate) fn matches(&self, query: &Query) -> bool {
        query.matches(&self.path, self.status)
    }

    /// The palette row: icon, name over the shortened path, then the sidebar's badges.
    pub(crate) fn render(&self, cx: &App) -> Div {
        let warning = cx.theme().warning;
        let muted = cx.theme().muted_foreground;
        let by_icon = self.dirty && !self.dot;
        h_flex()
            .test_id(|| format!("palette-project:{}", self.name))
            .w_full()
            .gap_3()
            .items_center()
            .child(
                Icon::new(self.icon)
                    .small()
                    .flex_none()
                    .when(by_icon, |i| i.text_color(warning)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        h_flex()
                            .min_w_0()
                            .gap_1p5()
                            .items_center()
                            .when(self.dot, |this| {
                                this.child(
                                    div().flex_none().size(px(6.)).rounded_full().bg(warning),
                                )
                            })
                            .child(div().min_w_0().truncate().child(self.name.clone())),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .truncate()
                            .child(self.dir.clone()),
                    ),
            )
            .child(
                h_flex()
                    .flex_none()
                    .gap_1()
                    .items_center()
                    .when(self.active, |this| {
                        this.child(div().text_xs().text_color(muted).child("current"))
                    })
                    .when(self.loading, |this| {
                        this.child(Tag::secondary().xsmall().child("…"))
                    })
                    .when_some(self.arrows.clone().filter(|_| !self.loading), |this, a| {
                        this.child(Tag::secondary().xsmall().child(a))
                    })
                    .when(self.pinned && self.section == Section::Open, |this| {
                        this.child(Icon::new(Lucide::Pin).xsmall())
                    }),
            )
    }
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

impl Osier {
    /// Every project the sidebar lists, ignoring its filter, in sidebar order.
    pub(crate) fn switch_project_entries(&self) -> Vec<ProjectEntry> {
        let dot_style = self.settings.appearance.changes_indicator == ChangesIndicator::Dot;
        let everything = Query::default();
        let mut out = Vec::new();
        for (ix, project) in self.projects.iter().enumerate() {
            let status = Self::open_row_status(project);
            let dirty = status.and_then(|s| s.dirty).unwrap_or(false);
            out.push(ProjectEntry {
                section: Section::Open,
                path: project.path.clone(),
                name: project.name.clone(),
                dir: tilde(&project.path),
                icon: Lucide::FolderGit2,
                active: self.active == Some(ix),
                pinned: self.state.is_pinned(&project.path),
                loading: project.loading,
                dirty,
                dot: dirty && dot_style,
                arrows: status.and_then(|s| ahead_behind(s.ahead, s.behind)),
                status,
            });
        }
        for path in self.pinned_matching(&everything) {
            let icon = if !self.project_missing(&path) {
                Lucide::Pin
            } else {
                Lucide::FolderX
            };
            out.push(self.closed_entry(Section::Pinned, path, icon, dot_style));
        }
        for (path, provider) in self.recent_matching(&everything) {
            let icon = if self.project_missing(&path) {
                Lucide::FolderX
            } else if provider.is_some() {
                Lucide::FolderSearch
            } else {
                Lucide::Folder
            };
            out.push(self.closed_entry(Section::Recent, path, icon, dot_style));
        }
        out
    }

    /// A project that is not open, with its last known status like `status_item` in the
    /// sidebar.
    fn closed_entry(
        &self,
        section: Section,
        path: PathBuf,
        icon: Lucide,
        dot_style: bool,
    ) -> ProjectEntry {
        let badge = self.recent_badge(&path);
        let dirty =
            self.recent_status.shows_changes() && badge.and_then(|b| b.dirty).unwrap_or(false);
        ProjectEntry {
            section,
            name: name_of(&path),
            dir: tilde(&path),
            icon,
            active: false,
            pinned: section == Section::Pinned,
            loading: false,
            dirty,
            dot: dirty && dot_style,
            arrows: badge.and_then(|b| ahead_behind(b.ahead, b.behind)),
            status: self.other_row_status(&path),
            path,
        }
    }

    /// Show the project at `path`: activate it when open, open it otherwise.
    pub(crate) fn switch_to_project(
        &mut self,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.projects.iter().position(|p| p.path == path) {
            Some(ix) => self.activate(ix, window, cx),
            None => self.open_project_notify(path, window, cx),
        }
    }
}
