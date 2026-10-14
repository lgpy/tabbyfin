//! Settings: playback, image cache, logs and account info.

use gpui_kit::component::button::Button;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme, IndexPath, WindowExt, h_flex, v_flex};
use gpui_kit::*;

use crate::config::{self, Config};
use crate::images::ImageCache;
use crate::session::Session;
use crate::tracks::{Choice, ChoiceSelect};

/// Max streaming bitrate choices in Mbit/s; 0 is no limit.
const BITRATES_MBPS: [i32; 10] = [0, 120, 60, 40, 20, 10, 8, 4, 2, 1];

pub struct SettingsPage {
    session: Session,
    scroll: ScrollHandle,
    bitrate: ChoiceSelect,
    autoplay: bool,
    auto_skip: bool,
    _subscriptions: Vec<Subscription>,
}

impl SettingsPage {
    pub fn new(session: Session, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let config = Config::load();
        let autoplay = config.autoplay_next_episode.unwrap_or(true);
        let auto_skip = config.auto_skip_intro.unwrap_or(false);
        let current = config.max_streaming_bitrate.map_or(0, |bps| (bps / 1_000_000) as i32);
        let choices: Vec<Choice> = BITRATES_MBPS
            .iter()
            .map(|&mbps| Choice {
                label: match mbps {
                    0 => "No limit".into(),
                    mbps => format!("{mbps} Mbit/s").into(),
                },
                value: mbps,
            })
            .collect();
        let selected = BITRATES_MBPS.iter().position(|&m| m == current).unwrap_or(0);
        let bitrate = cx.new(|cx| SelectState::new(choices, Some(IndexPath::default().row(selected)), window, cx));
        let _subscriptions = vec![cx.subscribe(&bitrate, |_, _, event: &SelectEvent<Vec<Choice>>, _| {
            if let SelectEvent::Confirm(Some(mbps)) = event {
                let mut config = Config::load();
                config.max_streaming_bitrate = (*mbps > 0).then(|| *mbps as u64 * 1_000_000);
                if let Err(err) = config.save() {
                    tracing::warn!("failed to save config: {err:#}");
                }
            }
        })];
        Self {
            session,
            scroll: ScrollHandle::new(),
            bitrate,
            autoplay,
            auto_skip,
            _subscriptions,
        }
    }

    fn set_autoplay(&mut self, autoplay: bool, cx: &mut Context<Self>) {
        self.autoplay = autoplay;
        save(|config| config.autoplay_next_episode = Some(autoplay));
        cx.notify();
    }

    fn set_auto_skip(&mut self, auto_skip: bool, cx: &mut Context<Self>) {
        self.auto_skip = auto_skip;
        save(|config| config.auto_skip_intro = Some(auto_skip));
        cx.notify();
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
                Err(err) => {
                    window.push_notification(Notification::error(format!("Couldn't clear the cache: {err}")), cx)
                }
            })
            .ok();
        })
        .detach();
    }
}

fn save(change: impl FnOnce(&mut Config)) {
    let mut config = Config::load();
    change(&mut config);
    if let Err(err) = config.save() {
        tracing::warn!("failed to save config: {err:#}");
    }
}

fn section(title: &'static str, cx: &App) -> Div {
    v_flex().gap_3().child(
        div()
            .text_lg()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(cx.theme().foreground)
            .child(title),
    )
}

fn hint(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
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
                                Switch::new("autoplay")
                                    .checked(self.autoplay)
                                    .label("Play the next episode automatically")
                                    .on_click(
                                        cx.listener(|this, checked: &bool, _, cx| this.set_autoplay(*checked, cx)),
                                    ),
                            )
                            .child(
                                Switch::new("auto-skip")
                                    .checked(self.auto_skip)
                                    .label("Skip intros and recaps automatically")
                                    .on_click(
                                        cx.listener(|this, checked: &bool, _, cx| this.set_auto_skip(*checked, cx)),
                                    ),
                            )
                            .child(hint(
                                "Audio and subtitle languages follow your Jellyfin profile (Settings → Playback in \
                                 the web client), as does whether subtitles show by default.",
                                cx,
                            ))
                            .child(
                                h_flex()
                                    .gap_4()
                                    .child(div().text_sm().child("Max streaming bitrate"))
                                    .child(Select::new(&self.bitrate).w(px(180.))),
                            )
                            .child(hint(
                                "Videos above this bitrate are transcoded by the server, which uses its CPU or GPU. \
                                 Lower it for slow connections; other videos play the original file.",
                                cx,
                            )),
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
                                        .on_click(
                                            cx.listener(|this, _, window, cx| this.clear_image_cache(window, cx)),
                                        ),
                                ),
                            ),
                    )
                    .child(
                        section("Logs", cx)
                            .child(hint(
                                format!(
                                    "A log file is written each day to {}. Attach it to bug reports.",
                                    config::log_dir().display()
                                ),
                                cx,
                            ))
                            .child(
                                h_flex().child(
                                    Button::new("open-logs")
                                        .outline()
                                        .label("Open log folder")
                                        .on_click(|_, _, cx| cx.open_with_system(&config::log_dir())),
                                ),
                            ),
                    )
                    .child(section("Account", cx).child(hint(account, cx)))
                    .child(section("About", cx).child(hint(format!("jelly-gpui {}", env!("CARGO_PKG_VERSION")), cx))),
            )
    }
}
