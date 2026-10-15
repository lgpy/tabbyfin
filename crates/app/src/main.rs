// Release builds on Windows run without a console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app_view;
mod browse;
mod changes;
mod components;
mod config;
mod credentials;
mod images;
mod live;
mod markers;
mod playback;
mod remote;
mod router;
mod runtime;
mod session;
mod syncplay;
mod theme;
mod tracks;
mod trickplay;
mod views;
mod window;

use gpui_kit::*;
use tracing_subscriber::EnvFilter;

use crate::app_view::AppView;
use crate::changes::Changes;
use crate::config::{Config, ThemePreference};
use crate::views::player::{
    self, ExitPlayer, NextChapter, NextEpisode, PrevChapter, PreviousEpisode, SeekBackward, SeekForward, Skip,
    SpeedDown, SpeedUp, SubtitleDelayDown, SubtitleDelayUp, ToggleFullscreen, ToggleMute, TogglePause, VolumeDown,
    VolumeUp,
};
use crate::views::shell::{self, Back, OpenSearch};

/// Daily log files kept in [`config::log_dir`].
const LOG_FILES_KEPT: usize = 7;

/// Logs to stderr and to a daily file (release builds on Windows have no console, so the file
/// is what bug reports can attach). `RUST_LOG` sets the filter; the default is `info`. Keep the
/// returned guard alive: dropping it flushes and stops the file writer.
fn init_logging() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    use tracing_subscriber::prelude::*;

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    // The appender prunes old files on start and complains if the folder doesn't exist yet.
    let _ = std::fs::create_dir_all(config::log_dir());
    let file = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("jelly-gpui")
        .filename_suffix("log")
        .max_log_files(LOG_FILES_KEPT)
        .build(config::log_dir());
    let (file_layer, guard, file_error) = match file {
        Ok(appender) => {
            let (writer, guard) = tracing_appender::non_blocking(appender);
            (
                Some(
                    tracing_subscriber::fmt::layer()
                        .with_ansi(false)
                        .fmt_fields(PlainFields)
                        .with_writer(writer),
                ),
                Some(guard),
                None,
            )
        }
        Err(err) => (None, None, Some(err)),
    };
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .with(file_layer)
        .init();
    if let Some(err) = file_error {
        tracing::warn!("not logging to a file in {}: {err}", config::log_dir().display());
    }
    guard
}

/// The default field format, as a type of its own. Layers cache formatted span fields by
/// formatter type, so with a shared type the file would get the terminal layer's colour codes.
struct PlainFields;

impl<'writer> tracing_subscriber::fmt::FormatFields<'writer> for PlainFields {
    fn format_fields<R: tracing_subscriber::field::RecordFields>(
        &self,
        writer: tracing_subscriber::fmt::format::Writer<'writer>,
        fields: R,
    ) -> std::fmt::Result {
        tracing_subscriber::fmt::format::DefaultFields::new().format_fields(writer, fields)
    }
}

/// Logging in the player host process: plain lines on stderr, which the app reads and logs
/// (with its own timestamps) under the `player` target.
fn init_host_logging() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .without_time()
        .with_writer(std::io::stderr)
        .init();
}

fn main() {
    // This executable is also the player host process (`::player::host`).
    if ::player::is_host_process() {
        init_host_logging();
        std::process::exit(::player::host::run());
    }
    let _log_guard = init_logging();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "starting jelly-gpui");

    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(|cx| {
            gpui_kit::init(cx);
            let config = Config::load();
            theme::apply(config.theme, None, cx);
            Changes::init(cx);
            let search_key = if cfg!(target_os = "macos") { "cmd-f" } else { "ctrl-f" };
            cx.bind_keys([
                KeyBinding::new(search_key, OpenSearch, Some(shell::KEY_CONTEXT)),
                KeyBinding::new("escape", Back, Some(shell::KEY_CONTEXT)),
                KeyBinding::new("backspace", Back, Some(shell::KEY_CONTEXT)),
                KeyBinding::new("alt-left", Back, Some(shell::KEY_CONTEXT)),
                KeyBinding::new("space", TogglePause, Some(player::KEY_CONTEXT)),
                KeyBinding::new("right", SeekForward, Some(player::KEY_CONTEXT)),
                KeyBinding::new("left", SeekBackward, Some(player::KEY_CONTEXT)),
                KeyBinding::new("up", VolumeUp, Some(player::KEY_CONTEXT)),
                KeyBinding::new("down", VolumeDown, Some(player::KEY_CONTEXT)),
                KeyBinding::new("m", ToggleMute, Some(player::KEY_CONTEXT)),
                KeyBinding::new("shift-n", NextEpisode, Some(player::KEY_CONTEXT)),
                KeyBinding::new("shift-p", PreviousEpisode, Some(player::KEY_CONTEXT)),
                KeyBinding::new("pagedown", NextChapter, Some(player::KEY_CONTEXT)),
                KeyBinding::new("pageup", PrevChapter, Some(player::KEY_CONTEXT)),
                KeyBinding::new("enter", Skip, Some(player::KEY_CONTEXT)),
                KeyBinding::new("s", Skip, Some(player::KEY_CONTEXT)),
                KeyBinding::new("f", ToggleFullscreen, Some(player::KEY_CONTEXT)),
                KeyBinding::new("escape", ExitPlayer, Some(player::KEY_CONTEXT)),
                KeyBinding::new("]", SpeedUp, Some(player::KEY_CONTEXT)),
                KeyBinding::new("[", SpeedDown, Some(player::KEY_CONTEXT)),
                KeyBinding::new("x", SubtitleDelayUp, Some(player::KEY_CONTEXT)),
                KeyBinding::new("z", SubtitleDelayDown, Some(player::KEY_CONTEXT)),
            ]);
            cx.bind_keys(browse::key_bindings());

            let launch = window::launch_placement(&config, cx);
            let options = WindowOptions {
                window_bounds: Some(launch.window_bounds()),
                display_id: launch.display_id,
                titlebar: Some(TitlebarOptions {
                    title: Some("jelly-gpui".into()),
                    ..Default::default()
                }),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                // Again with the window, whose appearance is the reliable one on Linux.
                if config.theme == ThemePreference::System {
                    theme::apply(ThemePreference::System, Some(window), cx);
                }
                cx.new(|cx| AppView::new(config, launch.placement, window, cx))
            })
            .expect("failed to open window");
        });
}
