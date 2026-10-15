//! The library's genres, tags and ratings dialog: tick any number of each. Items with any of
//! the ticked genres show, and likewise for tags and ratings; different kinds must all match.
//! Each tick reloads the grid behind the dialog. Up and Down move, Space ticks, Enter or Esc
//! closes.

use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::{ActiveTheme, WindowExt, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::smooth_scroll::smooth_wheel;
use crate::views::library::{LibraryPage, ValueKind};
use crate::zoom::{dp, dp_px};

const LIST_MAX_H: f32 = 420.;

pub fn open(page: Entity<LibraryPage>, window: &mut Window, cx: &mut App) {
    let dialog = cx.new(|cx| ValueFilters {
        page,
        highlighted: 0,
        focus_handle: cx.focus_handle(),
        scroll: ScrollHandle::new(),
    });
    let focus = dialog.read(cx).focus_handle.clone();
    window.open_dialog(cx, move |modal, window, _| {
        modal
            .title("Genres, tags and ratings")
            .w(dp_px(380., window))
            .on_ok(|_, _, _| true)
            .child(dialog.clone())
    });
    focus.focus(window, cx);
}

struct ValueFilters {
    page: Entity<LibraryPage>,
    /// Index into `entries`.
    highlighted: usize,
    focus_handle: FocusHandle,
    scroll: ScrollHandle,
}

/// A line of the dialog: a group's title, or a value to tick.
enum Line {
    Title(&'static str),
    Value(ValueKind, String),
}

impl ValueFilters {
    fn lines(&self, cx: &App) -> Vec<Line> {
        let values = self.page.read(cx).filter_values();
        let mut lines = Vec::new();
        for (kind, title, list) in [
            (ValueKind::Genre, "Genres", &values.genres),
            (ValueKind::Tag, "Tags", &values.tags),
            (ValueKind::Rating, "Ratings", &values.ratings),
        ] {
            if !list.is_empty() {
                lines.push(Line::Title(title));
                lines.extend(list.iter().map(|value| Line::Value(kind, value.clone())));
            }
        }
        lines
    }

    /// The values only, which the keyboard moves through.
    fn entries(&self, cx: &App) -> Vec<(ValueKind, String)> {
        self.lines(cx)
            .into_iter()
            .filter_map(|line| match line {
                Line::Value(kind, value) => Some((kind, value)),
                Line::Title(_) => None,
            })
            .collect()
    }

    fn toggle(&mut self, kind: ValueKind, value: &str, cx: &mut Context<Self>) {
        self.page.update(cx, |page, cx| page.toggle_value(kind, value, cx));
        cx.notify();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.modifiers.modified() {
            return;
        }
        let entries = self.entries(cx);
        match event.keystroke.key.as_str() {
            "down" => self.highlighted = (self.highlighted + 1).min(entries.len().saturating_sub(1)),
            "up" => self.highlighted = self.highlighted.saturating_sub(1),
            "space" => {
                if let Some((kind, value)) = entries.get(self.highlighted) {
                    self.toggle(*kind, value, cx);
                }
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }
}

impl Render for ValueFilters {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let lines = self.lines(cx);
        let page = self.page.read(cx);
        let muted = cx.theme().muted_foreground;
        let mut index = 0;
        let mut rows = Vec::new();
        for (i, line) in lines.into_iter().enumerate() {
            match line {
                Line::Title(title) => rows.push(
                    div()
                        .when(i > 0, |el| el.pt_3())
                        .pb_1()
                        .text_sm()
                        .text_color(muted)
                        .child(title)
                        .into_any_element(),
                ),
                Line::Value(kind, value) => {
                    let checked = page.chosen_values(kind).contains(&value);
                    let highlighted = index == self.highlighted;
                    let (dialog, toggled) = (cx.entity(), value.clone());
                    rows.push(
                        div()
                            .id(("value", index))
                            .px_2()
                            .py_1()
                            .rounded_lg()
                            .when(highlighted, |el| el.bg(cx.theme().accent))
                            .child(
                                Checkbox::new(("value-check", index))
                                    .checked(checked)
                                    .label(value)
                                    .tab_stop(false)
                                    .on_click(move |_, _, cx| {
                                        dialog.update(cx, |this, cx| this.toggle(kind, &toggled, cx))
                                    }),
                            )
                            .into_any_element(),
                    );
                    index += 1;
                }
            }
        }
        v_flex()
            .id("value-filters")
            .track_focus(&self.focus_handle)
            .capture_key_down(cx.listener(Self::on_key_down))
            .max_h(dp(LIST_MAX_H))
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .on_scroll_wheel(smooth_wheel("value-filters", &self.scroll))
            .children(rows)
    }
}
