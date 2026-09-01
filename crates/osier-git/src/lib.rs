//! Git layer for Osier: command runner, parsers and typed operations.
//!
//! No UI dependencies. Everything here is testable without a window.
//!
//! Design:
//! - every git invocation goes through [`GitRunner`] and produces a [`CommandRecord`]
//! - output is parsed from machine-readable formats only (porcelain v2, `-z`, custom
//!   `--format` with separator bytes) and parsers fail closed
//! - mutating operations live in [`ops`] and encode the safety rules

// Tests may panic on unexpected results; production code may not (workspace lints).
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod blame;
pub mod config;
pub mod diff;
pub mod error;
pub mod graph;
pub mod log;
pub mod refs;
pub mod repo;
pub mod runner;
pub mod show;
pub mod stash;
pub mod status;
pub mod submodule;
pub mod testing;
pub mod version;

pub use error::{GitError, Result};
pub use repo::Repository;
pub use runner::{
    CancelToken, CommandRecord, CommandSink, GitRunner, MemorySink, NoopSink, Progress,
};
pub use version::GitVersion;

pub mod ops;
