//! Settings: mpv location and config, image cache, and account info.

use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme, WindowExt, h_flex, v_flex};
use gpui_kit::*;

use crate::config::{self, Config};
use crate::images::ImageCache;
use crate::session::Session;

pub struct SettingsPage {
    session: Session,
    mpv_path: Entity<InputState>,
    use_mpv_config: bool,
    external_window: bool,
    scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl SettingsPage {
    pub fn new(session: Session, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = Config::load().player;
        let detected = player::locate_mpv(None);
        let mpv_path = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(format!("Automatic ({})", detected.display()))
                .default_value(settings.mpv_path.unwrap_or_default())
        });
        // Save the path when the user is done editing it.
        let _subscriptions = vec![cx.subscribe_in(&mpv_path, window, |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                this.save(window, cx);
            }
        })];
        Self {
            session,
            mpv_path,
            use_mpv_config: settings.use_mpv_config,
            external_window: settings.external_window,
            scroll: ScrollHandle::new(),
            _subscriptions,
        }
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut config = Config::load();
        let path = self.mpv_path.read(cx).value().trim().to_string();
        let changed = config.player.mpv_path.as_deref().unwrap_or("") != path
            || config.player.use_mpv_config != self.use_mpv_config
            || config.player.external_window != self.external_window;
        if !changed {
            return;
        }
        config.player.mpv_path = (!path.is_empty()).then_some(path);
        config.player.use_mpv_config = self.use_mpv_config;
        config.player.external_window = self.external_window;
        match config.save() {
            Ok(()) => window.push_notification(Notification::success("Settings saved. They apply to the next playback."), cx),
            Err(err) => window.push_notification(Notification::error(format!("Couldn't save settings: {err:#}")), cx),
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
                        section("Playback", cx)
                            .child(
                                Switch::new("external-window")
                                    .checked(self.external_window)
                                    .label("Play in a separate mpv window instead of inside the app")
                                    .on_click(cx.listener(|this, checked: &bool, window, cx| {
                                        this.external_window = *checked;
                                        this.save(window, cx);
                                        cx.notify();
                                    })),
                            )
                            .child(hint(
                                "mpv executable for the separate window (also used when libmpv isn't available). Leave empty to find it automatically.",
                                cx,
                            ))
                            .child(Input::new(&self.mpv_path))
                            .child(
                                Switch::new("use-mpv-config")
                                    .checked(self.use_mpv_config)
                                    .label("Use my mpv configuration (mpv.conf, input.conf and scripts)")
                                    .on_click(cx.listener(|this, checked: &bool, window, cx| {
                                        this.use_mpv_config = *checked;
                                        this.save(window, cx);
                                        cx.notify();
                                    })),
                            ),
                    )
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
