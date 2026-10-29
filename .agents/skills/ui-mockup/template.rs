// A disposable mockup: copy this outside the repo, edit, and run
//   python3 -I .agents/skills/ui-mockup/render.py <this file>
// It is compiled inside the app crate (module `ui_tests::scratch`), so `crate::…` paths work.

use crate::ui_tests::mockups::mockup;

#[test]
fn mockup_example() {
    mockup("example", egui::vec2(800., 600.), (), |ui, _| {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("Mockup");
            ui.label("Replace this with the widgets or page being designed.");
        });
    });
}
