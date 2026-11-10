//! Git page: log, pull, git config, prompts and the viewer.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::Icon;
use gpui_kit::component::setting::{
    NumberFieldOptions, SettingField, SettingGroup, SettingItem, SettingPage,
};
use gpui_kit::*;
use osier_config::Settings;

use crate::app::Osier;

use super::{get, set};

pub(super) fn git_page(view: &Entity<Osier>) -> SettingPage {
    let defaults = Settings::default();
    SettingPage::new("Git")
        .description("The log, pulling, the Git config tab, prompts and the file viewer.")
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
                        ("off".into(), "Off".into()),
                    ],
                    get(view, |o| {
                        match o.settings.git.log_unpushed {
                            osier_config::settings::UnpushedStyle::Dim => "dim",
                            osier_config::settings::UnpushedStyle::Colour => "colour",
                            osier_config::settings::UnpushedStyle::Off => "off",
                        }
                        .into()
                    }),
                    set(view, |o, v: SharedString, _| {
                        o.settings.git.log_unpushed = match v.as_ref() {
                            "colour" => osier_config::settings::UnpushedStyle::Colour,
                            "off" => osier_config::settings::UnpushedStyle::Off,
                            _ => osier_config::settings::UnpushedStyle::Dim,
                        }
                    }),
                )
                .default_value("dim".to_string()),
            )
            .description(
                "How the Log shows commits that are on a local branch but no remote yet. Dim \
                 fades their graph line and, a little, the rest of their row; Off draws them \
                 like any other commit.",
            ),
        ]))
        .group(SettingGroup::new().title("Pull").items(vec![
            SettingItem::new(
                "Offer to keep local copies of newly .gitignored files",
                SettingField::switch(
                    get(view, |o| o.settings.git.offer_keep_ignored),
                    set(view, |o, v: bool, _| o.settings.git.offer_keep_ignored = v),
                )
                .default_value(defaults.git.offer_keep_ignored),
            )
            .description(
                "When someone stops tracking a file and adds it to .gitignore, git deletes your \
                 copy of that file the next time you pull. With this on, Osier lists those files \
                 after a pull and offers to put your copies back as they were before the pull. \
                 They stay untracked and ignored, so they do not show up as changes or get \
                 committed again. Only files that an ignore rule now matches are offered: files \
                 that were simply deleted stay deleted, and files still on your disk are left \
                 alone. Pulls run outside Osier are not covered. There, run git show \
                 ORIG_HEAD:<path> > <path> right after the pull to bring a file back.",
            ),
        ]))
        .group(SettingGroup::new().title("Git config").items(vec![
            SettingItem::new(
                "Git config tab",
                SettingField::switch(
                    get(view, |o| o.settings.git.config_tab),
                    set(view, |o, v: bool, _| o.settings.git.config_tab = v),
                )
                .default_value(defaults.git.config_tab),
            )
            .description(
                "Show the Git config panel as a tab next to Patches. It lists the git settings \
                 that change how a repository behaves, with the value git uses and where it \
                 comes from. It is always available from a project's right-click menu in the \
                 sidebar.",
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
