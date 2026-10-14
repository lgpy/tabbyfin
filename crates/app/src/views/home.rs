//! Placeholder signed-in screen. Replaced by real browsing in milestone 4.

use gpui_kit::component::button::Button;
use gpui_kit::component::{ActiveTheme, v_flex};
use gpui_kit::*;

use crate::session::Session;

pub enum HomeEvent {
    SignOut,
}

pub struct HomeView {
    session: Session,
}

impl EventEmitter<HomeEvent> for HomeView {}

impl HomeView {
    pub fn new(session: Session) -> Self {
        Self { session }
    }
}

impl Render for HomeView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let server = format!(
            "{} · Jellyfin {}",
            self.session.server_name(),
            self.session.server.version.as_deref().unwrap_or("?")
        );
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_3()
            .bg(cx.theme().background)
            .child(
                div()
                    .text_xl()
                    .child(format!("Signed in as {}", self.session.user.name)),
            )
            .child(div().text_sm().text_color(cx.theme().muted_foreground).child(server))
            .child(
                Button::new("sign-out")
                    .outline()
                    .label("Sign out")
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(HomeEvent::SignOut))),
            )
    }
}
