//! The "Edit playlist" dialog: rename a playlist, and share it with everyone on the server
//! or keep it to yourself. Down moves from the name to the sharing switch (Space, Left or Right
//! flip it), Up goes back, and Enter saves.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme, Disableable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jellyfin::Client;

use crate::changes::Changes;
use crate::components::{Load, selection_ring};
use crate::router::Navigator;
use crate::runtime;

pub fn open(client: Client, nav: Navigator, playlist_id: String, name: String, window: &mut Window, cx: &mut App) {
    let editor = cx.new(|cx| EditPlaylist::new(client, nav, playlist_id, name, window, cx));
    let input = editor.read(cx).input.clone();
    window.open_dialog(cx, move |dialog, _, _| {
        let save = editor.clone();
        dialog
            .title("Edit playlist")
            .w(px(420.))
            // Enter is the dialog's Confirm; the dialog closes once the save went through.
            .on_ok(move |_, window, cx| {
                save.update(cx, |editor, cx| editor.save(window, cx));
                false
            })
            .child(editor.clone())
    });
    input.update(cx, |input, cx| input.focus(window, cx));
}

struct EditPlaylist {
    client: Client,
    nav: Navigator,
    playlist_id: String,
    name: String,
    input: Entity<InputState>,
    /// Shared with everyone on the server; read from the server when the dialog opens.
    public: Load<bool>,
    busy: bool,
    /// Focused while the keyboard is on the sharing switch (the input has focus otherwise).
    switch_focus: FocusHandle,
}

impl EditPlaylist {
    fn new(
        client: Client,
        nav: Navigator,
        playlist_id: String,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).default_value(name.clone()));
        let (fetch, id) = (client.clone(), playlist_id.clone());
        cx.spawn(async move |this, cx| {
            let result = runtime::spawn(async move { fetch.playlist_is_public(&id).await }).await;
            this.update(cx, |this, cx| {
                this.public = match result {
                    Ok(public) => Load::Loaded(public),
                    Err(err) => Load::Failed(describe(&err, &this.nav, cx)),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
        Self {
            client,
            nav,
            playlist_id,
            name,
            input,
            public: Load::Loading,
            busy: false,
            switch_focus: cx.focus_handle(),
        }
    }

    /// Up and Down between the name and the switch, and the keys that flip it. Captured, since
    /// the text field would take Up and Down itself.
    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.modifiers.modified() {
            return;
        }
        let on_switch = self.switch_focus.is_focused(window);
        match event.keystroke.key.as_str() {
            "down" if !on_switch && matches!(self.public, Load::Loaded(_)) => self.switch_focus.focus(window, cx),
            "up" if on_switch => self.input.update(cx, |input, cx| input.focus(window, cx)),
            "space" | "left" | "right" if on_switch => {
                if let Load::Loaded(public) = &mut self.public {
                    *public = match event.keystroke.key.as_str() {
                        "left" => false,
                        "right" => true,
                        _ => !*public,
                    };
                }
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let typed = self.input.read(cx).value().trim().to_owned();
        if typed.is_empty() {
            return;
        }
        // Only what changed; sharing stays as it is if it couldn't be read.
        let name = (typed != self.name).then_some(typed);
        let public = match self.public {
            Load::Loaded(public) => Some(public),
            _ => None,
        };
        self.busy = true;
        cx.notify();
        let (client, id) = (self.client.clone(), self.playlist_id.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result =
                runtime::spawn(async move { client.update_playlist(&id, name.as_deref(), public).await }).await;
            this.update_in(cx, |this, window, cx| {
                this.busy = false;
                cx.notify();
                match result {
                    Ok(()) => {
                        window.close_dialog(cx);
                        // The playlist's page and library grid show the new name.
                        Changes::notify_user_data(cx);
                    }
                    Err(err) => {
                        let message = describe(&err, &this.nav, cx);
                        window.push_notification(Notification::error(message), cx);
                    }
                }
            })
            .ok();
        })
        .detach();
    }
}

/// Another user's playlist answers 404 to changes (and 403 to some reads).
fn describe(err: &jellyfin::Error, nav: &Navigator, cx: &mut App) -> SharedString {
    match err {
        jellyfin::Error::NotFound | jellyfin::Error::Forbidden => {
            "This playlist belongs to someone else; it can't be changed.".into()
        }
        err => nav.handle_error(err, cx),
    }
}

impl Render for EditPlaylist {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let on_switch = self.switch_focus.is_focused(window);
        let sharing = match &self.public {
            Load::Loading => div().text_sm().text_color(muted).child("Checking who can see it…"),
            Load::Failed(message) => div().text_sm().text_color(muted).child(message.clone()),
            Load::Loaded(public) => div()
                .relative()
                .p_1()
                .m(px(-4.))
                .rounded_lg()
                .when(on_switch, |el| el.child(selection_ring(cx).rounded_lg()))
                .child(
                    Switch::new("playlist-public")
                        .checked(*public)
                        .disabled(self.busy)
                        .label("Everyone on this server can see it")
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.public = Load::Loaded(*checked);
                            cx.notify();
                        })),
                ),
        };
        v_flex()
            .gap_4()
            .track_focus(&self.switch_focus)
            .capture_key_down(cx.listener(Self::on_key_down))
            .child(
                v_flex()
                    .gap_1()
                    .child(div().text_sm().child("Name"))
                    .child(Input::new(&self.input).disabled(self.busy)),
            )
            .child(sharing)
            .child(
                h_flex().justify_end().child(
                    Button::new("save-playlist")
                        .primary()
                        .label("Save")
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                ),
            )
    }
}
