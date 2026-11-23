//! The playlist dialogs (`views/dialogs.rs`) over a page: *Add to playlist*'s rows, and the
//! buttons every dialog shares (`widgets/dialog.rs`), which leave the keyboard to the dialog.

use std::time::Duration;

use egui::accesskit::{Role, Toggled};
use egui_kittest::kittest::{NodeT as _, Queryable};
use jellyfin::models::BaseItemDto;

use crate::nav::NavRequest;
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
    fn title(&self) -> String {
        "Dialog".to_owned()
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
    server_with_playlists_taking(Duration::ZERO)
}

/// [`server_with_playlists`], taking `delay` to answer an addition.
fn server_with_playlists_taking(delay: Duration) -> FixtureServer {
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
    server.status_after("POST", "/Playlists/p2/Items", 204, delay);
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
    // The duplicate check read the playlist, then the movie went into that one, not the other.
    assert_eq!(server.requests("GET", "/Playlists/p2/Items"), 1);
    assert_eq!(server.requests("POST", "/Playlists/p2/Items"), 1);
    assert_eq!(server.requests("POST", "/Playlists/p1/Items"), 0);
    assert_eq!(server.queries("POST", "/Playlists/p2/Items"), ["ids=m1"]);
}

/// Where a click on the dimmed page lands: outside the dialog, on its backdrop.
const BACKDROP: egui::Pos2 = egui::pos2(5., 5.);

/// While the addition is out, Esc and a click on the backdrop leave the dialog open, since
/// closing it would drop the request and its toast; once the server answers it closes, having
/// sent the addition once.
#[test]
fn an_addition_in_flight_keeps_the_dialog_open() {
    let _config = ConfigDir::new("dialogs-add-busy");
    let server = server_with_playlists_taking(Duration::from_millis(600));
    let mut ui = add_to_playlist(&server);
    ui.step_until("the playlists", |ui| ui.query_by_label("Rainy days, 1 item").is_some());
    let row = ui.harness().get_by_label("Rainy days, 1 item").rect().center();
    ui.click_at(row);

    ui.harness_mut().key_press(egui::Key::Escape);
    ui.step();
    ui.step();
    ui.click_at(BACKDROP);
    assert!(
        ui.harness().query_by_label("Add to playlist").is_some(),
        "closed while busy"
    );

    ui.step_until("the dialog to close", |ui| {
        ui.query_by_label("Add to playlist").is_none()
    });
    assert_eq!(server.requests("POST", "/Playlists/p2/Items"), 1);
    assert!(
        ui.toasts().iter().any(|toast| toast.contains("Added to Rainy days.")),
        "no toast: {:?}",
        ui.toasts()
    );
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
    assert_eq!(server.requests("DELETE", "/Items/p1"), 0, "Cancel deleted it");
}

fn delete_confirmation(server: &FixtureServer) -> PageHarness {
    let playlist = item("p1", "Road trip", "Playlist", Some(3));
    PageHarness::new(session(server), move |_| {
        Box::new(Host {
            dialog: Some(Dialog::Delete(DeletePlaylist::new(&playlist))),
        })
    })
}

/// Esc closes a confirmation nothing was asked of, which is what makes it count that it doesn't
/// close one whose delete is out.
#[test]
fn escape_closes_an_idle_confirmation() {
    let _config = ConfigDir::new("dialogs-delete-escape");
    let server = FixtureServer::start();
    let mut ui = delete_confirmation(&server);
    ui.step();
    ui.harness_mut().key_press(egui::Key::Escape);
    ui.step();
    ui.step();
    assert!(ui.harness().query_by_label("Delete “Road trip”?").is_none());
    assert_eq!(server.requests("DELETE", "/Items/p1"), 0);
}

/// Delete, then Esc, Cancel and a click on the backdrop while the request is out: the dialog
/// stays, so the delete's result is still reported. When the reply comes the playlist's page is
/// left (it's gone) and a toast says so, with the delete sent once.
#[test]
fn a_delete_in_flight_cannot_be_dismissed() {
    let _config = ConfigDir::new("dialogs-delete-busy");
    let server = FixtureServer::start();
    server.status_after("DELETE", "/Items/p1", 204, Duration::from_millis(600));
    let mut ui = delete_confirmation(&server);
    ui.step();

    ui.harness().get_by_label("Delete").click();
    ui.step();
    ui.step();
    ui.harness_mut().key_press(egui::Key::Escape);
    ui.step();
    ui.step();
    ui.harness().get_by_label("Cancel").click();
    ui.step();
    ui.step();
    ui.click_at(BACKDROP);
    assert!(
        ui.harness().query_by_label("Delete “Road trip”?").is_some(),
        "the confirmation closed with its request out"
    );

    ui.step_until("the confirmation to close", |ui| {
        ui.query_by_label("Delete “Road trip”?").is_none()
    });
    assert_eq!(server.requests("DELETE", "/Items/p1"), 1);
    assert!(ui.state().nav.iter().any(|request| matches!(request, NavRequest::Back)));
    assert!(
        ui.toasts().iter().any(|toast| toast.contains("Deleted Road trip.")),
        "no toast: {:?}",
        ui.toasts()
    );
    assert_eq!(ui.state().services.as_ref().expect("the services").user_data, 1);
}

/// The same for a save: Esc leaves the edit dialog open until the server has answered, and the
/// save goes once, with only what changed.
#[test]
fn a_save_in_flight_keeps_the_edit_dialog_open() {
    let _config = ConfigDir::new("dialogs-edit-busy");
    let server = FixtureServer::start();
    server.json_value("GET", "/Playlists/p1", &serde_json::json!({ "OpenAccess": false }));
    server.status_after("POST", "/Playlists/p1", 204, Duration::from_millis(600));
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
    ui.harness().get_by_label(sharing).click();
    ui.step();
    ui.step();
    ui.harness().get_by_label("Save").click();
    ui.step();
    ui.step();

    ui.harness_mut().key_press(egui::Key::Escape);
    ui.step();
    ui.step();
    ui.click_at(BACKDROP);
    assert!(
        ui.harness().query_by_label("Edit playlist").is_some(),
        "the dialog closed with its save out"
    );

    ui.step_until("the dialog to close", |ui| ui.query_by_label("Edit playlist").is_none());
    assert_eq!(
        server.bodies("POST", "/Playlists/p1"),
        [serde_json::json!({ "IsPublic": true })]
    );
    assert_eq!(ui.state().services.as_ref().expect("the services").user_data, 1);
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
