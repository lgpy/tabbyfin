//! Settings: appearance, playback, subtitles, image cache, logs and account info.

use gpui_kit::component::button::Button;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme, IndexPath, WindowExt, h_flex, v_flex};
use gpui_kit::*;

use crate::config::{self, Config, SubtitleColor, SubtitleStyle, ThemePreference};
use crate::images::ImageCache;
use crate::playback::PlaybackManager;
use crate::session::Session;
use crate::theme;
use crate::tracks::{Choice, ChoiceSelect};

/// The theme choices, in the order the picker lists them.
const THEMES: [(ThemePreference, &str); 3] = [
    (ThemePreference::System, "Follow the system"),
    (ThemePreference::Light, "Light"),
    (ThemePreference::Dark, "Dark"),
];

const SUBTITLE_COLORS: [(SubtitleColor, &str); 2] =
    [(SubtitleColor::White, "White"), (SubtitleColor::Yellow, "Yellow")];

const SUBTITLE_STYLES: [(SubtitleStyle, &str); 2] =
    [(SubtitleStyle::Outline, "Outline"), (SubtitleStyle::Box, "Dark box")];

/// Subtitle sizes offered here; the player's menu sets any size in 10 % steps.
const SUBTITLE_SCALES: [f64; 6] = [0.75, 1., 1.25, 1.5, 1.75, 2.];

/// Max streaming bitrate choices in Mbit/s; 0 is no limit.
const BITRATES_MBPS: [i32; 10] = [0, 120, 60, 40, 20, 10, 8, 4, 2, 1];

pub struct SettingsPage {
    session: Session,
    scroll: ScrollHandle,
    theme: ChoiceSelect,
    subtitle_color: ChoiceSelect,
    subtitle_style: ChoiceSelect,
    subtitle_size: ChoiceSelect,
    bitrate: ChoiceSelect,
    autoplay: bool,
    auto_skip: bool,
    check_updates: bool,
    _subscriptions: Vec<Subscription>,
}

impl SettingsPage {
    pub fn new(session: Session, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let config = Config::load();
        let autoplay = config.autoplay_next_episode.unwrap_or(true);
        let auto_skip = config.auto_skip_intro.unwrap_or(false);
        let check_updates = config.check_for_updates.unwrap_or(true);
        let mut subscriptions = Vec::new();

        let theme = options_select(&THEMES, &config.theme, window, cx);
        subscriptions.push(on_pick(&theme, &THEMES, window, cx, |preference, window, cx| {
            save(|config| config.theme = preference);
            theme::apply(preference, Some(window), cx);
        }));

        let subtitle_color = options_select(&SUBTITLE_COLORS, &config.subtitle_color.unwrap_or_default(), window, cx);
        subscriptions.push(on_pick(&subtitle_color, &SUBTITLE_COLORS, window, cx, |color, _, _| {
            save(|config| config.subtitle_color = Some(color));
        }));
        let subtitle_style = options_select(&SUBTITLE_STYLES, &config.subtitle_style.unwrap_or_default(), window, cx);
        subscriptions.push(on_pick(&subtitle_style, &SUBTITLE_STYLES, window, cx, |style, _, _| {
            save(|config| config.subtitle_style = Some(style));
        }));
        // A size set in the player that isn't one of the steps here is listed too.
        let current_scale = config.subtitle_scale.unwrap_or(1.);
        let mut scales = SUBTITLE_SCALES.to_vec();
        if !scales.contains(&current_scale) {
            scales.push(current_scale);
            scales.sort_by(f64::total_cmp);
        }
        let scales: Vec<(f64, String)> = scales
            .into_iter()
            .map(|scale| (scale, format!("{:.0} %", scale * 100.)))
            .collect();
        let subtitle_size = options_select(&scales, &current_scale, window, cx);
        subscriptions.push(on_pick(&subtitle_size, &scales, window, cx, |scale, _, cx| {
            save(|config| config.subtitle_scale = Some(scale).filter(|&scale| scale != 1.));
            PlaybackManager::global(cx).update(cx, |playback, _| playback.set_subtitle_scale(scale));
        }));

        let bitrates: Vec<(i32, String)> = BITRATES_MBPS
            .iter()
            .map(|&mbps| match mbps {
                0 => (mbps, "No limit".to_string()),
                mbps => (mbps, format!("{mbps} Mbit/s")),
            })
            .collect();
        let current_bitrate = config.max_streaming_bitrate.map_or(0, |bps| (bps / 1_000_000) as i32);
        let bitrate = options_select(&bitrates, &current_bitrate, window, cx);
        subscriptions.push(on_pick(&bitrate, &bitrates, window, cx, |mbps, _, _| {
            save(|config| config.max_streaming_bitrate = (mbps > 0).then(|| mbps as u64 * 1_000_000));
        }));

        Self {
            session,
            scroll: ScrollHandle::new(),
            theme,
            subtitle_color,
            subtitle_style,
            subtitle_size,
            bitrate,
            autoplay,
            auto_skip,
            check_updates,
            _subscriptions: subscriptions,
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

    fn set_check_updates(&mut self, check: bool, cx: &mut Context<Self>) {
        self.check_updates = check;
        save(|config| config.check_for_updates = Some(check));
        cx.notify();
    }

    fn clear_image_cache(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        ImageCache::global(cx).update(cx, |cache, cx| cache.clear_memory(cx));
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

/// A picker over `options`, starting at `current` (or the first). Its values are indexes
/// into `options`.
fn options_select<T: PartialEq>(
    options: &[(T, impl AsRef<str>)],
    current: &T,
    window: &mut Window,
    cx: &mut App,
) -> ChoiceSelect {
    let choices: Vec<Choice> = options
        .iter()
        .enumerate()
        .map(|(i, (_, label))| Choice {
            label: label.as_ref().to_owned().into(),
            value: i as i32,
        })
        .collect();
    let selected = options.iter().position(|(value, _)| value == current).unwrap_or(0);
    cx.new(|cx| SelectState::new(choices, Some(IndexPath::default().row(selected)), window, cx))
}

/// Calls `picked` with the option chosen in a picker made by [`options_select`].
fn on_pick<T: Copy + 'static>(
    select: &ChoiceSelect,
    options: &[(T, impl AsRef<str>)],
    window: &mut Window,
    cx: &mut Context<SettingsPage>,
    picked: impl Fn(T, &mut Window, &mut App) + 'static,
) -> Subscription {
    let values: Vec<T> = options.iter().map(|(value, _)| *value).collect();
    cx.subscribe_in(
        select,
        window,
        move |_, _, event: &SelectEvent<Vec<Choice>>, window, cx| {
            if let SelectEvent::Confirm(Some(i)) = event
                && let Some(value) = values.get(*i as usize)
            {
                picked(*value, window, cx);
            }
        },
    )
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

/// A picker with its label, in a settings row.
fn labeled(label: &'static str, select: &ChoiceSelect) -> Div {
    h_flex()
        .gap_4()
        .child(div().w(px(170.)).flex_none().text_sm().child(label))
        .child(Select::new(select).w(px(180.)))
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
                    .child(section("Appearance", cx).child(labeled("Theme", &self.theme)))
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
                            .child(labeled("Max streaming bitrate", &self.bitrate))
                            .child(hint(
                                "Videos above this bitrate are transcoded by the server, which uses its CPU or GPU. \
                                 Lower it for slow connections; other videos play the original file.",
                                cx,
                            )),
                    )
                    .child(section("Home", cx).child(hint(
                        "The order of the sections and which libraries show follow your Jellyfin settings \
                         (Settings → Home in the web client).",
                        cx,
                    )))
                    .child(
                        section("Subtitles", cx)
                            .child(labeled("Colour", &self.subtitle_color))
                            .child(labeled("Style", &self.subtitle_style))
                            .child(labeled("Size", &self.subtitle_size))
                            .child(hint(
                                "For text subtitles such as SRT; styled (ASS) subtitles keep their own look and \
                                 only change size. The player's settings menu also changes the size while \
                                 watching.",
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
                    .child(
                        section("About", cx)
                            .child(hint(format!("jelly-gpui {}", env!("CARGO_PKG_VERSION")), cx))
                            .child(
                                Switch::new("check-updates")
                                    .checked(self.check_updates)
                                    .label("Tell me when a new version is out")
                                    .on_click(
                                        cx.listener(|this, checked: &bool, _, cx| this.set_check_updates(*checked, cx)),
                                    ),
                            )
                            .child(hint("Checks the GitHub releases once a day, when the app starts.", cx)),
                    ),
            )
    }
}
