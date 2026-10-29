---
name: ui-mockup
description: Render a mockup of a tabbyfin UI (a widget, menu, page or variant) as a real egui screenshot and open it as an HTML gallery for the user to look at. Use when asked for a mockup, a design preview, or "what would X look like", before or while building UI.
---

# UI mockups

Mockups are egui rendered headless by `egui_kittest` (wgpu on lavapipe, no GPU or window needed), so they use the app's real theme, fonts and widgets. Each one is a **disposable `.rs` file outside the repo** (your scratch or temp dir, e.g. `$CLAUDE_JOB_DIR/tmp/`), never a file in `src/`.

## Workflow

1. Copy `.agents/skills/ui-mockup/template.rs` to a temp dir and edit it. It is compiled inside the app crate (as `ui_tests::scratch`, only with the `mockup` feature), so `crate::theme`, `crate::widgets`, the pages and `crate::ui_tests::build_harness` are all usable, with fixture data like the other `ui_tests`.
   - One `#[test] fn mockup_<name>()` per image, each calling `mockup("<name>", size, state, |ui, state| { … })`. For variants, one test each (`menu_a`, `menu_b`), so they land side by side in the gallery.
   - A new design means writing the egui code; keep it in the temp file until the user picks a direction, then move it into the app.
   - Wrap a page in `egui::CentralPanel::default().show(ui, …)` to get its background. For the other theme, set it on the `Config` passed to `crate::theme::apply`, or make one mockup per theme.
2. Run `python3 -I .agents/skills/ui-mockup/render.py <file>`. It compiles the file with `--features mockup`, runs its tests, writes `target/mockups/*.png` and `index.html`, and opens the gallery (`wslview` on WSL). Pass `--no-open` to skip opening.
3. Look at the PNGs yourself first (Read the image) and fix obvious problems before telling the user. Say where the gallery is: `target/mockups/index.html`.

## Notes

- Hover, pointer-only menus and animations aren't captured; open the thing by default in the mockup (or `ui.run()` after the state change) to show a state.
- The first run compiles the app's tests with the feature, which takes a while; later runs are quick. Don't run it in parallel with other cargo builds (this WSL crashes on parallel builds).
- `target/mockups/` is gitignored and keeps earlier renders for comparison; delete it to start clean. Mockups are throwaway: don't add them to `docs/` or the roadmap, and don't commit the temp file.
