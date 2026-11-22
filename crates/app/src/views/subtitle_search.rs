//! The *Search for subtitles* dialog: the server's subtitle providers (the Open Subtitles
//! plugin and others) asked for a language, the results listed, and the one picked downloaded
//! into the item's library. The player's subtitle tab and the item's menu both open it, for
//! users whose policy allows subtitle management (`UserDto::can_manage_subtitles`).
//!
//! The server downloads in the background: its answer comes before the new stream does, so
//! once it has accepted the request the dialog reads the item's streams again until another
//! external subtitle shows up, and gives the item's versions to whoever opened the dialog
//! ([`Finished::Added`]); the player loads the new one into mpv.

use std::time::Duration;

use egui::{Key, Modifiers, Ui, Vec2, vec2};
use egui_material_icons::icons::{ICON_CHECK_CIRCLE, ICON_SUBTITLES};
use jellyfin::models::{
    BaseItemDto, Culture, ItemField, MediaSourceInfo, MediaStreamType, RemoteSubtitleInfo, UserDto,
};

use crate::i18n::{self, Language, tr};
use crate::task::Task;
use crate::theme;
use crate::toasts::Toast;
use crate::views::PageCx;
use crate::widgets::list_row::{self, Leading, ListRow};
use crate::widgets::text::Load;
use crate::widgets::{dialog, label, popup, select, surface};

/// Outside width, as the playlist dialogs'.
const WIDTH: f32 = 520.;
/// The results scroll once they're this tall.
const LIST_MAX_H: f32 = 320.;
const ROW_GAP: f32 = 6.;
const LANGUAGE_W: f32 = 240.;
/// How often, and how many times, the item is read again for the downloaded stream.
const POLL_EVERY: Duration = Duration::from_secs(1);
const POLL_TIMES: usize = 15;

/// What the dialog did this frame.
#[derive(Debug, Clone)]
pub enum Finished {
    /// Still open.
    Open,
    /// Closed without adding anything.
    Closed,
    /// A subtitle was added: the item's versions with it in them.
    Added(Vec<MediaSourceInfo>),
}

/// How a download ended.
enum Downloaded {
    Added(Vec<MediaSourceInfo>),
    /// The server took the request but no new stream appeared: it logs a failed download and
    /// answers success anyway.
    Missing,
}

pub struct SubtitleSearch {
    id: egui::Id,
    item_id: String,
    item_name: String,
    /// The language searched, a three-letter code.
    language: String,
    /// The server's languages as (code, name), once known.
    languages: Vec<(String, String)>,
    results: Load<Vec<RemoteSubtitleInfo>>,
    searching: Option<Task<jellyfin::Result<Vec<RemoteSubtitleInfo>>>>,
    cultures: Option<Task<jellyfin::Result<Vec<Culture>>>>,
    /// The download of the picked subtitle while it runs.
    working: Option<Task<jellyfin::Result<Downloaded>>>,
    highlighted: usize,
    keyboard: bool,
    scroll_to_highlight: bool,
}

impl SubtitleSearch {
    /// Opens the dialog on `item`, searching in the user's subtitle language.
    pub fn new(cx: &mut PageCx, item: &BaseItemDto) -> Self {
        let client = cx.services.session.client.clone();
        let cultures = Some(Task::spawn(cx.ctx, async move { client.cultures().await }));
        let mut dialog = Self {
            id: egui::Id::unique("subtitle_search"),
            item_id: item.id.clone(),
            item_name: item.name.clone(),
            language: initial_language(&cx.services.session.user, i18n::current()),
            languages: Vec::new(),
            results: Load::Loading,
            searching: None,
            cultures,
            working: None,
            highlighted: 0,
            keyboard: true,
            scroll_to_highlight: false,
        };
        dialog.search(cx);
        dialog
    }

    /// Looks for the language in `self.language`, dropping a search still running.
    fn search(&mut self, cx: &mut PageCx) {
        let client = cx.services.session.client.clone();
        let (item_id, language) = (self.item_id.clone(), self.language.clone());
        self.results = Load::Loading;
        self.highlighted = 0;
        self.searching = Some(Task::spawn(cx.ctx, async move {
            client.search_subtitles(&item_id, &language, false).await
        }));
    }

    pub fn item_id(&self) -> &str {
        &self.item_id
    }

    fn language_name(&self) -> &str {
        self.languages
            .iter()
            .find(|(code, _)| *code == self.language)
            .map_or(self.language.as_str(), |(_, name)| name.as_str())
    }

    pub fn show(&mut self, cx: &mut PageCx) -> Finished {
        if let Some(result) = self.cultures.as_mut().and_then(Task::poll) {
            self.cultures = None;
            if let Ok(cultures) = result {
                self.languages = crate::views::settings::language_list(cultures);
            }
        }
        if let Some(result) = self.searching.as_mut().and_then(Task::poll) {
            self.searching = None;
            self.results = match result {
                Ok(mut found) => {
                    // Exact matches first, then the provider's own order.
                    found.sort_by_key(|subtitle| !subtitle.is_hash_match);
                    Load::Loaded(found)
                }
                Err(jellyfin::Error::Forbidden) => Load::Failed(tr!("subtitle-search-forbidden")),
                Err(err) => Load::Failed(cx.services.handle_error(&err)),
            };
        }
        if let Some(result) = self.working.as_mut().and_then(Task::poll) {
            self.working = None;
            match result {
                Ok(Downloaded::Added(sources)) => {
                    cx.services.toast(Toast::success(tr!("subtitle-search-added")));
                    return Finished::Added(sources);
                }
                Ok(Downloaded::Missing) => {
                    cx.services.toast(Toast::error(tr!("subtitle-search-not-downloaded")));
                }
                Err(err) => {
                    let message = match err {
                        jellyfin::Error::Forbidden => tr!("subtitle-search-forbidden"),
                        err => cx.services.handle_error(&err),
                    };
                    cx.services.toast(Toast::error(message));
                }
            }
        }

        let ctx = cx.ctx;
        let busy = self.working.is_some();
        let mut chosen = None;
        let mut language = None;
        let id = self.id;
        let subtitle = self.item_name.clone();
        let modal = dialog::show(ctx, id, WIDTH, &tr!("subtitle-search-title"), Some(&subtitle), |ui| {
            let (enter, up, down) = ui.input_mut(|input| {
                (
                    input.consume_key(Modifiers::NONE, Key::Enter),
                    input.consume_key(Modifiers::NONE, Key::ArrowUp),
                    input.consume_key(Modifiers::NONE, Key::ArrowDown),
                )
            });
            language = self.language_select(ui, id);
            ui.add_space(surface::GAP_FIELD);

            let count = match &self.results {
                Load::Loaded(found) => found.len(),
                _ => 0,
            };
            self.highlighted = self.highlighted.min(count.saturating_sub(1));
            if count > 0 && !busy {
                if up || down {
                    let step = if down { 1 } else { -1 };
                    self.highlighted = popup::step(Some(self.highlighted), step, count).unwrap_or(0);
                    self.keyboard = true;
                    self.scroll_to_highlight = true;
                }
                if enter {
                    chosen = Some(self.highlighted);
                }
            }
            self.list(ui, busy, &mut chosen);
            if busy {
                ui.add_space(surface::GAP_FIELD);
                surface::waiting(ui, &tr!("subtitle-search-downloading"));
            }
            dialog::buttons(ui, dialog::cancel);
        });

        if let Some(language) = language.filter(|code| *code != self.language) {
            self.language = language;
            self.search(cx);
        }
        if let Some(index) = chosen {
            self.download(index, cx);
        }
        if modal.should_close() {
            Finished::Closed
        } else {
            Finished::Open
        }
    }

    /// The language picker; the language picked, when it changed.
    fn language_select(&self, ui: &mut Ui, id: egui::Id) -> Option<String> {
        let mut options: Vec<(&str, &str)> = self
            .languages
            .iter()
            .map(|(code, name)| (code.as_str(), name.as_str()))
            .collect();
        // A language the server doesn't list (a stored preference, or the list not here yet)
        // stays shown as its code.
        if !options.iter().any(|(code, _)| *code == self.language) {
            options.insert(0, (self.language.as_str(), self.language.as_str()));
        }
        let labels: Vec<&str> = options.iter().map(|(_, name)| *name).collect();
        let selected = options
            .iter()
            .position(|(code, _)| *code == self.language)
            .unwrap_or_default();
        let name = tr!("subtitle-search-language");
        let mut picked = None;
        ui.horizontal(|ui| {
            ui.label(label::muted(ui, name.as_str()));
            let (_, chosen) = select::select(ui, id.with("language"), LANGUAGE_W, &name, &labels, selected, false);
            picked = chosen.map(|chosen| options[chosen].0.to_owned());
        });
        picked
    }

    /// The results (or what stands in for them), in the height they need up to [`LIST_MAX_H`].
    fn list(&mut self, ui: &mut Ui, busy: bool, chosen: &mut Option<usize>) {
        let row_height = list_row::regular_height(ui);
        let rows = match &self.results {
            Load::Loaded(found) => found.len().max(1),
            _ => 1,
        };
        let height = (rows as f32 * (row_height + ROW_GAP) - ROW_GAP)
            .min(LIST_MAX_H)
            .max(row_height);
        ui.allocate_ui_with_layout(
            vec2(ui.available_width(), height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.spacing_mut().item_spacing.y = ROW_GAP;
                egui::ScrollArea::vertical().show(ui, |ui| self.lines(ui, busy, chosen));
            },
        );
    }

    fn lines(&mut self, ui: &mut Ui, busy: bool, chosen: &mut Option<usize>) {
        match &self.results {
            Load::Loading => {
                ui.label(label::muted(ui, tr!("subtitle-search-searching")));
            }
            Load::Failed(message) => {
                ui.label(label::muted(ui, message.clone()));
            }
            Load::Loaded(found) if found.is_empty() => {
                let language = self.language_name().to_owned();
                ui.label(label::muted(ui, tr!("subtitle-search-none", language = language)));
            }
            Load::Loaded(found) => {
                let pointer_moved = ui.input(|input| input.pointer.delta() != Vec2::ZERO);
                let mut hover = None;
                for (index, subtitle) in found.iter().enumerate() {
                    let highlighted = index == self.highlighted;
                    let response = subtitle_row(ui, subtitle, highlighted, highlighted && self.keyboard);
                    // The pointer takes the highlight, as in a menu, so Enter picks the row
                    // under it.
                    if response.hovered() && pointer_moved {
                        hover = Some(index);
                    }
                    if highlighted && std::mem::take(&mut self.scroll_to_highlight) {
                        response.scroll_to_me(Some(egui::Align::Center));
                    }
                    if response.clicked() && !busy {
                        *chosen = Some(index);
                    }
                }
                if let Some(index) = hover {
                    self.highlighted = index;
                    self.keyboard = false;
                }
            }
        }
    }

    /// Has the server download the result at `index`, then waits for the stream to appear.
    fn download(&mut self, index: usize, cx: &mut PageCx) {
        if self.working.is_some() {
            return;
        }
        let Load::Loaded(found) = &self.results else { return };
        let Some(subtitle) = found.get(index) else { return };
        let client = cx.services.session.client.clone();
        let (item_id, subtitle_id) = (self.item_id.clone(), subtitle.id.clone());
        self.working = Some(Task::spawn(cx.ctx, async move {
            let fields = [ItemField::MediaSources];
            let before = external_subtitles(&client.item(&item_id, &fields).await?);
            client.download_subtitle(&item_id, &subtitle_id).await?;
            for _ in 0..POLL_TIMES {
                tokio::time::sleep(POLL_EVERY).await;
                let item = client.item(&item_id, &fields).await?;
                if external_subtitles(&item) > before {
                    return Ok(Downloaded::Added(item.media_sources));
                }
            }
            Ok(Downloaded::Missing)
        }));
    }
}

/// How many external subtitle streams the item's versions have between them.
fn external_subtitles(item: &BaseItemDto) -> usize {
    item.media_sources
        .iter()
        .flat_map(|source| &source.media_streams)
        .filter(|stream| stream.kind == MediaStreamType::Subtitle && stream.is_external)
        .count()
}

/// The language the search starts in: the user's subtitle language, else the interface's. The
/// server wants three-letter codes, which is also how the preference is stored.
fn initial_language(user: &UserDto, interface: Language) -> String {
    let preferred = user
        .configuration
        .as_ref()
        .and_then(|config| config.subtitle_language_preference.as_deref())
        .map(str::trim)
        .filter(|code| !code.is_empty());
    match preferred {
        Some(code) => code.to_owned(),
        None => match interface {
            Language::English => "eng",
            Language::PortuguesePortugal => "por",
            Language::Spanish => "spa",
        }
        .to_owned(),
    }
}

/// What a result says about itself, after its name: whether it's made for this very file, the
/// provider, the format, and how often it was downloaded (or its rating).
fn detail(subtitle: &RemoteSubtitleInfo) -> String {
    let mut parts: Vec<String> = Vec::new();
    if subtitle.is_hash_match {
        parts.push(tr!("subtitle-search-exact"));
    }
    parts.extend(subtitle.provider_name.clone().filter(|name| !name.is_empty()));
    parts.extend(subtitle.format.as_ref().map(|format| format.to_uppercase()));
    match (subtitle.download_count, subtitle.community_rating) {
        (Some(count), _) if count > 0 => parts.push(tr!(
            "subtitle-search-downloads",
            count = count,
            digits = i18n::grouped(count.into())
        )),
        (_, Some(rating)) if rating > 0. => {
            parts.push(tr!(
                "subtitle-search-rating",
                rating = i18n::decimal(format!("{rating:.1}"))
            ));
        }
        _ => {}
    }
    parts.join(" · ")
}

fn subtitle_row(ui: &mut Ui, subtitle: &RemoteSubtitleInfo, highlighted: bool, ring: bool) -> egui::Response {
    let tokens = theme::tokens_of(ui);
    let leading = if subtitle.is_hash_match {
        Leading::Icon(ICON_CHECK_CIRCLE, tokens.brand)
    } else {
        Leading::Icon(ICON_SUBTITLES, tokens.ink_muted)
    };
    let detail = detail(subtitle);
    let label = tr!(
        "subtitle-search-row-label",
        name = subtitle.name.as_str(),
        detail = detail.as_str()
    );
    ListRow::new(&subtitle.name)
        .sunk()
        .leading(leading)
        .detail(Some(detail.as_str()).filter(|detail| !detail.is_empty()))
        .highlighted(highlighted)
        .ringed(ring)
        .no_hover()
        .label(&label)
        .show(ui)
        .row
}

#[cfg(test)]
mod tests {
    use jellyfin::models::UserDto;

    use super::{Language, initial_language};

    fn user(preference: Option<&str>) -> UserDto {
        serde_json::from_value(serde_json::json!({
            "Id": "u", "Name": "dev",
            "Configuration": { "SubtitleLanguagePreference": preference },
        }))
        .unwrap()
    }

    #[test]
    fn the_search_starts_in_the_users_language_or_the_interfaces() {
        assert_eq!(initial_language(&user(Some("fra")), Language::English), "fra");
        assert_eq!(initial_language(&user(Some("")), Language::PortuguesePortugal), "por");
        assert_eq!(initial_language(&user(None), Language::Spanish), "spa");
        assert_eq!(initial_language(&user(None), Language::English), "eng");
    }
}
