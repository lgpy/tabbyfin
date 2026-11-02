//! An item as an inset row of a list: a season's episodes and a playlist's entries
//! ([`MediaRow`]), and a series' seasons ([`season`]). Inset like `AvatarRow`: `inset` at rest,
//! `inset_hover` under the pointer or a page's cursor (with the keyboard's ring inside),
//! `radius_control`, [`GAP`] apart.
//!
//! An episode row has the item's wide image, its heading in `ink`, a muted line under it, up to
//! three lines of overview in `ink_muted`, and at its right end a round Watched toggle. A
//! playlist's row also has round ↑ ↓ ✕ buttons before the toggle, shown only while the row is
//! active (under the pointer or the cursor), as the SyncPlay queue's are: the list stays quiet,
//! and the keys move and remove any entry anyway.

use std::sync::Arc;

use egui::accesskit::Role;
use egui::text::LayoutJob;
use egui::{
    Align, Align2, Galley, Label, Layout, Rect, Response, RichText, Sense, TextFormat, Ui, UiBuilder, WidgetInfo,
};
use egui::{pos2, vec2};
use egui_material_icons::icons::{
    ICON_ARROW_DOWNWARD, ICON_ARROW_UPWARD, ICON_CHECK, ICON_CHECK_CIRCLE, ICON_CLOSE, ICON_EXPAND_LESS,
    ICON_EXPAND_MORE,
};
use jellyfin::ImageRequest;
use jellyfin::models::BaseItemDto;

use crate::images::ImageCache;
use crate::theme::{self, text};
use crate::widgets::button::Focus;
use crate::widgets::focus::{self, Place};
use crate::widgets::image;
use crate::widgets::round_button::{self, RoundButton};
use crate::widgets::surface;
use crate::widgets::text::{LANDSCAPE_H, LANDSCAPE_W};

/// Between the rows of a list.
pub const GAP: f32 = surface::GAP_ITEM;
/// The room inside a row, round its image.
const PAD: f32 = 10.;
/// Between the image, the text and the buttons.
const ROW_GAP: f32 = 16.;
/// Between the heading, the line under it and the overview.
const LINE_GAP: f32 = 4.;
/// The image's corners, concentric enough with the row's inside its padding.
const IMAGE_RADIUS: u8 = 6;
const WATCHED_SIZE: f32 = 36.;
const WATCHED_ICON: f32 = 20.;
const HANDLE_SIZE: f32 = 30.;
const HANDLE_ICON: f32 = 18.;
/// How many lines of overview show, as `main`'s `line_clamp(3)`.
const OVERVIEW_ROWS: usize = 3;

/// A season row's height, and the narrowest one gets before the list takes fewer columns.
const SEASON_H: f32 = 84.;
pub const SEASON_MIN_W: f32 = 300.;

/// A playlist entry's buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    Up,
    Down,
    Remove,
}

/// Which of a playlist entry's moves are possible: none past either end of the list.
#[derive(Clone, Copy, Debug)]
pub struct Handles {
    pub up: bool,
    pub down: bool,
}

pub struct MediaRow<'a> {
    item: &'a BaseItemDto,
    image: Option<ImageRequest>,
    heading: &'a str,
    detail: &'a str,
    overview: Option<&'a str>,
    played: bool,
    handles: Option<Handles>,
    highlighted: bool,
}

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
        }
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

    /// Draws the row across `ui`'s width.
    pub fn show(self, ui: &mut Ui, images: &mut ImageCache) -> MediaRowResponse {
        let tokens = theme::tokens_of(ui);
        let small = text::font(ui, "sm");
        let heading_font = text::font(ui, "base");
        let width = ui.available_width();
        let inner_width = width - 2. * PAD;
        let handles_w = if self.handles.is_some() {
            3. * HANDLE_SIZE + ROW_GAP
        } else {
            0.
        };
        let text_width = (inner_width - LANDSCAPE_W - 2. * ROW_GAP - handles_w - WATCHED_SIZE).max(1.);
        let overview = self.overview.map(|text| overview_galley(ui, text, text_width));
        let heading_h = ui.fonts_mut(|fonts| fonts.row_height(&heading_font));
        let detail_h = ui.fonts_mut(|fonts| fonts.row_height(&small));
        let mut text_h = heading_h + LINE_GAP + detail_h;
        if let Some(galley) = &overview {
            text_h += LINE_GAP + galley.size().y;
        }
        let height = LANDSCAPE_H.max(text_h) + 2. * PAD;
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
        let image_rect = Rect::from_min_size(inner.min, vec2(LANDSCAPE_W, LANDSCAPE_H));
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
        let text_left = image_rect.right() + ROW_GAP;
        let text_rect = Rect::from_min_max(
            pos2(text_left, inner.top()),
            pos2(
                (inner.right() - handles_w - WATCHED_SIZE - ROW_GAP).max(text_left + 1.),
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
        let watched_center = pos2(inner.right() - WATCHED_SIZE / 2., inner.top() + WATCHED_SIZE / 2.);
        let icon = if self.played {
            ICON_CHECK_CIRCLE
        } else {
            ICON_CHECK.outlined()
        };
        let watched = RoundButton::icon(icon, "Watched (W)")
            .size(WATCHED_SIZE, WATCHED_ICON)
            .on(self.played)
            .focus(Focus::Skip)
            .show_at(&mut content, watched_center, id.with("watched"));

        let mut handle = None;
        if let Some(handles) = self.handles
            && active
        {
            // Up, Down and Remove, left to right, as on `main`.
            let buttons = [
                (Handle::Up, ICON_ARROW_UPWARD, "Move up (Alt+Up)", handles.up),
                (Handle::Down, ICON_ARROW_DOWNWARD, "Move down (Alt+Down)", handles.down),
                (Handle::Remove, ICON_CLOSE, "Remove from playlist (Delete)", true),
            ];
            let mut x = inner.right() - WATCHED_SIZE - ROW_GAP - handles_w + ROW_GAP + HANDLE_SIZE / 2.;
            for (which, icon, label, enabled) in buttons {
                let response = RoundButton::icon(icon.outlined(), label)
                    .size(HANDLE_SIZE, HANDLE_ICON)
                    .enabled(enabled)
                    .focus(Focus::Skip)
                    .show_at(&mut content, pos2(x, watched_center.y), id.with(label));
                if response.clicked() {
                    handle = Some(which);
                }
                x += HANDLE_SIZE;
            }
        }
        MediaRowResponse { row, watched, handle }
    }
}

/// `text` wrapped to `width`, cut off after [`OVERVIEW_ROWS`] lines with an ellipsis. Laid out
/// here rather than through a `Label`: a label replaces the job's wrapping, and with it the
/// row limit.
fn overview_galley(ui: &Ui, text: &str, width: f32) -> Arc<Galley> {
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
    job.wrap.max_rows = OVERVIEW_ROWS;
    ui.fonts_mut(|fonts| fonts.layout_job(job))
}

/// How many season rows fit across `width`, each at least [`SEASON_MIN_W`] wide.
pub fn season_columns(width: f32) -> usize {
    (((width + GAP) / (SEASON_MIN_W + GAP)).floor() as usize).max(1)
}

/// A series' season as an inset row `width` wide: its poster, its name, `line` under it in
/// `ink_muted` (how many episodes, how many are left), and a chevron at the right end. `open`
/// says its episodes are showing: the row stays `inset_hover`, with a 3 pt `brand` bar at its
/// left edge and the chevron turned up in `brand`.
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
        let bar = Rect::from_min_max(
            pos2(rect.left(), rect.top() + 16.),
            pos2(rect.left() + 3., rect.bottom() - 16.),
        );
        ui.painter().rect_filled(bar, 2, tokens.brand);
    }
    if highlighted {
        focus::ring(ui, rect, tokens.radius_control as f32, Place::Inside);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}
