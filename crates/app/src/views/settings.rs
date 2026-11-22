//! Settings: a menu of sections on the left, split by where their settings are kept, and every
//! section on the right, one after another in a scrolling column. A menu entry scrolls its
//! section to the top, and the menu marks the section at the top as the column scrolls. A
//! section is its title, a tag naming where its settings are kept, and one card of rows split
//! by hairlines: each row's label over what it does, its control at the right, and the key it
//! saves in the label's tooltip.
//!
//! - **This device** (appearance, playback, subtitles, storage, about) is the config file: one
//!   per computer, shared by every account signed in on it. Every option keeps its config key
//!   and value (a change loads the config fresh, changes it and saves it, so it can't write over
//!   what playback or the window saved meanwhile), its choices in their order, the default that
//!   applies while the file leaves the key out, and what the change takes effect on: the theme,
//!   text size and animations apply at once through `AppEvent::AppearanceChanged`, the subtitle
//!   size goes to the playback manager while a session runs, and the rest are read when what
//!   they configure next starts (a playback session, the player page, the update check at
//!   launch).
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
//! On a phone in portrait (`layout::Size::Compact`) the page is a list of the sections by
//! store instead, each opening on a page of its own ([`Route::SettingsSection`]); the top bar's
//! Search button still filters every row, the results replacing the list while there's a term.
//!
//! The page isn't on the browse cursor: [`Page::scope`] is `None`, so egui's own focus handles
//! Tab (the menu, then the sections' controls, scrolling each into view), Space or Enter on a
//! menu entry, the arrows in a picker and Space on a switch, and Esc still goes back.

use std::path::Path;

use egui::{Align, Align2, Rect, RichText, Role, Sense, Ui, UiBuilder, WidgetInfo, pos2, vec2};
use egui_material_icons::MaterialIcon;
use egui_material_icons::icons::{
    ICON_ACCOUNT_CIRCLE, ICON_CHEVRON_RIGHT, ICON_CLOUD_DONE, ICON_COMPUTER, ICON_DNS, ICON_DOWNLOAD, ICON_HOME,
    ICON_INFO, ICON_PALETTE, ICON_PLAY_CIRCLE, ICON_STORAGE, ICON_SUBTITLES, ICON_TRANSLATE, ICON_UNDO,
};
use jellyfin::models::{Culture, SubtitleMode, UserConfiguration, UserConfigurationChanges};

use crate::config::{self, AndroidPlayer, Config, SubtitleColor, SubtitleStyle, ThemePreference};
use crate::i18n::{self, tr};
use crate::nav::{NavRequest, Route};
use crate::playback::Adjustments;
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
use crate::widgets::list_row;
use crate::widgets::round_button::RoundButton;
use crate::widgets::{backdrop, pill, select, surface, toggle};

/// The theme choices, in the order the picker lists them.
fn themes() -> [(ThemePreference, String); 3] {
    [
        (ThemePreference::System, tr!("settings-theme-system")),
        (ThemePreference::Light, tr!("settings-theme-light")),
        (ThemePreference::Dark, tr!("settings-theme-dark")),
    ]
}

fn subtitle_colors() -> [(SubtitleColor, String); 2] {
    [
        (SubtitleColor::White, tr!("settings-color-white")),
        (SubtitleColor::Yellow, tr!("settings-color-yellow")),
    ]
}

/// The players' names are their own, so they read the same in every language.
const PLAYERS: [(AndroidPlayer, &str); 2] = [(AndroidPlayer::Media3, "Media3"), (AndroidPlayer::Mpv, "mpv")];

fn subtitle_styles() -> [(SubtitleStyle, String); 2] {
    [
        (SubtitleStyle::Outline, tr!("settings-style-outline")),
        (SubtitleStyle::Box, tr!("settings-style-box")),
    ]
}

/// The server's subtitle modes, in the web client's order.
fn subtitle_modes() -> [(SubtitleMode, String); 5] {
    [
        (SubtitleMode::Default, tr!("settings-mode-default")),
        (SubtitleMode::Smart, tr!("settings-mode-smart")),
        (SubtitleMode::OnlyForced, tr!("settings-mode-only-forced")),
        (SubtitleMode::Always, tr!("settings-mode-always")),
        (SubtitleMode::Off, tr!("settings-mode-none")),
    ]
}

/// Subtitle sizes offered here; the player's menu sets any size in 10 % steps.
const SUBTITLE_SCALES: [f64; 6] = [0.75, 1., 1.25, 1.5, 1.75, 2.];

/// Subtitle positions offered here, as mpv's `sub-pos`: the bottom, then higher up the picture.
/// The player's menu sets any position in 5 % steps.
const SUBTITLE_POSITIONS: [u8; 10] = [100, 95, 90, 85, 80, 70, 60, 50, 40, 30];

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
/// The pickers' width beside their label (220 px).
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

    fn title(self) -> String {
        match self {
            Store::Device => tr!("settings-store-device"),
            Store::Server => tr!("settings-store-server"),
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
    /// Android only ([`Section::shown`]).
    Downloads,
    Storage,
    About,
    Account,
    Languages,
    Home,
}

impl Section {
    /// In the menu's order: the device's, then the server's. Not all of them are on every
    /// platform: [`Section::shown`] has the ones this one has.
    pub(crate) const ALL: [Section; 9] = [
        Section::Appearance,
        Section::Playback,
        Section::Subtitles,
        Section::Downloads,
        Section::Storage,
        Section::About,
        Section::Account,
        Section::Languages,
        Section::Home,
    ];

    /// The sections this platform has, in the menu's order: *Downloads* holds only Android's
    /// settings.
    pub(crate) fn shown() -> impl Iterator<Item = Section> {
        Self::ALL
            .into_iter()
            .filter(|&section| section != Section::Downloads || cfg!(target_os = "android"))
    }

    fn store(self) -> Store {
        match self {
            Section::Appearance
            | Section::Playback
            | Section::Subtitles
            | Section::Downloads
            | Section::Storage
            | Section::About => Store::Device,
            Section::Account | Section::Languages | Section::Home => Store::Server,
        }
    }

    fn title(self) -> String {
        match self {
            Section::Appearance => tr!("settings-section-appearance"),
            Section::Playback => tr!("settings-section-playback"),
            Section::Subtitles => tr!("settings-section-subtitles"),
            Section::Downloads => tr!("settings-section-downloads"),
            Section::Storage => tr!("settings-section-storage"),
            Section::About => tr!("settings-section-about"),
            Section::Account => tr!("settings-section-account"),
            Section::Languages => tr!("settings-section-languages"),
            Section::Home => tr!("settings-section-home"),
        }
    }

    /// What the section has, under its title in a phone's list of sections.
    fn summary(self) -> String {
        match self {
            Section::Appearance => tr!("settings-section-appearance-summary"),
            Section::Playback => tr!("settings-section-playback-summary"),
            Section::Subtitles => tr!("settings-section-subtitles-summary"),
            Section::Downloads => tr!("settings-section-downloads-summary"),
            Section::Storage => tr!("settings-section-storage-summary"),
            Section::About => tr!("settings-section-about-summary"),
            Section::Account => tr!("settings-section-account-summary"),
            Section::Languages => tr!("settings-section-languages-summary"),
            Section::Home => tr!("settings-section-home-summary"),
        }
    }

    fn icon(self) -> MaterialIcon {
        match self {
            Section::Appearance => ICON_PALETTE,
            Section::Playback => ICON_PLAY_CIRCLE,
            Section::Subtitles => ICON_SUBTITLES,
            Section::Downloads => ICON_DOWNLOAD,
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
///
/// The text is in the interface's language, so a row is made when it's drawn: one kept from an
/// earlier frame would stay in the language it was made in.
struct Row {
    section: Section,
    label: String,
    description: String,
    /// The config key, the server's field, or what the row is about when nothing is saved.
    key: &'static str,
    /// The value while the key is unset; empty for a row with nothing to save, and for the
    /// server's, which always has a value.
    unset: String,
}

fn language_row() -> Row {
    Row {
        section: Section::Appearance,
        label: tr!("settings-language"),
        description: tr!("settings-language-description"),
        key: "language",
        unset: tr!("settings-language-unset"),
    }
}
fn theme_row() -> Row {
    Row {
        section: Section::Appearance,
        label: tr!("settings-theme"),
        description: tr!("settings-theme-description"),
        key: "theme",
        unset: tr!("settings-theme-unset"),
    }
}
fn text_size_row() -> Row {
    Row {
        section: Section::Appearance,
        label: tr!("settings-text-size"),
        description: tr!("settings-text-size-description"),
        key: "text_size",
        unset: tr!("settings-text-size-unset"),
    }
}
fn animations_row() -> Row {
    Row {
        section: Section::Appearance,
        label: tr!("settings-animations"),
        description: tr!("settings-animations-description"),
        key: "animations",
        unset: tr!("settings-animations-unset"),
    }
}

fn spoilers_row() -> Row {
    Row {
        section: Section::Appearance,
        label: tr!("settings-spoilers"),
        description: tr!("settings-spoilers-description"),
        key: "hide_spoilers",
        unset: tr!("settings-spoilers-unset"),
    }
}

fn tracks_note_row() -> Row {
    note_row(Section::Playback, tr!("settings-tracks-note"))
}
fn autoplay_row() -> Row {
    Row {
        section: Section::Playback,
        label: tr!("settings-autoplay"),
        description: tr!("settings-autoplay-description"),
        key: "autoplay_next_episode",
        unset: tr!("settings-autoplay-unset"),
    }
}
fn skip_intro_row() -> Row {
    Row {
        section: Section::Playback,
        label: tr!("settings-skip-intro"),
        description: tr!("settings-skip-intro-description"),
        key: "auto_skip_intro",
        unset: tr!("settings-skip-intro-unset"),
    }
}
fn bitrate_row() -> Row {
    Row {
        section: Section::Playback,
        label: tr!("settings-bitrate"),
        description: tr!("settings-bitrate-description"),
        key: "max_streaming_bitrate",
        unset: tr!("settings-bitrate-unset"),
    }
}
fn cast_bitrate_row() -> Row {
    Row {
        section: Section::Playback,
        label: tr!("settings-cast-bitrate"),
        description: tr!("settings-cast-bitrate-description"),
        key: "cast_max_bitrate",
        unset: tr!("settings-cast-bitrate-unset"),
    }
}
fn player_row() -> Row {
    Row {
        section: Section::Playback,
        label: tr!("settings-player"),
        description: tr!("settings-player-description"),
        key: "android_player",
        unset: tr!("settings-player-unset"),
    }
}
fn hevc_row() -> Row {
    Row {
        section: Section::Playback,
        label: tr!("settings-hevc"),
        description: tr!("settings-hevc-description"),
        key: "prefer_hevc",
        unset: tr!("settings-hevc-unset"),
    }
}

fn subtitles_note_row() -> Row {
    note_row(Section::Subtitles, tr!("settings-subtitles-note"))
}
fn subtitle_tracks_note_row() -> Row {
    note_row(Section::Subtitles, tr!("settings-subtitle-tracks-note"))
}
fn subtitle_color_row() -> Row {
    Row {
        section: Section::Subtitles,
        label: tr!("settings-subtitle-color"),
        description: tr!("settings-subtitle-color-description"),
        key: "subtitle_color",
        unset: tr!("settings-subtitle-color-unset"),
    }
}
fn subtitle_style_row() -> Row {
    Row {
        section: Section::Subtitles,
        label: tr!("settings-subtitle-style"),
        description: tr!("settings-subtitle-style-description"),
        key: "subtitle_style",
        unset: tr!("settings-subtitle-style-unset"),
    }
}
fn subtitle_size_row() -> Row {
    Row {
        section: Section::Subtitles,
        label: tr!("settings-subtitle-size"),
        description: tr!("settings-subtitle-size-description"),
        key: "subtitle_scale",
        unset: tr!("settings-subtitle-size-unset"),
    }
}

fn subtitle_position_row() -> Row {
    Row {
        section: Section::Subtitles,
        label: tr!("settings-subtitle-position"),
        description: tr!("settings-subtitle-position-description"),
        key: "subtitle_position",
        unset: tr!("settings-subtitle-position-unset"),
    }
}

fn wifi_only_row() -> Row {
    Row {
        section: Section::Downloads,
        label: tr!("settings-wifi-only"),
        description: tr!("settings-wifi-only-description"),
        key: "download_wifi_only",
        unset: tr!("settings-wifi-only-unset"),
    }
}
/// Its smaller line shows the folder, as the storage rows do.
fn downloads_folder_row() -> Row {
    Row {
        section: Section::Downloads,
        label: tr!("settings-downloads-folder"),
        description: tr!("settings-downloads-folder-description"),
        key: "",
        unset: String::new(),
    }
}

/// The storage rows' smaller line shows the folder, which [`SettingsPage::storage`] fills in.
fn image_cache_row() -> Row {
    Row {
        section: Section::Storage,
        label: tr!("settings-image-cache"),
        description: tr!("settings-image-cache-description"),
        key: "",
        unset: String::new(),
    }
}
fn logs_row() -> Row {
    Row {
        section: Section::Storage,
        label: tr!("settings-logs"),
        description: tr!("settings-logs-description"),
        key: "",
        unset: String::new(),
    }
}

fn version_row() -> Row {
    info_row(Section::About, tr!("settings-version"))
}
fn check_for_updates_row() -> Row {
    Row {
        section: Section::About,
        label: tr!("settings-check-for-updates"),
        description: tr!("settings-check-for-updates-description"),
        key: "check_for_updates",
        unset: tr!("settings-check-for-updates-unset"),
    }
}

fn user_row() -> Row {
    info_row(Section::Account, tr!("settings-user"))
}
fn server_row() -> Row {
    info_row(Section::Account, tr!("settings-server"))
}
fn address_row() -> Row {
    info_row(Section::Account, tr!("settings-address"))
}
fn server_version_row() -> Row {
    info_row(Section::Account, tr!("settings-server-version"))
}

fn languages_note_row() -> Row {
    note_row(Section::Languages, tr!("settings-languages-note"))
}
fn audio_language_row() -> Row {
    Row {
        section: Section::Languages,
        label: tr!("settings-audio-language"),
        description: tr!("settings-audio-language-description"),
        key: "AudioLanguagePreference",
        unset: String::new(),
    }
}
fn play_default_audio_row() -> Row {
    Row {
        section: Section::Languages,
        label: tr!("settings-play-default-audio"),
        description: tr!("settings-play-default-audio-description"),
        key: "PlayDefaultAudioTrack",
        unset: String::new(),
    }
}
fn remember_audio_row() -> Row {
    Row {
        section: Section::Languages,
        label: tr!("settings-remember-audio"),
        description: tr!("settings-remember-audio-description"),
        key: "RememberAudioSelections",
        unset: String::new(),
    }
}
fn subtitle_language_row() -> Row {
    Row {
        section: Section::Languages,
        label: tr!("settings-subtitle-language"),
        description: tr!("settings-subtitle-language-description"),
        key: "SubtitleLanguagePreference",
        unset: String::new(),
    }
}
fn subtitle_mode_row() -> Row {
    Row {
        section: Section::Languages,
        label: tr!("settings-subtitle-mode"),
        description: tr!("settings-subtitle-mode-description"),
        key: "SubtitleMode",
        unset: String::new(),
    }
}
fn remember_subtitles_row() -> Row {
    Row {
        section: Section::Languages,
        label: tr!("settings-remember-subtitles"),
        description: tr!("settings-remember-subtitles-description"),
        key: "RememberSubtitleSelections",
        unset: String::new(),
    }
}

fn hide_played_in_latest_row() -> Row {
    Row {
        section: Section::Home,
        label: tr!("settings-hide-played-in-latest"),
        description: tr!("settings-hide-played-in-latest-description"),
        key: "HidePlayedInLatest",
        unset: String::new(),
    }
}
// No settings of our own for these: the web client's profile decides.
fn home_sections_note_row() -> Row {
    note_row(Section::Home, tr!("settings-home-sections-note"))
}

/// Every row, for the search. A row left out of here still shows in its section, but a search
/// would only find it through another row of the same section.
fn all_rows() -> Vec<Row> {
    vec![
        theme_row(),
        text_size_row(),
        animations_row(),
        spoilers_row(),
        language_row(),
        tracks_note_row(),
        autoplay_row(),
        skip_intro_row(),
        bitrate_row(),
        cast_bitrate_row(),
        hevc_row(),
        player_row(),
        subtitles_note_row(),
        subtitle_tracks_note_row(),
        subtitle_color_row(),
        subtitle_style_row(),
        subtitle_size_row(),
        subtitle_position_row(),
        wifi_only_row(),
        downloads_folder_row(),
        image_cache_row(),
        logs_row(),
        version_row(),
        check_for_updates_row(),
        user_row(),
        server_row(),
        address_row(),
        server_version_row(),
        languages_note_row(),
        audio_language_row(),
        play_default_audio_row(),
        remember_audio_row(),
        subtitle_language_row(),
        subtitle_mode_row(),
        remember_subtitles_row(),
        hide_played_in_latest_row(),
        home_sections_note_row(),
        // The section headings' own words find the section through these.
        section_row(Section::Storage),
        section_row(Section::Account),
        section_row(Section::Home),
    ]
}

/// A note's row: a description with nothing to change, and no label.
fn note_row(section: Section, description: String) -> Row {
    Row {
        section,
        label: String::new(),
        description,
        key: "",
        unset: String::new(),
    }
}

/// A row that's only information: a label and a value beside it.
fn info_row(section: Section, label: String) -> Row {
    Row {
        section,
        label,
        description: String::new(),
        key: "",
        unset: String::new(),
    }
}

/// A row that's never drawn, so that a search for a section's title or store finds it even
/// when no row's own words do. The search reads the section's title for every row anyway.
fn section_row(section: Section) -> Row {
    Row {
        section,
        label: String::new(),
        description: String::new(),
        key: "",
        unset: String::new(),
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
            row.label.clone(),
            row.description.clone(),
            row.key.to_owned(),
        ]
        .join(" ")
        .to_lowercase();
        self.text
            .to_lowercase()
            .split_whitespace()
            .all(|word| haystack.contains(word))
    }

    /// How many of `section`'s rows, of `rows`, the search finds.
    fn matches(&self, section: Section, rows: &[Row]) -> usize {
        rows.iter()
            .filter(|row| row.section == section && self.shows(row))
            .count()
    }
}

pub struct SettingsPage {
    /// The config as this page last read or wrote it, for the controls' values. A change reads
    /// the file again rather than writing this copy back.
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
    /// The one section this page shows, when it's a phone's page for it.
    only: Option<Section>,
    /// Whether the page drew in the compact layout, the sections each a page of their own.
    compact: bool,
    /// A section a note's link asked for, opened as its own page after the frame when the
    /// sections are pages of their own.
    push: Option<Section>,
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
            only: None,
            compact: false,
            push: None,
        }
    }

    /// A phone's page for one section.
    pub fn for_section(section: Section) -> Self {
        Self {
            only: Some(section),
            section,
            ..Self::new()
        }
    }

    /// The page's title, or the one section's, in the interface's language.
    fn title_for(only: Option<Section>) -> String {
        only.map_or_else(|| tr!("settings-title"), Section::title)
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

    /// Scrolls to `section`, from the menu or a note's link, leaving the search; or opens its
    /// page when the sections are pages of their own.
    fn open(&mut self, ctx: &egui::Context, section: Section) {
        if self.only.is_some() || self.compact {
            self.push = Some(section);
            return;
        }
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
            Job::Cleared(Ok(())) => cx.services.toast(Toast::success(tr!("settings-image-cache-cleared"))),
            Job::Cleared(Err(err)) => {
                tracing::warn!("clearing the image cache failed: {err}");
                cx.services.toast(Toast::error(tr!("settings-image-cache-failed")));
            }
            Job::Opened(Ok(())) => {}
            Job::Opened(Err(err)) => {
                tracing::warn!("couldn't open {}: {err}", config::log_dir().display());
                let folder = config::log_dir().display().to_string();
                cx.services
                    .toast(Toast::error(tr!("settings-logs-folder-failed", path = folder)));
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
    fn title(&self) -> String {
        Self::title_for(self.only)
    }

    /// A section's own page has nothing to search but what it shows.
    fn own_search(&self) -> Option<&str> {
        self.only.is_none().then_some(self.search.text.as_str())
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
        self.compact = crate::layout::compact(ui.ctx());
        // A section's own page, and a phone's: the list of sections, or what a search finds in
        // them.
        if self.only.is_some() || self.compact {
            ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
                if self.only.is_none() && !self.search.active() {
                    self.list(ui, cx);
                } else {
                    self.content(ui, cx);
                }
            });
            ui.advance_cursor_after_rect(rect);
            if let Some(section) = self.push.take()
                && self.only != Some(section)
            {
                cx.nav.push(NavRequest::Push(Route::SettingsSection(section)));
            }
            return;
        }
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
        let searching = self.search.active();
        // The rows only matter while they're being searched through.
        let rows = searching.then(all_rows);
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
                            for section in Section::shown().filter(|section| section.store() == store) {
                                let matches = rows.as_deref().map(|rows| self.search.matches(section, rows));
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

    /// A phone's list of sections, by store: each a row of its icon, its title over what it
    /// has, and a chevron, opening its own page.
    fn list(&mut self, ui: &mut Ui, cx: &mut PageCx) {
        let area = egui::ScrollArea::vertical()
            .id_salt("settings_list")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let side = crate::layout::side(ui.ctx()).min(PAGE_PADDING);
                egui::Frame::NONE
                    .inner_margin(egui::Margin::symmetric(side as i8, PAGE_PADDING as i8))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        for (index, store) in Store::ALL.into_iter().enumerate() {
                            if index > 0 {
                                ui.add_space(SECTION_GAP);
                            }
                            menu_heading(ui, store);
                            ui.add_space(SPACE.xs);
                            surface::frame(ui).inner_margin(egui::Margin::ZERO).show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.spacing_mut().item_spacing.y = 0.;
                                for section in Section::shown().filter(|section| section.store() == store) {
                                    if section_entry(ui, section).clicked() {
                                        cx.nav.push(NavRequest::Push(Route::SettingsSection(section)));
                                    }
                                }
                            });
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
                let column = (full - 2. * crate::layout::side(ui.ctx()).min(PAGE_PADDING)).clamp(0., CONTENT_W);
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
        // The rows only matter while they're being searched through.
        let rows = self.search.active().then(all_rows);
        let found: Vec<Section> = Section::shown()
            .filter(|&section| self.only.is_none_or(|only| only == section))
            .filter(|&section| {
                rows.as_deref()
                    .is_none_or(|rows| self.search.matches(section, rows) > 0)
            })
            .collect();
        let mut starts = Vec::with_capacity(found.len());
        if found.is_empty() {
            let message = tr!("settings-no-match", query = self.search.text.trim());
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
        let store = section.store();
        let store_title = store.title();
        ui.horizontal(|ui| {
            ui.label(label::heading(ui, section.title()));
            ui.add_space(SPACE.xs);
            pill::icon_tag(ui, store.icon(), &store_title);
        });
        if !explain {
            return;
        }
        let description = match store {
            Store::Device => tr!("settings-kept-on-device"),
            Store::Server => {
                let session = &cx.services.session;
                let user = session.user.name.as_str();
                match session.server.server_name.as_deref() {
                    Some(server) => tr!("settings-kept-in-account-on", user = user, server = server),
                    None => tr!("settings-kept-in-account", user = user),
                }
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
            Section::Playback => self.playback(ui, cx),
            Section::Subtitles => self.subtitles(ui, cx),
            Section::Downloads => self.downloads(ui, cx),
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
        let picked = self.pickers.choose(
            ui,
            &self.search,
            theme_row(),
            "settings_theme",
            &themes(),
            current,
            default,
        );
        if let Some(edit) = picked {
            self.save_appearance(cx, |config| config.theme = edit.or_unset().unwrap_or_default());
        }

        // `None` follows the system, whose size can differ from 100 %.
        let scale = format!("{:.0}", theme::system_text_scale() * 100.);
        let system = tr!("settings-text-size-system", scale = scale);
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
            text_size_row(),
            "settings_text_size",
            &text_sizes,
            current,
            Some(None),
        );
        if let Some(edit) = picked {
            self.save_appearance(cx, |config| config.text_size = edit.or_unset().flatten());
        }

        let current = self.config.animations.unwrap_or(true);
        if let Some(edit) = toggle(ui, &self.search, animations_row(), current, Some(true)) {
            self.save_appearance(cx, |config| config.animations = edit.or_unset());
        }

        let current = self.config.hide_spoilers.unwrap_or(false);
        if let Some(edit) = toggle(ui, &self.search, spoilers_row(), current, Some(false)) {
            let hide = edit.or_unset();
            self.save(|config| config.hide_spoilers = hide);
            crate::spoilers::set(cx.ctx, hide.unwrap_or(false));
        }

        // Each language by its own name; following the system says which one that is.
        let system = tr!("settings-language-system", language = i18n::Language::system().name());
        let languages: Vec<(Option<i18n::Language>, String)> = std::iter::once((None, system))
            .chain(i18n::Language::ALL.map(|language| (Some(language), language.name().to_owned())))
            .collect();
        let current = self.config.language;
        let picked = self.pickers.choose(
            ui,
            &self.search,
            language_row(),
            "settings_language",
            &languages,
            current,
            Some(None),
        );
        if let Some(edit) = picked {
            self.save_appearance(cx, |config| config.language = edit.or_unset().flatten());
        }
    }

    fn playback(&mut self, ui: &mut Ui, cx: &mut PageCx) {
        if note(ui, &self.search, tracks_note_row(), Some(Section::Languages)) {
            self.open(ui.ctx(), Section::Languages);
        }

        let current = self.config.autoplay_next_episode.unwrap_or(true);
        if let Some(edit) = toggle(ui, &self.search, autoplay_row(), current, Some(true)) {
            self.save(|config| config.autoplay_next_episode = edit.or_unset());
        }

        let current = self.config.auto_skip_intro.unwrap_or(false);
        if let Some(edit) = toggle(ui, &self.search, skip_intro_row(), current, Some(false)) {
            self.save(|config| config.auto_skip_intro = edit.or_unset());
        }

        // `AUTOMATIC` measures the connection before playing instead.
        let bitrates: Vec<(i32, String)> = std::iter::once((AUTOMATIC, tr!("settings-bitrate-auto-measured")))
            .chain(BITRATES_MBPS.iter().map(|&mbps| match mbps {
                0 => (mbps, tr!("settings-bitrate-unlimited")),
                mbps => (mbps, format!("{mbps} Mbit/s")),
            }))
            .collect();
        let current = bitrate_choice(&self.config);
        let default = Some(bitrate_choice(&Config::default()));
        let picked = self.pickers.choose(
            ui,
            &self.search,
            bitrate_row(),
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

        // 0 leaves it to the TV.
        let cast_bitrates: Vec<(i32, String)> = BITRATES_MBPS
            .iter()
            .map(|&mbps| match mbps {
                0 => (mbps, tr!("settings-bitrate-auto")),
                mbps => (mbps, format!("{mbps} Mbit/s")),
            })
            .collect();
        let current = self.config.cast_max_bitrate.map_or(0, |bps| (bps / 1_000_000) as i32);
        let picked = self.pickers.choose(
            ui,
            &self.search,
            cast_bitrate_row(),
            "settings_cast_bitrate",
            &cast_bitrates,
            current,
            Some(0),
        );
        if let Some(edit) = picked {
            let bitrate = match edit {
                Edit::Set(mbps) if mbps > 0 => Some(mbps as u64 * 1_000_000),
                Edit::Set(_) | Edit::Default => None,
            };
            self.save(|config| config.cast_max_bitrate = bitrate);
            cx.services.cast.set_max_bitrate(bitrate);
        }

        let current = self.config.prefer_hevc.unwrap_or(false);
        if let Some(edit) = toggle(ui, &self.search, hevc_row(), current, Some(false)) {
            self.save(|config| config.prefer_hevc = edit.or_unset());
        }

        // Only Android has a second player.
        if !cfg!(target_os = "android") {
            return;
        }
        let current = self.config.android_player.unwrap_or_default();
        let picked = self.pickers.choose(
            ui,
            &self.search,
            player_row(),
            "settings_player",
            &PLAYERS,
            current,
            Some(AndroidPlayer::default()),
        );
        if let Some(edit) = picked {
            self.save(|config| config.android_player = edit.or_unset());
        }
    }

    fn subtitles(&mut self, ui: &mut Ui, cx: &mut PageCx) {
        note(ui, &self.search, subtitles_note_row(), None);
        if note(ui, &self.search, subtitle_tracks_note_row(), Some(Section::Languages)) {
            self.open(ui.ctx(), Section::Languages);
        }

        let current = self.config.subtitle_color.unwrap_or_default();
        let picked = self.pickers.choose(
            ui,
            &self.search,
            subtitle_color_row(),
            "settings_subtitle_color",
            &subtitle_colors(),
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
            subtitle_style_row(),
            "settings_subtitle_style",
            &subtitle_styles(),
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
            subtitle_size_row(),
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

        // A position set in the player that isn't one of the steps here is listed too.
        let current = self.config.subtitle_position.unwrap_or(Adjustments::BOTTOM);
        let positions: Vec<(u8, String)> = subtitle_positions(current)
            .into_iter()
            .map(|position| (position, subtitle_position_label(position)))
            .collect();
        let picked = self.pickers.choose(
            ui,
            &self.search,
            subtitle_position_row(),
            "settings_subtitle_position",
            &positions,
            current,
            Some(Adjustments::BOTTOM),
        );
        if let Some(edit) = picked {
            let position = edit.or_unset().unwrap_or(Adjustments::BOTTOM);
            self.save(|config| config.subtitle_position = Some(position).filter(|&p| p != Adjustments::BOTTOM));
            cx.services.playback.set_subtitle_position(position);
        }
    }

    fn downloads(&mut self, ui: &mut Ui, cx: &mut PageCx) {
        let current = self.config.download_wifi_only.unwrap_or(true);
        if let Some(edit) = toggle(ui, &self.search, wifi_only_row(), current, Some(true)) {
            let wifi_only = edit.or_unset();
            self.save(|config| config.download_wifi_only = wifi_only);
            cx.services.downloads.set_wifi_only(wifi_only.unwrap_or(true));
        }
        if self.search.shows(&downloads_folder_row()) {
            let folder = config::downloads_dir().display().to_string();
            row_with_folder(ui, downloads_folder_row(), Some(&folder), None, |_| {});
        }
    }

    fn storage(&mut self, ui: &mut Ui, cx: &mut PageCx) {
        // Nothing to save for these two, so the row shows its folder instead of a default.
        let busy = self.job.is_some();
        if self.search.shows(&image_cache_row()) {
            let images = config::cache_dir().join("images").display().to_string();
            let mut clear = false;
            row_with_folder(ui, image_cache_row(), Some(&images), None, |ui| {
                clear = button::quiet(ui, &tr!("settings-clear"), ROW_BUTTON_W, ready(busy), Focus::Tab).clicked();
            });
            if clear {
                self.clear_image_cache(cx);
            }
        }

        if self.search.shows(&logs_row()) {
            let logs = config::log_dir().display().to_string();
            let mut open = false;
            row_with_folder(ui, logs_row(), Some(&logs), None, |ui| {
                open = button::quiet(ui, &tr!("settings-open-folder"), ROW_BUTTON_W, ready(busy), Focus::Tab).clicked();
            });
            if open {
                self.open_logs(cx);
            }
        }
    }

    fn about(&mut self, ui: &mut Ui) {
        info(ui, &self.search, version_row(), env!("CARGO_PKG_VERSION"));

        // The store updates the app on Android, so there's no check to turn off.
        if cfg!(target_os = "android") {
            return;
        }
        let current = self.config.check_for_updates.unwrap_or(true);
        if let Some(edit) = toggle(ui, &self.search, check_for_updates_row(), current, Some(true)) {
            self.save(|config| config.check_for_updates = edit.or_unset());
        }
    }

    fn account(&mut self, ui: &mut Ui, cx: &mut PageCx) {
        let session = &cx.services.session;
        // Who's signed in, as the sign-in screen lists accounts: one row for the user, the
        // server and its address, found by a search for any of the three.
        if [user_row(), server_row(), address_row()]
            .iter()
            .any(|row| self.search.shows(row))
        {
            let server = session.server.server_name.clone().unwrap_or_else(|| "Jellyfin".into());
            let address = session.client.base_url().as_str().trim_end_matches('/').to_owned();
            let subtitle = format!("{server} · {address}");
            let user = session.user.name.as_str();
            let label = tr!("settings-signed-in-as", user = user, server = subtitle.as_str());
            let names = [session.user.name.as_str()];
            ui.add_space(ROW_PAD);
            AvatarRow::new(&names, &session.user.name, &subtitle, &label).show(ui, false);
            ui.add_space(ROW_PAD);
        }
        let version = session.server.version.clone().unwrap_or_else(|| "?".into());
        info(ui, &self.search, server_version_row(), &version);
    }

    fn languages(&mut self, ui: &mut Ui, cx: &mut PageCx) {
        let Some(config) = self.server.loaded(ui) else {
            return;
        };
        note(ui, &self.search, languages_note_row(), None);

        let current = config.audio_language_preference.as_deref().unwrap_or("");
        let languages = self.server.language_choices(current);
        let picked = self.pickers.choose(
            ui,
            &self.search,
            audio_language_row(),
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
            play_default_audio_row(),
            config.play_default_audio_track,
            None,
        ) {
            let changes = UserConfigurationChanges {
                play_default_audio_track: Some(on),
                ..Default::default()
            };
            self.server.change(cx, changes);
        }

        if let Some(Edit::Set(on)) = toggle(
            ui,
            &self.search,
            remember_audio_row(),
            config.remember_audio_selections,
            None,
        ) {
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
            subtitle_language_row(),
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
            subtitle_mode_row(),
            "settings_subtitle_mode",
            &subtitle_modes(),
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
            remember_subtitles_row(),
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
                hide_played_in_latest_row(),
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
        note(ui, &self.search, home_sections_note_row(), None);
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
                    cx.services.toast(Toast::error(tr!("settings-save-failed")));
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
            surface::notice(ui, surface::Kind::Danger, &tr!("settings-load-failed"));
            ui.add_space(surface::GAP_FIELD);
            let retry = button::quiet(ui, &tr!("settings-retry"), ROW_BUTTON_W, State::Ready, Focus::Tab).clicked();
            if retry {
                self.failed = false;
                self.cultures_asked = !self.languages.is_empty();
            }
        } else {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(14.).color(theme::tokens_of(ui).ink_muted));
                ui.label(label::muted(ui, tr!("settings-loading")));
            });
        }
        ui.add_space(ROW_PAD);
        None
    }

    /// A language picker's choices: none first, then the server's languages, and the current
    /// code by itself if the server's list doesn't have it (or didn't load).
    fn language_choices<'a>(&'a self, current: &'a str) -> Vec<(&'a str, String)> {
        let mut choices: Vec<(&str, String)> = std::iter::once(("", tr!("settings-any-language")))
            .chain(self.languages.iter().map(|(code, name)| (code.as_str(), name.clone())))
            .collect();
        if !choices.iter().any(|(code, _)| *code == current) {
            choices.push((current, current.to_owned()));
        }
        choices
    }

    /// How saving is going, under the server's banner.
    fn status(&self) -> Option<String> {
        if self.save.is_some() || !self.pending.is_empty() {
            Some(tr!("settings-saving"))
        } else if self.saved {
            Some(format!(
                "{} {}",
                ICON_CLOUD_DONE.codepoint,
                tr!("settings-saved-to-server")
            ))
        } else {
            None
        }
    }
}

/// The server's cultures as (code, name), sorted by name, one per code.
pub(crate) fn language_list(cultures: Vec<Culture>) -> Vec<(String, String)> {
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
    let title = store.title();
    ui.horizontal(|ui| {
        ui.add_space(8.);
        ui.label(label::overline(ui, &title));
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
    let title = section.title();
    response.widget_info(|| WidgetInfo::selected(Role::Button, true, selected, &title));
    let focused = ui.memory(|memory| memory.has_focus(response.id));
    let radius = egui::CornerRadius::same(tokens.radius_control);
    if selected || response.hovered() || focused {
        ui.painter().rect_filled(rect, radius, tokens.inset_hover);
    }
    if selected {
        list_row::open_bar(ui, rect);
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
    ui.painter()
        .text(pos2(label_x, rect.center().y), Align2::LEFT_CENTER, &title, font, color);
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

/// A section in a phone's list: its icon in `brand`, its title over what it has, and a chevron at
/// the right, at least 64 points tall, with a hairline above all but the first. `inset_hover`
/// while pressed.
fn section_entry(ui: &mut Ui, section: Section) -> egui::Response {
    let tokens = theme::tokens_of(ui);
    let title_font = text::font(ui, "base");
    let summary_font = text::font(ui, "sm");
    let (title_h, summary_h) = ui.fonts_mut(|fonts| (fonts.row_height(&title_font), fonts.row_height(&summary_font)));
    let lines = title_h + summary_h;
    let height = (lines + 2. * ROW_PAD).max(64.);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    let title = section.title();
    response.widget_info(|| WidgetInfo::labeled(Role::Button, true, &title));
    let first = rect.top() <= ui.min_rect().top() + 0.5;
    let focused = ui.memory(|memory| memory.has_focus(response.id));
    if response.is_pointer_button_down_on() || response.hovered() || focused {
        ui.painter().rect_filled(rect, 0., tokens.inset_hover);
    }
    if !first {
        let stroke = egui::Stroke::new(1., tokens.surface_border);
        ui.painter().hline(rect.x_range().shrink(16.), rect.top(), stroke);
    }
    if focused {
        focus::ring(ui, rect, tokens.radius_control as f32, Place::Inside);
    }
    let icon = text::icon(ui, section.icon().font_family());
    let icon_x = rect.left() + 16. + icon.size / 2.;
    let chevron_x = rect.right() - 12. - icon.size / 2.;
    let left = icon_x + icon.size / 2. + 16.;
    let width = (chevron_x - icon.size / 2. - 8. - left).max(0.);
    let line = |text: &str, font: egui::FontId, colour| {
        let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, colour);
        job.wrap = egui::text::TextWrapping::truncate_at_width(width);
        ui.fonts_mut(|fonts| fonts.layout_job(job))
    };
    let title_galley = line(&title, title_font, tokens.ink);
    let summary = section.summary();
    let summary_galley = line(&summary, summary_font, tokens.ink_muted);
    let painter = ui.painter();
    painter.text(
        pos2(icon_x, rect.center().y),
        Align2::CENTER_CENTER,
        section.icon().codepoint,
        icon.clone(),
        tokens.brand,
    );
    painter.text(
        pos2(chevron_x, rect.center().y),
        Align2::CENTER_CENTER,
        ICON_CHEVRON_RIGHT.codepoint,
        icon,
        tokens.ink_muted,
    );
    let top = rect.center().y - lines / 2.;
    painter.galley(pos2(left, top), title_galley, tokens.ink);
    painter.galley(pos2(left, top + title_h), summary_galley, tokens.ink_muted);
    response
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
    // A phone's column is too narrow for both: the control goes under the text, at the right.
    let compact = crate::layout::compact(ui.ctx());
    let text_w = if compact {
        full.width()
    } else {
        (full.width() - PICKER_W - RESET_W - 2. * SPACE.sm).max(120.)
    };
    let text_rect = Rect::from_min_size(pos2(full.left(), top), vec2(text_w, f32::INFINITY));
    let text = ui
        .scope_builder(UiBuilder::new().max_rect(text_rect), |ui| {
            ui.spacing_mut().item_spacing.y = SPACE.xxs;
            let name = ui.label(
                RichText::new(row.label.as_str())
                    .font(text::font(ui, "base"))
                    .color(tokens.ink),
            );
            if let Some(tip) = key_tooltip(&row) {
                name.on_hover_text(tip);
            }
            if !row.description.is_empty() {
                ui.label(label::muted(ui, row.description.as_str()));
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
    let controls = if compact {
        let below = text.bottom() + SPACE.sm;
        Rect::from_min_max(
            pos2(full.left(), below),
            pos2(full.right(), below + ui.spacing().interact_size.y),
        )
    } else {
        Rect::from_min_max(
            pos2(text.right() + SPACE.sm, top),
            pos2(full.right(), text.bottom().max(top + ui.spacing().interact_size.y)),
        )
    };
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
        Store::Device if row.unset.is_empty() => tr!("settings-key-config", key = row.key),
        Store::Device => tr!("settings-key-config-unset", key = row.key, unset = row.unset.as_str()),
        Store::Server => tr!("settings-key-profile", key = row.key),
    })
}

/// The undo button of a setting that's off its default, naming the default in its tooltip.
/// Tab stops on it after the setting's control.
fn reset_button(ui: &mut Ui, default: &str) -> egui::Response {
    let tooltip = tr!("settings-reset-default-tooltip", default = default);
    let label = tr!("settings-reset-default");
    RoundButton::icon(ICON_UNDO, &label)
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
    let reset = default.filter(|&default| default != value).map(|default| {
        if default {
            tr!("settings-on")
        } else {
            tr!("settings-off")
        }
    });
    let label = row.label.clone();
    let mut flipped = None;
    let reset_clicked = self::row(ui, row, reset.as_deref(), |ui| {
        let mut on = value;
        if toggle::switch(ui, &mut on, &label).changed() {
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
        let name = row.label.clone();
        let reset_clicked = self::row(ui, row, reset, |ui| {
            // A value not among the choices (a bitrate set by hand) shows the first one.
            let menu = ui.make_persistent_id(id);
            let (response, chosen) = select::select(ui, menu, PICKER_W, &name, &labels, index.unwrap_or(0), true);
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
/// hand: the picker shows the first one, and Left and Right stay put).
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

/// A paragraph in a group that isn't a setting, when the search shows it, with a link to
/// `link`'s section after it. Returns whether the link was clicked.
fn note(ui: &mut Ui, search: &Search, row: Row, link: Option<Section>) -> bool {
    if !search.shows(&row) {
        return false;
    }
    hairline(ui);
    ui.add_space(ROW_PAD);
    let description = label::muted(ui, row.description.as_str());
    let mut clicked = false;
    match link {
        // Wrapped with the link at the end of its last line. A wrapped row's lines are a
        // control's height apart, so a note without a link is a plain label.
        Some(section) => {
            let title = section.title();
            ui.horizontal_wrapped(|ui| {
                ui.label(description);
                clicked = button::link(ui, &title, "sm").clicked();
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

/// The config a bitrate choice writes: *Automatic* turns measuring on and leaves the saved cap
/// alone (the measurement replaces it, and the cap is what turning *Automatic* off goes back
/// to), any other choice turns measuring off and saves the cap in bits per second, `None` for no
/// limit.
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

/// The positions the picker offers: the steps, plus the one set in the player when it isn't one.
fn subtitle_positions(current: u8) -> Vec<u8> {
    let mut positions = SUBTITLE_POSITIONS.to_vec();
    if !positions.contains(&current) {
        positions.push(current);
        positions.sort_by(|a, b| b.cmp(a));
    }
    positions
}

/// A position as the picker names it: the bottom, or how far up from it.
fn subtitle_position_label(position: u8) -> String {
    match Adjustments::BOTTOM.saturating_sub(position) {
        0 => tr!("settings-subtitle-position-bottom"),
        percent => tr!("settings-subtitle-position-raised", percent = percent),
    }
}

/// Opens a folder in the desktop's file manager. Not the `opener` crate: it's one program per
/// OS, and this is the only place the app opens anything on the desktop. Waits for the launcher,
/// which exits once the file manager has the folder, so a missing one is reported.
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
        AUTOMATIC, BITRATES_MBPS, SUBTITLE_POSITIONS, SUBTITLE_SCALES, Search, Section, Store, all_rows, apply_bitrate,
        bitrate_choice, language_list, stepped, subtitle_positions, subtitle_scales, themes,
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
        let themes: Vec<ThemePreference> = themes().iter().map(|(theme, _)| *theme).collect();
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
    fn the_players_subtitle_position_joins_the_steps() {
        assert_eq!(subtitle_positions(100), SUBTITLE_POSITIONS.to_vec());
        assert_eq!(subtitle_positions(75), [100, 95, 90, 85, 80, 75, 70, 60, 50, 40, 30]);
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
        assert_eq!(config.theme, ThemePreference::System);
        assert_eq!(config.text_size, None); // the system's
        assert_eq!(config.animations, None); // on
        assert_eq!(config.hide_spoilers, None); // off
        assert_eq!(config.autoplay_next_episode, None); // on
        assert_eq!(config.auto_skip_intro, None); // off
        assert_eq!(config.prefer_hevc, None); // off
        assert_eq!(config.check_for_updates, None); // on
        assert_eq!(config.subtitle_color, None); // white
        assert_eq!(config.subtitle_style, None); // outline
        assert_eq!(config.subtitle_scale, None); // 100 %
        assert_eq!(config.subtitle_position, None); // the bottom
        assert_eq!(bitrate_choice(&config), 0); // no limit
    }

    fn search(text: &str) -> Search {
        Search { text: text.into() }
    }

    /// The sections a search finds, in the menu's order.
    fn found(text: &str) -> Vec<Section> {
        let search = search(text);
        let rows = all_rows();
        Section::shown()
            .filter(|&section| search.matches(section, &rows) > 0)
            .collect()
    }

    #[test]
    fn every_section_has_rows_for_the_search() {
        let rows = all_rows();
        for section in Section::ALL {
            assert!(rows.iter().any(|row| row.section == section), "{section:?}");
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
        let rows = all_rows();
        assert!(
            rows.iter()
                .filter(|row| row.section == Section::Storage)
                .all(|row| storage.shows(row))
        );
        let server: Vec<Section> = Section::shown()
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
