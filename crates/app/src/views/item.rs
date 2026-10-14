//! Detail page for a movie, series, season or episode.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{ActiveTheme, IconName, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jellyfin::{Client, Ticks};
use jellyfin::models::{BaseItemDto, BaseItemKind, ItemField, MediaStreamType};

use crate::components::{
    CARD_GAP, LANDSCAPE_H, LANDSCAPE_W, Load, card_text, centered_message, centered_spinner,
    episode_label, landscape_image, poster_card, remote_image, runtime_text,
};
use crate::images::ImageCache;
use crate::playback::{PlaybackEvent, PlaybackManager};
use crate::router::Navigator;
use crate::runtime;
use crate::session::describe_error;

const BACKDROP_H: f32 = 340.;
const DETAIL_POSTER_W: f32 = 200.;
const DETAIL_POSTER_H: f32 = 300.;

struct ItemData {
    item: BaseItemDto,
    /// Seasons of a series, or episodes of a season.
    children: Vec<BaseItemDto>,
}

pub struct ItemPage {
    client: Client,
    nav: Navigator,
    item_id: String,
    data: Load<ItemData>,
    scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl ItemPage {
    pub fn new(client: Client, nav: Navigator, item_id: String, cx: &mut Context<Self>) -> Self {
        let _subscriptions = vec![
            cx.observe(&ImageCache::global(cx), |_, _, cx| cx.notify()),
            // Resume point and watched state change after playback.
            cx.subscribe(&PlaybackManager::global(cx), |this, _, event, cx| {
                if let PlaybackEvent::Stopped = event {
                    this.reload(cx);
                }
            }),
        ];
        let mut this = Self {
            client,
            nav,
            item_id,
            data: Load::Loading,
            scroll: ScrollHandle::new(),
            _subscriptions,
        };
        this.reload(cx);
        this
    }

    /// Fetches the item again. The current content stays up until the new data arrives.
    fn reload(&mut self, cx: &mut Context<Self>) {
        let (client, item_id) = (self.client.clone(), self.item_id.clone());
        cx.spawn(async move |this, cx| {
            let result = runtime::spawn(load(client, item_id)).await;
            this.update(cx, |this, cx| {
                this.data = match result {
                    Ok(data) => Load::Loaded(data),
                    Err(err) => Load::Failed(describe_error(&err).into()),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

async fn load(client: Client, item_id: String) -> jellyfin::Result<ItemData> {
    let item = client
        .item(&item_id, &[ItemField::Overview, ItemField::Genres, ItemField::MediaSources])
        .await?;
    let children = match item.kind {
        BaseItemKind::Series => client.seasons(&item.id, &[]).await?.items,
        BaseItemKind::Season => {
            let series_id = item.series_id.as_deref().unwrap_or_default();
            client
                .episodes(series_id, Some(&item.id), &[ItemField::Overview])
                .await?
                .items
        }
        _ => Vec::new(),
    };
    Ok(ItemData { item, children })
}

/// `2024 · 1h 42m · PG-13 · ★ 7.8`
fn meta_line(item: &BaseItemDto) -> String {
    let mut parts = Vec::new();
    if item.kind == BaseItemKind::Episode {
        let label = episode_label(item);
        if !label.is_empty() {
            parts.push(label);
        }
    }
    if let Some(year) = item.production_year {
        parts.push(year.to_string());
    }
    if let Some(ticks) = item.run_time_ticks.filter(|t| t.0 > 0) {
        parts.push(runtime_text(ticks));
    }
    if let Some(rating) = &item.official_rating {
        parts.push(rating.clone());
    }
    if let Some(score) = item.community_rating {
        parts.push(format!("★ {score:.1}"));
    }
    parts.join(" · ")
}

/// Play, or Resume plus Play from beginning when there's a saved position.
fn play_buttons(item: &BaseItemDto) -> impl IntoElement {
    let resume_at = item
        .user_data
        .as_ref()
        .map(|d| d.playback_position_ticks)
        .filter(|t| t.0 > 0);
    let play = |id: &'static str, label: String, start: Option<Ticks>, item: &BaseItemDto| {
        let item = item.clone();
        Button::new(id).icon(IconName::Play).label(label).on_click(move |_, _, cx| {
            PlaybackManager::global(cx).update(cx, |playback, cx| playback.play(&item, start, cx));
        })
    };
    h_flex()
        .pt_2()
        .gap_2()
        .map(|el| match resume_at {
            Some(position) => el
                .child(play("resume", format!("Resume from {}", timestamp(position)), Some(position), item).primary())
                .child(play("play-start", "Play from beginning".into(), None, item).outline()),
            None => el.child(play("play", "Play".into(), None, item).primary()),
        })
}

/// `1:02:03`, `4:05`.
fn timestamp(ticks: Ticks) -> String {
    let secs = ticks.as_secs_f64() as u64;
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

/// `English, Japanese` style summary of a stream kind in the first media source.
fn stream_summary(item: &BaseItemDto, kind: MediaStreamType) -> Option<String> {
    let source = item.media_sources.first()?;
    let names: Vec<String> = source
        .media_streams
        .iter()
        .filter(|s| s.kind == kind)
        .map(|s| {
            s.display_title
                .clone()
                .or_else(|| s.language.clone())
                .unwrap_or_else(|| "Unknown".into())
        })
        .collect();
    (!names.is_empty()).then(|| names.join(", "))
}

impl ItemPage {
    fn render_header(&self, item: &BaseItemDto, cx: &mut Context<Self>) -> impl IntoElement {
        let backdrop = item.backdrop_image().map(|r| r.fill_width(1280));
        let has_backdrop = backdrop.is_some();
        let backdrop = remote_image(backdrop, cx);
        let poster = remote_image(
            item.primary_image()
                .map(|r| r.fill_width((DETAIL_POSTER_W * 2.) as u32)),
            cx,
        );
        let playable = matches!(item.kind, BaseItemKind::Movie | BaseItemKind::Episode | BaseItemKind::Video);
        let title = match item.kind {
            BaseItemKind::Episode | BaseItemKind::Season => {
                item.series_name.clone().unwrap_or_default()
            }
            _ => String::new(),
        };

        let info = v_flex()
            .gap_2()
            .flex_1()
            .min_w_0()
            .when(!title.is_empty(), |el| {
                el.child(div().text_color(cx.theme().muted_foreground).child(title))
            })
            .child(div().text_3xl().font_weight(FontWeight::BOLD).child(item.name.clone()))
            .child(div().text_sm().text_color(cx.theme().muted_foreground).child(meta_line(item)))
            .when(!item.genres.is_empty(), |el| {
                el.child(div().text_sm().text_color(cx.theme().muted_foreground).child(item.genres.join(", ")))
            })
            .when(playable, |el| el.child(play_buttons(item)))
            .when_some(item.overview.clone(), |el, overview| {
                el.child(div().pt_2().max_w(px(820.)).child(overview))
            });

        div()
            .relative()
            .w_full()
            .when(has_backdrop, |el| {
                el.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .h(px(BACKDROP_H))
                        .overflow_hidden()
                        .child(backdrop)
                        .child(
                            div()
                                .absolute()
                                .inset_0()
                                .bg(linear_gradient(
                                    180.,
                                    linear_color_stop(cx.theme().background.opacity(0.3), 0.),
                                    linear_color_stop(cx.theme().background, 1.),
                                )),
                        ),
                )
            })
            .child(
                h_flex()
                    .relative()
                    .items_start()
                    .gap_6()
                    .px_6()
                    .pt(px(if has_backdrop { BACKDROP_H - 160. } else { 24. }))
                    .child(
                        div()
                            .flex_none()
                            .w(px(DETAIL_POSTER_W))
                            .h(px(DETAIL_POSTER_H))
                            .rounded_md()
                            .overflow_hidden()
                            .bg(cx.theme().muted)
                            .child(poster),
                    )
                    .child(info.pt(px(if has_backdrop { 120. } else { 0. }))),
            )
    }

    fn render_versions(&self, item: &BaseItemDto, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        if item.media_sources.is_empty() {
            return None;
        }
        let versions: Vec<String> = item
            .media_sources
            .iter()
            .map(|s| s.name.clone().unwrap_or_else(|| "Default".into()))
            .collect();
        let row = |label: &'static str, value: String| {
            h_flex()
                .gap_4()
                .child(div().w(px(90.)).flex_none().text_color(cx.theme().muted_foreground).child(label))
                .child(div().flex_1().child(value))
        };
        Some(
            v_flex()
                .px_6()
                .gap_1()
                .text_sm()
                .when(versions.len() > 1, |el| el.child(row("Versions", versions.join(", "))))
                .when_some(stream_summary(item, MediaStreamType::Audio), |el, audio| {
                    el.child(row("Audio", audio))
                })
                .when_some(stream_summary(item, MediaStreamType::Subtitle), |el, subs| {
                    el.child(row("Subtitles", subs))
                }),
        )
    }

    fn render_seasons(&self, seasons: &[BaseItemDto], cx: &mut Context<Self>) -> impl IntoElement {
        let cards: Vec<AnyElement> = seasons
            .iter()
            .enumerate()
            .map(|(i, season)| {
                let nav = self.nav.clone();
                let target = season.clone();
                poster_card(("season", i), season, move |_, window, cx| nav.open(&target, window, cx), cx)
                    .into_any_element()
            })
            .collect();
        v_flex()
            .px_6()
            .gap_3()
            .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child("Seasons"))
            .child(h_flex().flex_wrap().gap(px(CARD_GAP)).children(cards))
    }

    fn render_episodes(&self, episodes: &[BaseItemDto], cx: &mut Context<Self>) -> impl IntoElement {
        let rows: Vec<AnyElement> = episodes
            .iter()
            .enumerate()
            .map(|(i, episode)| {
                let nav = self.nav.clone();
                let target = episode.clone();
                let image = remote_image(
                    landscape_image(episode).map(|r| r.fill_width((LANDSCAPE_W * 2.) as u32)),
                    cx,
                );
                let (_, subtitle) = card_text(episode);
                let runtime = episode.run_time_ticks.map(runtime_text).unwrap_or_default();
                let played = episode.user_data.as_ref().is_some_and(|d| d.played);
                h_flex()
                    .id(("episode", i))
                    .items_start()
                    .gap_4()
                    .p_2()
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|style| style.bg(cx.theme().muted))
                    .on_click(move |_, window, cx| nav.open(&target, window, cx))
                    .child(
                        div()
                            .flex_none()
                            .w(px(LANDSCAPE_W))
                            .h(px(LANDSCAPE_H))
                            .rounded_md()
                            .overflow_hidden()
                            .bg(cx.theme().muted)
                            .child(image),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(div().font_weight(FontWeight::SEMIBOLD).child(subtitle))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(if played { format!("{runtime} · Watched") } else { runtime }),
                            )
                            .when_some(episode.overview.clone(), |el, overview| {
                                el.child(div().text_sm().line_clamp(3).child(overview))
                            }),
                    )
                    .into_any_element()
            })
            .collect();
        v_flex()
            .px_4()
            .gap_1()
            .child(div().px_2().text_lg().font_weight(FontWeight::SEMIBOLD).child("Episodes"))
            .children(rows)
    }
}

impl Render for ItemPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let data = match &self.data {
            Load::Loading => return centered_spinner().into_any_element(),
            Load::Failed(message) => return centered_message(message.clone(), cx).into_any_element(),
            Load::Loaded(data) => data,
        };
        let item = &data.item;

        div()
            .id("item")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .child(
                v_flex()
                    .pb_8()
                    .gap_6()
                    .child(self.render_header(item, cx))
                    .children(self.render_versions(item, cx))
                    .when(item.kind == BaseItemKind::Series && !data.children.is_empty(), |el| {
                        el.child(self.render_seasons(&data.children, cx))
                    })
                    .when(item.kind == BaseItemKind::Season && !data.children.is_empty(), |el| {
                        el.child(self.render_episodes(&data.children, cx))
                    }),
            )
            .into_any_element()
    }
}
