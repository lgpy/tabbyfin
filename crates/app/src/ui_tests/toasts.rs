//! Toasts through the UI: the lines a toast draws, its action button coming back to the app,
//! the close button, and the cap on how many are up at once.

use egui::accesskit::Role;
use egui_kittest::kittest::Queryable;

use crate::toasts::{Toast, ToastAction, Toasts};
use crate::ui_tests::build_harness;

/// The app's toasts: drawn over everything, with the action a click asks for.
struct ToastRun {
    toasts: Toasts,
    action: Option<ToastAction>,
}

impl ToastRun {
    fn new(toasts: Toasts) -> Self {
        Self { toasts, action: None }
    }

    fn show(&mut self, ui: &mut egui::Ui) {
        if let Some(action) = self.toasts.show(ui.ctx()) {
            self.action = Some(action);
        }
    }
}

fn toast_harness(toasts: Toasts) -> egui_kittest::Harness<'static, ToastRun> {
    build_harness(egui::vec2(800., 600.), ToastRun::new(toasts), |ui, state| {
        state.show(ui)
    })
}

/// A toast draws its title and text, and its button's click comes back to the app as the
/// action it carries, closing the toast.
#[test]
fn a_toast_shows_its_lines_and_runs_its_action() {
    let mut toasts = Toasts::default();
    toasts.push(Toast::success("Playlist saved").title("Saved"));
    toasts.push(
        Toast::error("Couldn't reach the server.").action("Retry", ToastAction::OpenUrl("https://example.com".into())),
    );
    let mut ui = toast_harness(toasts);

    assert!(ui.query_by_label("Saved").is_some());
    assert!(ui.query_by_label("Playlist saved").is_some());
    assert!(ui.query_by_label("Couldn't reach the server.").is_some());

    ui.get_by_label("Retry").click();
    ui.step();
    assert_eq!(
        ui.state().action,
        Some(ToastAction::OpenUrl("https://example.com".into()))
    );
    // The click shows in the tree a frame after its release, so a second frame draws the
    // toast list without it.
    ui.step();
    assert!(ui.query_by_label("Retry").is_none(), "the clicked toast stayed up");
    assert!(
        ui.query_by_label("Playlist saved").is_some(),
        "the other toast went with it"
    );
}

/// The close button takes a toast away without asking the app to do anything.
#[test]
fn the_close_button_drops_a_toast() {
    let mut toasts = Toasts::default();
    toasts.push(Toast::info("Connecting…"));
    let mut ui = toast_harness(toasts);

    // With no action button, the toast's only button is its close.
    ui.get_by_role(Role::Button).click();
    ui.step();
    ui.step();
    assert!(ui.query_by_label("Connecting…").is_none());
    assert!(ui.state().action.is_none());
}

/// More toasts than fit: the oldest goes, so the newest are the ones on screen.
#[test]
fn the_oldest_go_when_too_many_pile_up() {
    let mut toasts = Toasts::default();
    for i in 0..5 {
        toasts.push(Toast::info(i.to_string()));
    }
    let ui = toast_harness(toasts);

    assert!(ui.query_by_label("0").is_none(), "the oldest toast stayed up");
    for i in 1..5 {
        assert!(ui.query_by_label(&i.to_string()).is_some(), "toast {i} went");
    }
}
