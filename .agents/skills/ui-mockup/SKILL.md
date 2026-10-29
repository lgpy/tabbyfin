---
name: ui-mockup
description: Render a mockup of a tabbyfin UI (a widget, menu, page or variant) as a real egui screenshot and open it as an HTML gallery for the user to look at. Use when asked for a mockup, a design preview, or "what would X look like", before or while building UI.
---

# UI mockups

Mockups are egui rendered headless by `egui_kittest` (wgpu on lavapipe, no GPU or window needed), so they use the app's real theme, fonts and widgets. They are not checks: each is an `#[ignore]`d test in `crates/app/src/ui_tests/mockups.rs`.

## Workflow

1. Add a test to `crates/app/src/ui_tests/mockups.rs`:

   ```rust
   #[test]
   #[ignore = "mockup"]
   fn mockup_<name>() {
       mockup("<name>", egui::vec2(800., 600.), /* state */, |ui, state| {
           // draw with the real widgets or pages; wrap in egui::CentralPanel for a page background
       });
   }
   ```

   - The name must start with `mockup_` and the PNG name should say what it shows. For variants, one test each (`menu_a`, `menu_b`), so they land side by side in the gallery.
   - Reuse what exists: widgets from `crate::widgets`, pages with fixture data like the other `ui_tests` (`build_harness`, wiremock fixtures). A new design means writing the egui code; keep it in the mockup test until the user picks a direction, then move it into the app.
   - Dark and light: `crate::theme::apply(ctx, &config)` with a `Config` whose theme is set, inside the frame closure's first run, or one mockup per theme.
2. Run `python3 -I .agents/skills/ui-mockup/render.py [filter]`. It runs the `mockup_*` tests (or those matching `filter`), writes `target/mockups/*.png` and `index.html`, and opens the gallery (`wslview` on WSL). Pass `--no-open` to skip opening.
3. Look at the PNGs yourself first (Read the image) and fix obvious problems before telling the user. Say where the gallery is: `target/mockups/index.html`.

## Notes

- Hover, pointer-only menus and animations aren't captured; open the thing by default in the mockup (or call `ui.run()` after the state change) to show a state.
- The first build compiles wgpu and takes over a minute; later runs take seconds. Don't run it in parallel with other cargo builds (this WSL crashes on parallel builds).
- Mockups are throwaway. Don't add them to `docs/` or the roadmap; commit them only if the user wants them kept.
