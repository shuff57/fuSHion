//! Bouquet stems and hubs (see `sf_core::stem` for the contract): a **stem** prints as one
//! piece lying flat, and a **hub** is a puck on studs holding the stems' pegs in keyed holes.
//! Flower heads take a stem's tip bar in a [`BAR_MOUNT_DEPTH`] bar hole up into their hub
//! (see [`head_mount_cmds`]); leaves clip onto the stem's bar sections.
//!
//! # A stem, as printed
//! Millimetres, lying on the bed (z = 0). The stem's own frame (LDU, `-Y` up, bending toward
//! `+X`, `+Z` the bed side) maps to the print frame by `(x, y, z) → (x, −y, −z)/2.5` with the
//! axis lifted to [`STEM_FLAT`]: the centreline runs from the base shoulder at the origin up
//! `+y`, bending toward `+x` in the plane `z = STEM_FLAT`, and everything below `z = 0` is
//! cut off. That leaves every round section with a flat underside [`STEM_FLAT`] below its
//! axis: 4 mm wide on the rod, 3.75 mm on the peg, 1.05 mm on a bar (whose round bottom only
//! just reaches the bed: 0.09 mm is cut off).
//!
//! Along the centreline `s` (mm from the base shoulder):
//! * `s ∈ [−PEG_LEN, 0]`: the keyed **peg** ([`PEG_D`], straight, [`PEG_LEAD`] chamfered
//!   tip); its flat is the key, on the bed side like the stem's;
//! * `s ∈ [0, COLLAR_LEN]`: the **collar** (Ø[`COLLAR_D`]) whose face stops on the hub;
//! * then the **rod** ([`STEM_D`]), stepping down to Ø[`BAR_D`] for each **bar section**;
//! * the last `tip_bar` mm: the **tip bar** (Ø[`BAR_D`]).
//!
//! Each section is one solid (a revolve about the bend's centre, or an extrude when the stem
//! is straight), each smaller one reaching [`OVERLAP`] into its bigger neighbours, so the
//! volume is exactly the sum of the sections': every section's cross-section is symmetric
//! about the centreline's radius, so by Pappus its volume is its area times its length.
//!
//! The stem's id is engraved [`crate::LABEL_DEPTH`] into the peg's flat, mirrored like a
//! brick's (it reads from below).

use crate::detail::{
    BASE_HEIGHT, Detail, closed, cmd, finish_sketch, p3, r6, sketch_on, sketch_xy,
};
use crate::kit::{BAR_D, ClipSpec, MIN_FEATURE, clip_fit};
use crate::label::{Keepout, LABEL_DEPTH, engrave};
use crate::run::{measure, run_step};
use crate::shape::{Footprint, P2};
use crate::{CustomError, CustomPart, Result, StudProfile};
use serde::Serialize;
use serde_json::{Value, json};
use sf_core::{Hub, MountKind, StemDef, StemMount, StemStyle, StemTip, StyleBranch};
use solidcraft_engine::Session;
use std::f64::consts::PI;

/// The stem rod's diameter, mm (`sf_core::stem::STEM_D`).
pub const STEM_D: f64 = sf_core::stem::STEM_D as f64;
/// How far the flat underside is below the axis, mm.
pub const STEM_FLAT: f64 = sf_core::stem::STEM_FLAT as f64;
/// The base peg's diameter, mm.
pub const PEG_D: f64 = sf_core::stem::PEG_D as f64;
/// The base peg's length, mm.
pub const PEG_LEN: f64 = sf_core::stem::PEG_LEN as f64;
/// Longest stem, mm.
pub const MAX_STEM_LEN: f64 = sf_core::stem::MAX_STEM_LEN as f64;
/// The collar that stops the stem on the hub's top: its diameter, mm.
pub const COLLAR_D: f64 = 7.0;
/// The collar's length, mm.
pub const COLLAR_LEN: f64 = 2.0;
/// The peg's tip chamfer: this long, down to 0.5 mm less in radius, mm.
pub const PEG_LEAD: f64 = 0.5;
/// How far a smaller section reaches into its bigger neighbours, mm.
pub const OVERLAP: f64 = 0.05;
/// How deep a stem's tip bar goes into a flower head's bar hole, mm (from the hole's mouth
/// to where its crush ribs end). A tip bar this long seats the stem's rod on the mouth.
pub const BAR_MOUNT_DEPTH: f64 = 4.5;
/// The hub's peg holes: diameter (0.2 mm clearance), the key flat's distance from the axis
/// (0.05 mm clearance), and three crush ribs standing in to 0.1 mm inside the peg's radius.
pub const PEG_HOLE_D: f64 = PEG_D + 0.2;
/// The peg hole's key flat's distance from its axis, mm.
pub const PEG_HOLE_FLAT: f64 = STEM_FLAT + 0.05;
/// Radius the peg hole's crush ribs stand in to, mm.
pub const PEG_RIB_IN: f64 = PEG_D / 2.0 - 0.1;
/// The flower heads' bar holes: bore diameter (0.15 mm clearance a side) and three crush
/// ribs standing in to 0.05 mm inside the bar's radius (the kits' collar bore).
pub const BAR_BORE_D: f64 = BAR_D + 0.3;
/// Radius the bar hole's crush ribs stand in to, mm.
pub const BAR_RIB_IN: f64 = BAR_D / 2.0 - 0.05;
/// Width of a crush rib, mm.
const RIB_W: f64 = 0.6;
/// Floor kept under a hub's peg hole, above the socket's ceiling, mm.
const HUB_FLOOR: f64 = 0.6;
/// Wall kept round a hub's peg hole to the hub's side, and between two holes (two 0.4 mm
/// lines), mm.
const HUB_WALL: f64 = 1.0;
const HOLE_GAP: f64 = 0.8;

/// The shortest bar section a clip fits on with room to spare at both shoulders, mm.
pub fn min_bar_section() -> f64 {
    ClipSpec::default().height + 0.8
}

/// Area of a circle of radius `r` above a chord `d` below its centre (all of it if the
/// circle doesn't reach the chord).
pub fn d_area(r: f64, d: f64) -> f64 {
    if r <= d {
        return PI * r * r;
    }
    PI * r * r - (r * r * (d / r).acos() - d * (r * r - d * d).sqrt())
}

/// Volume of a ball of radius `r` above a plane `d` below its centre.
fn d_ball(r: f64, d: f64) -> f64 {
    if r <= d {
        return 4.0 / 3.0 * PI * r.powi(3);
    }
    let h = r - d;
    4.0 / 3.0 * PI * r.powi(3) - PI * h * h * (3.0 * r - h) / 3.0
}

/// Smallest turn a section is revolved through, degrees, and the largest bend radius, mm:
/// SolidCraft's checks refuse thin tori much flatter than that (found by trying: every
/// radius up to 100 mm and turn from 2° builds; from 400 mm many fail).
const MIN_REVOLVE: f64 = 3.0;
const MAX_REVOLVE_R: f64 = 100.0;
/// A gentler arc (or a short one) is lofted through round sections at most this far apart
/// along it, mm (a spline surface through them, within a few µm of the arc).
const LOFT_STEP: f64 = 15.0;

/// How a section is built.
enum Build {
    /// Straight: extruded.
    Extrude,
    /// Revolved about the arc's centre.
    Revolve,
    /// Lofted through round sections at these points along the arc.
    Loft(Vec<f64>),
}

/// A circular arc (or straight line) in the bed plane: from `p0` heading `a0` radians from
/// `+y` toward `+x`, turning `k` radians per mm (positive toward `+x`).
#[derive(Clone, Copy, Debug)]
pub struct Arc {
    /// Start point, mm.
    pub p0: [f64; 2],
    /// Start heading, radians.
    pub a0: f64,
    /// Curvature, radians per mm.
    pub k: f64,
}

impl Arc {
    /// The point `u` mm along.
    pub fn point(&self, u: f64) -> [f64; 2] {
        if self.k.abs() < 1e-9 {
            [
                self.p0[0] + u * self.a0.sin(),
                self.p0[1] + u * self.a0.cos(),
            ]
        } else {
            let a = self.a0 + self.k * u;
            [
                self.p0[0] + (self.a0.cos() - a.cos()) / self.k,
                self.p0[1] + (a.sin() - self.a0.sin()) / self.k,
            ]
        }
    }
    /// Heading `u` mm along (radians from `+y` toward `+x`).
    pub fn heading(&self, u: f64) -> f64 {
        self.a0 + self.k * u
    }
    /// Unit tangent `u` mm along.
    pub fn tangent(&self, u: f64) -> [f64; 2] {
        let a = self.heading(u);
        [a.sin(), a.cos()]
    }

    fn build(&self, u0: f64, u1: f64) -> Build {
        let k = self.k.abs();
        if k < 1e-9 {
            return Build::Extrude;
        }
        if 1.0 / k <= MAX_REVOLVE_R && k * (u1 - u0) >= MIN_REVOLVE.to_radians() {
            return Build::Revolve;
        }
        let n = ((u1 - u0) / LOFT_STEP).ceil().clamp(2.0, 49.0) as usize;
        Build::Loft(
            (0..=n)
                .map(|j| u0 + (u1 - u0) * j as f64 / n as f64)
                .collect(),
        )
    }

    /// The volume of a section of radius `r` from `u0` to `u1` (flat-bottomed): its area
    /// times its length (by Pappus; a loft follows the arc to within a few µm).
    fn section_volume(&self, u0: f64, u1: f64, r: f64) -> (f64, f64) {
        let v = d_area(r, STEM_FLAT) * (u1 - u0);
        (v, v)
    }

    /// A round section of radius `r` from `u0` to `u1` along it, its axis at [`STEM_FLAT`]
    /// (see [`Arc::build`]): the body `Part` (`first`) or joined to it.
    fn section(&self, name: &str, u0: f64, u1: f64, r: f64, first: bool) -> Vec<Value> {
        let op = |first: bool, sk: &str| {
            let mut op = if first {
                json!({"operation": "new", "body_name": "Part"})
            } else {
                json!({"operation": "join", "targets": ["Part"]})
            };
            op["name"] = json!(format!("{sk}Solid"));
            op["sketch"] = json!(sk);
            op["direction"] = json!("symmetric");
            op
        };
        match self.build(u0, u1) {
            Build::Revolve => {
                let c = [
                    self.p0[0] + self.a0.cos() / self.k,
                    self.p0[1] - self.a0.sin() / self.k,
                ];
                let rad = 1.0 / self.k.abs();
                let pm = self.point((u0 + u1) / 2.0);
                let u = [(pm[0] - c[0]) / rad, (pm[1] - c[1]) / rad, 0.0];
                let mut o = op(first, name);
                o["axis"] = json!({"origin": p3([c[0], c[1], 0.0]), "dir": [0, 0, 1]});
                o["angle"] = json!(format!(
                    "{} deg",
                    r6(((u1 - u0) * self.k.abs()).to_degrees())
                ));
                vec![
                    sketch_on(name, [c[0], c[1], 0.0], u, [0.0, 0.0, 1.0]),
                    crate::detail::circle([rad, STEM_FLAT], 2.0 * r),
                    finish_sketch(),
                    cmd("solid.revolve", o),
                ]
            }
            Build::Extrude => {
                let (a, b) = (self.point(u0), self.point(u1));
                let t = self.tangent(u0);
                let pm = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
                let mut o = op(first, name);
                o["distance"] = json!(r6(u1 - u0));
                vec![
                    sketch_on(
                        name,
                        [pm[0], pm[1], 0.0],
                        [t[1], -t[0], 0.0],
                        [0.0, 0.0, 1.0],
                    ),
                    crate::detail::circle([0.0, STEM_FLAT], 2.0 * r),
                    finish_sketch(),
                    cmd("solid.extrude", o),
                ]
            }
            Build::Loft(us) => {
                let mut v = Vec::new();
                let mut secs = Vec::new();
                for (j, &u) in us.iter().enumerate() {
                    let (p, t) = (self.point(u), self.tangent(u));
                    let sk = format!("{name}L{j}");
                    v.extend([
                        sketch_on(&sk, [p[0], p[1], 0.0], [t[1], -t[0], 0.0], [0.0, 0.0, 1.0]),
                        crate::detail::circle([0.0, STEM_FLAT], 2.0 * r),
                        finish_sketch(),
                    ]);
                    secs.push(json!({"sketch": sk}));
                }
                let mut o = if first {
                    json!({"operation": "new", "body_name": "Part"})
                } else {
                    json!({"operation": "join", "targets": ["Part"]})
                };
                o["sections"] = json!(secs);
                o["name"] = json!(format!("{name}Solid"));
                v.push(cmd("solid.loft", o));
                v
            }
        }
    }
}

/// A section along a centreline: `[s0, s1]` with radius `r`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Section {
    s0: f64,
    s1: f64,
    r: f64,
}

/// One rod (the main stem or a branch) as built: its arc, sections, and what ends it.
struct Rod {
    arc: Arc,
    secs: Vec<Section>,
    /// End point's distance along the arc and the rod's radius there.
    end: f64,
    end_r: f64,
    tip: Option<StemTip>,
    /// No tip bar and no tip: a rounded end.
    rounded: bool,
}

/// The main stem's arc in the print frame (mm).
pub fn main_arc(def: &StemDef) -> Arc {
    let l = f64::from(def.length).max(1e-3);
    Arc {
        p0: [0.0, 0.0],
        a0: 0.0,
        k: f64::from(def.bend).to_radians() / l,
    }
}

/// Branch `i`'s arc in the print frame (mm), from the main stem's centreline.
pub fn branch_arc(def: &StemDef, i: usize) -> Arc {
    let m = main_arc(def);
    let b = &def.branches[i];
    let at = f64::from(b.at);
    Arc {
        p0: m.point(at),
        a0: m.heading(at) + f64::from(b.angle).to_radians(),
        k: f64::from(b.bend).to_radians() / f64::from(b.length).max(1e-3),
    }
}

fn tip_check(id: &str, what: &str, tip: &Option<StemTip>) -> Result<()> {
    match tip {
        Some(StemTip::Bud { length, width }) | Some(StemTip::Blade { length, width })
            if !(2.0..=60.0).contains(length) || !(1.0..=30.0).contains(width) =>
        {
            Err(CustomError::InvalidShape(format!(
                "stem {id}: {what}'s tip is {length:.1} × {width:.1} mm; 2–60 by 1–30"
            )))
        }
        _ => Ok(()),
    }
}

/// The main stem's and every branch's sections, checked.
fn rods(def: &StemDef) -> Result<Vec<Rod>> {
    let id = &def.id;
    let bad = |why: String| Err(CustomError::InvalidShape(format!("stem {id}: {why}")));
    let l = f64::from(def.length);
    let tip = f64::from(def.tip_bar);
    let d = f64::from(def.diameter);
    if !(f64::from(sf_core::stem::MIN_STEM_D) - 1e-6..=MAX_ROD_D + 1e-6).contains(&d) {
        return bad(format!(
            "{d:.2} mm thick; {}–{MAX_ROD_D} mm",
            sf_core::stem::MIN_STEM_D
        ));
    }
    if !(l.is_finite() && l <= MAX_STEM_LEN + 1e-6) {
        return bad(format!("{l:.1} mm long; at most {MAX_STEM_LEN} mm"));
    }
    if !(0.0..=40.0).contains(&tip) {
        return bad(format!("tip bar {tip:.1} mm; 0 to 40 mm"));
    }
    let rod_to = l - tip;
    if rod_to < COLLAR_LEN + 5.0 {
        return bad(format!(
            "{l:.1} mm is too short for its collar, tip bar and 5 mm of rod"
        ));
    }
    if f64::from(def.bend).abs() > 180.0 {
        return bad("bends at most 180°".into());
    }
    tip_check(id, "the stem", &def.tip)?;
    let mut bars: Vec<(f64, f64)> = def
        .bars
        .iter()
        .map(|b| {
            (
                f64::from(b.at) - f64::from(b.length) / 2.0,
                f64::from(b.at) + f64::from(b.length) / 2.0,
            )
        })
        .collect();
    bars.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut secs = vec![Section {
        s0: 0.0,
        s1: COLLAR_LEN,
        r: COLLAR_D / 2.0,
    }];
    let mut s = COLLAR_LEN;
    for (k, &(a, b)) in bars.iter().enumerate() {
        if b - a < min_bar_section() - 1e-9 {
            return bad(format!(
                "bar section {k} is {:.1} mm; a clip needs {:.1} mm",
                b - a,
                min_bar_section()
            ));
        }
        if a < s + 1.0 || b > rod_to - 1.0 {
            return bad(format!(
                "bar section {k} ({a:.1}–{b:.1} mm) must keep 1 mm of rod either side, within {COLLAR_LEN:.1}–{rod_to:.1} mm"
            ));
        }
        secs.push(Section {
            s0: s,
            s1: a,
            r: d / 2.0,
        });
        secs.push(Section {
            s0: a,
            s1: b,
            r: BAR_D / 2.0,
        });
        s = b;
    }
    secs.push(Section {
        s0: s,
        s1: rod_to,
        r: d / 2.0,
    });
    if tip > 0.0 {
        secs.push(Section {
            s0: rod_to,
            s1: l,
            r: BAR_D / 2.0,
        });
    }
    let mut out = vec![Rod {
        arc: main_arc(def),
        secs,
        end: l,
        end_r: if tip > 0.0 { BAR_D / 2.0 } else { d / 2.0 },
        tip: if tip > 0.0 { None } else { def.tip },
        rounded: tip <= 0.0 && def.tip.is_none(),
    }];
    for (i, b) in def.branches.iter().enumerate() {
        let n = i + 1;
        let (at, bl, bt) = (f64::from(b.at), f64::from(b.length), f64::from(b.tip_bar));
        let bd = f64::from(def.branch_diameter(i));
        if bd > d + 1e-6 {
            return bad(format!("branch {n} is thicker than the stem"));
        }
        if !(COLLAR_LEN + 5.0..=rod_to - 2.0).contains(&at) {
            return bad(format!(
                "branch {n} leaves at {at:.1} mm; {:.1}–{:.1} mm",
                COLLAR_LEN + 5.0,
                rod_to - 2.0
            ));
        }
        if bars.iter().any(|&(a, e)| at > a - bd && at < e + bd) {
            return bad(format!("branch {n} leaves from a bar section"));
        }
        if !(0.0..=40.0).contains(&bt) || bl - bt < 3.0 * d {
            return bad(format!(
                "branch {n} is {bl:.1} mm with a {bt:.1} mm tip bar; it needs {:.1} mm of rod",
                3.0 * d
            ));
        }
        let ang = f64::from(b.angle).abs();
        if !(15.0..=120.0).contains(&ang) {
            return bad(format!("branch {n} turns {ang:.0}° off the stem; 15–120°"));
        }
        tip_check(id, &format!("branch {n}"), &b.tip)?;
        // Never quite as thick as the stem (equal rods crossing make a cusp).
        let bd = if bd > d - 0.1 { d - 0.1 } else { bd };
        let mut secs = vec![Section {
            s0: 0.0,
            s1: bl - bt,
            r: bd / 2.0,
        }];
        if bt > 0.0 {
            secs.push(Section {
                s0: bl - bt,
                s1: bl,
                r: BAR_D / 2.0,
            });
        }
        out.push(Rod {
            arc: branch_arc(def, i),
            secs,
            end: bl,
            end_r: if bt > 0.0 { BAR_D / 2.0 } else { bd / 2.0 },
            tip: if bt > 0.0 { None } else { b.tip },
            rounded: bt <= 0.0 && b.tip.is_none(),
        });
    }
    // Rods (past their forks) must keep clear of each other, so none fuse into a blob: each
    // sampled as discs along its centreline (its tip's too, at the tip's half-width).
    let discs = |r: &Rod, skip: f64| -> Vec<([f64; 2], f64)> {
        let mut v = Vec::new();
        let n = (r.end / 1.0).ceil().max(2.0) as usize;
        for j in 0..=n {
            let u = r.end * j as f64 / n as f64;
            if u >= skip {
                let rr = r
                    .secs
                    .iter()
                    .find(|s| u >= s.s0 - 1e-9 && u <= s.s1 + 1e-9)
                    .map_or(r.end_r, |s| s.r);
                v.push((r.arc.point(u), rr));
            }
        }
        let (tl, tw) = tip_reach(&r.tip);
        let t = r.arc.tangent(r.end);
        let e = r.arc.point(r.end);
        for j in 1..=(tl.ceil() as usize) {
            let u = j as f64;
            v.push((
                [e[0] + t[0] * u, e[1] + t[1] * u],
                tw * (1.0 - (u / tl - 0.35).abs()).clamp(0.2, 1.0),
            ));
        }
        v
    };
    for j in 1..out.len() {
        let b = &def.branches[j - 1];
        let rj = f64::from(def.branch_diameter(j - 1)) / 2.0;
        // How far along branch j before it is clear of the stem by its own lean.
        let fork_len = (d / 2.0 + rj + 1.0) / f64::from(b.angle).to_radians().abs().sin() + 1.0;
        let at = out[0].arc.point(f64::from(b.at));
        for i in 0..j {
            for (p, ri) in discs(&out[i], 0.0) {
                if i == 0 && (p[0] - at[0]).hypot(p[1] - at[1]) < fork_len + d {
                    continue;
                }
                for (q, rq) in discs(&out[j], fork_len) {
                    let gap = (p[0] - q[0]).hypot(p[1] - q[1]) - ri - rq;
                    if gap < 1.0 {
                        let other = if i == 0 {
                            "the stem".to_string()
                        } else {
                            format!("branch {i}")
                        };
                        return bad(format!(
                            "{other} and branch {j} come within {gap:.1} mm of each other; they need 1 mm"
                        ));
                    }
                }
            }
        }
    }
    Ok(out)
}

/// How far a tip reaches past its rod's end, and how wide (half), mm.
fn tip_reach(t: &Option<StemTip>) -> (f64, f64) {
    match t {
        Some(StemTip::Bud { length, width }) => {
            (f64::from(*length), f64::from(*width).max(BUD_MIN_W) / 2.0)
        }
        Some(StemTip::Blade { length, width }) => (f64::from(*length), f64::from(*width) / 2.0),
        None => (0.0, 0.0),
    }
}

/// Where flat features start under the bed (all cut off): an odd depth no rod or ball
/// bottom lands on, mm.
const BELOW: f64 = -0.37;

/// A bud is at least this wide, so its round underside reaches the bed, mm.
pub const BUD_MIN_W: f64 = 2.0 * STEM_FLAT + 0.4;
/// A blade's thickness, mm (as the stem's flat allows).
pub const BLADE_T: f64 = 1.6;

/// The peg's profile `(t, radius)` along its axis, `t` from the shoulder (0) to its tip.
fn peg_profile() -> [(f64, f64); 3] {
    [
        (0.0, PEG_D / 2.0),
        (PEG_LEN - PEG_LEAD, PEG_D / 2.0),
        (PEG_LEN, PEG_D / 2.0 - 0.5),
    ]
}

/// The peg's volume (with its flat and chamfer), mm³.
fn peg_volume() -> f64 {
    let p = peg_profile();
    let n = 200;
    let h = PEG_LEAD / n as f64;
    let f = |t: f64| d_area(p[1].1 + (p[2].1 - p[1].1) * t / PEG_LEAD, STEM_FLAT);
    let mut acc = f(0.0) + f(PEG_LEAD);
    for i in 1..n {
        acc += f(i as f64 * h) * if i % 2 == 1 { 4.0 } else { 2.0 };
    }
    d_area(p[0].1, STEM_FLAT) * p[1].0 + acc * h / 3.0
}

/// Bounds on a stem's volume before its engraving, mm³: `(low, high)`. They are equal (the
/// closed form) for a stem with no branches and no tip shape whose sections are revolved or
/// single chords: each section's flat-bottomed area times its length (by Pappus for a
/// revolve), the peg's chamfer integrated, half a flat-bottomed ball on a rounded end. A
/// gentle arc built from chords is bounded by its chords less the wedges where they overlap
/// and plus its joint balls; a branch, its fork and its tip add at least nothing (they
/// overlap the stem) and at most their own volumes.
pub fn stem_volume_bounds(def: &StemDef) -> Result<(f64, f64)> {
    let rods = rods(def)?;
    let rod_v = |r: &Rod| -> (f64, f64) {
        let (mut lo, mut hi) = (0.0, 0.0);
        for s in &r.secs {
            let (a, b) = r.arc.section_volume(s.s0, s.s1, s.r);
            lo += a;
            hi += b;
        }
        if r.rounded {
            let e = d_ball(r.end_r, STEM_FLAT) / 2.0;
            lo += e;
            hi += e;
        }
        (lo, hi)
    };
    let (mlo, mhi) = rod_v(&rods[0]);
    let (lo, mut hi) = (mlo + peg_volume(), mhi + peg_volume());
    for (i, r) in rods.iter().enumerate() {
        let (tl, tw) = tip_reach(&r.tip);
        hi += tl * 2.0 * tw * (STEM_FLAT + tw);
        if i > 0 {
            hi += rod_v(r).1;
            let (rn, g, _) = fork(def, i - 1);
            hi += d_ball(rn, STEM_FLAT) + g * g * STEM_FLAT;
        }
    }
    Ok((lo, hi))
}

/// Branch `i`'s fork: the node ball's radius, the gusset's reach along each rod, and its
/// corner points.
fn fork(def: &StemDef, i: usize) -> (f64, f64, [[f64; 2]; 3]) {
    let d = f64::from(def.diameter);
    let rn = d / 2.0 * 1.25;
    let g = 2.5 * d;
    let m = main_arc(def);
    let b = branch_arc(def, i);
    let at = f64::from(def.branches[i].at);
    (rn, g, [m.point(at), m.point(at + g), b.point(g)])
}

/// The commands for a tip shape at a rod's end.
fn tip_cmds(name: &str, arc: &Arc, end: f64, r_end: f64, tip: &StemTip) -> Vec<Value> {
    let p = arc.point(end);
    let t = arc.tangent(end);
    let n = [-t[1], t[0]];
    let w2 = |u: f64, v: f64| -> P2 { [p[0] + t[0] * u + n[0] * v, p[1] + t[1] * u + n[1] * v] };
    match *tip {
        StemTip::Bud { length, width } => {
            let (l, w) = (f64::from(length), f64::from(width).max(BUD_MIN_W));
            // A spindle round the axis (its profile an arc from the rod's radius out to the
            // bud's and in to a point), on a plinth of its outline so its underside rests on
            // the bed all along.
            let r0 = r_end * 0.95;
            // The plinth starts inside the rod's own footprint (no shallow crossings).
            let root = 0.8 * (r_end * r_end - STEM_FLAT * STEM_FLAT).max(0.0).sqrt();
            let sk = format!("{name}Sk");
            let pl = format!("{name}Plinth");
            let e = 0.3;
            vec![
                sketch_on(
                    &sk,
                    [p[0], p[1], STEM_FLAT],
                    [t[0], t[1], 0.0],
                    [0.0, 0.0, 1.0],
                ),
                crate::detail::line([-e, 0.0], [-e, r0]),
                crate::detail::arc3([-e, r0], [l * 0.45, w / 2.0], [l, 0.0]),
                crate::detail::line([l, 0.0], [-e, 0.0]),
                finish_sketch(),
                cmd(
                    "solid.revolve",
                    json!({"sketch": sk, "axis": {"origin": p3([p[0], p[1], STEM_FLAT]), "dir": p3([t[0], t[1], 0.0])}, "operation": "join", "targets": ["Part"], "name": format!("{name}Spindle")}),
                ),
                sketch_xy(&pl, BELOW),
                crate::detail::arc3(
                    w2(-1.0, -root),
                    w2(l * 0.45, -w / 2.0 * 0.92),
                    w2(l - 0.6, 0.0),
                ),
                crate::detail::arc3(
                    w2(l - 0.6, 0.0),
                    w2(l * 0.45, w / 2.0 * 0.92),
                    w2(-1.0, root),
                ),
                crate::detail::line(w2(-1.0, root), w2(-1.0, -root)),
                finish_sketch(),
                cmd(
                    "solid.extrude",
                    json!({"sketch": pl, "name": format!("{pl}Solid"), "distance": r6(1.0 - BELOW), "operation": "join", "targets": ["Part"]}),
                ),
            ]
        }
        StemTip::Blade { length, width } => {
            let (l, w) = (f64::from(length), f64::from(width));
            // Its root inside the rod's own footprint (no shallow crossings), flaring out past
            // the rod's end.
            let w0 = (1.6 * (r_end * r_end - STEM_FLAT * STEM_FLAT).max(0.0).sqrt()).min(w);
            let sk = format!("{name}Blade");
            vec![
                sketch_xy(&sk, BELOW),
                crate::detail::arc3(w2(-1.0, -w0 / 2.0), w2(l * 0.35, -w / 2.0), w2(l, 0.0)),
                crate::detail::arc3(w2(l, 0.0), w2(l * 0.35, w / 2.0), w2(-1.0, w0 / 2.0)),
                crate::detail::line(w2(-1.0, w0 / 2.0), w2(-1.0, -w0 / 2.0)),
                finish_sketch(),
                cmd(
                    "solid.extrude",
                    json!({"sketch": sk, "name": format!("{sk}Solid"), "distance": r6(BLADE_T - BELOW), "operation": "join", "targets": ["Part"]}),
                ),
            ]
        }
    }
}

/// The SolidCraft commands for a stem (see the module docs), and the engraved area (mm²).
pub fn stem_commands(def: &StemDef) -> Result<(Vec<Value>, f64)> {
    let rods = rods(def)?;
    let mut c = Vec::new();
    let mut first = true;
    for (ri, rod) in rods.iter().enumerate() {
        let secs = &rod.secs;
        // A branch's fork first, so where the branch meets the stem it meets the node's ball
        // (two rods of one thickness crossing make a cusp SolidCraft refuses).
        if ri > 0 {
            // The fork: a node (a ball a little thicker than the stem) and a gusset in the
            // crotch from the bed up to the axis, so the branch doesn't snap off.
            let (rn, _, pts) = fork(def, ri - 1);
            let at = pts[0];
            c.push(cmd(
                "solid.sphere",
                json!({"radius": r6(rn), "center": p3([at[0] + 1e-3, at[1] + 7e-4, STEM_FLAT]), "operation": "join", "targets": ["Part"], "name": format!("Fork{ri}")}),
            ));
            let gs = format!("Gusset{ri}");
            c.extend([
                sketch_xy(&gs, BELOW),
                closed(&pts),
                finish_sketch(),
                cmd("solid.extrude", json!({"sketch": gs, "name": format!("{gs}Solid"), "distance": r6(STEM_FLAT - BELOW), "operation": "join", "targets": ["Part"]})),
            ]);
        }
        for (i, sec) in secs.iter().enumerate() {
            // Each smaller section reaches OVERLAP into its bigger neighbours; a branch starts
            // inside the stem's centreline.
            let grow = |j: Option<&Section>| j.is_some_and(|n| n.r > sec.r + 1e-9);
            let s0 = sec.s0
                - if grow(i.checked_sub(1).and_then(|j| secs.get(j))) {
                    OVERLAP
                } else {
                    0.0
                };
            let s1 = sec.s1 + if grow(secs.get(i + 1)) { OVERLAP } else { 0.0 };
            c.extend(rod.arc.section(&format!("R{ri}S{i}"), s0, s1, sec.r, first));
            first = false;
        }
        let end = rod.arc.point(rod.end);
        if rod.rounded {
            c.push(cmd(
                "solid.sphere",
                json!({"radius": r6(rod.end_r), "center": p3([end[0], end[1], STEM_FLAT]), "operation": "join", "targets": ["Part"], "name": format!("R{ri}End")}),
            ));
        }
        if let Some(t) = &rod.tip {
            c.extend(tip_cmds(
                &format!("R{ri}Tip"),
                &rod.arc,
                rod.end,
                rod.end_r,
                t,
            ));
        }
    }
    // The peg: its profile revolved about its axis (straight down −y from the shoulder).
    let p = peg_profile();
    let prof: Vec<P2> = vec![
        [-OVERLAP, 0.0],
        [-OVERLAP, p[0].1],
        [p[1].0, p[1].1],
        [p[2].0, p[2].1],
        [p[2].0, 0.0],
    ];
    c.extend([
        sketch_on("PegSketch", [0.0, 0.0, STEM_FLAT], [0.0, -1.0, 0.0], [1.0, 0.0, 0.0]),
        closed(&prof),
        finish_sketch(),
        cmd(
            "solid.revolve",
            json!({"sketch": "PegSketch", "axis": {"origin": p3([0.0, 0.0, STEM_FLAT]), "dir": [0, -1, 0]}, "operation": "join", "targets": ["Part"], "name": "Peg"}),
        ),
    ]);
    // The flat: everything below the bed cut off.
    let (mut lo, mut hi) = ([-PEG_D, -PEG_LEN - 1.0], [PEG_D, PEG_D]);
    for rod in &rods {
        let reach = rod.end + tip_reach(&rod.tip).0;
        for i in 0..=64 {
            let q = rod.arc.point(reach * f64::from(i) / 64.0);
            lo = [
                lo[0].min(q[0] - COLLAR_D - 30.0),
                lo[1].min(q[1] - COLLAR_D - 30.0),
            ];
            hi = [
                hi[0].max(q[0] + COLLAR_D + 30.0),
                hi[1].max(q[1] + COLLAR_D + 30.0),
            ];
        }
    }
    c.push(cmd(
        "solid.box",
        json!({"length": r6(hi[0] - lo[0]), "width": r6(hi[1] - lo[1]), "height": 10.0, "corner": p3([lo[0], lo[1], -10.0]), "operation": "cut", "targets": ["Part"], "name": "Bed"}),
    ));
    // The id on the peg's flat, reading from below.
    let w = (PEG_D * PEG_D / 4.0 - STEM_FLAT * STEM_FLAT).sqrt();
    let strip = vec![vec![
        [-w, -PEG_LEN + PEG_LEAD + 0.2],
        [w, -PEG_LEN + PEG_LEAD + 0.2],
        [w, -0.3],
        [-w, -0.3],
    ]];
    let keep = Keepout {
        hollow: &strip,
        tubes: &[],
        tube_radius: 0.0,
        rects: &[],
        cuts: &[],
    };
    let mut area = 0.0;
    if let Some(e) = engrave(&def.id, &keep) {
        c.push(sketch_xy("IdSketch", -1.0));
        c.extend(e.loops.iter().map(|l| closed(l)));
        c.push(finish_sketch());
        c.push(cmd("solid.extrude", json!({"sketch": "IdSketch", "name": "Id", "distance": r6(1.0 + LABEL_DEPTH), "operation": "cut", "targets": ["Part"]})));
        area = e.area;
    }
    Ok((c, area))
}

/// Builds a stem lying flat (see the module docs), checked: one closed body, watertight, and
/// its volume within 0.2 % of the closed form ([`stem_volume_bounds`]; between the bounds
/// for a stem with branches or a tip shape).
pub fn build_stem(def: &StemDef) -> Result<CustomPart> {
    let (c, engraved) = stem_commands(def)?;
    let mut s = Session::default();
    for (i, x) in c.iter().enumerate() {
        run_step(&mut s, "Stem", i, x)?;
    }
    let (lo, hi) = stem_volume_bounds(def)?;
    let cut = engraved * LABEL_DEPTH;
    let m = measure(&mut s, "Stem")?;
    if m.bodies != 1 {
        return Err(CustomError::Mismatch {
            stage: "Stem".into(),
            details: format!("{} bodies, expected 1", m.bodies),
        });
    }
    let tol = crate::VOLUME_TOL;
    if m.volume < (lo - cut) * (1.0 - tol) || m.volume > (hi - cut) * (1.0 + tol) {
        return Err(CustomError::Mismatch {
            stage: "Stem".into(),
            details: format!(
                "volume {:.2} mm³, expected {:.2}–{:.2}",
                m.volume,
                lo - cut,
                hi - cut
            ),
        });
    }
    crate::run::finish_with(
        &mut s,
        "Stem",
        None,
        None,
        true,
        MAX_STEM_LEN + 2.0 * COLLAR_D + 60.0,
    )
}

// ---------------------------------------------------------------- fit numbers

/// How a stem fits what it meets: the numbers behind the fit coupon.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct StemFit {
    /// A bar section's flat (cut where it reaches below the bed), mm deep and mm wide.
    pub bar_flat_depth: f64,
    /// The bar's flat's width, mm.
    pub bar_flat_width: f64,
    /// The bar measured across its flat, mm.
    pub bar_across_flat: f64,
    /// A default clip's grip on the bar: its interference round the bar, and across the
    /// flat (both must be ≥ 0), mm.
    pub clip_interference: f64,
    /// The clip's grip across the bar's flat, mm.
    pub clip_interference_flat: f64,
    /// The clip's snap-on strain (≤ 3 %).
    pub clip_snap_strain: f64,
    /// Room between a clip and the rod's shoulders on the shortest bar section, mm each side.
    pub clip_shoulder_gap: f64,
    /// A head's bar hole: crush-rib interference on the bar's radius (where a rib meets the
    /// round), and where one meets the flat (negative: that rib doesn't touch), mm.
    pub head_rib_interference: f64,
    /// A head's crush rib where it meets the bar's flat, mm.
    pub head_rib_interference_flat: f64,
    /// The hub's peg hole: crush-rib interference on the peg's radius, the clearance round
    /// the rest, and the key flat's clearance, mm.
    pub peg_rib_interference: f64,
    /// Clearance round the peg between the ribs, mm.
    pub peg_clearance: f64,
    /// Clearance at the key flat, mm.
    pub peg_key_clearance: f64,
    /// How far the peg can turn in its hole before the key stops it, degrees.
    pub peg_key_play_deg: f64,
    /// Width of the peg's flat (the key) and the rod's, mm.
    pub peg_flat_width: f64,
    /// The Ø5 rod's flat's width, mm.
    pub rod_flat_width: f64,
}

/// The fit numbers (see [`StemFit`]).
pub fn stem_fit() -> StemFit {
    let rb = BAR_D / 2.0;
    let cut = (rb - STEM_FLAT).max(0.0);
    let clip = ClipSpec::default();
    let cf = clip_fit(&clip);
    let half = |r: f64| (r * r - STEM_FLAT * STEM_FLAT).max(0.0).sqrt() * 2.0;
    // The key: the peg's flat corner (at the flat's edge, radius PEG_D/2) swings until it
    // meets the hole's flat.
    let rp = PEG_D / 2.0;
    let a0 = (STEM_FLAT / rp).acos();
    let a1 = (PEG_HOLE_FLAT / rp).min(1.0).acos();
    StemFit {
        bar_flat_depth: cut,
        bar_flat_width: half(rb),
        bar_across_flat: BAR_D - cut,
        clip_interference: cf.interference,
        clip_interference_flat: cf.interference - cut,
        clip_snap_strain: cf.snap_strain,
        clip_shoulder_gap: (min_bar_section() - clip.height) / 2.0,
        head_rib_interference: rb - BAR_RIB_IN,
        head_rib_interference_flat: STEM_FLAT - BAR_RIB_IN,
        peg_rib_interference: rp - PEG_RIB_IN,
        peg_clearance: (PEG_HOLE_D - PEG_D) / 2.0,
        peg_key_clearance: PEG_HOLE_FLAT - STEM_FLAT,
        peg_key_play_deg: (a0 - a1).to_degrees(),
        peg_flat_width: half(rp),
        rod_flat_width: half(STEM_D / 2.0),
    }
}

// ---------------------------------------------------------------- crush-rib holes

/// Unit vectors square to unit `d`: `(e1, e2)` with `e1 × e2 = d`, `e1` along `hint` if
/// given (made square to `d`).
fn frame(d: [f64; 3], hint: Option<[f64; 3]>) -> ([f64; 3], [f64; 3]) {
    let norm = |v: [f64; 3]| {
        let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-12);
        v.map(|x| x / l)
    };
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let cross = |a: [f64; 3], b: [f64; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let h = hint.unwrap_or(if d[2].abs() < 0.9 {
        [0.0, 0.0, 1.0]
    } else {
        [1.0, 0.0, 0.0]
    });
    let e1 = norm([
        h[0] - dot(h, d) * d[0],
        h[1] - dot(h, d) * d[1],
        h[2] - dot(h, d) * d[2],
    ]);
    let e2 = cross(d, e1);
    (e1, e2)
}

/// Three crush ribs (prisms joined to `Part`) inside a bore along unit `dir` (pointing out
/// of the hole), from the plane through `from` (square to `dir`) `len` mm inward: each from
/// `r_in` out to `r_wall + 0.3`, at the angles (radians from `e1`) given.
#[allow(clippy::too_many_arguments)]
fn ribs(
    name: &str,
    from: [f64; 3],
    dir: [f64; 3],
    e1: [f64; 3],
    len: f64,
    r_in: f64,
    r_wall: f64,
    angles: &[f64],
) -> Vec<Value> {
    let e2 = [
        dir[1] * e1[2] - dir[2] * e1[1],
        dir[2] * e1[0] - dir[0] * e1[2],
        dir[0] * e1[1] - dir[1] * e1[0],
    ];
    let mut c = vec![sketch_on(name, from, e1, e2)];
    for &a in angles {
        let (s, co) = a.sin_cos();
        let m = |p: P2| -> P2 { [p[0] * co - p[1] * s, p[0] * s + p[1] * co] };
        let h = RIB_W / 2.0;
        c.push(closed(&[
            m([r_in, -h]),
            m([r_wall + 0.3, -h]),
            m([r_wall + 0.3, h]),
            m([r_in, h]),
        ]));
    }
    c.push(finish_sketch());
    // The sketch's normal is e1 × e2 = dir: extrude against it, into the part.
    c.push(cmd(
        "solid.extrude",
        json!({"sketch": name, "name": format!("{name}Solid"), "distance": r6(len), "direction": "negative", "operation": "join", "targets": ["Part"]}),
    ));
    c
}

/// A flower head's bar hole, up the vertical through `at` from its mouth `mouth` (the socket's
/// ceiling, or the rod's seat over a tube: see [`head_mouth`]): a Ø[`BAR_BORE_D`] bore
/// [`BAR_MOUNT_DEPTH`] deep with three crush ribs (from 0.3 mm above the mouth), its top a
/// cone steeper than 45° (to a point, or, `through`, down to Ø3.0 where a hole from above
/// continues it). Commands on `Part`, to run after everything joined round it.
pub(crate) fn head_mount_cmds(at: P2, mouth: f64, through: bool) -> Vec<Value> {
    let rb = BAR_BORE_D / 2.0;
    let top = mouth + BAR_MOUNT_DEPTH;
    let z_from = BASE_HEIGHT - StudProfile::fused().top_thickness - 0.5;
    let mut c = vec![cmd(
        "solid.cylinder",
        json!({"radius": r6(rb), "height": r6(top - z_from), "base": p3([at[0], at[1], z_from]), "operation": "cut", "targets": ["Part"], "name": "MountBore"}),
    )];
    let r_end = if through { 1.5 } else { 0.0 };
    let mut cone = json!({"radius": r6(rb), "height": r6((rb - r_end) * 1.2), "base": p3([at[0], at[1], top - 0.01]), "operation": "cut", "targets": ["Part"], "name": "MountRoof"});
    if r_end > 0.0 {
        cone["top_radius"] = json!(r6(r_end));
    }
    c.push(cmd("solid.cone", cone));
    // Ribs from 0.3 mm over the mouth up to the bore's top (the extrude runs against the
    // sketch's normal, `−z`... so the sketch faces down and the ribs go up).
    c.extend(ribs(
        "MountRibs",
        [at[0], at[1], mouth + 0.3],
        [0.0, 0.0, -1.0],
        [1.0, 0.0, 0.0],
        BAR_MOUNT_DEPTH - 0.3,
        BAR_RIB_IN,
        rb,
        &[0.3, 0.3 + 2.0 * PI / 3.0, 0.3 + 4.0 * PI / 3.0],
    ));
    c
}

/// How tall a head's solid must reach on its axis for a blind bar hole from `mouth`, mm.
pub(crate) fn head_mount_top(mouth: f64) -> f64 {
    mouth + BAR_MOUNT_DEPTH + BAR_BORE_D / 2.0 * 1.2 + MIN_FEATURE
}

/// The top a kit hub needs for a bar hole from `mouth` running into its centre's peg hole
/// (Ø3.2, [`crate::kit::PEG_LEN`] + 0.5 deep from the top), mm.
pub(crate) fn hub_mount_top(mouth: f64) -> f64 {
    mouth + BAR_MOUNT_DEPTH + (BAR_BORE_D / 2.0 - 1.5) * 1.2 - 0.1 + crate::kit::PEG_LEN + 0.5
}

/// Where a head's axis is on a 2×2 base's tube, the tube is cleared to Ø[`SEAT_D`] (a
/// 0.6 mm wall left) up to the socket's ceiling, so any stem rod (at most [`MAX_ROD_D`])
/// rests on the ceiling there too.
pub const SEAT_D: f64 = 5.3;
/// Thickest stem rod: it must fit a head's seat, mm.
pub const MAX_ROD_D: f64 = 5.0;

/// Whether `at` is on a tube of the base `sockets` (a full 2×2 corner).
fn on_tube(at: P2, sockets: &[(i32, i32)]) -> bool {
    let (i, j) = ((at[0] / 8.0).round() as i32, (at[1] / 8.0).round() as i32);
    let on_corner =
        (at[0] - f64::from(i) * 8.0).abs() < 1e-6 && (at[1] - f64::from(j) * 8.0).abs() < 1e-6;
    on_corner
        && [(i - 1, j - 1), (i, j - 1), (i - 1, j), (i, j)]
            .iter()
            .all(|c| sockets.contains(c))
}

/// Where the rod's end stops under a head (the bar hole's mouth): the socket's ceiling, mm.
pub(crate) fn head_mouth() -> f64 {
    BASE_HEIGHT - StudProfile::fused().top_thickness
}

/// The seat's command (see [`SEAT_D`]), to run after the tubes; none off a tube. It ends
/// 2 µm over the ceiling (never exactly on it, where booleans can fail).
pub(crate) fn tube_seat(at: P2, sockets: &[(i32, i32)]) -> Vec<Value> {
    if !on_tube(at, sockets) {
        return Vec::new();
    }
    let top = head_mouth() + 0.002;
    vec![cmd(
        "solid.cylinder",
        json!({"radius": r6(SEAT_D / 2.0), "height": r6(top + 1.0), "base": p3([at[0], at[1], -1.0]), "operation": "cut", "targets": ["Part"], "name": "Seat"}),
    )]
}

/// A mount in a part's base or assembled frame (mm, z up).
#[derive(Clone, Copy, Debug)]
pub(crate) struct MountMm {
    pub kind: MountKind,
    pub origin: [f64; 3],
    pub axis: [f64; 3],
    pub up: [f64; 3],
}

impl MountMm {
    /// A head's bar hole on the vertical through `at`, its mouth at `mouth`: the stem comes
    /// from below, the head's front `+x`.
    pub fn head(at: P2, mouth: f64) -> Self {
        Self {
            kind: MountKind::Hole,
            origin: [at[0], at[1], mouth],
            axis: [0.0, 0.0, -1.0],
            up: [1.0, 0.0, 0.0],
        }
    }

    /// In a mesh frame whose origin is `o` (mm, base frame): LDU, `-Y` up.
    pub fn to_ldu(self, o: [f64; 3]) -> StemMount {
        let v = |p: [f64; 3]| [p[0] as f32, -p[2] as f32, p[1] as f32];
        StemMount {
            kind: self.kind,
            origin: v([
                (self.origin[0] - o[0]) * 2.5,
                (self.origin[1] - o[1]) * 2.5,
                (self.origin[2] - o[2]) * 2.5,
            ]),
            axis: v(self.axis),
            up: v(self.up),
        }
    }
}

/// A mount declared in a request (`{"kind": "hole"|"clip", "origin", "axis", "up"}`, mm in
/// the detail's base frame), checked: unit, square vectors.
pub(crate) fn mount_from_json(v: &Value) -> Result<MountMm> {
    let bad = |why: &str| CustomError::InvalidShape(format!("mount: {why}"));
    let kind = match v.get("kind").and_then(Value::as_str) {
        Some("hole") => MountKind::Hole,
        Some("clip") => MountKind::Clip,
        _ => return Err(bad("`kind` is hole or clip")),
    };
    let vec3 = |k: &str| -> Result<[f64; 3]> {
        let a: Vec<f64> = serde_json::from_value(v.get(k).cloned().unwrap_or(Value::Null))
            .map_err(|_| bad(&format!("`{k}` is [x, y, z]")))?;
        <[f64; 3]>::try_from(a).map_err(|_| bad(&format!("`{k}` is [x, y, z]")))
    };
    let unit = |a: [f64; 3]| -> Option<[f64; 3]> {
        let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
        (l > 1e-6).then(|| a.map(|x| x / l))
    };
    let origin = vec3("origin")?;
    let axis = unit(vec3("axis")?).ok_or_else(|| bad("`axis` is zero"))?;
    let up = unit(vec3("up")?).ok_or_else(|| bad("`up` is zero"))?;
    if (axis[0] * up[0] + axis[1] * up[1] + axis[2] * up[2]).abs() > 0.02 {
        return Err(bad("`up` must be square to `axis`"));
    }
    Ok(MountMm {
        kind,
        origin,
        axis,
        up,
    })
}

// ---------------------------------------------------------------- hub

/// The hub's puck and keyed peg holes as decoration on its socket base (base frame), checked
/// first: every hole inside the puck with a [`HUB_WALL`] wall and a floor over the socket.
pub(crate) fn hub_decoration(h: &Hub) -> Result<Decoration> {
    let bad = |why: String| Err(CustomError::InvalidShape(format!("hub: {why}")));
    let p = StudProfile::fused();
    let cells: Vec<(i32, i32)> = h
        .cells
        .iter()
        .map(|&(a, b)| (i32::from(a), i32::from(b)))
        .collect();
    let f = Footprint::new(&cells)?;
    let height = f64::from(h.height);
    if !(BASE_HEIGHT + 1.0..=crate::MAX_PART_MM).contains(&height) {
        return bad(format!(
            "{height:.1} mm thick; {:.1} to {} mm",
            BASE_HEIGHT + 1.0,
            crate::MAX_PART_MM
        ));
    }
    let outer = f.offset_loops(p.pitch, p.clearance);
    let inner_wall = f.offset_loops(p.pitch, p.clearance + HUB_WALL);
    let inside = |loops: &[Vec<P2>], q: P2| {
        let mut c = false;
        for l in loops {
            for k in 0..l.len() {
                let (a, b) = (l[k], l[(k + 1) % l.len()]);
                if (a[1] > q[1]) != (b[1] > q[1])
                    && a[0] + (q[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]) > q[0]
                {
                    c = !c;
                }
            }
        }
        c
    };
    let mut c = vec![sketch_xy("PuckSketch", BASE_HEIGHT - crate::SINK)];
    c.extend(outer.iter().map(|l| closed(l)));
    c.push(finish_sketch());
    c.push(cmd(
        "solid.extrude",
        json!({"sketch": "PuckSketch", "name": "Puck", "distance": r6(height - BASE_HEIGHT + crate::SINK), "operation": "join", "targets": ["Part"]}),
    ));
    let floor = BASE_HEIGHT - p.top_thickness + HUB_FLOOR;
    let rh = PEG_HOLE_D / 2.0;
    let depth = PEG_LEN + 0.5;
    let mut axes: Vec<([f64; 3], [f64; 3])> = Vec::new();
    for (n, hole) in h.holes.iter().enumerate() {
        let n1 = n + 1;
        let top = hole.top.map(f64::from);
        let d = hole.dir.map(f64::from);
        let key = hole.key.map(f64::from);
        let dl = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        if (dl - 1.0).abs() > 1e-3 || d[2] < 0.5 {
            return bad(format!(
                "hole {n1}: `dir` must be a unit vector leaning at most 60° from upright"
            ));
        }
        if (key[0] * d[0] + key[1] * d[1] + key[2] * d[2]).abs() > 0.02 {
            return bad(format!("hole {n1}: `key` must be square to `dir`"));
        }
        if (top[2] - height).abs() > 0.05 {
            return bad(format!(
                "hole {n1}: its top {:.2} mm is not on the hub's top ({height:.2} mm)",
                top[2]
            ));
        }
        let lean = d[2].acos();
        let bottom = [
            top[0] - d[0] * depth,
            top[1] - d[1] * depth,
            top[2] - d[2] * depth,
        ];
        // Lowest point of the hole's wall, over the socket's ceiling with a floor.
        if bottom[2] - rh * lean.sin() < floor - 1e-6 {
            return bad(format!(
                "hole {n1} reaches {:.2} mm, under the {floor:.2} mm floor over the sockets: make the hub thicker",
                bottom[2] - rh * lean.sin()
            ));
        }
        // Its wall's outline, at the top and the bottom, inside the puck with a wall.
        let (e1, e2) = frame(d, None);
        for q in [top, bottom] {
            for i in 0..16 {
                let a = 2.0 * PI * f64::from(i) / 16.0;
                let w = [
                    q[0] + rh * (a.cos() * e1[0] + a.sin() * e2[0]),
                    q[1] + rh * (a.cos() * e1[1] + a.sin() * e2[1]),
                ];
                if !inside(&inner_wall, w) {
                    return bad(format!(
                        "hole {n1} comes within {HUB_WALL} mm of the hub's side"
                    ));
                }
            }
        }
        for (m, (t2, b2)) in axes.iter().enumerate() {
            let dist = seg_dist(top, bottom, *t2, *b2);
            if dist < 2.0 * rh + HOLE_GAP {
                return bad(format!(
                    "holes {} and {n1} are {dist:.2} mm apart at their closest; they need {:.2}",
                    m + 1,
                    2.0 * rh + HOLE_GAP
                ));
            }
        }
        axes.push((top, bottom));
        // The D: round (Ø PEG_HOLE_D) with its flat PEG_HOLE_FLAT out along `key`, from a
        // plane clear above the top down to the bottom.
        let up = rh * lean.tan() + 1.0;
        let start = [top[0] + d[0] * up, top[1] + d[1] * up, top[2] + d[2] * up];
        let ex = key;
        let ey = [
            d[1] * ex[2] - d[2] * ex[1],
            d[2] * ex[0] - d[0] * ex[2],
            d[0] * ex[1] - d[1] * ex[0],
        ];
        let fl = PEG_HOLE_FLAT;
        let w = (rh * rh - fl * fl).sqrt();
        let name = format!("PegHole{n1}");
        c.extend([
            sketch_on(&name, start, ex, ey),
            crate::detail::arc3([fl, -w], [-rh, 0.0], [fl, w]),
            crate::detail::line([fl, w], [fl, -w]),
            finish_sketch(),
            cmd(
                "solid.extrude",
                json!({"sketch": name, "name": format!("{name}Cut"), "distance": r6(depth + up), "direction": "negative", "operation": "cut", "targets": ["Part"]}),
            ),
        ]);
        // Crush ribs on the round side, from under the top down into the floor.
        let under = rh * lean.tan() + 0.5;
        let from = [
            top[0] - d[0] * under,
            top[1] - d[1] * under,
            top[2] - d[2] * under,
        ];
        // Into the floor, so their ends face nothing downward.
        c.extend(ribs(
            &format!("PegRibs{n1}"),
            from,
            d,
            ex,
            depth - under + 0.2,
            PEG_RIB_IN,
            rh,
            &[PI, PI * 2.0 / 3.0, PI * 4.0 / 3.0],
        ));
    }
    Ok((cells, c))
}

/// A detail's base cells and decoration commands.
pub(crate) type Decoration = (Vec<(i32, i32)>, Vec<Value>);

/// Shortest distance between segments `a0a1` and `b0b1`.
fn seg_dist(a0: [f64; 3], a1: [f64; 3], b0: [f64; 3], b1: [f64; 3]) -> f64 {
    let sub = |a: [f64; 3], b: [f64; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let (u, v, w) = (sub(a1, a0), sub(b1, b0), sub(a0, b0));
    let (a, b, c, d, e) = (dot(u, u), dot(u, v), dot(v, v), dot(u, w), dot(v, w));
    let den = a * c - b * b;
    let mut best = f64::MAX;
    // Sample the closed form and the ends (the segments are short; this is exact enough).
    let mut ts: Vec<(f64, f64)> = vec![(0.0, 0.0), (0.0, 1.0), (1.0, 0.0), (1.0, 1.0)];
    if den.abs() > 1e-12 {
        let s = ((b * e - c * d) / den).clamp(0.0, 1.0);
        let t = ((a * e - b * d) / den).clamp(0.0, 1.0);
        ts.push((s, t));
    }
    for i in 0..=20 {
        let s = f64::from(i) / 20.0;
        let t = ((e + b * s) / c.max(1e-12)).clamp(0.0, 1.0);
        ts.push((s, t));
    }
    for (s, t) in ts {
        let p = [a0[0] + u[0] * s, a0[1] + u[1] * s, a0[2] + u[2] * s];
        let q = [b0[0] + v[0] * t, b0[1] + v[1] * t, b0[2] + v[2] * t];
        let dd = sub(p, q);
        best = best.min(dot(dd, dd).sqrt());
    }
    best
}

/// A hub as a detail (`name`, `label`): its socket base, puck and keyed holes, with its
/// source `{"hub": h}`.
pub fn hub_detail(name: &str, label: &str, h: &Hub) -> Result<Detail> {
    let (cells, deco) = hub_decoration(h)?;
    let source = json!({"hub": h});
    crate::detail::assemble_with(name, label, &cells, deco, Vec::new(), None, source)
}

/// Builds a hub's print (see [`hub_detail`]).
pub fn build_hub(h: &Hub) -> Result<CustomPart> {
    Ok(hub_detail("hub", "hub", h)?.part)
}

/// A flower head's stem mount on the vertical through `at` over the base `sockets`: the bar
/// hole's commands (after the decoration), the seat's (after the tubes), the mount, and how
/// high the head's solid must reach on its axis (`through`: a kit hub, whose centre's peg
/// hole continues the bar hole).
pub(crate) struct HeadMount {
    pub cmds: Vec<Value>,
    pub post: Vec<Value>,
    pub mount: MountMm,
    pub top: f64,
}

pub(crate) fn head_mount(at: P2, sockets: &[(i32, i32)], through: bool) -> HeadMount {
    let mouth = head_mouth();
    HeadMount {
        cmds: head_mount_cmds(at, mouth, through),
        post: tube_seat(at, sockets),
        mount: MountMm::head(at, mouth),
        top: if through {
            hub_mount_top(mouth)
        } else {
            head_mount_top(mouth)
        },
    }
}

/// Whether a template's `params` ask for a stem mount (`"mount"`, default true).
pub(crate) fn wants_mount(params: &serde_json::Value) -> bool {
    params
        .get("mount")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true)
}

/// The column radius round a head's bar hole (a 1 mm wall round the bore).
pub(crate) const MOUNT_COLUMN_R: f64 = BAR_BORE_D / 2.0 + 1.0;

// ---------------------------------------------------------------- stem styles

fn sb(
    at: f32,
    angle: f32,
    length: f32,
    bend: f32,
    head: bool,
    tip: Option<StemTip>,
) -> StyleBranch {
    StyleBranch {
        at,
        angle,
        length,
        bend,
        head,
        tip,
    }
}

fn style(
    name: &str,
    diameter: f32,
    color: &str,
    head_on_tip: bool,
    tip: Option<StemTip>,
    branches: Vec<StyleBranch>,
    leaves: Vec<f32>,
) -> StemStyle {
    StemStyle {
        name: name.into(),
        diameter,
        color: color.into(),
        head_on_tip,
        tip,
        branches,
        leaves,
    }
}

/// The stems of the styles that carry no head of their own (they go in a bouquet on their
/// own): `"grass"` and `"pine"`, besides the fern's.
pub const STEM_ONLY: &[&str] = &["grass", "pine"];

/// The stem a design suggests, after LEGO's 10280 Flower Bouquet (see each template's text),
/// for template `template` with (filled-in) `params`; also the stem-only styles in
/// [`STEM_ONLY`]. `None` for designs that don't go on a stem of their own (a leaf clips onto
/// one; eyes, beaks and plates don't).
pub fn stem_style(template: &str, params: &serde_json::Value) -> Option<StemStyle> {
    let bud = |l: f32, w: f32| {
        Some(StemTip::Bud {
            length: l,
            width: w,
        })
    };
    let blade = |l: f32, w: f32| {
        Some(StemTip::Blade {
            length: l,
            width: w,
        })
    };
    let green = "Dark Green";
    Some(match template {
        "rose" => style("rose stem", 5.0, green, true, None, vec![], vec![0.35, 0.6]),
        "daisy" => style(
            "daisy sprig",
            4.0,
            green,
            true,
            None,
            vec![
                sb(0.5, 32.0, 0.32, -12.0, true, None),
                sb(0.68, -30.0, 0.24, 10.0, true, None),
            ],
            vec![],
        ),
        "aster" => style("aster stem", 5.0, green, true, None, vec![], vec![0.45]),
        "gerbera" => style("gerbera stem", 5.0, green, true, None, vec![], vec![]),
        "poppy" => style(
            "poppy stem",
            4.5,
            green,
            true,
            None,
            vec![sb(0.6, 28.0, 0.22, -18.0, false, bud(9.0, 5.0))],
            vec![],
        ),
        "cornflower" => style(
            "cornflower stem",
            3.5,
            green,
            true,
            None,
            vec![
                sb(0.62, -34.0, 0.16, 12.0, false, bud(6.0, 4.0)),
                sb(0.78, 30.0, 0.12, -10.0, false, bud(5.0, 3.8)),
            ],
            vec![],
        ),
        "flower_head" => style("flower stem", 5.0, green, true, None, vec![], vec![0.4]),
        "spike" => match params
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("lupin")
        {
            "lavender" => style(
                "lavender sprig",
                3.5,
                "Reddish Brown",
                true,
                None,
                vec![
                    sb(0.5, 24.0, 0.3, -8.0, true, None),
                    sb(0.62, -26.0, 0.26, 8.0, true, None),
                    sb(0.74, 20.0, 0.18, -6.0, true, None),
                ],
                vec![],
            ),
            kind => style(
                &format!("{kind} stalk"),
                5.0,
                green,
                true,
                None,
                vec![
                    sb(0.8, 30.0, 0.07, -10.0, false, bud(6.0, 4.0)),
                    sb(0.87, -30.0, 0.06, 10.0, false, bud(5.0, 3.8)),
                ],
                vec![0.3],
            ),
        },
        "umbel" => style(
            "umbel stem",
            3.5,
            green,
            true,
            None,
            vec![
                sb(0.74, 34.0, 0.2, -10.0, true, None),
                sb(0.8, -36.0, 0.18, 10.0, true, None),
                sb(0.88, 24.0, 0.1, -6.0, true, None),
            ],
            vec![],
        ),
        "fern" => style(
            "fern fan",
            3.5,
            green,
            false,
            blade(34.0, 6.0),
            vec![
                sb(0.45, 30.0, 0.35, -8.0, false, blade(26.0, 5.0)),
                sb(0.55, -32.0, 0.32, 8.0, false, blade(24.0, 5.0)),
            ],
            vec![],
        ),
        "grass" => style(
            "grass",
            3.5,
            green,
            false,
            blade(60.0, 6.0),
            vec![
                sb(0.3, 22.0, 0.45, -6.0, false, blade(50.0, 5.0)),
                sb(0.4, -24.0, 0.4, 6.0, false, blade(46.0, 5.0)),
            ],
            vec![],
        ),
        "pine" => style(
            "pine",
            3.5,
            green,
            false,
            blade(20.0, 2.6),
            vec![
                sb(0.45, 40.0, 0.25, 0.0, false, blade(16.0, 2.4)),
                sb(0.55, -40.0, 0.25, 0.0, false, blade(16.0, 2.4)),
                sb(0.65, 38.0, 0.2, 0.0, false, blade(14.0, 2.4)),
                sb(0.75, -38.0, 0.2, 0.0, false, blade(14.0, 2.4)),
            ],
            vec![],
        ),
        _ => return None,
    })
}
