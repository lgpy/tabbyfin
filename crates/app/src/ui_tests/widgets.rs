//! Shared widgets drawn on their own, outside any page: what they name to AccessKit and how
//! the keyboard reaches them.

use egui::accesskit::Role;
use egui_kittest::kittest::Queryable;

use crate::ui_tests::build_harness;
use crate::widgets::button::{self, Focus, State};
use crate::widgets::surface::{self, Kind};

/// Each kind of notice shows its message as it was given and names its icon by the kind, so a
/// screen reader hears "Error" before the message rather than a colour it can't see.
#[test]
fn notices_name_their_kind_and_message() {
    let notices = [
        (Kind::Danger, "Wrong username or password."),
        (Kind::Info, "This server is starting up."),
        (Kind::Success, "Account removed."),
    ];
    let ui = build_harness(egui::vec2(400., 300.), (), move |ui, _| {
        for (kind, message) in notices {
            surface::notice(ui, kind, message);
        }
    });
    for (kind, message) in notices {
        assert!(ui.query_by_label(message).is_some(), "{message:?} isn't shown");
        assert!(
            ui.query_by_role_and_label(Role::Image, kind.label()).is_some(),
            "no {kind:?} icon"
        );
    }
}

/// The field, a button that skips focus, and one that takes it.
fn focus_harness() -> egui_kittest::Harness<'static, String> {
    build_harness(egui::vec2(400., 300.), String::new(), |ui, text| {
        let field = ui.text_edit_singleline(text);
        if ui.memory(|memory| memory.focused()).is_none() {
            field.request_focus();
        }
        button::brand(ui, "Skipped", 200., State::Ready, Focus::Skip);
        button::quiet(ui, "Reached", 200., State::Ready, Focus::Tab);
    })
}

/// Tab from a field passes over a cozy button made with `Focus::Skip` and lands on the next one
/// that takes focus, as a dialog needs (`widgets/dialog.rs`).
#[test]
fn tab_skips_a_cozy_button_that_stays_out_of_focus() {
    let mut ui = focus_harness();
    ui.run();
    assert!(
        ui.get_by_role(Role::TextInput).is_focused(),
        "the field doesn't start with the keyboard"
    );

    ui.key_press(egui::Key::Tab);
    ui.run();
    assert!(
        !ui.get_by_label("Skipped").is_focused(),
        "Tab stopped on the skipped button"
    );
    assert!(
        ui.get_by_label("Reached").is_focused(),
        "Tab didn't reach the next button"
    );
}

/// A skipped button still takes a click.
#[test]
fn a_cozy_button_that_skips_focus_still_clicks() {
    let clicked = std::rc::Rc::new(std::cell::Cell::new(false));
    let seen = clicked.clone();
    let mut ui = build_harness(egui::vec2(400., 200.), (), move |ui, _| {
        if button::brand(ui, "Skipped", 200., State::Ready, Focus::Skip).clicked() {
            seen.set(true);
        }
    });
    ui.run();
    ui.get_by_label("Skipped").click();
    ui.run();
    assert!(clicked.get(), "the click didn't register");
}
