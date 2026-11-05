//! An item as an inset row of a list: a season's episodes and a playlist's entries
//! ([`MediaRow`]), and a series' seasons ([`season`]). Inset like `AvatarRow`: `inset` at rest,
//! `inset_hover` under the pointer or a page's cursor (with the keyboard's ring inside),
//! `radius_control`, [`GAP`] apart.
//!
//! An episode row has the item's wide image, its heading in `ink`, a muted line under it, up to
//! three lines of overview in `ink_muted`, and at its right end a round Watched toggle. A
//! [`MediaRow::compact`] row is smaller (a 160×90 image, two lines of overview) for lists laid
//! out in columns ([`episode_columns`]), so a long season fits in about a window. A
//! playlist's row also has round ↑ ↓ ✕ buttons before the toggle, shown only while the row is
//! active (under the pointer or the cursor), as the SyncPlay queue's are: the list stays quiet,
//! and the keys move and remove any entry anyway.

use std::sync::Arc;

use egui::accesskit::Role;
use egui::text::LayoutJob;
use egui::{
    Align, Align2, Color32, CornerRadius, Galley, Label, Layout, Rect, Response, RichText, Sense, TextFormat, Ui,
    UiBuilder, WidgetInfo,
};
use egui::{pos2, vec2};
use egui_material_icons::icons::{ICON_CHECK, ICON_CHECK_CIRCLE, ICON_EXPAND_LESS, ICON_EXPAND_MORE};
use jellyfin::ImageRequest;
use jellyfin::models::BaseItemDto;

use crate::images::ImageCache;
use crate::theme::{self, text};
use crate::widgets::button::Focus;
use crate::widgets::focus::{self, Place};
use crate::widgets::image;
use crate::widgets::list_row::{self, Handle, Handles};
use crate::widgets::round_button::{self, RoundButton};
use crate::widgets::surface;
use crate::widgets::text::{LANDSCAPE_H, LANDSCAPE_W};

/// Between the rows of a list.
pub const GAP: f32 = surface::GAP_ITEM;
/// The room inside a row, round its image.
const PAD: f32 = 10.;
/// Between the heading, the line under it and the overview.
const LINE_GAP: f32 = 4.;
/// The image's corners, concentric enough with the row's inside its padding.
const IMAGE_RADIUS: u8 = 6;

/// What changes between a full row and a compact one.
struct Size {
    image_w: f32,
    image_h: f32,
    /// Between the image, the text and the buttons.
    gap: f32,
    watched: f32,
    watched_icon: f32,
    /// How many lines of overview show.
    overview_rows: usize,
}

/// A row on a line of its own (its overview clamped to 3 lines).
const FULL: Size = Size {
    image_w: LANDSCAPE_W,
    image_h: LANDSCAPE_H,
    gap: 16.,
    watched: 36.,
    watched_icon: 20.,
    overview_rows: 3,
};

/// A row in a list of columns.
const COMPACT: Size = Size {
    image_w: 160.,
    image_h: 90.,
    gap: 14.,
    watched: 32.,
    watched_icon: 18.,
    overview_rows: 2,
};

/// The narrowest a compact row gets before the list takes fewer columns.
pub const EPISODE_MIN_W: f32 = 420.;

/// A season row's height, and the narrowest one gets before the list takes fewer columns.
const SEASON_H: f32 = 84.;
pub const SEASON_MIN_W: f32 = 300.;

/// A chapter row's thumbnail, the room round it, and its corners: the row's, and the picture's
/// inside it.
pub const CHAPTER_THUMB_W: f32 = 128.;
const CHAPTER_THUMB_H: f32 = 72.;
const CHAPTER_PAD: f32 = 8.;
const CHAPTER_ROW_RADIUS: u8 = 10;
const CHAPTER_IMAGE_RADIUS: u8 = 6;
/// Between the thumbnail and the chapter's name.
const CHAPTER_TEXT_GAP: f32 = 14.;

pub struct MediaRow<'a> {
    item: &'a BaseItemDto,
    image: Option<ImageRequest>,
    heading: &'a str,
    detail: &'a str,
    overview: Option<&'a str>,
    played: bool,
    handles: Option<Handles>,
    highlighted: bool,
    compact: bool,
    phone: bool,
}

/// A phone's row ([`MediaRow::phone`]): the image's size, the Watched toggle's, and how many
/// lines the heading and the overview take.
const PHONE_IMAGE_W: f32 = 136.;
const PHONE_IMAGE_H: f32 = 76.5;
const PHONE_WATCHED: f32 = 36.;
const PHONE_WATCHED_ICON: f32 = 20.;
const PHONE_HEADING_ROWS: usize = 2;
const PHONE_OVERVIEW_ROWS: usize = 2;
/// How far a phone row's pressed fill reaches past it.
const PHONE_FILL_OUT: f32 = 6.;
/// Between a phone's rows, which have no fill to set them apart.
pub const PHONE_GAP: f32 = 18.;

/// What a row's parts did this frame. `row` covers the whole row; the toggle and the handles
/// are on top of it and take the clicks on them.
pub struct MediaRowResponse {
    pub row: Response,
    pub watched: Response,
    pub handle: Option<Handle>,
}

impl<'a> MediaRow<'a> {
    /// `item` is what the image's placeholder comes from; `heading` names the row to AccessKit.
    pub fn new(item: &'a BaseItemDto, image: Option<ImageRequest>, heading: &'a str, detail: &'a str) -> Self {
        Self {
            item,
            image,
            heading,
            detail,
            overview: None,
            played: false,
            handles: None,
            highlighted: false,
            compact: false,
            phone: false,
        }
    }

    /// A phone's row (`layout::Size::Compact`): no fill at rest, the image and beside it the
    /// heading on up to two lines, the detail under it and the Watched toggle at the right; the
    /// overview's first two lines under them across the width.
    pub fn phone(mut self, phone: bool) -> Self {
        self.phone = phone;
        self
    }

    pub fn overview(mut self, overview: Option<&'a str>) -> Self {
        self.overview = overview.filter(|text| !text.is_empty());
        self
    }

    pub fn played(mut self, played: bool) -> Self {
        self.played = played;
        self
    }

    /// A playlist entry's ↑ ↓ ✕, shown while the row is active.
    pub fn handles(mut self, handles: Handles) -> Self {
        self.handles = Some(handles);
        self
    }

    /// The page's cursor is on it.
    pub fn highlighted(mut self, highlighted: bool) -> Self {
        self.highlighted = highlighted;
        self
    }

    /// The smaller row, for a list in columns.
    pub fn compact(mut self, compact: bool) -> Self {
        self.compact = compact;
        self
    }

    /// Draws the row across `ui`'s width.
    pub fn show(self, ui: &mut Ui, images: &mut ImageCache) -> MediaRowResponse {
        if self.phone {
            return self.show_phone(ui, images);
        }
        let size = if self.compact { &COMPACT } else { &FULL };
        let tokens = theme::tokens_of(ui);
        let small = text::font(ui, "sm");
        let heading_font = text::font(ui, "base");
        let width = ui.available_width();
        let inner_width = width - 2. * PAD;
        let handles_w = if self.handles.is_some() {
            list_row::HANDLES_W + size.gap
        } else {
            0.
        };
        let text_width = (inner_width - size.image_w - 2. * size.gap - handles_w - size.watched).max(1.);
        let overview = self
            .overview
            .map(|text| overview_galley(ui, text, text_width, size.overview_rows));
        let heading_h = ui.fonts_mut(|fonts| fonts.row_height(&heading_font));
        let detail_h = ui.fonts_mut(|fonts| fonts.row_height(&small));
        let mut text_h = heading_h + LINE_GAP + detail_h;
        if let Some(galley) = &overview {
            text_h += LINE_GAP + galley.size().y;
        }
        let height = size.image_h.max(text_h) + 2. * PAD;
        let (rect, row) = ui.allocate_exact_size(vec2(width, height), Sense::click());
        row.widget_info(|| WidgetInfo::labeled(Role::Button, true, self.heading));
        let active = self.highlighted || ui.rect_contains_pointer(rect);
        let fill = if active { tokens.inset_hover } else { tokens.inset };
        ui.painter().rect_filled(rect, tokens.radius_control, fill);
        if self.highlighted {
            focus::ring(ui, rect, tokens.radius_control as f32, Place::Inside);
        }

        let inner = rect.shrink(PAD);
        // The row's content goes in a Ui of its own, so that placing the buttons in their rects
        // moves that Ui's cursor rather than the list's (the next row would overlap this one).
        let mut content = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)));
        let id = row.id;
        let image_rect = Rect::from_min_size(inner.min, vec2(size.image_w, size.image_h));
        image::show(
            &content,
            images,
            image_rect,
            self.image.as_ref(),
            Some(self.item),
            IMAGE_RADIUS,
        );

        // A very narrow window (or a big zoom) can leave nothing beside the image; the column
        // keeps a point rather than turning the rect inside out.
        let text_left = image_rect.right() + size.gap;
        let text_rect = Rect::from_min_max(
            pos2(text_left, inner.top()),
            pos2(
                (inner.right() - handles_w - size.watched - size.gap).max(text_left + 1.),
                inner.bottom(),
            ),
        );
        let mut text_ui = content.new_child(
            UiBuilder::new()
                .max_rect(text_rect)
                .layout(Layout::top_down(Align::Min)),
        );
        text_ui.spacing_mut().item_spacing.y = LINE_GAP;
        text_ui.add(Label::new(RichText::new(self.heading).font(heading_font).color(tokens.ink)).truncate());
        if !self.detail.is_empty() {
            text_ui.add(Label::new(RichText::new(self.detail).font(small).color(tokens.ink_muted)).truncate());
        }
        if let Some(galley) = overview {
            text_ui.add(Label::new(galley).selectable(false));
        }

        // The Watched toggle at the row's right end, level with the heading.
        let watched_center = pos2(inner.right() - size.watched / 2., inner.top() + size.watched / 2.);
        let icon = if self.played {
            ICON_CHECK_CIRCLE
        } else {
            ICON_CHECK.outlined()
        };
        let watched = RoundButton::icon(icon, "Watched (W)")
            .size(size.watched, size.watched_icon)
            .on(self.played)
            .focus(Focus::Skip)
            .show_at(&mut content, watched_center, id.with("watched"));

        // Up, Down and Remove, against the toggle.
        let handle = self.handles.filter(|_| active).and_then(|moves| {
            let right = inner.right() - size.watched;
            let remove = "Remove from playlist (Delete)";
            list_row::handles(&mut content, right, watched_center.y, id, moves, remove)
        });
        MediaRowResponse { row, watched, handle }
    }

    /// [`MediaRow::phone`]'s row across `ui`'s width.
    fn show_phone(self, ui: &mut Ui, images: &mut ImageCache) -> MediaRowResponse {
        let tokens = theme::tokens_of(ui);
        let width = ui.available_width();
        let gap = 12.;
        let text_w = (width - PHONE_IMAGE_W - gap - PHONE_WATCHED - 4.).max(1.);
        let heading = wrapped(ui, self.heading, "base", tokens.ink, text_w, PHONE_HEADING_ROWS);
        let detail = (!self.detail.is_empty()).then(|| wrapped(ui, self.detail, "sm", tokens.ink_muted, text_w, 1));
        let overview = self
            .overview
            .map(|text| overview_galley(ui, text, width, PHONE_OVERVIEW_ROWS));
        let text_h = heading.size().y + detail.as_ref().map_or(0., |detail| LINE_GAP + detail.size().y);
        let top_h = PHONE_IMAGE_H.max(text_h);
        let height = top_h + overview.as_ref().map_or(0., |galley| 6. + galley.size().y);
        let (rect, row) = ui.allocate_exact_size(vec2(width, height), Sense::click());
        row.widget_info(|| WidgetInfo::labeled(Role::Button, true, self.heading));
        let filled = rect.expand(PHONE_FILL_OUT);
        if self.highlighted || row.is_pointer_button_down_on() {
            ui.painter()
                .rect_filled(filled, tokens.radius_control, tokens.inset_hover);
        }
        if self.highlighted {
            focus::ring(ui, filled, tokens.radius_control as f32, Place::Inside);
        }
        let mut content = ui.new_child(UiBuilder::new().max_rect(rect).layout(Layout::top_down(Align::Min)));
        let image_rect = Rect::from_min_size(rect.min, vec2(PHONE_IMAGE_W, PHONE_IMAGE_H));
        image::show(
            &content,
            images,
            image_rect,
            self.image.as_ref(),
            Some(self.item),
            IMAGE_RADIUS,
        );
        if let Some(fraction) = crate::widgets::card::progress(self.item) {
            crate::widgets::card::progress_bar(&content, image_rect, fraction);
        }
        let text_left = image_rect.right() + gap;
        let mut y = rect.top() + (PHONE_IMAGE_H - text_h).max(0.) / 2.;
        let heading_h = heading.size().y;
        content.painter().galley(pos2(text_left, y), heading, tokens.ink);
        y += heading_h + LINE_GAP;
        if let Some(detail) = detail {
            content.painter().galley(pos2(text_left, y), detail, tokens.ink_muted);
        }
        if let Some(galley) = overview {
            content
                .painter()
                .galley(pos2(rect.left(), rect.top() + top_h + 6.), galley, tokens.ink_muted);
        }
        let watched_center = pos2(rect.right() - PHONE_WATCHED / 2., rect.top() + PHONE_IMAGE_H / 2.);
        let icon = if self.played {
            ICON_CHECK_CIRCLE
        } else {
            ICON_CHECK.outlined()
        };
        let watched = RoundButton::icon(icon, "Watched (W)")
            .size(PHONE_WATCHED, PHONE_WATCHED_ICON)
            .on(self.played)
            .focus(Focus::Skip)
            .show_at(&mut content, watched_center, row.id.with("watched"));
        MediaRowResponse {
            row,
            watched,
            handle: None,
        }
    }
}

/// `text` at `size` wrapped to `width`, cut off after `rows` lines with an ellipsis.
fn wrapped(ui: &Ui, text: &str, size: &str, color: Color32, width: f32, rows: usize) -> Arc<Galley> {
    let mut job = LayoutJob::default();
    job.append(
        text,
        0.,
        TextFormat {
            font_id: text::font(ui, size),
            color,
            ..Default::default()
        },
    );
    job.wrap.max_width = width;
    job.wrap.max_rows = rows;
    ui.fonts_mut(|fonts| fonts.layout_job(job))
}

/// `text` wrapped to `width`, cut off after `rows` lines with an ellipsis. Laid out here rather
/// than through a `Label`: a label replaces the job's wrapping, and with it the row limit.
fn overview_galley(ui: &Ui, text: &str, width: f32, rows: usize) -> Arc<Galley> {
    let mut job = LayoutJob::default();
    job.append(
        text,
        0.,
        TextFormat {
            font_id: text::font(ui, "sm"),
            color: theme::tokens_of(ui).ink_muted,
            ..Default::default()
        },
    );
    job.wrap.max_width = width;
    job.wrap.max_rows = rows;
    ui.fonts_mut(|fonts| fonts.layout_job(job))
}

/// How many compact rows fit across `width`, each at least [`EPISODE_MIN_W`] wide.
pub fn episode_columns(width: f32) -> usize {
    (((width + GAP) / (EPISODE_MIN_W + GAP)).floor() as usize).max(1)
}

/// How many season rows fit across `width`, each at least [`SEASON_MIN_W`] wide.
pub fn season_columns(width: f32) -> usize {
    (((width + GAP) / (SEASON_MIN_W + GAP)).floor() as usize).max(1)
}

/// A series' season as an inset row `width` wide: its poster, its name, `line` under it in
/// `ink_muted` (how many episodes, how many are left), and a chevron at the right end. `open`
/// says its episodes are showing: the row stays `inset_hover`, with a `brand` bar at its left
/// (`list_row::open_bar`) and the chevron turned up in `brand`.
#[allow(clippy::too_many_arguments)]
pub fn season(
    ui: &mut Ui,
    images: &mut ImageCache,
    item: &BaseItemDto,
    poster: Option<ImageRequest>,
    line: &str,
    width: f32,
    open: bool,
    highlighted: bool,
) -> Response {
    let tokens = theme::tokens_of(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(width, SEASON_H), Sense::click());
    response.widget_info(|| WidgetInfo::selected(Role::Button, true, open, &item.name));
    let fill = if response.hovered() || highlighted || open {
        tokens.inset_hover
    } else {
        tokens.inset
    };
    ui.painter().rect_filled(rect, tokens.radius_control, fill);
    let height = SEASON_H - 2. * PAD;
    let poster_rect = Rect::from_min_size(rect.min + vec2(PAD + 4., PAD), vec2(height * 2. / 3., height));
    image::show(ui, images, poster_rect, poster.as_ref(), Some(item), IMAGE_RADIUS);

    let chevron_x = rect.right() - 26.;
    let x = poster_rect.right() + 14.;
    let text_w = (chevron_x - 16. - x).max(1.);
    let painter = ui.painter();
    let name = painter.layout(item.name.clone(), text::font(ui, "base"), tokens.ink, text_w);
    painter.galley(pos2(x, rect.center().y - 10. - name.size().y / 2.), name, tokens.ink);
    painter.text(
        pos2(x, rect.center().y + 12.),
        Align2::LEFT_CENTER,
        line,
        text::font(ui, "sm"),
        tokens.ink_muted,
    );
    let (chevron, color) = if open {
        (ICON_EXPAND_LESS, tokens.brand)
    } else {
        (ICON_EXPAND_MORE, tokens.ink_muted)
    };
    round_button::glyph(ui, pos2(chevron_x, rect.center().y), chevron, 24., color);
    if open {
        list_row::open_bar(ui, rect);
    }
    if highlighted {
        focus::ring(ui, rect, tokens.radius_control as f32, Place::Inside);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A chapter row's colours, from the caller: the player draws it over the video, in its own dark
/// set.
pub struct ChapterColours {
    /// The current chapter's row, and its name.
    pub current: Color32,
    pub ink_current: Color32,
    /// A row under the pointer.
    pub hover: Color32,
    pub ink: Color32,
    pub ink_muted: Color32,
}

/// A chapter of the player's chapters menu: its thumbnail (when the server extracted them), its
/// name over its start, centred on the thumbnail. The current one is tinted `colours.current`
/// with its name in `colours.ink_current`, and a row under the pointer takes `colours.hover`. A
/// click-only button named "name, start" to AccessKit, selected on the current one.
pub fn chapter(
    ui: &mut Ui,
    images: &mut ImageCache,
    image: Option<&ImageRequest>,
    name: &str,
    start: &str,
    current: bool,
    colours: ChapterColours,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width(), CHAPTER_THUMB_H + 2. * CHAPTER_PAD),
        Sense::CLICK,
    );
    response.widget_info(|| WidgetInfo::selected(Role::Button, ui.is_enabled(), current, format!("{name}, {start}")));
    let radius = CornerRadius::same(CHAPTER_ROW_RADIUS);
    if current {
        ui.painter().rect_filled(rect, radius, colours.current);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, radius, colours.hover);
    }
    let thumb = Rect::from_min_size(
        rect.min + vec2(CHAPTER_PAD, CHAPTER_PAD),
        vec2(CHAPTER_THUMB_W, CHAPTER_THUMB_H),
    );
    image::show(ui, images, thumb, image, None, CHAPTER_IMAGE_RADIUS);
    let x = thumb.right() + CHAPTER_TEXT_GAP;
    let colour = if current { colours.ink_current } else { colours.ink };
    ui.painter().text(
        pos2(x, thumb.center().y - 2.),
        Align2::LEFT_BOTTOM,
        name,
        text::font(ui, "base"),
        colour,
    );
    ui.painter().text(
        pos2(x, thumb.center().y + 2.),
        Align2::LEFT_TOP,
        start,
        text::font(ui, "sm"),
        colours.ink_muted,
    );
    response
}
