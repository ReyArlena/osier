use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::separator::Separator;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, StyledExt as _, TitleBar, h_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app::Osier;
use crate::format::tilde;

impl Osier {
    pub(crate) fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let project = self.active_project();
        let branch = project
            .and_then(|p| p.snapshot.as_ref())
            .and_then(|s| s.status.branch.head.clone())
            .unwrap_or_else(|| "detached".into());
        let has_project = project.is_some();
        let later = "Available in a later phase";

        TitleBar::new().child(
            h_flex()
                .size_full()
                .px_2()
                .pr_3()
                .gap_2()
                .child(Icon::new(Lucide::FolderGit2))
                .when_some(project, |this, p| {
                    this.child(div().text_sm().font_medium().child(p.name.clone()))
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(tilde(&p.path)),
                        )
                        .child(Separator::vertical().h_4().mx_1())
                        .child(
                            Button::new("branch")
                                .ghost()
                                .small()
                                .icon(Lucide::GitBranch)
                                .label(branch)
                                .tooltip("Branches")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.set_tab(crate::app::Tab::Branches, cx)
                                })),
                        )
                })
                .when(!has_project, |this| {
                    this.child(div().text_sm().font_medium().child("Osier"))
                })
                .child(div().flex_1())
                .child(
                    Button::new("refresh")
                        .ghost()
                        .small()
                        .icon(Lucide::RefreshCw)
                        .label("Refresh")
                        .tooltip("Refresh (ctrl-r)")
                        .disabled(!has_project)
                        .on_click(cx.listener(|this, _, _, cx| this.refresh_active(cx))),
                )
                .child(Separator::vertical().h_4().mx_1())
                .child(
                    Button::new("fetch")
                        .ghost()
                        .small()
                        .icon(Lucide::CloudDownload)
                        .label("Fetch")
                        .disabled(true)
                        .tooltip(later),
                )
                .child(
                    Button::new("pull")
                        .ghost()
                        .small()
                        .icon(Lucide::ArrowDown)
                        .label("Pull")
                        .disabled(true)
                        .tooltip(later),
                )
                .child(
                    Button::new("push")
                        .ghost()
                        .small()
                        .icon(Lucide::ArrowUp)
                        .label("Push")
                        .disabled(true)
                        .tooltip(later),
                )
                .child(Separator::vertical().h_4().mx_1())
                .child(
                    Button::new("stash")
                        .ghost()
                        .small()
                        .icon(Lucide::Archive)
                        .disabled(true)
                        .tooltip(later),
                )
                .child(
                    Button::new("patch")
                        .ghost()
                        .small()
                        .icon(Lucide::ClipboardCopy)
                        .disabled(true)
                        .tooltip(later),
                )
                .child(
                    Button::new("command-log")
                        .ghost()
                        .small()
                        .icon(Lucide::SquareTerminal)
                        .toggled(self.command_log_visible)
                        .tooltip("Git command log (ctrl-`)")
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_command_log(cx))),
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
                        .tooltip("Toggle light/dark (ctrl-shift-t)")
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_theme(cx))),
                )
                .child(
                    Button::new("settings")
                        .ghost()
                        .small()
                        .icon(Lucide::Settings)
                        .disabled(true)
                        .tooltip(later),
                ),
        )
    }
}
