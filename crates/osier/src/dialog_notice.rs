//! A notice inside a dialog: an icon and text on a soft tint of the kind's colour, with a
//! matching border. For something the user should read before going on (an error that stops
//! the dialog, a warning, a pointer elsewhere, a success). Plain status lines stay muted text.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::test_ids::TestId as _;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(dead_code)] // Success: part of the component, not used yet.
pub(crate) enum NoticeKind {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(IntoElement)]
pub(crate) struct DialogNotice {
    kind: NoticeKind,
    text: SharedString,
    test_id: Option<&'static str>,
}

impl DialogNotice {
    pub(crate) fn new(kind: NoticeKind, text: impl Into<SharedString>) -> Self {
        Self {
            kind,
            text: text.into(),
            test_id: None,
        }
    }

    pub(crate) fn info(text: impl Into<SharedString>) -> Self {
        Self::new(NoticeKind::Info, text)
    }

    #[allow(dead_code)] // Part of the component; not used in a dialog yet.
    pub(crate) fn success(text: impl Into<SharedString>) -> Self {
        Self::new(NoticeKind::Success, text)
    }

    pub(crate) fn warning(text: impl Into<SharedString>) -> Self {
        Self::new(NoticeKind::Warning, text)
    }

    pub(crate) fn error(text: impl Into<SharedString>) -> Self {
        Self::new(NoticeKind::Error, text)
    }

    pub(crate) fn test_id(mut self, id: &'static str) -> Self {
        self.test_id = Some(id);
        self
    }
}

impl RenderOnce for DialogNotice {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let (color, icon) = match self.kind {
            NoticeKind::Info => (theme.info, Lucide::Info),
            NoticeKind::Success => (theme.success, Lucide::CircleCheck),
            NoticeKind::Warning => (theme.warning, Lucide::TriangleAlert),
            NoticeKind::Error => (theme.danger, Lucide::CircleAlert),
        };
        h_flex()
            .when_some(self.test_id, |this, id| this.test_id(id))
            .w_full()
            .gap_3()
            .items_start()
            .p_3()
            .rounded(theme.radius)
            .border_1()
            .border_color(color.opacity(0.4))
            .bg(color.opacity(0.08))
            .child(
                // Lines up with the first line of text.
                div()
                    .pt(px(2.))
                    .child(Icon::new(icon).small().text_color(color)),
            )
            .child(div().flex_1().min_w_0().text_sm().child(self.text))
    }
}
