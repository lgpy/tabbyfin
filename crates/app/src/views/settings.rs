//! Settings: gpui-kit's settings panel, a page per topic in a searchable sidebar.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::Button;
use gpui_kit::component::group_box::GroupBoxVariant;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::setting::{SettingField, SettingGroup, SettingItem, SettingPage, Settings};
use gpui_kit::component::{ActiveTheme, IndexPath, WindowExt};
use gpui_kit::prelude::FluentBuilder as _;
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
/// The bitrate choice that measures the connection instead.
const AUTOMATIC: i32 = -1;

/// The pickers' width beside their label; below the label, in a narrow window, they fill it.
const PICKER_W: f32 = 220.;

pub struct SettingsPage {
    session: Session,
    text_size: ChoiceSelect,
    focus_handle: FocusHandle,
    theme: ChoiceSelect,
    subtitle_color: ChoiceSelect,
    subtitle_style: ChoiceSelect,
    subtitle_size: ChoiceSelect,
    bitrate: ChoiceSelect,
    animations: bool,
    autoplay: bool,
    auto_skip: bool,
    prefer_hevc: bool,
    check_updates: bool,
    _subscriptions: Vec<Subscription>,
}

impl SettingsPage {
    pub fn new(session: Session, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let config = Config::load();
        let animations = config.animations.unwrap_or(true);
        let autoplay = config.autoplay_next_episode.unwrap_or(true);
        let auto_skip = config.auto_skip_intro.unwrap_or(false);
        let prefer_hevc = config.prefer_hevc.unwrap_or(false);
        let check_updates = config.check_for_updates.unwrap_or(true);
        let mut subscriptions = Vec::new();

        let theme = options_select(&THEMES, &config.theme, window, cx);
        subscriptions.push(on_pick(&theme, &THEMES, window, cx, |preference, window, cx| {
            save(|config| config.theme = preference);
            theme::apply(preference, Some(window), cx);
        }));

        // `None` follows the system; its size is shown, as it can differ from 100%.
        let system = format!("Follow the system ({:.0}%)", theme::system_text_scale() * 100.);
        let text_sizes: Vec<(Option<u16>, String)> = std::iter::once((None, system))
            .chain(
                theme::TEXT_SIZES
                    .iter()
                    .map(|&percent| (Some(percent), format!("{percent}%"))),
            )
            .collect();
        let text_size = options_select(&text_sizes, &config.text_size, window, cx);
        subscriptions.push(on_pick(&text_size, &text_sizes, window, cx, |size, _, cx| {
            save(|config| config.text_size = size);
            theme::apply_text_size(cx);
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

        // `AUTOMATIC` measures the connection before playing instead.
        let bitrates: Vec<(i32, String)> = std::iter::once((AUTOMATIC, "Automatic (measured)".to_string()))
            .chain(BITRATES_MBPS.iter().map(|&mbps| match mbps {
                0 => (mbps, "No limit".to_string()),
                mbps => (mbps, format!("{mbps} Mbit/s")),
            }))
            .collect();
        let current_bitrate = match config.automatic_bitrate {
            Some(true) => AUTOMATIC,
            _ => config.max_streaming_bitrate.map_or(0, |bps| (bps / 1_000_000) as i32),
        };
        let bitrate = options_select(&bitrates, &current_bitrate, window, cx);
        subscriptions.push(on_pick(&bitrate, &bitrates, window, cx, |mbps, _, _| {
            save(|config| {
                config.automatic_bitrate = Some(mbps == AUTOMATIC);
                if mbps != AUTOMATIC {
                    config.max_streaming_bitrate = (mbps > 0).then(|| mbps as u64 * 1_000_000);
                }
            });
        }));

        Self {
            session,
            text_size,
            focus_handle: cx.focus_handle(),
            theme,
            subtitle_color,
            subtitle_style,
            subtitle_size,
            bitrate,
            animations,
            autoplay,
            auto_skip,
            prefer_hevc,
            check_updates,
            _subscriptions: subscriptions,
        }
    }

    fn set_animations(&mut self, on: bool, cx: &mut Context<Self>) {
        self.animations = on;
        save(|config| config.animations = Some(on));
        crate::motion::set_enabled(on, cx);
        cx.notify();
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

    fn set_prefer_hevc(&mut self, prefer: bool, cx: &mut Context<Self>) {
        self.prefer_hevc = prefer;
        save(|config| config.prefer_hevc = Some(prefer));
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
                    tracing::warn!("clearing the image cache failed: {err}");
                    let message = "Couldn't clear the image cache. Close anything using its folder and try again.";
                    window.push_notification(Notification::error(message), cx)
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

fn open_logs(cx: &mut App) {
    cx.open_with_system(&config::log_dir());
}

/// A picker as a setting's field.
fn picker(select: &ChoiceSelect) -> SettingField<SharedString> {
    let select = select.clone();
    SettingField::render(move |options, _, _| {
        Select::new(&select).map(|el| {
            if options.layout() == Axis::Vertical {
                el.w_full()
            } else {
                el.w(rems(PICKER_W / 16.))
            }
        })
    })
}

/// A switch on one of the page's flags, with the value it has when unset in the config.
fn switch(
    page: &Entity<SettingsPage>,
    get: fn(&SettingsPage) -> bool,
    set: fn(&mut SettingsPage, bool, &mut Context<SettingsPage>),
    default: bool,
) -> SettingField<bool> {
    let (read, write) = (page.clone(), page.clone());
    SettingField::switch(
        move |cx| get(read.read(cx)),
        move |on, cx| write.update(cx, |page, cx| set(page, on, cx)),
    )
    .default_value(default)
}

/// Text in place of a field, for a setting that's only information.
fn info(text: impl Into<SharedString>) -> SettingField<SharedString> {
    let text = text.into();
    SettingField::render(move |_, _, cx| {
        div()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(text.clone())
    })
}

impl SettingsPage {
    fn pages(&self, cx: &mut Context<Self>) -> Vec<SettingPage> {
        let page = cx.entity();
        let server = &self.session.server;
        let clear_cache = page.downgrade();

        vec![
            SettingPage::new("Appearance")
                .icon(Lucide::Palette)
                .default_open(true)
                .groups([
                    SettingGroup::new().items([
                        SettingItem::new("Theme", picker(&self.theme)),
                        SettingItem::new("Text size", picker(&self.text_size)),
                        SettingItem::new(
                            "Animations",
                            switch(&page, |p| p.animations, Self::set_animations, true),
                        )
                        .description("Animations and smooth scrolling."),
                    ]),
                    SettingGroup::new().title("Home").item(
                        SettingItem::new("Sections and libraries", info("From your Jellyfin profile")).description(
                            "The order of the sections and which libraries show follow your Jellyfin settings \
                                 (Settings → Home in the web client).",
                        ),
                    ),
                ]),
            SettingPage::new("Playback")
                .icon(Lucide::CirclePlay)
                .description(
                    "Audio and subtitle languages, and whether subtitles show by default, follow your Jellyfin \
                     profile (Settings → Playback in the web client).",
                )
                .groups([
                    SettingGroup::new().items([
                        SettingItem::new(
                            "Play the next episode automatically",
                            switch(&page, |p| p.autoplay, Self::set_autoplay, true),
                        ),
                        SettingItem::new(
                            "Skip intros and recaps automatically",
                            switch(&page, |p| p.auto_skip, Self::set_auto_skip, false),
                        ),
                    ]),
                    SettingGroup::new().title("Streaming").items([
                        SettingItem::new("Max streaming bitrate", picker(&self.bitrate))
                            .description(
                                "Videos above this bitrate are transcoded by the server. Lower it for slow \
                                 connections. Automatic measures the connection before playing (at most every 10 \
                                 minutes) and uses 80% of it.",
                            )
                            .keywords(["quality", "transcode"]),
                        SettingItem::new(
                            "Ask for HEVC when the server transcodes",
                            switch(&page, |p| p.prefer_hevc, Self::set_prefer_hevc, false),
                        )
                        .description(
                            "HEVC needs half the bandwidth of H.264 for the same picture, but is much harder for \
                             the server to encode. Used only if the server allows it (Dashboard → Playback → \
                             Transcoding).",
                        )
                        .keywords(["h.265", "codec"]),
                    ]),
                ]),
            SettingPage::new("Subtitles")
                .icon(Lucide::Captions)
                .description(
                    "For text subtitles such as SRT; styled (ASS) subtitles keep their own look and only change \
                     size. The player's settings menu also changes the size while watching.",
                )
                .group(SettingGroup::new().items([
                    SettingItem::new("Colour", picker(&self.subtitle_color)).keywords(["color"]),
                    SettingItem::new("Style", picker(&self.subtitle_style)).keywords(["outline", "box"]),
                    SettingItem::new("Size", picker(&self.subtitle_size)),
                ])),
            SettingPage::new("Storage")
                .icon(Lucide::HardDrive)
                .resettable(false)
                .groups([
                    SettingGroup::new().item(
                        SettingItem::new(
                            "Image cache",
                            SettingField::render(move |_, _, _| {
                                let page = clear_cache.clone();
                                Button::new("clear-cache")
                                    .outline()
                                    .label("Clear")
                                    .on_click(move |_, window, cx| {
                                        page.update(cx, |page, cx| page.clear_image_cache(window, cx)).ok();
                                    })
                            }),
                        )
                        .description(format!(
                            "Posters and backdrops are kept in {}.",
                            config::cache_dir().join("images").display()
                        )),
                    ),
                    SettingGroup::new().item(
                        SettingItem::new(
                            "Logs",
                            SettingField::render(|_, _, _| {
                                Button::new("open-logs")
                                    .outline()
                                    .label("Open folder")
                                    .on_click(|_, _, cx| open_logs(cx))
                            }),
                        )
                        .description(format!(
                            "A log file is written each day to {}. Attach it to bug reports.",
                            config::log_dir().display()
                        )),
                    ),
                ]),
            SettingPage::new("Account")
                .icon(Lucide::CircleUser)
                .resettable(false)
                .group(SettingGroup::new().items([
                    SettingItem::new("User", info(self.session.user.name.clone())),
                    SettingItem::new(
                        "Server",
                        info(server.server_name.clone().unwrap_or_else(|| "Jellyfin".into())),
                    ),
                    SettingItem::new("Address", info(self.session.client.base_url().to_string())),
                    SettingItem::new(
                        "Jellyfin version",
                        info(server.version.clone().unwrap_or_else(|| "?".into())),
                    ),
                ])),
            SettingPage::new("About").icon(Lucide::Info).group(
                SettingGroup::new().items([
                    SettingItem::new("Version", info(env!("CARGO_PKG_VERSION"))),
                    SettingItem::new(
                        "Tell me when a new version is out",
                        switch(&page, |p| p.check_updates, Self::set_check_updates, true),
                    )
                    .description("Checks the GitHub releases once a day, when the app starts."),
                ]),
            ),
        ]
    }
}

impl Focusable for SettingsPage {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SettingsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().track_focus(&self.focus_handle).child(
            Settings::new("settings")
                .with_group_variant(GroupBoxVariant::Outline)
                .pages(self.pages(cx)),
        )
    }
}
