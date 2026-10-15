//! Settings: appearance, playback, subtitles, image cache, logs and account info.

use gpui_kit::component::button::Button;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme, IndexPath, WindowExt, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::browse::{self, Browse, Cursor, Position, Section, on_browse_actions};
use crate::components::selection_ring;
use crate::smooth_scroll::smooth_wheel;

use crate::config::{self, Config, SubtitleColor, SubtitleStyle, ThemePreference};
use crate::images::ImageCache;
use crate::playback::PlaybackManager;
use crate::session::Session;
use crate::theme;
use crate::tracks::{Choice, ChoiceSelect, cycle_choice, step_choice};
use crate::zoom::dp;

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

/// Space around a control for the keyboard cursor's ring.
const RING_GAP: f32 = 3.;
const CONTENT_W: f32 = 720.;

pub struct SettingsPage {
    session: Session,
    scroll: ScrollHandle,
    text_size: ChoiceSelect,
    cursor: Cursor,
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
            scroll: ScrollHandle::new(),
            text_size,
            cursor: Cursor::default(),
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

/// Everything the keyboard cursor can reach, top to bottom.
#[derive(Clone, Copy, PartialEq)]
enum Control {
    Theme,
    TextSize,
    Animations,
    Autoplay,
    AutoSkip,
    Bitrate,
    PreferHevc,
    SubtitleColor,
    SubtitleStyle,
    SubtitleSize,
    ClearCache,
    OpenLogs,
    CheckUpdates,
}

/// A line of the page. Lines are the scroll container's direct children, so the cursor can
/// scroll a control into view by its line's index.
enum Line {
    Title(&'static str),
    Hint(SharedString),
    Control(Control),
}

impl SettingsPage {
    fn lines(&self) -> Vec<Line> {
        use Control::*;
        use Line::{Control as C, Hint, Title};
        let server = &self.session.server;
        let account = format!(
            "Signed in as {} on {} (Jellyfin {}) at {}",
            self.session.user.name,
            server.server_name.as_deref().unwrap_or("Jellyfin"),
            server.version.as_deref().unwrap_or("?"),
            self.session.client.base_url(),
        );
        vec![
            Title("Appearance"),
            C(Theme),
            C(TextSize),
            C(Animations),
            Title("Playback"),
            Hint(
                "Audio and subtitle languages, and whether subtitles show by default, follow your Jellyfin \
                 profile (Settings → Playback in the web client)."
                    .into(),
            ),
            C(Autoplay),
            C(AutoSkip),
            C(Bitrate),
            Hint(
                "Videos above this bitrate are transcoded by the server. Lower it for slow connections. \
                 Automatic measures the connection before playing (at most every 10 minutes) and uses 80% of it."
                    .into(),
            ),
            C(PreferHevc),
            Hint(
                "HEVC needs half the bandwidth of H.264 for the same picture, but is much harder for the server to \
                 encode. Used only if the server allows it (Dashboard → Playback → Transcoding)."
                    .into(),
            ),
            Title("Home"),
            Hint(
                "The order of the sections and which libraries show follow your Jellyfin settings (Settings → \
                 Home in the web client)."
                    .into(),
            ),
            Title("Subtitles"),
            C(SubtitleColor),
            C(SubtitleStyle),
            C(SubtitleSize),
            Hint(
                "For text subtitles such as SRT; styled (ASS) subtitles keep their own look and only change \
                 size. The player's settings menu also changes the size while watching."
                    .into(),
            ),
            Title("Storage"),
            Hint(format!("Images are cached in {}.", config::cache_dir().join("images").display()).into()),
            C(ClearCache),
            Title("Logs"),
            Hint(
                format!(
                    "A log file is written each day to {}. Attach it to bug reports.",
                    config::log_dir().display()
                )
                .into(),
            ),
            C(OpenLogs),
            Title("Account"),
            Hint(account.into()),
            Title("About"),
            Hint(format!("jelly-gpui {}", env!("CARGO_PKG_VERSION")).into()),
            C(CheckUpdates),
            Hint("Checks the GitHub releases once a day, when the app starts.".into()),
        ]
    }

    fn controls(&self) -> Vec<Control> {
        self.lines()
            .into_iter()
            .filter_map(|line| match line {
                Line::Control(control) => Some(control),
                _ => None,
            })
            .collect()
    }

    fn select(&self, control: Control) -> Option<&ChoiceSelect> {
        match control {
            Control::Theme => Some(&self.theme),
            Control::TextSize => Some(&self.text_size),
            Control::Bitrate => Some(&self.bitrate),
            Control::SubtitleColor => Some(&self.subtitle_color),
            Control::SubtitleStyle => Some(&self.subtitle_style),
            Control::SubtitleSize => Some(&self.subtitle_size),
            _ => None,
        }
    }

    /// Sets a switch, or flips it with `None`.
    fn set_switch(&mut self, control: Control, on: Option<bool>, cx: &mut Context<Self>) {
        match control {
            Control::Animations => self.set_animations(on.unwrap_or(!self.animations), cx),
            Control::Autoplay => self.set_autoplay(on.unwrap_or(!self.autoplay), cx),
            Control::AutoSkip => self.set_auto_skip(on.unwrap_or(!self.auto_skip), cx),
            Control::CheckUpdates => self.set_check_updates(on.unwrap_or(!self.check_updates), cx),
            Control::PreferHevc => self.set_prefer_hevc(on.unwrap_or(!self.prefer_hevc), cx),
            _ => {}
        }
    }

    fn render_control(&self, control: Control, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let switch =
            |id: &'static str, checked: bool, label: &'static str, set: fn(&mut Self, bool, &mut Context<Self>)| {
                Switch::new(id)
                    .checked(checked)
                    .label(label)
                    .on_click(cx.listener(move |this, checked: &bool, _, cx| set(this, *checked, cx)))
                    .into_any_element()
            };
        let element = match control {
            Control::Theme
            | Control::TextSize
            | Control::Bitrate
            | Control::SubtitleColor
            | Control::SubtitleStyle
            | Control::SubtitleSize => {
                let select = self.select(control).expect("a picker");
                Select::new(select).w(rems(220. / 16.)).into_any_element()
            }
            Control::Animations => switch(
                "animations",
                self.animations,
                "Animations and smooth scrolling",
                Self::set_animations,
            ),
            Control::Autoplay => switch(
                "autoplay",
                self.autoplay,
                "Play the next episode automatically",
                Self::set_autoplay,
            ),
            Control::AutoSkip => switch(
                "auto-skip",
                self.auto_skip,
                "Skip intros and recaps automatically",
                Self::set_auto_skip,
            ),
            Control::PreferHevc => switch(
                "prefer-hevc",
                self.prefer_hevc,
                "Ask for HEVC when the server transcodes",
                Self::set_prefer_hevc,
            ),
            Control::CheckUpdates => switch(
                "check-updates",
                self.check_updates,
                "Tell me when a new version is out",
                Self::set_check_updates,
            ),
            Control::ClearCache => Button::new("clear-cache")
                .outline()
                .label("Clear image cache")
                .on_click(cx.listener(|this, _, window, cx| this.clear_image_cache(window, cx)))
                .into_any_element(),
            Control::OpenLogs => Button::new("open-logs")
                .outline()
                .label("Open log folder")
                .on_click(|_, _, cx| open_logs(cx))
                .into_any_element(),
        };
        // Only the control itself is ringed, not a picker's label.
        let ringed = |element: AnyElement| {
            div()
                .relative()
                .p(dp(RING_GAP))
                .rounded_xl()
                .child(element)
                .when(self.cursor.is_selected(0, index), |el| {
                    el.child(selection_ring(cx).rounded_xl())
                })
        };
        match self.select(control) {
            Some(_) => h_flex()
                .gap(dp(16. - RING_GAP))
                .child(
                    div()
                        .w(rems(170. / 16.))
                        .flex_none()
                        .text_sm()
                        .child(control_label(control)),
                )
                .child(ringed(element))
                .into_any_element(),
            // A ring's width to the left, so the switch or button lines up with the text.
            None => h_flex().ml(dp(-RING_GAP)).child(ringed(element)).into_any_element(),
        }
    }
}

/// A picker's label, left of it.
fn control_label(control: Control) -> &'static str {
    match control {
        Control::Theme => "Theme",
        Control::TextSize => "Text size",
        Control::Bitrate => "Max streaming bitrate",
        Control::SubtitleColor => "Colour",
        Control::SubtitleStyle => "Style",
        Control::SubtitleSize => "Size",
        _ => "",
    }
}

fn open_logs(cx: &mut App) {
    cx.open_with_system(&config::log_dir());
}

impl Browse for SettingsPage {
    fn cursor(&mut self) -> &mut Cursor {
        &mut self.cursor
    }

    fn sections(&self, _window: &Window, _cx: &App) -> Vec<Section> {
        vec![Section::grid(self.controls().len(), 1)]
    }

    fn reveal(&mut self, position: Position, _window: &mut Window, _cx: &mut Context<Self>) {
        let lines = self.lines();
        let Some(line) = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| matches!(line, Line::Control(_)))
            .nth(position.index)
            .map(|(i, _)| i)
        else {
            return;
        };
        // A section's first control brings its title along.
        if line > 0 && matches!(lines[line - 1], Line::Title(_)) {
            browse::reveal(&self.scroll, line - 1, px(24.));
        }
        browse::reveal(&self.scroll, line, px(24.));
    }

    /// Left and Right step a picker, or turn a switch off and on.
    fn adjust(&mut self, position: Position, forward: bool, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(&control) = self.controls().get(position.index) else {
            return false;
        };
        match self.select(control) {
            Some(select) => {
                step_choice(&select.clone(), forward, window, cx);
            }
            None => self.set_switch(control, Some(forward), cx),
        }
        // Buttons ignore them, rather than the cursor wandering off sideways.
        true
    }

    /// Enter moves a picker to its next choice, flips a switch or presses a button.
    fn activate(&mut self, position: Position, window: &mut Window, cx: &mut Context<Self>) {
        let Some(&control) = self.controls().get(position.index) else {
            return;
        };
        if let Some(select) = self.select(control) {
            cycle_choice(&select.clone(), window, cx);
            return;
        }
        match control {
            Control::ClearCache => self.clear_image_cache(window, cx),
            Control::OpenLogs => open_logs(cx),
            control => self.set_switch(control, None, cx),
        }
        cx.notify();
    }
}

impl Focusable for SettingsPage {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SettingsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut control_index = 0;
        let lines = self.lines();
        // A hint sits close to the title or control it follows, which it's about.
        let follows_control: Vec<bool> = std::iter::once(false)
            .chain(lines.iter().map(|line| matches!(line, Line::Control(_))))
            .collect();
        let lines: Vec<AnyElement> = lines
            .into_iter()
            .enumerate()
            .map(|(i, line)| {
                let element = match line {
                    Line::Title(title) => div()
                        .when(i > 0, |el| el.pt_8())
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(cx.theme().foreground)
                        .child(title)
                        .into_any_element(),
                    Line::Hint(text) => div()
                        .map(|el| if follows_control[i] { el.pt_1() } else { el.pt_2() })
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(text)
                        .into_any_element(),
                    Line::Control(control) => {
                        control_index += 1;
                        div()
                            .pt(dp(12. - RING_GAP))
                            .child(self.render_control(control, control_index - 1, cx))
                            .into_any_element()
                    }
                };
                div().w_full().max_w(dp(CONTENT_W)).child(element).into_any_element()
            })
            .collect();
        on_browse_actions(div().size_full().track_focus(&self.focus_handle), cx).child(
            div()
                .id("settings")
                .size_full()
                .overflow_y_scroll()
                .track_scroll(&self.scroll)
                .on_scroll_wheel(smooth_wheel("settings", &self.scroll))
                .p_6()
                .children(lines),
        )
    }
}
