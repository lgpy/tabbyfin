//! Mewdusa, the jellycat from the app icon, drawn as the loading indicator.
//!
//! Traced from the 2000×2000 drawings in `packaging/mascot/` and painted as paths every frame:
//! the head pulses and bobs, the tentacles wave, and now and then it blinks or flicks an ear.
//! With animations off it holds still in its rest pose.
//!
//! It has three faces, each from its own drawing. A loader shows the cute or the evil one (eyes
//! cut flat by slanted brows, an "ω" mouth), picked at random when it appears. Empty pages show
//! the confused one, scratching its head under a question mark.
//!
//! Blinks and flicks come from a hash of the time slot rather than stored state, so a mascot can
//! be built fresh on every render and still never repeat in lockstep.

use std::collections::hash_map::RandomState;
use std::f32::consts::TAU;
use std::hash::{BuildHasher, Hasher};
use std::sync::OnceLock;
use std::time::Instant;

use gpui_kit::*;

/// A point in the drawings' 2000×2000 space.
type P = [f32; 2];

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

const BODY: u32 = 0xc396e7;
const EAR_INNER: u32 = 0x8a63a9;
const TENTACLE_TOP: u32 = 0xbd97e7;
const TENTACLE_BOTTOM: u32 = 0x9a8ee9;
/// The blush's colour with a low alpha; it's stacked to fade out from the middle.
const BLUSH: u32 = 0xe08ce022;
const WHITE: u32 = 0xffffff;
const QUESTION_MARK: u32 = 0xdd7a93;

const TENTACLE_WIDTH: f32 = 134.;
const EAR_ROUNDING: f32 = 40.;
const EAR_INNER_ROUNDING: f32 = 30.;
const MOUTH_WIDTH: f32 = 26.;
const CLOSED_EYE_WIDTH: f32 = 34.;
const QUESTION_WIDTH: f32 = 72.;

/// How often a loader shows the evil face rather than the cute one.
const EVIL_ODDS: f32 = 0.5;

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

/// The mascot as a loader in a `size` square, animating unless animations are off. Its face is
/// picked when it appears and kept while it's showing.
pub fn mascot(size: Rems) -> impl IntoElement {
    mascot_with(size, |window| {
        // Element state lives as long as the element is drawn in consecutive frames, so a
        // loader that goes away and comes back picks again.
        window.with_global_id("mascot-face".into(), |id, window| {
            window.with_element_state(id, |kept: Option<Face>, _| {
                let face = kept_or_picked(kept, || noise(RandomState::new().build_hasher().finish()));
                (face, face)
            })
        })
    })
}

/// The confused mascot, for a page with nothing on it.
pub fn confused_mascot(size: Rems) -> impl IntoElement {
    mascot_with(size, |_| Face::Confused)
}

/// The face kept from the last frame, or else a new pick from `roll`.
fn kept_or_picked(kept: Option<Face>, roll: impl FnOnce() -> f32) -> Face {
    kept.unwrap_or_else(|| Face::for_loader(roll()))
}

fn mascot_with(size: Rems, face: impl 'static + FnOnce(&mut Window) -> Face) -> impl IntoElement {
    canvas(
        move |_, window, _| face(window),
        |bounds, face, window, cx| {
            let still = cx.reduce_motion();
            let pose = if still {
                Pose::rest(face)
            } else {
                Pose::at(clock(), face)
            };
            Painter::new(bounds, window).paint(&pose, face);
            if !still {
                window.request_animation_frame();
            }
        },
    )
    .size(size)
    .flex_none()
}

/// Seconds since the first mascot was drawn. Shared, so a mascot rebuilt on the next render
/// carries on where it was.
fn clock() -> f32 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f32()
}

#[derive(Clone, Copy)]
enum Eyes {
    /// Open, squashed vertically to this fraction while blinking.
    Open(f32),
    Closed,
}

struct Pose {
    /// Vertical offset of everything but the shadow.
    bob: f32,
    /// The head's horizontal and vertical scale around [`BODY_CENTER`].
    scale: [f32; 2],
    tentacles: [Vec<P>; 5],
    /// How far each ear is toward its flicked pose, 0 to 1.
    ears: [f32; 2],
    eyes: Eyes,
    /// 0 at the top of the bob, 1 at the bottom.
    low: f32,
    /// The confused face's question mark's offset from its place, on top of the bob.
    float: f32,
}

impl Pose {
    fn rest(face: Face) -> Self {
        Pose {
            bob: 0.,
            scale: [1., 1.],
            tentacles: std::array::from_fn(|i| tentacle(face, i).points.to_vec()),
            ears: [0., 0.],
            eyes: Eyes::Open(1.),
            low: 0.5,
            float: 0.,
        }
    }

    fn at(time: f32, face: Face) -> Self {
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
            tentacles: std::array::from_fn(|i| wave(tentacle(face, i), phase)),
            ears: [0, 1].map(|i| (flicks[i] + droop).min(1.)),
            eyes: blink(time),
            low: (bob + BOB) / (2. * BOB),
            float: QUESTION_FLOAT * (phase - 1.6).sin(),
        }
    }

    fn body(&self, p: P) -> P {
        [
            BODY_CENTER[0] + (p[0] - BODY_CENTER[0]) * self.scale[0],
            BODY_CENTER[1] + (p[1] - BODY_CENTER[1]) * self.scale[1] + self.bob,
        ]
    }
}

/// Tentacle `i` of `face`: the confused face scratches its head with the last one.
fn tentacle(face: Face, i: usize) -> &'static Tentacle {
    match face {
        Face::Confused if i == TENTACLES.len() - 1 => &SCRATCHING,
        _ => &TENTACLES[i],
    }
}

/// A travelling wave along the tentacle, pushing each point sideways; the tip moves most.
fn wave(tentacle: &Tentacle, phase: f32) -> Vec<P> {
    let points = tentacle.points;
    let last = points.len() - 1;
    (0..=last)
        .map(|i| {
            let (a, b) = (points[i.saturating_sub(1)], points[(i + 1).min(last)]);
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let len = dx.hypot(dy).max(1.);
            let along = i as f32 / last as f32;
            let offset = tentacle.sway * along.powf(1.3) * (phase - along * 2.4 + tentacle.phase).sin();
            [points[i][0] - dy / len * offset, points[i][1] + dx / len * offset]
        })
        .collect()
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

/// Paints in drawing units, mapped into the element's bounds.
struct Painter<'a> {
    origin: Point<Pixels>,
    scale: f32,
    window: &'a mut Window,
}

impl<'a> Painter<'a> {
    fn new(bounds: Bounds<Pixels>, window: &'a mut Window) -> Self {
        let side = bounds.size.width.min(bounds.size.height);
        let origin = bounds.origin
            + point(
                (bounds.size.width - side) / 2. - side * (FRAME_ORIGIN[0] / FRAME_SIZE),
                (bounds.size.height - side) / 2. - side * (FRAME_ORIGIN[1] / FRAME_SIZE),
            );
        Painter {
            origin,
            scale: f32::from(side) / FRAME_SIZE,
            window,
        }
    }

    fn paint(&mut self, pose: &Pose, face: Face) {
        let shadow = hsla(0., 0., 0., 0.08 + 0.1 * pose.low);
        self.ellipse(SHADOW, 500. + 140. * pose.low, 30., shadow);

        for points in &pose.tentacles {
            let points: Vec<P> = points.iter().map(|p| [p[0], p[1] + pose.bob]).collect();
            self.tentacle(&points);
        }

        let head: Vec<P> = HEAD.iter().map(|&p| pose.body(p)).collect();
        let mut path = PathBuilder::fill();
        self.trace(&mut path, &smooth(&head, true));
        path.close();
        self.fill(path, rgb(BODY));

        for (ear, &flick) in EARS.iter().zip(&pose.ears) {
            let outer: Vec<P> = ear
                .up
                .iter()
                .zip(ear.flicked)
                .map(|(&a, &b)| pose.body(lerp(a, b, flick)))
                .collect();
            self.rounded_polygon(&outer, EAR_ROUNDING, rgb(BODY));
            let inner: Vec<P> = ear
                .inner_up
                .iter()
                .zip(&ear.inner_flicked)
                .map(|(&a, &b)| pose.body(lerp(a, b, flick)))
                .collect();
            self.rounded_polygon(&inner, EAR_INNER_ROUNDING, rgb(EAR_INNER));
        }

        // Soft cheeks: stacked translucent ellipses, since paths can't take radial gradients.
        for (center, rx, ry) in CHEEKS {
            for layer in 0..6 {
                let shrink = 1. - layer as f32 * 0.14;
                self.ellipse(pose.body(center), rx * shrink, ry * shrink, rgba(BLUSH));
            }
        }

        match (pose.eyes, face) {
            (Eyes::Open(open), Face::Cute | Face::Confused) => {
                for (center, r) in EYES {
                    self.ellipse(
                        pose.body(center),
                        r * pose.scale[0],
                        r * open * pose.scale[1],
                        rgb(WHITE),
                    );
                }
            }
            // GPUI paths have no clip, so the eye is sampled and its points pushed below the brow.
            // The brow stays put while the eye squashes, as the lid comes down behind it.
            (Eyes::Open(open), Face::Evil) => {
                for ((center, r), brow) in EYES.into_iter().zip(BROWS) {
                    let points: Vec<P> = (0..EYE_SAMPLES)
                        .map(|i| {
                            let angle = i as f32 / EYE_SAMPLES as f32 * TAU;
                            let p = [center[0] + r * angle.cos(), center[1] + r * open * angle.sin()];
                            pose.body(below_brow(p, brow))
                        })
                        .collect();
                    self.polygon(&points, rgb(WHITE));
                }
            }
            (Eyes::Closed, _) => {
                let lids = CLOSED_EYES.map(|curve| cubic(curve).map(|p| pose.body(p)));
                self.round_strokes(&lids, CLOSED_EYE_WIDTH, rgb(WHITE));
            }
        }
        let mouth = match face {
            Face::Cute | Face::Confused => MOUTH.map(cubic).to_vec(),
            Face::Evil => EVIL_MOUTH.to_vec(),
        };
        let mouth: Vec<[P; 4]> = mouth.iter().map(|curve| curve.map(|p| pose.body(p))).collect();
        self.round_strokes(&mouth, MOUTH_WIDTH, rgb(WHITE));

        if face == Face::Confused {
            self.scratching(pose);
            let float = |p: P| {
                let [x, y] = pose.body(p);
                [x, y + pose.float]
            };
            let hook = QUESTION_HOOK.map(|curve| curve.map(float));
            self.round_strokes(&hook, QUESTION_WIDTH, rgb(QUESTION_MARK));
            let (center, r) = QUESTION_DOT;
            self.ellipse(float(center), r, r, rgb(QUESTION_MARK));
        }
    }

    /// The scratching tentacle's raised end, painted again in front of the head. It was painted
    /// whole behind the head with the others, so it still comes out from under it. In front it's
    /// one solid colour, the bottom of the gradient, as in the drawing.
    fn scratching(&mut self, pose: &Pose) {
        let points: Vec<P> = pose.tentacles[TENTACLES.len() - 1]
            .iter()
            .map(|p| [p[0], p[1] + pose.bob])
            .collect();
        let segments = smooth(&points, false);
        let mut path = PathBuilder::stroke(self.len(TENTACLE_WIDTH));
        self.trace(&mut path, &segments[SCRATCHING_FRONT..]);
        self.fill(path, rgb(TENTACLE_BOTTOM));
        let half = TENTACLE_WIDTH / 2.;
        self.ellipse(points[points.len() - 1], half, half, rgb(TENTACLE_BOTTOM));
    }

    fn point(&self, p: P) -> Point<Pixels> {
        self.origin + point(px(p[0] * self.scale), px(p[1] * self.scale))
    }

    fn len(&self, length: f32) -> Pixels {
        px(length * self.scale)
    }

    fn fill(&mut self, path: PathBuilder, color: impl Into<Background>) {
        if let Ok(path) = path.build() {
            self.window.paint_path(path, color);
        }
    }

    /// Adds `segments` to `path`, starting with a move to the first one's start.
    fn trace(&self, path: &mut PathBuilder, segments: &[[P; 4]]) {
        path.move_to(self.point(segments[0][0]));
        for [_, a, b, end] in segments {
            path.cubic_bezier_to(self.point(*end), self.point(*a), self.point(*b));
        }
    }

    fn ellipse(&mut self, center: P, rx: f32, ry: f32, color: impl Into<Background>) {
        // Four cubic quarter arcs; 0.5523 puts the control points where they best fit a circle.
        const K: f32 = 0.5523;
        let [x, y] = center;
        let (kx, ky) = (rx * K, ry * K);
        let segments = [
            [[x + rx, y], [x + rx, y + ky], [x + kx, y + ry], [x, y + ry]],
            [[x, y + ry], [x - kx, y + ry], [x - rx, y + ky], [x - rx, y]],
            [[x - rx, y], [x - rx, y - ky], [x - kx, y - ry], [x, y - ry]],
            [[x, y - ry], [x + kx, y - ry], [x + rx, y - ky], [x + rx, y]],
        ];
        let mut path = PathBuilder::fill();
        self.trace(&mut path, &segments);
        path.close();
        self.fill(path, color);
    }

    /// Strokes each cubic curve `[start, control, control, end]` on its own with round ends,
    /// which also rounds the joins between them; GPUI's strokes have butt ends and mitred joins,
    /// which spike at a cusp like the middle of the evil mouth.
    fn round_strokes(&mut self, curves: &[[P; 4]], width: f32, color: Rgba) {
        for curve in curves {
            let mut path = PathBuilder::stroke(self.len(width));
            self.trace(&mut path, &[*curve]);
            self.fill(path, color);
        }
        for &[start, _, _, end] in curves {
            self.ellipse(start, width / 2., width / 2., color);
            self.ellipse(end, width / 2., width / 2., color);
        }
    }

    fn polygon(&mut self, points: &[P], color: Rgba) {
        let mut path = PathBuilder::fill();
        path.move_to(self.point(points[0]));
        for &p in &points[1..] {
            path.line_to(self.point(p));
        }
        path.close();
        self.fill(path, color);
    }

    /// A filled polygon whose corners are rounded by stroking its edges `rounding` wide.
    fn rounded_polygon(&mut self, points: &[P], rounding: f32, color: Rgba) {
        self.polygon(points, color);
        let edges: Vec<[P; 4]> = (0..points.len())
            .map(|i| {
                let (a, b) = (points[i], points[(i + 1) % points.len()]);
                [a, lerp(a, b, 1. / 3.), lerp(a, b, 2. / 3.), b]
            })
            .collect();
        self.round_strokes(&edges, rounding, color);
    }

    /// A tentacle with a top-to-bottom gradient and a round tip. The root is under the head.
    fn tentacle(&mut self, points: &[P]) {
        let segments = smooth(points, false);
        let mut path = PathBuilder::stroke(self.len(TENTACLE_WIDTH));
        self.trace(&mut path, &segments);
        let gradient = linear_gradient(
            180.,
            linear_color_stop(rgb(TENTACLE_TOP), 0.),
            linear_color_stop(rgb(TENTACLE_BOTTOM), 1.),
        );
        self.fill(path, gradient);

        // The gradient spans the stroke's bounds, so the tip takes the colour at its height there.
        let (top, bottom) = points
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p[1]), hi.max(p[1])));
        let half = TENTACLE_WIDTH / 2.;
        let tip = points[points.len() - 1];
        let t = ((tip[1] - (top - half)) / (bottom - top + TENTACLE_WIDTH)).clamp(0., 1.);
        let (from, to) = (rgb(TENTACLE_TOP), rgb(TENTACLE_BOTTOM));
        let color = Rgba {
            r: from.r + (to.r - from.r) * t,
            g: from.g + (to.g - from.g) * t,
            b: from.b + (to.b - from.b) * t,
            a: 1.,
        };
        self.ellipse(tip, half, half, color);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BROWS, EVIL_ODDS, EYES, Eyes, FLICK_EVERY, Face, HEAD, SCRATCHING, SCRATCHING_FRONT, TENTACLE_WIDTH,
        below_brow, blink, cubic, flicks, kept_or_picked, noise, smooth,
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
}
