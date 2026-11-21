//! Slant 3D's v2 stud: fused studs (pairs and triples), each end lobe with a leaf-spring fin.
//!
//! Every stud is fused: a row's studs are split into **units** of two (a pair, "peanut") or
//! three (a triple) neighbouring studs, joined by narrow waists. See [`layout`] for how a
//! row is split.
//!
//! Frames used here:
//! * **unit frame**: origin at the unit's centre (a pair's waist centre, a triple's middle
//!   stud centre), `v` along the row toward lobe B, `u` toward the fin side;
//! * **lobe frame**: origin at an end lobe's centre, `u` toward the fin side, `v` away from
//!   the waist next to it. Lobe B's frame is the unit frame moved by `+(n − 1)·pitch/2` along
//!   `v` (`n` studs); lobe A's is its mirror image about the unit's centre line `v = 0`.
//!
//! # The outline (lobe B's frame; `a = pitch / 2`, `r = stud_diameter / 2`)
//!
//! A brick pushed on has tubes (radius `R_t = tube_outer_diameter / 2`) at the four diagonal
//! points `(±a, ±a)` around each lobe, and the inside face of its wall at `u = a − clearance −
//! wall`. Every junction below is tangent, so the outline has no corners:
//!
//! 1. **Waist arcs** about `(±a, −a)`, radius `R_w = R_t + waist_clearance`: a tube beside the
//!    waist, on either side, clears it. The waist is `2(a − R_w)` wide.
//! 2. **Fin bump** `F`: the circle with its rightmost point at `(fin_reach, −fin_peak)` that
//!    touches the fin-side waist arc from outside. With `s = a − fin_reach` and
//!    `h = a − fin_peak`, its radius is `ρ = (s² + h² − R_w²) / (2(R_w − s))`.
//! 3. **Blend** `E`: a circle containing the stud circle and touching it at `blend_angle` (the
//!    point `T`), and containing `F`, touching it at `P`. With `d` the unit vector at
//!    `blend_angle` and `c_F` the bump's centre, `E` has centre `−k·d` and radius `r + k`,
//!    `k = ((r − ρ)² − |c_F|²) / (2(c_F·d − (r − ρ)))`.
//! 4. **Stud circle** from `T` over the far end and down the inner side.
//! 5. **Inner round** `G`, radius `waist_round`, inside the stud circle and touching it, and
//!    touching the inner waist arc from outside.
//!
//! Lobe A is lobe B mirrored about the unit's centre line.
//!
//! A **triple's middle lobe** has a waist on each side and no fin. Its outline is the stud
//! circle joined to each of the four waist arcs around it by the round `G` (mirrored in `u`,
//! `v` or both): the inner side's construction on both sides, so it is symmetric, tangent at
//! every junction and, like any stud circle, tangent to the tubes and 0.3 mm clear of a wall.
//! The waist on its fin side runs from the end lobe's bump to the middle lobe's round, with
//! the same tube-clearance arc as a pair's.
//!
//! A unit with **no fin side** (a row with neighbours on both sides, such as a 3-wide brick's
//! middle row) has no fins, slots or pockets: every lobe is the stud circle on both sides,
//! joined to the waists by `G` as above.

use crate::geom::{self, Frame, Loop, Seg};
use crate::profile::{MIN_FIN_SECTION, MIN_POCKET_FLOOR, SLANT_CAP, StudProfile, StudStyle};
use crate::shape::P2;
use crate::{CustomError, Result};
use std::collections::BTreeSet;
use std::f64::consts::{FRAC_PI_4, PI};

/// Radius of the round that carries the pocket's edge from the offset of the fin's blend arc
/// over to the slot's far edge, past the fin's tip, in mm.
const TIP_ROUND: f64 = 0.5;
/// How far inside the slot's far edge the tip round comes nearest it, in mm. The slot's end
/// crosses the round square right there, so the two outlines never run together; from above
/// the relief's edge steps out by this much (too little to show, or to print).
const TIP_GAP: f64 = 0.01;
/// The least room the slot's end, outside the lobe, leaves to the lobe, in mm.
const SLOT_END_ROOM: f64 = 0.02;
/// How far past the root's corner the pocket runs along the root line before its fade turns
/// up into the offset waist arc, in mm (so its edge crosses the stud's waist edge cleanly).
const ROOT_FADE_START: f64 = 0.05;
/// The thinnest rim a pocket may leave to the brick's outside face, in mm.
const MIN_RIM: f64 = 0.6;
/// The least room a pocket's edge leaves to the slot's far wall, in mm (so the pocket never
/// reaches under the stud across the slot).
const POCKET_SLOT_ROOM: f64 = 0.1;
/// How far the pocket outline used for the slot's area runs on below the slot's end, in mm
/// (see [`Lobe::slot_outside_pocket`]).
const PROBE_DROP: f64 = 0.1;

/// One lobe's slot and pocket, and the outlines of the units it belongs to, worked out from a
/// profile.
#[derive(Clone, Debug)]
pub(crate) struct Lobe {
    /// Distance from a lobe centre to the waist line (half the pitch).
    pub a: f64,
    /// The finned pair's outline, in the unit frame.
    pub peanut: Loop,
    /// The finned triple's outline, in the unit frame.
    pub triple: Loop,
    /// A finless pair's outline, in the unit frame.
    pub pair_plain: Loop,
    /// A finless triple's outline, in the unit frame.
    pub triple_plain: Loop,
    /// The slot, in lobe B's frame.
    pub slot: Loop,
    /// The fin's footprint, in lobe B's frame: from the root (the slot's end) to the tip,
    /// between the slot and the lobe's outline.
    pub fin: Loop,
    /// The pocket under the fin, in lobe B's frame: the fin grown by `pocket_margin`.
    pub pocket: Loop,
    /// The pocket's outline with its root end run on [`PROBE_DROP`] below the slot's end,
    /// where the slot has nothing, so it crosses the slot's outline cleanly.
    pocket_probe: Loop,
    /// Pocket depth into the brick's top.
    pub pocket_depth: f64,
    /// How deep the slot cuts into the brick's top (beside the pocket).
    pub groove: f64,
    /// Dimensions for reports.
    pub dims: SlantDims,
}

/// The derived dimensions of a v2 stud (mm; points in a lobe's own frame: origin at its
/// centre, `u` toward the fin side, `v` away from the waist), for reports and tests.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SlantDims {
    /// Radius of the stud circle (inner side and far end).
    pub stud_radius: f64,
    /// How far the fin reaches across the row.
    pub fin_reach: f64,
    /// The fin's outermost point.
    pub fin_peak: P2,
    /// Centre and radius of the fin bump.
    pub bump_center: P2,
    /// Radius of the fin bump.
    pub bump_radius: f64,
    /// Where the bump meets the waist arc.
    pub bump_start: P2,
    /// Centre of the blend arc.
    pub blend_center: P2,
    /// Radius of the blend arc.
    pub blend_radius: f64,
    /// Where the bump meets the blend arc.
    pub blend_start: P2,
    /// Where the blend arc meets the stud circle.
    pub blend_end: P2,
    /// Centre of the inner round.
    pub round_center: P2,
    /// Radius of the inner round.
    pub round_radius: f64,
    /// Waist width.
    pub waist_width: f64,
    /// Radius of the waist's concave sides.
    pub waist_radius: f64,
    /// Slot width.
    pub slot_width: f64,
    /// Slot centre line, outboard of the lobe centre line.
    pub slot_center: f64,
    /// Where the slot's straight part starts and ends (`v`).
    pub slot_v: (f64, f64),
    /// Centre of the hook.
    pub hook_center: P2,
    /// Hook centre-line radius.
    pub hook_radius: f64,
    /// Where the hook's outer edge leaves the stud circle, in degrees from the fin direction:
    /// the cap is the stud circle beyond it.
    pub cap_angle: f64,
    /// Widest fin: from the slot's outer edge to the fin's outermost point.
    pub fin_width: f64,
    /// From the waist line to the slot's end, along the row.
    pub root_length: f64,
    /// The fin root across the row at the slot's end, from the slot to the waist arc.
    pub root_width: f64,
    /// Thinnest solid between the slot and either waist arc.
    pub min_section: f64,
    /// Pocket depth.
    pub pocket_depth: f64,
    /// How far the pocket reaches past the fin all round.
    pub pocket_margin: f64,
    /// Pocket reach outboard of the lobe centre.
    pub pocket_out: f64,
    /// The top's rim between the pocket and the brick's outside face, across the row.
    pub pocket_rim: f64,
    /// Where the fin's free outline starts at its root: the slot's end meets the fin-side
    /// waist arc here. The pocket's edge leaves it along the root line.
    pub root_corner: P2,
    /// Centre of the pocket's root fade: tangent to the root line just past the root's
    /// corner, it sweeps into the offset waist arc, tangent to that too.
    pub root_cap_center: P2,
    /// Radius of the pocket's root fade.
    pub root_cap_radius: f64,
    /// Where the fin's tip leaves the lobe: the hook's inner edge meets the blend arc.
    pub tip_corner: P2,
    /// Centre of the pocket's tip round: tangent to the offset blend arc, it carries the
    /// pocket's edge round the fin's tip to within [`Self::tip_gap`] of the slot's far edge.
    pub tip_round_center: P2,
    /// Radius of the pocket's tip round.
    pub tip_round_radius: f64,
    /// How close the tip round comes to the slot's far edge, where the slot ends.
    pub tip_gap: f64,
    /// Area of one fin's footprint.
    pub fin_area: f64,
    /// Area of one pocket.
    pub pocket_area: f64,
    /// Area of a finned pair (peanut), before its slots are cut.
    pub pair_area: f64,
    /// Area of a finned triple, before its slots are cut.
    pub triple_area: f64,
    /// Area of a finless pair.
    pub pair_plain_area: f64,
    /// Area of a finless triple.
    pub triple_plain_area: f64,
}

fn line(a: P2, b: P2) -> Seg {
    Seg::Line { a, b }
}

fn add(p: P2, q: P2, k: f64) -> P2 {
    [p[0] + k * q[0], p[1] + k * q[1]]
}

fn sub(p: P2, q: P2) -> P2 {
    [p[0] - q[0], p[1] - q[1]]
}

fn norm(p: P2) -> f64 {
    p[0].hypot(p[1])
}

fn unit(p: P2) -> P2 {
    let l = norm(p);
    [p[0] / l, p[1] / l]
}

fn dot(p: P2, q: P2) -> f64 {
    p[0] * q[0] + p[1] * q[1]
}

/// Lobe B's frame point `q` in the peanut frame.
fn up(q: P2, a: f64) -> P2 {
    [q[0], q[1] + a]
}

/// A segment mirrored about the waist line (`v ↦ −v`, peanut frame) and run backwards, so a
/// chain of lobe B read counter-clockwise becomes lobe A's, also counter-clockwise.
fn mirrored(s: &Seg) -> Seg {
    let m = |p: P2| [p[0], -p[1]];
    match *s {
        Seg::Line { a, b } => Seg::Line { a: m(b), b: m(a) },
        Seg::Arc { a, b, c, ccw } => Seg::Arc {
            a: m(b),
            b: m(a),
            c: m(c),
            ccw,
        },
    }
}

/// A unit's outline (unit frame, counter-clockwise) from lobe B's chain `end` (lobe frame,
/// from the fin-side waist arc round the far end to the inner waist arc) and, for a triple,
/// the middle lobe's fin side `mid_right` (lower waist to upper) and inner side `mid_left`
/// (upper waist to lower). Lobe A is lobe B mirrored about `v = 0`; neighbouring lobes are
/// joined by the waist arcs about `(±a, w)`, `w` each waist's position.
fn assemble(n: usize, end: &[Seg], mid_right: &[Seg], mid_left: &[Seg], a: f64) -> Loop {
    let off = (n as f64 - 1.0) * a;
    let shift = |s: &Seg| match *s {
        Seg::Line { a: p0, b: p1 } => line(up(p0, off), up(p1, off)),
        Seg::Arc {
            a: p0,
            b: p1,
            c,
            ccw,
        } => Seg::Arc {
            a: up(p0, off),
            b: up(p1, off),
            c: up(c, off),
            ccw,
        },
    };
    let b_chain: Vec<Seg> = end.iter().map(shift).collect();
    let a_chain: Vec<Seg> = b_chain.iter().rev().map(mirrored).collect();
    let start = |l: &[Seg]| match l[0] {
        Seg::Line { a, .. } | Seg::Arc { a, .. } => a,
    };
    let finish = |l: &[Seg]| match l[l.len() - 1] {
        Seg::Line { b, .. } | Seg::Arc { b, .. } => b,
    };
    let waist = |k: usize| (2.0 * k as f64 + 2.0 - n as f64) * a;
    let mut out = a_chain.clone();
    // Up the fin side.
    let mut at = finish(&a_chain);
    for k in 0..n - 1 {
        let last = k == n - 2;
        let next = if last {
            start(&b_chain)
        } else {
            start(mid_right)
        };
        out.push(Seg::Arc {
            a: at,
            b: next,
            c: [a, waist(k)],
            ccw: false,
        });
        if !last {
            out.extend_from_slice(mid_right);
            at = finish(mid_right);
        }
    }
    out.extend_from_slice(&b_chain);
    // Down the inner side.
    let mut at = finish(&b_chain);
    for k in (0..n - 1).rev() {
        let last = k == 0;
        let next = if last {
            start(&a_chain)
        } else {
            start(mid_left)
        };
        out.push(Seg::Arc {
            a: at,
            b: next,
            c: [-a, waist(k)],
            ccw: false,
        });
        if !last {
            out.extend_from_slice(mid_left);
            at = finish(mid_left);
        }
    }
    out
}

impl Lobe {
    /// The v2 geometry of `p`, or why its numbers don't make one.
    pub fn new(p: &StudProfile) -> Result<Self> {
        let StudStyle::SlantV2 {
            fin_reach,
            fin_peak,
            blend_angle,
            waist_clearance,
            waist_round,
            slot_width,
            slot_offset,
            root_width,
            hook_radius,
            pocket_depth,
            pocket_margin,
        } = p.stud_style
        else {
            return Err(CustomError::InvalidProfile(
                "not a Slant v2 stud profile".into(),
            ));
        };
        let bad = |why: String| Err(CustomError::InvalidProfile(format!("Slant v2 stud: {why}")));
        for (name, v) in [
            ("fin_reach", fin_reach),
            ("blend_angle", blend_angle),
            ("waist_clearance", waist_clearance),
            ("waist_round", waist_round),
            ("slot_width", slot_width),
            ("root_width", root_width),
            ("hook_radius", hook_radius),
            ("pocket_depth", pocket_depth),
            ("pocket_margin", pocket_margin),
        ] {
            if !(v.is_finite() && v > 0.0) {
                return bad(format!("{name} must be a positive number, got {v}"));
            }
        }
        for (name, v) in [("slot_offset", slot_offset), ("fin_peak", fin_peak)] {
            if !(v.is_finite() && v >= 0.0) {
                return bad(format!("{name} must be 0 or more mm, got {v}"));
            }
        }
        let r = p.stud_diameter / 2.0;
        let a = p.pitch / 2.0;
        let t = slot_width;
        let hw = t / 2.0;
        if p.top_thickness < pocket_depth + MIN_POCKET_FLOOR - 1e-9 {
            return bad(format!(
                "a {pocket_depth} mm pocket needs a top at least {:.2} mm thick (it is {})",
                pocket_depth + MIN_POCKET_FLOOR,
                p.top_thickness
            ));
        }
        // The fin presses into the wall of a brick pushed on by `preload`; it must stay inside
        // the room its slot gives it.
        let wall_face = a - p.clearance - p.wall;
        let preload = fin_reach - wall_face;
        if fin_reach <= r || preload >= slot_width - 0.05 {
            return bad(format!(
                "fin_reach {fin_reach} mm must be between the stud radius {r} and {:.2} (the wall's inside face {wall_face:.2} plus the slot less 0.05)",
                wall_face + slot_width - 0.05
            ));
        }
        if fin_peak >= r {
            return bad(format!("fin_peak {fin_peak} mm must be under {r}"));
        }

        // 1. Waist arcs about (±a, −a).
        let rw = p.tube_outer_diameter / 2.0 + waist_clearance;
        if a - rw < 0.3 {
            return bad(format!(
                "the waist would be {:.2} mm wide: tubes this big leave no waist",
                2.0 * (a - rw)
            ));
        }
        let tf = [a, -a];
        let ti = [-a, -a];
        // 2. Fin bump: rightmost point (fin_reach, −fin_peak), touching the waist arc outside.
        let (s, h) = (a - fin_reach, a - fin_peak);
        let rho = (s * s + h * h - rw * rw) / (2.0 * (rw - s));
        if !(rho > 0.05 && rho < fin_reach) {
            return bad(format!(
                "the fin's bump comes out {rho:.3} mm in radius (fin_peak too close to the waist)"
            ));
        }
        let cf = [fin_reach - rho, -fin_peak];
        let j = add(cf, unit(sub(tf, cf)), rho);
        // 3. Blend: contains the stud circle and the bump, touching each.
        let th = blend_angle.to_radians();
        let d = [th.cos(), th.sin()];
        let den = 2.0 * (dot(cf, d) - (r - rho));
        let k = ((r - rho).powi(2) - dot(cf, cf)) / den;
        if !(den < 0.0 && k.is_finite() && k > 0.0 && k < 100.0) {
            return bad(format!(
                "no arc blends the fin into the stud circle at {blend_angle}° (raise blend_angle or fin_peak)"
            ));
        }
        let ce = [-k * d[0], -k * d[1]];
        let re = r + k;
        let pp = add(cf, unit(sub(cf, ce)), rho);
        let tt = [r * d[0], r * d[1]];
        if !(pp[1] > -fin_peak && pp[1] < tt[1]) {
            return bad("the fin's blend starts outside its bump".into());
        }
        // 5. Inner round: inside the stud circle, outside the inner waist arc.
        let dg = r - waist_round;
        let dd = norm(ti);
        let cos_delta = (dg * dg + dd * dd - (rw + waist_round).powi(2)) / (2.0 * dg * dd);
        if !(dg > 0.0 && (-1.0..=1.0).contains(&cos_delta)) {
            return bad(format!(
                "a {waist_round} mm round does not fit between the stud and the waist"
            ));
        }
        let psi = PI + FRAC_PI_4 - cos_delta.acos();
        let dir_g = [psi.cos(), psi.sin()];
        let cg = [dg * dir_g[0], dg * dir_g[1]];
        let g1 = [r * dir_g[0], r * dir_g[1]];
        let g2 = add(cg, unit(sub(ti, cg)), waist_round);

        // Lobe B's chain, from the fin-side waist arc round to the inner one (lobe frame).
        let chain = [
            Seg::Arc {
                a: j,
                b: pp,
                c: cf,
                ccw: true,
            },
            Seg::Arc {
                a: pp,
                b: tt,
                c: ce,
                ccw: true,
            },
            Seg::Arc {
                a: tt,
                b: g1,
                c: [0.0, 0.0],
                ccw: true,
            },
            Seg::Arc {
                a: g1,
                b: g2,
                c: cg,
                ccw: true,
            },
        ];
        // A finless end lobe: the inner round mirrored onto the fin side, the stud circle
        // round the far end, the inner round (lobe frame, from the fin-side waist arc to the
        // inner one).
        let mu = |q: P2| [-q[0], q[1]];
        let plain = [
            Seg::Arc {
                a: mu(g2),
                b: mu(g1),
                c: mu(cg),
                ccw: true,
            },
            Seg::Arc {
                a: mu(g1),
                b: g1,
                c: [0.0, 0.0],
                ccw: true,
            },
            Seg::Arc {
                a: g1,
                b: g2,
                c: cg,
                ccw: true,
            },
        ];
        // A triple's middle lobe, in its own frame (the unit frame): its fin side from the
        // lower waist to the upper one, and its inner side from the upper waist to the lower.
        let mv = |q: P2| [q[0], -q[1]];
        let muv = |q: P2| [-q[0], -q[1]];
        let mid_right = [
            Seg::Arc {
                a: mu(g2),
                b: mu(g1),
                c: mu(cg),
                ccw: true,
            },
            Seg::Arc {
                a: mu(g1),
                b: muv(g1),
                c: [0.0, 0.0],
                ccw: true,
            },
            Seg::Arc {
                a: muv(g1),
                b: muv(g2),
                c: muv(cg),
                ccw: true,
            },
        ];
        let mid_left = [
            Seg::Arc {
                a: mv(g2),
                b: mv(g1),
                c: mv(cg),
                ccw: true,
            },
            Seg::Arc {
                a: mv(g1),
                b: g1,
                c: [0.0, 0.0],
                ccw: true,
            },
            Seg::Arc {
                a: g1,
                b: g2,
                c: cg,
                ccw: true,
            },
        ];
        let peanut = assemble(2, &chain, &mid_right, &mid_left, a);
        let triple = assemble(3, &chain, &mid_right, &mid_left, a);
        let pair_plain = assemble(2, &plain, &mid_right, &mid_left, a);
        let triple_plain = assemble(3, &plain, &mid_right, &mid_left, a);

        // The slot (lobe B's frame) starts where the fin's root is `root_width` across.
        let uc = slot_offset + hw;
        let fin_edge = uc + hw;
        let across = a - fin_edge - root_width;
        if across <= 0.0 || across >= rw {
            return bad("the fin root does not fit beside the slot".into());
        }
        let v0 = -a + (rw * rw - across * across).sqrt();
        if v0 > j[1] {
            return bad("the slot starts beyond the fin's bump (smaller root_width)".into());
        }
        let min_section = norm(sub([fin_edge, v0], tf)) - rw;
        let inner_section = norm(sub([uc - hw, v0], ti)) - rw;
        let min_section = min_section.min(inner_section);
        if min_section < MIN_FIN_SECTION {
            return bad(format!(
                "the slot leaves {min_section:.2} mm to the waist (under {MIN_FIN_SECTION})"
            ));
        }
        let v_s = r - SLANT_CAP - hw - hook_radius;
        if v_s <= v0 + 0.2 {
            return bad(
                "the hook leaves no straight slot (smaller hook_radius or root_width)".into(),
            );
        }
        if hook_radius <= hw + 0.2 {
            return bad("the hook is tighter than the slot is wide".into());
        }
        let hc = [uc + hook_radius, v_s];
        // Where the hook's outer edge leaves the stud circle: the cap is the stud circle
        // beyond it, so the blend must end before it.
        let cap_angle = {
            let (ro, l) = (hook_radius + hw, norm(hc));
            let x = (r * r - ro * ro + l * l) / (2.0 * l);
            let y = (r * r - x * x).max(0.0).sqrt();
            let e = unit(hc);
            let q = [x * e[0] - y * e[1], x * e[1] + y * e[0]];
            q[1].atan2(q[0]).to_degrees()
        };
        if cap_angle < blend_angle {
            return bad(format!(
                "the blend ends at {blend_angle}°, past the slot's mouth (the cap starts at {cap_angle:.1}°)"
            ));
        }
        let (ri, ro) = (hook_radius - hw, hook_radius + hw);
        // The hook turns 135°: its inner edge ends at `p4`, heading 45° back toward the
        // waist, already outside the lobe (which lies inside the blend circle there).
        let at_hook = |rad: f64, t: f64| [hc[0] + rad * t.cos(), hc[1] + rad * t.sin()];
        let hook_angle = |q: P2| (q[1] - hc[1]).atan2(q[0] - hc[0]);
        let p4 = at_hook(ri, FRAC_PI_4);
        if norm(sub(p4, ce)) < re + SLOT_END_ROOM {
            return bad(
                "the hook's inner edge must leave the lobe before its 45° end (a larger hook_radius or a smaller blend_angle)"
                    .into(),
            );
        }

        // The fin: from the slot's end (the root, p2 to the waist arc at `root`) round the
        // lobe's outline to where the hook's inner edge leaves the blend arc (`tip`), back
        // along the slot. Its free outline is everything but the root line. The hook's inner
        // circle meets the blend circle twice; the tip is the crossing on the hook's arc (45°
        // to 180° about its centre).
        let root = [fin_edge + root_width, v0];
        let on_hook = |q: P2| (FRAC_PI_4..=PI).contains(&hook_angle(q));
        let Some(tip) =
            geom::circles_meet(hc, ri, ce, re).and_then(|qs| qs.into_iter().find(|&q| on_hook(q)))
        else {
            return bad("the slot's hook misses the fin's blend arc".into());
        };
        let blend_at = |q: P2| (q[1] - ce[1]).atan2(q[0] - ce[0]);
        if !(blend_at(pp) < blend_at(tip) && blend_at(tip) < blend_at(tt)) {
            return bad("the slot leaves the lobe off the fin's blend arc".into());
        }
        let p1 = [uc - hw, v0];
        let p2 = [uc + hw, v0];
        let p3 = [uc + hw, v_s];
        let p8 = [uc - hw, v_s];
        let fin = vec![
            line(p2, root),
            Seg::Arc {
                a: root,
                b: j,
                c: tf,
                ccw: false,
            },
            Seg::Arc {
                a: j,
                b: pp,
                c: cf,
                ccw: true,
            },
            Seg::Arc {
                a: pp,
                b: tip,
                c: ce,
                ccw: true,
            },
            Seg::Arc {
                a: tip,
                b: p3,
                c: hc,
                ccw: true,
            },
            line(p3, p2),
        ];

        // Pocket: the fin grown by `m`, its outline one smooth line from the root round the
        // tip and into the slot. Each free edge of the fin moves out by `m` (arcs to
        // concentric arcs: the waist arc, concave, to radius `rw − m`; on the slot side the
        // edges lie inside the slot, `m < slot_width`), joined tangent at every turn:
        // * **Root fade.** The pocket stops on the root line (the slot's end line), so nothing
        //   under the root is cut. It runs along that line [`ROOT_FADE_START`] past the root's
        //   corner (crossing the stud's waist edge there cleanly, never along it) to `t1`, and
        //   turns up on the circle tangent to the line at `t1` and to the offset waist arc from
        //   outside: with `h0 = v0 + a` and `s1 = a − t1.u`, `|c_r − tf| = rw − m + ρ_r` gives
        //   `ρ_r = (s1² + h0² − (rw − m)²) / (2·(rw − m − h0))`. The band between the stud's
        //   waist edge and the pocket's edge so tapers to nothing at the root's corner.
        // * **Tip round.** The offset blend arc runs on past the fin's tip, and a round of
        //   radius [`TIP_ROUND`] carries it over into the slot: inside the offset blend circle
        //   and inside the circle [`TIP_GAP`] within the slot's far (outer) edge, touching
        //   both (`|c_t − ce| = re + m − ρ_t`, `|c_t − hc| = ro − TIP_GAP − ρ_t`). It touches
        //   the second at `y`, next to the slot's far edge, so from above the relief's edge
        //   runs from the offset blend round to the slot's far edge with no notch. Past `y`
        //   the round runs on inside the slot to `k` on the hook's offset (radius `ri + m`,
        //   the slot's centre line), out of sight.
        // The slot ends inside the pocket (see the slot below), so only these lines show.
        let m = pocket_margin;
        if m >= slot_width - POCKET_SLOT_ROOM {
            return bad(format!(
                "pocket_margin {m} mm would carry the pocket across the {slot_width} mm slot (keep it under {:.2})",
                slot_width - POCKET_SLOT_ROOM
            ));
        }
        let out_of = |q: P2, n: P2| add(q, n, m);
        let n_j = unit(sub(tf, j));
        let n_pp = unit(sub(pp, cf));
        // Root fade.
        let h0 = v0 + a;
        let t1 = [root[0] + ROOT_FADE_START, v0];
        let s1 = a - t1[0];
        let rr = (s1 * s1 + h0 * h0 - (rw - m).powi(2)) / (2.0 * (rw - m - h0));
        if !(rw - m > h0 && rr > 0.0) {
            return bad("the pocket's root fade does not fit beside the root".into());
        }
        let c_r = [t1[0], v0 + rr];
        let t2 = add(tf, unit(sub(c_r, tf)), rw - m);
        let waist_at = |q: P2| (q[1] - tf[1]).atan2(q[0] - tf[0]);
        if waist_at(t2) <= waist_at(out_of(j, n_j)) {
            return bad("the pocket's root fade runs past the fin's bump".into());
        }
        // Tip round.
        let rt = TIP_ROUND;
        let r_far = ro - TIP_GAP;
        let Some(c_t) = geom::circles_meet(ce, re + m - rt, hc, r_far - rt)
            .and_then(|qs| qs.into_iter().find(|&q| on_hook(q)))
        else {
            return bad("no round carries the pocket from the fin's tip into the slot".into());
        };
        let z = add(ce, unit(sub(c_t, ce)), re + m);
        let toward_far = unit(sub(c_t, hc));
        let y = add(hc, toward_far, r_far);
        let theta_y = hook_angle(y);
        let Some(k) = geom::circles_meet(c_t, rt, hc, ri + m)
            .and_then(|qs| qs.into_iter().find(|&q| hook_angle(q) > theta_y))
        else {
            return bad("the pocket's tip round misses the slot's centre line".into());
        };
        if !(blend_at(z) > blend_at(tip) && theta_y > FRAC_PI_4 && hook_angle(k) < PI) {
            return bad("the pocket's tip round does not fit round the fin's tip".into());
        }
        let round = Seg::Arc {
            a: z,
            b: k,
            c: c_t,
            ccw: true,
        };
        let slot_side = |v: f64| [fin_edge - m, v];
        let pocket_from = |start: P2| {
            vec![
                line(start, t1),
                Seg::Arc {
                    a: t1,
                    b: t2,
                    c: c_r,
                    ccw: true,
                },
                Seg::Arc {
                    a: t2,
                    b: out_of(j, n_j),
                    c: tf,
                    ccw: false,
                },
                Seg::Arc {
                    a: out_of(j, n_j),
                    b: out_of(pp, n_pp),
                    c: cf,
                    ccw: true,
                },
                Seg::Arc {
                    a: out_of(pp, n_pp),
                    b: z,
                    c: ce,
                    ccw: true,
                },
                round,
                Seg::Arc {
                    a: k,
                    b: slot_side(v_s),
                    c: hc,
                    ccw: true,
                },
                line(slot_side(v_s), start),
            ]
        };
        let pocket = pocket_from(slot_side(v0));
        let pocket_probe = pocket_from(slot_side(v0 - PROBE_DROP));
        // Between `z` and `y` the round is the pocket's edge beside the stud's cap: it must
        // stay off it (past `y` it lies in the slot).
        let to_y = Seg::Arc {
            a: z,
            b: y,
            c: c_t,
            ccw: true,
        };
        if (0..=64).any(|i| geom::contains(&peanut, up(to_y.at(f64::from(i) / 64.0), a))) {
            return bad("the pocket's tip round runs under the stud's cap".into());
        }

        // Slot, in lobe B's frame: straight from its end at the root up along `v`, the hook
        // (inner edge from `p3` to its 45° end `p4`; far edge back from `y_far` to `p8`), and
        // an end that lies inside the pocket, wholly outside the lobe, so all the slot shows
        // of its end from above is the pocket's edge: a straight cut from `p4` to `y_in`, then
        // out along the radius through `y` to the far edge at `y_far`, crossing the pocket's
        // tip round square at `y`. The step that leaves in the relief's edge at `y` is
        // [`TIP_GAP`].
        let y_in = add(hc, toward_far, r_far - TIP_GAP);
        let y_far = add(hc, toward_far, ro);
        let end = line(p4, y_in);
        if geom::segment_distance(ce, p4, y_in) < re + SLOT_END_ROOM
            || pocket.iter().any(|s| geom::meets(s, &end))
        {
            return bad("the slot's end does not fit inside the pocket, clear of the lobe".into());
        }
        let slot = vec![
            line(p1, p2),
            line(p2, p3),
            Seg::Arc {
                a: p3,
                b: p4,
                c: hc,
                ccw: false,
            },
            end,
            line(y_in, y_far),
            Seg::Arc {
                a: y_far,
                b: p8,
                c: hc,
                ccw: true,
            },
            line(p8, p1),
        ];
        let (_, hi) = geom::bbox(&pocket);
        let u_out = hi[0];
        let rim = a - p.clearance - u_out;
        if rim < MIN_RIM - 1e-9 {
            return bad(format!(
                "the pocket reaches {u_out:.2} mm outboard, leaving {rim:.2} mm (under {MIN_RIM}) to the brick's side"
            ));
        }

        let dims = SlantDims {
            stud_radius: r,
            fin_reach,
            fin_peak: [fin_reach, -fin_peak],
            bump_center: cf,
            bump_radius: rho,
            bump_start: j,
            blend_center: ce,
            blend_radius: re,
            blend_start: pp,
            blend_end: tt,
            round_center: cg,
            round_radius: waist_round,
            waist_width: 2.0 * (a - rw),
            waist_radius: rw,
            slot_width: t,
            slot_center: uc,
            slot_v: (v0, v_s),
            hook_center: hc,
            hook_radius,
            cap_angle,
            fin_width: fin_reach - fin_edge,
            root_length: v0 + a,
            root_width,
            min_section,
            pocket_depth,
            pocket_margin: m,
            pocket_out: u_out,
            pocket_rim: rim,
            root_corner: root,
            root_cap_center: c_r,
            root_cap_radius: rr,
            tip_corner: tip,
            tip_round_center: c_t,
            tip_round_radius: rt,
            tip_gap: TIP_GAP,
            fin_area: geom::area(&fin),
            pocket_area: geom::area(&pocket),
            pair_area: geom::area(&peanut),
            triple_area: geom::area(&triple),
            pair_plain_area: geom::area(&pair_plain),
            triple_plain_area: geom::area(&triple_plain),
        };
        Ok(Self {
            a,
            peanut,
            triple,
            pair_plain,
            triple_plain,
            slot,
            fin,
            pocket,
            pocket_probe,
            pocket_depth,
            // The slot cuts the top as deep as the pocket beside it, so the relief has
            // one flat floor (no step along the slot).
            groove: pocket_depth,
            dims,
        })
    }

    /// Lobe B's frame in the peanut frame.
    fn lobe_b(&self) -> Frame {
        Frame {
            o: [0.0, self.a],
            x: [1.0, 0.0],
            y: [0.0, 1.0],
        }
    }

    /// The outline of a unit of `n` studs (2 or 3), with or without fins, in the unit frame.
    pub fn outline(&self, n: usize, finned: bool) -> &Loop {
        match (n, finned) {
            (2, true) => &self.peanut,
            (2, false) => &self.pair_plain,
            (_, true) => &self.triple,
            (_, false) => &self.triple_plain,
        }
    }

    /// Area of a pocket.
    pub fn pocket_area(&self) -> f64 {
        geom::area(&self.pocket)
    }

    /// Area of a slot inside its peanut (what the slot takes out of the stud).
    pub fn slot_in_peanut(&self) -> f64 {
        geom::intersection_area(&self.lobe_b().map_loop(&self.slot), &self.peanut)
    }

    /// Area of a slot outside its pocket (where the slot grooves the brick's top).
    ///
    /// The pocket's root line runs along the slot's end, so their outlines share a stretch
    /// there, which [`geom::intersection_area`] can't take. The probe outline is the pocket's
    /// with that end run on below the slot's end, where the slot has nothing: it shares the
    /// same area with the slot and crosses its outline cleanly.
    pub fn slot_outside_pocket(&self) -> f64 {
        geom::area(&self.slot) - geom::intersection_area(&self.slot, &self.pocket_probe)
    }

    /// Volume one lobe's slot removes: through the stud, and a groove into the top beside
    /// the pocket.
    pub fn slot_volume(&self, stud_height: f64) -> f64 {
        self.slot_in_peanut() * stud_height + self.slot_outside_pocket() * self.groove
    }

    /// Whether lobe B's frame point `q` is on a fin (over the pocket, past the slot's end).
    pub fn on_fin(&self, q: P2) -> bool {
        q[1] > self.dims.slot_v.0 + 1e-9 && geom::contains(&self.pocket, q)
    }
}

/// A unit of fused studs: two (a pair) or three (a triple) neighbouring cells of one row,
/// and the direction its fins face (none for a row with neighbours on both sides).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Unit {
    /// Lobe A's cell, the first along the row.
    pub first: (i32, i32),
    /// Unit step (cells) along the row, from lobe A toward lobe B.
    pub step: (i32, i32),
    /// Number of studs: 2 or 3.
    pub n: usize,
    /// Unit step (cells) toward the fin side, if the unit has fins.
    pub fin: Option<(i32, i32)>,
}

impl Unit {
    /// The unit's cells, from lobe A to lobe B.
    pub fn cells(&self) -> Vec<(i32, i32)> {
        (0..self.n as i32)
            .map(|k| {
                (
                    self.first.0 + k * self.step.0,
                    self.first.1 + k * self.step.1,
                )
            })
            .collect()
    }

    /// The unit frame in the plane of the part (`u` toward the fin side, or for a finless
    /// unit the step turned a quarter clockwise).
    pub fn frame(&self, pitch: f64) -> Frame {
        let c = self.cells();
        let centre = |c: (i32, i32)| {
            [
                (f64::from(c.0) + 0.5) * pitch,
                (f64::from(c.1) + 0.5) * pitch,
            ]
        };
        let (ca, cb) = (centre(c[0]), centre(c[c.len() - 1]));
        let y = [f64::from(self.step.0), f64::from(self.step.1)];
        let x = match self.fin {
            Some(f) => [f64::from(f.0), f64::from(f.1)],
            None => [y[1], -y[0]],
        };
        Frame {
            o: [(ca[0] + cb[0]) / 2.0, (ca[1] + cb[1]) / 2.0],
            x,
            y,
        }
    }

    /// The frames of the lobes that carry fins (B, then A mirrored about the unit's centre
    /// line), in the part's plane: none for a finless unit.
    pub fn lobe_frames(&self, pitch: f64) -> Vec<Frame> {
        if self.fin.is_none() {
            return Vec::new();
        }
        let f = self.frame(pitch);
        let off = (self.n as f64 - 1.0) * pitch / 2.0;
        vec![
            Frame {
                o: f.map([0.0, off]),
                x: f.x,
                y: f.y,
            },
            Frame {
                o: f.map([0.0, -off]),
                x: f.x,
                y: [-f.y[0], -f.y[1]],
            },
        ]
    }
}

/// How a run of `len` studs (at least 2) splits into units, as their sizes in order along
/// the row: pairs, and for an odd run one triple, as close to the middle as the run's parity
/// allows. Of the `k = (len − 3) / 2` pairs an odd run has, `⌈k / 2⌉` come before the
/// triple: 3 → T, 5 → P T, 7 → P T P, 9 → P P T P, 11 → P P T P P.
pub(crate) fn split_run(len: usize) -> Vec<usize> {
    if len.is_multiple_of(2) {
        return vec![2; len / 2];
    }
    let k = (len - 3) / 2;
    let before = k.div_ceil(2);
    let mut out = vec![2; before];
    out.push(3);
    out.extend(std::iter::repeat_n(2, k - before));
    out
}

/// Splits `studs` into units of fused studs. Every stud belongs to one: none stays round.
///
/// Studs run in rows along the longer side of the stud area (ties: along Y). Each row's
/// studs split into maximal runs of neighbours, and each run into units by [`split_run`].
/// A unit's fins face a side where neither of its end cells (the cells whose lobes carry
/// fins) has a neighbour in `cells` (the whole part), so they press on the outside wall of
/// a brick pushed on: the positive side when that is open, else the negative side (so
/// single rows face +X, or +Y when they run along X). A unit with neighbours on both sides
/// has no fins.
///
/// A run of one stud cannot be fused. If rows along the first choice of axis leave one, the
/// other axis is tried; if both do, the part is refused, naming the lone studs.
pub(crate) fn layout(cells: &BTreeSet<(i32, i32)>, studs: &[(i32, i32)]) -> Result<Vec<Unit>> {
    let set: BTreeSet<(i32, i32)> = studs.iter().copied().collect();
    let span = |k: fn(&(i32, i32)) -> i32| {
        let lo = set.iter().map(k).min().unwrap_or(0);
        let hi = set.iter().map(k).max().unwrap_or(0);
        hi - lo
    };
    let first = span(|c| c.1) >= span(|c| c.0);
    let mut lone = Vec::new();
    for along_y in [first, !first] {
        match layout_along(cells, &set, along_y) {
            Ok(units) => return Ok(units),
            Err(cells) => {
                if lone.is_empty() {
                    lone = cells;
                }
            }
        }
    }
    let list: Vec<String> = lone.iter().map(|c| format!("({}, {})", c.0, c.1)).collect();
    Err(CustomError::InvalidShape(format!(
        "fused studs come in pairs or triples, and the stud{} at {} would stand alone in its row \
         (a 1×1, a 1-wide slope's single stud, or a footprint cell with no neighbour along \
         its row): use a real part, or add a neighbouring stud",
        if list.len() == 1 { "" } else { "s" },
        list.join(", ")
    )))
}

/// Fused units given from outside (by the packer, which chose them for the part's spot), as
/// [`Unit`]s: each must be 2 or 3 neighbouring cells of one row, inside `cells`, and no two
/// may share a cell. Fins face a side where neither end cell has a neighbour in `cells`, the
/// positive side when both are open, as in [`layout`]; a unit hemmed in on both sides has
/// none.
pub(crate) fn units_from(
    cells: &BTreeSet<(i32, i32)>,
    given: &[Vec<(i32, i32)>],
) -> Result<Vec<Unit>> {
    let bad = |why: String| Err(CustomError::InvalidShape(why));
    let mut used: BTreeSet<(i32, i32)> = BTreeSet::new();
    let mut out = Vec::new();
    for (k, u) in given.iter().enumerate() {
        if !(u.len() == 2 || u.len() == 3) {
            return bad(format!(
                "unit {k} has {} studs; fused units are pairs or triples",
                u.len()
            ));
        }
        let mut u = u.clone();
        u.sort_unstable();
        let step = (u[1].0 - u[0].0, u[1].1 - u[0].1);
        if !(step == (1, 0) || step == (0, 1))
            || u.windows(2).any(|w| (w[1].0 - w[0].0, w[1].1 - w[0].1) != step)
        {
            return bad(format!(
                "unit {k} {u:?} is not a row of neighbouring cells"
            ));
        }
        for c in &u {
            if !cells.contains(c) {
                return bad(format!("unit {k}: cell {c:?} is not part of the footprint"));
            }
            if !used.insert(*c) {
                return bad(format!("unit {k}: cell {c:?} is in two units"));
            }
        }
        let cross = (step.1, step.0);
        let ends = [u[0], u[u.len() - 1]];
        let open = |d: (i32, i32)| {
            ends.iter()
                .all(|q| !cells.contains(&(q.0 + d.0, q.1 + d.1)))
        };
        let fin = if open(cross) {
            Some(cross)
        } else if open((-cross.0, -cross.1)) {
            Some((-cross.0, -cross.1))
        } else {
            None
        };
        out.push(Unit {
            first: u[0],
            step,
            n: u.len(),
            fin,
        });
    }
    Ok(out)
}

/// The units of `set` in rows along Y (or X), or the studs left alone in their row.
fn layout_along(
    cells: &BTreeSet<(i32, i32)>,
    set: &BTreeSet<(i32, i32)>,
    along_y: bool,
) -> std::result::Result<Vec<Unit>, Vec<(i32, i32)>> {
    let (step, cross) = if along_y {
        ((0, 1), (1, 0))
    } else {
        ((1, 0), (0, 1))
    };
    let key = |c: &(i32, i32)| if along_y { (c.0, c.1) } else { (c.1, c.0) };
    let mut order: Vec<(i32, i32)> = set.iter().copied().collect();
    order.sort_by_key(key);
    // Maximal runs of neighbours along the row.
    let mut runs: Vec<Vec<(i32, i32)>> = Vec::new();
    for c in order {
        match runs.last_mut() {
            Some(run) if run.last() == Some(&(c.0 - step.0, c.1 - step.1)) => run.push(c),
            _ => runs.push(vec![c]),
        }
    }
    let lone: Vec<(i32, i32)> = runs.iter().filter(|r| r.len() == 1).map(|r| r[0]).collect();
    if !lone.is_empty() {
        return Err(lone);
    }
    let mut units = Vec::new();
    for run in runs {
        let mut i = 0;
        for n in split_run(run.len()) {
            let ends = [run[i], run[i + n - 1]];
            let open = |d: (i32, i32)| {
                ends.iter()
                    .all(|q| !cells.contains(&(q.0 + d.0, q.1 + d.1)))
            };
            let fin = if open(cross) {
                Some(cross)
            } else if open((-cross.0, -cross.1)) {
                Some((-cross.0, -cross.1))
            } else {
                None
            };
            units.push(Unit {
                first: run[i],
                step,
                n,
                fin,
            });
            i += n;
        }
    }
    Ok(units)
}

/// The fin region of lobe B (lobe frame): inside the peanut, on the fin side of the slot,
/// from the slot's end out. For tests: it must lie wholly over the pocket.
#[cfg(test)]
pub(crate) fn fin_probe_points(l: &Lobe) -> Vec<P2> {
    let mut pts = Vec::new();
    let to_peanut = |q: P2| [q[0], q[1] + l.a];
    let n = 400;
    for i in 0..=n {
        // Rows from just above the root line (the fin starts there; the line itself is the
        // pocket's edge).
        for j in 0..=n {
            let q = [
                -0.1 + 3.0 * f64::from(i) / f64::from(n),
                l.dims.slot_v.0
                    + 1e-6
                    + (l.dims.stud_radius + 0.1 - l.dims.slot_v.0) * f64::from(j) / f64::from(n),
            ];
            if geom::contains(&l.peanut, to_peanut(q)) && !geom::contains(&l.slot, q) {
                // On the fin side of the slot's straight part, or under the hook.
                let hc = l.dims.hook_center;
                let under_hook = q[1] > l.dims.slot_v.1
                    && norm([q[0] - hc[0], q[1] - hc[1]]) < l.dims.hook_radius
                    && q[0] > l.dims.slot_center;
                let beside = q[1] <= l.dims.slot_v.1 && q[0] > l.dims.slot_center;
                if under_hook || beside {
                    pts.push(q);
                }
            }
        }
    }
    pts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lobe() -> Lobe {
        Lobe::new(&StudProfile::slant_v2_experimental()).unwrap()
    }

    /// Area of a loop by sampling a fine grid (independent of Green's theorem).
    fn sampled(inside: impl Fn(P2) -> bool, lo: P2, hi: P2, step: f64) -> f64 {
        let nx = ((hi[0] - lo[0]) / step).ceil() as i32;
        let ny = ((hi[1] - lo[1]) / step).ceil() as i32;
        let mut n = 0u64;
        for i in 0..nx {
            for j in 0..ny {
                let q = [
                    lo[0] + (f64::from(i) + 0.5) * step,
                    lo[1] + (f64::from(j) + 0.5) * step,
                ];
                if inside(q) {
                    n += 1;
                }
            }
        }
        n as f64 * step * step
    }

    /// Unit tangent of a segment at its start (`end = false`) or end, in its direction of
    /// travel.
    fn tangent(s: &Seg, end: bool) -> P2 {
        match *s {
            Seg::Line { a, b } => unit(sub(b, a)),
            Seg::Arc { a, b, c, ccw } => {
                let p = if end { b } else { a };
                let n = unit(sub(p, c));
                if ccw { [-n[1], n[0]] } else { [n[1], -n[0]] }
            }
        }
    }

    #[test]
    fn the_outline_is_closed_and_tangent_at_every_junction() {
        let l = lobe();
        let n = l.peanut.len();
        assert_eq!(n, 10);
        for k in 0..n {
            let (s, t) = (&l.peanut[k], &l.peanut[(k + 1) % n]);
            let (Seg::Arc { b: end, .. }, Seg::Arc { a: start, .. }) = (*s, *t) else {
                panic!("the peanut is all arcs");
            };
            assert!(norm(sub(end, start)) < 1e-12, "gap after segment {k}");
            let (u, w) = (tangent(s, true), tangent(t, false));
            assert!(
                norm(sub(u, w)) < 1e-9,
                "corner after segment {k}: {u:?} vs {w:?}"
            );
            // Each arc's ends lie on its circle.
            if let Seg::Arc { a, b, c, .. } = *s {
                assert!(
                    (norm(sub(a, c)) - norm(sub(b, c))).abs() < 1e-12,
                    "segment {k}"
                );
            }
        }
        // Mirror symmetric about the waist line.
        for i in 0..60 {
            for j in 0..130 {
                let q = [
                    -3.0 + 0.1 * f64::from(i) + 0.013,
                    0.05 * f64::from(j) + 0.007,
                ];
                assert_eq!(
                    geom::contains(&l.peanut, q),
                    geom::contains(&l.peanut, [q[0], -q[1]]),
                    "{q:?}"
                );
            }
        }
        let d = l.dims;
        assert!((d.waist_width - 1.39).abs() < 1e-9);
        assert!((d.waist_radius - 3.305).abs() < 1e-12);
        // The fin peaks at u = 2.85; the stud circle stays Ø4.8.
        assert!((d.bump_center[0] + d.bump_radius - 2.85).abs() < 1e-12);
        assert!((d.stud_radius - 2.4).abs() < 1e-12);
        // The blend touches the stud circle at 40°, inside the slot's mouth.
        assert!((d.blend_end[1].atan2(d.blend_end[0]).to_degrees() - 40.0).abs() < 1e-9);
        assert!(d.cap_angle > 40.0);
    }

    #[test]
    fn peanut_area_matches_a_fine_grid() {
        let l = lobe();
        let step = 0.002;
        let grid = sampled(
            |q| geom::contains(&l.peanut, q),
            [-3.0, -6.5],
            [3.0, 6.5],
            step,
        );
        assert!(
            (geom::area(&l.peanut) - grid).abs() < 2e-3,
            "{} vs {grid}",
            geom::area(&l.peanut)
        );
        // Inside: the stud circles, the fin peaks; outside: the tubes' clearance at the waist.
        for q in [
            [0.0, 4.0],
            [-2.39, 4.0],
            [2.84, 3.7],
            [0.0, 0.0],
            [0.69, 0.0],
        ] {
            assert!(geom::contains(&l.peanut, q), "{q:?}");
        }
        for q in [
            [0.7, 0.0],
            [-0.7, 0.0],
            [2.86, 3.7],
            [0.0, 6.41],
            [-2.41, 4.0],
        ] {
            assert!(!geom::contains(&l.peanut, q), "{q:?}");
        }
    }

    #[test]
    fn slot_and_pocket_areas_match_a_fine_grid() {
        let l = lobe();
        let step = 0.002;
        let (lo, hi) = ([-0.5, -4.2], [3.6, 2.9]);
        let slot = sampled(|q| geom::contains(&l.slot, q), lo, hi, step);
        assert!(
            (geom::area(&l.slot) - slot).abs() < 2e-3,
            "{} vs {slot}",
            geom::area(&l.slot)
        );
        let to_peanut = |q: P2| [q[0], q[1] + l.a];
        let in_peanut = sampled(
            |q| geom::contains(&l.slot, q) && geom::contains(&l.peanut, to_peanut(q)),
            lo,
            hi,
            step,
        );
        assert!(
            (l.slot_in_peanut() - in_peanut).abs() < 2e-3,
            "{} vs {in_peanut}",
            l.slot_in_peanut()
        );
        let outside = sampled(
            |q| geom::contains(&l.slot, q) && !geom::contains(&l.pocket, q),
            lo,
            hi,
            step,
        );
        assert!(
            (l.slot_outside_pocket() - outside).abs() < 2e-3,
            "{} vs {outside}",
            l.slot_outside_pocket()
        );
        let pocket = sampled(|q| geom::contains(&l.pocket, q), lo, hi, step);
        assert!(
            (l.pocket_area() - pocket).abs() < 2e-3,
            "{} vs {pocket}",
            l.pocket_area()
        );
        // The slot runs out of the lobe (some of it is outside the peanut), and about half of
        // it is over the pocket.
        assert!(l.slot_in_peanut() < geom::area(&l.slot) - 0.05);
        let half = geom::area(&l.slot) / 2.0;
        assert!(
            (l.slot_outside_pocket() - half).abs() < 0.1,
            "{}",
            l.slot_outside_pocket()
        );
    }

    /// The slot's groove area matches sampling for any pocket band, not only the default half
    /// slot: a slot whose end line is level with the pocket's root corner once fooled the area
    /// routine's inside test (the 0.4 mm nozzle profile's groove came out 0.27 mm² too big).
    #[test]
    fn slot_outside_pocket_any_margin() {
        let set = |p: &mut StudProfile, so: f64, sw: f64, hr: f64, pm: f64| {
            if let StudStyle::SlantV2 {
                slot_offset,
                slot_width,
                hook_radius,
                pocket_margin,
                ..
            } = &mut p.stud_style
            {
                (*slot_offset, *slot_width, *hook_radius, *pocket_margin) = (so, sw, hr, pm);
            }
        };
        let mut cases = vec![StudProfile::fused(), StudProfile::fused_nozzle04()];
        for (so, sw, hr, pm) in [
            (0.25, 0.5, 1.2, 0.3),
            (0.25, 0.5, 1.2, 0.38),
            (0.15, 0.6, 1.3, 0.3),
        ] {
            let mut p = StudProfile::fused();
            set(&mut p, so, sw, hr, pm);
            cases.push(p);
        }
        for p in cases {
            let l = Lobe::new(&p).unwrap();
            let (lo, hi) = geom::bbox(&l.slot);
            let outside = sampled(
                |q| geom::contains(&l.slot, q) && !geom::contains(&l.pocket, q),
                lo,
                hi,
                0.002,
            );
            assert!(
                (l.slot_outside_pocket() - outside).abs() < 2e-3,
                "{:?}: {} vs {outside}",
                p.stud_style,
                l.slot_outside_pocket()
            );
        }
    }

    #[test]
    fn the_fin_hangs_over_its_pocket_and_the_rest_does_not() {
        let l = lobe();
        let fin = fin_probe_points(&l);
        assert!(fin.len() > 1000);
        for q in &fin {
            assert!(
                geom::contains(&l.pocket, *q),
                "fin point {q:?} is not over the pocket"
            );
        }
        // The stud's inner half and its cap beyond the hook stand on the top.
        for q in [[-1.0, 0.0], [0.0, 2.3], [0.1, -2.0], [-2.0, 0.5]] {
            assert!(!geom::contains(&l.pocket, q), "{q:?}");
        }
        // Dimensions as designed.
        let d = l.dims;
        assert!((d.slot_center - 0.5).abs() < 1e-12);
        assert!((d.fin_width - 2.1).abs() < 1e-9);
        assert!((d.root_width - 0.8).abs() < 1e-12);
        assert!(d.min_section >= 0.6, "{}", d.min_section);
    }

    #[test]
    fn the_pocket_is_the_fin_grown_by_an_even_margin() {
        let l = lobe();
        let d = l.dims;
        let m = d.pocket_margin;
        assert!((m - 0.25).abs() < 1e-12);
        // The fin's free outline: everything but its root line.
        let free = &l.fin[1..];
        let gap = |q: P2| {
            free.iter()
                .map(|s| geom::distance(s, q))
                .fold(f64::INFINITY, f64::min)
        };
        let (rw, a) = (d.waist_radius, l.a);
        let tf = [a, -a];
        let (root, tip, hc, ce) = (d.root_corner, d.tip_corner, d.hook_center, d.blend_center);
        let ri = d.hook_radius - d.slot_width / 2.0;
        let ro = d.hook_radius + d.slot_width / 2.0;
        let n_tip = unit(sub(tip, ce));
        let tip_n = add(tip, n_tip, m);
        // Root line, root fade, offset waist, bump, blend, tip round, hook, slot side.
        let n = l.pocket.len();
        assert_eq!(n, 8);
        assert_eq!(l.fin.len(), 6);
        let ends = |s: &Seg| match *s {
            Seg::Line { a, b } | Seg::Arc { a, b, .. } => (a, b),
        };
        let (t1, t2) = ends(&l.pocket[1]);
        let (z, k) = ends(&l.pocket[5]);
        // The offset waist, bump, blend (up to the tip's normal), hook (from where the round
        // meets it) and slot side are exactly `m` from the fin.
        let blend_at = |q: P2| (q[1] - ce[1]).atan2(q[0] - ce[0]);
        for (k, s) in l.pocket.iter().enumerate() {
            if ![2, 3, 4, 6, 7].contains(&k) {
                continue;
            }
            for i in 0..=200 {
                let q = s.at(f64::from(i) / 200.0);
                if k == 4 && blend_at(q) > blend_at(tip) {
                    continue;
                }
                assert!(
                    (gap(q) - m).abs() < 1e-9,
                    "pocket edge {k} at {q:?} is {} from the fin",
                    gap(q)
                );
            }
        }
        // The root fade: along the root line from the slot to `ROOT_FADE_START` past the
        // root's corner, then on radius `ρ_r` (centre square above `t1`) into the offset waist
        // arc, which it touches from outside. It stays within `m` of the fin, beside the stud.
        let (cr, rr) = (d.root_cap_center, d.root_cap_radius);
        assert!(
            (t1[1] - root[1]).abs() < 1e-15 && (t1[0] - root[0] - ROOT_FADE_START).abs() < 1e-12
        );
        assert!((cr[0] - t1[0]).abs() < 1e-15 && (cr[1] - t1[1] - rr).abs() < 1e-12);
        assert!((norm(sub(cr, tf)) - (rw - m + rr)).abs() < 1e-12);
        assert!((norm(sub(t2, tf)) - (rw - m)).abs() < 1e-12);
        for i in 0..=200 {
            let q = l.pocket[1].at(f64::from(i) / 200.0);
            assert!(gap(q) <= m + 1e-9, "{q:?}");
            assert!(
                norm(sub(q, tf)) < rw - 0.005,
                "the fade runs along the stud: {q:?}"
            );
        }
        // The tip round: inside the offset blend circle and the circle `TIP_GAP` within the
        // slot's far edge, touching both; on into the slot to the hook's offset at `k`.
        let (ct, rt) = (d.tip_round_center, d.tip_round_radius);
        assert!((rt - TIP_ROUND).abs() < 1e-15 && (d.tip_gap - TIP_GAP).abs() < 1e-15);
        assert!((norm(sub(ct, ce)) - (d.blend_radius + m - rt)).abs() < 1e-12);
        assert!((norm(sub(ct, hc)) - (ro - d.tip_gap - rt)).abs() < 1e-12);
        assert!((norm(sub(z, ce)) - (d.blend_radius + m)).abs() < 1e-12);
        assert!((norm(sub(k, hc)) - (ri + m)).abs() < 1e-12);
        assert!(blend_at(z) > blend_at(tip));

        // Closed, and tangent everywhere but at the two ends of the slot side's line, where it
        // meets the root line (under the stud, at the slot's end), and where the round meets
        // the hook's offset inside the slot. Both lie in the slot, out of sight.
        let mut corners = Vec::new();
        for k in 0..n {
            let (s, t) = (&l.pocket[k], &l.pocket[(k + 1) % n]);
            assert!(
                norm(sub(ends(s).1, ends(t).0)) < 1e-12,
                "gap after pocket edge {k}"
            );
            if norm(sub(tangent(s, true), tangent(t, false))) > 1e-9 {
                corners.push(k);
            }
        }
        assert_eq!(corners, vec![5, n - 1]);
        assert!(geom::contains(&l.slot, k));

        // Area: the fin; a band `m` wide along each free edge offset in full (an arc of
        // radius `r` turning by `θ` gives `θ·(r·m ± m²/2)`, + convex, − concave); the fade
        // beside the root; and the region past the fin's tip, up to the hook's offset.
        let turn = |c: P2, p: P2, q: P2| {
            let (a0, a1) = (
                (p[1] - c[1]).atan2(p[0] - c[0]),
                (q[1] - c[1]).atan2(q[0] - c[0]),
            );
            (a1 - a0)
                .rem_euclid(2.0 * PI)
                .min((a0 - a1).rem_euclid(2.0 * PI))
        };
        let cross =
            |o: P2, p: P2, q: P2| (p[0] - o[0]) * (q[1] - o[1]) - (p[1] - o[1]) * (q[0] - o[0]);
        let tri = |o: P2, p: P2, q: P2| cross(o, p, q).abs() / 2.0;
        // A circular segment: radius `r`, turning by `phi`.
        let seg = |r: f64, phi: f64| r * r * (phi - phi.sin()) / 2.0;
        let waist = turn(tf, t2, d.bump_start) * (rw * m - m * m / 2.0);
        let bump =
            turn(d.bump_center, d.bump_start, d.blend_start) * (d.bump_radius * m + m * m / 2.0);
        let blend = turn(ce, d.blend_start, tip) * (d.blend_radius * m + m * m / 2.0);
        let hook_at = |q: P2| (q[1] - hc[1]).atan2(q[0] - hc[0]);
        let hook = (PI - hook_at(k)) * (ri * m + m * m / 2.0);
        let straight = (d.slot_v.1 - d.slot_v.0) * m;
        // The fade: between the waist arc's radii through the root's corner and `t2`, the
        // stud's sector less what lies on the far side of the pocket's edge (the triangles
        // tf–root–t1 and tf–t1–t2, less the fade's segment, which bulges toward tf).
        let fade = rw * rw * turn(tf, root, t2) / 2.0
            - (tri(tf, root, t1) + tri(tf, t1, t2) - seg(rr, turn(cr, t1, t2)));
        // Past the tip: fanned from the tip over the offset blend (tip's normal to `z`), the
        // round (`z` to `k`), down the hook's radius to its inner edge and back along that edge
        // (concave here) to the tip.
        let k_in = add(hc, unit(sub(k, hc)), ri);
        let past_tip = tri(tip, tip_n, z)
            + tri(tip, z, k)
            + tri(tip, k, k_in)
            + seg(d.blend_radius + m, turn(ce, tip_n, z))
            + seg(rt, turn(ct, z, k))
            - seg(ri, turn(hc, tip, k_in));
        let hand = d.fin_area + waist + bump + blend + hook + straight + fade + past_tip;
        assert!(
            (d.pocket_area - hand).abs() < 1e-12,
            "{} vs {hand}",
            d.pocket_area
        );
        println!(
            "fin {:.4} mm², pocket {:.4} mm² (margin {m}): bands waist {waist:.4}, bump {bump:.4}, \
             blend {blend:.4}, hook {hook:.4}, slot {straight:.4}; root fade r {rr:.4} \
             {fade:.5}, past the tip (round r {rt}) {past_tip:.5}; pocket {:.4} mm³ per fin; out \
             to u {:.3} (rim {:.3}); root corner {root:?}, tip {tip:?}; least section {:.3}",
            d.fin_area,
            d.pocket_area,
            d.pocket_area * d.pocket_depth,
            d.pocket_out,
            d.pocket_rim,
            d.min_section
        );
    }

    #[test]
    fn the_relief_reads_as_one_smooth_line_from_above() {
        // From above the relief is the pocket and the slot's groove together (one depth).
        // Its outer edge is the pocket's from the root's corner round the tip to `y`, then the
        // slot's far edge: the slot's end lies inside the pocket, and the pocket's round past
        // `y` inside the slot, so neither shows, and the two meet at `y` with a `TIP_GAP` step.
        let l = lobe();
        let d = l.dims;
        let ends = |s: &Seg| match *s {
            Seg::Line { a, b } | Seg::Arc { a, b, .. } => (a, b),
        };
        let inside = |lp: &Loop, s: &Seg, from: f64, to: f64| {
            (0..=100).all(|i| {
                let q = s.at(from + (to - from) * f64::from(i) / 100.0);
                geom::contains(lp, q)
            })
        };
        // The slot: root line, fin-side edge, hook inner edge, end, radial step, far edge.
        assert_eq!(l.slot.len(), 7);
        let (p4, y_in) = ends(&l.slot[3]);
        let (_, y_far) = ends(&l.slot[4]);
        // The slot's end, and its inner edge from the fin's tip on, lie in the pocket.
        assert!(inside(&l.pocket, &l.slot[3], 0.0, 1.0));
        let Seg::Arc { a: p3, c: hc, .. } = l.slot[2] else {
            panic!("the hook");
        };
        let tip_from = Seg::Arc {
            a: d.tip_corner,
            b: p4,
            c: hc,
            ccw: false,
        };
        assert!(inside(&l.pocket, &tip_from, 0.001, 1.0));
        assert!(norm(sub(p3, [d.slot_center + d.slot_width / 2.0, d.slot_v.1])) < 1e-12);
        // The round past `y`, and the hook's offset, lie in the slot.
        let (ct, rt) = (d.tip_round_center, d.tip_round_radius);
        let y = add(ct, unit(sub(y_far, ct)), rt);
        let (z, k) = ends(&l.pocket[5]);
        let past_y = Seg::Arc {
            a: y,
            b: k,
            c: ct,
            ccw: true,
        };
        assert!(inside(&l.slot, &past_y, 0.001, 1.0));
        assert!(inside(&l.slot, &l.pocket[6], 0.0, 0.99));
        // The step at `y`: the slot's radial end crosses the round square there, `TIP_GAP`
        // inside the far edge, and the round up to `y` stays off the slot.
        let step = unit(sub(y_far, y_in));
        assert!(norm(sub(y, add(y_far, step, -d.tip_gap))) < 1e-12);
        assert!(dot(step, unit(sub(y, ct))).abs() > 1.0 - 1e-12);
        let to_y = Seg::Arc {
            a: z,
            b: y,
            c: ct,
            ccw: true,
        };
        assert!((0..100).all(|i| !geom::contains(&l.slot, to_y.at(f64::from(i) / 100.0))));
        // So the visible edge, the pocket's from the root line to `y`, is tangent throughout.
        let mut edge: Vec<Seg> = l.pocket[..5].to_vec();
        edge.push(to_y);
        for w in edge.windows(2) {
            assert!(norm(sub(ends(&w[0]).1, ends(&w[1]).0)) < 1e-12);
            assert!(norm(sub(tangent(&w[0], true), tangent(&w[1], false))) < 1e-9);
        }
        // And the round's far side meets the slot's far edge heading the same way.
        assert!(norm(sub(tangent(&to_y, true), tangent(&l.slot[5], false))) < 1e-9);
    }

    #[test]
    fn the_pocket_frees_the_whole_fin_and_nothing_else() {
        let l = lobe();
        let d = l.dims;
        let p = StudProfile::fused();
        let to_peanut = |q: P2| [q[0], q[1] + l.a];
        let (lo, hi) = geom::bbox(&l.pocket);
        // An odd step and offset, so no sample lands on the root line or the slot's edges.
        let step = 0.003_917;
        let (mut in_fin, mut out) = (0, 0);
        let nx = ((hi[0] - lo[0] + 0.4) / step) as i32;
        let ny = ((hi[1] - lo[1] + 0.4) / step) as i32;
        for i in 0..nx {
            for j in 0..ny {
                let q = [
                    lo[0] - 0.2 + (f64::from(i) + 0.3141) * step,
                    lo[1] - 0.2 + (f64::from(j) + 0.2718) * step,
                ];
                let pocket = geom::contains(&l.pocket, q);
                if geom::contains(&l.fin, q) {
                    // No ledge: every point of the fin, root to tip, is over the pocket.
                    assert!(pocket, "fin point {q:?} is not over the pocket");
                    in_fin += 1;
                } else if pocket {
                    // Nothing else of the stud stands over it: past the fin it is the slot
                    // or the top beside the stud, never the root or the stud across the slot.
                    assert!(
                        geom::contains(&l.slot, q) || !geom::contains(&l.peanut, to_peanut(q)),
                        "pocket point {q:?} undercuts the stud"
                    );
                    out += 1;
                }
            }
        }
        assert!(in_fin > 100_000 && out > 50_000, "{in_fin} {out}");
        // Nothing of the pocket lies below the slot's end: the root stands on the bare top.
        for s in &l.pocket {
            for i in 0..=100 {
                let q = s.at(f64::from(i) / 100.0);
                assert!(q[1] >= d.slot_v.0 - 1e-12, "{q:?}");
            }
        }
        // The root keeps its section, and the top keeps a rim to the brick's side, clear of
        // the rounded corners; the pocket reaches no nearer the end wall than the slot's
        // groove (the slot's far edge), which stays short of an end wall's ribs.
        assert!(d.min_section >= 0.6, "{}", d.min_section);
        assert!(d.pocket_rim >= 0.6, "{}", d.pocket_rim);
        assert!((d.pocket_out - (d.fin_reach + d.pocket_margin)).abs() < 1e-12);
        let end_wall = l.a - p.clearance - p.wall;
        let (_, slot_hi) = geom::bbox(&l.slot);
        assert!(hi[1] < slot_hi[1], "{hi:?} {slot_hi:?}");
        assert!(
            slot_hi[1] < end_wall - p.wall_rib_protrusion() - 0.15,
            "{slot_hi:?}"
        );
        assert!(hi[0] < l.a - p.clearance - p.edge_radius - 0.3, "{hi:?}");
    }

    #[test]
    fn profiles_that_would_not_print_or_fit_are_refused() {
        let with = |f: &dyn Fn(&mut StudStyle)| {
            let mut p = StudProfile::slant_v2_experimental();
            f(&mut p.stud_style);
            Lobe::new(&p)
        };
        // A thin root leaves too little between the slot and the waist.
        let e = with(&|s| {
            if let StudStyle::SlantV2 { root_width, .. } = s {
                *root_width = 0.3;
            }
        });
        assert!(e.is_err_and(|e| e.to_string().contains("to the waist")));
        // A fin that would press further than its slot lets it.
        let e = with(&|s| {
            if let StudStyle::SlantV2 { fin_reach, .. } = s {
                *fin_reach = 3.2;
            }
        });
        assert!(e.is_err());
        // A blend that ends past the slot's mouth would put the cap off the stud circle.
        let e = with(&|s| {
            if let StudStyle::SlantV2 { blend_angle, .. } = s {
                *blend_angle = 60.0;
            }
        });
        assert!(e.is_err());
        // A pocket wider than the slot would reach under the stud across it.
        let e = with(&|s| {
            if let StudStyle::SlantV2 { pocket_margin, .. } = s {
                *pocket_margin = 0.45;
            }
        });
        assert!(e.is_err_and(|e| e.to_string().contains("across")));
    }

    /// Closed, all arcs, tangent at every junction, each arc's ends on its circle.
    fn assert_smooth(l: &Loop, what: &str) {
        let n = l.len();
        for k in 0..n {
            let (s, t) = (&l[k], &l[(k + 1) % n]);
            let (Seg::Arc { b: end, .. }, Seg::Arc { a: start, .. }) = (*s, *t) else {
                panic!("{what}: all arcs");
            };
            assert!(
                norm(sub(end, start)) < 1e-12,
                "{what}: gap after segment {k}"
            );
            let (u, w) = (tangent(s, true), tangent(t, false));
            assert!(
                norm(sub(u, w)) < 1e-9,
                "{what}: corner after segment {k}: {u:?} vs {w:?}"
            );
            if let Seg::Arc { a, b, c, .. } = *s {
                assert!(
                    (norm(sub(a, c)) - norm(sub(b, c))).abs() < 1e-12,
                    "{what}: segment {k}"
                );
            }
        }
    }

    #[test]
    fn triples_and_finless_units_are_smooth_and_symmetric() {
        let l = lobe();
        for (what, outline, n, segs) in [
            ("finned triple", &l.triple, 3, 18),
            ("finless pair", &l.pair_plain, 2, 8),
            ("finless triple", &l.triple_plain, 3, 16),
        ] {
            assert_eq!(outline.len(), segs, "{what}");
            assert_smooth(outline, what);
            // Mirror symmetric about the unit's centre line (and finless ones across it too).
            let half = if n == 2 { 6.5 } else { 10.5 };
            for i in 0..60 {
                for j in 0..(half * 20.0) as i32 {
                    let q = [
                        -3.0 + 0.1 * f64::from(i) + 0.013,
                        0.05 * f64::from(j) + 0.007,
                    ];
                    let inside = geom::contains(outline, q);
                    assert_eq!(
                        inside,
                        geom::contains(outline, [q[0], -q[1]]),
                        "{what} {q:?}"
                    );
                    if !std::ptr::eq(outline, &l.triple) {
                        assert_eq!(
                            inside,
                            geom::contains(outline, [-q[0], q[1]]),
                            "{what} {q:?}"
                        );
                    }
                }
            }
        }
        let a = l.a;
        // The triple's middle lobe: the plain stud circle on both sides, the waists cut by the
        // tube-clearance arcs (0.695 mm half-width on the waist lines).
        for q in [[2.39, 0.0], [-2.39, 0.0], [0.69, a], [0.69, -a], [-0.69, a]] {
            assert!(geom::contains(&l.triple, q), "{q:?}");
        }
        for q in [
            [2.41, 0.0],
            [-2.41, 0.0],
            [0.70, a],
            [0.70, -a],
            [-0.70, -a],
        ] {
            assert!(!geom::contains(&l.triple, q), "{q:?}");
        }
        // Its end lobes keep the pair's fins: 2.85 mm out at 0.3 mm toward the waist.
        for v in [2.0 * a - 0.3, -2.0 * a + 0.3] {
            assert!(geom::contains(&l.triple, [2.84, v]) && !geom::contains(&l.triple, [2.86, v]));
        }
        // Areas: each end of a triple is half a pair; a finless lobe end differs from a
        // finned one only on the fin side. Checked against a fine grid.
        let step = 0.002;
        for (outline, half) in [
            (&l.triple, 10.5),
            (&l.pair_plain, 6.5),
            (&l.triple_plain, 10.5),
        ] {
            let grid = sampled(
                |q| geom::contains(outline, q),
                [-3.0, -half],
                [3.0, half],
                step,
            );
            assert!(
                (geom::area(outline) - grid).abs() < 3e-3,
                "{} vs {grid}",
                geom::area(outline)
            );
        }
        let d = l.dims;
        assert!(
            (d.triple_plain_area - d.pair_plain_area - (d.triple_area - d.pair_area)).abs() < 1e-9
        );
        println!(
            "areas: pair {:.4}, triple {:.4}, finless pair {:.4}, finless triple {:.4} mm²",
            d.pair_area, d.triple_area, d.pair_plain_area, d.triple_plain_area
        );
    }

    #[test]
    fn a_triples_end_lobes_are_cut_as_a_pairs() {
        // Each end lobe of a triple meets its slot exactly as a pair's lobe does, so the
        // slot volume per lobe is the same.
        let l = lobe();
        let b = Frame {
            o: [0.0, 2.0 * l.a],
            x: [1.0, 0.0],
            y: [0.0, 1.0],
        };
        let in_triple = geom::intersection_area(&b.map_loop(&l.slot), &l.triple);
        assert!(
            (in_triple - l.slot_in_peanut()).abs() < 1e-12,
            "{in_triple} vs {}",
            l.slot_in_peanut()
        );
        // And the pocket stays on the end lobe: clear of the middle lobe's waist line.
        let pocket = b.map_loop(&l.pocket);
        for s in &pocket {
            for k in 0..=20 {
                assert!(s.at(f64::from(k) / 20.0)[1] > l.a + 1.5);
            }
        }
    }

    #[test]
    fn runs_split_into_pairs_and_one_middle_triple() {
        assert_eq!(split_run(2), vec![2]);
        assert_eq!(split_run(3), vec![3]);
        assert_eq!(split_run(4), vec![2, 2]);
        assert_eq!(split_run(5), vec![2, 3]);
        assert_eq!(split_run(6), vec![2, 2, 2]);
        assert_eq!(split_run(7), vec![2, 3, 2]);
        assert_eq!(split_run(9), vec![2, 2, 3, 2]);
        assert_eq!(split_run(11), vec![2, 2, 3, 2, 2]);
        for n in 2..40 {
            assert_eq!(split_run(n).iter().sum::<usize>(), n);
        }
    }

    #[test]
    fn studs_fuse_along_rows_with_fins_outward() {
        let rect = |w: i32, d: i32| -> BTreeSet<(i32, i32)> {
            (0..w).flat_map(|i| (0..d).map(move |j| (i, j))).collect()
        };
        let all = |s: &BTreeSet<(i32, i32)>| s.iter().copied().collect::<Vec<_>>();
        let unit = |first, step, n, fin| Unit {
            first,
            step,
            n,
            fin,
        };
        let c = rect(2, 4);
        let u = layout(&c, &all(&c)).unwrap();
        assert_eq!(u.len(), 4);
        assert!(u.contains(&unit((0, 0), (0, 1), 2, Some((-1, 0)))));
        assert!(u.contains(&unit((1, 2), (0, 1), 2, Some((1, 0)))));
        // 2×3: one triple per row.
        let c = rect(2, 3);
        let u = layout(&c, &all(&c)).unwrap();
        assert_eq!(
            u,
            vec![
                unit((0, 0), (0, 1), 3, Some((-1, 0))),
                unit((1, 0), (0, 1), 3, Some((1, 0)))
            ]
        );
        // 1×5: a pair, then a triple.
        let c = rect(1, 5);
        let u = layout(&c, &all(&c)).unwrap();
        assert_eq!(
            u,
            vec![
                unit((0, 0), (0, 1), 2, Some((1, 0))),
                unit((0, 2), (0, 1), 3, Some((1, 0)))
            ]
        );
        let c = rect(4, 1);
        let u = layout(&c, &all(&c)).unwrap();
        assert!(
            u.iter()
                .all(|q| q.fin == Some((0, 1)) && q.step == (1, 0) && q.n == 2)
        );
        // A 3-wide brick: the middle row has neighbours on both sides: fused, finless.
        let c = rect(3, 4);
        let u = layout(&c, &all(&c)).unwrap();
        assert_eq!(u.len(), 6);
        let mid: Vec<_> = u.iter().filter(|q| q.first.0 == 1).collect();
        assert_eq!(mid.len(), 2);
        assert!(mid.iter().all(|q| q.fin.is_none()));
        // Every stud is in exactly one unit.
        for (w, d) in [(2, 7), (3, 5), (1, 9), (6, 2)] {
            let c = rect(w, d);
            let u = layout(&c, &all(&c)).unwrap();
            let mut seen: Vec<_> = u.iter().flat_map(Unit::cells).collect();
            seen.sort_unstable();
            assert_eq!(seen, all(&c));
        }
        // A lone stud is refused.
        let c = rect(1, 1);
        let e = layout(&c, &all(&c)).unwrap_err().to_string();
        assert!(e.contains("(0, 0)") && e.contains("alone"), "{e}");
        // An L that leaves a lone stud along Y fuses along X instead.
        let c: BTreeSet<_> = [(0, 0), (1, 0), (2, 0), (0, 1), (1, 1), (0, 2), (1, 2)]
            .into_iter()
            .collect();
        let u = layout(&c, &all(&c)).unwrap();
        assert!(u.iter().all(|q| q.step == (1, 0)));
        assert!(u.contains(&unit((0, 0), (1, 0), 3, Some((0, -1)))));
        assert!(u.contains(&unit((0, 1), (1, 0), 2, None)));
        assert!(u.contains(&unit((0, 2), (1, 0), 2, Some((0, 1)))));
        // A T of four can't be split either way.
        let c: BTreeSet<_> = [(0, 0), (1, 0), (2, 0), (1, 1)].into_iter().collect();
        assert!(layout(&c, &all(&c)).is_err());
    }
}
