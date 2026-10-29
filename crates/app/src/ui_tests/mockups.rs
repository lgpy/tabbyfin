//! Support for UI mockups: real egui rendered headless to PNGs. The mockups themselves are
//! disposable files outside the tree that `ui_tests/mod.rs` includes under the `mockup` feature;
//! `.agents/skills/ui-mockup/` has the workflow and `render.py`, which turns the PNGs and the
//! sidecars written here into a page.
//!
//! What's here is for those files to pick from, so a given run uses only some of it.
#![allow(dead_code)]

use std::path::PathBuf;
use std::rc::Rc;

use image::RgbaImage;
use image::imageops::{self, FilterType};

use egui_kittest::kittest::Queryable as _;

use crate::ui_tests::{ConfigDir, FixtureServer, PageRun, harness_builder, session, step_until};
use crate::views::{Page, PageCx};

/// The size a mockup draws at unless it asks for another, in points: one size for every mockup,
/// so a proposal and today's screen line up when the gallery compares them.
pub const WINDOW: egui::Vec2 = egui::vec2(1000., 760.);

/// Pixels per point in the saved PNGs: the gallery shows them at their size in points, so they
/// stay sharp on a HiDPI screen.
const SCALE: f32 = 2.;

/// How many times finer than [`SCALE`] a mockup is rendered before it's scaled down. The
/// headless renderer has no multisampling, which the window has (`main.rs`), and the mascot's
/// meshes rely on it for smooth edges, since epaint feathers only the shapes it tessellates.
const SUPERSAMPLE: f32 = 2.;

/// The empty margin `egui_kittest` leaves around a harness's `Ui`: the harness is made this much
/// larger on every side and the margin cropped off, so a mockup gets all of its size and no
/// transparent edge.
const HARNESS_MARGIN: f32 = 8.;

/// Which of the app's themes a mockup is drawn in.
#[derive(Clone, Copy)]
pub enum Themes {
    Dark,
    Light,
    /// One render per theme; the gallery shows them as one mockup with a theme toggle.
    Both,
}

/// What [`Mockup::behind`] paints under a page. `Rc` so each theme's frame can share it.
type Paint = Rc<dyn Fn(&egui::Ui)>;

/// A mockup to render: `Mockup::new("welcome.cozy").themes(Themes::Both).show(|| (), |ui, _| …)`.
///
/// The name is `group.variant`: the gallery puts a group's variants side by side, and compares
/// them when there are two (a variant named `now` is today's app).
pub struct Mockup {
    name: String,
    size: egui::Vec2,
    themes: Themes,
    until: Option<String>,
    behind: Option<Paint>,
}

impl Mockup {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            size: WINDOW,
            themes: Themes::Dark,
            until: None,
            behind: None,
        }
    }

    pub fn size(self, size: egui::Vec2) -> Self {
        Self { size, ..self }
    }

    pub fn themes(self, themes: Themes) -> Self {
        Self { themes, ..self }
    }

    /// Steps frames until something labelled `label` is drawn before rendering: for what
    /// arrives from the fixture server (a sign-in form after the server answers, a page's
    /// items), which settling the frames alone doesn't wait for.
    pub fn until(self, label: &str) -> Self {
        Self {
            until: Some(label.to_owned()),
            ..self
        }
    }

    /// Renders `frame` once per theme, each with a fresh `state`: what a page keeps (its
    /// textures, say) belongs to the harness it was made in.
    pub fn show<S: 'static>(self, state: impl Fn() -> S, frame: impl FnMut(&mut egui::Ui, &mut S) + Clone + 'static) {
        let themes: &[egui::Theme] = match self.themes {
            Themes::Dark => &[egui::Theme::Dark],
            Themes::Light => &[egui::Theme::Light],
            Themes::Both => &[egui::Theme::Dark, egui::Theme::Light],
        };
        for &theme in themes {
            render(&self, theme, state(), frame.clone());
        }
    }

    /// Paints `paint` under a [`Self::page`], in place of the panel's fill: a backdrop the
    /// page would sit on (`|ui| backdrop::fill(ui.painter(), ui.max_rect(), theme::tokens_of(ui))`).
    pub fn behind(self, paint: impl Fn(&egui::Ui) + 'static) -> Self {
        Self {
            behind: Some(Rc::new(paint)),
            ..self
        }
    }

    /// A signed-in page, opened as the shell opens it (without its top bar), with services over
    /// `server`:
    /// `.page(&server, |cx| crate::views::open(Route::Home, cx))`. The server answers with the
    /// fixtures it was given (`FixtureServer::json`).
    pub fn page(self, server: &FixtureServer, open: impl FnOnce(&mut PageCx) -> Box<dyn Page> + Clone + 'static) {
        let behind = self.behind.clone();
        self.show(
            || PageRun::new(session(server), open.clone()),
            move |ui, run| {
                // The shell's frame for a page: the pages keep their own margins.
                let frame = match &behind {
                    Some(paint) => {
                        paint(ui);
                        egui::Frame::NONE
                    }
                    None => egui::Frame::NONE.fill(ui.visuals().panel_fill),
                };
                egui::CentralPanel::default().frame(frame).show(ui, |ui| run.show(ui));
            },
        );
    }
}

/// Renders `frame` at `size` in the dark theme: [`Mockup`] for one that needs nothing else.
pub fn mockup<S: 'static>(name: &str, size: egui::Vec2, state: S, frame: impl FnMut(&mut egui::Ui, &mut S) + 'static) {
    render(&Mockup::new(name).size(size), egui::Theme::Dark, state, frame);
}

/// Writes `<name>.<theme>.png` and its `.json` sidecar to `<TABBYFIN_MOCKUP_DIR>`, the directory
/// `render.py` sets (`target/mockups` by default).
fn render<S: 'static>(
    mockup: &Mockup,
    theme: egui::Theme,
    state: S,
    frame: impl FnMut(&mut egui::Ui, &mut S) + 'static,
) {
    let Mockup { name, size, .. } = mockup;
    let size = *size;
    let dir = std::env::var_os("TABBYFIN_MOCKUP_DIR").map_or_else(|| PathBuf::from("target/mockups"), PathBuf::from);
    std::fs::create_dir_all(&dir).unwrap();
    let image = snapshot(size, SCALE * SUPERSAMPLE, theme, mockup.until.as_deref(), state, frame);
    let (width, height) = ((size.x * SCALE).round() as u32, (size.y * SCALE).round() as u32);
    let image = imageops::resize(&image, width, height, FilterType::Triangle);
    let theme = match theme {
        egui::Theme::Dark => "dark",
        egui::Theme::Light => "light",
    };
    image.save(dir.join(format!("{name}.{theme}.png"))).unwrap();
    let sidecar = serde_json::json!({ "name": name, "theme": theme, "size": [size.x, size.y], "scale": SCALE });
    std::fs::write(dir.join(format!("{name}.{theme}.json")), sidecar.to_string()).unwrap();
}

/// `frame` drawn at `size` points and `pixels_per_point`, settled (once `until` is drawn, if
/// given) and rendered.
///
/// It holds a [`ConfigDir`], as the other UI tests do, so what a page saves goes to a temporary
/// config and never the user's; that also renders one mockup at a time.
fn snapshot<S: 'static>(
    size: egui::Vec2,
    pixels_per_point: f32,
    theme: egui::Theme,
    until: Option<&str>,
    state: S,
    frame: impl FnMut(&mut egui::Ui, &mut S) + 'static,
) -> RgbaImage {
    let _config = ConfigDir::new("mockup");
    let mut ui = harness_builder(size + egui::Vec2::splat(2. * HARNESS_MARGIN))
        .with_pixels_per_point(pixels_per_point)
        .with_theme(theme)
        .build_ui_state(frame, state);
    if let Some(label) = until {
        step_until(&mut ui, label, |ui| ui.query_by_label(label).is_some());
    }
    // A spinner repaints forever and never settles: what's drawn by then is the picture.
    let _ = ui.try_run();
    let mut image = ui.render().expect("render (needs a Vulkan adapter; lavapipe works)");
    let px = |points: f32| (points * pixels_per_point).round() as u32;
    imageops::crop(
        &mut image,
        px(HARNESS_MARGIN),
        px(HARNESS_MARGIN),
        px(size.x),
        px(size.y),
    )
    .to_image()
}

/// The longest edge an image keeps when an agent reads it; anything larger is scaled down to it,
/// so the preview is laid out to fit and costs what one image costs.
const PREVIEW_EDGE: f32 = 1568.;
const PREVIEW_GAP: f32 = 12.;
const PREVIEW_LABEL: f32 = 22.;

/// One image on the preview sheet, as `render.py` lists it.
#[derive(serde::Deserialize)]
struct Tile {
    path: PathBuf,
    label: String,
    /// The part to show, in the mockup's points (`x, y, w, h`); all of it if absent.
    crop: Option<[f32; 4]>,
}

#[derive(serde::Deserialize)]
struct Preview {
    out: PathBuf,
    tiles: Vec<Tile>,
}

/// `render.py`'s preview sheet: the mockups an agent should look at, or a region of one, laid out
/// in a single image no larger than [`PREVIEW_EDGE`], so checking a run is one image read rather
/// than one per mockup. Does nothing unless `TABBYFIN_MOCKUP_PREVIEW` names the list to draw.
#[test]
fn preview() {
    let Some(list) = std::env::var_os("TABBYFIN_MOCKUP_PREVIEW") else {
        return;
    };
    let preview: Preview = serde_json::from_str(&std::fs::read_to_string(list).unwrap()).unwrap();
    let images: Vec<(String, RgbaImage)> = preview
        .tiles
        .into_iter()
        .map(|tile| {
            let mut image = image::open(&tile.path).unwrap().to_rgba8();
            if let Some([x, y, w, h]) = tile.crop {
                let px = |points: f32| (points * SCALE).round() as u32;
                let (x, y) = (px(x).min(image.width() - 1), px(y).min(image.height() - 1));
                let (w, h) = (px(w).min(image.width() - x), px(h).min(image.height() - y));
                image = imageops::crop(&mut image, x, y, w, h).to_image();
            }
            (tile.label, image)
        })
        .collect();
    let n = images.len();
    assert!(n > 0, "nothing to preview");
    let aspect = images
        .iter()
        .map(|(_, i)| i.height() as f32 / i.width() as f32)
        .fold(0., f32::max);
    // The fewest columns whose sheet still fits: as large as each tile can be. A lone crop isn't
    // blown up past its own pixels.
    let widest = images.iter().map(|(_, i)| i.width()).max().unwrap() as f32;
    let layout = |columns: usize| {
        let width = ((PREVIEW_EDGE - PREVIEW_GAP * (columns as f32 + 1.)) / columns as f32)
            .min(widest)
            .floor();
        let rows = n.div_ceil(columns) as f32;
        let height = rows * (PREVIEW_LABEL + (width * aspect).ceil() + PREVIEW_GAP) + PREVIEW_GAP;
        (width, height)
    };
    let columns = (1..=n).find(|&c| layout(c).1 <= PREVIEW_EDGE).unwrap_or(n);
    let (tile_width, height) = layout(columns);
    let width = columns.min(n) as f32 * (tile_width + PREVIEW_GAP) + PREVIEW_GAP;
    let tiles: Vec<(String, egui::ColorImage)> = images
        .into_iter()
        .map(|(label, image)| {
            let h = (tile_width * image.height() as f32 / image.width() as f32).round() as u32;
            let image = imageops::resize(&image, tile_width as u32, h, FilterType::Triangle);
            let size = [image.width() as usize, image.height() as usize];
            (label, egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw()))
        })
        .collect();
    let mut textures: Vec<egui::TextureHandle> = Vec::new();
    let sheet = snapshot(
        egui::vec2(width, height),
        1.,
        egui::Theme::Dark,
        None,
        (),
        move |ui, _| {
            if textures.is_empty() {
                textures = tiles
                    .iter()
                    .map(|(label, image)| {
                        ui.ctx()
                            .load_texture(label, image.clone(), egui::TextureOptions::LINEAR)
                    })
                    .collect();
            }
            let origin = ui.max_rect().min;
            let painter = ui.painter();
            painter.rect_filled(ui.max_rect(), 0., egui::Color32::from_gray(0x24));
            let row_height = PREVIEW_LABEL + (tile_width * aspect).ceil() + PREVIEW_GAP;
            for (i, ((label, _), texture)) in tiles.iter().zip(&textures).enumerate() {
                let at = origin
                    + egui::vec2(
                        PREVIEW_GAP + (i % columns) as f32 * (tile_width + PREVIEW_GAP),
                        PREVIEW_GAP + (i / columns) as f32 * row_height,
                    );
                let font = egui::FontId::proportional(14.);
                painter.text(at, egui::Align2::LEFT_TOP, label, font, egui::Color32::from_gray(0xe0));
                let rect = egui::Rect::from_min_size(at + egui::vec2(0., PREVIEW_LABEL), texture.size_vec2());
                let uv = egui::Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(1., 1.));
                painter.image(texture.id(), rect, uv, egui::Color32::WHITE);
            }
        },
    );
    sheet.save(&preview.out).unwrap();
}
