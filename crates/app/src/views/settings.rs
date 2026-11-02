//! Settings: a menu of sections on the left, split by where their settings are kept, and every
//! section on the right, one after another in a scrolling column. A menu entry scrolls its
//! section to the top, and the menu marks the section at the top as the column scrolls. A
//! section is its title, a tag naming where its settings are kept, and one card of rows split
//! by hairlines: each row's label over what it does, its control at the right, and the key it
//! saves in the label's tooltip.
//!
//! - **This device** (appearance, playback, subtitles, storage, about) is the config file: one
//!   per computer, shared by every account signed in on it. Every option keeps `main`'s config
//!   key and value (a change loads the config fresh, changes it and saves it, so it can't write
//!   over what playback or the window saved meanwhile), the same choices in the same order, the
//!   default that applies while the file leaves the key out, and what the change takes effect
//!   on: the theme, text size and animations apply at once through
//!   `AppEvent::AppearanceChanged`, the subtitle size goes to the playback manager while a
//!   session runs, and the rest are read when what they configure next starts (a playback
//!   session, the player page, the update check at launch).
//! - **Jellyfin server** (account, languages, home) is the signed-in user's configuration on the
//!   server, which the web client and every other app share. It's read when the page opens and
//!   each change is saved in the background, one request at a time ([`ServerSettings`]). The
//!   server applies the track preferences itself, when `PlaybackInfo` picks an item's default
//!   streams, so they take effect from the next item on.
//!
//! A device setting that's off its default has an undo button left of its control, which leaves
//! the key out of the file again; the server's have no default of the app's to go back to.
//!
//! The top bar's search box filters the page while it's on top (`Page::own_search`): every row
//! of both stores by its section, label, description and key, leaving out the sections with
//! nothing left; a menu entry leaves the search.
//!
//! The page isn't on the browse cursor, as on `main`: [`Page::scope`] is `None`, so egui's own
//! focus handles Tab (the menu, then the sections' controls, scrolling each into
//! view), Space or Enter on a menu entry, the arrows in a picker and Space on a switch, and Esc
//! still goes back.

use std::path::Path;

use egui::{Align, Align2, Rect, RichText, Role, Sense, Ui, UiBuilder, WidgetInfo, pos2, vec2};
use egui_material_icons::MaterialIcon;
use egui_material_icons::icons::{
    ICON_ACCOUNT_CIRCLE, ICON_CLOUD_DONE, ICON_COMPUTER, ICON_DNS, ICON_HOME, ICON_INFO, ICON_PALETTE,
    ICON_PLAY_CIRCLE, ICON_STORAGE, ICON_SUBTITLES, ICON_TRANSLATE, ICON_UNDO,
};
use jellyfin::models::{Culture, SubtitleMode, UserConfiguration, UserConfigurationChanges};

use crate::config::{self, Config, SubtitleColor, SubtitleStyle, ThemePreference};
use crate::services::AppEvent;
use crate::task::Task;
use crate::theme;
use crate::theme::SPACE;
use crate::theme::text;
use crate::toasts::Toast;
use crate::views::{Page, PageCx};
use crate::widgets::avatar_row::AvatarRow;
use crate::widgets::button::{self, Focus, State};
use crate::widgets::focus::{self, Place};
use crate::widgets::label;
use crate::widgets::round_button::RoundButton;
use crate::widgets::{backdrop, select, surface, toggle};

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

/// The server's subtitle modes, in the web client's order.
const SUBTITLE_MODES: [(SubtitleMode, &str); 5] = [
    (SubtitleMode::Default, "Default"),
    (SubtitleMode::Smart, "Smart"),
    (SubtitleMode::OnlyForced, "Only forced"),
    (SubtitleMode::Always, "Always"),
    (SubtitleMode::Off, "None"),
];

/// Subtitle sizes offered here; the player's menu sets any size in 10 % steps.
const SUBTITLE_SCALES: [f64; 6] = [0.75, 1., 1.25, 1.5, 1.75, 2.];

/// Max streaming bitrate choices in Mbit/s; 0 is no limit.
const BITRATES_MBPS: [i32; 10] = [0, 120, 60, 40, 20, 10, 8, 4, 2, 1];
/// The bitrate choice that measures the connection instead.
const AUTOMATIC: i32 = -1;

/// The page's top and bottom padding, and the width the section's form is held to, centred in
/// what's left: rows of labels and controls stretched over a wide window read badly.
const PAGE_PADDING: f32 = 24.;
const CONTENT_W: f32 = 760.;
/// The menu's width: a quarter of the page, within these.
const MENU_MIN_W: f32 = 190.;
const MENU_MAX_W: f32 = 250.;
/// The `brand` bar at the open section's menu entry.
const MENU_BAR_W: f32 = 3.;
/// The pickers' width beside their label (`main`'s 220 px).
const PICKER_W: f32 = 220.;
/// The gap under a menu heading, and between one section's card and the next one's title.
const HEADING_GAP: f32 = 4.;
const SECTION_GAP: f32 = 32.;
/// The room inside a section's card, across and down: the rows bring their own above and below
/// them ([`ROW_PAD`]), so the card's own is small.
const CARD_PADDING: [i8; 2] = [20, 6];
/// The room above and below a row's text, inside the hairlines.
const ROW_PAD: f32 = 12.;
/// The undo button left of a changed setting's control, and its icon.
const RESET_W: f32 = 30.;
const RESET_ICON: f32 = 18.;
/// The storage rows' buttons.
const ROW_BUTTON_W: f32 = 132.;

/// How long the menu keeps marking the section an entry scrolled to before following the
/// column again, should the scroll never get there (it was taken over by the wheel).
const ARRIVAL_WAIT: f64 = 1.;
/// How far below the column's top a section's start can be and still count as the one at the
/// top: a scroll lands it there within rounding.
const TOP_SLACK: f32 = 8.;

/// Where a section's settings are kept, which is how the menu groups them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Store {
    /// This computer's config file.
    Device,
    /// The signed-in user's configuration on the server.
    Server,
}

impl Store {
    const ALL: [Store; 2] = [Store::Device, Store::Server];

    fn title(self) -> &'static str {
        match self {
            Store::Device => "This device",
            Store::Server => "Jellyfin server",
        }
    }

    fn icon(self) -> MaterialIcon {
        match self {
            Store::Device => ICON_COMPUTER,
            Store::Server => ICON_DNS,
        }
    }
}

/// A page of the menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Section {
    Appearance,
    Playback,
    Subtitles,
    Storage,
    About,
    Account,
    Languages,
    Home,
}

impl Section {
    /// In the menu's order: the device's, then the server's.
    pub(crate) const ALL: [Section; 8] = [
        Section::Appearance,
        Section::Playback,
        Section::Subtitles,
        Section::Storage,
        Section::About,
        Section::Account,
        Section::Languages,
        Section::Home,
    ];

    fn store(self) -> Store {
        match self {
            Section::Appearance | Section::Playback | Section::Subtitles | Section::Storage | Section::About => {
                Store::Device
            }
            Section::Account | Section::Languages | Section::Home => Store::Server,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Section::Appearance => "Appearance",
            Section::Playback => "Playback",
            Section::Subtitles => "Subtitles",
            Section::Storage => "Storage",
            Section::About => "About",
            Section::Account => "Account",
            Section::Languages => "Languages",
            Section::Home => "Home",
        }
    }

    fn icon(self) -> MaterialIcon {
        match self {
            Section::Appearance => ICON_PALETTE,
            Section::Playback => ICON_PLAY_CIRCLE,
            Section::Subtitles => ICON_SUBTITLES,
            Section::Storage => ICON_STORAGE,
            Section::About => ICON_INFO,
            Section::Account => ICON_ACCOUNT_CIRCLE,
            Section::Languages => ICON_TRANSLATE,
            Section::Home => ICON_HOME,
        }
    }
}

/// What a row shows besides its control: the setting's name, what it does (when it takes
/// effect included), the key it saves, and the value that applies while the file doesn't have
/// that key. [`row`] draws them around the control; the search reads them all.
///
/// A row with no label is a note (only its description), and one with no description or key
/// is information with nothing to change.
#[derive(Clone, Copy)]
struct Row<'a> {
    section: Section,
    label: &'a str,
    description: &'a str,
    /// The config key, the server's field, or what the row is about when nothing is saved.
    key: &'a str,
    /// The value while the key is unset; empty for a row with nothing to save, and for the
    /// server's, which always has a value.
    unset: &'a str,
}

const THEME: Row = Row {
    section: Section::Appearance,
    label: "Theme",
    description: "Dark, light, or the desktop's colour scheme. Applies at once.",
    key: "theme",
    unset: "dark",
};
const TEXT_SIZE: Row = Row {
    section: Section::Appearance,
    label: "Text size",
    description: "Scales the app's text and controls, not its layout. Applies at once.",
    key: "text_size",
    unset: "the system text size",
};
const ANIMATIONS: Row = Row {
    section: Section::Appearance,
    label: "Animations",
    description: "Animations and smooth scrolling. Applies at once.",
    key: "animations",
    unset: "on",
};

const TRACKS_NOTE: Row = Row {
    section: Section::Playback,
    label: "",
    description: "Which audio and subtitle tracks an item starts with is kept on the server, for every app \
                  you use.",
    key: "",
    unset: "",
};
const AUTOPLAY: Row = Row {
    section: Section::Playback,
    label: "Play the next episode automatically",
    description: "When an episode ends, so the player stays open. From the next item on.",
    key: "autoplay_next_episode",
    unset: "on",
};
const SKIP_INTRO: Row = Row {
    section: Section::Playback,
    label: "Skip intros and recaps automatically",
    description: "Each intro or recap once as it starts, never in a SyncPlay group. From the next item on.",
    key: "auto_skip_intro",
    unset: "off",
};
const BITRATE: Row = Row {
    section: Section::Playback,
    label: "Max streaming bitrate",
    description: "Videos above this bitrate are transcoded by the server. Lower it for slow connections. \
                  Automatic measures the connection before playing (at most every 10 minutes) and uses 80% of \
                  it, saving automatic_bitrate and keeping the cap for when it's turned off. Read when the next \
                  item starts.",
    key: "max_streaming_bitrate",
    unset: "no limit",
};
const HEVC: Row = Row {
    section: Section::Playback,
    label: "Ask for HEVC when the server transcodes",
    description: "HEVC needs half the bandwidth of H.264 for the same picture, but is much harder for the \
                  server to encode. Used only if the server allows it (Dashboard › Playback › Transcoding). \
                  Read when the next item starts.",
    key: "prefer_hevc",
    unset: "off",
};

const SUBTITLES_NOTE: Row = Row {
    section: Section::Subtitles,
    label: "",
    description: "How text subtitles such as SRT look; styled (ASS) subtitles keep their own look and only \
                  change size. The player's settings menu also changes the size while watching.",
    key: "",
    unset: "",
};
const SUBTITLE_TRACKS_NOTE: Row = Row {
    section: Section::Subtitles,
    label: "",
    description: "Which subtitles turn on by themselves, and in which language, is kept on the server.",
    key: "",
    unset: "",
};
const SUBTITLE_COLOR: Row = Row {
    section: Section::Subtitles,
    label: "Colour",
    description: "Of text subtitles. Read when the next item starts.",
    key: "subtitle_color",
    unset: "white",
};
const SUBTITLE_STYLE: Row = Row {
    section: Section::Subtitles,
    label: "Style",
    description: "An outline, or a translucent dark box, behind text subtitles. Read when the next item starts.",
    key: "subtitle_style",
    unset: "outline",
};
const SUBTITLE_SIZE: Row = Row {
    section: Section::Subtitles,
    label: "Size",
    description: "Shared with the player's settings menu. Goes to the playback manager at once, so a running \
                  session changes with it.",
    key: "subtitle_scale",
    unset: "100 %",
};

/// The storage rows' smaller line shows the folder, which [`SettingsPage::storage`] fills in.
const IMAGE_CACHE: Row = Row {
    section: Section::Storage,
    label: "Image cache",
    description: "Posters and backdrops are kept here and reload as pages ask for them again.",
    key: "",
    unset: "",
};
const LOGS: Row = Row {
    section: Section::Storage,
    label: "Logs",
    description: "A log file is written each day. Attach it to bug reports.",
    key: "",
    unset: "",
};

const VERSION: Row = info_row(Section::About, "Version");
const CHECK_FOR_UPDATES: Row = Row {
    section: Section::About,
    label: "Tell me when a new version is out",
    description: "Checks the GitHub releases once a day, when the app starts.",
    key: "check_for_updates",
    unset: "on",
};

const USER: Row = info_row(Section::Account, "User");
const SERVER: Row = info_row(Section::Account, "Server");
const ADDRESS: Row = info_row(Section::Account, "Address");
const SERVER_VERSION: Row = info_row(Section::Account, "Jellyfin version");

const LANGUAGES_NOTE: Row = Row {
    section: Section::Languages,
    label: "",
    description: "The server picks the tracks an item starts with from these. The player's menus change them \
                  while watching. From the next item on.",
    key: "",
    unset: "",
};
const AUDIO_LANGUAGE: Row = Row {
    section: Section::Languages,
    label: "Audio language",
    description: "The audio track in this language, when the file has one.",
    key: "AudioLanguagePreference",
    unset: "",
};
const PLAY_DEFAULT_AUDIO: Row = Row {
    section: Section::Languages,
    label: "Play the default audio track",
    description: "The track the file marks as default, whatever its language.",
    key: "PlayDefaultAudioTrack",
    unset: "",
};
const REMEMBER_AUDIO: Row = Row {
    section: Section::Languages,
    label: "Remember the audio track I pick",
    description: "A track picked by hand is picked again for the next episodes of the series.",
    key: "RememberAudioSelections",
    unset: "",
};
const SUBTITLE_LANGUAGE: Row = Row {
    section: Section::Languages,
    label: "Subtitle language",
    description: "The language the subtitle mode looks for.",
    key: "SubtitleLanguagePreference",
    unset: "",
};
const SUBTITLE_MODE: Row = Row {
    section: Section::Languages,
    label: "Subtitle mode",
    description: "Which subtitles turn on by themselves. Default: the ones the file marks as default or \
                  forced. Smart: the subtitle language's when the audio is in another. Only forced: forced \
                  ones only. Always: the subtitle language's whatever the audio. None: none until one is \
                  picked.",
    key: "SubtitleMode",
    unset: "",
};
const REMEMBER_SUBTITLES: Row = Row {
    section: Section::Languages,
    label: "Remember the subtitles I pick",
    description: "Subtitles picked by hand, or turned off, are picked again for the next episodes of the \
                  series.",
    key: "RememberSubtitleSelections",
    unset: "",
};

const HIDE_PLAYED_IN_LATEST: Row = Row {
    section: Section::Home,
    label: "Leave watched items out of Latest",
    description: "Home's Latest rows show only what you haven't watched. Home reloads when it's saved.",
    key: "HidePlayedInLatest",
    unset: "",
};
// No settings of our own for these, as on `main`: the web client's profile decides.
const HOME_SECTIONS_NOTE: Row = Row {
    section: Section::Home,
    label: "",
    description: "Which sections Home shows, and the libraries in them, follow your Jellyfin profile \
                  (Settings › Home in the web client).",
    key: "",
    unset: "",
};

/// Every row, for the search. A row left out of here still shows in its section, but a search
/// would only find it through another row of the same section.
const ROWS: [Row; 33] = [
    THEME,
    TEXT_SIZE,
    ANIMATIONS,
    TRACKS_NOTE,
    AUTOPLAY,
    SKIP_INTRO,
    BITRATE,
    HEVC,
    SUBTITLES_NOTE,
    SUBTITLE_TRACKS_NOTE,
    SUBTITLE_COLOR,
    SUBTITLE_STYLE,
    SUBTITLE_SIZE,
    IMAGE_CACHE,
    LOGS,
    VERSION,
    CHECK_FOR_UPDATES,
    USER,
    SERVER,
    ADDRESS,
    SERVER_VERSION,
    LANGUAGES_NOTE,
    AUDIO_LANGUAGE,
    PLAY_DEFAULT_AUDIO,
    REMEMBER_AUDIO,
    SUBTITLE_LANGUAGE,
    SUBTITLE_MODE,
    REMEMBER_SUBTITLES,
    HIDE_PLAYED_IN_LATEST,
    HOME_SECTIONS_NOTE,
    // The section headings' own words find the section through these.
    section_row(Section::Storage),
    section_row(Section::Account),
    section_row(Section::Home),
];

/// A row that's only information: a label and a value beside it.
const fn info_row(section: Section, label: &'static str) -> Row<'static> {
    Row {
        section,
        label,
        description: "",
        key: "",
        unset: "",
    }
}

/// A row that's never drawn, so that a search for a section's title or store finds it even
/// when no row's own words do. The search reads the section's title for every row anyway.
const fn section_row(section: Section) -> Row<'static> {
    Row {
        section,
        label: "",
        description: "",
        key: "",
        unset: "",
    }
}

/// The search box's text, and what it matches.
#[derive(Default)]
struct Search {
    text: String,
}

impl Search {
    fn active(&self) -> bool {
        !self.text.trim().is_empty()
    }

    /// Whether `row` is shown: always without a search, otherwise when every word typed is in
    /// its store's or section's title, its label, its description or its key, ignoring case.
    fn shows(&self, row: &Row) -> bool {
        if !self.active() {
            return true;
        }
        let haystack = [
            row.section.store().title(),
            row.section.title(),
            row.label,
            row.description,
            row.key,
        ]
        .join(" ")
        .to_lowercase();
        self.text
            .to_lowercase()
            .split_whitespace()
            .all(|word| haystack.contains(word))
    }

    /// How many of `section`'s rows the search finds.
    fn matches(&self, section: Section) -> usize {
        ROWS.iter()
            .filter(|row| row.section == section && self.shows(row))
            .count()
    }
}

pub struct SettingsPage {
    /// The config as this page last read or wrote it, for the controls' values. A change reads
    /// the file again rather than writing this copy back, as `main`'s `save` did.
    config: Config,
    /// What a button started (clearing the image cache, opening the logs folder), until its
    /// outcome is a toast.
    job: Option<Task<Job>>,
    /// Left and Right on a focused picker.
    pickers: Pickers,
    /// The section the menu marks: the one at the top of the column when it was last drawn.
    section: Section,
    /// A menu entry's section, scrolled to the top when the column draws next.
    scroll_to: Option<Section>,
    /// The section a menu entry scrolled to and when (`InputState::time`), marked until the
    /// scroll gets there rather than each section it passes on the way.
    arriving: Option<(Section, f64)>,
    search: Search,
    server: ServerSettings,
}

/// What a button on the page asked for, with how it went.
enum Job {
    /// Cleared the image cache's folder.
    Cleared(std::io::Result<()>),
    /// Opened the logs folder in the desktop's file manager.
    Opened(std::io::Result<()>),
}

impl SettingsPage {
    pub fn new() -> Self {
        Self {
            config: Config::load(),
            job: None,
            pickers: Pickers::default(),
            section: Section::Appearance,
            scroll_to: None,
            arriving: None,
            search: Search::default(),
            server: ServerSettings::default(),
        }
    }

    /// Writes a change through the config file: loaded fresh, changed, saved, and kept for the
    /// controls. Passwords and tokens never go through here; they're in `credentials`.
    fn save(&mut self, change: impl FnOnce(&mut Config)) {
        let mut config = Config::load();
        change(&mut config);
        if let Err(err) = config.save() {
            tracing::warn!("failed to save config: {err:#}");
        }
        self.config = config;
    }

    /// Saves an appearance setting and asks the app to apply it: the theme, the text size and
    /// the animations are the app's, not the session's, so they apply on every screen.
    fn save_appearance(&mut self, cx: &mut PageCx, change: impl FnOnce(&mut Config)) {
        self.save(change);
        cx.services.events.push(AppEvent::AppearanceChanged);
    }

    /// Scrolls to `section`, from the menu or a note's link, leaving the search.
    fn open(&mut self, ctx: &egui::Context, section: Section) {
        self.section = section;
        self.scroll_to = Some(section);
        self.arriving = Some((section, ctx.input(|input| input.time)));
        self.search.text.clear();
    }

    /// Marks the section at the top of the column, `top`, in the menu; after a menu entry, its
    /// section until the scroll has brought it there (or stopped short of it for a while).
    fn follow(&mut self, ctx: &egui::Context, top: Section) {
        if let Some((section, since)) = self.arriving {
            let now = ctx.input(|input| input.time);
            if top == section || now - since > ARRIVAL_WAIT {
                self.arriving = None;
            }
        }
        self.section = self.arriving.map_or(top, |(section, _)| section);
    }

    /// Drops the image cache's memory now and deletes its folder, which a large cache with many
    /// posters in it can take a moment to do: the delete runs on a blocking thread and the page
    /// says how it went. Images reload from the server as pages ask for them again.
    fn clear_image_cache(&mut self, cx: &mut PageCx) {
        cx.services.images.clear_memory();
        let dir = config::cache_dir().join("images");
        self.job = Some(Task::spawn(cx.ctx, async move {
            // The blocking delete isn't cancelled if the page goes and drops the task; only its
            // outcome goes unreported.
            let removed = tokio::task::spawn_blocking(move || match std::fs::remove_dir_all(&dir) {
                Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err),
                _ => Ok(()),
            })
            .await;
            Job::Cleared(match removed {
                Ok(result) => result,
                Err(err) => Err(std::io::Error::other(err)),
            })
        }));
    }

    /// Opens the logs folder, where a file is written each day. On a blocking thread, so a
    /// launcher that takes its time can't hold up a frame.
    fn open_logs(&mut self, cx: &mut PageCx) {
        let dir = config::log_dir();
        self.job = Some(Task::spawn(cx.ctx, async move {
            let opened = tokio::task::spawn_blocking(move || open_folder(&dir)).await;
            Job::Opened(match opened {
                Ok(result) => result,
                Err(err) => Err(std::io::Error::other(err)),
            })
        }));
    }

    /// The outcome of [`SettingsPage::job`], as a toast.
    fn report_job(&mut self, cx: &mut PageCx) {
        let Some(job) = self.job.as_mut().and_then(Task::poll) else {
            return;
        };
        self.job = None;
        match job {
            Job::Cleared(Ok(())) => cx.services.toast(Toast::success("Image cache cleared.")),
            Job::Cleared(Err(err)) => {
                tracing::warn!("clearing the image cache failed: {err}");
                let message = "Couldn't clear the image cache. Close anything using its folder and try again.";
                cx.services.toast(Toast::error(message));
            }
            Job::Opened(Ok(())) => {}
            Job::Opened(Err(err)) => {
                tracing::warn!("couldn't open {}: {err}", config::log_dir().display());
                let message = format!("Couldn't open the logs folder ({}).", config::log_dir().display());
                cx.services.toast(Toast::error(message));
            }
        }
    }
}

impl Default for SettingsPage {
    fn default() -> Self {
        Self::new()
    }
}

impl Page for SettingsPage {
    fn title(&self) -> &str {
        "Settings"
    }

    fn own_search(&self) -> Option<&str> {
        Some(&self.search.text)
    }

    fn set_own_search(&mut self, text: &str) {
        self.search.text = text.to_owned();
    }

    fn show(&mut self, ui: &mut egui::Ui, cx: &mut PageCx) {
        self.pickers.begin_frame(cx.ctx);
        self.report_job(cx);
        self.server.tick(cx);

        // The menu and the sections side by side on the backdrop, with no line between them:
        // the sections' surfaces set the column apart. Drawn in that order, so Tab goes through
        // the menu before the sections' controls.
        let rect = ui.available_rect_before_wrap();
        let menu_w = (rect.width() * 0.25).clamp(MENU_MIN_W, MENU_MAX_W);
        let menu = Rect::from_min_max(rect.min, pos2(rect.left() + menu_w, rect.bottom()));
        let content = Rect::from_min_max(pos2(menu.right(), rect.top()), rect.max);
        ui.scope_builder(UiBuilder::new().max_rect(menu), |ui| self.menu(ui));
        ui.scope_builder(UiBuilder::new().max_rect(content), |ui| self.content(ui, cx));
        ui.advance_cursor_after_rect(rect);
    }
}

impl SettingsPage {
    /// The two groups of sections.
    fn menu(&mut self, ui: &mut Ui) {
        let area = egui::ScrollArea::vertical()
            .id_salt("settings_menu")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Frame::NONE
                    .inner_margin(egui::Margin::symmetric(12, PAGE_PADDING as i8))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        for (index, store) in Store::ALL.into_iter().enumerate() {
                            if index > 0 {
                                ui.add_space(SPACE.xl);
                            }
                            menu_heading(ui, store);
                            for section in Section::ALL.into_iter().filter(|section| section.store() == store) {
                                let searching = self.search.active();
                                let matches = searching.then(|| self.search.matches(section));
                                let selected = !searching && self.section == section;
                                if menu_entry(ui, section, selected, matches).clicked() {
                                    self.open(ui.ctx(), section);
                                }
                            }
                        }
                    });
            });
        backdrop::scroll_edge(ui, &area);
    }

    /// The column of sections, scrolling under the top bar. A search's results keep a scroll
    /// position of their own, so leaving the search puts the column back where it was.
    fn content(&mut self, ui: &mut Ui, cx: &mut PageCx) {
        let searching = self.search.active();
        let area = egui::ScrollArea::vertical()
            .id_salt(("settings_content", searching))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let viewport = ui.clip_rect();
                let full = ui.available_width();
                let column = (full - 2. * PAGE_PADDING).clamp(0., CONTENT_W);
                let side = ((full - column) * 0.5).max(0.);
                // A nested `vertical` Ui, not `with_layout` around the whole column: a frame's
                // contents lay out the way its parent does, and they have to stack downwards.
                ui.horizontal_top(|ui| {
                    ui.add_space(side);
                    ui.vertical(|ui| {
                        ui.set_width(column);
                        ui.add_space(PAGE_PADDING);
                        let starts = self.sections(ui, cx);
                        ui.add_space(PAGE_PADDING);
                        if searching {
                            return;
                        }
                        // Room under the last section for it to scroll to the top too, so its
                        // menu entry can bring it there and the menu then marks it.
                        if let Some(&(_, last)) = starts.last() {
                            let below = ui.cursor().top() - last;
                            ui.add_space((viewport.height() - below).max(0.));
                        }
                        let top = starts
                            .iter()
                            .rev()
                            .find(|(_, start)| *start <= viewport.top() + TOP_SLACK)
                            .or(starts.first())
                            .map(|&(section, _)| section);
                        if let Some(top) = top {
                            self.follow(ui.ctx(), top);
                        }
                    });
                });
            });
        backdrop::scroll_edge(ui, &area);
    }

    /// Every section with a row the search finds (all of them without a search), and the
    /// section a menu entry asked for scrolled to the top. The first section of each store says
    /// what keeping them there means. Returns where each section starts: the page's padding
    /// above the first one, otherwise the gap above its title.
    fn sections(&mut self, ui: &mut Ui, cx: &mut PageCx) -> Vec<(Section, f32)> {
        let found: Vec<Section> = Section::ALL
            .into_iter()
            .filter(|&section| self.search.matches(section) > 0)
            .collect();
        let mut starts = Vec::with_capacity(found.len());
        if found.is_empty() {
            let message = format!("No settings match “{}”.", self.search.text.trim());
            ui.label(label::muted(ui, message));
            return starts;
        }
        let mut store = None;
        for (index, section) in found.into_iter().enumerate() {
            let start = if index == 0 {
                ui.cursor().top() - PAGE_PADDING
            } else {
                let start = ui.cursor().top();
                ui.add_space(SECTION_GAP);
                start
            };
            let first_of_store = store != Some(section.store());
            store = Some(section.store());
            self.heading(ui, cx, section, first_of_store);
            ui.add_space(SPACE.lg);
            self.section(ui, cx, section);
            starts.push((section, start));
            if self.scroll_to == Some(section) {
                self.scroll_to = None;
                let anchor = Rect::from_min_size(pos2(ui.min_rect().left(), start), vec2(1., 1.));
                ui.scroll_to_rect(anchor, Some(Align::TOP));
            }
        }
        starts
    }

    /// A section's title with a tag naming where its settings are kept, so the device's and the
    /// server's can't be mistaken for each other, and, when `explain`, a line saying what that
    /// means (with how saving to the server is going).
    fn heading(&self, ui: &mut Ui, cx: &PageCx, section: Section, explain: bool) {
        let tokens = theme::tokens_of(ui);
        let store = section.store();
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(section.title())
                    .font(text::font(ui, "xl"))
                    .color(tokens.ink),
            );
            ui.add_space(SPACE.xs);
            store_tag(ui, store);
        });
        if !explain {
            return;
        }
        let description = match store {
            Store::Device => "Kept on this computer, for every account signed in here.".to_string(),
            Store::Server => {
                let session = &cx.services.session;
                let server = session.server.server_name.as_deref().unwrap_or("the server");
                format!(
                    "Kept in {}'s account on {server}, so the web client and other Jellyfin apps share them.",
                    session.user.name
                )
            }
        };
        ui.add_space(SPACE.xs);
        ui.label(label::muted(ui, description));
        if store == Store::Server
            && let Some(status) = self.server.status()
        {
            ui.label(label::muted(ui, status));
        }
    }

    /// A section's card and its rows.
    fn section(&mut self, ui: &mut Ui, cx: &mut PageCx, section: Section) {
        card(ui, |ui| match section {
            Section::Appearance => self.appearance(ui, cx),
            Section::Playback => self.playback(ui),
            Section::Subtitles => self.subtitles(ui, cx),
            Section::Storage => self.storage(ui, cx),
            Section::About => self.about(ui),
            Section::Account => self.account(ui, cx),
            Section::Languages => self.languages(ui, cx),
            Section::Home => self.home(ui, cx),
        });
    }

    fn appearance(&mut self, ui: &mut Ui, cx: &mut PageCx) {
        let current = self.config.theme;
        let default = Some(ThemePreference::default());
        let picked = self
            .pickers
            .choose(ui, &self.search, THEME, "settings_theme", &THEMES, current, default);
        if let Some(edit) = picked {
            self.save_appearance(cx, |config| config.theme = edit.or_unset().unwrap_or_default());
        }

        // `None` follows the system, whose size can differ from 100 %.
        let system = format!("Follow the system ({:.0}%)", theme::system_text_scale() * 100.);
        let text_sizes: Vec<(Option<u16>, String)> = std::iter::once((None, system))
            .chain(
                theme::TEXT_SIZES
                    .iter()
                    .map(|&percent| (Some(percent), format!("{percent}%"))),
            )
            .collect();
        let current = self.config.text_size;
        let picked = self.pickers.choose(
            ui,
            &self.search,
            TEXT_SIZE,
            "settings_text_size",
            &text_sizes,
            current,
            Some(None),
        );
        if let Some(edit) = picked {
            self.save_appearance(cx, |config| config.text_size = edit.or_unset().flatten());
        }

        let current = self.config.animations.unwrap_or(true);
        if let Some(edit) = toggle(ui, &self.search, ANIMATIONS, current, Some(true)) {
            self.save_appearance(cx, |config| config.animations = edit.or_unset());
        }
    }

    fn playback(&mut self, ui: &mut Ui) {
        if note(ui, &self.search, TRACKS_NOTE, Some("Languages")) {
            self.open(ui.ctx(), Section::Languages);
        }

        let current = self.config.autoplay_next_episode.unwrap_or(true);
        if let Some(edit) = toggle(ui, &self.search, AUTOPLAY, current, Some(true)) {
            self.save(|config| config.autoplay_next_episode = edit.or_unset());
        }

        let current = self.config.auto_skip_intro.unwrap_or(false);
        if let Some(edit) = toggle(ui, &self.search, SKIP_INTRO, current, Some(false)) {
            self.save(|config| config.auto_skip_intro = edit.or_unset());
        }

        // `AUTOMATIC` measures the connection before playing instead.
        let bitrates: Vec<(i32, String)> = std::iter::once((AUTOMATIC, "Automatic (measured)".to_string()))
            .chain(BITRATES_MBPS.iter().map(|&mbps| match mbps {
                0 => (mbps, "No limit".to_string()),
                mbps => (mbps, format!("{mbps} Mbit/s")),
            }))
            .collect();
        let current = bitrate_choice(&self.config);
        let default = Some(bitrate_choice(&Config::default()));
        let picked = self.pickers.choose(
            ui,
            &self.search,
            BITRATE,
            "settings_bitrate",
            &bitrates,
            current,
            default,
        );
        match picked {
            Some(Edit::Set(mbps)) => self.save(|config| apply_bitrate(config, mbps)),
            Some(Edit::Default) => self.save(|config| {
                config.automatic_bitrate = None;
                config.max_streaming_bitrate = None;
            }),
            None => {}
        }

        let current = self.config.prefer_hevc.unwrap_or(false);
        if let Some(edit) = toggle(ui, &self.search, HEVC, current, Some(false)) {
            self.save(|config| config.prefer_hevc = edit.or_unset());
        }
    }

    fn subtitles(&mut self, ui: &mut Ui, cx: &mut PageCx) {
        note(ui, &self.search, SUBTITLES_NOTE, None);
        if note(ui, &self.search, SUBTITLE_TRACKS_NOTE, Some("Languages")) {
            self.open(ui.ctx(), Section::Languages);
        }

        let current = self.config.subtitle_color.unwrap_or_default();
        let picked = self.pickers.choose(
            ui,
            &self.search,
            SUBTITLE_COLOR,
            "settings_subtitle_color",
            &SUBTITLE_COLORS,
            current,
            Some(SubtitleColor::default()),
        );
        if let Some(edit) = picked {
            self.save(|config| config.subtitle_color = edit.or_unset());
        }

        let current = self.config.subtitle_style.unwrap_or_default();
        let picked = self.pickers.choose(
            ui,
            &self.search,
            SUBTITLE_STYLE,
            "settings_subtitle_style",
            &SUBTITLE_STYLES,
            current,
            Some(SubtitleStyle::default()),
        );
        if let Some(edit) = picked {
            self.save(|config| config.subtitle_style = edit.or_unset());
        }

        // A size set in the player that isn't one of the steps here is listed too.
        let current = self.config.subtitle_scale.unwrap_or(1.);
        let scales: Vec<(f64, String)> = subtitle_scales(current)
            .into_iter()
            .map(|scale| (scale, format!("{:.0} %", scale * 100.)))
            .collect();
        let picked = self.pickers.choose(
            ui,
            &self.search,
            SUBTITLE_SIZE,
            "settings_subtitle_size",
            &scales,
            current,
            Some(1.),
        );
        if let Some(edit) = picked {
            let scale = edit.or_unset().unwrap_or(1.);
            self.save(|config| config.subtitle_scale = Some(scale).filter(|&scale| scale != 1.));
            cx.services.playback.set_subtitle_scale(scale);
        }
    }

    fn storage(&mut self, ui: &mut Ui, cx: &mut PageCx) {
        // Nothing to save for these two, so the row shows its folder instead of a default.
        let busy = self.job.is_some();
        if self.search.shows(&IMAGE_CACHE) {
            let images = config::cache_dir().join("images").display().to_string();
            let mut clear = false;
            row_with_folder(ui, IMAGE_CACHE, Some(&images), None, |ui| {
                clear = button::quiet(ui, "Clear", ROW_BUTTON_W, ready(busy), Focus::Tab).clicked();
            });
            if clear {
                self.clear_image_cache(cx);
            }
        }

        if self.search.shows(&LOGS) {
            let logs = config::log_dir().display().to_string();
            let mut open = false;
            row_with_folder(ui, LOGS, Some(&logs), None, |ui| {
                open = button::quiet(ui, "Open folder", ROW_BUTTON_W, ready(busy), Focus::Tab).clicked();
            });
            if open {
                self.open_logs(cx);
            }
        }
    }

    fn about(&mut self, ui: &mut Ui) {
        info(ui, &self.search, VERSION, env!("CARGO_PKG_VERSION"));

        let current = self.config.check_for_updates.unwrap_or(true);
        if let Some(edit) = toggle(ui, &self.search, CHECK_FOR_UPDATES, current, Some(true)) {
            self.save(|config| config.check_for_updates = edit.or_unset());
        }
    }

    fn account(&mut self, ui: &mut Ui, cx: &mut PageCx) {
        let session = &cx.services.session;
        // Who's signed in, as the sign-in screen lists accounts: one row for the user, the
        // server and its address, found by a search for any of the three.
        if [USER, SERVER, ADDRESS].iter().any(|row| self.search.shows(row)) {
            let server = session.server.server_name.clone().unwrap_or_else(|| "Jellyfin".into());
            let address = session.client.base_url().as_str().trim_end_matches('/').to_owned();
            let subtitle = format!("{server} · {address}");
            let label = format!("Signed in as {} on {subtitle}", session.user.name);
            let names = [session.user.name.as_str()];
            ui.add_space(ROW_PAD);
            AvatarRow::new(&names, &session.user.name, &subtitle, &label).show(ui, false);
            ui.add_space(ROW_PAD);
        }
        let version = session.server.version.clone().unwrap_or_else(|| "?".into());
        info(ui, &self.search, SERVER_VERSION, &version);
    }

    fn languages(&mut self, ui: &mut Ui, cx: &mut PageCx) {
        let Some(config) = self.server.loaded(ui) else {
            return;
        };
        note(ui, &self.search, LANGUAGES_NOTE, None);

        let current = config.audio_language_preference.as_deref().unwrap_or("");
        let languages = self.server.language_choices(current);
        let picked = self.pickers.choose(
            ui,
            &self.search,
            AUDIO_LANGUAGE,
            "settings_audio_language",
            &languages,
            current,
            None,
        );
        if let Some(Edit::Set(code)) = picked {
            let changes = UserConfigurationChanges {
                audio_language_preference: Some(code.to_string()),
                ..Default::default()
            };
            self.server.change(cx, changes);
        }

        if let Some(Edit::Set(on)) = toggle(
            ui,
            &self.search,
            PLAY_DEFAULT_AUDIO,
            config.play_default_audio_track,
            None,
        ) {
            let changes = UserConfigurationChanges {
                play_default_audio_track: Some(on),
                ..Default::default()
            };
            self.server.change(cx, changes);
        }

        if let Some(Edit::Set(on)) = toggle(ui, &self.search, REMEMBER_AUDIO, config.remember_audio_selections, None) {
            let changes = UserConfigurationChanges {
                remember_audio_selections: Some(on),
                ..Default::default()
            };
            self.server.change(cx, changes);
        }

        let current = config.subtitle_language_preference.as_deref().unwrap_or("");
        let languages = self.server.language_choices(current);
        let picked = self.pickers.choose(
            ui,
            &self.search,
            SUBTITLE_LANGUAGE,
            "settings_subtitle_language",
            &languages,
            current,
            None,
        );
        if let Some(Edit::Set(code)) = picked {
            let changes = UserConfigurationChanges {
                subtitle_language_preference: Some(code.to_string()),
                ..Default::default()
            };
            self.server.change(cx, changes);
        }

        let picked = self.pickers.choose(
            ui,
            &self.search,
            SUBTITLE_MODE,
            "settings_subtitle_mode",
            &SUBTITLE_MODES,
            config.subtitle_mode,
            None,
        );
        if let Some(Edit::Set(mode)) = picked {
            let changes = UserConfigurationChanges {
                subtitle_mode: Some(mode),
                ..Default::default()
            };
            self.server.change(cx, changes);
        }

        if let Some(Edit::Set(on)) = toggle(
            ui,
            &self.search,
            REMEMBER_SUBTITLES,
            config.remember_subtitle_selections,
            None,
        ) {
            let changes = UserConfigurationChanges {
                remember_subtitle_selections: Some(on),
                ..Default::default()
            };
            self.server.change(cx, changes);
        }
    }

    fn home(&mut self, ui: &mut Ui, cx: &mut PageCx) {
        if let Some(config) = self.server.loaded(ui)
            && let Some(Edit::Set(on)) = toggle(
                ui,
                &self.search,
                HIDE_PLAYED_IN_LATEST,
                config.hide_played_in_latest,
                None,
            )
        {
            let changes = UserConfigurationChanges {
                hide_played_in_latest: Some(on),
                ..Default::default()
            };
            self.server.change(cx, changes);
        }
        note(ui, &self.search, HOME_SECTIONS_NOTE, None);
    }
}

/// The user's configuration on the server, for the server's sections: read when the page
/// opens, shown with the page's changes on top, and saved in the background.
///
/// One save is out at a time. A change made meanwhile waits in `pending`, merged with any
/// others, and goes when the save is back: each save reads the whole configuration and writes
/// it back (`Client::update_user_configuration`), so two at once could undo each other.
#[derive(Default)]
struct ServerSettings {
    /// As the server last gave it, with the changes not yet saved applied.
    config: Option<UserConfiguration>,
    load: Option<Task<jellyfin::Result<UserConfiguration>>>,
    /// The load failed: the sections say so, with *Retry*.
    failed: bool,
    /// The languages to choose from, as (code, name) sorted by name. Empty until they load,
    /// and if they don't: the pickers then offer only "Any language" and the current code.
    languages: Vec<(String, String)>,
    cultures: Option<Task<jellyfin::Result<Vec<Culture>>>>,
    /// Whether the cultures were asked for, so a failure isn't asked again every frame.
    cultures_asked: bool,
    pending: UserConfigurationChanges,
    save: Option<Task<(UserConfigurationChanges, jellyfin::Result<()>)>>,
    /// Something was saved since the page opened, for the banner.
    saved: bool,
}

impl ServerSettings {
    /// Starts the loads the page needs and takes in what came back.
    fn tick(&mut self, cx: &mut PageCx) {
        if self.config.is_none() && self.load.is_none() && !self.failed {
            let client = cx.services.session.client.clone();
            self.load = Some(Task::spawn(cx.ctx, async move {
                Ok(client.current_user().await?.configuration.unwrap_or_default())
            }));
        }
        if !self.cultures_asked {
            self.cultures_asked = true;
            let client = cx.services.session.client.clone();
            self.cultures = Some(Task::spawn(cx.ctx, async move { client.cultures().await }));
        }

        if let Some(loaded) = self.load.as_mut().and_then(Task::poll) {
            self.load = None;
            match loaded {
                Ok(mut config) => {
                    // Changes made before a reload still show, and still go to the server.
                    self.pending.apply(&mut config);
                    self.config = Some(config);
                }
                Err(err) => {
                    tracing::warn!("couldn't read the user's settings: {err}");
                    self.failed = true;
                }
            }
        }
        if let Some(cultures) = self.cultures.as_mut().and_then(Task::poll) {
            self.cultures = None;
            match cultures {
                Ok(cultures) => self.languages = language_list(cultures),
                Err(err) => tracing::warn!("couldn't read the server's languages: {err}"),
            }
        }
        if let Some((changes, saved)) = self.save.as_mut().and_then(Task::poll) {
            self.save = None;
            match saved {
                Ok(()) => {
                    self.saved = true;
                    // Home's Latest rows change with it.
                    if changes.hide_played_in_latest.is_some() {
                        cx.services.user_data += 1;
                    }
                }
                Err(err) => {
                    tracing::warn!("couldn't save the user's settings: {err}");
                    cx.services.toast(Toast::error(
                        "Couldn't save your settings to the server. They're back as the server has them.",
                    ));
                    self.config = None;
                }
            }
        }
        self.flush(cx);
    }

    /// Shows `changes` at once and saves them when no other save is out.
    fn change(&mut self, cx: &mut PageCx, changes: UserConfigurationChanges) {
        if let Some(config) = &mut self.config {
            changes.apply(config);
        }
        self.pending.merge(changes);
        self.flush(cx);
    }

    fn flush(&mut self, cx: &mut PageCx) {
        if self.save.is_some() || self.pending.is_empty() || self.config.is_none() {
            return;
        }
        let changes = std::mem::take(&mut self.pending);
        let client = cx.services.session.client.clone();
        self.save = Some(Task::spawn(cx.ctx, async move {
            let saved = client.update_user_configuration(&changes).await;
            (changes, saved)
        }));
    }

    /// The configuration to draw the rows with, or, while it isn't there, a line saying why
    /// (with *Retry* after a failure).
    fn loaded(&mut self, ui: &mut Ui) -> Option<UserConfiguration> {
        if let Some(config) = &self.config {
            return Some(config.clone());
        }
        ui.add_space(ROW_PAD);
        if self.failed {
            surface::notice(
                ui,
                surface::Kind::Danger,
                "Couldn't load your settings from the server.",
            );
            ui.add_space(surface::GAP_FIELD);
            let retry = button::quiet(ui, "Retry", ROW_BUTTON_W, State::Ready, Focus::Tab).clicked();
            if retry {
                self.failed = false;
                self.cultures_asked = !self.languages.is_empty();
            }
        } else {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(14.).color(theme::tokens_of(ui).ink_muted));
                ui.label(label::muted(ui, "Loading your settings from the server…"));
            });
        }
        ui.add_space(ROW_PAD);
        None
    }

    /// A language picker's choices: none first, then the server's languages, and the current
    /// code by itself if the server's list doesn't have it (or didn't load).
    fn language_choices<'a>(&'a self, current: &'a str) -> Vec<(&'a str, &'a str)> {
        let mut choices: Vec<(&str, &str)> = std::iter::once(("", "Any language"))
            .chain(self.languages.iter().map(|(code, name)| (code.as_str(), name.as_str())))
            .collect();
        if !choices.iter().any(|(code, _)| *code == current) {
            choices.push((current, current));
        }
        choices
    }

    /// How saving is going, under the server's banner.
    fn status(&self) -> Option<String> {
        if self.save.is_some() || !self.pending.is_empty() {
            Some("Saving…".into())
        } else if self.saved {
            Some(format!("{} Saved to the server", ICON_CLOUD_DONE.codepoint))
        } else {
            None
        }
    }
}

/// The server's cultures as (code, name), sorted by name, one per code.
fn language_list(cultures: Vec<Culture>) -> Vec<(String, String)> {
    let mut languages: Vec<(String, String)> = cultures
        .into_iter()
        .filter_map(|culture| {
            let code = culture.three_letter_iso_language_name.filter(|code| !code.is_empty())?;
            Some((code, culture.display_name))
        })
        .collect();
    languages.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()).then(a.0.cmp(&b.0)));
    languages.dedup_by(|a, b| a.0 == b.0);
    languages
}

/// A store's name over its sections in the menu.
fn menu_heading(ui: &mut Ui, store: Store) {
    ui.horizontal(|ui| {
        ui.add_space(8.);
        ui.label(label::overline(ui, store.title()));
    });
    ui.add_space(HEADING_GAP);
}

/// A section in the menu: its icon and title across the menu's width, on an `inset_hover` pill
/// while it's open (with a `brand` bar at its left end and its icon in `brand`) or under the
/// pointer or the keyboard. While searching, `matches` is how many of its rows
/// the search finds, shown at the right end; one with none is greyed.
fn menu_entry(ui: &mut Ui, section: Section, selected: bool, matches: Option<usize>) -> egui::Response {
    let tokens = theme::tokens_of(ui);
    let height = (ui.spacing().interact_size.y + 8.).max(40.);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    response.widget_info(|| WidgetInfo::selected(Role::Button, true, selected, section.title()));
    let focused = ui.memory(|memory| memory.has_focus(response.id));
    let radius = egui::CornerRadius::same(tokens.radius_control);
    if selected || response.hovered() || focused {
        ui.painter().rect_filled(rect, radius, tokens.inset_hover);
    }
    if selected {
        let bar = Rect::from_center_size(
            pos2(rect.left() + MENU_BAR_W / 2. + 2., rect.center().y),
            vec2(MENU_BAR_W, rect.height() - 16.),
        );
        ui.painter().rect_filled(bar, MENU_BAR_W / 2., tokens.brand);
    }
    if focused {
        focus::ring(ui, rect, tokens.radius_control as f32, Place::Inside);
    }
    let color = match matches {
        Some(0) => tokens.ink_muted,
        _ => tokens.ink,
    };
    let icon_color = if selected { tokens.brand } else { color };
    let icon = text::icon(ui, section.icon().font_family());
    let icon_x = rect.left() + 14. + icon.size / 2.;
    ui.painter().text(
        pos2(icon_x, rect.center().y),
        Align2::CENTER_CENTER,
        section.icon().codepoint,
        icon.clone(),
        icon_color,
    );
    let font = text::font(ui, "base");
    let label_x = icon_x + icon.size / 2. + 10.;
    ui.painter().text(
        pos2(label_x, rect.center().y),
        Align2::LEFT_CENTER,
        section.title(),
        font,
        color,
    );
    if let Some(count) = matches.filter(|&count| count > 0) {
        ui.painter().text(
            pos2(rect.right() - 8., rect.center().y),
            Align2::RIGHT_CENTER,
            count.to_string(),
            text::font(ui, "xs"),
            tokens.ink_muted,
        );
    }
    response
}

/// Where a section's settings are kept, by its title: the store's icon and name on an inset pill.
fn store_tag(ui: &mut Ui, store: Store) {
    let tokens = theme::tokens_of(ui);
    let font = text::font(ui, "sm");
    egui::Frame::NONE
        .fill(tokens.inset)
        .corner_radius(egui::CornerRadius::same(u8::MAX))
        .inner_margin(egui::Margin::symmetric(10, 3))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 6.;
            ui.horizontal(|ui| {
                ui.label(store.icon().rich_text().size(font.size).color(tokens.ink_muted));
                ui.label(RichText::new(store.title()).font(font).color(tokens.ink_muted));
            });
        });
}

/// A section's rows on one surface spanning the column. The rows space themselves and draw the
/// hairlines between them ([`hairline`]), so the card adds no gap of its own between them.
fn card(ui: &mut Ui, contents: impl FnOnce(&mut Ui)) {
    surface::frame(ui)
        .inner_margin(egui::Margin::symmetric(CARD_PADDING[0], CARD_PADDING[1]))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.;
            contents(ui);
        });
}

/// The line between two rows of a card, across its width: drawn above every row but the
/// first, which is at the card's top with nothing yet above it.
fn hairline(ui: &mut Ui) {
    let y = ui.cursor().top();
    if y <= ui.min_rect().top() + 0.5 {
        return;
    }
    let stroke = egui::Stroke::new(1., theme::tokens_of(ui).surface_border);
    ui.painter().hline(ui.max_rect().x_range(), y, stroke);
}

/// One setting: its label over what it does at the left, and its control at the right,
/// centred on them. The label's tooltip names the key it saves and, for the device's, the
/// value while the key is unset. `reset` is the default's name for a device setting that's off
/// it: an undo button left of the control then puts it back. Returns whether that was clicked.
fn row(ui: &mut Ui, row: Row, reset: Option<&str>, control: impl FnOnce(&mut Ui)) -> bool {
    row_with_folder(ui, row, None, reset, control)
}

/// [`row`], with a folder under the description in monospace (the storage rows').
fn row_with_folder(
    ui: &mut Ui,
    row: Row,
    folder: Option<&str>,
    reset: Option<&str>,
    control: impl FnOnce(&mut Ui),
) -> bool {
    let tokens = theme::tokens_of(ui);
    hairline(ui);
    let full = ui.available_rect_before_wrap();
    let top = full.top() + ROW_PAD;
    // The text keeps clear of the widest control and its undo button, so every row of a card
    // wraps at the same width.
    let text_w = (full.width() - PICKER_W - RESET_W - 2. * SPACE.sm).max(120.);
    let text_rect = Rect::from_min_size(pos2(full.left(), top), vec2(text_w, f32::INFINITY));
    let text = ui
        .scope_builder(UiBuilder::new().max_rect(text_rect), |ui| {
            ui.spacing_mut().item_spacing.y = SPACE.xxs;
            let name = ui.label(RichText::new(row.label).font(text::font(ui, "base")).color(tokens.ink));
            if let Some(tip) = key_tooltip(&row) {
                name.on_hover_text(tip);
            }
            if !row.description.is_empty() {
                ui.label(label::muted(ui, row.description));
            }
            if let Some(folder) = folder {
                // Monospace, as a path: `RichText::monospace` would apply its family after the
                // table's font, so the font id is built directly.
                let mut font = text::font(ui, "xs");
                font.family = egui::FontFamily::Monospace;
                ui.label(RichText::new(folder).font(font).color(tokens.ink_muted));
            }
        })
        .response
        .rect;
    let controls = Rect::from_min_max(
        pos2(text.right() + SPACE.sm, top),
        pos2(full.right(), text.bottom().max(top + ui.spacing().interact_size.y)),
    );
    let mut reset_clicked = false;
    let drawn = ui
        .scope_builder(
            UiBuilder::new()
                .max_rect(controls)
                .layout(egui::Layout::right_to_left(Align::Center)),
            |ui| {
                control(ui);
                if let Some(default) = reset {
                    reset_clicked = reset_button(ui, default).clicked();
                }
            },
        )
        .response
        .rect;
    let bottom = text.bottom().max(drawn.bottom()) + ROW_PAD;
    ui.advance_cursor_after_rect(Rect::from_min_max(full.min, pos2(full.right(), bottom)));
    reset_clicked
}

/// What a row's label says under the pointer: the key it saves and where, and for the device's
/// the value that applies while the file leaves it out. `None` for a row that saves nothing.
fn key_tooltip(row: &Row) -> Option<String> {
    if row.key.is_empty() {
        return None;
    }
    Some(match row.section.store() {
        Store::Device if row.unset.is_empty() => format!("{} in the config file", row.key),
        Store::Device => format!("{} in the config file · unset: {}", row.key, row.unset),
        Store::Server => format!("{} in your Jellyfin profile", row.key),
    })
}

/// The undo button of a setting that's off its default, naming the default in its tooltip.
/// Tab stops on it after the setting's control.
fn reset_button(ui: &mut Ui, default: &str) -> egui::Response {
    let tooltip = format!("Back to the default: {default}");
    RoundButton::icon(ICON_UNDO, "Back to the default")
        .size(RESET_W, RESET_ICON)
        .tooltip(&tooltip)
        .show(ui)
}

/// A change from a device setting's control: a value picked, or back to the default.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Edit<T> {
    Set(T),
    Default,
}

impl<T> Edit<T> {
    /// What the config keeps: the value, or `None` to leave the key out, so the default
    /// applies.
    fn or_unset(self) -> Option<T> {
        match self {
            Edit::Set(value) => Some(value),
            Edit::Default => None,
        }
    }
}

/// A switch for a flag setting, when the search shows it: returns the edit, if there was one.
/// `default` is the device setting's default, for its undo button; `None` for the server's.
fn toggle(ui: &mut Ui, search: &Search, row: Row, value: bool, default: Option<bool>) -> Option<Edit<bool>> {
    if !search.shows(&row) {
        return None;
    }
    let reset = default
        .filter(|&default| default != value)
        .map(|default| if default { "On" } else { "Off" });
    let mut flipped = None;
    let reset_clicked = self::row(ui, row, reset, |ui| {
        let mut on = value;
        if toggle::switch(ui, &mut on, row.label).changed() {
            flipped = Some(Edit::Set(on));
        }
    });
    if reset_clicked { Some(Edit::Default) } else { flipped }
}

/// The pickers' keys: egui's focus (Tab) puts it on a select (`widgets::select`), and Left and
/// Right there step its value to the previous or next choice, stopping at the ends, without
/// opening its list (Enter or Space does that). The step comes back from [`Pickers::choose`] as
/// the same value a pick from the list gives, so the page saves it the same way.
#[derive(Default)]
struct Pickers {
    /// The picker egui had the focus on at the end of the last frame, and the `Id` egui knows
    /// it by (the page knows it by its id salt).
    focused: Option<(&'static str, egui::Id)>,
    /// The Left (-1) or Right (1) taken for that picker this frame.
    step: Option<i32>,
}

impl Pickers {
    /// Takes Left and Right out of egui's input while a picker has the focus, so nothing else
    /// sees them, and keeps which way to step. Judged from the end of the last frame, as egui
    /// judges its own focus.
    fn begin_frame(&mut self, ctx: &egui::Context) {
        self.step = None;
        let Some(focused_widget) = ctx.memory(|memory| memory.focused()) else {
            self.focused = None;
            return;
        };
        if self.focused.is_none_or(|(_, id)| id != focused_widget) {
            self.focused = None;
            return;
        }
        // A list is open (opened with the pointer): the keys are egui's until it closes.
        if egui::Popup::is_any_open(ctx) {
            return;
        }
        for (key, step) in [(egui::Key::ArrowLeft, -1), (egui::Key::ArrowRight, 1)] {
            if crate::keys::take_exact(ctx, egui::Modifiers::NONE, key) {
                self.step = Some(step);
            }
        }
    }

    /// A picker over `options`, with `current` selected, when the search shows it: returns the
    /// choice when one was picked in the list or stepped to with Left or Right, or
    /// [`Edit::Default`] from the undo button. `default` is the device setting's default, for
    /// that button; `None` for the server's.
    #[allow(clippy::too_many_arguments)]
    fn choose<T: Copy + PartialEq, L: AsRef<str>>(
        &mut self,
        ui: &mut Ui,
        search: &Search,
        row: Row,
        id: &'static str,
        options: &[(T, L)],
        current: T,
        default: Option<T>,
    ) -> Option<Edit<T>> {
        if !search.shows(&row) {
            return None;
        }
        let index = options.iter().position(|(value, _)| *value == current);
        let labels: Vec<&str> = options.iter().map(|(_, label)| label.as_ref()).collect();
        let mut picked = None;
        // Left or Right on the focused picker steps it, as picking it from the list would.
        if self.focused.is_some_and(|(focused, _)| focused == id)
            && let Some(step) = self.step.take()
        {
            let values: Vec<T> = options.iter().map(|(value, _)| *value).collect();
            picked = stepped(&values, current, step);
        }
        let reset = default
            .filter(|default| *default != current)
            .and_then(|default| options.iter().find(|(value, _)| *value == default))
            .map(|(_, label)| label.as_ref());
        let mut focused = None;
        let reset_clicked = self::row(ui, row, reset, |ui| {
            // A value not among the choices (a bitrate set by hand) shows the first one.
            let menu = ui.make_persistent_id(id);
            let (response, chosen) = select::select(ui, menu, PICKER_W, row.label, &labels, index.unwrap_or(0), true);
            if let Some(chosen) = chosen {
                picked = Some(options[chosen].0);
            }
            // egui's focus is what Tab moves. `Response::has_focus` also wants the window
            // itself to be focused, which never happened on the window-manager-less test
            // display (and adds nothing for a real keyboard: keys need the focus anyway).
            if ui.ctx().memory(|memory| memory.has_focus(response.id)) {
                focused = Some(response.id);
                // Left and Right are the picker's own while it has the focus, not egui's focus
                // movement to the widget beside it. The lock holds from the frame after the
                // picker gains the focus, as the search box's does (`views/shell.rs`).
                ui.memory_mut(|memory| {
                    memory.set_focus_lock_filter(
                        response.id,
                        egui::EventFilter {
                            tab: false,
                            horizontal_arrows: true,
                            vertical_arrows: false,
                            escape: false,
                        },
                    );
                });
            }
        });
        if let Some(focused_id) = focused {
            self.focused = Some((id, focused_id));
        }
        if reset_clicked {
            return Some(Edit::Default);
        }
        picked.map(Edit::Set)
    }
}

/// The choice one step (Left is -1, Right is 1) from `current` in `options`, stopping at either
/// end; `None` when it can't move, or when `current` isn't one of the choices (a bitrate set by
/// hand: the picker shows the first one, as `main`'s select did, and Left and Right stay put).
fn stepped<T: Copy + PartialEq>(options: &[T], current: T, step: i32) -> Option<T> {
    let index = options.iter().position(|value| *value == current)?;
    let next = (index as i32 + step).clamp(0, options.len() as i32 - 1) as usize;
    (next != index).then(|| options[next])
}

/// A row that's only information, when the search shows it: there's nothing to change, so no
/// control and no key.
fn info(ui: &mut Ui, search: &Search, row: Row, value: &str) {
    if !search.shows(&row) {
        return;
    }
    self::row(ui, row, None, |ui| {
        ui.label(RichText::new(value).color(theme::tokens_of(ui).ink_muted));
    });
}

/// A paragraph in a group that isn't a setting, when the search shows it, with a link to the
/// section `link` names after it. Returns whether the link was clicked.
fn note(ui: &mut Ui, search: &Search, row: Row, link: Option<&str>) -> bool {
    if !search.shows(&row) {
        return false;
    }
    hairline(ui);
    ui.add_space(ROW_PAD);
    let font = text::font(ui, "sm");
    let description = label::muted(ui, row.description);
    let mut clicked = false;
    match link {
        // Wrapped with the link at the end of its last line. A wrapped row's lines are a
        // control's height apart, so a note without a link is a plain label.
        Some(link) => {
            ui.horizontal_wrapped(|ui| {
                ui.label(description);
                let link = RichText::new(format!("{link} ›"))
                    .font(font)
                    .color(theme::tokens_of(ui).brand);
                clicked = ui.link(link).clicked();
            });
        }
        None => {
            ui.label(description);
        }
    }
    ui.add_space(ROW_PAD);
    clicked
}

/// A row button's state: disabled while a storage job runs.
fn ready(busy: bool) -> State {
    if busy { State::Disabled } else { State::Ready }
}

/// The bitrate picker's current choice, from the config: [`AUTOMATIC`] when it measures the
/// connection, otherwise the cap in Mbit/s (`0` for none).
fn bitrate_choice(config: &Config) -> i32 {
    match config.automatic_bitrate {
        Some(true) => AUTOMATIC,
        _ => config.max_streaming_bitrate.map_or(0, |bps| (bps / 1_000_000) as i32),
    }
}

/// The config a bitrate choice writes, as `main` saved it: *Automatic* turns measuring on and
/// leaves the saved cap alone (the measurement replaces it, and the cap is what turning
/// *Automatic* off goes back to), any other choice turns measuring off and saves the cap in bits
/// per second, `None` for no limit.
fn apply_bitrate(config: &mut Config, mbps: i32) {
    config.automatic_bitrate = Some(mbps == AUTOMATIC);
    if mbps != AUTOMATIC {
        config.max_streaming_bitrate = (mbps > 0).then(|| mbps as u64 * 1_000_000);
    }
}

/// The sizes the picker offers: the steps, plus the size set in the player when it isn't one.
fn subtitle_scales(current: f64) -> Vec<f64> {
    let mut scales = SUBTITLE_SCALES.to_vec();
    if !scales.contains(&current) {
        scales.push(current);
        scales.sort_by(f64::total_cmp);
    }
    scales
}

/// Opens a folder in the desktop's file manager. Not the `opener` crate: it's one program per
/// OS, and this is the only place the app opens anything on the desktop (`main`'s
/// `open_with_system`). Waits for the launcher, which exits once the file manager has the
/// folder, so a missing one is reported.
fn open_folder(path: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    let program = "explorer";
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let program = "xdg-open";
    let mut child = std::process::Command::new(program).arg(path).spawn()?;
    child.wait().map(|_| ())
}

#[cfg(test)]
mod tests {
    use jellyfin::models::Culture;

    use crate::config::{Config, ThemePreference};

    use super::{
        AUTOMATIC, BITRATES_MBPS, ROWS, SUBTITLE_SCALES, Search, Section, Store, THEMES, apply_bitrate, bitrate_choice,
        language_list, stepped, subtitle_scales,
    };

    #[test]
    fn every_bitrate_choice_reads_back() {
        for mbps in std::iter::once(AUTOMATIC).chain(BITRATES_MBPS) {
            let mut config = Config::default();
            apply_bitrate(&mut config, mbps);
            assert_eq!(bitrate_choice(&config), mbps, "{mbps} Mbit/s");
        }
    }

    #[test]
    fn automatic_keeps_the_cap_it_replaces() {
        let mut config = Config {
            max_streaming_bitrate: Some(20_000_000),
            ..Default::default()
        };
        apply_bitrate(&mut config, AUTOMATIC);
        assert_eq!(config.automatic_bitrate, Some(true));
        assert_eq!(config.max_streaming_bitrate, Some(20_000_000));
    }

    #[test]
    fn a_cap_turns_measuring_off() {
        let mut config = Config {
            automatic_bitrate: Some(true),
            ..Default::default()
        };
        apply_bitrate(&mut config, 8);
        assert_eq!(config.automatic_bitrate, Some(false));
        assert_eq!(config.max_streaming_bitrate, Some(8_000_000));
        apply_bitrate(&mut config, 0);
        assert_eq!(config.max_streaming_bitrate, None);
    }

    #[test]
    fn left_and_right_step_a_picker_and_stop_at_the_ends() {
        let themes: Vec<ThemePreference> = THEMES.iter().map(|(theme, _)| *theme).collect();
        assert_eq!(
            stepped(&themes, ThemePreference::System, 1),
            Some(ThemePreference::Light)
        );
        assert_eq!(
            stepped(&themes, ThemePreference::Light, -1),
            Some(ThemePreference::System)
        );
        assert_eq!(stepped(&themes, ThemePreference::Dark, 1), None);
        assert_eq!(stepped(&themes, ThemePreference::System, -1), None);
        // A value that isn't one of the choices doesn't step: the picker shows the first one.
        assert_eq!(stepped(&[0, 120], 15, 1), None);
    }

    #[test]
    fn the_players_subtitle_size_joins_the_steps() {
        assert_eq!(subtitle_scales(1.), SUBTITLE_SCALES.to_vec());
        assert_eq!(subtitle_scales(1.15), [0.75, 1., 1.15, 1.25, 1.5, 1.75, 2.]);
    }

    /// The page says what each key means when it's unset; this is that value taken from the
    /// config itself, so the two can't drift apart.
    #[test]
    fn the_defaults_the_page_shows() {
        let config = Config::default();
        assert_eq!(config.theme, ThemePreference::Dark);
        assert_eq!(config.text_size, None); // the system's
        assert_eq!(config.animations, None); // on
        assert_eq!(config.autoplay_next_episode, None); // on
        assert_eq!(config.auto_skip_intro, None); // off
        assert_eq!(config.prefer_hevc, None); // off
        assert_eq!(config.check_for_updates, None); // on
        assert_eq!(config.subtitle_color, None); // white
        assert_eq!(config.subtitle_style, None); // outline
        assert_eq!(config.subtitle_scale, None); // 100 %
        assert_eq!(bitrate_choice(&config), 0); // no limit
    }

    fn search(text: &str) -> Search {
        Search { text: text.into() }
    }

    /// The sections a search finds, in the menu's order.
    fn found(text: &str) -> Vec<Section> {
        let search = search(text);
        Section::ALL
            .into_iter()
            .filter(|&section| search.matches(section) > 0)
            .collect()
    }

    #[test]
    fn every_section_has_rows_for_the_search() {
        for section in Section::ALL {
            assert!(ROWS.iter().any(|row| row.section == section), "{section:?}");
        }
    }

    /// A word finds rows by their label, description or key, in both stores, ignoring case;
    /// every word has to be there.
    #[test]
    fn the_search_finds_rows_on_both_sides() {
        assert_eq!(
            found("SUBTITLE"),
            [Section::Playback, Section::Subtitles, Section::Languages]
        );
        assert_eq!(found("bitrate"), [Section::Playback]);
        assert_eq!(found("subtitle mode"), [Section::Languages]);
        assert_eq!(found("hevc transcodes"), [Section::Playback]);
        // A config key, and a server field.
        assert_eq!(found("auto_skip_intro"), [Section::Playback]);
        assert_eq!(found("hideplayedinlatest"), [Section::Home]);
        assert!(found("nothing like this").is_empty());
    }

    /// A store's or a section's name finds all of its rows.
    #[test]
    fn the_search_finds_a_whole_section_or_store() {
        let storage = search("storage");
        assert_eq!(found("storage"), [Section::Storage]);
        assert!(
            ROWS.iter()
                .filter(|row| row.section == Section::Storage)
                .all(|row| storage.shows(row))
        );
        let server: Vec<Section> = Section::ALL
            .into_iter()
            .filter(|section| section.store() == Store::Server)
            .collect();
        assert_eq!(found("jellyfin server"), server);
        assert!(!search("").active() && !search("  ").active());
    }

    #[test]
    fn languages_are_sorted_by_name_once_each() {
        let culture = |name: &str, code: Option<&str>| Culture {
            display_name: name.into(),
            three_letter_iso_language_name: code.map(Into::into),
        };
        let languages = language_list(vec![
            culture("Japanese", Some("jpn")),
            culture("english", Some("eng")),
            culture("Unknown", None),
            culture("Japanese (Japan)", Some("jpn")),
            culture("Empty", Some("")),
        ]);
        assert_eq!(
            languages,
            [
                ("eng".to_string(), "english".to_string()),
                ("jpn".to_string(), "Japanese".to_string())
            ]
        );
    }
}
