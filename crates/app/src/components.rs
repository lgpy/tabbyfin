//! Shared building blocks for browsing pages.

use std::hash::{Hash, Hasher};

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::component::popover::Popover;
use gpui_kit::component::{ActiveTheme, Disableable, Icon, IconName, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jellyfin::models::{BaseItemDto, BaseItemKind, BaseItemPerson, ChapterInfo, ImageType};
use jellyfin::{ImageRequest, Ticks};

use crate::images::ImageCache;
use crate::mascot::{confused_mascot, mascot};
use crate::motion;
use crate::smooth_scroll::glide_by;
use crate::zoom::{dp, dp_px};

pub const POSTER_W: f32 = 150.;
pub const POSTER_H: f32 = 225.;
pub const LANDSCAPE_W: f32 = 260.;
pub const LANDSCAPE_H: f32 = 146.;
pub const CARD_GAP: f32 = 16.;
/// A shelf's side padding (`px_6`).
const SHELF_PADDING: f32 = 24.;
/// Height of the text block under a card (title + subtitle lines).
pub const CARD_TEXT_H: f32 = 44.;

/// Result of an async page load.
pub enum Load<T> {
    Loading,
    Loaded(T),
    Failed(SharedString),
}

/// An image filling its parent. While it loads, its blurhash from `item` if there is one, or
/// nothing (the parent's background shows).
pub fn remote_image(req: Option<ImageRequest>, item: Option<&BaseItemDto>, cx: &mut App) -> AnyElement {
    image(req, item, false, cx)
}

/// [`remote_image`] with the cards' rounded corners. The image has to be rounded itself: a
/// rounded parent clips it to its rectangle only.
pub fn rounded_image(req: Option<ImageRequest>, item: Option<&BaseItemDto>, cx: &mut App) -> AnyElement {
    image(req, item, true, cx)
}

fn image(req: Option<ImageRequest>, item: Option<&BaseItemDto>, rounded: bool, cx: &mut App) -> AnyElement {
    let Some(req) = req else {
        return div().size_full().into_any_element();
    };
    let cache = ImageCache::global(cx);
    let image = cache.update(cx, |cache, cx| cache.get(&req, cx));
    let hash = item
        .zip(req.tag.as_deref())
        .and_then(|(item, tag)| item.blur_hash(req.image_type, tag));
    let preview = hash.and_then(|hash| cache.update(cx, |cache, cx| cache.placeholder(hash, cx)));
    // Pinned to the parent's edges: a `size_full` image's percentage height doesn't resolve here,
    // so it'd take the picture's own aspect ratio and run past the card's (clipped, square) bottom.
    let cover = |image| {
        img(ImageSource::Render(image))
            .absolute()
            .inset_0()
            .size_full()
            .object_fit(ObjectFit::Cover)
            .when(rounded, |img| img.rounded_xl())
    };
    let Some(image) = image else {
        return match preview {
            Some(preview) => cover(preview).into_any_element(),
            None => div().size_full().into_any_element(),
        };
    };
    // Fades in over the blurred preview when it loads (and when its page opens).
    let id = {
        let mut hasher = std::hash::DefaultHasher::new();
        req.hash(&mut hasher);
        ("image", hasher.finish() as usize)
    };
    div()
        .relative()
        .size_full()
        .children(preview.map(cover))
        .child(motion::fade_in(div().absolute().inset_0().child(cover(image)), id))
        .into_any_element()
}

/// Tall image for an item. An episode's own image is a landscape still that a poster would
/// crop, so it shows its series' poster, as Jellyfin's web client does (Latest lists a single
/// new episode on its own rather than under its series).
pub fn poster_image(item: &BaseItemDto) -> Option<ImageRequest> {
    let series_poster = || {
        let series_id = item.series_id.as_ref()?;
        let tag = item.series_primary_image_tag.clone()?;
        Some(ImageRequest::new(series_id, ImageType::Primary, Some(tag)))
    };
    match item.kind {
        BaseItemKind::Episode => series_poster().or_else(|| item.primary_image()),
        _ => item.primary_image(),
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

/// Inset from the card's edges, clear of its rounded corners.
fn progress_bar(fraction: f32, cx: &App) -> impl IntoElement {
    div()
        .absolute()
        .bottom_2()
        .left_2()
        .right_2()
        .h(dp(6.))
        .rounded_full()
        .overflow_hidden()
        .border_1()
        .border_color(crate::theme::over_video::track_outline())
        .bg(crate::theme::over_video::track())
        .child(
            div()
                .h_full()
                .w(relative(fraction))
                .rounded_full()
                .bg(cx.theme().primary),
        )
}

/// The keyboard cursor's highlight, drawn over its parent's edge (inside, so scrolling
/// containers don't clip it). The parent must be `relative` and rounded like this.
pub fn selection_ring(cx: &App) -> Div {
    div()
        .absolute()
        .inset_0()
        .rounded_xl()
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
    let title = div().text_sm().truncate().child(title).into_any_element();
    let subtitle = div()
        .text_xs()
        .truncate()
        .text_color(cx.theme().muted_foreground)
        .child(subtitle)
        .into_any_element();
    card_with_text(
        id,
        item,
        (width, height),
        image,
        title,
        Some(subtitle),
        selected,
        on_click,
        cx,
    )
}

/// [`card`] with its own title, and what goes under it if anything.
#[allow(clippy::too_many_arguments)]
fn card_with_text(
    id: ElementId,
    item: &BaseItemDto,
    (width, height): (f32, f32),
    image: Option<ImageRequest>,
    title: AnyElement,
    under: Option<AnyElement>,
    selected: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &mut App,
) -> Stateful<Div> {
    let played = is_played(item);
    let progress = progress(item);
    let image = rounded_image(image, Some(item), cx);
    v_flex()
        .id(id)
        .w(dp(width))
        .flex_none()
        .gap_1()
        .on_click(on_click)
        .child(
            div()
                .relative()
                .w(dp(width))
                .h(dp(height))
                .rounded_xl()
                .overflow_hidden()
                .bg(cx.theme().muted)
                .hover(|style| style.opacity(0.8))
                .child(image)
                .when(played, |el| el.child(played_badge(cx)))
                .when_some(progress, |el, fraction| el.child(progress_bar(fraction, cx)))
                .when(selected, |el| el.child(selection_ring(cx))),
        )
        .child(title)
        .children(under)
}

/// An episode in its season's row: "Episode 2" and its name (the season is already the row's
/// title), `scale` times a landscape card's size.
pub fn episode_card(
    id: impl Into<ElementId>,
    episode: &BaseItemDto,
    scale: f32,
    selected: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &mut App,
) -> Stateful<Div> {
    let (width, height) = (LANDSCAPE_W * scale, LANDSCAPE_H * scale);
    let image = landscape_image(episode).map(|r| r.fill_width((width * 2.).ceil() as u32));
    // "Episode 2" then its name, dimmer; a little below the image, and larger than other
    // cards' titles. The name is cut short when it doesn't fit.
    let number = episode.index_number.map(|number| format!("Episode {number}"));
    // Untitled episodes are named "Episode 3" by Jellyfin, which would say it twice.
    let name = Some(episode.name.trim()).filter(|name| !name.is_empty() && !is_placeholder_episode_name(name));
    let title = h_flex()
        .pt_1()
        .gap_2()
        .min_w_0()
        .text_base()
        .children(number.map(|number| div().flex_none().font_weight(FontWeight::MEDIUM).child(number)))
        .children(name.map(|name| {
            div()
                .min_w_0()
                .truncate()
                .text_color(cx.theme().muted_foreground)
                .child(name.to_owned())
        }))
        .into_any_element();
    card_with_text(
        id.into(),
        episode,
        (width, height),
        image,
        title,
        None,
        selected,
        on_click,
        cx,
    )
}

/// "Episode 3", "episode 03": the name Jellyfin gives an episode without a title.
fn is_placeholder_episode_name(name: &str) -> bool {
    name.split_once(' ').is_some_and(|(word, number)| {
        word.eq_ignore_ascii_case("episode") && !number.is_empty() && number.chars().all(|c| c.is_ascii_digit())
    })
}

pub fn poster_card(
    id: impl Into<ElementId>,
    item: &BaseItemDto,
    selected: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &mut App,
) -> Stateful<Div> {
    poster_card_scaled(id, item, 1., selected, on_click, cx)
}

/// [`poster_card`], `scale` times the size.
pub fn poster_card_scaled(
    id: impl Into<ElementId>,
    item: &BaseItemDto,
    scale: f32,
    selected: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &mut App,
) -> Stateful<Div> {
    let (width, height) = (POSTER_W * scale, POSTER_H * scale);
    let image = poster_image(item).map(|r| r.fill_width((width * 2.).ceil() as u32));
    card(id.into(), item, (width, height), image, selected, on_click, cx)
}

pub fn landscape_card(
    id: impl Into<ElementId>,
    item: &BaseItemDto,
    selected: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &mut App,
) -> Stateful<Div> {
    landscape_card_scaled(id, item, 1., selected, on_click, cx)
}

/// [`landscape_card`], `scale` times the size.
pub fn landscape_card_scaled(
    id: impl Into<ElementId>,
    item: &BaseItemDto,
    scale: f32,
    selected: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &mut App,
) -> Stateful<Div> {
    let (width, height) = (LANDSCAPE_W * scale, LANDSCAPE_H * scale);
    let image = landscape_image(item).map(|r| r.fill_width((width * 2.).ceil() as u32));
    card(id.into(), item, (width, height), image, selected, on_click, cx)
}

/// A chapter of `item_id`: its image when the server extracted one, its name and start.
pub fn chapter_card(
    id: impl Into<ElementId>,
    item_id: &str,
    index: usize,
    chapter: &ChapterInfo,
    selected: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &mut App,
) -> Stateful<Div> {
    let image = chapter.image_tag.clone().map(|tag| ImageRequest {
        index: Some(index as u32),
        ..ImageRequest::new(item_id, ImageType::Chapter, Some(tag)).fill_width((LANDSCAPE_W * 2.) as u32)
    });
    let image = rounded_image(image, None, cx);
    let name = chapter.name.clone().unwrap_or_else(|| format!("Chapter {}", index + 1));
    v_flex()
        .id(id)
        .w(dp(LANDSCAPE_W))
        .flex_none()
        .gap_1()
        .on_click(on_click)
        .child(
            div()
                .relative()
                .w(dp(LANDSCAPE_W))
                .h(dp(LANDSCAPE_H))
                .rounded_xl()
                .overflow_hidden()
                .bg(cx.theme().muted)
                .hover(|style| style.opacity(0.8))
                .child(image)
                .when(selected, |el| el.child(selection_ring(cx))),
        )
        .child(div().text_sm().truncate().child(name))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(timestamp(chapter.start_position_ticks.into())),
        )
}

pub const PERSON_W: f32 = 120.;
pub const PERSON_H: f32 = 180.;

/// A cast or crew member, `scale` times the base size.
pub fn person_card(
    id: impl Into<ElementId>,
    person: &BaseItemPerson,
    scale: f32,
    selected: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &mut App,
) -> Stateful<Div> {
    let (width, height) = (PERSON_W * scale, PERSON_H * scale);
    let photo = person
        .primary_image_tag
        .clone()
        .map(|tag| ImageRequest::new(&person.id, ImageType::Primary, Some(tag)).fill_width((width * 2.).ceil() as u32));
    let has_photo = photo.is_some();
    let photo = rounded_image(photo, None, cx);
    let initials: String = person
        .name
        .split_whitespace()
        .filter_map(|word| word.chars().next())
        .take(2)
        .collect();
    let detail = match (&person.role, person.kind.as_deref()) {
        (Some(role), _) if !role.is_empty() => role.clone(),
        (_, Some("Actor")) | (_, None) => String::new(),
        (_, Some(kind)) => kind_label(kind),
    };
    v_flex()
        .id(id.into())
        .w(dp(width))
        .flex_none()
        .gap_1()
        .on_click(on_click)
        .child(
            div()
                .relative()
                .w(dp(width))
                .h(dp(height))
                .rounded_xl()
                .overflow_hidden()
                .bg(cx.theme().muted)
                .hover(|style| style.opacity(0.8))
                .flex()
                .items_center()
                .justify_center()
                .map(|el| {
                    if has_photo {
                        el.child(photo)
                    } else {
                        el.text_2xl().text_color(cx.theme().muted_foreground).child(initials)
                    }
                })
                .when(selected, |el| el.child(selection_ring(cx))),
        )
        .child(div().text_sm().truncate().child(person.name.clone()))
        .child(
            div()
                .text_xs()
                .truncate()
                .text_color(cx.theme().muted_foreground)
                .child(detail),
        )
}

/// `GuestStar` -> `Guest star`.
fn kind_label(kind: &str) -> String {
    let mut label = String::new();
    for (i, c) in kind.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            label.push(' ');
            label.extend(c.to_lowercase());
        } else {
            label.push(c);
        }
    }
    label
}

/// Grey card used while a grid page is still loading.
pub fn placeholder_card(cx: &App) -> impl IntoElement {
    v_flex()
        .w(dp(POSTER_W))
        .flex_none()
        .gap_1()
        .child(div().w(dp(POSTER_W)).h(dp(POSTER_H)).rounded_xl().bg(cx.theme().muted))
        .child(div().h(dp(CARD_TEXT_H - 4.)))
}

/// A titled, horizontally scrolling row of cards, for the item pages: more room under the
/// title than [`shelf_with_rail`]'s default. The cards are the direct children of `scroll`,
/// so [`crate::browse::reveal`] can scroll to them; `rail` adds listeners or styles to their
/// row.
pub fn shelf(
    id: impl Into<ElementId>,
    title: impl Into<SharedString>,
    cards: Vec<AnyElement>,
    scroll: &ScrollHandle,
    rail: impl FnOnce(Stateful<Div>) -> Stateful<Div>,
    cx: &App,
) -> impl IntoElement {
    shelf_with_rail(id, title, cards, CARD_GAP, 16., scroll, rail, cx)
}

/// A titled, horizontally scrolling row of cards, `gap` apart and `title_gap` below the title.
#[allow(clippy::too_many_arguments)]
pub fn shelf_with_rail(
    id: impl Into<ElementId>,
    title: impl Into<SharedString>,
    cards: Vec<AnyElement>,
    gap: f32,
    title_gap: f32,
    scroll: &ScrollHandle,
    rail: impl FnOnce(Stateful<Div>) -> Stateful<Div>,
    cx: &App,
) -> impl IntoElement {
    let id = id.into();
    // From the cards' bounds in the last layout: none before the first one, so the arrows
    // show from the next frame. Child bounds don't include the scroll offset.
    let offset = scroll.offset().x;
    let viewport = scroll.bounds();
    let content_end = cards
        .len()
        .checked_sub(1)
        .and_then(|last| scroll.bounds_for_item(last))
        .map(|last| last.right() - viewport.left());
    // Hidden when the cards fit; greyed out at either end.
    let scrolls = content_end.is_some_and(|end| end > viewport.size.width + px(0.5));
    let at_start = offset >= px(-0.5);
    let at_end = content_end.is_none_or(|end| -offset + viewport.size.width >= end - px(0.5));
    let arrow = |name: &'static str, icon: IconName, tooltip: &'static str, disabled: bool, forward: bool| {
        let scroll = scroll.clone();
        let key = id.clone();
        Button::new((id.clone(), name))
            .ghost()
            .small()
            .icon(icon)
            .tooltip(tooltip)
            .disabled(disabled)
            .on_click(move |_, window, cx| {
                // A page: what shows between the row's side paddings.
                let page = (scroll.bounds().size.width - dp_px(SHELF_PADDING * 2., window)).max(px(0.));
                let distance = if forward { -page } else { page };
                glide_by(key.clone(), &scroll, Axis::Horizontal, distance, window, cx);
            })
    };
    v_flex()
        .gap(dp(title_gap))
        .child(
            h_flex()
                .px_6()
                .justify_between()
                // As tall as the arrows, so the row keeps its height when they hide.
                .min_h_6()
                .child(
                    div()
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(cx.theme().foreground)
                        .child(title.into()),
                )
                .when(scrolls, |el| {
                    el.child(
                        h_flex()
                            .gap_1()
                            .child(arrow("previous", IconName::ChevronLeft, "Scroll left", at_start, false))
                            .child(arrow("next", IconName::ChevronRight, "Scroll right", at_end, true)),
                    )
                }),
        )
        .child(rail(
            div()
                .id(id)
                .flex()
                .px_6()
                .pb_2()
                .gap(dp(gap))
                .overflow_x_scroll()
                .track_scroll(scroll)
                .children(cards),
        ))
}

/// The mascot in the middle of whatever is still loading.
pub fn centered_spinner() -> impl IntoElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .child(mascot(dp(96.)))
}

pub fn centered_message(message: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .text_color(cx.theme().muted_foreground)
        .child(message.into())
}

/// A page with nothing on it: the confused mascot over `message`.
pub fn empty_state(message: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap_3()
        .child(confused_mascot(dp(96.)))
        .child(div().text_color(cx.theme().muted_foreground).child(message.into()))
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

/// A menu the keyboard cursor can open as well as the mouse: a [`PopupMenu`] in a controlled
/// popover (gpui-kit's `DropdownMenu` only opens on a click). `menu` is the open menu, built
/// by the owner when it opens; `on_open_change` hears a click on the trigger.
pub fn menu_popover(
    id: impl Into<ElementId>,
    anchor: Anchor,
    menu: Option<Entity<PopupMenu>>,
    trigger: Button,
    on_open_change: impl Fn(&bool, &mut Window, &mut App) + 'static,
    cx: &App,
) -> Popover {
    Popover::new(id)
        .anchor(anchor)
        // The menu draws its own surface and dismisses itself.
        .appearance(false)
        .open(menu.is_some())
        .when_some(menu.as_ref(), |popover, menu| {
            popover.track_focus(&menu.focus_handle(cx))
        })
        .on_open_change(on_open_change)
        .trigger(trigger)
        .content(move |_, _, _| match &menu {
            Some(menu) => menu.clone().into_any_element(),
            None => div().into_any_element(),
        })
}

/// Whether focus should go back to the owner after `menu` closed: it's still in the menu or
/// nowhere, rather than in something a choice opened (a dialog, another page).
pub fn menu_left_focus(menu: &Entity<PopupMenu>, window: &Window, cx: &App) -> bool {
    menu.focus_handle(cx).contains_focused(window, cx) || window.focused(cx).is_none()
}

#[cfg(test)]
mod tests {
    use super::is_placeholder_episode_name;

    #[test]
    fn jellyfins_names_for_untitled_episodes() {
        assert!(is_placeholder_episode_name("Episode 3"));
        assert!(is_placeholder_episode_name("episode 03"));
        assert!(!is_placeholder_episode_name("Episode"));
        assert!(!is_placeholder_episode_name("Episode IV"));
        assert!(!is_placeholder_episode_name("Frozen Bonds"));
        assert!(!is_placeholder_episode_name("Episode 3: The Return"));
    }
}
