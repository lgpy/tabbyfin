//! Support for UI mockups: real egui rendered headless to PNGs. The mockups themselves are
//! disposable files outside the tree that `ui_tests/mod.rs` includes under the `mockup` feature;
//! `.agents/skills/ui-mockup/` has the workflow and `render.py`.

use std::path::PathBuf;

use crate::ui_tests::build_harness;

/// Renders `frame` at `size` to `<TABBYFIN_MOCKUP_DIR>/<name>.png`, the directory `render.py`
/// sets (`target/mockups` by default).
pub fn mockup<S: 'static>(name: &str, size: egui::Vec2, state: S, frame: impl FnMut(&mut egui::Ui, &mut S) + 'static) {
    let dir = std::env::var_os("TABBYFIN_MOCKUP_DIR").map_or_else(|| PathBuf::from("target/mockups"), PathBuf::from);
    std::fs::create_dir_all(&dir).unwrap();
    let mut ui = build_harness(size, state, frame);
    ui.run();
    let image = ui.render().expect("render (needs a Vulkan adapter; lavapipe works)");
    image.save(dir.join(format!("{name}.png"))).unwrap();
}
