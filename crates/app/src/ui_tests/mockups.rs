//! UI mockups: real egui rendered headless to PNGs, for looking at a design before (or while)
//! building it. Not checks, so every one is `#[ignore]`d; `.agents/skills/ui-mockup/` has the
//! workflow and `render.py`, which runs them and opens a gallery.

use std::path::PathBuf;

use crate::toasts::{Toast, ToastAction, Toasts};
use crate::ui_tests::build_harness;

/// Renders `frame` at `size` to `<TABBYFIN_MOCKUP_DIR>/<name>.png`, the directory `render.py`
/// sets (`target/mockups` by default).
fn mockup<S: 'static>(name: &str, size: egui::Vec2, state: S, frame: impl FnMut(&mut egui::Ui, &mut S) + 'static) {
    let dir = std::env::var_os("TABBYFIN_MOCKUP_DIR").map_or_else(|| PathBuf::from("target/mockups"), PathBuf::from);
    std::fs::create_dir_all(&dir).unwrap();
    let mut ui = build_harness(size, state, frame);
    ui.run();
    let image = ui.render().expect("render (needs a Vulkan adapter; lavapipe works)");
    image.save(dir.join(format!("{name}.png"))).unwrap();
}

#[test]
#[ignore = "mockup"]
fn mockup_toasts() {
    let mut toasts = Toasts::default();
    toasts.push(Toast::success("Playlist saved").title("Saved"));
    toasts.push(
        Toast::error("Couldn't reach the server.").action("Retry", ToastAction::OpenUrl("https://example.com".into())),
    );
    mockup("toasts", egui::vec2(800., 600.), toasts, |ui, toasts| {
        toasts.show(ui.ctx());
    });
}
