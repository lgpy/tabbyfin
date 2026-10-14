//! Shared building blocks for browsing pages.

use gpui_kit::component::button::Button;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme, Icon, IconName, Sizable, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jellyfin::models::{BaseItemDto, BaseItemKind, ImageType};
use jellyfin::{ImageRequest, Ticks};

use crate::images::ImageCache;

pub const POSTER_W: f32 = 150.;
pub const POSTER_H: f32 = 225.;
pub const LANDSCAPE_W: f32 = 260.;
pub const LANDSCAPE_H: f32 = 146.;
pub const CARD_GAP: f32 = 16.;
/// Height of the text block under a card (title + subtitle lines).
pub const CARD_TEXT_H: f32 = 44.;

/// Result of an async page load.
pub enum Load<T> {
    Loading,
    Loaded(T),
    Failed(SharedString),
}

/// An image filling its parent, or nothing while it loads (the parent's background shows).
pub fn remote_image(req: Option<ImageRequest>, cx: &mut App) -> AnyElement {
    let image = req.and_then(|req| ImageCache::global(cx).update(cx, |cache, cx| cache.get(&req, cx)));
    match image {
        Some(image) => img(image).size_full().object_fit(ObjectFit::Cover).into_any_element(),
        None => div().size_full().into_any_element(),
    }
}

/// Wide image for an item: its own thumb/backdrop, then the parent's, then the primary image
/// (episode screen grabs are already landscape).
pub fn landscape_image(item: &BaseItemDto) -> Option<ImageRequest> {
    item.image(ImageType::Thumb)
        .or_else(|| {
            let id = item.parent_thumb_item_id.as_ref()?;
            let tag = item.parent_thumb_image_tag.clone()?;
            (item.kind != BaseItemKind::Episode).then(|| ImageRequest::new(id, ImageType::Thumb, Some(tag)))
        })
        .or_else(|| {
            (item.kind == BaseItemKind::Episode)
                .then(|| item.image(ImageType::Primary))
                .flatten()
        })
        .or_else(|| item.backdrop_image())
        .or_else(|| item.image(ImageType::Primary))
}

/// Title and secondary line for a card. Episodes show the series as the title.
pub fn card_text(item: &BaseItemDto) -> (String, String) {
    match item.kind {
        BaseItemKind::Episode => (
            item.series_name.clone().unwrap_or_else(|| item.name.clone()),
            format!("{} · {}", episode_label(item), item.name),
        ),
        BaseItemKind::Season => (
            item.name.clone(),
            item.user_data
                .as_ref()
                .and_then(|d| d.unplayed_item_count)
                .filter(|n| *n > 0)
                .map(|n| format!("{n} unwatched"))
                .unwrap_or_default(),
        ),
        _ => (
            item.name.clone(),
            item.production_year.map(|y| y.to_string()).unwrap_or_default(),
        ),
    }
}

/// `S1:E2`, or an empty string when numbering is unknown.
pub fn episode_label(item: &BaseItemDto) -> String {
    match (item.parent_index_number, item.index_number) {
        (Some(s), Some(e)) => format!("S{s}:E{e}"),
        (None, Some(e)) => format!("E{e}"),
        _ => String::new(),
    }
}

/// `1:02:03`, `4:05`.
pub fn timestamp(time: std::time::Duration) -> String {
    let secs = time.as_secs();
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// `1h 42m`, `45m`.
pub fn runtime_text(ticks: Ticks) -> String {
    let minutes = (ticks.as_secs_f64() / 60.).round() as u64;
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h {m}m"),
    }
}

fn is_played(item: &BaseItemDto) -> bool {
    item.user_data.as_ref().is_some_and(|d| d.played)
}

fn progress(item: &BaseItemDto) -> Option<f32> {
    let pct = item.user_data.as_ref()?.played_percentage?;
    (pct > 0. && pct < 100.).then_some(pct as f32 / 100.)
}

fn played_badge(cx: &App) -> impl IntoElement {
    div()
        .absolute()
        .top_1()
        .right_1()
        .rounded_full()
        .p_0p5()
        .bg(cx.theme().primary)
        .text_color(cx.theme().primary_foreground)
        .child(Icon::new(IconName::Check).xsmall())
}

fn progress_bar(fraction: f32, cx: &App) -> impl IntoElement {
    div()
        .absolute()
        .bottom_0()
        .left_0()
        .right_0()
        .h(px(4.))
        .bg(black().opacity(0.6))
        .child(div().h_full().w(relative(fraction)).bg(cx.theme().primary))
}

/// The keyboard cursor's highlight, drawn over its parent's edge (inside, so scrolling
/// containers don't clip it). The parent must be `relative` and rounded like this.
pub fn selection_ring(cx: &App) -> Div {
    div()
        .absolute()
        .inset_0()
        .rounded_md()
        .border_3()
        .border_color(cx.theme().foreground)
}

fn card(
    id: ElementId,
    item: &BaseItemDto,
    (width, height): (f32, f32),
    image: Option<ImageRequest>,
    selected: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &mut App,
) -> Stateful<Div> {
    let (title, subtitle) = card_text(item);
    let played = is_played(item);
    let progress = progress(item);
    let image = remote_image(image, cx);
    v_flex()
        .id(id)
        .w(px(width))
        .flex_none()
        .gap_1()
        .cursor_pointer()
        .on_click(on_click)
        .child(
            div()
                .relative()
                .w(px(width))
                .h(px(height))
                .rounded_md()
                .overflow_hidden()
                .bg(cx.theme().muted)
                .hover(|style| style.opacity(0.8))
                .child(image)
                .when(played, |el| el.child(played_badge(cx)))
                .when_some(progress, |el, fraction| el.child(progress_bar(fraction, cx)))
                .when(selected, |el| el.child(selection_ring(cx))),
        )
        .child(div().text_sm().truncate().child(title))
        .child(
            div()
                .text_xs()
                .truncate()
                .text_color(cx.theme().muted_foreground)
                .child(subtitle),
        )
}

pub fn poster_card(
    id: impl Into<ElementId>,
    item: &BaseItemDto,
    selected: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &mut App,
) -> Stateful<Div> {
    let image = item.primary_image().map(|r| r.fill_width((POSTER_W * 2.) as u32));
    card(id.into(), item, (POSTER_W, POSTER_H), image, selected, on_click, cx)
}

pub fn landscape_card(
    id: impl Into<ElementId>,
    item: &BaseItemDto,
    selected: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &mut App,
) -> Stateful<Div> {
    let image = landscape_image(item).map(|r| r.fill_width((LANDSCAPE_W * 2.) as u32));
    card(
        id.into(),
        item,
        (LANDSCAPE_W, LANDSCAPE_H),
        image,
        selected,
        on_click,
        cx,
    )
}

/// Grey card used while a grid page is still loading.
pub fn placeholder_card(cx: &App) -> impl IntoElement {
    v_flex()
        .w(px(POSTER_W))
        .flex_none()
        .gap_1()
        .child(div().w(px(POSTER_W)).h(px(POSTER_H)).rounded_md().bg(cx.theme().muted))
        .child(div().h(px(CARD_TEXT_H - 4.)))
}

/// A titled, horizontally scrolling row of cards. The cards are the direct children of
/// `scroll`, so [`crate::browse::reveal`] can scroll to them.
pub fn shelf(
    id: impl Into<ElementId>,
    title: impl Into<SharedString>,
    cards: Vec<AnyElement>,
    scroll: &ScrollHandle,
    cx: &App,
) -> impl IntoElement {
    v_flex()
        .gap_2()
        .child(
            div()
                .px_6()
                .text_lg()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(cx.theme().foreground)
                .child(title.into()),
        )
        .child(
            div()
                .id(id)
                .flex()
                .px_6()
                .pb_2()
                .gap(px(CARD_GAP))
                .overflow_x_scroll()
                .track_scroll(scroll)
                .children(cards),
        )
}

pub fn centered_spinner() -> impl IntoElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .child(Spinner::new())
}

pub fn centered_message(message: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .text_color(cx.theme().muted_foreground)
        .child(message.into())
}

/// A failed load with a Retry button.
pub fn error_state(
    message: impl Into<SharedString>,
    on_retry: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> impl IntoElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap_3()
        .child(div().text_color(cx.theme().muted_foreground).child(message.into()))
        .child(Button::new("retry").outline().label("Retry").on_click(on_retry))
}
