//! Settings: a menu of sections on the left, split by where their settings are kept, and the
//! open section on the right.
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
//! The search box above the menu filters every row of both by its section, label, description
//! and key, and shows what's left grouped the same way, each group under its banner.
//!
//! The page isn't on the browse cursor, as on `main`: [`Page::scope`] is `None`, so egui's own
//! focus handles Tab (the search box, the menu, then the section's controls), Space or Enter on
//! a menu entry, the arrows in a picker and Space on a switch, and Esc still goes back.

use std::path::Path;

use egui::{Align2, Id, Rect, RichText, Role, Sense, Ui, UiBuilder, WidgetInfo, pos2, vec2};
use egui_material_icons::MaterialIcon;
use egui_material_icons::icons::{
    ICON_ACCOUNT_CIRCLE, ICON_CLOSE, ICON_CLOUD_DONE, ICON_COMPUTER, ICON_DNS, ICON_HOME, ICON_INFO, ICON_PALETTE,
    ICON_PLAY_CIRCLE, ICON_SEARCH, ICON_STORAGE, ICON_SUBTITLES, ICON_TRANSLATE,
};
use jellyfin::models::{Culture, SubtitleMode, UserConfiguration, UserConfigurationChanges};

use crate::config::{self, Config, SubtitleColor, SubtitleStyle, ThemePreference};
use crate::services::AppEvent;
use crate::task::Task;
use crate::theme;
use crate::theme::text;
use crate::toasts::Toast;
use crate::views::{Page, PageCx};

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
/// The pickers' width beside their label (`main`'s 220 px).
const PICKER_W: f32 = 220.;
/// The gaps between a group's heading and its box, between the rows in it, and between groups.
const HEADING_GAP: f32 = 4.;
const ROW_GAP: f32 = 10.;
const SECTION_GAP: f32 = 24.;
/// The size of the icons beside the group headings, and of the banners' icons.
const HEADING_ICON: f32 = 20.;
const BANNER_ICON: f32 = 28.;

/// The search box's id, which the tests find it by too.
const SEARCH_ID: &str = "settings_search";

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
                  server to encode. Used only if the server allows it (Dashboard → Playback → Transcoding). \
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
                  (Settings → Home in the web client).",
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
    /// The menu's open section, shown while nothing is searched for.
    section: Section,
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

    /// Opens `section` from the menu or a note's link, leaving the search.
    fn open(&mut self, section: Section) {
        self.section = section;
        self.search.text.clear();
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

    fn show(&mut self, ui: &mut egui::Ui, cx: &mut PageCx) {
        self.pickers.begin_frame(cx.ctx);
        self.report_job(cx);
        self.server.tick(cx);

        // The menu, a line, and the section, side by side. Drawn in that order, so Tab goes
        // through the search box and the menu before the section's controls.
        let rect = ui.available_rect_before_wrap();
        let menu_w = (rect.width() * 0.25).clamp(MENU_MIN_W, MENU_MAX_W);
        let menu = Rect::from_min_max(rect.min, pos2(rect.left() + menu_w, rect.bottom()));
        let content = Rect::from_min_max(pos2(menu.right() + 1., rect.top()), rect.max);
        ui.scope_builder(UiBuilder::new().max_rect(menu), |ui| self.menu(ui));
        let line = egui::Stroke::new(1., theme::tokens_of(ui).border);
        ui.painter().vline(menu.right() + 0.5, rect.y_range(), line);
        ui.scope_builder(UiBuilder::new().max_rect(content), |ui| self.content(ui, cx));
        ui.advance_cursor_after_rect(rect);
    }
}

impl SettingsPage {
    /// The search box over the two groups of sections.
    fn menu(&mut self, ui: &mut Ui) {
        egui::ScrollArea::vertical()
            .id_salt("settings_menu")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Frame::NONE
                    .inner_margin(egui::Margin::symmetric(12, PAGE_PADDING as i8))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        self.search_box(ui);
                        for store in Store::ALL {
                            ui.add_space(SECTION_GAP * 0.75);
                            menu_heading(ui, store);
                            for section in Section::ALL.into_iter().filter(|section| section.store() == store) {
                                let searching = self.search.active();
                                let matches = searching.then(|| self.search.matches(section));
                                let selected = !searching && self.section == section;
                                if menu_entry(ui, section, selected, matches).clicked() {
                                    self.open(section);
                                }
                            }
                        }
                    });
            });
    }

    /// `main`'s settings search: the search icon inside the field's left padding and, while
    /// it has text, a clear button at its right end. Esc in the field clears it as it lets go.
    fn search_box(&mut self, ui: &mut Ui) {
        let icon = text::icon(ui, ICON_SEARCH.font_family());
        let inset = (10. + icon.size + 6.).round() as i8;
        let clear = !self.search.text.is_empty();
        let response = ui.add(
            egui::TextEdit::singleline(&mut self.search.text)
                .id(Id::unique(SEARCH_ID))
                .hint_text("Search settings")
                .desired_width(f32::INFINITY)
                .margin(egui::Margin {
                    left: inset,
                    right: if clear { inset } else { 10 },
                    top: 9,
                    bottom: 9,
                }),
        );
        let tokens = theme::tokens_of(ui);
        let rect = response.rect;
        ui.painter().text(
            pos2(rect.left() + 10. + icon.size / 2., rect.center().y),
            Align2::CENTER_CENTER,
            ICON_SEARCH.codepoint,
            icon.clone(),
            tokens.muted_foreground,
        );
        if response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.search.text.clear();
        }
        if clear {
            // Not focusable, as the top bar's isn't: Esc clears from the keyboard.
            let center = pos2(rect.right() - 10. - icon.size / 2., rect.center().y);
            let hit = Rect::from_center_size(center, vec2(icon.size + 8., icon.size + 8.));
            let button = ui.interact(hit, Id::unique("settings_search_clear"), Sense::CLICK);
            button.widget_info(|| WidgetInfo::labeled(Role::Button, ui.is_enabled(), "Clear search"));
            let color = if button.hovered() {
                tokens.foreground
            } else {
                tokens.muted_foreground
            };
            ui.painter()
                .text(center, Align2::CENTER_CENTER, ICON_CLOSE.codepoint, icon, color);
            if button.clicked() {
                self.search.text.clear();
            }
        }
    }

    /// The open section, or what the search found, scrolling under the top bar. Each section
    /// and the results keep a scroll position of their own.
    fn content(&mut self, ui: &mut Ui, cx: &mut PageCx) {
        let searching = self.search.active();
        let salt = if searching { "results" } else { self.section.title() };
        egui::ScrollArea::vertical()
            .id_salt(("settings_content", salt))
            .auto_shrink([false, false])
            .show(ui, |ui| {
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
                        if searching {
                            self.results(ui, cx);
                        } else {
                            self.banner(ui, cx, self.section.store());
                            self.section(ui, cx, self.section);
                        }
                        ui.add_space(PAGE_PADDING);
                    });
                });
            });
    }

    /// Every section with a row the search finds, under the banner of where it's kept.
    fn results(&mut self, ui: &mut Ui, cx: &mut PageCx) {
        let found: Vec<Section> = Section::ALL
            .into_iter()
            .filter(|&section| self.search.matches(section) > 0)
            .collect();
        if found.is_empty() {
            let message = format!("No settings match “{}”.", self.search.text.trim());
            ui.label(RichText::new(message).color(theme::tokens_of(ui).muted_foreground));
            return;
        }
        for store in Store::ALL {
            let sections: Vec<Section> = found
                .iter()
                .copied()
                .filter(|section| section.store() == store)
                .collect();
            if sections.is_empty() {
                continue;
            }
            self.banner(ui, cx, store);
            for section in sections {
                self.section(ui, cx, section);
            }
            ui.add_space(SECTION_GAP);
        }
    }

    /// Where the settings under it are kept, so the two kinds can't be mistaken for each other:
    /// the device's config, or the account on the server (with how its saving is going).
    fn banner(&self, ui: &mut Ui, cx: &PageCx, store: Store) {
        let tokens = theme::tokens_of(ui);
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
        egui::Frame::NONE
            .fill(tokens.muted)
            .corner_radius(egui::CornerRadius::same(tokens.radius_lg))
            .inner_margin(12)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_top(|ui| {
                    ui.label(store.icon().rich_text().size(BANNER_ICON).color(tokens.foreground));
                    ui.add_space(4.);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(store.title()).font(text::font(ui, "lg")).strong());
                        ui.label(
                            RichText::new(description)
                                .font(text::font(ui, "sm"))
                                .color(tokens.muted_foreground),
                        );
                        if store == Store::Server
                            && let Some(status) = self.server.status()
                        {
                            ui.label(
                                RichText::new(status)
                                    .font(text::font(ui, "sm"))
                                    .color(tokens.muted_foreground),
                            );
                        }
                    });
                });
            });
    }

    /// A section's heading and its box of rows.
    fn section(&mut self, ui: &mut Ui, cx: &mut PageCx, section: Section) {
        group(ui, section.icon(), section.title(), |ui| match section {
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
        let picked = self
            .pickers
            .choose(ui, &self.search, THEME, "settings_theme", &THEMES, current);
        if let Some(theme) = picked {
            self.save_appearance(cx, |config| config.theme = theme);
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
        let picked = self
            .pickers
            .choose(ui, &self.search, TEXT_SIZE, "settings_text_size", &text_sizes, current);
        if let Some(size) = picked {
            self.save_appearance(cx, |config| config.text_size = size);
        }

        let current = self.config.animations.unwrap_or(true);
        if let Some(on) = toggle(ui, &self.search, ANIMATIONS, current) {
            self.save_appearance(cx, |config| config.animations = Some(on));
        }
    }

    fn playback(&mut self, ui: &mut Ui) {
        if note(ui, &self.search, TRACKS_NOTE, Some("Languages")) {
            self.open(Section::Languages);
        }

        let current = self.config.autoplay_next_episode.unwrap_or(true);
        if let Some(on) = toggle(ui, &self.search, AUTOPLAY, current) {
            self.save(|config| config.autoplay_next_episode = Some(on));
        }

        let current = self.config.auto_skip_intro.unwrap_or(false);
        if let Some(on) = toggle(ui, &self.search, SKIP_INTRO, current) {
            self.save(|config| config.auto_skip_intro = Some(on));
        }

        // `AUTOMATIC` measures the connection before playing instead.
        let bitrates: Vec<(i32, String)> = std::iter::once((AUTOMATIC, "Automatic (measured)".to_string()))
            .chain(BITRATES_MBPS.iter().map(|&mbps| match mbps {
                0 => (mbps, "No limit".to_string()),
                mbps => (mbps, format!("{mbps} Mbit/s")),
            }))
            .collect();
        let current = bitrate_choice(&self.config);
        let picked = self
            .pickers
            .choose(ui, &self.search, BITRATE, "settings_bitrate", &bitrates, current);
        if let Some(mbps) = picked {
            self.save(|config| apply_bitrate(config, mbps));
        }

        let current = self.config.prefer_hevc.unwrap_or(false);
        if let Some(on) = toggle(ui, &self.search, HEVC, current) {
            self.save(|config| config.prefer_hevc = Some(on));
        }
    }

    fn subtitles(&mut self, ui: &mut Ui, cx: &mut PageCx) {
        note(ui, &self.search, SUBTITLES_NOTE, None);
        if note(ui, &self.search, SUBTITLE_TRACKS_NOTE, Some("Languages")) {
            self.open(Section::Languages);
        }

        let current = self.config.subtitle_color.unwrap_or_default();
        let picked = self.pickers.choose(
            ui,
            &self.search,
            SUBTITLE_COLOR,
            "settings_subtitle_color",
            &SUBTITLE_COLORS,
            current,
        );
        if let Some(color) = picked {
            self.save(|config| config.subtitle_color = Some(color));
        }

        let current = self.config.subtitle_style.unwrap_or_default();
        let picked = self.pickers.choose(
            ui,
            &self.search,
            SUBTITLE_STYLE,
            "settings_subtitle_style",
            &SUBTITLE_STYLES,
            current,
        );
        if let Some(style) = picked {
            self.save(|config| config.subtitle_style = Some(style));
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
        );
        if let Some(scale) = picked {
            self.save(|config| config.subtitle_scale = Some(scale).filter(|&scale| scale != 1.));
            cx.services.playback.set_subtitle_scale(scale);
        }
    }

    fn storage(&mut self, ui: &mut Ui, cx: &mut PageCx) {
        // Nothing to save for these two, so the row's smaller line shows the folder instead of
        // a config key.
        let busy = self.job.is_some();
        if self.search.shows(&IMAGE_CACHE) {
            let images = config::cache_dir().join("images").display().to_string();
            let mut clear = false;
            row(
                ui,
                Row {
                    key: &images,
                    ..IMAGE_CACHE
                },
                |ui| {
                    clear = ui.add_enabled(!busy, egui::Button::new("Clear")).clicked();
                },
            );
            if clear {
                self.clear_image_cache(cx);
            }
        }

        if self.search.shows(&LOGS) {
            let logs = config::log_dir().display().to_string();
            let mut open = false;
            row(ui, Row { key: &logs, ..LOGS }, |ui| {
                open = ui.add_enabled(!busy, egui::Button::new("Open folder")).clicked();
            });
            if open {
                self.open_logs(cx);
            }
        }
    }

    fn about(&mut self, ui: &mut Ui) {
        info(ui, &self.search, VERSION, env!("CARGO_PKG_VERSION"));

        let current = self.config.check_for_updates.unwrap_or(true);
        if let Some(on) = toggle(ui, &self.search, CHECK_FOR_UPDATES, current) {
            self.save(|config| config.check_for_updates = Some(on));
        }
    }

    fn account(&mut self, ui: &mut Ui, cx: &mut PageCx) {
        let session = &cx.services.session;
        info(ui, &self.search, USER, &session.user.name);
        let server = session.server.server_name.clone().unwrap_or_else(|| "Jellyfin".into());
        info(ui, &self.search, SERVER, &server);
        info(ui, &self.search, ADDRESS, session.client.base_url().as_str());
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
        );
        if let Some(code) = picked {
            let changes = UserConfigurationChanges {
                audio_language_preference: Some(code.to_string()),
                ..Default::default()
            };
            self.server.change(cx, changes);
        }

        if let Some(on) = toggle(ui, &self.search, PLAY_DEFAULT_AUDIO, config.play_default_audio_track) {
            let changes = UserConfigurationChanges {
                play_default_audio_track: Some(on),
                ..Default::default()
            };
            self.server.change(cx, changes);
        }

        if let Some(on) = toggle(ui, &self.search, REMEMBER_AUDIO, config.remember_audio_selections) {
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
        );
        if let Some(code) = picked {
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
        );
        if let Some(mode) = picked {
            let changes = UserConfigurationChanges {
                subtitle_mode: Some(mode),
                ..Default::default()
            };
            self.server.change(cx, changes);
        }

        if let Some(on) = toggle(
            ui,
            &self.search,
            REMEMBER_SUBTITLES,
            config.remember_subtitle_selections,
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
            && let Some(on) = toggle(ui, &self.search, HIDE_PLAYED_IN_LATEST, config.hide_played_in_latest)
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
        let muted = theme::tokens_of(ui).muted_foreground;
        if self.failed {
            let mut retry = false;
            ui.horizontal(|ui| {
                ui.label(RichText::new("Couldn't load your settings from the server.").color(muted));
                retry = ui.button("Retry").clicked();
            });
            if retry {
                self.failed = false;
                self.cultures_asked = !self.languages.is_empty();
            }
        } else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Loading your settings from the server…").color(muted));
            });
        }
        ui.add_space(ROW_GAP);
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
    let tokens = theme::tokens_of(ui);
    ui.horizontal(|ui| {
        ui.add_space(8.);
        ui.label(
            store
                .icon()
                .rich_text()
                .size(text::font(ui, "sm").size)
                .color(tokens.muted_foreground),
        );
        ui.label(
            RichText::new(store.title().to_uppercase())
                .font(text::font(ui, "xs"))
                .strong()
                .color(tokens.muted_foreground),
        );
    });
    ui.add_space(HEADING_GAP);
}

/// A section in the menu: its icon and title across the menu's width, filled while it's open
/// and under the pointer or the keyboard. While searching, `matches` is how many of its rows
/// the search finds, shown at the right end; one with none is greyed.
fn menu_entry(ui: &mut Ui, section: Section, selected: bool, matches: Option<usize>) -> egui::Response {
    let tokens = theme::tokens_of(ui);
    let height = (ui.spacing().interact_size.y + 10.).max(32.);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    response.widget_info(|| WidgetInfo::selected(Role::Button, true, selected, section.title()));
    let focused = ui.memory(|memory| memory.has_focus(response.id));
    let radius = egui::CornerRadius::same(tokens.radius);
    if selected {
        ui.painter().rect_filled(rect, radius, tokens.accent);
    } else if response.hovered() || focused {
        ui.painter().rect_filled(rect, radius, tokens.muted);
    }
    if focused {
        ui.painter()
            .rect_stroke(rect, radius, ui.visuals().selection.stroke, egui::StrokeKind::Inside);
    }
    let color = match matches {
        Some(0) => tokens.muted_foreground,
        _ if selected => tokens.accent_foreground,
        _ => tokens.foreground,
    };
    let icon = text::icon(ui, section.icon().font_family());
    let icon_x = rect.left() + 8. + icon.size / 2.;
    ui.painter().text(
        pos2(icon_x, rect.center().y),
        Align2::CENTER_CENTER,
        section.icon().codepoint,
        icon.clone(),
        color,
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
            tokens.muted_foreground,
        );
    }
    response
}

/// A group on the page: its heading, and a box around its rows.
fn group(ui: &mut Ui, icon: MaterialIcon, title: &str, contents: impl FnOnce(&mut Ui)) {
    ui.add_space(SECTION_GAP);
    let tokens = theme::tokens_of(ui);
    ui.horizontal(|ui| {
        ui.label(icon.rich_text().size(HEADING_ICON).color(tokens.muted_foreground));
        ui.label(RichText::new(title).heading());
    });
    ui.add_space(HEADING_GAP);
    // `main`'s `GroupBox::outline`: the border and the controls' radius over the page, with no
    // fill of its own (an unchecked checkbox would read poorly on a grey box in light).
    egui::Frame::NONE
        .stroke(egui::Stroke::new(1., tokens.border))
        .corner_radius(egui::CornerRadius::same(tokens.radius))
        .inner_margin(12)
        .show(ui, |ui| {
            // The box spans the form's width, not just its widest row.
            ui.set_width(ui.available_width());
            contents(ui);
        });
}

/// One setting: its label on the left and its control on the right, then what it does and the
/// key it saves.
fn row(ui: &mut Ui, row: Row, control: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(row.label).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), control);
    });
    // `main`'s setting descriptions were `text_sm` in the muted colour.
    ui.label(
        RichText::new(row.description)
            .font(text::font(ui, "sm"))
            .color(theme::tokens_of(ui).muted_foreground),
    );
    if !row.key.is_empty() {
        let metadata = if row.unset.is_empty() {
            row.key.to_string()
        } else {
            format!("{} · unset: {}", row.key, row.unset)
        };
        // The keys are monospace, as the file is: `RichText::monospace` would apply its family
        // after the table's font, so the font id is built directly.
        let mut font = text::font(ui, "xs");
        font.family = egui::FontFamily::Monospace;
        ui.label(
            RichText::new(metadata)
                .font(font)
                .color(theme::tokens_of(ui).muted_foreground),
        );
    }
    ui.add_space(ROW_GAP);
}

/// A switch for a flag setting, when the search shows it: returns the value it was flipped to,
/// if it was.
fn toggle(ui: &mut Ui, search: &Search, row: Row, value: bool) -> Option<bool> {
    if !search.shows(&row) {
        return None;
    }
    let mut flipped = None;
    self::row(ui, row, |ui| {
        let mut checked = value;
        if ui.add(egui::Checkbox::without_text(&mut checked)).changed() {
            flipped = Some(checked);
        }
    });
    flipped
}

/// The pickers' keys: egui's focus (Tab) puts it on a `ComboBox`, and Left and Right there step
/// its value to the previous or next choice, stopping at the ends — a 0.37 combo box's list is
/// pointer-only, so this is how a picker is changed without a mouse. The step comes back from
/// [`Pickers::choose`] as the same value a click gives, so the page saves it the same way.
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
    /// choice when one was picked in the list or stepped to with Left or Right.
    fn choose<T: Copy + PartialEq, L: AsRef<str>>(
        &mut self,
        ui: &mut Ui,
        search: &Search,
        row: Row,
        id: &'static str,
        options: &[(T, L)],
        current: T,
    ) -> Option<T> {
        if !search.shows(&row) {
            return None;
        }
        let index = options.iter().position(|(value, _)| *value == current);
        let selected = index
            .and_then(|index| options.get(index))
            .or_else(|| options.first())
            .map_or("", |(_, label)| label.as_ref());
        let mut picked = None;
        // Left or Right on the focused picker steps it, as picking it from the list would.
        if self.focused.is_some_and(|(focused, _)| focused == id)
            && let Some(step) = self.step.take()
        {
            let values: Vec<T> = options.iter().map(|(value, _)| *value).collect();
            picked = stepped(&values, current, step);
        }
        let mut focused = None;
        self::row(ui, row, |ui| {
            let response = egui::ComboBox::from_id_salt(id)
                .width(PICKER_W)
                .selected_text(selected)
                .show_ui(ui, |ui| {
                    for (value, label) in options {
                        if theme::selectable_label(ui, *value == current, label.as_ref()).clicked() {
                            picked = Some(*value);
                        }
                    }
                })
                .response;
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
        picked
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
    ui.horizontal(|ui| {
        ui.label(RichText::new(row.label).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(value);
        });
    });
    ui.add_space(ROW_GAP);
}

/// A paragraph in a group that isn't a setting, when the search shows it, with a link to the
/// section `link` names after it. Returns whether the link was clicked.
fn note(ui: &mut Ui, search: &Search, row: Row, link: Option<&str>) -> bool {
    if !search.shows(&row) {
        return false;
    }
    let font = text::font(ui, "sm");
    let description = RichText::new(row.description)
        .font(font.clone())
        .color(theme::tokens_of(ui).muted_foreground);
    let mut clicked = false;
    match link {
        // Wrapped with the link at the end of its last line. A wrapped row's lines are a
        // control's height apart, so a note without a link is a plain label.
        Some(link) => {
            ui.horizontal_wrapped(|ui| {
                ui.label(description);
                clicked = ui.link(RichText::new(format!("{link} ›")).font(font)).clicked();
            });
        }
        None => {
            ui.label(description);
        }
    }
    ui.add_space(ROW_GAP);
    clicked
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
