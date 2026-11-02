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
use egui::emath::{easing, lerp};
use egui::epaint::Shadow;
use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Label, Rect, Response, Sense, Ui, Vec2, WidgetText, pos2, vec2,
};
use egui_material_icons::FONT_FAMILY;
use egui_material_icons::icons::{ICON_HIDE_IMAGE, ICON_PLAY_ARROW};
use jellyfin::ImageRequest;
use jellyfin::models::{BaseItemDto, BaseItemPerson, ChapterInfo, ImageType};

use crate::images::ImageCache;
use crate::item_menu::is_played;
use crate::theme;
use crate::theme::text;
use crate::widgets::focus::{self, Place};
use crate::widgets::text::{
    CARD_TEXT_H, LANDSCAPE_H, LANDSCAPE_W, PERSON_H, PERSON_W, POSTER_H, POSTER_W, card_text, kind_label,
    landscape_image, poster_image, timestamp,
};
use crate::widgets::{avatar, badge, image, round_button};

/// Between a card's image and its title, and between its lines (`main`'s `gap_1`).
const GAP: f32 = 4.;

/// How much bigger the play button gets under the pointer.
const PLAY_GROWTH: f32 = 1.1;
/// How big the play button starts as it fades in, against its resting size.
const PLAY_ENTER_SCALE: f32 = 0.7;
/// How long the play button takes to fade in as the pointer comes onto its card (or out).
const PLAY_APPEAR: f32 = 0.18;
/// How long the play button takes to grow and deepen under the pointer (or settle back).
const PLAY_GROW: f32 = 0.14;
/// What lifts the play button off the picture.
const PLAY_SHADOW: Shadow = Shadow {
    offset: [0, 4],
    blur: 16,
    spread: 0,
    color: Color32::from_black_alpha(90),
};

/// The progress bar's height and its distance from the image's sides and bottom.
const BAR_H: f32 = 5.;
const BAR_INSET: f32 = 10.;
/// The bar's frosted track: white at about 27 % (premultiplied).
const BAR_TRACK: Color32 = Color32::from_rgba_premultiplied(70, 70, 70, 70);
/// The dark wash behind the bar: how far up the image it reaches, and its alpha at the edge.
const SCRIM_H: f32 = 48.;
const SCRIM_ALPHA: u8 = 110;

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

/// A chapter of `item_id`: its image when the server extracted one (a crossed-out picture when it
/// didn't for this one), its name and start. A shelf none of whose chapters has an image uses
/// [`chapter_without_image`] instead.
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
    if request.is_none() {
        missing_image(ui, &response, image);
    }
    cursor_ring(ui, image, selected);
    let name = chapter_name(index, chapter);
    let lines = lines(ui, image, "sm", "xs", GAP);
    put(ui, lines.title, line(ui, &name, "sm", false));
    put(
        ui,
        lines.subtitle,
        line(ui, &timestamp(chapter.start_position_ticks.into()), "xs", true),
    );
    response
}

/// A chapter with no image to show, for a shelf none of whose chapters has one: a short tile
/// with its number, name and start, rather than a row of empty pictures.
pub fn chapter_without_image(ui: &mut Ui, index: usize, chapter: &ChapterInfo, selected: bool) -> Response {
    const PADDING: f32 = 12.;
    let heights = line_heights(ui, "sm", "xs");
    let size = vec2(LANDSCAPE_W, PADDING + lines_height(&heights) - GAP + PADDING);
    let (tile, response) = ui.allocate_exact_size(size, Sense::click());
    let tokens = theme::tokens_of(ui);
    ui.painter().rect_filled(tile, image::RADIUS, tokens.muted);
    if response.hovered() {
        image::dim(ui, tile, image::RADIUS);
    }
    // The number gets the room of two digits, so the names line up along the shelf.
    let number_font = text::font(ui, "2xl");
    let number_width = ui.fonts_mut(|fonts| {
        fonts
            .layout_no_wrap("00".into(), number_font.clone(), tokens.muted_foreground)
            .size()
            .x
    });
    ui.painter().text(
        pos2(tile.left() + PADDING, tile.center().y),
        Align2::LEFT_CENTER,
        (index + 1).to_string(),
        number_font,
        tokens.muted_foreground,
    );
    let left = tile.left() + PADDING + number_width + PADDING;
    let width = (tile.right() - PADDING - left).max(0.);
    let title = Rect::from_min_size(pos2(left, tile.top() + PADDING), vec2(width, heights.title));
    let subtitle = Rect::from_min_size(pos2(left, title.bottom() + GAP), vec2(width, heights.subtitle));
    put(ui, title, line(ui, &chapter_name(index, chapter), "sm", false));
    put(
        ui,
        subtitle,
        line(ui, &timestamp(chapter.start_position_ticks.into()), "xs", true),
    );
    cursor_ring(ui, tile, selected);
    response
}

/// A chapter's name, or its number when it has none.
fn chapter_name(index: usize, chapter: &ChapterInfo) -> String {
    chapter.name.clone().unwrap_or_else(|| format!("Chapter {}", index + 1))
}

/// A crossed-out picture in the middle of an image that has nothing to show.
fn missing_image(ui: &Ui, card: &Response, image: Rect) {
    let tokens = theme::tokens_of(ui);
    // Over the grey box, under the dimming, so the icon dims with the card.
    ui.painter().rect_filled(image, image::RADIUS, tokens.muted);
    let size = (image.height() * 0.3).clamp(20., 40.);
    ui.painter().text(
        image.center(),
        Align2::CENTER_CENTER,
        ICON_HIDE_IMAGE.codepoint,
        FontId::new(size, egui::FontFamily::Name(FONT_FAMILY.into())),
        tokens.muted_foreground,
    );
    if card.hovered() && ui.rect_contains_pointer(image) {
        image::dim(ui, image, image::RADIUS);
    }
}

/// A cast or crew member, `scale` times the base size: their photo, or their initials on their
/// avatar's pastel.
pub fn person(ui: &mut Ui, cache: &mut ImageCache, person: &BaseItemPerson, scale: f32, selected: bool) -> Response {
    let size = vec2(PERSON_W * scale, PERSON_H * scale);
    let request = person
        .primary_image_tag
        .clone()
        .map(|tag| ImageRequest::new(&person.id, ImageType::Primary, Some(tag)));
    let (response, image) = picture(ui, cache, size, request.as_ref(), None);
    if request.is_none() {
        // The person's pastel, the colour their avatar has anywhere else, rather than a grey box.
        let tokens = theme::tokens_of(ui);
        ui.painter()
            .rect_filled(image, image::RADIUS, avatar::colour(tokens, &person.name));
        if response.hovered() && ui.rect_contains_pointer(image) {
            image::dim(ui, image, image::RADIUS);
        }
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
            tokens.avatar_foreground,
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
            badge::check(ui, image);
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
    let shown = card.hovered() && ui.rect_contains_pointer(image);
    let rest = (image.width().min(image.height()) * 0.16).clamp(16., 28.);
    let center = image.center();
    let within = |pos: Option<egui::Pos2>| pos.is_some_and(|pos| pos.distance(center) <= rest);
    let on_it = shown && within(card.hover_pos());
    // It fades and swells in as the pointer comes onto the card and out as it leaves; with the
    // animations setting off both happen at once.
    let ctx = ui.ctx();
    let (appear_time, grow_time) = if theme::animations(ctx) {
        (PLAY_APPEAR, PLAY_GROW)
    } else {
        (0., 0.)
    };
    let id = card.id.with("play button");
    let appear = ctx.animate_bool_with_time_and_easing(id, shown, appear_time, easing::cubic_out);
    if appear <= 0. {
        return false;
    }
    let grow = ctx.animate_bool_with_time_and_easing(id.with("grow"), on_it, grow_time, easing::cubic_out);
    let tokens = theme::tokens_of(ui);
    // Under the pointer it grows a little and deepens, so it reads as the thing that will be
    // clicked. The shadow lifts it off a picture of its own colour.
    let radius = rest * lerp(PLAY_ENTER_SCALE..=1., appear) * lerp(1. ..=PLAY_GROWTH, grow);
    let fill = tokens
        .brand
        .lerp_to_gamma(tokens.brand_hover, grow)
        .gamma_multiply(appear);
    let disc = Rect::from_center_size(center, Vec2::splat(2. * radius));
    let shadow = Shadow {
        color: PLAY_SHADOW.color.gamma_multiply(appear),
        ..PLAY_SHADOW
    };
    ui.painter()
        .add(shadow.as_shape(disc, CornerRadius::same(radius.round() as u8)));
    ui.painter().circle_filled(center, radius, fill);
    // Centred by the triangle's own box, then nudged right by a sixth of its width (about 0.08
    // of the icon's size) so its weight, a third of the way in from its flat side, sits in the
    // middle.
    let size = radius * 1.25;
    round_button::glyph(
        ui,
        center + vec2(size * 0.08, 0.),
        ICON_PLAY_ARROW,
        size,
        tokens.brand_foreground.gamma_multiply(appear),
    );
    if on_it {
        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    shown && card.clicked() && within(card.interact_pointer_pos())
}

/// The keyboard cursor's ring, inside the card's edge so scrolling containers don't clip it.
fn cursor_ring(ui: &Ui, image: Rect, selected: bool) {
    if selected {
        focus::ring(ui, image, image::RADIUS as f32, Place::Inside);
    }
}

/// How far through it is, for the bar along the bottom of its card.
fn progress(item: &BaseItemDto) -> Option<f32> {
    let percentage = item.user_data.as_ref()?.played_percentage?;
    (percentage > 0. && percentage < 100.).then_some((percentage / 100.) as f32)
}

/// A round bar along the bottom of the image, inset clear of the rounded corners, on a frosted
/// track over a dark wash up from the edge, so its full length shows on bright pictures and
/// dark ones alike. The fill is the player's lavender in both themes: it sits on the picture,
/// where the light theme's deep `brand` would sink into a dark one.
fn progress_bar(ui: &Ui, image: Rect, fraction: f32) {
    scrim(ui, image);
    let bar = Rect::from_min_max(
        pos2(image.left() + BAR_INSET, image.bottom() - BAR_INSET - BAR_H),
        pos2(image.right() - BAR_INSET, image.bottom() - BAR_INSET),
    )
    .intersect(image);
    if bar.width() <= 0. {
        return;
    }
    let radius = CornerRadius::same((BAR_H / 2.).ceil() as u8);
    let painter = ui.painter();
    painter.rect_filled(bar, radius, BAR_TRACK);
    let filled = Rect::from_min_max(
        bar.min,
        pos2(bar.left() + bar.width() * fraction.clamp(0., 1.), bar.bottom()),
    );
    painter.rect_filled(filled, radius, theme::over_video::BRAND);
}

/// A black wash over the bottom [`SCRIM_H`] of `image`, clear at its top and darkest at the
/// edge, following the image's rounded corners.
fn scrim(ui: &Ui, image: Rect) {
    let top = (image.bottom() - SCRIM_H).max(image.top());
    let height = image.bottom() - top;
    if height <= 0. {
        return;
    }
    let colour = |y: f32| {
        // Eased in, so the wash has no visible top edge.
        let t = ((y - top) / height).clamp(0., 1.);
        Color32::from_black_alpha((f32::from(SCRIM_ALPHA) * t * t) as u8)
    };
    let radius = f32::from(image::RADIUS).min(image.width() / 2.).min(height);
    // The image's bottom edge at `x`: straight, except where its corners curve up.
    let bottom = |x: f32| {
        let into = (image.left() + radius - x).max(x - (image.right() - radius)).max(0.);
        image.bottom() - radius + (radius * radius - into * into).max(0.).sqrt()
    };
    // Columns half a point apart, each from the wash's top down to the edge.
    let mut mesh = egui::Mesh::default();
    let columns = (image.width() * 2.).ceil() as u32;
    for i in 0..=columns {
        let x = image.left() + image.width() * i as f32 / columns as f32;
        let lower = bottom(x).max(top);
        mesh.colored_vertex(pos2(x, top), colour(top));
        mesh.colored_vertex(pos2(x, lower), colour(lower));
        if i > 0 {
            let a = 2 * i;
            mesh.add_triangle(a - 2, a - 1, a);
            mesh.add_triangle(a - 1, a, a + 1);
        }
    }
    ui.painter().add(egui::Shape::mesh(mesh));
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
