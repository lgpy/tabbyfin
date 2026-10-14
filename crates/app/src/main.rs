use gpui_kit::*;
use tracing_subscriber::EnvFilter;

struct JellyApp;

impl Render for JellyApp {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child("jelly-gpui")
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);

        let options = WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some("jelly-gpui".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |_, cx| cx.new(|_| JellyApp))
            .expect("failed to open window");

        tracing::info!("window opened");
    });
}
