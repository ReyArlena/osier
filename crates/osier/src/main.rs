//! Osier: fast native git GUI.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod actions;
mod app;
mod askpass;
mod background;
mod blame_view;
mod changes_ops;
mod cli;
mod command_log;
mod config_watch;
mod dialog_buttons;
mod dms_cheatsheet;
mod edit_pane;
mod external;
mod fade;
mod format;
mod github;
mod graph;
mod hunk_ops;
mod ignore_ops;
mod inline_code;
mod instance;
mod keymap;
mod new_repo;
mod ops;
mod palette;
mod patch_ops;
mod prefix;
mod project;
mod rebase_dialog;
mod recent_status;
mod remote_ops;
mod repo_watch;
mod row_actions;
mod row_menu;
mod runtime_info;
mod settings_panel;
mod sidebar_query;
mod squash_ui;
mod switch_project;
mod system_scheme;
mod test_ids;
mod theme_cards;
mod themes;
mod viewer;
mod views;

use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::component::TitleBar;
use gpui_kit::*;

use crate::command_log::UiCommandSink;

const APP_ID: &str = "com.reyarlena.osier";

fn init_logging() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_env("OSIER_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}

fn main() {
    let mode = cli::parse(std::env::args().skip(1));
    let open_arg: Option<PathBuf> = match mode {
        cli::Mode::Gui { path, new_instance } => {
            // Absolute before handing off: the running instance has another working dir.
            let path = path.map(|p| std::fs::canonicalize(&p).unwrap_or(p));
            if !new_instance
                && let Some(socket) = osier_config::Paths::discover()
                    .ok()
                    .and_then(|p| instance::socket_path(p.state_dir()))
                && instance::hand_off(&socket, path.as_deref())
            {
                match &path {
                    Some(p) => println!("Opened {} in the running Osier.", p.display()),
                    None => println!("Osier is already running; brought it to the front."),
                }
                return;
            }
            path
        }
        cli::Mode::CredentialHelper {
            account,
            host,
            operation,
        } => {
            std::process::exit(cli::run_credential_helper(&account, &host, &operation));
        }
        cli::Mode::GhApi {
            account,
            host,
            args,
        } => {
            std::process::exit(cli::run_gh_api(&account, &host, &args));
        }
        cli::Mode::Askpass(prompt) => {
            init_logging();
            let code = askpass::run(prompt);
            std::process::exit(if code == std::process::ExitCode::SUCCESS {
                0
            } else {
                1
            });
        }
        cli::Mode::Version { verbose } => {
            println!("osier {}", env!("CARGO_PKG_VERSION"));
            if verbose {
                let paths = osier_config::Paths::discover().ok();
                let runner = osier_git::GitRunner::new(Arc::new(osier_git::NoopSink));
                let git = osier_git::GitVersion::detect(&runner)
                    .ok()
                    .map(|v| v.to_string());
                print!(
                    "{}",
                    runtime_info::RuntimeInfo::collect(paths.as_ref(), git)
                );
            }
            return;
        }
        cli::Mode::Help => {
            println!("{}", cli::HELP);
            return;
        }
    };
    init_logging();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "starting osier");

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
    // Become the instance others hand their paths to (unless one is already listening).
    if let Some(socket) = instance::socket_path(paths.state_dir())
        && let Err(error) = instance::serve(&socket)
    {
        tracing::warn!(%error, "cannot listen for other osier invocations");
    }
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

    // Remote images in markdown and later the GitHub API need a real http client; gpui
    // starts with one that fails every request.
    let http = match reqwest_client::ReqwestClient::user_agent(&format!(
        "osier/{}",
        env!("CARGO_PKG_VERSION")
    )) {
        Ok(client) => Some(Arc::new(client)),
        Err(error) => {
            tracing::warn!(%error, "http client unavailable, remote images disabled");
            None
        }
    };
    let mut application = gpui_kit::application().with_assets(gpui_kit::assets::AllAssets);
    if let Some(http) = http {
        application = application.with_http_client(http);
    }
    application.run(move |cx| {
        gpui_kit::init(cx);
        themes::ThemeStore::ensure(paths.themes_dir(), true, cx);
        app::init(cx);

        let bounds = match state.window {
            Some(w) if w.width >= 600. && w.height >= 400. => WindowBounds::Windowed(Bounds::new(
                point(px(w.x), px(w.y)),
                size(px(w.width), px(w.height)),
            )),
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
