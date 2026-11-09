//! Every toast the app raises, kept for the Notifications panel so one that slid away can be
//! read later. `window.push_notice(..)` shows the toast and records it; the panel reads the
//! record. Session only: nothing is saved.

use std::time::{SystemTime, UNIX_EPOCH};

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{IconName, Sizable as _};
use gpui_kit::*;
use osier_git::GitError;

use crate::app::Osier;
use crate::test_ids::TestId as _;

/// Oldest notices are dropped past this many.
const CAP: usize = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NoticeKind {
    Info,
    Success,
    Warning,
    Error,
}

impl NoticeKind {
    pub(crate) fn icon(self) -> IconName {
        match self {
            Self::Info => IconName::Info,
            Self::Success => IconName::CircleCheck,
            Self::Warning => IconName::TriangleAlert,
            Self::Error => IconName::CircleX,
        }
    }
}

/// One toast as the user saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Notice {
    /// Set when recorded; 0 until then.
    pub id: u64,
    pub kind: NoticeKind,
    pub title: Option<String>,
    pub message: String,
    /// When it was raised, seconds since the epoch. Set when recorded.
    pub at: i64,
    /// Offer "View command log" (error toasts for git failures).
    pub command_log: bool,
    /// The failed command to expand there, when the error carries one.
    pub command: Option<u64>,
}

impl Notice {
    fn new(kind: NoticeKind, message: impl Into<String>) -> Self {
        Self {
            id: 0,
            kind,
            title: None,
            message: message.into(),
            at: 0,
            command_log: false,
            command: None,
        }
    }

    pub(crate) fn info(message: impl Into<String>) -> Self {
        Self::new(NoticeKind::Info, message)
    }

    pub(crate) fn success(message: impl Into<String>) -> Self {
        Self::new(NoticeKind::Success, message)
    }

    pub(crate) fn warning(message: impl Into<String>) -> Self {
        Self::new(NoticeKind::Warning, message)
    }

    pub(crate) fn error(message: impl Into<String>) -> Self {
        Self::new(NoticeKind::Error, message)
    }

    pub(crate) fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Add "View command log", opening the log on the command behind `error` when it has one.
    pub(crate) fn command_log(mut self, error: &GitError) -> Self {
        self.command_log = true;
        self.command = error.record().map(|r| r.id);
        self
    }

    /// The gpui-kit toast for this notice.
    fn toast(&self, owner: Option<WeakEntity<Osier>>) -> Notification {
        let mut toast = match self.kind {
            NoticeKind::Info => Notification::info(self.message.clone()),
            NoticeKind::Success => Notification::success(self.message.clone()),
            NoticeKind::Warning => Notification::warning(self.message.clone()),
            NoticeKind::Error => Notification::error(self.message.clone()),
        };
        if let Some(title) = &self.title {
            toast = toast.title(title.clone());
        }
        if self.command_log {
            // Under the message (the toast's own action slot goes beside it and squeezes the
            // text); like an action it keeps the toast up until dismissed.
            let id = self.command;
            toast = toast.autohide(false).content(move |_, _, _| {
                let owner = owner.clone();
                div()
                    .mt_1()
                    .flex()
                    .justify_end()
                    .child(
                        Button::new("show-command-log")
                            .test_id("show-command-log")
                            .ghost()
                            .xsmall()
                            .label("View command log")
                            .on_click(move |_, _, cx| {
                                if let Some(owner) = &owner {
                                    let _ = owner
                                        .update(cx, |this, cx| this.show_failed_command(id, cx));
                                }
                            }),
                    )
                    .into_any_element()
            });
        }
        toast
    }
}

impl From<&str> for Notice {
    fn from(message: &str) -> Self {
        Self::info(message)
    }
}

impl From<String> for Notice {
    fn from(message: String) -> Self {
        Self::info(message)
    }
}

/// The record, newest last, plus who to tell when it changes.
#[derive(Default)]
pub(crate) struct NoticeLog {
    notices: Vec<Notice>,
    seq: u64,
    owner: Option<WeakEntity<Osier>>,
}

impl Global for NoticeLog {}

impl NoticeLog {
    /// Point the log at the app view: the panel lives there, and error toasts open its
    /// command log.
    pub(crate) fn attach(owner: WeakEntity<Osier>, cx: &mut App) {
        cx.default_global::<Self>().owner = Some(owner);
    }

    pub(crate) fn notices(cx: &App) -> &[Notice] {
        cx.try_global::<Self>()
            .map_or(&[], |log| log.notices.as_slice())
    }

    /// Id of the newest notice, 0 when none was raised yet.
    pub(crate) fn latest(cx: &App) -> u64 {
        cx.try_global::<Self>().map_or(0, |log| log.seq)
    }

    /// How many notices came after `seen` (an id from `latest`).
    pub(crate) fn unread(seen: u64, cx: &App) -> usize {
        Self::notices(cx).iter().filter(|n| n.id > seen).count()
    }

    pub(crate) fn clear(cx: &mut App) {
        cx.default_global::<Self>().notices.clear();
    }

    fn record(&mut self, mut notice: Notice) -> Notice {
        self.seq += 1;
        notice.id = self.seq;
        notice.at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        if self.notices.len() >= CAP {
            let drop = self.notices.len() - CAP + 1;
            self.notices.drain(..drop);
        }
        self.notices.push(notice.clone());
        notice
    }
}

pub(crate) trait WindowNoticeExt {
    /// Show a toast and keep it in the Notifications panel.
    fn push_notice(&mut self, notice: impl Into<Notice>, cx: &mut App);
}

impl WindowNoticeExt for Window {
    fn push_notice(&mut self, notice: impl Into<Notice>, cx: &mut App) {
        let log = cx.default_global::<NoticeLog>();
        let notice = log.record(notice.into());
        let owner = log.owner.clone();
        self.push_notification(notice.toast(owner.clone()), cx);
        // The panel and the title-bar badge live on the app view. Deferred: this may run
        // inside one of its own updates.
        if let Some(owner) = owner {
            cx.defer(move |cx| {
                let _ = owner.update(cx, |_, cx| cx.notify());
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CAP, Notice, NoticeKind, NoticeLog};

    #[test]
    fn plain_text_is_an_info_notice() {
        let n: Notice = "Recents cleared".into();
        assert_eq!(n.kind, NoticeKind::Info);
        assert_eq!(n.message, "Recents cleared");
        assert_eq!(n.title, None);
        assert!(!n.command_log);
    }

    #[test]
    fn the_log_numbers_notices_and_keeps_the_newest() {
        let mut log = NoticeLog::default();
        for i in 0..(CAP + 5) {
            let n = log.record(Notice::warning(format!("n{i}")));
            assert_eq!(n.id, i as u64 + 1);
            assert!(n.at > 0);
        }
        assert_eq!(log.notices.len(), CAP);
        assert_eq!(log.notices[0].message, "n5");
        assert_eq!(log.seq, CAP as u64 + 5);
    }
}
