//! Search across movies, shows and episodes. Searches as you type, after a short pause.

use std::time::Duration;

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jellyfin::models::{BaseItemDto, BaseItemKind, ItemField, ItemSortBy};
use jellyfin::{Client, ItemsQuery};

use crate::browse::{self, Browse, Cursor, Move, Position, Section, on_browse_actions};
use crate::components::{
    CARD_GAP, LANDSCAPE_W, POSTER_W, centered_message, centered_spinner, landscape_card, poster_card,
};
use crate::images::ImageCache;
use crate::router::Navigator;
use crate::runtime;
use crate::smooth_scroll::smooth_wheel;
use crate::zoom::{dp, dp_px};

const DEBOUNCE: Duration = Duration::from_millis(300);
const LIMIT: u32 = 60;
const PADDING: f32 = 24.;

enum Results {
    Idle,
    Searching,
    Loaded(Vec<BaseItemDto>),
    Failed(SharedString),
}

pub struct SearchPage {
    client: Client,
    nav: Navigator,
    input: Entity<InputState>,
    results: Results,
    /// What `results` are for (or will be).
    term: String,
    /// The pending search. Replacing it cancels the previous one.
    search: Option<Task<()>>,
    scroll: ScrollHandle,
    cursor: Cursor,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl SearchPage {
    pub fn new(client: Client, nav: Navigator, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search movies, shows and episodes"));
        input.update(cx, |input, cx| input.focus(window, cx));
        let _subscriptions = vec![
            cx.observe(&ImageCache::global(cx), |_, _, cx| cx.notify()),
            cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => this.schedule_search(cx),
                InputEvent::PressEnter { .. } => this.enter_results(window, cx),
                _ => {}
            }),
        ];
        Self {
            client,
            nav,
            input,
            results: Results::Idle,
            term: String::new(),
            search: None,
            scroll: ScrollHandle::new(),
            cursor: Cursor::default(),
            focus_handle: cx.focus_handle(),
            _subscriptions,
        }
    }

    fn schedule_search(&mut self, cx: &mut Context<Self>) {
        let term = self.input.read(cx).value().trim().to_string();
        // Enter also reports a change; don't search again (and drop the cursor) for that.
        if term == self.term {
            return;
        }
        self.term = term.clone();
        if term.is_empty() {
            self.search = None;
            self.results = Results::Idle;
            cx.notify();
            return;
        }
        self.results = Results::Searching;
        self.cursor.hide();
        cx.notify();

        let client = self.client.clone();
        self.search = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DEBOUNCE).await;
            let query = ItemsQuery {
                search_term: Some(term),
                include_item_types: vec![BaseItemKind::Movie, BaseItemKind::Series, BaseItemKind::Episode],
                recursive: true,
                sort_by: vec![ItemSortBy::SortName],
                fields: vec![ItemField::PrimaryImageAspectRatio],
                limit: Some(LIMIT),
                ..Default::default()
            };
            let result = runtime::spawn(async move { client.items(&query).await }).await;
            this.update(cx, |this, cx| {
                this.results = match result {
                    Ok(result) => Results::Loaded(result.items),
                    Err(err) => Results::Failed(this.nav.handle_error(&err, cx)),
                };
                cx.notify();
            })
            .ok();
        }));
    }

    /// Result groups at the current window width, empty ones left out.
    fn groups(&self, window: &Window) -> Vec<Group<'_>> {
        let Results::Loaded(items) = &self.results else {
            return Vec::new();
        };
        let available = window.viewport_size().width - dp_px(PADDING * 2., window);
        [
            ("Movies", BaseItemKind::Movie),
            ("Shows", BaseItemKind::Series),
            ("Episodes", BaseItemKind::Episode),
        ]
        .into_iter()
        .map(|(title, kind)| {
            let landscape = kind == BaseItemKind::Episode;
            let width = if landscape { LANDSCAPE_W } else { POSTER_W };
            Group {
                title,
                items: items.iter().filter(|item| item.kind == kind).collect(),
                landscape,
                columns: ((available + dp_px(CARD_GAP, window)) / dp_px(width + CARD_GAP, window))
                    .floor()
                    .max(1.) as usize,
            }
        })
        .filter(|group| !group.items.is_empty())
        .collect()
    }

    /// Up in the search box goes on into the top bar; Down from there comes back to the box.
    fn up_from_box(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key == "up"
            && !event.keystroke.modifiers.modified()
            && self.input.focus_handle(cx).is_focused(window)
        {
            cx.stop_propagation();
            self.nav.focus_top_bar(window, cx);
        }
    }

    /// Enter in the search box: move to the first result.
    fn enter_results(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.groups(window).is_empty() {
            return;
        }
        self.cursor.show_at(Position::default());
        self.focus_handle.focus(window, cx);
        self.reveal(Position::default(), window, cx);
        cx.notify();
    }

    fn render_results(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        match &self.results {
            Results::Idle => return centered_message("Type to search your libraries.", cx).into_any_element(),
            Results::Searching => return centered_spinner().into_any_element(),
            Results::Failed(message) => return centered_message(message.clone(), cx).into_any_element(),
            Results::Loaded(items) if items.is_empty() => {
                return centered_message("No results.", cx).into_any_element();
            }
            Results::Loaded(_) => {}
        }
        // Titles and rows are the scroll container's direct children, for `browse::reveal`.
        let mut children: Vec<AnyElement> = Vec::new();
        for (g, group) in self.groups(window).iter().enumerate() {
            children.push(
                div()
                    .when(g > 0, |el| el.pt_6())
                    .pb_3()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(group.title)
                    .into_any_element(),
            );
            for (r, row) in group.items.chunks(group.columns).enumerate() {
                let cards = row.iter().enumerate().map(|(c, item)| {
                    let index = r * group.columns + c;
                    let nav = self.nav.clone();
                    let target = (*item).clone();
                    let on_click =
                        move |_: &ClickEvent, window: &mut Window, cx: &mut App| nav.open(&target, window, cx);
                    let id = (SharedString::from(group.title), index);
                    let selected = self.cursor.is_selected(g, index);
                    if group.landscape {
                        landscape_card(id, item, selected, on_click, cx).into_any_element()
                    } else {
                        poster_card(id, item, selected, on_click, cx).into_any_element()
                    }
                });
                children.push(
                    h_flex()
                        .pb(dp(CARD_GAP))
                        .gap(dp(CARD_GAP))
                        .items_start()
                        .children(cards.collect::<Vec<_>>())
                        .into_any_element(),
                );
            }
        }
        v_flex()
            .id("search-results")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .on_scroll_wheel(smooth_wheel("search-results", &self.scroll))
            .px(dp(PADDING))
            .pb_8()
            .text_color(cx.theme().foreground)
            .children(children)
            .into_any_element()
    }
}

struct Group<'a> {
    title: &'static str,
    items: Vec<&'a BaseItemDto>,
    landscape: bool,
    columns: usize,
}

impl Browse for SearchPage {
    fn cursor(&mut self) -> &mut Cursor {
        &mut self.cursor
    }

    fn sections(&self, window: &Window, _cx: &App) -> Vec<Section> {
        self.groups(window)
            .iter()
            .map(|group| Section::grid(group.items.len(), group.columns))
            .collect()
    }

    fn reveal(&mut self, position: Position, window: &mut Window, _cx: &mut Context<Self>) {
        // Child index of the group's title: each earlier group has a title and its rows.
        let groups = self.groups(window);
        let title: usize = groups[..position.section]
            .iter()
            .map(|group| 1 + group.items.len().div_ceil(group.columns))
            .sum();
        let row = position.index / groups[position.section].columns;
        if row == 0 {
            browse::reveal(&self.scroll, title, px(0.));
        }
        browse::reveal(&self.scroll, title + 1 + row, px(CARD_GAP));
    }

    fn activate(&mut self, position: Position, window: &mut Window, cx: &mut Context<Self>) {
        let item = self
            .groups(window)
            .get(position.section)
            .and_then(|group| group.items.get(position.index))
            .map(|item| (*item).clone());
        if let Some(item) = item {
            self.nav.open(&item, window, cx);
        }
    }

    /// Up from the first row goes back to the search box.
    fn at_edge(&mut self, mv: Move, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(mv, Move::Up | Move::PrevSection) {
            self.cursor.hide();
            self.input.update(cx, |input, cx| input.focus(window, cx));
            cx.notify();
        }
    }
}

impl Focusable for SearchPage {
    /// The results once the cursor is in them, else the search box.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        if self.cursor.selected().is_some() {
            self.focus_handle.clone()
        } else {
            self.input.focus_handle(cx)
        }
    }
}

impl Render for SearchPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let results = self.render_results(window, cx);
        let results = on_browse_actions(div().flex_1().min_h_0().track_focus(&self.focus_handle), cx).child(results);
        v_flex()
            .size_full()
            .gap_4()
            .capture_key_down(cx.listener(Self::up_from_box))
            .child(div().px_6().pt_6().max_w(dp(640.)).child(Input::new(&self.input)))
            .child(results)
    }
}
