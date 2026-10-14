//! Settings: image cache and account info.

use gpui_kit::component::button::Button;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, WindowExt, h_flex, v_flex};
use gpui_kit::*;

use crate::config;
use crate::images::ImageCache;
use crate::session::Session;

pub struct SettingsPage {
    session: Session,
    scroll: ScrollHandle,
}

impl SettingsPage {
    pub fn new(session: Session) -> Self {
        Self {
            session,
            scroll: ScrollHandle::new(),
        }
    }

    fn clear_image_cache(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        ImageCache::global(cx).update(cx, |cache, _| cache.clear_memory());
        let dir = config::cache_dir().join("images");
        cx.spawn_in(window, async move |_, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    match std::fs::remove_dir_all(&dir) {
                        Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err),
                        _ => Ok(()),
                    }
                })
                .await;
            cx.update(|window, cx| match result {
                Ok(()) => window.push_notification(Notification::success("Image cache cleared."), cx),
                Err(err) => window.push_notification(Notification::error(format!("Couldn't clear the cache: {err}")), cx),
            })
            .ok();
        })
        .detach();
    }
}

fn section(title: &'static str, cx: &App) -> Div {
    v_flex()
        .gap_3()
        .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).text_color(cx.theme().foreground).child(title))
}

fn hint(text: impl Into<SharedString>, cx: &App) -> Div {
    div().text_sm().text_color(cx.theme().muted_foreground).child(text.into())
}

impl Render for SettingsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let server = &self.session.server;
        let account = format!(
            "Signed in as {} on {} (Jellyfin {}) at {}",
            self.session.user.name,
            server.server_name.as_deref().unwrap_or("Jellyfin"),
            server.version.as_deref().unwrap_or("?"),
            self.session.client.base_url(),
        );

        div()
            .id("settings")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .child(
                v_flex()
                    .p_6()
                    .gap_8()
                    .max_w(px(720.))
                    .child(
                        section("Storage", cx)
                            .child(hint(
                                format!("Images are cached in {}.", config::cache_dir().join("images").display()),
                                cx,
                            ))
                            .child(
                                h_flex().child(
                                    Button::new("clear-cache")
                                        .outline()
                                        .label("Clear image cache")
                                        .on_click(cx.listener(|this, _, window, cx| this.clear_image_cache(window, cx))),
                                ),
                            ),
                    )
                    .child(section("Account", cx).child(hint(account, cx)))
                    .child(section("About", cx).child(hint(format!("jelly-gpui {}", env!("CARGO_PKG_VERSION")), cx))),
            )
    }
}
