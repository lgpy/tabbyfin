//! Cards: a cover image with the item's title under it, in `main`'s sizes and scales: poster,
//! landscape, episode, chapter and person cards, and the grey placeholder a grid shows while it
//! loads.
//!
//! A card allocates its own space (the image plus a block for the text) and answers the pointer
//! with its [`Response`], so the caller handles clicks, scrolling the selected card into view
//! (`Cursor::take_reveal`) and the item's context menu. What `main` drew over the image comes with it: the played
//! badge, the progress bar, dimming under the pointer and the cursor's ring. Titles and
//! subtitles are cut short when they don't fit, and the sizes follow A1's text size.

use egui::emath::GuiRounding as _;
use egui::{
    Align, Align2, CornerRadius, FontId, Label, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, WidgetText, pos2,
    vec2,
};
use egui_material_icons::FONT_FAMILY;
use egui_material_icons::icons::{ICON_CHECK, ICON_PLAY_ARROW};
use jellyfin::ImageRequest;
use jellyfin::models::{BaseItemDto, BaseItemPerson, ChapterInfo, ImageType};

use crate::images::ImageCache;
use crate::item_menu::is_played;
use crate::theme;
use crate::theme::text;
use crate::widgets::focus::{self, Place};
use crate::widgets::image;
use crate::widgets::text::{
    CARD_TEXT_H, LANDSCAPE_H, LANDSCAPE_W, PERSON_H, PERSON_W, POSTER_H, POSTER_W, card_text, kind_label,
    landscape_image, poster_image, timestamp,
};

/// Between a card's image and its title, and between its lines (`main`'s `gap_1`).
const GAP: f32 = 4.;

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
    let lines = lines(ui, image, "sm", "xs", GAP);
    put(ui, lines.title, line(ui, &title, "sm", false));
    put(ui, lines.subtitle, line(ui, &subtitle, "xs", true));
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
    let lines = lines(ui, image, "sm", "xs", GAP);
    put(ui, lines.title, line(ui, &name, "sm", false));
    put(
        ui,
        lines.subtitle,
        line(ui, &timestamp(chapter.start_position_ticks.into()), "xs", true),
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
        // `main`'s `text_2xl`, whatever the card's scale, and it grows with the text size.
        ui.painter().text(
            image.center(),
            Align2::CENTER_CENTER,
            initials,
            text::font(ui, "2xl"),
            theme::tokens_of(ui).muted_foreground,
        );
    }
    cursor_ring(ui, image, selected);
    let detail = match (&person.role, person.kind.as_deref()) {
        (Some(role), _) if !role.is_empty() => role.clone(),
        (_, Some("Actor")) | (_, None) => String::new(),
        (_, Some(kind)) => kind_label(kind),
    };
    let lines = lines(ui, image, "sm", "xs", GAP);
    put(ui, lines.title, line(ui, &person.name, "sm", false));
    put(ui, lines.subtitle, line(ui, &detail, "xs", true));
    response
}

/// Grey card used while a grid page is still loading. It takes a real card's place without
/// asking for anything.
pub fn placeholder(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(POSTER_W, POSTER_H + text_height(ui)), Sense::hover());
    ui.painter().rect_filled(
        Rect::from_min_size(rect.min, vec2(POSTER_W, POSTER_H)),
        image::RADIUS,
        theme::tokens_of(ui).muted,
    );
}

/// The room a card takes under its image: the gap down from the image, the title, the gap
/// between the lines and the subtitle — `main`'s `4 + 20 + 4 + 16`. `CARD_TEXT_H` is the floor,
/// so a grid's rows keep their height whatever the text size.
pub fn text_height(ui: &Ui) -> f32 {
    let heights = line_heights(ui, "sm", "xs");
    CARD_TEXT_H.max(lines_height(&heights))
}

/// The two lines and the two gaps: what [`text_height`] reserves.
fn lines_height(heights: &LineHeights) -> f32 {
    GAP + heights.title + GAP + heights.subtitle
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
    // `hovered` rather than a bare rectangle test: it also needs the card to be the widget under the
    // pointer, clipped to what's visible, so a card half hidden by its shelf's edge or covered by
    // something else doesn't dim (and flicker as the two disagree from frame to frame).
    if response.hovered() && ui.rect_contains_pointer(image) {
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

/// A play button in the middle of `card`'s image while the pointer is on it, for a page whose
/// cards can be played straight away (Home's). Returns whether it was clicked; the card's own
/// click then shouldn't open the item.
///
/// It's part of the card rather than a widget of its own: one on top would take the hover from
/// the card, which would stop dimming (and drop the button) as the pointer reached it.
pub fn play_button(ui: &Ui, card: &Response) -> bool {
    let image = Rect::from_min_max(
        card.rect.min,
        pos2(card.rect.right(), card.rect.bottom() - text_height(ui)),
    );
    if !(card.hovered() && ui.rect_contains_pointer(image)) {
        return false;
    }
    let radius = (image.width().min(image.height()) * 0.16).clamp(16., 28.);
    let center = image.center();
    let within = |pos: Option<egui::Pos2>| pos.is_some_and(|pos| pos.distance(center) <= radius);
    let on_it = within(card.hover_pos());
    let tokens = theme::tokens_of(ui);
    // The circle goes see-through under the pointer, so it reads as the thing that will be
    // clicked; the arrow stays solid.
    let fill = if on_it {
        tokens.primary.gamma_multiply(0.7)
    } else {
        tokens.primary
    };
    ui.painter().circle_filled(center, radius, fill);
    ui.painter().text(
        center,
        Align2::CENTER_CENTER,
        ICON_PLAY_ARROW.codepoint,
        FontId::new(radius * 1.3, egui::FontFamily::Name(FONT_FAMILY.into())),
        tokens.primary_foreground,
    );
    if on_it {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    card.clicked() && within(card.interact_pointer_pos())
}

/// The keyboard cursor's ring, inside the card's edge so scrolling containers don't clip it.
fn cursor_ring(ui: &Ui, image: Rect, selected: bool) {
    if selected {
        focus::ring(ui, image, image::RADIUS as f32, Place::Inside);
    }
}

/// A tick in a filled circle, 4 px in from the image's top right corner: `main`'s primary
/// colour with the tick in its foreground, so it reads on a dark theme and a light one alike.
fn played_badge(ui: &Ui, image: Rect) {
    let radius = 8.;
    let center = image.right_top() + vec2(-4. - radius, 4. + radius);
    let tokens = theme::tokens_of(ui);
    ui.painter().circle_filled(center, radius, tokens.primary);
    ui.painter().text(
        center,
        Align2::CENTER_CENTER,
        ICON_CHECK.codepoint,
        FontId::new(2. * radius - 4., egui::FontFamily::Name(FONT_FAMILY.into())),
        tokens.primary_foreground,
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
    painter.rect_filled(bar, radius, theme::over_video::TRACK);
    let filled = Rect::from_min_max(
        bar.min,
        pos2(bar.left() + bar.width() * fraction.clamp(0., 1.), bar.bottom()),
    );
    painter.rect_filled(filled, radius, theme::tokens_of(ui).primary);
    painter.rect_stroke(
        bar,
        radius,
        Stroke::new(1., theme::over_video::TRACK_OUTLINE),
        StrokeKind::Inside,
    );
}

/// Where a card's two text lines go under `image`.
struct Lines {
    title: Rect,
    subtitle: Rect,
}

fn lines(ui: &Ui, image: Rect, title: &str, subtitle: &str, top_gap: f32) -> Lines {
    let heights = line_heights(ui, title, subtitle);
    let top = image.bottom() + top_gap;
    let width = image.width();
    Lines {
        title: Rect::from_min_size(pos2(image.left(), top), vec2(width, heights.title)),
        subtitle: Rect::from_min_size(
            pos2(image.left(), top + heights.title + GAP),
            vec2(width, heights.subtitle),
        ),
    }
}

/// The line heights of a card's title and subtitle, from the text-style table.
struct LineHeights {
    title: f32,
    subtitle: f32,
}

fn line_heights(ui: &Ui, title: &str, subtitle: &str) -> LineHeights {
    let title = text::font(ui, title);
    let subtitle = text::font(ui, subtitle);
    LineHeights {
        title: ui.fonts_mut(|fonts| fonts.row_height(&title)),
        subtitle: ui.fonts_mut(|fonts| fonts.row_height(&subtitle)),
    }
}

/// A one-line label cut short with an ellipsis, sized from the table and muted when it's a
/// subtitle.
fn line(ui: &Ui, text: &str, style: &str, muted: bool) -> WidgetText {
    let mut label = egui::RichText::new(text).font(self::text::font(ui, style));
    if muted {
        label = label.color(theme::tokens_of(ui).muted_foreground);
    }
    label.into()
}

/// Puts a line at the left of `rect`, cut short when it doesn't fit. `Ui::put` centres and
/// justifies its widget, which stretched a title across the card's width; `main`'s lines sat at
/// the card's left edge and kept their length.
fn put(ui: &mut Ui, rect: Rect, text: WidgetText) {
    // Whole pixels, so the text is crisp and egui's debug "Unaligned" overlay stays off.
    let rect = rect.round_ui();
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::top_down(Align::Min)),
        |ui| {
            ui.add(Label::new(text).truncate());
            // The card allocated the line's whole width; without this the scope would report
            // only the label's width and pull the row's next card back under this one.
            ui.expand_to_include_rect(rect);
        },
    );
}
