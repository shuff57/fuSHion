//! Slant 3D's v2 stud: peanuts of two fused studs, each lobe with a leaf-spring fin.
//!
//! Frames used here:
//! * **peanut frame**: origin at the waist centre (midway between the two stud centres), `v`
//!   along the row toward lobe B, `u` toward the fin side;
//! * **lobe frame**: origin at a lobe's centre, `u` toward the fin side, `v` away from the
//!   waist. Lobe B's frame is the peanut frame moved by `+pitch/2` along `v`; lobe A's is its
//!   mirror image about the waist line.
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
//! Lobe A is lobe B mirrored about the waist line.

use crate::geom::{self, Frame, Loop, Seg};
use crate::profile::{MIN_FIN_SECTION, MIN_POCKET_FLOOR, SLANT_CAP, StudProfile, StudStyle};
use crate::shape::P2;
use crate::{CustomError, Result};
use std::collections::BTreeSet;
use std::f64::consts::{FRAC_PI_4, PI};

/// How far a pocket reaches past its slot's end, toward the waist, in mm (so the pocket's
/// end wall and the slot's end wall are never the same face).
const POCKET_LEAD: f64 = 0.2;
/// How far a slot's tail runs past the point where the fin's tip leaves the lobe, in mm.
const TAIL_PAST: f64 = 0.3;
/// How far the pocket's outline runs on past the slot's tail, in mm.
const POCKET_TAIL: f64 = 0.2;
/// How far the pocket reaches outboard of the slot tail and the fin, in mm.
const POCKET_OUT: f64 = 0.15;
/// The thinnest rim a pocket may leave to the brick's outside face, in mm.
const MIN_RIM: f64 = 0.5;

/// One lobe's slot and pocket, and the peanut it belongs to, worked out from a profile.
#[derive(Clone, Debug)]
pub(crate) struct Lobe {
    /// Distance from a lobe centre to the waist line (half the pitch).
    pub a: f64,
    /// The peanut outline, in the peanut frame.
    pub peanut: Loop,
    /// The slot, in lobe B's frame.
    pub slot: Loop,
    /// The pocket under the fin, in lobe B's frame.
    pub pocket: Loop,
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
    /// Length of the straight tail after the hook, on the centre line.
    pub tail: f64,
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
    /// Pocket reach outboard of the lobe centre.
    pub pocket_out: f64,
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

/// How far from `p` along unit `dir` a ray leaves the circle about `c` of radius `r` (0 when
/// `p` is already outside).
fn leave(p: P2, dir: P2, c: P2, r: f64) -> f64 {
    let q = sub(p, c);
    let (b, cc) = (dot(q, dir), dot(q, q) - r * r);
    if cc >= 0.0 {
        0.0
    } else {
        -b + (b * b - cc).sqrt()
    }
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
        let shift = |s: &Seg| match *s {
            Seg::Line { a: p0, b: p1 } => line(up(p0, a), up(p1, a)),
            Seg::Arc {
                a: p0,
                b: p1,
                c,
                ccw,
            } => Seg::Arc {
                a: up(p0, a),
                b: up(p1, a),
                c: up(c, a),
                ccw,
            },
        };
        let b_chain: Vec<Seg> = chain.iter().map(shift).collect();
        let jb = up(j, a);
        let g2b = up(g2, a);
        let m = |q: P2| [q[0], -q[1]];
        let mut peanut = vec![Seg::Arc {
            a: m(jb),
            b: jb,
            c: [a, 0.0],
            ccw: false,
        }];
        peanut.extend(b_chain.iter().copied());
        peanut.push(Seg::Arc {
            a: g2b,
            b: m(g2b),
            c: [-a, 0.0],
            ccw: false,
        });
        peanut.extend(b_chain.iter().rev().map(mirrored));

        // Slot, in lobe B's frame. It starts where the fin's root is `root_width` across.
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
        let th = FRAC_PI_4; // the hook ends heading 45° back toward the waist
        let (ct, st) = (th.cos(), th.sin());
        let dir = [st, -ct];
        let out_n = [ct, st]; // from the fin side of the tail to its far side
        let p4 = [
            hc[0] + (hook_radius - hw) * ct,
            hc[1] + (hook_radius - hw) * st,
        ];
        // The tail runs on until the fin side's edge is TAIL_PAST outside the lobe (which
        // lies inside the blend circle).
        let tail = leave(p4, dir, ce, re) + TAIL_PAST;
        let p1 = [uc - hw, v0];
        let p2 = [uc + hw, v0];
        let p3 = [uc + hw, v_s];
        let p5 = add(p4, dir, tail);
        let p6 = add(p5, out_n, t);
        let p7 = add(p4, out_n, t);
        let p8 = [uc - hw, v_s];
        let slot = vec![
            line(p1, p2),
            line(p2, p3),
            Seg::Arc {
                a: p3,
                b: p4,
                c: hc,
                ccw: false,
            },
            line(p4, p5),
            line(p5, p6),
            line(p6, p7),
            Seg::Arc {
                a: p7,
                b: p8,
                c: hc,
                ccw: true,
            },
            line(p8, p1),
        ];

        // Pocket: everything on the fin side of the slot's centre line, out past the fin.
        let m_start = [uc, v_s];
        let m_end = [hc[0] + hook_radius * ct, hc[1] + hook_radius * st];
        let e2 = add(m_end, dir, tail + POCKET_TAIL);
        let u_out = e2[0].max(fin_reach) + POCKET_OUT;
        let vp0 = v0 - POCKET_LEAD;
        if u_out > a - p.clearance - MIN_RIM {
            return bad(format!(
                "the pocket reaches {u_out:.2} mm outboard, leaving under {MIN_RIM} mm to the brick's side"
            ));
        }
        let pocket = geom::ccw(vec![
            line([uc, vp0], [u_out, vp0]),
            line([u_out, vp0], [u_out, e2[1]]),
            line([u_out, e2[1]], e2),
            line(e2, m_end),
            Seg::Arc {
                a: m_end,
                b: m_start,
                c: hc,
                ccw: true,
            },
            line(m_start, [uc, vp0]),
        ]);

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
            tail,
            cap_angle,
            fin_width: fin_reach - fin_edge,
            root_length: v0 + a,
            root_width,
            min_section,
            pocket_depth,
            pocket_out: u_out,
        };
        Ok(Self {
            a,
            peanut,
            slot,
            pocket,
            pocket_depth,
            groove: pocket_depth / 2.0,
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

    /// Area of the peanut.
    pub fn peanut_area(&self) -> f64 {
        geom::area(&self.peanut)
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
    pub fn slot_outside_pocket(&self) -> f64 {
        geom::area(&self.slot) - geom::intersection_area(&self.slot, &self.pocket)
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

/// A peanut: two neighbouring stud cells and the direction its fins face.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Peanut {
    /// Lobe A's cell.
    pub a: (i32, i32),
    /// Lobe B's cell (one step along the row from `a`).
    pub b: (i32, i32),
    /// Unit step (cells) toward the fin side.
    pub fin: (i32, i32),
}

impl Peanut {
    /// The peanut frame in the plane of the part.
    pub fn frame(&self, pitch: f64) -> Frame {
        let centre = |c: (i32, i32)| {
            [
                (f64::from(c.0) + 0.5) * pitch,
                (f64::from(c.1) + 0.5) * pitch,
            ]
        };
        let (ca, cb) = (centre(self.a), centre(self.b));
        let e = [
            f64::from(self.b.0 - self.a.0),
            f64::from(self.b.1 - self.a.1),
        ];
        Frame {
            o: [(ca[0] + cb[0]) / 2.0, (ca[1] + cb[1]) / 2.0],
            x: [f64::from(self.fin.0), f64::from(self.fin.1)],
            y: e,
        }
    }

    /// The two lobe frames (B, then A mirrored about the waist line), in the part's plane.
    pub fn lobe_frames(&self, pitch: f64) -> [Frame; 2] {
        let f = self.frame(pitch);
        let a = pitch / 2.0;
        [
            Frame {
                o: f.map([0.0, a]),
                x: f.x,
                y: f.y,
            },
            Frame {
                o: f.map([0.0, -a]),
                x: f.x,
                y: [-f.y[0], -f.y[1]],
            },
        ]
    }
}

/// Splits `studs` into peanuts and plain round studs.
///
/// Studs pair up along the longer side of the stud area (ties: along Y), greedily from the
/// low end of each row. A pair's fins face a side where neither cell of `cells` (the whole
/// part) has a neighbour, so they press on the outside wall of the brick pushed on: the
/// negative side when only that is open, else the positive side (so single rows face +X, or
/// +Y when they run along X). A pair with neighbours on both sides, and a stud left over at
/// the end of a row, stays round.
pub(crate) fn layout(
    cells: &BTreeSet<(i32, i32)>,
    studs: &[(i32, i32)],
) -> (Vec<Peanut>, Vec<(i32, i32)>) {
    let set: BTreeSet<(i32, i32)> = studs.iter().copied().collect();
    let span = |k: fn(&(i32, i32)) -> i32| {
        let lo = set.iter().map(k).min().unwrap_or(0);
        let hi = set.iter().map(k).max().unwrap_or(0);
        hi - lo
    };
    let along_y = span(|c| c.1) >= span(|c| c.0);
    let (step, cross) = if along_y {
        ((0, 1), (1, 0))
    } else {
        ((1, 0), (0, 1))
    };
    let key = |c: &(i32, i32)| if along_y { (c.0, c.1) } else { (c.1, c.0) };
    let mut order: Vec<(i32, i32)> = set.iter().copied().collect();
    order.sort_by_key(key);
    let mut used = BTreeSet::new();
    let mut peanuts = Vec::new();
    let mut round = Vec::new();
    for c in order {
        if used.contains(&c) {
            continue;
        }
        let n = (c.0 + step.0, c.1 + step.1);
        let open = |d: (i32, i32)| {
            [c, n]
                .iter()
                .all(|q| !cells.contains(&(q.0 + d.0, q.1 + d.1)))
        };
        let fin = if !set.contains(&n) || used.contains(&n) {
            None
        } else if open(cross) {
            Some(cross)
        } else if open((-cross.0, -cross.1)) {
            Some((-cross.0, -cross.1))
        } else {
            None
        };
        match fin {
            Some(fin) => {
                used.insert(c);
                used.insert(n);
                peanuts.push(Peanut { a: c, b: n, fin });
            }
            None => {
                used.insert(c);
                round.push(c);
            }
        }
    }
    (peanuts, round)
}

/// The fin region of lobe B (lobe frame): inside the peanut, on the fin side of the slot,
/// from the slot's end out. For tests: it must lie wholly over the pocket.
#[cfg(test)]
pub(crate) fn fin_probe_points(l: &Lobe) -> Vec<P2> {
    let mut pts = Vec::new();
    let to_peanut = |q: P2| [q[0], q[1] + l.a];
    let n = 400;
    for i in 0..=n {
        for j in 0..=n {
            let q = [
                -0.1 + 3.0 * f64::from(i) / f64::from(n),
                l.dims.slot_v.0
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
            (l.peanut_area() - grid).abs() < 2e-3,
            "{} vs {grid}",
            l.peanut_area()
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
    }

    #[test]
    fn studs_pair_along_rows_with_fins_outward() {
        let rect = |w: i32, d: i32| -> BTreeSet<(i32, i32)> {
            (0..w).flat_map(|i| (0..d).map(move |j| (i, j))).collect()
        };
        let all = |s: &BTreeSet<(i32, i32)>| s.iter().copied().collect::<Vec<_>>();
        let c = rect(2, 4);
        let (p, r) = layout(&c, &all(&c));
        assert!(r.is_empty());
        assert_eq!(p.len(), 4);
        assert!(p.contains(&Peanut {
            a: (0, 0),
            b: (0, 1),
            fin: (-1, 0)
        }));
        assert!(p.contains(&Peanut {
            a: (1, 2),
            b: (1, 3),
            fin: (1, 0)
        }));
        let c = rect(2, 3);
        let (p, r) = layout(&c, &all(&c));
        assert_eq!((p.len(), r.len()), (2, 2));
        assert!(r.contains(&(0, 2)) && r.contains(&(1, 2)));
        let c = rect(1, 4);
        let (p, _) = layout(&c, &all(&c));
        assert!(p.iter().all(|q| q.fin == (1, 0)));
        let c = rect(4, 1);
        let (p, _) = layout(&c, &all(&c));
        assert!(p.iter().all(|q| q.fin == (0, 1) && q.b.0 == q.a.0 + 1));
        // A 3-wide brick: the middle row has neighbours on both sides and stays round.
        let c = rect(3, 4);
        let (p, r) = layout(&c, &all(&c));
        assert_eq!(p.len(), 4);
        assert_eq!(r, vec![(1, 0), (1, 1), (1, 2), (1, 3)]);
    }
}
