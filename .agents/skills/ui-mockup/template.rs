// A disposable mockup: copy this outside the repo, edit, and run
//   python3 -I .agents/skills/ui-mockup/render.py <this file>
// It is compiled inside the app crate (module `ui_tests::scratch`), so `crate::…` paths work.
// Replace this header with the page's intro: what was asked, and what the proposal changes.

use crate::ui_tests::mockups::{Mockup, Themes};

// ## The example
// What this section shows and what to look at.

/// Today's screen
/// What it looks like now, drawn by the real page.
#[test]
fn mockup_example_now() {
    Mockup::new("example.now").themes(Themes::Both).show(
        || (),
        |ui, _| {
            egui::CentralPanel::default().show(ui, |ui| {
                ui.heading("Mockup");
            });
        },
    );
}

/// The proposal
/// What differs from today's, and why.
#[test]
fn mockup_example_proposed() {
    Mockup::new("example.proposed").themes(Themes::Both).show(
        || (),
        |ui, _| {
            egui::CentralPanel::default().show(ui, |ui| {
                ui.heading("Mockup");
                ui.label("Replace this with the widgets or page being designed.");
            });
        },
    );
}
