//! The item page's header on a phone in portrait (`layout::Size::Compact`), laid out like a
//! card: the backdrop dimmed in a band, the poster (or an episode's still, as wide as the page)
//! centred on it, and under it, centred, the series line, the title, the meta line and the
//! genres. Then playing it across the page, with how much is left under Resume; the other
//! actions as round buttons with their names under them; the synopsis's first lines with *More*
//! for the rest; and the crew and studio as label and value rows.

use egui::accesskit::Role;
use egui::text::{LayoutJob, TextWrapping};
use egui::{Align, FontId, Label, Rect, Response, RichText, Sense, Stroke, TextFormat, Ui, WidgetInfo, pos2, vec2};
use egui_material_icons::MaterialIcon;
use egui_material_icons::icons::{ICON_CHECK, ICON_CHECK_CIRCLE, ICON_FAVORITE};
use jellyfin::Ticks;
use jellyfin::models::{BaseItemDto, BaseItemKind};

use super::header::{DownloadLook, action_look, bar_wash, header_image, meta_job};
use super::{
    ACTIONS, Asked, BACKDROP_FILL_W, COMPACT_BAND_H, COMPACT_IMAGE_TOP, COMPACT_OVERVIEW_ROWS, COMPACT_PICKER_LABEL_W,
    COMPACT_POSTER_H, COMPACT_POSTER_W, COMPACT_TITLE_SIZE, Fills, ItemAction, ItemData, meta_line,
};
use crate::browse::Position;
use crate::images::ImageCache;
use crate::item_menu::{is_favorite, is_played};
use crate::nav::Route;
use crate::theme::{self, text};
use crate::views::UnderBar;
use crate::widgets::focus::{self, Place};
use crate::widgets::text::{episode_label, runtime_text};
use crate::widgets::{backdrop, button, image, pill, round_button};

/// How much of the page's colour lies over the backdrop, so the poster stands off it.
const DIM: f32 = 0.55;
/// A round action's cell (the button and its name under it), the button's radius and its icon.
const ACTION_CELL: f32 = 72.;
const ACTION_RADIUS: f32 = 24.;
const ACTION_ICON: f32 = 22.;
/// What lifts the poster off the band.
const SHADOW: egui::epaint::Shadow = egui::epaint::Shadow {
    offset: [0, 6],
    blur: 24,
    spread: 0,
    color: egui::Color32::from_black_alpha(110),
};

/// The header; returns its rectangle, for the cursor scrolling back to the actions.
#[allow(clippy::too_many_arguments)]
pub(super) fn header(
    ui: &mut Ui,
    data: &ItemData,
    actions: &[ItemAction],
    selected: Option<Position>,
    fills: &Fills,
    download: &DownloadLook,
    images: &mut ImageCache,
    under_bar: Option<&UnderBar>,
    overview_open: bool,
    asked: &mut Asked,
) -> Rect {
    let item = &data.item;
    let tokens = theme::tokens_of(ui);
    let top = ui.cursor().min;
    let backdrop = item.backdrop_image();
    if let Some(backdrop) = &backdrop {
        // Edge to edge, and up behind the top bar as the desktop's banner is.
        let lift = under_bar.map_or(0., |under| under.bar.height());
        let band = Rect::from_min_max(
            pos2(ui.clip_rect().left(), top.y - lift),
            pos2(ui.clip_rect().right(), top.y + COMPACT_BAND_H),
        );
        let window = ui.ctx().content_rect();
        let mut shapes = image::banner(ui, images, band, Some(backdrop), Some(item), BACKDROP_FILL_W);
        shapes.push(backdrop::wash(band, window, DIM, DIM, tokens));
        // A point past the band, over the picture's anti-aliased bottom edge, which a phone's
        // fractional pixel scale leaves as a faint line below an opaque fade the band's size.
        shapes.push(backdrop::fade_into(
            band.with_max_y(band.bottom() + 1.),
            window,
            0.,
            tokens,
        ));
        match under_bar {
            Some(under) => {
                shapes.extend(bar_wash(ui, under.bar));
                under.paint(ui, shapes);
            }
            None => ui.painter().extend(shapes),
        }
    }
    ui.add_space(if backdrop.is_some() { COMPACT_IMAGE_TOP } else { 16. });

    // The poster, or an episode's still as wide as the page.
    let (request, size) = header_image(item);
    let width = ui.available_width();
    let size = if size.x > size.y {
        vec2(width, width * size.y / size.x)
    } else {
        vec2(COMPACT_POSTER_W, COMPACT_POSTER_H)
    };
    let (row, _) = ui.allocate_exact_size(vec2(width, size.y), Sense::hover());
    let picture = Rect::from_center_size(row.center(), size);
    ui.painter().add(SHADOW.as_shape(picture, image::RADIUS));
    image::show(ui, images, picture, request.as_ref(), Some(item), image::RADIUS);
    ui.add_space(14.);

    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 6.;
        titles(ui, item, asked);
        ui.add_space(8.);
        let episode = data
            .up_next
            .as_ref()
            .map(episode_label)
            .filter(|label| !label.is_empty());
        let primary = actions.iter().position(|&action| {
            let look = action_look(action, item, episode.as_deref());
            look.primary && look.label.is_some()
        });
        if let Some(index) = primary {
            let action = actions[index];
            let look = action_look(action, item, episode.as_deref());
            let label = look.label.unwrap_or_default();
            let width = ui.available_width();
            // A centred layout centres the button's icon and label in its width too.
            let response = ui
                .vertical_centered(|ui| button::brand_icon_wide(ui, look.icon, &label, ringed(selected, index), width))
                .inner;
            if response.clicked() {
                asked.action = Some(action);
            }
            if let ItemAction::Resume(position) = action {
                let runtime = data.up_next.as_ref().unwrap_or(item).run_time_ticks;
                if let Some(runtime) = runtime.filter(|runtime| runtime.0 > 0) {
                    progress_line(ui, position, runtime);
                }
            }
            ui.add_space(4.);
        }
        round_actions(ui, data, actions, primary, selected, fills, download, asked);
        overview(ui, item, overview_open, asked);
        details(ui, item);
        tags(ui, item);
    });
    Rect::from_min_max(top, pos2(ui.max_rect().right(), ui.cursor().min.y))
}

fn ringed(selected: Option<Position>, index: usize) -> bool {
    selected
        == Some(Position {
            section: ACTIONS,
            index,
        })
}

/// The series line (opening the series), the title, the meta line and the genres, centred.
fn titles(ui: &mut Ui, item: &BaseItemDto, asked: &mut Asked) {
    let tokens = theme::tokens_of(ui);
    ui.vertical_centered(|ui| {
        if matches!(item.kind, BaseItemKind::Episode | BaseItemKind::Season)
            && let Some(series) = &item.series_name
        {
            match &item.series_id {
                Some(id) => {
                    if button::link(ui, series, "base").clicked() {
                        asked.open = Some(Route::Item {
                            id: id.clone(),
                            name: series.clone(),
                        });
                    }
                }
                None => {
                    ui.label(
                        RichText::new(series)
                            .font(text::font(ui, "base"))
                            .color(tokens.ink_muted),
                    );
                }
            }
        }
        // A fixed size, as the desktop's 40, so the text size setting doesn't move the header.
        let title_font = FontId::new(COMPACT_TITLE_SIZE, text::font(ui, "base").family);
        ui.add(Label::new(RichText::new(&item.name).font(title_font).color(tokens.ink)).wrap());
        let meta = meta_line(item);
        if !meta.is_empty() {
            let mut job = meta_job(ui, &meta, tokens.ink_muted);
            job.halign = Align::Center;
            ui.label(job);
        }
        if !item.genres.is_empty() {
            let genres = RichText::new(item.genres.join(" · "))
                .font(text::font(ui, "sm"))
                .color(tokens.ink_muted);
            ui.add(Label::new(genres).wrap());
        }
    });
}

/// A thin bar of how far the resume point is through the runtime, and what's left after it.
fn progress_line(ui: &mut Ui, position: Ticks, runtime: Ticks) {
    let tokens = theme::tokens_of(ui);
    let fraction = (position.0 as f64 / runtime.0 as f64).clamp(0., 1.) as f32;
    let left = format!("{} left", runtime_text(Ticks((runtime.0 - position.0).max(0))));
    let galley = ui
        .painter()
        .layout_no_wrap(left, text::font(ui, "xs"), tokens.ink_muted);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), galley.size().y), Sense::hover());
    let track_w = (rect.width() - galley.size().x - 12.).max(0.);
    let track = Rect::from_center_size(pos2(rect.left() + track_w / 2., rect.center().y), vec2(track_w, 4.));
    ui.painter().rect_filled(track, 2, tokens.inset_hover);
    let filled = Rect::from_min_max(track.min, pos2(track.left() + track.width() * fraction, track.bottom()));
    ui.painter().rect_filled(filled, 2, tokens.brand);
    ui.painter().galley(
        pos2(rect.right() - galley.size().x, rect.top()),
        galley,
        tokens.ink_muted,
    );
}

/// The actions other than the main one, as round buttons with their names under them, centred
/// in a row: Watched and Favorite toggles among them, More opening its menu.
#[allow(clippy::too_many_arguments)]
fn round_actions(
    ui: &mut Ui,
    data: &ItemData,
    actions: &[ItemAction],
    primary: Option<usize>,
    selected: Option<Position>,
    fills: &Fills,
    download: &DownloadLook,
    asked: &mut Asked,
) {
    let item = &data.item;
    let shown: Vec<usize> = (0..actions.len()).filter(|&index| Some(index) != primary).collect();
    if shown.is_empty() {
        return;
    }
    let now = ui.input(|input| input.time);
    let animations = theme::animations(ui.ctx());
    let width = ui.available_width();
    let cell_w = ACTION_CELL.min(width / shown.len() as f32);
    let (row, _) = ui.allocate_exact_size(vec2(width, ACTION_CELL), Sense::hover());
    let start = row.center().x - cell_w * shown.len() as f32 / 2.;
    for (slot, &index) in shown.iter().enumerate() {
        let action = actions[index];
        let cell = Rect::from_min_size(pos2(start + cell_w * slot as f32, row.top()), vec2(cell_w, ACTION_CELL));
        let (icon, name, on, scale) = match action {
            ItemAction::TogglePlayed => {
                let played = is_played(item);
                let icon = if played {
                    ICON_CHECK_CIRCLE
                } else {
                    ICON_CHECK.outlined()
                };
                let scale = if animations { fills.played.scale(now) } else { 1. };
                (icon, "Watched", Some(played), scale)
            }
            ItemAction::ToggleFavorite => {
                let favorite = is_favorite(item);
                let icon = if favorite {
                    ICON_FAVORITE
                } else {
                    ICON_FAVORITE.outlined()
                };
                let scale = if animations { fills.favorite.scale(now) } else { 1. };
                (icon, "Favorite", Some(favorite), scale)
            }
            ItemAction::Download => (download.icon, download.short.as_str(), download.on, 1.),
            _ => (action_look(action, item, None).icon, short_name(action), None, 1.),
        };
        let response = round_action(ui, cell, icon, name, on, scale, ringed(selected, index));
        if action == ItemAction::Download
            && let Some(done) = download.progress
        {
            progress_arc(ui, pos2(cell.center().x, cell.top() + ACTION_RADIUS), done);
        }
        if action == ItemAction::More {
            asked.more = Some(response.clone());
        }
        if response.clicked() {
            asked.action = Some(action);
        }
    }
    if animations && (fills.played.popping(now) || fills.favorite.popping(now)) {
        ui.ctx().request_repaint();
    }
}

/// An action's name under its round button.
fn short_name(action: ItemAction) -> &'static str {
    match action {
        ItemAction::PlayFromStart => "From start",
        ItemAction::Shuffle => "Shuffle",
        ItemAction::Trailer => "Trailer",
        ItemAction::More => "More",
        ItemAction::Resume(_) => "Resume",
        ItemAction::PlayAll => "Play all",
        _ => "Play",
    }
}

/// One round action in `cell`: a circle outlined on the surface (`inset_hover` with a `brand`
/// outline and icon while a toggle is on, or while pressed), its name in `xs` under it. The
/// whole cell takes the touch.
fn round_action(
    ui: &mut Ui,
    cell: Rect,
    icon: MaterialIcon,
    name: &str,
    on: Option<bool>,
    scale: f32,
    ringed: bool,
) -> Response {
    let tokens = theme::tokens_of(ui);
    let response = ui.interact(cell, ui.scope_id().with(("round_action", name)), Sense::CLICK);
    match on {
        Some(on) => response.widget_info(|| WidgetInfo::selected(Role::Button, true, on, name)),
        None => response.widget_info(|| WidgetInfo::labeled(Role::Button, true, name)),
    }
    let lit = on == Some(true);
    let pressed = response.is_pointer_button_down_on() || response.hovered();
    let center = pos2(cell.center().x, cell.top() + ACTION_RADIUS);
    let fill = if lit || pressed {
        tokens.inset_hover
    } else {
        tokens.surface
    };
    let outline = if lit { tokens.brand } else { tokens.surface_border };
    ui.painter()
        .circle(center, ACTION_RADIUS, fill, Stroke::new(1., outline));
    let ink = if lit { tokens.brand } else { tokens.ink };
    round_button::glyph(ui, center, icon, ACTION_ICON * scale, ink);
    ui.painter().text(
        pos2(cell.center().x, center.y + ACTION_RADIUS + 14.),
        egui::Align2::CENTER_CENTER,
        name,
        text::font(ui, "xs"),
        if lit { tokens.brand } else { tokens.ink_muted },
    );
    if ringed {
        let circle = Rect::from_center_size(center, egui::Vec2::splat(2. * ACTION_RADIUS));
        focus::ring(ui, circle, ACTION_RADIUS, Place::Outside);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A download's progress as a `brand` arc round its round button, clockwise from the top,
/// `done` of the way round.
fn progress_arc(ui: &Ui, center: egui::Pos2, done: f32) {
    let tokens = theme::tokens_of(ui);
    let radius = ACTION_RADIUS - 2.;
    let done = done.clamp(0., 1.);
    let steps = ((48. * done).ceil() as usize).max(2);
    let points: Vec<egui::Pos2> = (0..=steps)
        .map(|step| {
            let angle = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * done * step as f32 / steps as f32;
            center + radius * egui::vec2(angle.cos(), angle.sin())
        })
        .collect();
    if done > 0. {
        ui.painter()
            .add(egui::Shape::line(points, Stroke::new(2.5, tokens.brand)));
    }
}

/// The synopsis: its first [`COMPACT_OVERVIEW_ROWS`] lines, and *More* (or *Less*) when there's
/// more than that.
fn overview(ui: &mut Ui, item: &BaseItemDto, open: bool, asked: &mut Asked) {
    let Some(overview) = item.overview.as_deref().filter(|text| !text.is_empty()) else {
        return;
    };
    let tokens = theme::tokens_of(ui);
    ui.add_space(10.);
    let mut job = LayoutJob::single_section(
        overview.to_owned(),
        TextFormat {
            font_id: text::font(ui, "base"),
            color: tokens.ink,
            ..Default::default()
        },
    );
    job.wrap = TextWrapping {
        max_width: ui.available_width(),
        max_rows: if open { usize::MAX } else { COMPACT_OVERVIEW_ROWS },
        break_anywhere: false,
        overflow_character: Some('…'),
    };
    let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
    let cut = galley.elided;
    ui.add(Label::new(galley).selectable(false));
    if cut || open {
        let word = if open { "Less" } else { "More" };
        let label = RichText::new(word)
            .font(text::font(ui, "sm"))
            .strong()
            .color(tokens.brand);
        let response = ui.add(Label::new(label).sense(Sense::CLICK).selectable(false));
        // Named apart from the More button's menu.
        let name = if open { "Show less" } else { "Show more" };
        response.widget_info(|| WidgetInfo::labeled(Role::Link, true, name));
        let response = crate::layout::grow(ui, response, Sense::CLICK);
        if response.clicked() {
            asked.toggle_overview = true;
        }
    }
}

/// The directors, writers and studios, a row each: the name of the row in a muted column, then
/// the names.
fn details(ui: &mut Ui, item: &BaseItemDto) {
    let people = |kind: &str| -> String {
        item.people
            .iter()
            .filter(|person| person.kind.as_deref() == Some(kind))
            .map(|person| person.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let studios = item
        .studios
        .iter()
        .map(|studio| studio.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let rows = [
        ("Director", people("Director")),
        ("Writers", people("Writer")),
        ("Studio", studios),
    ];
    if rows.iter().all(|(_, value)| value.is_empty()) {
        return;
    }
    let tokens = theme::tokens_of(ui);
    ui.add_space(6.);
    for (name, value) in rows.iter().filter(|(_, value)| !value.is_empty()) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.;
            let (cell, _) = ui.allocate_exact_size(vec2(COMPACT_PICKER_LABEL_W, 18.), Sense::hover());
            ui.painter().text(
                cell.left_center(),
                egui::Align2::LEFT_CENTER,
                *name,
                text::font(ui, "sm"),
                tokens.ink_muted,
            );
            let value = RichText::new(value).font(text::font(ui, "sm")).color(tokens.ink);
            ui.add(Label::new(value).wrap());
        });
    }
}

/// The tags as pills, wrapping onto rows.
fn tags(ui: &mut Ui, item: &BaseItemDto) {
    if item.tags.is_empty() {
        return;
    }
    ui.add_space(8.);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6., 6.);
        ui.set_row_height(pill::tag_height(ui));
        for name in &item.tags {
            if ui.available_size_before_wrap().x < pill::tag_width(ui, name) {
                ui.end_row();
            }
            pill::tag(ui, name);
        }
    });
}
