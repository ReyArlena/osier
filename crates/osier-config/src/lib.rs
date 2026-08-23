//! Settings, state and XDG paths for Osier.
//!
//! - `config.toml` in the config dir: user settings, meant to be synced with dotfiles
//! - `state.toml` in the state dir: open projects, recents, window layout
//! - themes in `<config>/themes/*.json`
//!
//! Both files are optional; missing files mean defaults. Unknown keys are kept on read so a
//! newer Osier does not throw away settings an older one cannot understand yet.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod paths;
pub mod settings;
pub mod state;

pub use paths::Paths;
pub use settings::Settings;
pub use state::State;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("could not determine the home directory")]
    NoHome,
    #[error("{path}: {source}")]
    Io {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path}: {source}")]
    Parse {
        path: std::path::PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("serialising {what}: {source}")]
    Serialise {
        what: &'static str,
        #[source]
        source: toml::ser::Error,
    },
}

pub type Result<T> = std::result::Result<T, ConfigError>;

pub(crate) fn read_toml<T: serde::de::DeserializeOwned + Default>(
    path: &std::path::Path,
) -> Result<T> {
    match std::fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text).map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            source,
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(source) => Err(ConfigError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

pub(crate) fn write_toml<T: serde::Serialize>(
    path: &std::path::Path,
    what: &'static str,
    value: &T,
) -> Result<()> {
    let text =
        toml::to_string_pretty(value).map_err(|source| ConfigError::Serialise { what, source })?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| ConfigError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    // Write next to the target and rename so a crash never leaves a half-written file.
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text).map_err(|source| ConfigError::Io {
        path: tmp.clone(),
        source,
    })?;
    std::fs::rename(&tmp, path).map_err(|source| ConfigError::Io {
        path: path.to_path_buf(),
        source,
    })
}
