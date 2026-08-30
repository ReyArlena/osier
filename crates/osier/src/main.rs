//! Osier: fast native git GUI.

mod actions;
mod app;
mod changes_ops;
mod command_log;
mod format;
mod graph;
mod hunk_ops;
mod ops;
mod project;
mod views;

use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::component::{ActiveTheme as _, Theme, ThemeRegistry, TitleBar};
use gpui_kit::*;

use crate::command_log::UiCommandSink;

const APP_ID: &str = "com.reyarlena.osier";
const BUNDLED_THEMES: &[&str] = &[include_str!("../../../themes/catppuccin.json")];

fn init_logging() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_env("OSIER_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}

/// Load bundled and user themes into the registry.
fn init_themes(paths: &osier_config::Paths, cx: &mut App) {
    let registry = ThemeRegistry::global_mut(cx);
    for source in BUNDLED_THEMES {
        if let Err(error) = registry.load_themes_from_str(source) {
            tracing::warn!(%error, "failed to load bundled theme");
        }
    }
    let user_dir = paths.themes_dir();
    if user_dir.is_dir()
        && let Err(error) = ThemeRegistry::watch_dir(user_dir.clone(), cx, |cx| {
            // Re-apply the active theme so edits show up live.
            let name = cx.theme().theme_name().clone();
            app::apply_theme_by_name(&name, cx);
        })
    {
        tracing::warn!(%error, dir = %user_dir.display(), "failed to watch user themes");
    }
}

/// Apply a theme by name, keeping the font resolution gpui-kit did at startup.
pub(crate) fn apply_theme(name: &str, cx: &mut App) -> bool {
    let Some(config) = ThemeRegistry::global(cx).themes().get(name).cloned() else {
        tracing::warn!(name, "theme not found in registry");
        return false;
    };
    Theme::update(cx, |theme| theme.apply_config(&config));
    true
}

fn main() {
    init_logging();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "starting osier");

    let open_arg: Option<PathBuf> = std::env::args_os().nth(1).map(PathBuf::from);

    let paths = match osier_config::Paths::discover() {
        Ok(p) => p,
        Err(error) => {
            tracing::error!(%error, "cannot resolve config directories");
            std::process::exit(2);
        }
    };
    if let Err(error) = osier_config::Settings::ensure_exists(&paths) {
        tracing::warn!(%error, "could not write default config");
    }
    let settings = osier_config::Settings::load(&paths).unwrap_or_else(|error| {
        tracing::error!(%error, "config.toml is invalid, using defaults");
        osier_config::Settings::default()
    });
    let state = osier_config::State::load(&paths).unwrap_or_else(|error| {
        tracing::error!(%error, "state.toml is invalid, starting fresh");
        osier_config::State::default()
    });

    let sink = Arc::new(UiCommandSink::default());
    let runner = osier_git::GitRunner::new(sink.clone());
    // Keep going on failure: the UI shows the error and the user can fix PATH.
    let startup_error = osier_git::GitVersion::check(&runner)
        .map_err(|error| {
            tracing::error!(%error, "git check failed");
            error.to_string()
        })
        .err();

    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            init_themes(&paths, cx);
            app::init(cx);

            let bounds = match state.window {
                Some(w) if w.width >= 600. && w.height >= 400. => WindowBounds::Windowed(
                    Bounds::new(point(px(w.x), px(w.y)), size(px(w.width), px(w.height))),
                ),
                _ => WindowBounds::centered(size(px(1400.), px(860.)), cx),
            };
            let options = WindowOptions {
                window_bounds: Some(bounds),
                window_min_size: Some(size(px(1000.), px(600.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Osier".into()),
                    ..Default::default()
                }),
                app_id: Some(APP_ID.into()),
                ..TitleBar::window_options()
            };
            let opened = gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| {
                    app::Osier::new(
                        paths,
                        settings,
                        state,
                        runner,
                        sink,
                        open_arg,
                        startup_error,
                        window,
                        cx,
                    )
                })
            });
            if let Err(error) = opened {
                tracing::error!(%error, "failed to open main window");
                cx.quit();
                return;
            }
            cx.activate(true);
        });
}
