//! The playlist dialogs (`views/dialogs.rs`) over a page: *Add to playlist*'s rows, and the
//! buttons every dialog shares (`widgets/dialog.rs`), which leave the keyboard to the dialog.

use egui::accesskit::{Role, Toggled};
use egui_kittest::kittest::{NodeT as _, Queryable};
use jellyfin::models::BaseItemDto;

use crate::ui_tests::{ConfigDir, FixtureServer, PageHarness, session};
use crate::views::dialogs::{AddToPlaylist, DeletePlaylist, EditPlaylist};
use crate::views::{Page, PageCx};

/// A page that only holds a dialog, opened with the page, until the dialog is done.
struct Host {
    dialog: Option<Dialog>,
}

enum Dialog {
    Add(AddToPlaylist),
    Delete(DeletePlaylist),
    Edit(EditPlaylist),
}

impl Page for Host {
    fn title(&self) -> &str {
        "Dialog"
    }

    fn show(&mut self, _ui: &mut egui::Ui, cx: &mut PageCx) {
        let open = match &mut self.dialog {
            Some(Dialog::Add(dialog)) => dialog.show(cx),
            Some(Dialog::Delete(dialog)) => dialog.show(cx),
            Some(Dialog::Edit(dialog)) => dialog.show(cx),
            None => return,
        };
        if !open {
            self.dialog = None;
        }
    }
}

fn item(id: &str, name: &str, kind: &str, entries: Option<u32>) -> BaseItemDto {
    serde_json::from_value(serde_json::json!({
        "Id": id,
        "Name": name,
        "Type": kind,
        "ChildCount": entries,
    }))
    .expect("item")
}

/// The server with two playlists, ready to take an addition to either.
fn server_with_playlists() -> FixtureServer {
    let server = FixtureServer::start();
    server.json_value(
        "GET",
        "/Items",
        &serde_json::json!({
            "Items": [
                { "Id": "p1", "Name": "Road trip", "Type": "Playlist", "ChildCount": 3 },
                { "Id": "p2", "Name": "Rainy days", "Type": "Playlist", "ChildCount": 1 },
            ],
            "TotalRecordCount": 2,
        }),
    );
    server.json_value(
        "GET",
        "/Playlists/p2/Items",
        &serde_json::json!({ "Items": [], "TotalRecordCount": 0 }),
    );
    server.status("POST", "/Playlists/p2/Items", 204);
    server
}

fn add_to_playlist(server: &FixtureServer) -> PageHarness {
    let movie = item("m1", "Big Buck Bunny", "Movie", None);
    PageHarness::new(session(server), move |cx| {
        Box::new(Host {
            dialog: Some(Dialog::Add(AddToPlaylist::new(cx, &movie))),
        })
    })
}

/// Each playlist is one row, named with its entry count, and typing a name nobody has adds a
/// row that makes it.
#[test]
fn add_to_playlist_lists_each_playlist_as_a_row() {
    let _config = ConfigDir::new("dialogs-add-rows");
    let server = server_with_playlists();
    let mut ui = add_to_playlist(&server);
    ui.step_until("the playlists", |ui| ui.query_by_label("Road trip, 3 items").is_some());
    let harness = ui.harness();
    assert!(
        harness
            .query_by_role_and_label(Role::Button, "Road trip, 3 items")
            .is_some()
    );
    assert!(
        harness
            .query_by_role_and_label(Role::Button, "Rainy days, 1 item")
            .is_some()
    );

    ui.harness_mut().get_by_role(Role::TextInput).type_text("Movie night");
    ui.step_until("the new playlist row", |ui| {
        ui.query_by_role_and_label(Role::Button, "New playlist “Movie night”")
            .is_some()
    });
    assert!(
        ui.harness().query_by_label_contains("Road trip").is_none(),
        "the filter kept a playlist that doesn't match"
    );
}

/// A click on a row adds the item to that playlist and closes the dialog.
#[test]
fn a_click_on_a_row_adds_to_that_playlist() {
    let _config = ConfigDir::new("dialogs-add-click");
    let server = server_with_playlists();
    let mut ui = add_to_playlist(&server);
    ui.step_until("the playlists", |ui| ui.query_by_label("Rainy days, 1 item").is_some());
    let row = ui.harness().get_by_label("Rainy days, 1 item").rect().center();
    ui.click_at(row);
    // It closes only once the server took the addition.
    ui.step_until("the dialog to close", |ui| {
        ui.query_by_label("Add to playlist").is_none()
    });
}

/// The confirmation's buttons stay out of Tab's way (the dialog's keys are its own), and
/// *Cancel* closes it without deleting.
#[test]
fn delete_confirmation_buttons_take_clicks_not_focus() {
    let _config = ConfigDir::new("dialogs-delete");
    let server = FixtureServer::start();
    let playlist = item("p1", "Road trip", "Playlist", Some(3));
    let mut ui = PageHarness::new(session(&server), move |_| {
        Box::new(Host {
            dialog: Some(Dialog::Delete(DeletePlaylist::new(&playlist))),
        })
    });
    ui.step();
    assert!(ui.harness().query_by_label("Delete “Road trip”?").is_some());

    ui.harness_mut().key_press(egui::Key::Tab);
    ui.step();
    for label in ["Delete", "Cancel"] {
        assert!(!ui.harness().get_by_label(label).is_focused(), "Tab focused {label}");
    }

    ui.harness().get_by_label("Cancel").click();
    ui.step();
    ui.step();
    assert!(
        ui.harness().query_by_label("Delete “Road trip”?").is_none(),
        "Cancel didn't close the confirmation"
    );
}

/// The edit dialog's sharing is a switch showing what the server says, flipped by a click and
/// passed over by Tab, since the dialog moves its own highlight onto it (Down from the name).
#[test]
fn edit_dialog_sharing_is_a_switch_that_stays_out_of_tab() {
    let _config = ConfigDir::new("dialogs-sharing");
    let server = FixtureServer::start();
    server.json_value("GET", "/Playlists/p1", &serde_json::json!({ "OpenAccess": false }));
    let playlist = item("p1", "Road trip", "Playlist", Some(3));
    let mut ui = PageHarness::new(session(&server), move |cx| {
        Box::new(Host {
            dialog: Some(Dialog::Edit(EditPlaylist::new(cx, &playlist))),
        })
    });
    let sharing = "Everyone on this server can see it";
    ui.step_until("the sharing switch", |harness| {
        harness.query_by_role_and_label(Role::Switch, sharing).is_some()
    });
    let toggled = |ui: &PageHarness| {
        ui.harness()
            .get_by_role_and_label(Role::Switch, sharing)
            .accesskit_node()
            .toggled()
    };
    assert_eq!(toggled(&ui), Some(Toggled::False), "the server's not shared");

    ui.harness().get_by_label(sharing).click();
    // The click flips it in the frame that reads it; the next frame draws it on.
    ui.step();
    ui.step();
    assert_eq!(toggled(&ui), Some(Toggled::True), "a click shares it");

    ui.harness_mut().key_press(egui::Key::Tab);
    ui.step();
    assert!(
        !ui.harness().get_by_label(sharing).is_focused(),
        "Tab focused the switch"
    );
}
