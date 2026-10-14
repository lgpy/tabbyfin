mod app_view;
mod components;
mod config;
mod credentials;
mod images;
mod router;
mod runtime;
mod session;
mod views;

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;
use tracing_subscriber::EnvFilter;

use crate::app_view::AppView;
use crate::views::shell::{self, Back};

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
            Theme::change(ThemeMode::Dark, None, cx);
            cx.bind_keys([
                KeyBinding::new("escape", Back, Some(shell::KEY_CONTEXT)),
                KeyBinding::new("backspace", Back, Some(shell::KEY_CONTEXT)),
                KeyBinding::new("alt-left", Back, Some(shell::KEY_CONTEXT)),
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
