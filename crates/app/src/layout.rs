//! The window's layout size and whether it's driven by touch, which the pages and widgets read
//! to fit a phone.
//!
//! Touch is Android's: a desktop window stays [`Size::Expanded`] however narrow it gets, so
//! nothing on the desktop changes. The UI tests turn touch on for a context with [`set_touch`]
//! to drive the phone's layout headless.
//!
//! The sizes are Android's window size classes, by the width the app lays out in (egui's
//! content rectangle, clear of the system bars), in points, which are Android's dp.

use egui::{Context, Id, Rect, Ui, Vec2};

/// Under this many points wide, the layout is [`Size::Compact`]: a phone in portrait.
pub const COMPACT_BELOW: f32 = 600.;
/// Under this, [`Size::Medium`]: a phone in landscape, a small tablet in portrait.
pub const MEDIUM_BELOW: f32 = 840.;
/// The least a touch target measures across: Android's 48 dp.
pub const TARGET: f32 = 48.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    /// A phone in portrait: a bottom bar for the places to go, one column for the rest.
    Compact,
    /// The desktop's layout with touch targets.
    Medium,
    /// The desktop's layout.
    Expanded,
}

fn touch_id() -> Id {
    Id::unique("layout_touch")
}

/// Whether the app is driven by touch: on Android, unless a test said otherwise.
pub fn touch(ctx: &Context) -> bool {
    ctx.data(|data| data.get_temp::<bool>(touch_id()))
        .unwrap_or(cfg!(target_os = "android"))
}

/// Turns touch on or off for `ctx`: the UI tests' way to the phone's layout.
#[cfg_attr(not(test), allow(dead_code))]
pub fn set_touch(ctx: &Context, on: bool) {
    ctx.data_mut(|data| data.insert_temp(touch_id(), on));
}

/// The layout size for the window's width; [`Size::Expanded`] without touch.
pub fn size(ctx: &Context) -> Size {
    if !touch(ctx) {
        return Size::Expanded;
    }
    of_width(ctx.content_rect().width())
}

/// The size for a window `width` points wide, with touch.
pub fn of_width(width: f32) -> Size {
    if width < COMPACT_BELOW {
        Size::Compact
    } else if width < MEDIUM_BELOW {
        Size::Medium
    } else {
        Size::Expanded
    }
}

pub fn compact(ctx: &Context) -> bool {
    size(ctx) == Size::Compact
}

/// A page's distance from the window's sides: 24 points, 16 on a phone.
pub fn side(ctx: &Context) -> f32 {
    if compact(ctx) { 16. } else { 24. }
}

/// The rectangle a control drawn in `rect` answers touches in: grown to [`TARGET`] each way
/// round its centre with touch, `rect` itself without. The control draws at its own size; only
/// what it senses grows, so a row of small buttons keeps its look.
pub fn target(ui: &Ui, rect: Rect) -> Rect {
    if !touch(ui.ctx()) {
        return rect;
    }
    Rect::from_center_size(rect.center(), rect.size().max(Vec2::splat(TARGET)))
}

/// `response` answering touches over [`target`]'s rectangle too, for a control smaller than a
/// finger: the grown rectangle is sensed as its own widget and the two responses joined, so a
/// tap just outside the drawn control presses it. Without touch, `response` as it is.
pub fn grow(ui: &Ui, response: egui::Response, sense: egui::Sense) -> egui::Response {
    let grown = target(ui, response.rect);
    if grown == response.rect {
        return response;
    }
    let outer = ui.interact(grown, response.id.with("touch_target"), sense);
    response.union(outer)
}

/// How the page's main scroll area moved in a frame, for the bottom bar to hide while the page
/// scrolls down and come back as it scrolls up.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PageScroll {
    /// Points the content moved up this frame (the offset grew); negative when it moved down.
    pub delta: f32,
    /// The area's offset from its top now.
    pub offset: f32,
    /// Scrolled as far down as it goes. The offset moves back here when the area grows (as the
    /// bar slides away), which isn't a scroll up.
    pub at_end: bool,
    /// How far it scrolls at all.
    pub max: f32,
}

fn page_scroll_id() -> Id {
    Id::unique("page_scroll")
}

/// Notes where the page's main scroll area `area` is this frame, `offset` of at most `max`
/// (`backdrop::scroll_edge` does, since every page's main area calls it). A new area, as a page
/// opens, has moved nowhere yet.
pub fn note_page_scroll(ctx: &Context, area: Id, offset: f32, max: f32) {
    let last_id = area.with("page_scroll_last");
    ctx.data_mut(|data| {
        let last = data.get_temp::<f32>(last_id);
        data.insert_temp(last_id, offset);
        let delta = last.map_or(0., |last| offset - last);
        let scroll = data.get_temp_mut_or_default::<PageScroll>(page_scroll_id());
        scroll.delta += delta;
        scroll.offset = offset;
        scroll.at_end = offset >= max - 1.;
        scroll.max = max;
    });
}

/// How the page scrolled since this was last taken, if it was drawn.
pub fn take_page_scroll(ctx: &Context) -> Option<PageScroll> {
    ctx.data_mut(|data| data.remove_temp::<PageScroll>(page_scroll_id()))
}

/// Whether the soft keyboard is up: it takes the bottom of the window out of the content
/// rectangle, much more of it than the gesture bar does.
pub fn keyboard_open(ctx: &Context) -> bool {
    touch(ctx) && ctx.viewport_rect().bottom() - ctx.content_rect().bottom() > 120.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_follow_androids_window_classes() {
        assert_eq!(of_width(369.), Size::Compact);
        assert_eq!(of_width(599.), Size::Compact);
        assert_eq!(of_width(600.), Size::Medium);
        assert_eq!(of_width(820.), Size::Medium);
        assert_eq!(of_width(840.), Size::Expanded);
    }

    #[test]
    fn the_desktop_is_always_expanded() {
        let ctx = Context::default();
        set_touch(&ctx, false);
        assert_eq!(size(&ctx), Size::Expanded);
    }
}
