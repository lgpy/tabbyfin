//! The "Add to playlist" dialog. Typing filters the user's playlists; Enter (or a click) adds
//! the item to the highlighted one, or makes a new playlist with the typed name.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, Icon, Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jellyfin::models::{BaseItemDto, BaseItemKind, ItemSortBy};
use jellyfin::{Client, ItemsQuery};

use crate::changes::Changes;
use crate::components::Load;
use crate::router::Navigator;
use crate::runtime;
use crate::smooth_scroll::smooth_wheel;
use crate::zoom::{dp, dp_px};

const LIST_MAX_H: f32 = 320.;

pub fn open(client: Client, nav: Navigator, item: &BaseItemDto, window: &mut Window, cx: &mut App) {
    let picker = cx.new(|cx| AddToPlaylist::new(client, nav, item, window, cx));
    let input = picker.read(cx).input.clone();
    window.open_dialog(cx, move |dialog, window, _| {
        // Enter is the dialog's Confirm, which the text field never sees: it picks the
        // highlighted line, and the dialog closes once that's done.
        let chosen = picker.clone();
        dialog
            .title("Add to playlist")
            .w(dp_px(420., window))
            .on_ok(move |_, window, cx| {
                chosen.update(cx, |picker, cx| picker.choose(picker.highlighted, window, cx));
                false
            })
            .child(picker.clone())
    });
    input.update(cx, |input, cx| input.focus(window, cx));
}

struct AddToPlaylist {
    client: Client,
    nav: Navigator,
    item_id: String,
    item_name: String,
    /// A movie or episode is looked for in the playlist first, since 12.2 would add it a
    /// second time. (A series or season adds its episodes; the server sorts those out.)
    check_duplicate: bool,
    playlists: Load<Vec<BaseItemDto>>,
    input: Entity<InputState>,
    highlighted: usize,
    busy: bool,
    scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

/// A line of the list: an existing playlist, or a new one named by what's typed.
enum Row<'a> {
    Existing(&'a BaseItemDto),
    New(String),
}

enum Outcome {
    Added,
    Created,
    AlreadyThere,
}

impl AddToPlaylist {
    fn new(client: Client, nav: Navigator, item: &BaseItemDto, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Find a playlist or name a new one"));
        let _subscriptions = vec![cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
            if let InputEvent::Change = event {
                this.highlighted = 0;
                cx.notify();
            }
        })];
        let mut this = Self {
            client,
            nav,
            item_id: item.id.clone(),
            item_name: item.name.clone(),
            check_duplicate: matches!(
                item.kind,
                BaseItemKind::Movie | BaseItemKind::Episode | BaseItemKind::Video
            ),
            playlists: Load::Loading,
            input,
            highlighted: 0,
            busy: false,
            scroll: ScrollHandle::new(),
            _subscriptions,
        };
        this.load(cx);
        this
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let client = self.client.clone();
        let query = ItemsQuery {
            include_item_types: vec![BaseItemKind::Playlist],
            recursive: true,
            sort_by: vec![ItemSortBy::SortName],
            ..Default::default()
        };
        cx.spawn(async move |this, cx| {
            let result = runtime::spawn(async move { client.items(&query).await }).await;
            this.update(cx, |this, cx| {
                this.playlists = match result {
                    Ok(result) => Load::Loaded(result.items),
                    Err(err) => Load::Failed(this.nav.handle_error(&err, cx)),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn rows(&self, cx: &App) -> Vec<Row<'_>> {
        let Load::Loaded(playlists) = &self.playlists else {
            return Vec::new();
        };
        let typed = self.input.read(cx).value().trim().to_owned();
        let matches = |p: &BaseItemDto| p.name.to_lowercase().contains(&typed.to_lowercase());
        let mut rows: Vec<Row> = playlists.iter().filter(|p| matches(p)).map(Row::Existing).collect();
        if !typed.is_empty() && !playlists.iter().any(|p| p.name.eq_ignore_ascii_case(&typed)) {
            rows.push(Row::New(typed));
        }
        rows
    }

    /// Up and Down move the highlight while the text field has focus.
    fn on_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let count = self.rows(cx).len();
        if count == 0 || event.keystroke.modifiers.modified() {
            return;
        }
        match event.keystroke.key.as_str() {
            "down" => self.highlighted = (self.highlighted + 1).min(count - 1),
            "up" => self.highlighted = self.highlighted.saturating_sub(1),
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn choose(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let (playlist_id, name) = match self.rows(cx).get(index) {
            Some(Row::Existing(playlist)) => (Some(playlist.id.clone()), playlist.name.clone()),
            Some(Row::New(name)) => (None, name.clone()),
            None => return,
        };
        self.busy = true;
        cx.notify();
        let (client, item_id, check_duplicate) = (self.client.clone(), self.item_id.clone(), self.check_duplicate);
        let new_name = name.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = runtime::spawn(async move {
                let ids = [item_id.clone()];
                let Some(playlist_id) = playlist_id else {
                    client.create_playlist(&new_name, &ids).await?;
                    return Ok(Outcome::Created);
                };
                if check_duplicate {
                    let entries = client.playlist_items(&playlist_id, &[]).await?;
                    if entries.items.iter().any(|entry| entry.id == item_id) {
                        return Ok(Outcome::AlreadyThere);
                    }
                }
                client.add_to_playlist(&playlist_id, &ids).await?;
                Ok(Outcome::Added)
            })
            .await;
            this.update_in(cx, |this, window, cx| {
                this.busy = false;
                cx.notify();
                let note = match result {
                    Ok(Outcome::Added) => Notification::success(format!("Added to {name}.")),
                    Ok(Outcome::Created) => Notification::success(format!("Made the playlist {name}.")),
                    Ok(Outcome::AlreadyThere) => {
                        Notification::info(format!("{} is already in {name}.", this.item_name))
                    }
                    // Someone else's playlist, shared for viewing only.
                    Err(jellyfin::Error::Forbidden) => {
                        let message = format!("You can't change {name}; it belongs to someone else.");
                        return window.push_notification(Notification::error(message), cx);
                    }
                    Err(err) => {
                        let message = this.nav.handle_error(&err, cx);
                        return window.push_notification(Notification::error(message), cx);
                    }
                };
                window.close_dialog(cx);
                window.push_notification(note, cx);
                // Open playlist pages reload with the new entry.
                Changes::notify_user_data(cx);
            })
            .ok();
        })
        .detach();
    }

    fn render_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let message = |text: SharedString| {
            div()
                .py_2()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(text)
                .into_any_element()
        };
        match &self.playlists {
            Load::Loading => return message("Loading playlists…".into()),
            Load::Failed(error) => return message(error.clone()),
            Load::Loaded(_) => {}
        }
        let rows = self.rows(cx);
        if rows.is_empty() {
            let typed = !self.input.read(cx).value().trim().is_empty();
            return message(if typed {
                "No playlist has that name.".into()
            } else {
                "No playlists yet. Type a name to make one.".into()
            });
        }
        let lines: Vec<AnyElement> = rows
            .into_iter()
            .enumerate()
            .map(|(i, row)| {
                let (icon, label, detail) = match row {
                    Row::Existing(playlist) => (
                        Lucide::ListVideo,
                        SharedString::from(playlist.name.clone()),
                        playlist.child_count.map(entry_count),
                    ),
                    Row::New(name) => (Lucide::Plus, format!("New playlist “{name}”").into(), None),
                };
                h_flex()
                    .id(("playlist", i))
                    .gap_2()
                    .px_2()
                    .py_1p5()
                    .rounded_lg()
                    .when(i == self.highlighted, |el| el.bg(cx.theme().accent))
                    .hover(|style| style.bg(cx.theme().muted))
                    .on_click(cx.listener(move |this, _, window, cx| this.choose(i, window, cx)))
                    .child(Icon::new(icon).small().text_color(cx.theme().muted_foreground))
                    .child(div().flex_1().min_w_0().truncate().child(label))
                    .children(
                        detail.map(|detail| div().text_sm().text_color(cx.theme().muted_foreground).child(detail)),
                    )
                    .into_any_element()
            })
            .collect();
        v_flex()
            .id("playlists")
            .max_h(dp(LIST_MAX_H))
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .on_scroll_wheel(smooth_wheel("playlists", &self.scroll))
            .children(lines)
            .into_any_element()
    }
}

fn entry_count(count: u32) -> String {
    match count {
        1 => "1 item".into(),
        n => format!("{n} items"),
    }
}

impl Render for AddToPlaylist {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_3()
            .capture_key_down(cx.listener(Self::on_key_down))
            .child(Input::new(&self.input).disabled(self.busy))
            .child(self.render_list(cx))
    }
}
