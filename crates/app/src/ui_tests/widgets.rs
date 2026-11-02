//! Shared widgets drawn on their own, outside any page: what they name to AccessKit and how
//! the keyboard reaches them.

use egui::accesskit::{Role, Toggled};
use egui_kittest::kittest::{NodeT as _, Queryable};
use egui_material_icons::icons::{ICON_ARROW_UPWARD, ICON_CLOSE, ICON_GROUPS, ICON_SETTINGS, ICON_SHUFFLE};

use crate::ui_tests::build_harness;
use crate::widgets::avatar_row::{self, AvatarRow};
use crate::widgets::badge;
use crate::widgets::button::{self, Focus, State};
use crate::widgets::list_row::{Handle, Handles, Leading, ListRow};
use crate::widgets::pill::{self, Mark};
use crate::widgets::round_button::RoundButton;
use crate::widgets::surface::{self, Kind};
use crate::widgets::toggle;

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

/// A round button names itself to AccessKit by its label, a toggle says whether it's on, and
/// the avatar button is named like the icon ones.
#[test]
fn round_buttons_name_themselves_and_toggles_say_whether_they_are_on() {
    let ui = build_harness(egui::vec2(400., 200.), (), |ui, _| {
        ui.horizontal(|ui| {
            RoundButton::icon(ICON_SETTINGS, "Settings").show(ui);
            RoundButton::icon(ICON_GROUPS, "Watch together").on(true).show(ui);
            RoundButton::icon(ICON_SHUFFLE, "Shuffle is off").on(false).show(ui);
            RoundButton::avatar("dev", "dev on Tabbyfin").show(ui);
        });
    });
    for name in ["Settings", "dev on Tabbyfin"] {
        let button = ui.get_by_role_and_label(Role::Button, name);
        assert_eq!(button.accesskit_node().toggled(), None, "{name:?} reads as a toggle");
    }
    let on = ui.get_by_role_and_label(Role::Button, "Watch together");
    assert_eq!(on.accesskit_node().toggled(), Some(Toggled::True));
    let off = ui.get_by_role_and_label(Role::Button, "Shuffle is off");
    assert_eq!(off.accesskit_node().toggled(), Some(Toggled::False));
}

/// A round button that skips focus (the search box's clear button, the SyncPlay panel's) is
/// passed over by Tab but still clicks; a disabled one takes no click.
#[test]
fn round_buttons_skip_focus_and_disabled_ones_ignore_clicks() {
    let clicks = std::rc::Rc::new(std::cell::Cell::new(0));
    let seen = clicks.clone();
    let mut ui = build_harness(egui::vec2(400., 200.), String::new(), move |ui, text| {
        let field = ui.text_edit_singleline(text);
        if ui.memory(|memory| memory.focused()).is_none() {
            field.request_focus();
        }
        ui.horizontal(|ui| {
            let skipped = RoundButton::icon(ICON_CLOSE, "Clear").focus(Focus::Skip).show(ui);
            let disabled = RoundButton::icon(ICON_ARROW_UPWARD, "Move up").enabled(false).show(ui);
            RoundButton::icon(ICON_SETTINGS, "Reached").show(ui);
            if skipped.clicked() || disabled.clicked() {
                seen.set(seen.get() + 1);
            }
        });
    });
    ui.run();
    ui.key_press(egui::Key::Tab);
    ui.run();
    assert!(
        !ui.get_by_label("Clear").is_focused(),
        "Tab stopped on the skipped button"
    );
    assert!(
        ui.get_by_label("Reached").is_focused(),
        "Tab didn't reach the next button"
    );

    ui.get_by_label("Clear").click();
    ui.run();
    ui.get_by_label("Move up").click();
    ui.run();
    assert_eq!(
        clicks.get(),
        1,
        "the skipped button should click and the disabled one not"
    );
}

/// A row with an action (a SyncPlay group to join) is one button named by its label: the
/// `brand` Join is part of it, not a second stop. People listed under a group are list items,
/// the user's own marked "you".
#[test]
fn an_action_row_is_one_button_and_people_are_list_items() {
    let joined = std::rc::Rc::new(std::cell::Cell::new(false));
    let seen = joined.clone();
    let mut ui = build_harness(egui::vec2(400., 300.), (), move |ui, _| {
        let names = ["ana", "bo"];
        let row = AvatarRow::new(&names, "Movie night", "ana and bo · Playing", "Join Movie night")
            .action("Join")
            .focus(Focus::Skip)
            .show(ui, true);
        if row.row.clicked() {
            seen.set(true);
        }
        avatar_row::person(ui, "dev", Some("you"));
        avatar_row::person(ui, "ana", None);
    });
    ui.run();
    assert!(ui.query_by_label("Join").is_none(), "the action is a second widget");
    ui.get_by_role_and_label(Role::Button, "Join Movie night").click();
    ui.run();
    assert!(joined.get(), "a click on the row didn't join");
    assert!(ui.query_by_role_and_label(Role::ListItem, "dev, you").is_some());
    assert!(ui.query_by_role_and_label(Role::ListItem, "ana").is_some());
}

/// A switch tells AccessKit it's a switch, by its name, with its state; a click and Space (once
/// Tab has put the keyboard on it) each flip it.
#[test]
fn a_switch_flips_by_click_and_space_and_names_its_state() {
    let mut ui = build_harness(egui::vec2(200., 100.), false, |ui, on| {
        toggle::switch(ui, on, "Animations");
    });
    ui.run();
    let toggled = |ui: &egui_kittest::Harness<'static, bool>| {
        ui.get_by_role_and_label(Role::Switch, "Animations")
            .accesskit_node()
            .toggled()
    };
    assert_eq!(toggled(&ui), Some(Toggled::False));

    ui.get_by_label("Animations").click();
    ui.run();
    assert!(*ui.state(), "a click turns it on");
    assert_eq!(toggled(&ui), Some(Toggled::True));

    ui.key_press(egui::Key::Tab);
    ui.run();
    assert!(ui.get_by_label("Animations").is_focused(), "Tab reaches it");
    ui.key_press(egui::Key::Space);
    ui.run();
    assert!(!*ui.state(), "Space turns it off");
    assert_eq!(toggled(&ui), Some(Toggled::False));
}

/// A row's handles exist only while it's active, say which one was pressed, stop at the list's
/// ends, and don't press the row under them.
#[test]
fn a_list_rows_handles_show_while_active_and_report_the_press() {
    #[derive(Default)]
    struct State {
        highlighted: bool,
        handle: Option<Handle>,
        row_clicks: usize,
    }
    let mut ui = build_harness(egui::vec2(400., 200.), State::default(), |ui, state| {
        let row = ListRow::new("Sintel")
            .compact()
            .leading(Leading::Number(2))
            .handles(Handles { up: false, down: true }, "Remove (Delete)")
            .highlighted(state.highlighted)
            .role(Role::MenuItem)
            .label("Sintel, playing")
            .show(ui);
        state.handle = row.handle.or(state.handle);
        state.row_clicks += usize::from(row.row.clicked());
    });
    assert!(ui.query_by_role_and_label(Role::MenuItem, "Sintel, playing").is_some());
    assert!(ui.query_by_label("Move down (Alt+Down)").is_none());

    ui.state_mut().highlighted = true;
    ui.run();
    assert!(ui.get_by_label("Move up (Alt+Up)").accesskit_node().is_disabled());
    ui.get_by_label("Move down (Alt+Down)").click();
    ui.run();
    assert_eq!(ui.state().handle, Some(Handle::Down));
    assert_eq!(ui.state().row_clicks, 0);
}

/// A dialog's row reads its detail too, and a click lands on the row.
#[test]
fn a_sunk_list_row_clicks_and_reads_its_detail() {
    let mut ui = build_harness(egui::vec2(400., 200.), 0, |ui, clicks| {
        let row = ListRow::new("Road trip")
            .sunk()
            .detail(Some("12 items"))
            .label("Road trip, 12 items")
            .no_hover()
            .show(ui);
        *clicks += usize::from(row.row.clicked());
    });
    ui.get_by_role_and_label(Role::Button, "Road trip, 12 items").click();
    ui.run();
    assert_eq!(*ui.state(), 1);
}

/// A filter's state reaches a screen reader, not just its colour and icon, and a tag and a label
/// read as text rather than as something to press.
#[test]
fn pills_and_labels_name_themselves() {
    let ui = build_harness(egui::vec2(400., 200.), (), |ui, _| {
        pill::filter(ui, "Drama", Mark::Off, false);
        pill::filter(ui, "Comedy", Mark::Included, false);
        pill::filter(ui, "Horror", Mark::Excluded, false);
        pill::tag(ui, "dystopia");
        badge::label(ui, "Transcoding", egui::Color32::BLACK, egui::Color32::WHITE);
    });
    for name in ["Drama", "Comedy, included", "Horror, excluded"] {
        assert!(ui.query_by_role_and_label(Role::Button, name).is_some(), "{name}");
    }
    for name in ["dystopia", "Transcoding"] {
        assert!(ui.query_by_role_and_label(Role::Label, name).is_some(), "{name}");
    }
}
