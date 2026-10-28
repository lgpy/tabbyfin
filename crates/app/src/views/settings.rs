//! Settings: appearance, playback, subtitles, storage, account and about, as headed groups on
//! one scrolling page.
//!
//! `main` had gpui-kit's settings panel: a page per topic in a searchable sidebar, with its
//! keyword search, reset buttons and collapsible groups. This is one page with every section
//! open, since the features come first and the look is R1's; what carries over is what each row
//! says and does. Every option keeps the same config key and value as on `main` (a change loads
//! the config fresh, changes it and saves it, so it can't write over what playback or the window
//! saved meanwhile), the same choices in the same order, the default that applies while the file
//! leaves the key out, and what the change takes effect on: the theme, text size and animations
//! apply at once through `AppEvent::AppearanceChanged`, the subtitle size goes to the playback
//! manager while a session runs, and the rest are read when what they configure next starts
//! (a playback session, the player page, the update check at launch).
//!
//! The page isn't on the browse cursor, as on `main`: [`Page::scope`] is `None`, so egui's own
//! focus handles Tab, the arrows in a picker and Space on a switch, and Esc still goes back.

use std::path::Path;

use egui::{RichText, Ui};
use egui_material_icons::MaterialIcon;
use egui_material_icons::icons::{
    ICON_ACCOUNT_CIRCLE, ICON_INFO, ICON_PALETTE, ICON_PLAY_CIRCLE, ICON_STORAGE, ICON_SUBTITLES,
};

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

/// Subtitle sizes offered here; the player's menu sets any size in 10 % steps.
const SUBTITLE_SCALES: [f64; 6] = [0.75, 1., 1.25, 1.5, 1.75, 2.];

/// Max streaming bitrate choices in Mbit/s; 0 is no limit.
const BITRATES_MBPS: [i32; 10] = [0, 120, 60, 40, 20, 10, 8, 4, 2, 1];
/// The bitrate choice that measures the connection instead.
const AUTOMATIC: i32 = -1;

/// The page's top and bottom padding, and the width its form is held to, centred in what's
/// left: rows of labels and controls stretched over a wide window read badly.
const PAGE_PADDING: f32 = 24.;
const CONTENT_W: f32 = 820.;
/// The pickers' width beside their label (`main`'s 220 px).
const PICKER_W: f32 = 220.;
/// The gaps between a group's heading and its box, between the rows in it, and between groups.
const HEADING_GAP: f32 = 4.;
const ROW_GAP: f32 = 10.;
const SECTION_GAP: f32 = 24.;
/// The size of the icons beside the group headings.
const HEADING_ICON: f32 = 20.;

pub struct SettingsPage {
    /// The config as this page last read or wrote it, for the controls' values. A change reads
    /// the file again rather than writing this copy back, as `main`'s `save` did.
    config: Config,
    /// What a button started (clearing the image cache, opening the logs folder), until its
    /// outcome is a toast.
    job: Option<Task<Job>>,
    /// Left and Right on a focused picker.
    pickers: Pickers,
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
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let full = ui.available_width();
            let column = full.min(CONTENT_W);
            let side = ((full - column) * 0.5).max(0.);
            // A nested `vertical` Ui, not `with_layout` around the whole column: a frame's
            // contents lay out the way its parent does, and they have to stack downwards.
            ui.horizontal_top(|ui| {
                ui.add_space(side);
                ui.vertical(|ui| {
                    ui.set_width(column);
                    self.groups(ui, cx);
                });
            });
        });
    }
}

impl SettingsPage {
    /// The page's groups, each a heading over a box of rows.
    fn groups(&mut self, ui: &mut Ui, cx: &mut PageCx) {
        ui.add_space(PAGE_PADDING);
        section(ui, ICON_PALETTE, "Appearance", |ui| self.appearance(ui, cx));
        section(ui, ICON_PLAY_CIRCLE, "Playback", |ui| self.playback(ui, cx));
        section(ui, ICON_SUBTITLES, "Subtitles", |ui| self.subtitles(ui, cx));
        section(ui, ICON_STORAGE, "Storage", |ui| self.storage(ui, cx));
        section(ui, ICON_ACCOUNT_CIRCLE, "Account", |ui| self.account(ui, cx));
        section(ui, ICON_INFO, "About", |ui| self.about(ui, cx));
        ui.add_space(PAGE_PADDING);
    }

    fn appearance(&mut self, ui: &mut Ui, cx: &mut PageCx) {
        let current = self.config.theme;
        let picked = self.choose(
            ui,
            Row {
                label: "Theme",
                description: "Dark, light, or the desktop's colour scheme. Applies at once.",
                key: "theme",
                unset: "dark",
            },
            "settings_theme",
            &THEMES,
            current,
        );
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
        let picked = self.choose(
            ui,
            Row {
                label: "Text size",
                description: "Scales the app's text and controls, not its layout. Applies at once.",
                key: "text_size",
                unset: "the system text size",
            },
            "settings_text_size",
            &text_sizes,
            current,
        );
        if let Some(size) = picked {
            self.save_appearance(cx, |config| config.text_size = size);
        }

        let current = self.config.animations.unwrap_or(true);
        let flipped = toggle(
            ui,
            Row {
                label: "Animations",
                description: "Animations and smooth scrolling. Applies at once.",
                key: "animations",
                unset: "on",
            },
            current,
        );
        if let Some(on) = flipped {
            self.save_appearance(cx, |config| config.animations = Some(on));
        }

        // No toggle of our own, as on `main`: the web client's profile decides.
        note(
            ui,
            "Home's sections and the libraries they show follow your Jellyfin profile (Settings → Home in the \
             web client).",
        );
    }

    fn playback(&mut self, ui: &mut Ui, _cx: &mut PageCx) {
        note(
            ui,
            "Audio and subtitle languages, and whether subtitles show by default, follow your Jellyfin profile \
             (Settings → Playback in the web client).",
        );

        let current = self.config.autoplay_next_episode.unwrap_or(true);
        let flipped = toggle(
            ui,
            Row {
                label: "Play the next episode automatically",
                description: "When an episode ends, so the player stays open. From the next item on.",
                key: "autoplay_next_episode",
                unset: "on",
            },
            current,
        );
        if let Some(on) = flipped {
            self.save(|config| config.autoplay_next_episode = Some(on));
        }

        let current = self.config.auto_skip_intro.unwrap_or(false);
        let flipped = toggle(
            ui,
            Row {
                label: "Skip intros and recaps automatically",
                description: "Each intro or recap once as it starts, never in a SyncPlay group. From the next \
                              item on.",
                key: "auto_skip_intro",
                unset: "off",
            },
            current,
        );
        if let Some(on) = flipped {
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
        let picked = self.choose(
            ui,
            Row {
                label: "Max streaming bitrate",
                description: "Videos above this bitrate are transcoded by the server. Lower it for slow \
                              connections. Automatic measures the connection before playing (at most every 10 \
                              minutes) and uses 80% of it, saving automatic_bitrate and keeping the cap for when \
                              it's turned off. Read when the next item starts.",
                key: "max_streaming_bitrate",
                unset: "no limit",
            },
            "settings_bitrate",
            &bitrates,
            current,
        );
        if let Some(mbps) = picked {
            self.save(|config| apply_bitrate(config, mbps));
        }

        let current = self.config.prefer_hevc.unwrap_or(false);
        let flipped = toggle(
            ui,
            Row {
                label: "Ask for HEVC when the server transcodes",
                description: "HEVC needs half the bandwidth of H.264 for the same picture, but is much harder for \
                              the server to encode. Used only if the server allows it (Dashboard → Playback → \
                              Transcoding). Read when the next item starts.",
                key: "prefer_hevc",
                unset: "off",
            },
            current,
        );
        if let Some(on) = flipped {
            self.save(|config| config.prefer_hevc = Some(on));
        }
    }

    fn subtitles(&mut self, ui: &mut Ui, cx: &mut PageCx) {
        note(
            ui,
            "For text subtitles such as SRT; styled (ASS) subtitles keep their own look and only change size. \
             The player's settings menu also changes the size while watching.",
        );

        let current = self.config.subtitle_color.unwrap_or_default();
        let picked = self.choose(
            ui,
            Row {
                label: "Colour",
                description: "Of text subtitles. Read when the next item starts.",
                key: "subtitle_color",
                unset: "white",
            },
            "settings_subtitle_color",
            &SUBTITLE_COLORS,
            current,
        );
        if let Some(color) = picked {
            self.save(|config| config.subtitle_color = Some(color));
        }

        let current = self.config.subtitle_style.unwrap_or_default();
        let picked = self.choose(
            ui,
            Row {
                label: "Style",
                description: "An outline, or a translucent dark box, behind text subtitles. Read when the next \
                              item starts.",
                key: "subtitle_style",
                unset: "outline",
            },
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
        let picked = self.choose(
            ui,
            Row {
                label: "Size",
                description: "Shared with the player's settings menu. Goes to the playback manager at once, so a \
                              running session changes with it.",
                key: "subtitle_scale",
                unset: "100 %",
            },
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
        let images = config::cache_dir().join("images").display().to_string();
        let mut clear = false;
        row(
            ui,
            Row {
                label: "Image cache",
                description: "Posters and backdrops are kept here and reload as pages ask for them again.",
                key: &images,
                unset: "",
            },
            |ui| {
                clear = ui.add_enabled(!busy, egui::Button::new("Clear")).clicked();
            },
        );
        if clear {
            self.clear_image_cache(cx);
        }

        let logs = config::log_dir().display().to_string();
        let mut open = false;
        row(
            ui,
            Row {
                label: "Logs",
                description: "A log file is written each day. Attach it to bug reports.",
                key: &logs,
                unset: "",
            },
            |ui| {
                open = ui.add_enabled(!busy, egui::Button::new("Open folder")).clicked();
            },
        );
        if open {
            self.open_logs(cx);
        }
    }

    fn account(&mut self, ui: &mut Ui, cx: &mut PageCx) {
        let session = &cx.services.session;
        info(ui, "User", &session.user.name);
        let server = session.server.server_name.clone().unwrap_or_else(|| "Jellyfin".into());
        info(ui, "Server", &server);
        info(ui, "Address", session.client.base_url().as_str());
        let version = session.server.version.clone().unwrap_or_else(|| "?".into());
        info(ui, "Jellyfin version", &version);
    }

    fn about(&mut self, ui: &mut Ui, _cx: &mut PageCx) {
        info(ui, "Version", env!("CARGO_PKG_VERSION"));

        let current = self.config.check_for_updates.unwrap_or(true);
        let flipped = toggle(
            ui,
            Row {
                label: "Tell me when a new version is out",
                description: "Checks the GitHub releases once a day, when the app starts.",
                key: "check_for_updates",
                unset: "on",
            },
            current,
        );
        if let Some(on) = flipped {
            self.save(|config| config.check_for_updates = Some(on));
        }
    }
}

/// A group on the page: its heading, and a box around its rows.
fn section(ui: &mut Ui, icon: MaterialIcon, title: &str, contents: impl FnOnce(&mut Ui)) {
    ui.add_space(SECTION_GAP);
    let tokens = theme::tokens_of(ui);
    ui.horizontal(|ui| {
        ui.label(icon.rich_text().size(HEADING_ICON).color(tokens.muted_foreground));
        ui.label(RichText::new(title).heading());
    });
    ui.add_space(HEADING_GAP);
    // `main`'s `GroupBox`: its `group_box` colour, the border and the controls' radius.
    egui::Frame::group(ui.style())
        .fill(tokens.group_box)
        .stroke(egui::Stroke::new(1., tokens.border))
        .corner_radius(egui::CornerRadius::same(tokens.radius))
        .inner_margin(12)
        .show(ui, |ui| {
            // The box spans the form's width, not just its widest row.
            ui.set_width(ui.available_width());
            contents(ui);
        });
}

/// What a row shows besides its control: the setting's name, what it does (when it takes
/// effect included), the config key it writes, and the value that applies while the file
/// doesn't have that key. [`row`] draws them around the control.
#[derive(Clone, Copy)]
struct Row<'a> {
    label: &'a str,
    description: &'a str,
    /// The config key, or what the row is about when nothing is saved for it.
    key: &'a str,
    /// The value while the key is unset; empty for a row with nothing to save.
    unset: &'a str,
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
        ui.label(
            RichText::new(metadata)
                .font(text::font(ui, "xs"))
                .monospace()
                .color(theme::tokens_of(ui).muted_foreground),
        );
    }
    ui.add_space(ROW_GAP);
}

/// A switch for a flag setting: returns the value it was flipped to, if it was.
fn toggle(ui: &mut Ui, row: Row, value: bool) -> Option<bool> {
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
/// [`SettingsPage::choose`] as the same value a click gives, so the page saves it the same way.
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
}

/// The choice one step (Left is -1, Right is 1) from `current` in `options`, stopping at either
/// end; `None` when it can't move, or when `current` isn't one of the choices (a bitrate set by
/// hand: the picker shows the first one, as `main`'s select did, and Left and Right stay put).
fn stepped<T: Copy + PartialEq>(options: &[T], current: T, step: i32) -> Option<T> {
    let index = options.iter().position(|value| *value == current)?;
    let next = (index as i32 + step).clamp(0, options.len() as i32 - 1) as usize;
    (next != index).then(|| options[next])
}

impl SettingsPage {
    /// A picker over `options`, with `current` selected: returns the choice when one was picked
    /// in the list or stepped to with Left or Right.
    fn choose<T: Copy + PartialEq, L: AsRef<str>>(
        &mut self,
        ui: &mut Ui,
        row: Row,
        id: &'static str,
        options: &[(T, L)],
        current: T,
    ) -> Option<T> {
        let index = options.iter().position(|(value, _)| *value == current);
        let selected = index
            .and_then(|index| options.get(index))
            .or_else(|| options.first())
            .map_or("", |(_, label)| label.as_ref());
        let mut picked = None;
        // Left or Right on the focused picker steps it, as picking it from the list would.
        if self.pickers.focused.is_some_and(|(focused, _)| focused == id)
            && let Some(step) = self.pickers.step.take()
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
                        if ui.selectable_label(*value == current, label.as_ref()).clicked() {
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
            self.pickers.focused = Some((id, focused_id));
        }
        picked
    }
}

/// A row that's only information: there's nothing to change, so no control and no config key.
fn info(ui: &mut Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(value);
        });
    });
    ui.add_space(ROW_GAP);
}

/// A paragraph in a group that isn't a setting.
fn note(ui: &mut Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .font(crate::theme::text::font(ui, "sm"))
            .color(theme::tokens_of(ui).muted_foreground),
    );
    ui.add_space(ROW_GAP);
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
    use crate::config::{Config, ThemePreference};

    use super::{
        AUTOMATIC, BITRATES_MBPS, SUBTITLE_SCALES, THEMES, apply_bitrate, bitrate_choice, stepped, subtitle_scales,
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
}
