//! What a text field holds, for the soft keyboard, and the keyboard over a field it just opened
//! over.
//!
//! A field records its [`Kind`] each frame after it's added, and asks to be revealed when the
//! keyboard has opened over it: [`kind`] and [`reveal`], which `widgets::field::shown` calls for
//! the field helpers. On Android the focused field's kind picks the keyboard
//! (`android::after_frame`) and a field the keyboard has just opened over scrolls itself into
//! view; on the desktop the kind is only recorded, and nothing reads it.

use egui::{Id, Response};

/// What a field holds, which Android's soft keyboard suits itself to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    /// Anything: the keyboard as it comes.
    #[default]
    Text,
    /// A server address or another URL: the keyboard's URL layout, without suggestions.
    Url,
    /// A name, such as a user name: no suggestions, and no personalised learning.
    Name,
    /// A search, whose action key says Search rather than Done.
    Search,
}

/// Where a field's kind is kept: its own id, so a field reads its own kind and not the one the
/// last field held before it.
fn key(id: Id) -> Id {
    id.with("keyboard_kind")
}

/// Records what the focused field holds, so the frame's kind is known after it was drawn. Only the
/// focused field's kind is recorded: the keyboard follows the field the user is typing in.
pub fn kind(response: &Response, kind: Kind) {
    if response.ctx.memory(|memory| memory.focused()) == Some(response.id) {
        response.ctx.data_mut(|data| data.insert_temp(key(response.id), kind));
    }
}

/// What the frame's focused field holds, or [`Kind::Text`] when it didn't say (a password field
/// says nothing more than egui's own IME purpose already does).
#[cfg(target_os = "android")]
pub fn focused(ctx: &egui::Context) -> Kind {
    ctx.memory(|memory| memory.focused())
        .and_then(|id| ctx.data(|data| data.get_temp(key(id))))
        .unwrap_or_default()
}

/// Scrolls a field that the soft keyboard has just opened or grown over into view, so the user can
/// see what they type. Nothing happens on the desktop.
pub fn reveal(response: &Response) {
    #[cfg(target_os = "android")]
    reveal_above_keyboard(response);
    #[cfg(not(target_os = "android"))]
    let _ = response;
}

/// Only on the frame the keyboard's inset grew (`android::raw_input` notices it) and only when the
/// field is now outside the area left above the keyboard: the keyboard shrinks egui's safe area,
/// little else moves the form, and the user's own scrolling is left alone. egui takes the scroll
/// request as the scroll area it's in is laid out, so it has to come from the field itself.
#[cfg(target_os = "android")]
fn reveal_above_keyboard(response: &Response) {
    if !crate::android::keyboard_grew() {
        return;
    }
    let focused = response.ctx.memory(|memory| memory.focused()) == Some(response.id);
    if !focused || response.ctx.content_rect().contains_rect(response.rect) {
        return;
    }
    response.scroll_to_me(None);
}
