//! Detail page for a movie, series, season or episode.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{ActiveTheme, IconName, IndexPath, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jellyfin::models::{BaseItemDto, BaseItemKind, ItemField};
use jellyfin::{Client, Ticks};

use crate::changes::{Changes, UserDataChanged};
use crate::components::{
    CARD_GAP, LANDSCAPE_H, LANDSCAPE_W, Load, card_text, centered_spinner, episode_label, error_state, landscape_image,
    poster_card, remote_image, runtime_text, timestamp,
};
use crate::images::ImageCache;
use crate::playback::{PlayOptions, PlaybackManager, SubtitleChoice};
use crate::router::Navigator;
use crate::runtime;
use crate::tracks::{Choice, ChoiceSelect, SUBTITLES_OFF, track_pickers};

const BACKDROP_H: f32 = 340.;
const DETAIL_POSTER_W: f32 = 200.;
const DETAIL_POSTER_H: f32 = 300.;
const PICKER_W: f32 = 260.;

struct ItemData {
    item: BaseItemDto,
    /// Seasons of a series, or episodes of a season.
    children: Vec<BaseItemDto>,
}

/// Version and track choices for a playable item. Defaults come from the server, which
/// applies the user's language preferences.
struct Pickers {
    version: Option<ChoiceSelect>,
    audio: Option<ChoiceSelect>,
    subtitle: Option<ChoiceSelect>,
    _version_changed: Option<Subscription>,
}

pub struct ItemPage {
    client: Client,
    nav: Navigator,
    item_id: String,
    data: Load<ItemData>,
    pickers: Option<Pickers>,
    scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl ItemPage {
    pub fn new(client: Client, nav: Navigator, item_id: String, cx: &mut Context<Self>) -> Self {
        let _subscriptions = vec![
            cx.observe(&ImageCache::global(cx), |_, _, cx| cx.notify()),
            // Resume point and watched state change after playback or marking items watched.
            cx.subscribe(&Changes::global(cx), |this, _, _: &UserDataChanged, cx| this.reload(cx)),
        ];
        let mut this = Self {
            client,
            nav,
            item_id,
            data: Load::Loading,
            pickers: None,
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
                    Err(err) => Load::Failed(this.nav.handle_error(&err, cx)),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn play(&mut self, start: Option<Ticks>, cx: &mut Context<Self>) {
        let Load::Loaded(data) = &self.data else { return };
        let mut options = PlayOptions {
            start,
            ..Default::default()
        };
        if let Some(pickers) = &self.pickers {
            let selected =
                |select: &Option<ChoiceSelect>| select.as_ref().and_then(|s| s.read(cx).selected_value().copied());
            options.media_source_id = selected(&pickers.version)
                .and_then(|i| data.item.media_sources.get(i as usize))
                .map(|source| source.id.clone());
            options.audio_stream_index = selected(&pickers.audio);
            options.subtitle = match selected(&pickers.subtitle) {
                Some(SUBTITLES_OFF) => SubtitleChoice::Off,
                Some(index) => SubtitleChoice::Stream(index),
                None => SubtitleChoice::Auto,
            };
        }
        let item = data.item.clone();
        PlaybackManager::global(cx).update(cx, |playback, cx| playback.play(&item, options, cx));
    }

    fn set_played(&mut self, item_id: String, played: bool, window: &mut Window, cx: &mut Context<Self>) {
        let client = self.client.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = runtime::spawn(async move {
                if played {
                    client.mark_played(&item_id).await
                } else {
                    client.mark_unplayed(&item_id).await
                }
            })
            .await;
            this.update_in(cx, |this, window, cx| match result {
                Ok(()) => Changes::notify_user_data(cx),
                Err(err) => {
                    let message = this.nav.handle_error(&err, cx);
                    window.push_notification(Notification::error(message), cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn ensure_pickers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pickers.is_some() {
            return;
        }
        let Load::Loaded(data) = &self.data else { return };
        if data.item.media_sources.is_empty() {
            return;
        }
        let sources = data.item.media_sources.clone();
        let version = (sources.len() > 1).then(|| {
            let choices = sources
                .iter()
                .enumerate()
                .map(|(i, s)| Choice {
                    label: s.name.clone().unwrap_or_else(|| format!("Version {}", i + 1)).into(),
                    value: i as i32,
                })
                .collect();
            cx.new(|cx| SelectState::new(choices, Some(IndexPath::default()), window, cx))
        });
        let _version_changed = version.as_ref().map(|select| {
            cx.subscribe_in(
                select,
                window,
                move |this, _, event: &SelectEvent<Vec<Choice>>, window, cx| {
                    let SelectEvent::Confirm(Some(index)) = event else {
                        return;
                    };
                    if let Some(source) = sources.get(*index as usize) {
                        let (audio, subtitle) = track_pickers(
                            source,
                            source.default_audio_stream_index,
                            source.default_subtitle_stream_index,
                            window,
                            cx,
                        );
                        if let Some(pickers) = &mut this.pickers {
                            pickers.audio = audio;
                            pickers.subtitle = subtitle;
                        }
                        cx.notify();
                    }
                },
            )
        });
        let first = &data.item.media_sources[0];
        let (audio, subtitle) = track_pickers(
            first,
            first.default_audio_stream_index,
            first.default_subtitle_stream_index,
            window,
            cx,
        );
        self.pickers = Some(Pickers {
            version,
            audio,
            subtitle,
            _version_changed,
        });
    }
}

async fn load(client: Client, item_id: String) -> jellyfin::Result<ItemData> {
    let item = client
        .item(
            &item_id,
            &[ItemField::Overview, ItemField::Genres, ItemField::MediaSources],
        )
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

fn is_played(item: &BaseItemDto) -> bool {
    item.user_data.as_ref().is_some_and(|d| d.played)
}

impl ItemPage {
    /// Play / Resume buttons and the watched toggle.
    fn render_actions(&self, item: &BaseItemDto, cx: &mut Context<Self>) -> impl IntoElement {
        let playable = matches!(
            item.kind,
            BaseItemKind::Movie | BaseItemKind::Episode | BaseItemKind::Video
        );
        let resume_at = item
            .user_data
            .as_ref()
            .map(|d| d.playback_position_ticks)
            .filter(|t| t.0 > 0);
        let played = is_played(item);
        let item_id = item.id.clone();

        h_flex()
            .pt_2()
            .gap_2()
            .when(playable, |el| match resume_at {
                Some(position) => el
                    .child(
                        Button::new("resume")
                            .primary()
                            .icon(IconName::Play)
                            .label(format!("Resume from {}", timestamp(position.into())))
                            .on_click(cx.listener(move |this, _, _, cx| this.play(Some(position), cx))),
                    )
                    .child(
                        Button::new("play-start")
                            .outline()
                            .label("Play from beginning")
                            .on_click(cx.listener(|this, _, _, cx| this.play(None, cx))),
                    ),
                None => el.child(
                    Button::new("play")
                        .primary()
                        .icon(IconName::Play)
                        .label("Play")
                        .on_click(cx.listener(|this, _, _, cx| this.play(None, cx))),
                ),
            })
            .child(
                Button::new("toggle-played")
                    .outline()
                    .icon(if played { IconName::CircleCheck } else { IconName::Check })
                    .label(if played { "Watched" } else { "Mark watched" })
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.set_played(item_id.clone(), !played, window, cx)),
                    ),
            )
    }

    fn render_header(&self, item: &BaseItemDto, cx: &mut Context<Self>) -> impl IntoElement {
        let backdrop = item.backdrop_image().map(|r| r.fill_width(1280));
        let has_backdrop = backdrop.is_some();
        let backdrop = remote_image(backdrop, cx);
        let poster = remote_image(
            item.primary_image()
                .map(|r| r.fill_width((DETAIL_POSTER_W * 2.) as u32)),
            cx,
        );
        let title = match item.kind {
            BaseItemKind::Episode | BaseItemKind::Season => item.series_name.clone().unwrap_or_default(),
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
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(meta_line(item)),
            )
            .when(!item.genres.is_empty(), |el| {
                el.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(item.genres.join(", ")),
                )
            })
            .child(self.render_actions(item, cx))
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
                        .child(div().absolute().inset_0().bg(linear_gradient(
                            180.,
                            linear_color_stop(cx.theme().background.opacity(0.3), 0.),
                            linear_color_stop(cx.theme().background, 1.),
                        ))),
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

    fn render_pickers(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let pickers = self.pickers.as_ref()?;
        let row = |label: &'static str, select: &ChoiceSelect| {
            h_flex()
                .gap_4()
                .child(
                    div()
                        .w(px(90.))
                        .flex_none()
                        .text_color(cx.theme().muted_foreground)
                        .child(label),
                )
                .child(Select::new(select).w(px(PICKER_W)).menu_width(px(PICKER_W * 1.5)))
        };
        Some(
            v_flex()
                .px_6()
                .gap_2()
                .text_sm()
                .when_some(pickers.version.as_ref(), |el, s| el.child(row("Version", s)))
                .when_some(pickers.audio.as_ref(), |el, s| el.child(row("Audio", s)))
                .when_some(pickers.subtitle.as_ref(), |el, s| el.child(row("Subtitles", s))),
        )
    }

    fn render_seasons(&self, seasons: &[BaseItemDto], cx: &mut Context<Self>) -> impl IntoElement {
        let cards: Vec<AnyElement> = seasons
            .iter()
            .enumerate()
            .map(|(i, season)| {
                let nav = self.nav.clone();
                let target = season.clone();
                poster_card(
                    ("season", i),
                    season,
                    move |_, window, cx| nav.open(&target, window, cx),
                    cx,
                )
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
                let played = is_played(episode);
                let episode_id = episode.id.clone();
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
                            .child(div().text_sm().text_color(cx.theme().muted_foreground).child(runtime))
                            .when_some(episode.overview.clone(), |el, overview| {
                                el.child(div().text_sm().line_clamp(3).child(overview))
                            }),
                    )
                    .child(
                        Button::new(("toggle-played", i))
                            .ghost()
                            .icon(if played { IconName::CircleCheck } else { IconName::Check })
                            .tooltip(if played { "Mark unwatched" } else { "Mark watched" })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                // Don't open the episode underneath.
                                cx.stop_propagation();
                                this.set_played(episode_id.clone(), !played, window, cx);
                            })),
                    )
                    .into_any_element()
            })
            .collect();
        v_flex()
            .px_4()
            .gap_1()
            .child(
                div()
                    .px_2()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Episodes"),
            )
            .children(rows)
    }
}

impl Render for ItemPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_pickers(window, cx);
        let data = match &self.data {
            Load::Loading => return centered_spinner().into_any_element(),
            Load::Failed(message) => {
                let retry = cx.listener(|this, _, _, cx| {
                    this.data = Load::Loading;
                    this.reload(cx);
                });
                return error_state(message.clone(), retry, cx).into_any_element();
            }
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
                    .children(self.render_pickers(cx))
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
