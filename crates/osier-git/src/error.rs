//! Typed errors for the git layer.

use std::path::PathBuf;

use crate::runner::CommandRecord;

pub type Result<T> = std::result::Result<T, GitError>;

/// Error produced by the git layer. Every variant that comes from a git process carries the
/// [`CommandRecord`] so the UI can show the exact command and stderr.
#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("git executable not found: {0}")]
    GitNotFound(String),

    #[error("git {found} is too old, Osier needs {required} or newer")]
    GitTooOld { found: String, required: String },

    #[error("not a git repository: {0}")]
    NotARepo(PathBuf),

    #[error("repository is locked (index.lock present)")]
    Locked(Box<CommandRecord>),

    #[error("authentication required")]
    AuthRequired(Box<CommandRecord>),

    #[error("non fast-forward update rejected")]
    NonFastForward(Box<CommandRecord>),

    #[error("merge conflict")]
    Conflict(Box<CommandRecord>),

    #[error("working tree has local changes that would be overwritten")]
    Dirty(Box<CommandRecord>),

    #[error("branch is not fully merged")]
    BranchNotMerged(Box<CommandRecord>),

    #[error("invalid reference name: {0}")]
    InvalidRefName(String),

    #[error("remote unreachable")]
    Network(Box<CommandRecord>),

    #[error("cancelled")]
    Cancelled(Box<CommandRecord>),

    #[error("git exited with status {status}")]
    Failed {
        status: i32,
        record: Box<CommandRecord>,
    },

    #[error("git timed out after {seconds}s")]
    Timeout {
        seconds: u64,
        record: Box<CommandRecord>,
    },

    #[error("could not parse git output ({context}): {detail}")]
    Parse {
        context: &'static str,
        detail: String,
    },

    #[error("invalid utf-8 in git output ({0})")]
    Utf8(&'static str),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl GitError {
    pub(crate) fn failed(status: i32, record: CommandRecord) -> Self {
        Self::Failed {
            status,
            record: Box::new(record),
        }
    }

    pub(crate) fn timeout(seconds: u64, record: CommandRecord) -> Self {
        Self::Timeout {
            seconds,
            record: Box::new(record),
        }
    }

    pub(crate) fn parse(context: &'static str, detail: impl Into<String>) -> Self {
        Self::Parse {
            context,
            detail: detail.into(),
        }
    }

    /// The command record, when this error came from running git.
    pub fn record(&self) -> Option<&CommandRecord> {
        match self {
            Self::Locked(r)
            | Self::AuthRequired(r)
            | Self::NonFastForward(r)
            | Self::Conflict(r)
            | Self::Dirty(r)
            | Self::BranchNotMerged(r)
            | Self::Network(r)
            | Self::Cancelled(r) => Some(r),
            Self::Failed { record, .. } | Self::Timeout { record, .. } => Some(record),
            _ => None,
        }
    }
}
