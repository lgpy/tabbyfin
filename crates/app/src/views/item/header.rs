//! The item page's header: the backdrop fading into the window's gradient, the poster (or an
//! episode's still), and beside it the title with its actions, the meta lines, the synopsis and
//! the tags; then the version picker under it.
//!
//! Playing the item is the one `brand` button (`button::brand_icon`); every other action is a
//! `button::quiet_icon` square, Watched and Favorite among them as toggles that turn `brand`
//! when on. Picked over round icon buttons by C11's mockup.

use egui::text::LayoutJob;
use egui::{Align, Align2, FontId, Label, Rect, Response, RichText, Sense, TextFormat, Ui, pos2, vec2};
use egui_material_icons::MaterialIcon;
use egui_material_icons::icons::{
    ICON_CHECK, ICON_CHECK_CIRCLE, ICON_FAVORITE, ICON_MORE_HORIZ, ICON_MOVIE, ICON_OPEN_IN_NEW, ICON_PLAY_ARROW,
    ICON_REPLAY, ICON_SHUFFLE, ICON_STAR,
};
use jellyfin::ImageRequest;
use jellyfin::models::{BaseItemDto, BaseItemKind, ImageType, MediaSourceInfo};

use super::{
    ACTION_GAP, ACTIONS, Asked, BACKDROP_FILL_W, BACKDROP_H, DETAIL_POSTER_H, DETAIL_POSTER_W, DETAIL_STILL_H, Fills,
    ItemAction, ItemData, PICKER_H, PICKER_LABEL_W, PICKER_W, PICKERS, TEXT_MAX_W, TITLE_MIN_W, TITLE_SIZE, meta_line,
};
use crate::browse::Position;
use crate::images::ImageCache;
use crate::item_menu::{is_favorite, is_played};
use crate::nav::Route;
use crate::theme::{self, text};
use crate::widgets::focus;
use crate::widgets::text::{episode_label, poster_image, timestamp};
use crate::widgets::{backdrop, button, image, select, tag};

/// The header: the backdrop with its fade, the poster, and the item's title, actions and text.
/// Returns its rectangle, for the cursor scrolling back to the actions.
pub(super) fn header(
    ui: &mut Ui,
    data: &ItemData,
    actions: &[ItemAction],
    selected: Option<Position>,
    fills: &Fills,
    images: &mut ImageCache,
    asked: &mut Asked,
) -> Rect {
    let item = &data.item;
    let top = ui.cursor().min;
    let backdrop = item.backdrop_image();
    if let Some(backdrop) = &backdrop {
        // Full width, past the page's padding: `main`'s backdrop ran edge to edge.
        let band = Rect::from_min_max(
            pos2(ui.clip_rect().left(), top.y),
            pos2(ui.clip_rect().right(), top.y + BACKDROP_H),
        );
        image::show_at_width(ui, images, band, Some(backdrop), Some(item), 0, BACKDROP_FILL_W);
        fade_to_background(ui, band);
    }
    ui.add_space(if backdrop.is_some() { BACKDROP_H - 160. } else { 24. });
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 24.;
        let (request, size) = header_image(item);
        let (poster, _) = ui.allocate_exact_size(size, Sense::hover());
        image::show(ui, images, poster, request.as_ref(), Some(item), image::RADIUS);
        ui.vertical(|ui| {
            if backdrop.is_some() {
                // Lines the text up beside the poster, as on `main`.
                ui.add_space(120.);
            }
            info_column(ui, data, actions, selected, fills, asked);
        });
    });
    Rect::from_min_max(top, pos2(ui.max_rect().right(), ui.cursor().min.y))
}

/// The header's picture and its size: the item's poster, or for an episode its own still in a
/// wide frame (an episode's image is a screen grab, which a tall poster would crop), and its
/// series' poster when it has no image of its own.
fn header_image(item: &BaseItemDto) -> (Option<ImageRequest>, egui::Vec2) {
    let poster = vec2(DETAIL_POSTER_W, DETAIL_POSTER_H);
    if item.kind != BaseItemKind::Episode {
        return (item.primary_image(), poster);
    }
    match item.image(ImageType::Primary) {
        // A tall image is a poster after all, and goes in the poster's frame.
        Some(own) => match item.primary_image_aspect_ratio {
            Some(ratio) if ratio < 1. => (Some(own), poster),
            // Episode stills are 16:9 when the server doesn't say.
            ratio => {
                let ratio = ratio.unwrap_or(16. / 9.).clamp(1., 2.4) as f32;
                (Some(own), vec2(DETAIL_STILL_H * ratio, DETAIL_STILL_H))
            }
        },
        None => (poster_image(item), poster),
    }
}

/// The title's column: the series line, the title and its actions, the meta line, genres,
/// studios, the synopsis and the tags.
fn info_column(
    ui: &mut Ui,
    data: &ItemData,
    actions: &[ItemAction],
    selected: Option<Position>,
    fills: &Fills,
    asked: &mut Asked,
) {
    let item = &data.item;
    let (ink, weak) = (theme::tokens_of(ui).ink, theme::tokens_of(ui).ink_muted);
    ui.spacing_mut().item_spacing.y = 8.;
    if matches!(item.kind, BaseItemKind::Episode | BaseItemKind::Season)
        && let Some(series) = &item.series_name
    {
        // `main`'s series line took the body size; the lines under the title are `text_sm`. It
        // opens the series, underlined under the pointer like a link.
        let label = Label::new(RichText::new(series).font(text::font(ui, "base")).color(weak));
        match &item.series_id {
            Some(id) => {
                let response = ui
                    .add(label.sense(Sense::click()))
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                if response.hovered() {
                    let rect = response.rect;
                    ui.painter()
                        .hline(rect.x_range(), rect.bottom(), egui::Stroke::new(1., weak));
                }
                if response.clicked() {
                    asked.open = Some(Route::Item {
                        id: id.clone(),
                        name: series.clone(),
                    });
                }
            }
            None => {
                ui.add(label);
            }
        }
    }

    let episode = data
        .up_next
        .as_ref()
        .map(episode_label)
        .filter(|label| !label.is_empty());
    let actions_width = actions_width(ui, data, actions, episode.as_deref());
    // `main`'s 40-point title: a fixed size, so the text size setting doesn't move the header.
    let title_font = FontId::new(TITLE_SIZE, text::font(ui, "base").family);
    let available = ui.available_width();
    let gap = 16.;
    if available >= TITLE_MIN_W + gap + actions_width {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            let title_w = available - gap - actions_width;
            ui.vertical(|ui| {
                ui.set_max_width(title_w);
                ui.add(Label::new(RichText::new(&item.name).font(title_font.clone()).color(ink)).wrap());
            });
            ui.add_space((ui.available_width() - actions_width).max(0.));
            actions_row(ui, data, actions, selected, episode.as_deref(), fills, asked);
        });
    } else {
        // A narrow window: the actions go under the title rather than squeeze it to a word a
        // line.
        ui.add(Label::new(RichText::new(&item.name).font(title_font).color(ink)).wrap());
        actions_row(ui, data, actions, selected, episode.as_deref(), fills, asked);
    }

    let meta = meta_line(item);
    if !meta.is_empty() {
        let font = text::font(ui, "sm");
        // The text font has no ★, so the rating's star is drawn from the icon font.
        let mut job = LayoutJob::default();
        let format = |font_id| TextFormat {
            font_id,
            color: weak,
            valign: Align::Center,
            ..Default::default()
        };
        for (index, part) in meta.split('★').enumerate() {
            if index > 0 {
                job.append(
                    ICON_STAR.codepoint,
                    0.,
                    format(FontId::new(font.size, ICON_STAR.font_family())),
                );
            }
            job.append(part, 0., format(font.clone()));
        }
        ui.label(job);
    }
    if !item.genres.is_empty() {
        ui.label(
            RichText::new(item.genres.join(", "))
                .font(text::font(ui, "sm"))
                .color(weak),
        );
    }
    if !item.studios.is_empty() {
        let studios: Vec<&str> = item.studios.iter().map(|studio| studio.name.as_str()).collect();
        ui.label(RichText::new(studios.join(", ")).font(text::font(ui, "sm")).color(weak));
    }
    if let Some(overview) = &item.overview
        && !overview.is_empty()
    {
        ui.add_space(8.);
        ui.scope(|ui| {
            ui.set_max_width(ui.available_width().min(TEXT_MAX_W));
            // The synopsis is `main`'s `text_base`, a step above the meta lines.
            let synopsis = RichText::new(overview.clone()).font(text::font(ui, "base")).color(ink);
            ui.add(Label::new(synopsis).wrap());
        });
    }
    if !item.tags.is_empty() {
        ui.add_space(16.);
        ui.scope(|ui| {
            ui.set_max_width(ui.available_width().min(TEXT_MAX_W));
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6., 6.);
                for name in &item.tags {
                    // A pill doesn't wrap as a unit, so its label would break per letter in the
                    // row's leftover space; start a new row when the whole pill doesn't fit.
                    if ui.available_size_before_wrap().x < tag::width(ui, name) {
                        ui.end_row();
                    }
                    tag::show(ui, name);
                }
            });
        });
    }
}

/// What an action button shows: its icon, its tooltip, and the label playing it carries.
struct ActionLook {
    icon: MaterialIcon,
    tooltip: String,
    label: Option<String>,
    primary: bool,
}

fn action_look(action: ItemAction, item: &BaseItemDto, episode: Option<&str>) -> ActionLook {
    let labelled = |verb: &str| {
        Some(match episode {
            Some(episode) => format!("{verb} {episode}"),
            None => verb.to_owned(),
        })
    };
    match action {
        ItemAction::Resume(position) => ActionLook {
            icon: ICON_PLAY_ARROW,
            tooltip: format!("Resume from {}", timestamp(position.into())),
            label: labelled("Resume"),
            primary: true,
        },
        ItemAction::PlayFromStart => ActionLook {
            icon: ICON_REPLAY,
            tooltip: "Play from beginning".to_owned(),
            label: None,
            primary: false,
        },
        ItemAction::Play => ActionLook {
            icon: ICON_PLAY_ARROW,
            tooltip: "Play".to_owned(),
            label: labelled("Play"),
            primary: true,
        },
        ItemAction::PlayAll => ActionLook {
            icon: ICON_PLAY_ARROW,
            tooltip: "Play all".to_owned(),
            label: Some("Play all".to_owned()),
            primary: true,
        },
        ItemAction::Shuffle => ActionLook {
            icon: ICON_SHUFFLE,
            tooltip: "Shuffle".to_owned(),
            label: None,
            primary: false,
        },
        ItemAction::Trailer => ActionLook {
            icon: if item.local_trailer_count.unwrap_or(0) > 0 {
                ICON_MOVIE
            } else {
                ICON_OPEN_IN_NEW
            },
            tooltip: "Trailer".to_owned(),
            label: None,
            primary: false,
        },
        ItemAction::More => ActionLook {
            icon: ICON_MORE_HORIZ,
            tooltip: "More".to_owned(),
            label: None,
            primary: false,
        },
        // Drawn as toggles, which take their icon from the item's state, and the More menu's
        // entries, which have their own icons.
        ItemAction::TogglePlayed
        | ItemAction::ToggleFavorite
        | ItemAction::AddToPlaylist
        | ItemAction::EditPlaylist
        | ItemAction::DeletePlaylist => ActionLook {
            icon: ICON_MORE_HORIZ,
            tooltip: String::new(),
            label: None,
            primary: false,
        },
    }
}

/// How wide the actions row is, for the title to know what's left of its line. A pixel over is
/// fine: the row right-aligns itself in what the title leaves.
fn actions_width(ui: &Ui, data: &ItemData, actions: &[ItemAction], episode: Option<&str>) -> f32 {
    let mut width = 0.;
    for (index, &action) in actions.iter().enumerate() {
        if index > 0 {
            width += ACTION_GAP;
        }
        let look = action_look(action, &data.item, episode);
        width += match (look.primary, &look.label) {
            (true, Some(label)) => button::brand_icon_width(ui, label),
            _ => button::COZY_HEIGHT,
        };
    }
    width
}

/// The buttons beside the title: playing it, the one `brand` button with its label, then quiet
/// icon buttons, Watched and Favorite toggles among them, and the More menu.
fn actions_row(
    ui: &mut Ui,
    data: &ItemData,
    actions: &[ItemAction],
    selected: Option<Position>,
    episode: Option<&str>,
    fills: &Fills,
    asked: &mut Asked,
) {
    let item = &data.item;
    let played = is_played(item);
    let favorite = is_favorite(item);
    let now = ui.input(|input| input.time);
    // With the animations setting off the icons are drawn at their size, at once.
    let animations = theme::animations(ui.ctx());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = ACTION_GAP;
        for (index, &action) in actions.iter().enumerate() {
            let ringed = selected
                == Some(Position {
                    section: ACTIONS,
                    index,
                });
            let response = match action {
                ItemAction::TogglePlayed => {
                    let icon = if played {
                        ICON_CHECK_CIRCLE
                    } else {
                        ICON_CHECK.outlined()
                    };
                    let scale = if animations { fills.played.scale(now) } else { 1. };
                    button::quiet_icon(ui, icon, "Watched", Some(played), scale, ringed)
                }
                ItemAction::ToggleFavorite => {
                    let icon = if favorite {
                        ICON_FAVORITE
                    } else {
                        ICON_FAVORITE.outlined()
                    };
                    let scale = if animations { fills.favorite.scale(now) } else { 1. };
                    button::quiet_icon(ui, icon, "Favorite", Some(favorite), scale, ringed)
                }
                _ => {
                    let look = action_look(action, item, episode);
                    let response = match (look.primary, &look.label) {
                        (true, Some(label)) => {
                            button::brand_icon(ui, look.icon, label, ringed).on_hover_text(look.tooltip.as_str())
                        }
                        _ => button::quiet_icon(ui, look.icon, &look.tooltip, None, 1., ringed),
                    };
                    if action == ItemAction::More {
                        asked.more = Some(response.clone());
                    }
                    response
                }
            };
            if response.clicked() {
                asked.action = Some(action);
            }
        }
    });
    if animations && (fills.played.popping(now) || fills.favorite.popping(now)) {
        // The icons are still growing into place: keep the frames coming.
        ui.ctx().request_repaint();
    }
}

/// The version label's column and the picker, when the item has more than one version.
pub(super) fn pickers_row(
    ui: &mut Ui,
    data: &ItemData,
    selected: Option<Position>,
    version: usize,
    asked: &mut Asked,
) -> Rect {
    let top = ui.cursor().min;
    let sources = &data.item.media_sources;
    let current = version.min(sources.len() - 1);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 16.;
        let (label, _) = ui.allocate_exact_size(vec2(PICKER_LABEL_W, PICKER_H), Sense::hover());
        ui.painter().text(
            label.left_center(),
            Align2::LEFT_CENTER,
            "Version",
            text::font(ui, "sm"),
            theme::tokens_of(ui).muted_foreground,
        );
        let ringed = selected
            == Some(Position {
                section: PICKERS,
                index: 0,
            });
        let name = version_label(&sources[current], current);
        let response = picker(ui, &name, ringed);
        asked.version_picker = Some(response);
    });
    Rect::from_min_max(top, pos2(ui.max_rect().right(), ui.cursor().min.y))
}

/// The version select (`widgets::select`), with the cursor's ring round it. Clicking it opens
/// the list of versions; the keyboard changes the choice on the cursor instead.
fn picker(ui: &mut Ui, name: &str, ringed: bool) -> Response {
    let response = select::button(ui, vec2(PICKER_W, PICKER_H), name, "Version", false);
    if ringed {
        focus::ring_widget(ui, &response);
    }
    response
}

pub(super) fn version_label(source: &MediaSourceInfo, index: usize) -> String {
    source.name.clone().unwrap_or_else(|| format!("Version {}", index + 1))
}

/// Fades the backdrop image into the page at its bottom, as `main`'s gradient did: into the
/// window's backdrop at that height, so the picture melts into the gradient rather than ending
/// on a flat band of colour.
fn fade_to_background(ui: &Ui, band: Rect) {
    // `main`'s first stop: the background at 30 %, so the image shows through at the top.
    let window = ui.ctx().content_rect();
    backdrop::fade_into(ui.painter(), band, window, 77. / 255., theme::tokens_of(ui));
}
