//! Slant 3D's v2 stud: peanuts of two fused studs, each lobe with a leaf-spring fin.
//!
//! Frames used here:
//! * **peanut frame**: origin at the waist centre (midway between the two stud centres), `v`
//!   along the row toward lobe B, `u` toward the fin side;
//! * **lobe frame**: origin at a lobe's centre, `u` toward the fin side, `v` away from the
//!   waist. Lobe B's frame is the peanut frame moved by `+pitch/2` along `v`; lobe A's is its
//!   mirror image about the waist line.

use crate::geom::{self, Frame, Loop, Seg};
use crate::profile::{MIN_POCKET_FLOOR, SLANT_CAP, StudProfile, StudStyle};
use crate::shape::P2;
use crate::{CustomError, Result};
use std::collections::BTreeSet;
use std::f64::consts::PI;

/// How far a pocket reaches past its slot's end, toward the waist, in mm (so the pocket's
/// end wall and the slot's end wall are never the same face).
const POCKET_LEAD: f64 = 0.2;
/// How far a slot's tail runs past the point where the fin's tip leaves the lobe, in mm.
const TAIL_PAST: f64 = 0.3;
/// How far the pocket's outline runs on past the slot's tail, in mm.
const POCKET_TAIL: f64 = 0.2;
/// How far the pocket reaches outboard of the slot tail and the lobe, in mm.
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

/// The derived dimensions of a v2 stud (mm), for reports and tests.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SlantDims {
    /// Lobe diameter.
    pub lobe_diameter: f64,
    /// Waist width.
    pub waist_width: f64,
    /// Radius of the waist's concave sides.
    pub waist_radius: f64,
    /// Slot width.
    pub slot_width: f64,
    /// Slot centre line, outboard of the lobe centre line.
    pub slot_center: f64,
    /// Where the slot's straight part starts and ends (`v` in the lobe frame).
    pub slot_v: (f64, f64),
    /// Centre of the hook (lobe frame) and its centre-line radius.
    pub hook_center: P2,
    /// Hook centre-line radius.
    pub hook_radius: f64,
    /// Length of the straight tail after the hook, on the centre line.
    pub tail: f64,
    /// Widest fin: from the slot's outer edge to the lobe's edge.
    pub fin_width: f64,
    /// Root length (waist line to slot end).
    pub root_length: f64,
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

fn norm(p: P2) -> f64 {
    p[0].hypot(p[1])
}

impl Lobe {
    /// The v2 geometry of `p`, or why its numbers don't make one.
    pub fn new(p: &StudProfile) -> Result<Self> {
        let StudStyle::SlantV2 {
            lobe_scale,
            waist_width,
            slot_width,
            slot_offset,
            root_length,
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
            ("lobe_scale", lobe_scale),
            ("waist_width", waist_width),
            ("slot_width", slot_width),
            ("root_length", root_length),
            ("hook_radius", hook_radius),
            ("pocket_depth", pocket_depth),
        ] {
            if !(v.is_finite() && v > 0.0) {
                return bad(format!("{name} must be a positive number, got {v}"));
            }
        }
        if !(slot_offset.is_finite() && slot_offset >= 0.0) {
            return bad(format!(
                "slot_offset must be 0 or more mm, got {slot_offset}"
            ));
        }
        let r = lobe_scale * p.stud_diameter / 2.0;
        let a = p.pitch / 2.0;
        let w = waist_width / 2.0;
        let t = slot_width;
        let hw = t / 2.0;
        if 2.0 * r >= p.pitch - 2.0 * p.clearance {
            return bad(format!("lobes Ø{:.2} mm are wider than a cell", 2.0 * r));
        }
        if w >= r - 0.1 || w <= 0.0 {
            return bad("the waist must be narrower than a lobe".into());
        }
        if p.top_thickness < pocket_depth + MIN_POCKET_FLOOR - 1e-9 {
            return bad(format!(
                "a {pocket_depth} mm pocket needs a top at least {:.2} mm thick (it is {})",
                pocket_depth + MIN_POCKET_FLOOR,
                p.top_thickness
            ));
        }
        if root_length <= POCKET_LEAD + 0.2 || root_length >= a - 1.0 {
            return bad(format!(
                "root_length {root_length} mm must be between {:.1} and {:.1}",
                POCKET_LEAD + 0.2,
                a - 1.0
            ));
        }

        // Peanut: the waist's concave sides are arcs of radius R about (±c, 0), tangent to
        // both lobes: (R + w)² + a² = (R + r)².
        let big_r = (w * w + a * a - r * r) / (2.0 * (r - w));
        let c = big_r + w;
        let k = 1.0 / (big_r + r);
        let (tu, tv) = (c * r * k, a * big_r * k);
        let trb = [tu, -tv];
        let trt = [tu, tv];
        let tlt = [-tu, tv];
        let tlb = [-tu, -tv];
        let peanut = vec![
            Seg::Arc {
                a: trb,
                b: trt,
                c: [c, 0.0],
                ccw: false,
            },
            Seg::Arc {
                a: trt,
                b: tlt,
                c: [0.0, a],
                ccw: true,
            },
            Seg::Arc {
                a: tlt,
                b: tlb,
                c: [-c, 0.0],
                ccw: false,
            },
            Seg::Arc {
                a: tlb,
                b: trb,
                c: [0.0, -a],
                ccw: true,
            },
        ];
        // Half-width of the peanut at `v` (peanut frame, |v| ≤ a + r).
        let half_width = |v: f64| {
            let v = v.abs();
            if v >= tv {
                (r * r - (v - a) * (v - a)).max(0.0).sqrt()
            } else {
                c - (big_r * big_r - v * v).sqrt()
            }
        };

        // Slot, in lobe B's frame.
        let uc = slot_offset + hw;
        let v0 = root_length - a;
        let v_s = r - SLANT_CAP - hw - hook_radius;
        if v_s <= v0 + 0.2 {
            return bad(
                "the hook leaves no straight slot (smaller hook_radius or root_length)".into(),
            );
        }
        if hook_radius <= hw + 0.2 {
            return bad("the hook is tighter than the slot is wide".into());
        }
        if uc + hw + 0.2 > half_width(root_length) {
            return bad("the slot's end lies outside the waist".into());
        }
        let hc = [uc + hook_radius, v_s];
        let th = PI / 4.0; // the hook ends heading 45° back toward the waist
        let (ct, st) = (th.cos(), th.sin());
        let dir = [st, -ct];
        let out_n = [ct, st]; // from the fin side of the tail to its far side
        let p4 = [
            hc[0] + (hook_radius - hw) * ct,
            hc[1] + (hook_radius - hw) * st,
        ];
        // The tail runs on until the fin side's edge is TAIL_PAST outside the lobe.
        let exit = {
            let (b, cc) = (p4[0] * dir[0] + p4[1] * dir[1], norm(p4).powi(2) - r * r);
            if cc >= 0.0 {
                0.0
            } else {
                -b + (b * b - cc).sqrt()
            }
        };
        let tail = exit + TAIL_PAST;
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

        // Pocket: everything on the fin side of the slot's centre line, out past the lobe.
        let m_start = [uc, v_s];
        let m_end = [hc[0] + hook_radius * ct, hc[1] + hook_radius * st];
        let e2 = add(m_end, dir, tail + POCKET_TAIL);
        let u_out = e2[0].max(r) + POCKET_OUT;
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
            lobe_diameter: 2.0 * r,
            waist_width,
            waist_radius: big_r,
            slot_width: t,
            slot_center: uc,
            slot_v: (v0, v_s),
            hook_center: hc,
            hook_radius,
            tail,
            fin_width: r - (uc + hw),
            root_length,
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
                    + (l.dims.lobe_diameter / 2.0 + 0.1 - l.dims.slot_v.0) * f64::from(j)
                        / f64::from(n),
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

    #[test]
    fn peanut_area_matches_its_closed_form() {
        let l = lobe();
        let (r, a) = (l.dims.lobe_diameter / 2.0, l.a);
        let w = 1.6;
        let big_r = l.dims.waist_radius;
        let c = big_r + w;
        assert!(
            ((c * c + a * a).sqrt() - (big_r + r)).abs() < 1e-12,
            "arcs tangent"
        );
        let tv = a * big_r / (big_r + r);
        // Two disks, plus the waist band |v| ≤ tv between the concave arcs, less the two caps
        // of the disks that lie in that band.
        let band = 4.0 * c * tv
            - 2.0 * (tv * (big_r * big_r - tv * tv).sqrt() + big_r * big_r * (tv / big_r).asin());
        let d = a - tv;
        let cap = r * r * (d / r).acos() - d * (r * r - d * d).sqrt();
        let hand = 2.0 * PI * r * r + band - 2.0 * cap;
        assert!(
            (l.peanut_area() - hand).abs() < 1e-9,
            "{} vs {hand}",
            l.peanut_area()
        );
        // Waist width as asked.
        assert!((c - big_r - w).abs() < 1e-12);
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
        assert!((d.lobe_diameter - 5.28).abs() < 1e-12);
        assert!((d.slot_center - 0.5).abs() < 1e-12);
        assert!((d.fin_width - 1.89).abs() < 1e-9);
        assert!((d.slot_v.0 + 3.2).abs() < 1e-12);
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
