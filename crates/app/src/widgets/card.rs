//! Cards: a cover image with the item's title under it, in `main`'s sizes and scales: poster,
//! landscape, episode, chapter and person cards, and the grey placeholder a grid shows while it
//! loads.
//!
//! A card allocates its own space (the image plus a block for the text) and answers the pointer
//! with its [`Response`], so the caller handles clicks, the keyboard's [`crate::browse::reveal`]
//! and the item's context menu. What `main` drew over the image comes with it: the played
//! badge, the progress bar, dimming under the pointer and the cursor's ring. Titles and
//! subtitles are cut short when they don't fit, and the sizes follow A1's text size.

use egui::text::LayoutJob;
use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Label, Rect, Response, RichText, Sense, Stroke, StrokeKind,
    TextFormat, TextStyle, Ui, Vec2, WidgetText, pos2, vec2,
};
use egui_material_icons::FONT_FAMILY;
use egui_material_icons::icons::ICON_CHECK;
use jellyfin::ImageRequest;
use jellyfin::models::{BaseItemDto, BaseItemPerson, ChapterInfo, ImageType};

use crate::images::ImageCache;
use crate::item_menu::is_played;
use crate::widgets::image;
use crate::widgets::text::{
    CARD_TEXT_H, LANDSCAPE_H, LANDSCAPE_W, PERSON_H, PERSON_W, POSTER_H, POSTER_W, card_text,
    is_placeholder_episode_name, kind_label, landscape_image, poster_image, timestamp,
};

/// Between a card's image and its title, and between its lines (`main`'s `gap_1`).
const GAP: f32 = 4.;
/// Episode cards' titles are one step bigger than other cards' (`main`'s `text_base` against
/// `text_sm`).
const EPISODE_TITLE_STEP: f32 = 2.;

/// A poster-shaped card.
pub fn poster(ui: &mut Ui, cache: &mut ImageCache, item: &BaseItemDto, selected: bool) -> Response {
    poster_scaled(ui, cache, item, 1., selected)
}

/// [`poster`], `scale` times the size.
pub fn poster_scaled(ui: &mut Ui, cache: &mut ImageCache, item: &BaseItemDto, scale: f32, selected: bool) -> Response {
    let size = vec2(POSTER_W * scale, POSTER_H * scale);
    item_card(ui, cache, item, size, poster_image(item).as_ref(), selected)
}

/// A wide card.
pub fn landscape(ui: &mut Ui, cache: &mut ImageCache, item: &BaseItemDto, selected: bool) -> Response {
    landscape_scaled(ui, cache, item, 1., selected)
}

/// [`landscape`], `scale` times the size.
pub fn landscape_scaled(
    ui: &mut Ui,
    cache: &mut ImageCache,
    item: &BaseItemDto,
    scale: f32,
    selected: bool,
) -> Response {
    let size = vec2(LANDSCAPE_W * scale, LANDSCAPE_H * scale);
    item_card(ui, cache, item, size, landscape_image(item).as_ref(), selected)
}

/// An item's card: its image, then [`card_text`]'s title and subtitle under it.
fn item_card(
    ui: &mut Ui,
    cache: &mut ImageCache,
    item: &BaseItemDto,
    size: Vec2,
    request: Option<&ImageRequest>,
    selected: bool,
) -> Response {
    let (response, image) = picture(ui, cache, size, request, Some(item));
    cursor_ring(ui, image, selected);
    let (title, subtitle) = card_text(item);
    let lines = lines(ui, image, 0.);
    put(ui, lines.title, line(ui, &title, 0., false));
    put(ui, lines.subtitle, line(ui, &subtitle, -2., true));
    response
}

/// An episode in its season's row: "Episode 2" and its name (the season is the row's title),
/// `scale` times a landscape card's size.
pub fn episode(ui: &mut Ui, cache: &mut ImageCache, episode: &BaseItemDto, scale: f32, selected: bool) -> Response {
    let size = vec2(LANDSCAPE_W * scale, LANDSCAPE_H * scale);
    let (response, image) = picture(ui, cache, size, landscape_image(episode).as_ref(), Some(episode));
    cursor_ring(ui, image, selected);
    let lines = lines(ui, image, EPISODE_TITLE_STEP);
    put(ui, lines.title, episode_title(ui, episode));
    response
}

/// A chapter of `item_id`: its image when the server extracted one, its name and start.
pub fn chapter(
    ui: &mut Ui,
    cache: &mut ImageCache,
    item_id: &str,
    index: usize,
    chapter: &ChapterInfo,
    selected: bool,
) -> Response {
    let request = chapter.image_tag.clone().map(|tag| ImageRequest {
        index: Some(index as u32),
        ..ImageRequest::new(item_id, ImageType::Chapter, Some(tag))
    });
    let size = vec2(LANDSCAPE_W, LANDSCAPE_H);
    let (response, image) = picture(ui, cache, size, request.as_ref(), None);
    cursor_ring(ui, image, selected);
    let name = chapter.name.clone().unwrap_or_else(|| format!("Chapter {}", index + 1));
    let lines = lines(ui, image, 0.);
    put(ui, lines.title, line(ui, &name, 0., false));
    put(
        ui,
        lines.subtitle,
        line(ui, &timestamp(chapter.start_position_ticks.into()), -2., true),
    );
    response
}

/// A cast or crew member, `scale` times the base size: their photo, or their initials.
pub fn person(ui: &mut Ui, cache: &mut ImageCache, person: &BaseItemPerson, scale: f32, selected: bool) -> Response {
    let size = vec2(PERSON_W * scale, PERSON_H * scale);
    let request = person
        .primary_image_tag
        .clone()
        .map(|tag| ImageRequest::new(&person.id, ImageType::Primary, Some(tag)));
    let (response, image) = picture(ui, cache, size, request.as_ref(), None);
    if request.is_none() {
        let initials: String = person
            .name
            .split_whitespace()
            .filter_map(|word| word.chars().next())
            .take(2)
            .collect();
        let font = FontId::new(24. * scale, TextStyle::Body.resolve(ui.style()).family);
        ui.painter().text(
            image.center(),
            Align2::CENTER_CENTER,
            initials,
            font,
            ui.visuals().weak_text_color(),
        );
    }
    cursor_ring(ui, image, selected);
    let detail = match (&person.role, person.kind.as_deref()) {
        (Some(role), _) if !role.is_empty() => role.clone(),
        (_, Some("Actor")) | (_, None) => String::new(),
        (_, Some(kind)) => kind_label(kind),
    };
    let lines = lines(ui, image, 0.);
    put(ui, lines.title, line(ui, &person.name, 0., false));
    put(ui, lines.subtitle, line(ui, &detail, -2., true));
    response
}

/// Grey card used while a grid page is still loading. It takes a real card's place without
/// asking for anything.
pub fn placeholder(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(POSTER_W, POSTER_H + text_height(ui)), Sense::hover());
    ui.painter().rect_filled(
        Rect::from_min_size(rect.min, vec2(POSTER_W, POSTER_H)),
        image::RADIUS,
        ui.visuals().faint_bg_color,
    );
}

/// The room a card takes under its image: a title line, a subtitle line and the gap above them,
/// and at least `main`'s `CARD_TEXT_H`, so a grid's rows keep their height whatever the text
/// size.
pub fn text_height(ui: &Ui) -> f32 {
    let heights = line_heights(ui, 0.);
    CARD_TEXT_H.max(GAP + heights.title + heights.subtitle)
}

/// A card's image, with `item`'s blurhash while it loads, its played badge and progress bar,
/// and dimming while the pointer is on it. The cursor's ring is [`cursor_ring`], after those.
///
/// Returns the card's response and the image's rectangle.
fn picture(
    ui: &mut Ui,
    cache: &mut ImageCache,
    size: Vec2,
    request: Option<&ImageRequest>,
    item: Option<&BaseItemDto>,
) -> (Response, Rect) {
    let (rect, response) = ui.allocate_exact_size(vec2(size.x, size.y + text_height(ui)), Sense::click());
    let image = Rect::from_min_size(rect.min, size);
    image::show(ui, cache, image, request, item, image::RADIUS);
    if ui.rect_contains_pointer(image) {
        image::dim(ui, image, image::RADIUS);
    }
    if let Some(item) = item {
        if is_played(item) {
            played_badge(ui, image);
        }
        if let Some(fraction) = progress(item) {
            progress_bar(ui, image, fraction);
        }
    }
    (response, image)
}

/// The keyboard cursor's ring, inside the card's edge so scrolling containers don't clip it.
fn cursor_ring(ui: &Ui, image: Rect, selected: bool) {
    if selected {
        ui.painter().rect_stroke(
            image,
            CornerRadius::same(image::RADIUS),
            Stroke::new(3., ui.visuals().strong_text_color()),
            StrokeKind::Inside,
        );
    }
}

/// A tick in a filled circle, top right.
fn played_badge(ui: &Ui, image: Rect) {
    let radius = 8.;
    let center = image.right_top() + vec2(-radius - 6., radius + 6.);
    ui.painter()
        .circle_filled(center, radius, ui.visuals().selection.bg_fill);
    ui.painter().text(
        center,
        Align2::CENTER_CENTER,
        ICON_CHECK.codepoint,
        FontId::new(2. * radius - 4., egui::FontFamily::Name(FONT_FAMILY.into())),
        Color32::WHITE,
    );
}

/// How far through it is, for the bar along the bottom of its card.
fn progress(item: &BaseItemDto) -> Option<f32> {
    let percentage = item.user_data.as_ref()?.played_percentage?;
    (percentage > 0. && percentage < 100.).then_some((percentage / 100.) as f32)
}

/// A bar along the bottom of the image, inset clear of the rounded corners: dark with a light
/// outline, so its full length shows on dark images too.
fn progress_bar(ui: &Ui, image: Rect, fraction: f32) {
    let bar = Rect::from_min_max(
        pos2(image.left() + 8., image.bottom() - 14.),
        pos2(image.right() - 8., image.bottom() - 8.),
    )
    .intersect(image);
    if bar.width() <= 0. {
        return;
    }
    let radius = CornerRadius::same(3);
    let painter = ui.painter();
    painter.rect_filled(bar, radius, Color32::from_black_alpha(153));
    let filled = Rect::from_min_max(
        bar.min,
        pos2(bar.left() + bar.width() * fraction.clamp(0., 1.), bar.bottom()),
    );
    painter.rect_filled(filled, radius, ui.visuals().selection.bg_fill);
    painter.rect_stroke(
        bar,
        radius,
        Stroke::new(1., Color32::from_white_alpha(128)),
        StrokeKind::Inside,
    );
}

/// Where a card's two text lines go under `image`.
struct Lines {
    title: Rect,
    subtitle: Rect,
}

fn lines(ui: &Ui, image: Rect, title_step: f32) -> Lines {
    let heights = line_heights(ui, title_step);
    let top = image.bottom() + GAP;
    let width = image.width();
    Lines {
        title: Rect::from_min_size(pos2(image.left(), top), vec2(width, heights.title)),
        subtitle: Rect::from_min_size(pos2(image.left(), top + heights.title), vec2(width, heights.subtitle)),
    }
}

/// The line heights of a card's title (`title_step` points over the body size) and subtitle
/// (`main`'s `text_xs` under `text_sm`).
struct LineHeights {
    title: f32,
    subtitle: f32,
}

fn line_heights(ui: &Ui, title_step: f32) -> LineHeights {
    let body = TextStyle::Body.resolve(ui.style());
    let title = FontId::new(body.size + title_step, body.family.clone());
    let subtitle = FontId::new((body.size - 2.).max(6.), body.family.clone());
    LineHeights {
        title: ui.fonts_mut(|fonts| fonts.row_height(&title)),
        subtitle: ui.fonts_mut(|fonts| fonts.row_height(&subtitle)),
    }
}

/// A one-line label cut short with an ellipsis, `size` points off the body size and muted when
/// it's a subtitle.
fn line(ui: &Ui, text: &str, size: f32, muted: bool) -> WidgetText {
    let body = TextStyle::Body.resolve(ui.style());
    let mut text = RichText::new(text).font(FontId::new((body.size + size).max(6.), body.family));
    if muted {
        text = text.color(ui.visuals().weak_text_color());
    }
    text.into()
}

/// "Episode 2" then its name, dimmer, on one line: the number stays whole, the name is cut
/// short when it doesn't fit.
fn episode_title(ui: &Ui, episode: &BaseItemDto) -> WidgetText {
    let body = TextStyle::Body.resolve(ui.style());
    let font = FontId::new(body.size + EPISODE_TITLE_STEP, body.family.clone());
    let visuals = ui.visuals();
    let mut job = LayoutJob::default();
    let style = |color| TextFormat {
        font_id: font.clone(),
        color,
        ..Default::default()
    };
    if let Some(number) = episode.index_number {
        job.append(&format!("Episode {number}"), 0., style(visuals.text_color()));
    }
    // Untitled episodes are named "Episode 3" by Jellyfin, which would say it twice.
    let name = episode.name.trim();
    if !name.is_empty() && !is_placeholder_episode_name(name) {
        job.append(&format!("  {name}"), 0., style(visuals.weak_text_color()));
    }
    WidgetText::LayoutJob(job.into())
}

fn put(ui: &mut Ui, rect: Rect, text: WidgetText) {
    ui.put(rect, Label::new(text).halign(Align::LEFT).truncate());
}
