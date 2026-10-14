// Release builds on Windows run without a console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app_view;
mod changes;
mod components;
mod config;
mod credentials;
mod images;
mod playback;
mod router;
mod runtime;
mod session;
mod tracks;
mod views;

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;
use tracing_subscriber::EnvFilter;

use crate::app_view::AppView;
use crate::changes::Changes;
use crate::views::player::{
    self, ExitPlayer, NextEpisode, SeekBackward, SeekForward, ToggleFullscreen, ToggleMute, TogglePause, VolumeDown,
    VolumeUp,
};
use crate::views::shell::{self, Back, OpenSearch};

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(|cx| {
            gpui_kit::init(cx);
            Theme::change(ThemeMode::Dark, None, cx);
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
                KeyBinding::new("f", ToggleFullscreen, Some(player::KEY_CONTEXT)),
                KeyBinding::new("escape", ExitPlayer, Some(player::KEY_CONTEXT)),
            ]);

            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1280.), px(800.)), cx)),
                titlebar: Some(TitlebarOptions {
                    title: Some("jelly-gpui".into()),
                    ..Default::default()
                }),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| AppView::new(window, cx)))
                .expect("failed to open window");
        });
}
