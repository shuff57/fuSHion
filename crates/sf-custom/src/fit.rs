//! Fit checks in the plane: where a brick pushed onto a part meets the part's studs.
//!
//! A brick pushed on reaches the studs with its walls (a ring between its outline, pulled in
//! by the clearance, and the hollow) and its tubes (rings at the inner corners of its stud
//! grid). Its centre ribs start [`RIB_GAP`](crate::RIB_GAP) above its floor, above the studs,
//! so they never meet them. Each stud's top outline (a peanut less its slots, or a circle) is
//! sampled every few microns and every sample's signed distance to that material is taken:
//! positive is clearance, negative is overlap. Samples on a Slant v2 fin (over its pocket,
//! past the slot's end) are counted apart from the rest: a fin may press into a wall or a
//! tube, the rest may not. The deepest line of every wall and tube is also tested for lying
//! inside a stud, which catches a wall that crosses a stud outright.

use crate::geom::{self, Frame, Loop, Seg};
use crate::profile::{StudProfile, StudStyle};
use crate::shape::{CustomShape, P2};
use crate::slant::{self, Lobe};
use crate::{CustomError, Result};
use std::collections::BTreeSet;
use std::f64::consts::TAU;

/// Samples per outline segment.
const SAMPLES: usize = 6000;

/// A brick pushed onto a part: `w × d` studs, its stud cell `(0, 0)` over the part's cell
/// `at`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TopBrick {
    /// Studs along X.
    pub w: u32,
    /// Studs along Y.
    pub d: u32,
    /// The part's cell under the brick's cell `(0, 0)`.
    pub at: (i32, i32),
}

/// What a brick pushed on meets (mm; clearances are positive, overlaps negative).
#[derive(Clone, Debug, PartialEq)]
pub struct FitReport {
    /// Least clearance between a tube and any stud material that is not a fin.
    pub body_tube_clearance: f64,
    /// Least clearance between a wall and any stud material that is not a fin.
    pub body_wall_clearance: f64,
    /// The feature behind the least of the two.
    pub body_against: String,
    /// Deepest a fin presses into a wall (0 when none touches).
    pub fin_wall_preload: f64,
    /// Deepest a fin presses into a tube (0 when none touches).
    pub fin_tube_preload: f64,
    /// Least clearance of any fin (negative: its preload).
    pub fin_clearance: f64,
    /// Least clearance between a round stud and a tube (`None` without round studs or tubes).
    pub round_tube_clearance: Option<f64>,
    /// Walls or tubes whose middle lies inside a stud (a wall across a waist, say).
    pub crossings: Vec<String>,
}

impl FitReport {
    /// Least clearance of anything that is not a fin.
    pub fn body_clearance(&self) -> f64 {
        self.body_tube_clearance.min(self.body_wall_clearance)
    }
}

/// One stud's top: its outline samples (with whether each is on a fin) and an inside test.
struct Stud {
    samples: Vec<(P2, bool)>,
    region: Region,
}

enum Region {
    Round {
        c: P2,
        r: f64,
    },
    Peanut {
        peanut: Loop,
        slots: [Loop; 2],
        lobes: [Frame; 2],
    },
}

/// A frame's inverse map (frames here are orthonormal).
fn local(f: &Frame, p: P2) -> P2 {
    let q = [p[0] - f.o[0], p[1] - f.o[1]];
    [q[0] * f.x[0] + q[1] * f.x[1], q[0] * f.y[0] + q[1] * f.y[1]]
}

fn sample(l: &[Seg], keep: impl Fn(P2) -> bool, out: &mut Vec<P2>) {
    for s in l {
        for k in 0..SAMPLES {
            let p = s.at(k as f64 / SAMPLES as f64);
            if keep(p) {
                out.push(p);
            }
        }
    }
}

impl Stud {
    fn round(c: P2, r: f64) -> Self {
        let n = 4 * SAMPLES;
        let samples = (0..n)
            .map(|k| {
                let t = TAU * k as f64 / n as f64;
                ([c[0] + r * t.cos(), c[1] + r * t.sin()], false)
            })
            .collect();
        Self {
            samples,
            region: Region::Round { c, r },
        }
    }

    fn peanut(lobe: &Lobe, q: &slant::Peanut, pitch: f64) -> Self {
        let peanut = q.frame(pitch).map_loop(&lobe.peanut);
        let lobes = q.lobe_frames(pitch);
        let slots = [lobes[0].map_loop(&lobe.slot), lobes[1].map_loop(&lobe.slot)];
        let in_slot = |p: P2| slots.iter().any(|s| geom::contains(s, p));
        let mut pts = Vec::new();
        sample(&peanut, |p| !in_slot(p), &mut pts);
        for (k, s) in slots.iter().enumerate() {
            sample(
                s,
                |p| geom::contains(&peanut, p) && !geom::contains(&slots[1 - k], p),
                &mut pts,
            );
        }
        let region = Region::Peanut {
            peanut,
            slots,
            lobes,
        };
        let samples = pts
            .into_iter()
            .map(|p| (p, region.on_fin(lobe, p)))
            .collect();
        Self { samples, region }
    }
}

impl Region {
    fn contains(&self, p: P2) -> bool {
        match self {
            Self::Round { c, r } => (p[0] - c[0]).hypot(p[1] - c[1]) < *r,
            Self::Peanut { peanut, slots, .. } => {
                geom::contains(peanut, p) && !slots.iter().any(|s| geom::contains(s, p))
            }
        }
    }

    fn on_fin(&self, lobe: &Lobe, p: P2) -> bool {
        match self {
            Self::Round { .. } => false,
            Self::Peanut { lobes, .. } => lobes.iter().any(|f| lobe.on_fin(local(f, p))),
        }
    }
}

/// The material of a brick pushed on that can meet studs.
enum Feature {
    /// Between the outer rectangle and the inner one (each `(min, max)`).
    Walls { outer: (P2, P2), inner: (P2, P2) },
    /// A tube ring.
    Tube { c: P2, ro: f64, ri: f64 },
}

fn sd_box(p: P2, (lo, hi): (P2, P2)) -> f64 {
    let q = [
        (p[0] - (lo[0] + hi[0]) / 2.0).abs() - (hi[0] - lo[0]) / 2.0,
        (p[1] - (lo[1] + hi[1]) / 2.0).abs() - (hi[1] - lo[1]) / 2.0,
    ];
    q[0].max(0.0).hypot(q[1].max(0.0)) + q[0].max(q[1]).min(0.0)
}

impl Feature {
    /// Signed distance from `p` to the material: negative inside.
    fn sd(&self, p: P2) -> f64 {
        match *self {
            Self::Walls { outer, inner } => sd_box(p, outer).max(-sd_box(p, inner)),
            Self::Tube { c, ro, ri } => {
                let d = (p[0] - c[0]).hypot(p[1] - c[1]);
                (d - ro).max(ri - d)
            }
        }
    }

    fn name(&self) -> String {
        match *self {
            Self::Walls { .. } => "wall".into(),
            Self::Tube { c, .. } => format!("tube at ({:.0}, {:.0})", c[0], c[1]),
        }
    }

    fn is_wall(&self) -> bool {
        matches!(self, Self::Walls { .. })
    }

    /// Points along the middle of the material, every 0.01 mm or so.
    fn core(&self) -> Vec<P2> {
        match *self {
            Self::Walls { outer, inner } => {
                let lo = [
                    (outer.0[0] + inner.0[0]) / 2.0,
                    (outer.0[1] + inner.0[1]) / 2.0,
                ];
                let hi = [
                    (outer.1[0] + inner.1[0]) / 2.0,
                    (outer.1[1] + inner.1[1]) / 2.0,
                ];
                let corners = [lo, [hi[0], lo[1]], hi, [lo[0], hi[1]]];
                let mut out = Vec::new();
                for k in 0..4 {
                    let (a, b) = (corners[k], corners[(k + 1) % 4]);
                    let n = ((b[0] - a[0]).abs() + (b[1] - a[1]).abs()) / 0.01;
                    let n = n.ceil() as usize;
                    for i in 0..n {
                        let s = i as f64 / n as f64;
                        out.push([a[0] + s * (b[0] - a[0]), a[1] + s * (b[1] - a[1])]);
                    }
                }
                out
            }
            Self::Tube { c, ro, ri } => {
                let r = (ro + ri) / 2.0;
                let n = (TAU * r / 0.01).ceil() as usize;
                (0..n)
                    .map(|k| {
                        let t = TAU * k as f64 / n as f64;
                        [c[0] + r * t.cos(), c[1] + r * t.sin()]
                    })
                    .collect()
            }
        }
    }

    /// How deep the material is from its middle to its nearer face.
    fn half_thickness(&self, p: &StudProfile) -> f64 {
        match self {
            Self::Walls { .. } => p.wall / 2.0,
            Self::Tube { ro, ri, .. } => (ro - ri) / 2.0,
        }
    }
}

/// The studs of a box part (its peanuts and round studs, as the build lays them out).
fn studs(shape: &CustomShape, p: &StudProfile) -> Result<Vec<Stud>> {
    let CustomShape::Box { w, d, .. } = *shape else {
        return Err(CustomError::InvalidShape(
            "fit checks are made on box parts".into(),
        ));
    };
    if w == 0 || d == 0 || w > 64 || d > 64 {
        return Err(CustomError::InvalidShape(format!("size {w}x{d}")));
    }
    p.validate()?;
    let (w, d) = (w as i32, d as i32);
    let cells: BTreeSet<(i32, i32)> = (0..w).flat_map(|i| (0..d).map(move |j| (i, j))).collect();
    let all: Vec<(i32, i32)> = cells.iter().copied().collect();
    let centre = |c: (i32, i32)| {
        [
            (f64::from(c.0) + 0.5) * p.pitch,
            (f64::from(c.1) + 0.5) * p.pitch,
        ]
    };
    let rs = p.stud_diameter / 2.0;
    Ok(match p.stud_style {
        StudStyle::Solid => all.iter().map(|&c| Stud::round(centre(c), rs)).collect(),
        StudStyle::SlantV2 { .. } => {
            let lobe = Lobe::new(p)?;
            let (peanuts, round) = slant::layout(&cells, &all);
            peanuts
                .iter()
                .map(|q| Stud::peanut(&lobe, q, p.pitch))
                .chain(round.iter().map(|&c| Stud::round(centre(c), rs)))
                .collect()
        }
    })
}

/// The walls and tubes of `top` over the part's stud grid.
fn features(p: &StudProfile, top: &TopBrick) -> Vec<Feature> {
    let (x0, y0) = (f64::from(top.at.0) * p.pitch, f64::from(top.at.1) * p.pitch);
    let (x1, y1) = (
        x0 + f64::from(top.w) * p.pitch,
        y0 + f64::from(top.d) * p.pitch,
    );
    let (c, t) = (p.clearance, p.clearance + p.wall);
    let mut out = vec![Feature::Walls {
        outer: ([x0 + c, y0 + c], [x1 - c, y1 - c]),
        inner: ([x0 + t, y0 + t], [x1 - t, y1 - t]),
    }];
    for i in 1..top.w {
        for j in 1..top.d {
            out.push(Feature::Tube {
                c: [x0 + f64::from(i) * p.pitch, y0 + f64::from(j) * p.pitch],
                ro: p.tube_outer_diameter / 2.0,
                ri: p.tube_inner_diameter / 2.0,
            });
        }
    }
    out
}

/// Where `top`, pushed onto the box `shape` built with `profile`, meets its studs.
pub fn fit(shape: &CustomShape, profile: &StudProfile, top: &TopBrick) -> Result<FitReport> {
    let studs = studs(shape, profile)?;
    let feats = features(profile, top);
    let mut r = FitReport {
        body_tube_clearance: f64::INFINITY,
        body_wall_clearance: f64::INFINITY,
        body_against: String::new(),
        fin_wall_preload: 0.0,
        fin_tube_preload: 0.0,
        fin_clearance: f64::INFINITY,
        round_tube_clearance: None,
        crossings: Vec::new(),
    };
    let lobe = Lobe::new(profile).ok();
    let mut least_body = f64::INFINITY;
    for s in &studs {
        let round = matches!(s.region, Region::Round { .. });
        for &(q, fin) in &s.samples {
            for f in &feats {
                let sd = f.sd(q);
                if fin {
                    r.fin_clearance = r.fin_clearance.min(sd);
                    let slot = if f.is_wall() {
                        &mut r.fin_wall_preload
                    } else {
                        &mut r.fin_tube_preload
                    };
                    *slot = slot.max(-sd);
                    continue;
                }
                if f.is_wall() {
                    r.body_wall_clearance = r.body_wall_clearance.min(sd);
                } else {
                    r.body_tube_clearance = r.body_tube_clearance.min(sd);
                    if round {
                        let c = r.round_tube_clearance.get_or_insert(f64::INFINITY);
                        *c = c.min(sd);
                    }
                }
                if sd < least_body {
                    least_body = sd;
                    r.body_against = f.name();
                }
            }
        }
        // A wall or tube whose middle lies in the stud: the stud is through it.
        for f in &feats {
            for q in f.core() {
                if s.region.contains(q) {
                    let fin = lobe.as_ref().is_some_and(|l| s.region.on_fin(l, q));
                    let depth = -f.half_thickness(profile);
                    if fin {
                        r.fin_clearance = r.fin_clearance.min(depth);
                    } else if f.is_wall() {
                        r.body_wall_clearance = r.body_wall_clearance.min(depth);
                    } else {
                        r.body_tube_clearance = r.body_tube_clearance.min(depth);
                    }
                    if !fin && depth < least_body {
                        least_body = depth;
                        r.body_against = f.name();
                    }
                    let what = format!("{} across a stud", f.name());
                    if !r.crossings.contains(&what) {
                        r.crossings.push(what);
                    }
                    break;
                }
            }
        }
    }
    Ok(r)
}

/// A Slant v2 peanut drawn in its own frame (origin at the waist centre, `v` along the row
/// toward lobe B, `u` toward the fin side), for figures.
#[derive(Clone, Debug)]
pub struct SlantOutline {
    /// The peanut's outline, closed.
    pub peanut: Vec<P2>,
    /// Lobe B's slot then lobe A's, closed.
    pub slots: [Vec<P2>; 2],
    /// Lobe B's pocket then lobe A's, closed.
    pub pockets: [Vec<P2>; 2],
    /// The stud's top outline (peanut less slots), and whether each point is on a fin.
    pub edge: Vec<(P2, bool)>,
}

/// The outline of `profile`'s v2 peanut (an error if it has none).
pub fn slant_outline(profile: &StudProfile) -> Result<SlantOutline> {
    let lobe = Lobe::new(profile)?;
    let a = lobe.a;
    let lobes = [
        Frame {
            o: [0.0, a],
            x: [1.0, 0.0],
            y: [0.0, 1.0],
        },
        Frame {
            o: [0.0, -a],
            x: [1.0, 0.0],
            y: [0.0, -1.0],
        },
    ];
    let poly = |l: &[Seg]| {
        let mut out = Vec::new();
        for s in l {
            let n = match s {
                Seg::Line { .. } => 2,
                Seg::Arc { .. } => 120,
            };
            out.extend((0..n).map(|k| s.at(k as f64 / n as f64)));
        }
        out
    };
    let slots = lobes.map(|f| f.map_loop(&lobe.slot));
    let pockets = lobes.map(|f| f.map_loop(&lobe.pocket));
    let peanut = slant::Peanut {
        a: (0, 0),
        b: (0, 1),
        fin: (1, 0),
    };
    // That peanut's frame is the peanut frame moved to (pitch/2, pitch); undo the move.
    let stud = Stud::peanut(&lobe, &peanut, profile.pitch);
    let o = peanut.frame(profile.pitch).o;
    let edge = stud
        .samples
        .iter()
        .step_by(10)
        .map(|&(p, fin)| ([p[0] - o[0], p[1] - o[1]], fin))
        .collect();
    Ok(SlantOutline {
        peanut: poly(&lobe.peanut),
        slots: [poly(&slots[0]), poly(&slots[1])],
        pockets: [poly(&pockets[0]), poly(&pockets[1])],
        edge,
    })
}
