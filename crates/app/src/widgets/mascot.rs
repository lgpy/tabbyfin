//! Mewdusa, the jellycat from the app icon, drawn as the loading indicator.
//!
//! Ported from `main`'s `mascot.rs`, which traced the 2000×2000 drawings in `packaging/mascot/`:
//! the head pulses and bobs, the tentacles wave, and now and then it blinks or flicks an ear.
//! With animations off it holds still in its rest pose.
//!
//! It has three faces, each from its own drawing. A loader shows the cute or the evil one (eyes
//! cut flat by slanted brows, an "ω" mouth), picked at random when it appears. Empty pages show
//! the confused one, scratching its head under a question mark.
//!
//! Blinks and flicks come from a hash of the time slot rather than stored state, so a mascot can
//! be built fresh on every render and still never repeat in lockstep.
//!
//! egui fills convex paths only, and these outlines are concave, so every piece is tessellated
//! with `lyon_tessellation`: once, into a mesh in the drawings' own units ([`Shapes`]). Drawing
//! a pose moves those cached vertices — a scale and a bob for the head and everything on it, each
//! ear's own flick for its vertices, a travelling wave for the tentacles' — then maps them into
//! the space the mascot was given. Nothing is tessellated again, and the meshes are not
//! antialiased (epaint feathers only the shapes it tessellates itself): at the sizes drawn, the
//! edges of the head are the only place it shows.
//!
//! A mascot that animates asks for a repaint once per animation step ([`ANIMATION_STEP`]) and one
//! with animations off asks for none, so a loader costs nothing once it has drawn.

use std::collections::hash_map::RandomState;
use std::f32::consts::TAU;
use std::hash::{BuildHasher, Hasher};
use std::sync::OnceLock;
use std::time::Duration;

use egui::epaint::{Vertex, WHITE_UV};
use egui::{Color32, Mesh, Pos2, Rect, Sense, Ui, Vec2, pos2};
use lyon_tessellation::path::Path;
use lyon_tessellation::path::math::point;
use lyon_tessellation::{
    BuffersBuilder, FillOptions, FillTessellator, FillVertex, LineCap, LineJoin, StrokeOptions, StrokeTessellator,
    StrokeVertex, VertexBuffers,
};

/// A point in the drawings' 2000×2000 space.
type P = [f32; 2];

/// How often an animating mascot asks for the next frame: the pose moves the whole time it's
/// drawn, so it needs a frame each step while it's on screen, and none once it's gone. 30 a
/// second is smooth enough for something this size, for half the drawing.
const ANIMATION_STEP: Duration = Duration::from_millis(33);

/// The drawings' frame, plus room above for the ears: at the top of the bob the stretched head
/// lifts them past y = 0.
const FRAME_ORIGIN: P = [-40., -110.];
const FRAME_SIZE: f32 = 2080.;

/// Seconds per pulse.
const PERIOD: f32 = 1.0;
/// How far the tentacle tips usually swing, in drawing units.
const SWAY: f32 = 126.;
const BOB: f32 = 46.;
const SQUASH: f32 = 0.045;
/// Where the head scales from as it pulses.
const BODY_CENTER: P = [980., 600.];

const BLINK_LEN: f32 = 0.24;
const BLINK_EVERY: f32 = 3.6;
const FLICK_LEN: f32 = 0.55;
const FLICK_EVERY: f32 = 5.0;

const BODY: Color32 = Color32::from_rgb(0xc3, 0x96, 0xe7);
const EAR_INNER: Color32 = Color32::from_rgb(0x8a, 0x63, 0xa9);
const TENTACLE_TOP: Color32 = Color32::from_rgb(0xbd, 0x97, 0xe7);
const TENTACLE_BOTTOM: Color32 = Color32::from_rgb(0x9a, 0x8e, 0xe9);
/// The blush's colour with a low alpha; it's stacked to fade out from the middle.
const BLUSH: Color32 = Color32::from_rgba_unmultiplied_const(0xe0, 0x8c, 0xe0, 0x22);
const WHITE: Color32 = Color32::WHITE;
const QUESTION_MARK: Color32 = Color32::from_rgb(0xdd, 0x7a, 0x93);

const TENTACLE_WIDTH: f32 = 134.;
const EAR_ROUNDING: f32 = 40.;
const EAR_INNER_ROUNDING: f32 = 30.;
const MOUTH_WIDTH: f32 = 26.;
const CLOSED_EYE_WIDTH: f32 = 34.;
const QUESTION_WIDTH: f32 = 72.;

/// How often a loader shows the evil face rather than the cute one.
const EVIL_ODDS: f32 = 0.5;

/// How far a flattened outline may stray from the curve, in drawing units: fine enough that a
/// 160-point mascot shows no corners.
const TOLERANCE: f32 = 0.5;

/// The head's outline without the ears. The points behind the ears stay inside both ear poses.
#[rustfmt::skip]
const HEAD: [P; 46] = [
    [350., 470.], [372., 360.], [430., 262.], [505., 205.], [570., 160.], [650., 122.], [730., 94.],
    [850., 70.], [970., 64.], [1090., 64.], [1170., 73.], [1250., 86.], [1330., 104.], [1400., 170.],
    [1470., 262.], [1495., 350.], [1513., 410.], [1549., 475.], [1640., 550.], [1725., 600.],
    [1783., 650.], [1834., 725.], [1855., 825.], [1851., 875.], [1830., 925.], [1782., 975.],
    [1678., 1025.], [1575., 1052.], [1489., 1072.], [1330., 1083.], [1090., 1093.], [890., 1091.],
    [650., 1080.], [410., 1053.], [290., 1028.], [210., 997.], [146., 950.], [116., 900.],
    [111., 850.], [117., 800.], [134., 750.], [171., 700.], [262., 650.], [320., 600.],
    [341., 550.], [348., 500.],
];

/// An ear's rest pose and its flicked pose (from the flick drawing), as polygons with rounded
/// corners. The base points sit inside the head and don't move, so the seam stays hidden.
struct Ear {
    up: &'static [P],
    flicked: &'static [P],
    inner_up: [P; 3],
    inner_flicked: [P; 3],
}

#[rustfmt::skip]
const EARS: [Ear; 2] = [
    Ear {
        up: &[[312., 190.], [522., 192.], [580., 178.], [515., 420.], [362., 462.], [334., 330.]],
        flicked: &[[268., 350.], [522., 192.], [580., 178.], [515., 420.], [362., 462.], [305., 420.]],
        inner_up: [[356., 256.], [458., 275.], [406., 405.]],
        inner_flicked: [[332., 352.], [466., 284.], [402., 420.]],
    },
    Ear {
        up: &[[1275., 108.], [1335., 118.], [1495., 66.], [1506., 190.], [1505., 385.], [1505., 430.], [1390., 330.]],
        flicked: &[[1275., 108.], [1335., 118.], [1612., 250.], [1560., 325.], [1515., 395.], [1505., 430.], [1390., 330.]],
        inner_up: [[1337., 172.], [1447., 130.], [1420., 275.]],
        inner_flicked: [[1345., 182.], [1545., 242.], [1440., 300.]],
    },
];

/// A tentacle's rest pose from its root (hidden under the head) to its tip, where in the wave it
/// starts so neighbours swing one after another, and how far its tip swings.
struct Tentacle {
    points: &'static [P],
    phase: f32,
    sway: f32,
}

#[rustfmt::skip]
const TENTACLES: [Tentacle; 5] = [
    Tentacle {
        points: &[[487., 1000.], [480., 1150.], [472., 1290.], [450., 1385.], [400., 1452.], [320., 1500.], [220., 1517.], [128., 1507.]],
        phase: 0.0,
        sway: SWAY,
    },
    Tentacle {
        points: &[[752., 1000.], [756., 1200.], [764., 1450.], [780., 1640.], [778., 1730.], [772., 1795.]],
        phase: 0.8,
        sway: SWAY,
    },
    Tentacle {
        points: &[[1012., 1000.], [1022., 1200.], [1050., 1460.], [1085., 1620.], [1120., 1715.], [1143., 1770.]],
        phase: 1.6,
        sway: SWAY,
    },
    Tentacle {
        points: &[[1227., 1000.], [1245., 1200.], [1295., 1360.], [1370., 1500.], [1460., 1620.], [1585., 1745.]],
        phase: 2.4,
        sway: SWAY,
    },
    Tentacle {
        points: &[[1427., 1000.], [1462., 1200.], [1500., 1290.], [1560., 1350.], [1660., 1380.], [1750., 1365.], [1815., 1330.], [1858., 1285.]],
        phase: 3.2,
        sway: SWAY,
    },
];

/// Centre and radius of each eye.
const EYES: [(P, f32); 2] = [([686., 621.], 106.), ([1228., 544.], 111.)];
/// The closed eyes from the blink drawing, as quadratic curves: start, control, end.
const CLOSED_EYES: [[P; 3]; 2] = [
    [[572., 647.], [680., 607.], [798., 625.]],
    [[1115., 583.], [1230., 539.], [1342., 514.]],
];
/// The pouty mouth: up to the peak, round over it, down the other side.
const MOUTH: [[P; 3]; 3] = [
    [[932., 790.], [957., 747.5], [982., 705.]],
    [[982., 705.], [994., 690.], [1010., 700.]],
    [[1010., 700.], [1044., 727.5], [1078., 755.]],
];
/// The evil eyes keep only what's below these brow lines, one per eye, each given by two points.
const BROWS: [[P; 2]; 2] = [[[560., 573.], [815., 592.]], [[1095., 569.], [1355., 460.]]];
/// The evil "ω" mouth as cubic curves: start, control, control, end.
const EVIL_MOUTH: [[P; 4]; 5] = [
    [[895., 718.], [888., 770.], [915., 798.], [945., 796.]],
    [[945., 796.], [972., 794.], [985., 770.], [990., 745.]],
    [[990., 745.], [1000., 775.], [1025., 795.], [1055., 790.]],
    [[1055., 790.], [1090., 786.], [1112., 760.], [1106., 730.]],
    [[1106., 730.], [1104., 712.], [1095., 700.], [1088., 690.]],
];
/// Points sampled around an evil eye before it's cut by its brow.
const EYE_SAMPLES: usize = 48;

/// The confused face's rightmost tentacle: it comes out from under the head, curls down and
/// back up, and scratches the side of the head with its tip.
#[rustfmt::skip]
const SCRATCHING: Tentacle = Tentacle {
    points: &[[1430., 980.], [1470., 1030.], [1530., 1105.], [1590., 1180.], [1670., 1245.], [1760., 1255.],
        [1850., 1215.], [1895., 1140.], [1900., 1060.], [1860., 975.], [1790., 905.], [1700., 855.]],
    phase: 3.2,
    // A gentler sway than the others, so the tip stays on the head.
    sway: 0.35 * SWAY,
};
/// The point from which the scratching tentacle is painted again over the head. It's below the
/// head, so the seam doesn't show.
const SCRATCHING_FRONT: usize = 6;
/// The question mark's hook as cubic curves, and its dot's centre and radius.
const QUESTION_HOOK: [[P; 4]; 3] = [
    [[1744., 134.], [1786., 104.], [1862., 108.], [1891., 192.]],
    [[1891., 192.], [1908., 250.], [1852., 292.], [1790., 293.]],
    [[1790., 293.], [1752., 294.], [1735., 302.], [1726., 322.]],
];
const QUESTION_DOT: (P, f32) = ([1680., 439.], 54.);
/// How far the question mark floats up and down, in drawing units.
const QUESTION_FLOAT: f32 = 16.;
/// Centre and radii of each cheek.
const CHEEKS: [(P, f32, f32); 2] = [([626., 801.], 155., 95.), ([1338., 681.], 145., 88.)];
const SHADOW: P = [990., 1915.];
/// The shadow's width at the top and bottom of the bob.
const SHADOW_WIDTH: f32 = 500.;
const SHADOW_GROWTH: f32 = 140.;
const SHADOW_HEIGHT: f32 = 30.;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Face {
    Cute,
    Evil,
    Confused,
}

impl Face {
    /// The cute or the evil face for `roll`, a number in 0..1.
    fn for_loader(roll: f32) -> Self {
        if roll < EVIL_ODDS { Face::Evil } else { Face::Cute }
    }
}

/// The size a page draws the mascot at in its loader and over an empty state, as on `main`.
pub const PAGE: f32 = 96.;

/// The mascot as a loader in a `size` square, animating unless animations are off. Its face is
/// picked when it appears and kept while it's showing.
pub fn loader(ui: &mut Ui, size: f32) {
    let face = kept_face(ui);
    show(ui, size, face);
}

/// The confused mascot, for a page with nothing on it.
pub fn confused(ui: &mut Ui, size: f32) {
    show(ui, size, Face::Confused);
}

/// Draws `face` in a `size` square at the `ui`'s cursor, and asks for the next frame while it
/// moves.
fn show(ui: &mut Ui, size: f32, face: Face) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    // The animations setting is egui's own animation time, which `theme::apply_animations` turns
    // off with it: `main`'s loader read the same switch (`cx.reduce_motion()`).
    let still = ui.style().animation_time == 0.;
    let pose = if still {
        Pose::rest()
    } else {
        Pose::at(ui.input(|input| input.time) as f32)
    };
    paint(ui, Shapes::get(), &pose, face, rect);
    if !still {
        ui.ctx().request_repaint_after(ANIMATION_STEP);
    }
}

/// The face a loader keeps from its last pass, or a new pick. Kept in egui's memory under the
/// `ui`'s id, so it lasts as long as the same place keeps drawing a loader in consecutive passes
/// — a loader that goes away and comes back picks again, as `main`'s element state did.
fn kept_face(ui: &Ui) -> Face {
    #[derive(Clone, Copy)]
    struct Kept {
        face: Face,
        pass: u64,
    }

    let id = ui.scope_id().with("mascot-face");
    let pass = ui.ctx().cumulative_pass_nr();
    let picked = || Kept {
        face: kept_or_picked(None, || noise(RandomState::new().build_hasher().finish())),
        pass,
    };
    ui.data_mut(|data| {
        let kept = data.get_temp_mut_or_insert_with(id, picked);
        if kept.pass + 1 != pass {
            *kept = picked();
        } else {
            kept.pass = pass;
        }
        kept.face
    })
}

/// The face kept from the last frame, or else a new pick from `roll`.
fn kept_or_picked(kept: Option<Face>, roll: impl FnOnce() -> f32) -> Face {
    kept.unwrap_or_else(|| Face::for_loader(roll()))
}

#[derive(Clone, Copy)]
enum Eyes {
    /// Open, squashed vertically to this fraction while blinking.
    Open(f32),
    Closed,
}

/// Where the mascot is at one moment: what the pose does to the drawings' own shapes.
struct Pose {
    /// Vertical offset of everything but the shadow.
    bob: f32,
    /// The head's horizontal and vertical scale around [`BODY_CENTER`].
    scale: [f32; 2],
    /// How far each ear is toward its flicked pose, 0 to 1.
    ears: [f32; 2],
    eyes: Eyes,
    /// 0 at the top of the bob, 1 at the bottom.
    low: f32,
    /// The confused face's question mark's offset from its place, on top of the bob.
    float: f32,
    /// The phase the tentacles wave at, or `None` to hold them in their rest pose.
    phase: Option<f32>,
}

impl Pose {
    fn rest() -> Self {
        Pose {
            bob: 0.,
            scale: [1., 1.],
            ears: [0., 0.],
            eyes: Eyes::Open(1.),
            low: 0.5,
            float: 0.,
            phase: None,
        }
    }

    fn at(time: f32) -> Self {
        let phase = time * TAU / PERIOD;
        let pulse = phase.sin();
        // The bob lags the pulse a little, and the ears droop a touch after that.
        let bob = -BOB * (phase - 0.6).sin();
        let droop = 0.07 * (0.5 + 0.5 * (phase - 1.).sin());
        let flicks = flicks(time);
        Pose {
            bob,
            // Narrow and tall on the push, then relaxed.
            scale: [1. - SQUASH * pulse, 1. + SQUASH * pulse],
            ears: [0, 1].map(|i| (flicks[i] + droop).min(1.)),
            eyes: blink(time),
            low: (bob + BOB) / (2. * BOB),
            float: QUESTION_FLOAT * (phase - 1.6).sin(),
            phase: Some(phase),
        }
    }

    /// The transform from the drawings' space to this moment's: the head's scale around
    /// [`BODY_CENTER`], then the bob.
    fn body(&self) -> Affine {
        Affine::scale(BODY_CENTER, self.scale).then(Affine::translate([0., self.bob]))
    }
}

/// One blink at a random moment in each [`BLINK_EVERY`]: the eyes squash, close, and open again.
fn blink(time: f32) -> Eyes {
    let slot = (time / BLINK_EVERY).floor();
    let at = slot * BLINK_EVERY + 0.4 + noise(slot as u64) * (BLINK_EVERY - BLINK_LEN - 0.8);
    match (time - at) / BLINK_LEN {
        u if !(0. ..=1.).contains(&u) => Eyes::Open(1.),
        u if u < 0.25 => Eyes::Open(1. - u / 0.25 * 0.65),
        u if u < 0.7 => Eyes::Closed,
        u => Eyes::Open(0.35 + (u - 0.7) / 0.3 * 0.65),
    }
}

/// One ear flick at a random moment in each [`FLICK_EVERY`]: out, back, and a smaller second
/// flick, like a cat's. Usually one ear, now and then both.
fn flicks(time: f32) -> [f32; 2] {
    let slot = (time / FLICK_EVERY).floor();
    let n = slot as u64;
    let at = slot * FLICK_EVERY + 1. + noise(n ^ 0xf11c) * (FLICK_EVERY - FLICK_LEN - 1.5);
    let u = (time - at) / FLICK_LEN;
    if !(0. ..=1.).contains(&u) {
        return [0., 0.];
    }
    let amount = (u * TAU).sin().powi(2) * if u < 0.5 { 1. } else { 0.55 };
    let pick = noise(n ^ 0xea25);
    [
        if pick < 0.55 { amount } else { 0. },
        if pick >= 0.45 { amount } else { 0. },
    ]
}

/// A repeatable number in 0..1 for `n` (SplitMix64).
fn noise(n: u64) -> f32 {
    let mut x = n.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^= x >> 31;
    (x >> 40) as f32 / (1u64 << 24) as f32
}

fn lerp(a: P, b: P, t: f32) -> P {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

/// Moves `p` straight down onto the brow line if it's above it. Clamping every point of a shape
/// whose outline is one top and one bottom curve cuts it along the line, as a clip would: where a
/// column lies wholly above the line, both its points land on it and it has no width.
fn below_brow(p: P, [a, b]: [P; 2]) -> P {
    let line = a[1] + (b[1] - a[1]) * (p[0] - a[0]) / (b[0] - a[0]);
    [p[0], p[1].max(line)]
}

/// The quadratic curve `[start, control, end]` as the same curve in cubic form.
fn cubic([start, control, end]: [P; 3]) -> [P; 4] {
    [start, lerp(start, control, 2. / 3.), lerp(end, control, 2. / 3.), end]
}

/// Cubic Bézier segments `[start, control, control, end]` through `points` (Catmull-Rom).
fn smooth(points: &[P], closed: bool) -> Vec<[P; 4]> {
    let n = points.len() as isize;
    let at = |i: isize| {
        if closed {
            points[i.rem_euclid(n) as usize]
        } else {
            points[i.clamp(0, n - 1) as usize]
        }
    };
    let segments = if closed { n } else { n - 1 };
    (0..segments)
        .map(|i| {
            let (p0, p1, p2, p3) = (at(i - 1), at(i), at(i + 1), at(i + 2));
            [
                p1,
                [p1[0] + (p2[0] - p0[0]) / 6., p1[1] + (p2[1] - p0[1]) / 6.],
                [p2[0] - (p3[0] - p1[0]) / 6., p2[1] - (p3[1] - p1[1]) / 6.],
                p2,
            ]
        })
        .collect()
}

// ---------------------------------------------------------------- tessellation (once, at startup)

/// The tessellation of every piece of the drawings, in drawing units: built once, on the first
/// mascot, and moved into each pose from there on.
struct Shapes {
    /// The ellipse under the mascot: [`Pose::low`] stretches it sideways and fades it.
    shadow: Part,
    /// The five tentacles behind the head, in [`TENTACLES`]' order.
    tentacles: [Part; TENTACLES.len()],
    /// The confused face's last tentacle, whole: it's drawn behind the head with the others.
    scratching: Part,
    /// Its tip, painted again over the head so it comes out from under it (see
    /// [`SCRATCHING_FRONT`]).
    scratching_front: Part,
    head: Part,
    /// Each ear's outline and the shape inside it, in [`EARS`]' order.
    ears: [Part; EARS.len()],
    inner_ears: [Part; EARS.len()],
    cheeks: [Part; CHEEKS.len()],
    /// The round eyes, and the evil ones with their tops cut by the brows.
    eyes: [Part; EYES.len()],
    evil_eyes: [Part; EYES.len()],
    /// Both lids, as one mesh: the blink drawing's curves lie on the same head.
    lids: Part,
    mouth: Part,
    evil_mouth: Part,
    /// The confused face's question mark: hook and dot.
    question: Part,
}

impl Shapes {
    fn get() -> &'static Self {
        static SHAPES: OnceLock<Shapes> = OnceLock::new();
        SHAPES.get_or_init(Shapes::new)
    }

    fn new() -> Self {
        Shapes {
            shadow: still(painted(ellipse(SHADOW, SHADOW_WIDTH, SHADOW_HEIGHT), Color32::BLACK)),
            tentacles: TENTACLES
                .map(|tentacle: Tentacle| waving(gradient(tentacle_mesh(&tentacle), &tentacle), &tentacle)),
            // Behind the head it's one of the tentacles, gradient and all.
            scratching: waving(gradient(tentacle_mesh(&SCRATCHING), &SCRATCHING), &SCRATCHING),
            // Over the head it's one solid colour, the bottom of the gradient, as in the drawing.
            scratching_front: waving(painted(scratching_front_mesh(), TENTACLE_BOTTOM), &SCRATCHING),
            head: still(painted(filled(&smooth(&HEAD, true), true), BODY)),
            ears: EARS.map(|ear: Ear| flicked(rounded_polygon(ear.up, EAR_ROUNDING), ear.up, ear.flicked, BODY)),
            inner_ears: EARS.map(|ear: Ear| {
                flicked(
                    rounded_polygon(&ear.inner_up, EAR_INNER_ROUNDING),
                    &ear.inner_up,
                    &ear.inner_flicked,
                    EAR_INNER,
                )
            }),
            cheeks: CHEEKS.map(|(center, rx, ry)| {
                // Soft cheeks: stacked translucent ellipses, since paths can't take radial
                // gradients.
                let mut mesh = Mesh::default();
                for layer in 0..6 {
                    let shrink = 1. - layer as f32 * 0.14;
                    mesh.append(painted(ellipse(center, rx * shrink, ry * shrink), BLUSH));
                }
                still(mesh)
            }),
            eyes: EYES.map(|(center, r)| still(painted(ellipse(center, r, r), WHITE))),
            evil_eyes: std::array::from_fn(|eye| {
                let (center, r) = EYES[eye];
                // GPUI had no clip either, so the eye is sampled and its points pushed below the
                // brow; the blink then squashes what's left towards the eye's middle.
                let points: Vec<P> = (0..EYE_SAMPLES)
                    .map(|i| {
                        let angle = i as f32 / EYE_SAMPLES as f32 * TAU;
                        let p = [center[0] + r * angle.cos(), center[1] + r * angle.sin()];
                        below_brow(p, BROWS[eye])
                    })
                    .collect();
                still(painted(polygon(&points), WHITE))
            }),
            lids: still(painted(
                stroked(&CLOSED_EYES.map(cubic), false, CLOSED_EYE_WIDTH),
                WHITE,
            )),
            mouth: still(painted(stroked(&MOUTH.map(cubic), false, MOUTH_WIDTH), WHITE)),
            evil_mouth: still(painted(stroked(&EVIL_MOUTH, false, MOUTH_WIDTH), WHITE)),
            question: still(painted(
                {
                    let mut mesh = stroked(&QUESTION_HOOK, false, QUESTION_WIDTH);
                    let (center, r) = QUESTION_DOT;
                    mesh.append(ellipse(center, r, r));
                    mesh
                },
                QUESTION_MARK,
            )),
        }
    }
}

/// A tessellated piece of a drawing, in drawing units.
struct Part {
    mesh: Mesh,
    /// What the pose does to the mesh's vertices on top of the part's transform.
    motion: Motion,
}

/// The animation that moves a part's vertices beyond the pose's transform.
enum Motion {
    /// Nothing: the pose's transform carries the whole part.
    Still,
    /// A tentacle's travelling wave: each vertex swings this far ([dx, dy]) and trails the wave
    /// by this much of its phase (`lag`).
    Wave(Vec<[f32; 3]>),
    /// An ear's flick: each vertex moves this far ([dx, dy]) when the ear is fully flicked.
    Flick(Vec<[f32; 2]>),
}

fn still(mesh: Mesh) -> Part {
    Part {
        mesh,
        motion: Motion::Still,
    }
}

/// An ear that the flick moves: `mesh` is its rest pose, and its vertices follow `up`'s points to
/// `flicked` (see [`flick_shift`]).
fn flicked(mesh: Mesh, up: &[P], flicked: &[P], color: Color32) -> Part {
    let motion = Motion::Flick(flick_shift(up, flicked, &mesh));
    Part {
        mesh: painted(mesh, color),
        motion,
    }
}

/// A tentacle that the wave moves (see [`wave_shift`]).
fn waving(mesh: Mesh, tentacle: &Tentacle) -> Part {
    let motion = Motion::Wave(wave_shift(tentacle, &mesh));
    Part { mesh, motion }
}

/// A tentacle's stroke with a round tip, in drawing units.
fn tentacle_mesh(tentacle: &Tentacle) -> Mesh {
    let mut mesh = stroked(&smooth(tentacle.points, false), false, TENTACLE_WIDTH);
    mesh.append(tip_ellipse(tentacle.points));
    mesh
}

/// The scratching tentacle again, from the point where it comes out from under the head to its
/// tip, for painting over the head.
fn scratching_front_mesh() -> Mesh {
    let segments = smooth(SCRATCHING.points, false);
    let mut mesh = stroked(&segments[SCRATCHING_FRONT..], false, TENTACLE_WIDTH);
    mesh.append(tip_ellipse(SCRATCHING.points));
    mesh
}

/// The round tip that keeps a tentacle's stroke from ending in a corner.
fn tip_ellipse(points: &[P]) -> Mesh {
    let half = TENTACLE_WIDTH / 2.;
    ellipse(points[points.len() - 1], half, half)
}

/// Colours a tentacle's mesh from the top of its stroke down. The gradient spans the stroke's
/// bounds, so the tip takes the colour at its height there, as `main`'s did.
fn gradient(mut mesh: Mesh, tentacle: &Tentacle) -> Mesh {
    let (top, bottom) = bounds_y(tentacle.points);
    let at = |y: f32| ((y - (top - TENTACLE_WIDTH / 2.)) / (bottom - top + TENTACLE_WIDTH)).clamp(0., 1.);
    for vertex in &mut mesh.vertices {
        vertex.color = mix(TENTACLE_TOP, TENTACLE_BOTTOM, at(vertex.pos.y));
    }
    mesh
}

/// Each vertex's share of the tentacle's wave: the direction and size it swings (the tip most,
/// the root not at all), and how much of the wave's phase it trails by.
fn wave_shift(tentacle: &Tentacle, mesh: &Mesh) -> Vec<[f32; 3]> {
    let points = tentacle.points;
    let last = points.len() - 1;
    mesh.vertices
        .iter()
        .map(|vertex| {
            let (segment, within) = closest(points, [vertex.pos.x, vertex.pos.y]);
            let (a, b) = (points[segment.saturating_sub(1)], points[(segment + 1).min(last)]);
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let length = dx.hypot(dy).max(1.);
            let along = (segment as f32 + within) / last as f32;
            let swing = tentacle.sway * along.powf(1.3);
            [-dy / length * swing, dx / length * swing, along * 2.4 - tentacle.phase]
        })
        .collect()
}

/// Each vertex's share of an ear's flick: the drawing's own points move by the difference
/// between the two poses, and the vertices between them by inverse-distance weighting from those
/// points, which is exact at the points and smooth in between.
fn flick_shift(up: &[P], flicked: &[P], mesh: &Mesh) -> Vec<[f32; 2]> {
    mesh.vertices
        .iter()
        .map(|vertex| {
            let p = [vertex.pos.x, vertex.pos.y];
            let (mut dx, mut dy, mut weight) = (0., 0., 0.);
            for (&a, &b) in up.iter().zip(flicked) {
                let distance = (p[0] - a[0]).powi(2) + (p[1] - a[1]).powi(2);
                let share = 1. / distance.max(1e-3);
                dx += share * (b[0] - a[0]);
                dy += share * (b[1] - a[1]);
                weight += share;
            }
            [dx / weight, dy / weight]
        })
        .collect()
}

/// The segment of `points` nearest to `p`, and how far along it `p` lies (0 at its start, 1 at
/// its end).
fn closest(points: &[P], p: P) -> (usize, f32) {
    let mut best = (0, 0.);
    let mut nearest = f32::MAX;
    for i in 0..points.len().saturating_sub(1) {
        let (a, b) = (points[i], points[i + 1]);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let length = (dx * dx + dy * dy).max(1e-6);
        let along = (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / length).clamp(0., 1.);
        let q = [a[0] + dx * along, a[1] + dy * along];
        let distance = (p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2);
        if distance < nearest {
            nearest = distance;
            best = (i, along);
        }
    }
    best
}

// ----------------------------------------------------------------------------- one frame of pose

/// One part as a frame draws it.
struct Drawn<'a> {
    part: &'a Part,
    /// Where the pose puts it, in drawing units.
    pose: Affine,
    /// The wave's phase for a tentacle, or how far an ear is through its flick: what the part's
    /// own motion needs.
    motion: f32,
    /// A colour for the whole part, for the shadow, whose alpha follows the bob.
    tint: Option<Color32>,
}

/// Everything `face` shows in this pose, back to front, as `main` painted it.
fn frame<'a>(shapes: &'a Shapes, pose: &Pose, face: Face) -> Vec<Drawn<'a>> {
    let body = pose.body();
    let bob = Affine::translate([0., pose.bob]);
    let wave = pose.phase.unwrap_or(0.);
    let mut parts = Vec::with_capacity(17);

    let low = pose.low;
    parts.push(Drawn {
        part: &shapes.shadow,
        pose: Affine::scale(SHADOW, [(SHADOW_WIDTH + SHADOW_GROWTH * low) / SHADOW_WIDTH, 1.]),
        motion: 0.,
        tint: Some(Color32::from_black_alpha(((0.08 + 0.1 * low) * 255.).round() as u8)),
    });
    for (i, tentacle) in shapes.tentacles.iter().enumerate() {
        if face == Face::Confused && i == TENTACLES.len() - 1 {
            continue;
        }
        parts.push(Drawn {
            part: tentacle,
            pose: bob,
            motion: wave,
            tint: None,
        });
    }
    if face == Face::Confused {
        parts.push(Drawn {
            part: &shapes.scratching,
            pose: bob,
            motion: wave,
            tint: None,
        });
    }
    parts.push(Drawn {
        part: &shapes.head,
        pose: body,
        motion: 0.,
        tint: None,
    });
    for (ear, (outer, inner)) in shapes.ears.iter().zip(&shapes.inner_ears).enumerate() {
        for part in [outer, inner] {
            parts.push(Drawn {
                part,
                pose: body,
                motion: pose.ears[ear],
                tint: None,
            });
        }
    }
    for cheek in &shapes.cheeks {
        parts.push(Drawn {
            part: cheek,
            pose: body,
            motion: 0.,
            tint: None,
        });
    }
    let eyes = if face == Face::Evil {
        &shapes.evil_eyes
    } else {
        &shapes.eyes
    };
    match pose.eyes {
        Eyes::Closed => parts.push(Drawn {
            part: &shapes.lids,
            pose: body,
            motion: 0.,
            tint: None,
        }),
        Eyes::Open(open) => {
            for (part, (center, _)) in eyes.iter().zip(EYES) {
                parts.push(Drawn {
                    part,
                    // The lid comes down: the eye squashes towards its own middle, as `main`'s
                    // clamping of the evil eye's points did.
                    pose: body.then(Affine::scale(center, [1., open])),
                    motion: 0.,
                    tint: None,
                });
            }
        }
    }
    parts.push(Drawn {
        part: if face == Face::Evil {
            &shapes.evil_mouth
        } else {
            &shapes.mouth
        },
        pose: body,
        motion: 0.,
        tint: None,
    });
    if face == Face::Confused {
        parts.push(Drawn {
            part: &shapes.scratching_front,
            pose: bob,
            motion: wave,
            tint: None,
        });
        parts.push(Drawn {
            part: &shapes.question,
            pose: body.then(Affine::translate([0., pose.float])),
            motion: 0.,
            tint: None,
        });
    }
    parts
}

/// Paints the mascot into `rect`, its square face filling it.
fn paint(ui: &Ui, shapes: &Shapes, pose: &Pose, face: Face, rect: Rect) {
    let view = view(rect);
    let painter = ui.painter();
    for drawn in frame(shapes, pose, face) {
        let transform = drawn.pose.then(view);
        let mut mesh = drawn.part.mesh.clone();
        for (i, vertex) in mesh.vertices.iter_mut().enumerate() {
            let [x, y] = [vertex.pos.x, vertex.pos.y];
            let [x, y] = match &drawn.part.motion {
                Motion::Still => [x, y],
                Motion::Wave(shifts) => {
                    let [dx, dy, lag] = shifts[i];
                    let swing = (drawn.motion - lag).sin();
                    [x + dx * swing, y + dy * swing]
                }
                Motion::Flick(shifts) => {
                    let [dx, dy] = shifts[i];
                    [x + dx * drawn.motion, y + dy * drawn.motion]
                }
            };
            vertex.pos = transform.pos([x, y]);
            if let Some(tint) = drawn.tint {
                vertex.color = tint;
            }
        }
        painter.add(egui::Shape::mesh(mesh));
    }
}

/// The drawings' space to the `rect`'s: the frame's square centred in it.
fn view(rect: Rect) -> Affine {
    let side = rect.width().min(rect.height());
    let scale = side / FRAME_SIZE;
    let origin = rect.min.to_vec2() + Vec2::new((rect.width() - side) / 2., (rect.height() - side) / 2.)
        - Vec2::new(FRAME_ORIGIN[0], FRAME_ORIGIN[1]) * scale;
    Affine {
        a: scale,
        b: 0.,
        c: origin.x,
        d: 0.,
        e: scale,
        f: origin.y,
    }
}

/// An affine map of the drawings' space: `x' = a x + b y + c`, `y' = d x + e y + f`.
#[derive(Clone, Copy, Debug)]
struct Affine {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    e: f32,
    f: f32,
}

impl Affine {
    /// Scales `x` by `scale[0]` and `y` by `scale[1]`, around `center`.
    fn scale(center: P, [x, y]: [f32; 2]) -> Self {
        Affine {
            a: x,
            b: 0.,
            c: center[0] * (1. - x),
            d: 0.,
            e: y,
            f: center[1] * (1. - y),
        }
    }

    fn translate([x, y]: P) -> Self {
        Affine {
            a: 1.,
            b: 0.,
            c: x,
            d: 0.,
            e: 1.,
            f: y,
        }
    }

    /// This map, then `next`.
    fn then(self, next: Self) -> Self {
        Affine {
            a: next.a * self.a + next.b * self.d,
            b: next.a * self.b + next.b * self.e,
            c: next.a * self.c + next.b * self.f + next.c,
            d: next.d * self.a + next.e * self.d,
            e: next.d * self.b + next.e * self.e,
            f: next.d * self.c + next.e * self.f + next.f,
        }
    }

    fn pos(self, [x, y]: P) -> Pos2 {
        pos2(self.a * x + self.b * y + self.c, self.d * x + self.e * y + self.f)
    }
}

// ------------------------------------------------------------------------------------ the paths

/// A path through `segments`, starting at the first one's start, closed if asked.
fn path(segments: &[[P; 4]], closed: bool) -> Path {
    let mut builder = Path::builder();
    builder.begin(point(segments[0][0][0], segments[0][0][1]));
    for [_, a, b, end] in segments {
        builder.cubic_bezier_to(point(a[0], a[1]), point(b[0], b[1]), point(end[0], end[1]));
    }
    builder.end(closed);
    builder.build()
}

/// The closed outline `segments` as a filled mesh.
fn filled(segments: &[[P; 4]], closed: bool) -> Mesh {
    let mut tessellator = FillTessellator::new();
    let mut buffers = VertexBuffers::new();
    let mut builder = BuffersBuilder::new(&mut buffers, |vertex: FillVertex| untextured(vertex.position()));
    tessellator
        .tessellate_path(
            &path(segments, closed),
            &FillOptions::default().with_tolerance(TOLERANCE),
            &mut builder,
        )
        .expect("the mascot's outlines are simple paths");
    mesh(buffers)
}

/// `segments` stroked `width` wide, with round ends and joins: GPUI's strokes had butt ends and
/// mitred joins, which `main` rounded by stroking each curve and its ends separately.
fn stroked(segments: &[[P; 4]], closed: bool, width: f32) -> Mesh {
    let mut tessellator = StrokeTessellator::new();
    let mut buffers = VertexBuffers::new();
    let mut builder = BuffersBuilder::new(&mut buffers, |vertex: StrokeVertex| untextured(vertex.position()));
    let options = StrokeOptions::default()
        .with_line_width(width)
        .with_tolerance(TOLERANCE)
        .with_line_cap(LineCap::Round)
        .with_line_join(LineJoin::Round);
    tessellator
        .tessellate_path(&path(segments, closed), &options, &mut builder)
        .expect("the mascot's strokes are simple paths");
    mesh(buffers)
}

/// The edges of `points` as straight cubic segments.
fn edges(points: &[P], closed: bool) -> Vec<[P; 4]> {
    let count = if closed { points.len() } else { points.len() - 1 };
    (0..count)
        .map(|i| {
            let (a, b) = (points[i], points[(i + 1) % points.len()]);
            [a, lerp(a, b, 1. / 3.), lerp(a, b, 2. / 3.), b]
        })
        .collect()
}

fn polygon(points: &[P]) -> Mesh {
    filled(&edges(points, true), true)
}

/// A filled polygon whose corners are rounded by stroking its edges `rounding` wide.
fn rounded_polygon(points: &[P], rounding: f32) -> Mesh {
    let mut mesh = polygon(points);
    mesh.append(stroked(&edges(points, true), true, rounding));
    mesh
}

/// An ellipse as four cubic quarter arcs; 0.5523 puts the control points where they best fit a
/// circle.
fn ellipse(center: P, rx: f32, ry: f32) -> Mesh {
    const K: f32 = 0.5523;
    let [x, y] = center;
    let (kx, ky) = (rx * K, ry * K);
    let segments = [
        [[x + rx, y], [x + rx, y + ky], [x + kx, y + ry], [x, y + ry]],
        [[x, y + ry], [x - kx, y + ry], [x - rx, y + ky], [x - rx, y]],
        [[x - rx, y], [x - rx, y - ky], [x - kx, y - ry], [x, y - ry]],
        [[x, y - ry], [x + kx, y - ry], [x + rx, y - ky], [x + rx, y]],
    ];
    filled(&segments, true)
}

fn untextured(position: lyon_tessellation::math::Point) -> Vertex {
    Vertex {
        pos: pos2(position.x, position.y),
        uv: WHITE_UV,
        color: WHITE,
    }
}

fn mesh(buffers: VertexBuffers<Vertex, u32>) -> Mesh {
    Mesh {
        indices: buffers.indices,
        vertices: buffers.vertices,
        texture_id: Default::default(),
    }
}

fn painted(mut mesh: Mesh, color: Color32) -> Mesh {
    for vertex in &mut mesh.vertices {
        vertex.color = color;
    }
    mesh
}

fn mix(from: Color32, to: Color32, t: f32) -> Color32 {
    let channel = |from: u8, to: u8| (f32::from(from) + (f32::from(to) - f32::from(from)) * t).round() as u8;
    Color32::from_rgb(
        channel(from.r(), to.r()),
        channel(from.g(), to.g()),
        channel(from.b(), to.b()),
    )
}

fn bounds_y(points: &[P]) -> (f32, f32) {
    points
        .iter()
        .fold((f32::MAX, f32::MIN), |(low, high), p| (low.min(p[1]), high.max(p[1])))
}

#[cfg(test)]
mod tests {
    use super::{
        BROWS, EVIL_ODDS, EYES, Eyes, FLICK_EVERY, Face, HEAD, SCRATCHING, SCRATCHING_FRONT, Shapes, TENTACLE_WIDTH,
        below_brow, blink, cubic, flicks, frame, kept_or_picked, noise, smooth,
    };

    #[test]
    fn noise_stays_in_range() {
        for n in 0..10_000 {
            assert!((0. ..1.).contains(&noise(n)));
        }
    }

    #[test]
    fn blinks_once_per_slot() {
        // Sampled every 10 ms, each 3.6 s slot has one closed stretch.
        let closed_starts = (0..3600)
            .map(|i| matches!(blink(i as f32 * 0.01), Eyes::Closed))
            .collect::<Vec<_>>()
            .windows(2)
            .filter(|w| !w[0] && w[1])
            .count();
        assert_eq!(closed_starts, 10);
    }

    #[test]
    fn flicks_settle_back_to_rest() {
        let peak = (0..(FLICK_EVERY * 1000.) as usize)
            .map(|i| flicks(i as f32 * 0.001))
            .fold(0f32, |m, [l, r]| m.max(l).max(r));
        assert!(peak > 0.9 && peak <= 1.);
        assert_eq!(flicks(0.), [0., 0.]);
    }

    #[test]
    fn brow_clamp_keeps_points_below_the_line() {
        for ((center, r), brow) in EYES.into_iter().zip(BROWS) {
            let [a, b] = brow;
            let line = |x: f32| a[1] + (b[1] - a[1]) * (x - a[0]) / (b[0] - a[0]);
            for i in 0..360 {
                let angle = (i as f32).to_radians();
                let p = [center[0] + r * angle.cos(), center[1] + r * angle.sin()];
                let q = below_brow(p, brow);
                assert_eq!(q[0], p[0], "only moves straight down");
                assert!(q[1] >= line(q[0]) - 1e-3, "{q:?} is above {brow:?}");
                if p[1] >= line(p[0]) {
                    assert_eq!(q, p, "a point already below the brow moved");
                }
            }
        }
    }

    #[test]
    fn loader_keeps_its_face() {
        for face in [Face::Cute, Face::Evil] {
            let kept = kept_or_picked(Some(face), || panic!("picked again"));
            assert_eq!(kept, face);
        }
        assert_eq!(kept_or_picked(None, || 0.), Face::Evil);
        assert_eq!(kept_or_picked(None, || EVIL_ODDS), Face::Cute);
        assert_eq!(kept_or_picked(None, || 0.999), Face::Cute);
    }

    #[test]
    fn scratching_front_starts_below_the_head() {
        let head_bottom = HEAD.iter().fold(f32::MIN, |m, p| m.max(p[1]));
        let seam = SCRATCHING.points[SCRATCHING_FRONT];
        assert!(
            seam[1] - TENTACLE_WIDTH / 2. > head_bottom,
            "{seam:?} would show a seam on the head"
        );
    }

    #[test]
    fn cubic_keeps_the_quadratic_ends() {
        let [start, a, b, end] = cubic([[0., 0.], [30., 60.], [60., 0.]]);
        assert_eq!((start, end), ([0., 0.], [60., 0.]));
        assert_eq!((a, b), ([20., 40.], [40., 40.]));
    }

    #[test]
    fn smooth_passes_through_points() {
        let points = [[0., 0.], [10., 5.], [20., 0.], [30., 5.]];
        let open = smooth(&points, false);
        assert_eq!(open.len(), 3);
        assert_eq!(open[1][0], [10., 5.]);
        assert_eq!(open[2][3], [30., 5.]);
        assert_eq!(smooth(&points, true).len(), 4);
    }

    #[test]
    fn every_face_tessellates() {
        let shapes = Shapes::get();
        let pose = super::Pose::at(0.3);
        for face in [Face::Cute, Face::Evil, Face::Confused] {
            let parts = frame(shapes, &pose, face);
            assert!(parts.len() > 10, "{face:?} draws only {} parts", parts.len());
            for drawn in &parts {
                let mesh = &drawn.part.mesh;
                assert!(
                    !mesh.vertices.is_empty() && !mesh.indices.is_empty(),
                    "{face:?} draws an empty mesh"
                );
            }
        }
        // Each face has its own pieces: the confused one scratches its head and asks, while the
        // others don't.
        let confused = frame(shapes, &pose, Face::Confused);
        assert!(confused.iter().any(|drawn| std::ptr::eq(drawn.part, &shapes.question)));
        assert!(
            confused
                .iter()
                .any(|drawn| std::ptr::eq(drawn.part, &shapes.scratching_front))
        );
        for face in [Face::Cute, Face::Evil] {
            let parts = frame(shapes, &pose, face);
            assert!(!parts.iter().any(|drawn| std::ptr::eq(drawn.part, &shapes.question)));
            assert!(!parts.iter().any(|drawn| std::ptr::eq(drawn.part, &shapes.scratching)));
        }
    }
}
