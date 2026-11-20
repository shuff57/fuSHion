//! Builds Slant 3D's v2 stud (fused pairs and triples) through SolidCraft and checks it against
//! hand calculations made here, independently of the crate's own geometry: the peanut, a
//! triple's middle lobe and finless units in closed form (their width
//! integrated arc by arc), the slot and pocket areas by sampling a fine grid with inside tests
//! written from the stud's dimensions (the pocket as every point within its margin of the
//! fin's free edges that is not under the rest of the stud, with its root cap).
//!
//! STLs are written to `/tmp/fushion-custom/` for inspection.

use serde_json::json;
use sf_custom::{
    CustomPart, CustomShape, StudProfile, analyze_stl, build, build_from_commands, commands,
    slant_dims,
};
use std::f64::consts::PI;

const OUT: &str = "/tmp/fushion-custom";

/// The v2 lobe geometry, written out from the documented construction. Points are in lobe
/// B's frame (origin at its centre, `u` toward the fin side, `v` away from the waist); each
/// tangency is found by bisection rather than the crate's closed forms.
struct Hand {
    r: f64,
    a: f64,
    rw: f64,
    cf: [f64; 2],
    rho: f64,
    j: [f64; 2],
    ce: [f64; 2],
    re: f64,
    p: [f64; 2],
    t: [f64; 2],
    cg: [f64; 2],
    rg: f64,
    g1: [f64; 2],
    g2: [f64; 2],
    uc: f64,
    hw: f64,
    v0: f64,
    vs: f64,
    hc: [f64; 2],
    rh: f64,
    tail: f64,
    /// The fin's root corner (the slot's end meets the fin-side waist arc) and its tip (the
    /// hook's inner edge leaves the blend arc).
    root: [f64; 2],
    tip: [f64; 2],
    /// Pocket margin.
    m: f64,
    /// The pocket's root cap: centre, radius and where it meets the offset waist arc.
    cap_c: [f64; 2],
    cap_r: f64,
    cap_t: [f64; 2],
}

const S45: f64 = std::f64::consts::FRAC_1_SQRT_2;

fn dist(p: [f64; 2], q: [f64; 2]) -> f64 {
    (p[0] - q[0]).hypot(p[1] - q[1])
}

/// `p + k · (q − p) / |q − p|`.
fn toward(p: [f64; 2], q: [f64; 2], k: f64) -> [f64; 2] {
    let l = dist(p, q);
    [p[0] + k * (q[0] - p[0]) / l, p[1] + k * (q[1] - p[1]) / l]
}

/// The root of `f` in `[lo, hi]`, where it changes sign.
fn bisect(f: impl Fn(f64) -> f64, mut lo: f64, mut hi: f64) -> f64 {
    let up = f(hi) > f(lo);
    for _ in 0..200 {
        let m = (lo + hi) / 2.0;
        if (f(m) > 0.0) == up {
            hi = m;
        } else {
            lo = m;
        }
    }
    (lo + hi) / 2.0
}

/// `∫ √(R² − (x − c)²) dx`.
fn sq(x: f64, c: f64, r: f64) -> f64 {
    let y = x - c;
    0.5 * (y * (r * r - y * y).max(0.0).sqrt() + r * r * (y / r).clamp(-1.0, 1.0).asin())
}

impl Hand {
    fn new() -> Self {
        let d = slant_dims(&StudProfile::slant_v2_experimental()).unwrap();
        let (r, a) = (2.4, 4.0);
        // Waist arcs about (±4, −4), 0.05 mm clear of a Ø6.51 tube: a 1.39 mm waist.
        let rw = 3.255 + 0.05;
        let (tf, ti) = ([a, -a], [-a, -a]);
        assert!((d.waist_width - 2.0 * (a - rw)).abs() < 1e-12);
        // Fin bump: rightmost point (2.85, −0.3), touching the fin-side waist arc outside.
        let rho = bisect(|k| dist([2.85 - k, -0.3], tf) - k - rw, 0.01, 2.8);
        let cf = [2.85 - rho, -0.3];
        let j = toward(cf, tf, rho);
        // Blend: about −k·(cos 40°, sin 40°), radius r + k; touches the bump inside.
        let dir = [40f64.to_radians().cos(), 40f64.to_radians().sin()];
        let at = |k: f64| [-k * dir[0], -k * dir[1]];
        let k = bisect(|k| dist(cf, at(k)) - (r + k - rho), 0.0, 1000.0);
        let (ce, re) = (at(k), r + k);
        let p = toward(ce, cf, re);
        let t = [r * dir[0], r * dir[1]];
        // Inner round: 0.5 mm, inside the stud circle, outside the inner waist arc.
        let rg = 0.5;
        let cg_at = |psi: f64| [(r - rg) * psi.cos(), (r - rg) * psi.sin()];
        let psi = bisect(|psi| dist(cg_at(psi), ti) - rw - rg, PI, 1.25 * PI);
        let cg = cg_at(psi);
        let g1 = [r * psi.cos(), r * psi.sin()];
        let g2 = toward(cg, ti, rg);
        for (got, want) in [
            (d.bump_center, cf),
            (d.bump_start, j),
            (d.blend_center, ce),
            (d.blend_start, p),
            (d.blend_end, t),
            (d.round_center, cg),
        ] {
            assert!(dist(got, want) < 1e-9, "{got:?} vs {want:?}");
        }
        // Slot: 0.5 wide, inner edge 0.25 outboard; it starts where the fin root (from its
        // outer edge to the waist arc) is 0.8 across. Hook radius 1.2 whose outer edge peaks
        // 0.75 below the stud circle's far edge.
        let (uc, hw, rh) = (0.5, 0.25, 1.2);
        let v0 = -a + (rw * rw - (a - uc - hw - 0.8_f64).powi(2)).sqrt();
        let vs = r - 0.75 - hw - rh;
        assert!((d.slot_center - uc).abs() < 1e-12);
        assert!((d.slot_v.0 - v0).abs() < 1e-12 && (d.slot_v.1 - vs).abs() < 1e-12);
        // The fin's tip: the hook's inner edge (radius 0.95 about the hook centre) meets the
        // blend circle between 45° and 90° about the hook centre (the hook's 45° end is
        // already outside the lobe, so the tail never touches the fin).
        let hc = [uc + rh, vs];
        let on_hook = |t: f64| [hc[0] + (rh - hw) * t.cos(), hc[1] + (rh - hw) * t.sin()];
        assert!(dist(on_hook(PI / 4.0), ce) > re);
        let th = bisect(|th| dist(on_hook(th), ce) - re, PI / 4.0, PI / 2.0);
        let tip = on_hook(th);
        let root = [uc + hw + 0.8, v0];
        assert!(dist(d.tip_corner, tip) < 1e-9 && dist(d.root_corner, root) < 1e-12);
        // The root cap: a circle through the root's corner, square there to the waist arc
        // (its centre along the waist's tangent from the corner), touching the waist arc's
        // 0.25 mm offset from outside.
        let m = 0.25;
        let n = toward(root, tf, 1.0);
        let t_dir = [-(n[1] - root[1]), n[0] - root[0]];
        let cap_at = |k: f64| [root[0] + k * t_dir[0], root[1] + k * t_dir[1]];
        let cap_r = bisect(|k| dist(cap_at(k), tf) - (rw - m + k), 0.01, 2.0);
        let cap_c = cap_at(cap_r);
        let cap_t = toward(tf, cap_c, rw - m);
        assert!(
            dist(d.root_cap_center, cap_c) < 1e-9,
            "{:?} vs {cap_c:?}",
            d.root_cap_center
        );
        Self {
            r,
            a,
            rw,
            cf,
            rho,
            j,
            ce,
            re,
            p,
            t,
            cg,
            rg,
            g1,
            g2,
            uc,
            hw,
            v0,
            vs,
            hc,
            rh,
            tail: d.tail,
            root,
            tip,
            m,
            cap_c,
            cap_r,
            cap_t,
        }
    }

    /// The peanut's right (fin side) and left edges at `vp` (peanut frame, `0 ≤ vp ≤ a + r`).
    fn edges(&self, vp: f64) -> (f64, f64) {
        let (a, w) = (self.a, vp - self.a);
        let waist = a - (self.rw * self.rw - vp * vp).max(0.0).sqrt();
        let circle = |c: [f64; 2], r: f64| (r * r - (w - c[1]).powi(2)).max(0.0).sqrt();
        let right = if w <= self.j[1] {
            waist
        } else if w <= self.p[1] {
            self.cf[0] + circle(self.cf, self.rho)
        } else if w <= self.t[1] {
            self.ce[0] + circle(self.ce, self.re)
        } else {
            circle([0.0, 0.0], self.r)
        };
        let left = if w <= self.g2[1] {
            -waist
        } else if w <= self.g1[1] {
            self.cg[0] - circle(self.cg, self.rg)
        } else {
            -circle([0.0, 0.0], self.r)
        };
        (right, left)
    }

    /// Inside the peanut (lobe B's frame).
    fn peanut(&self, q: [f64; 2]) -> bool {
        let vp = (q[1] + self.a).abs();
        if vp > self.a + self.r {
            return false;
        }
        let (right, left) = self.edges(vp);
        left <= q[0] && q[0] <= right
    }

    /// Angle of `q` about the hook centre, degrees.
    fn hook_angle(&self, q: [f64; 2]) -> f64 {
        (q[1] - self.hc[1]).atan2(q[0] - self.hc[0]).to_degrees()
    }

    fn m_end(&self) -> [f64; 2] {
        [self.hc[0] + self.rh * S45, self.hc[1] + self.rh * S45]
    }

    /// Inside the slot: within 0.25 mm of its centre line (straight, 135° hook, tail).
    fn slot(&self, q: [f64; 2]) -> bool {
        let straight = (q[0] - self.uc).abs() <= self.hw && q[1] >= self.v0 && q[1] <= self.vs;
        let t = self.hook_angle(q);
        let d = (q[0] - self.hc[0]).hypot(q[1] - self.hc[1]);
        let hook = q[1] >= self.vs && (45.0..=180.0).contains(&t) && (d - self.rh).abs() <= self.hw;
        let m = self.m_end();
        let (dx, dy) = (q[0] - m[0], q[1] - m[1]);
        let along = dx * S45 - dy * S45;
        let across = dx * S45 + dy * S45;
        let tail = (0.0..=self.tail).contains(&along) && across.abs() <= self.hw;
        straight || hook || tail
    }

    /// On the fin: past the slot's end, inside the peanut and not in the slot, beside the
    /// slot's straight part, or above it inside the hook's inner edge or short of the hook's
    /// 45° end (where the lobe runs on past the hook's inner circle, with no slot).
    fn fin(&self, q: [f64; 2]) -> bool {
        q[1] >= self.v0
            && self.peanut(q)
            && !self.slot(q)
            && if q[1] <= self.vs {
                q[0] >= self.uc + self.hw
            } else {
                dist(q, self.hc) <= self.rh - self.hw || self.hook_angle(q) < 45.0
            }
    }

    /// Distance from `q` to the fin's free edges: the waist arc from the root's corner, the
    /// bump, the blend to the tip, the hook's inner edge and the slot's straight edge (each
    /// arc turns by under 180°).
    fn to_free_edges(&self, q: [f64; 2]) -> f64 {
        let arc = |c: [f64; 2], r: f64, from: [f64; 2], to: [f64; 2]| {
            let cross = |p: [f64; 2], s: [f64; 2]| {
                (p[0] - c[0]) * (s[1] - c[1]) - (p[1] - c[1]) * (s[0] - c[0])
            };
            let turn = cross(from, to).signum();
            if cross(from, q) * turn >= 0.0 && cross(q, to) * turn >= 0.0 {
                (dist(q, c) - r).abs()
            } else {
                dist(q, from).min(dist(q, to))
            }
        };
        let (a, p3) = (self.a, [self.uc + self.hw, self.vs]);
        let straight = if q[1] < self.v0 {
            dist(q, [p3[0], self.v0])
        } else if q[1] > self.vs {
            dist(q, p3)
        } else {
            (q[0] - p3[0]).abs()
        };
        [
            arc([a, -a], self.rw, self.root, self.j),
            arc(self.cf, self.rho, self.j, self.p),
            arc(self.ce, self.re, self.p, self.tip),
            arc(self.hc, self.rh - self.hw, self.tip, p3),
            straight,
        ]
        .into_iter()
        .fold(f64::INFINITY, f64::min)
    }

    /// Inside the pocket: on the fin, or where the rest of the stud does not stand (in the
    /// slot or beside the stud) and either inside the root cap's circle, in the wedge about
    /// the waist arc's centre between the root's corner and where the cap meets the offset
    /// arc, or elsewhere at or above the slot's end within the margin of the fin's free edges.
    fn pocket(&self, q: [f64; 2]) -> bool {
        if self.fin(q) {
            return true;
        }
        if self.peanut(q) && !self.slot(q) {
            return false;
        }
        let tf = [self.a, -self.a];
        let cross = |p: [f64; 2], s: [f64; 2]| {
            (p[0] - tf[0]) * (s[1] - tf[1]) - (p[1] - tf[1]) * (s[0] - tf[0])
        };
        let turn = cross(self.root, self.cap_t).signum();
        let in_wedge = cross(self.root, q) * turn >= 0.0
            && cross(q, self.cap_t) * turn >= 0.0
            && dist(q, tf) <= self.rw;
        if in_wedge {
            dist(q, self.cap_c) <= self.cap_r
        } else {
            q[1] >= self.v0 && self.to_free_edges(q) <= self.m
        }
    }

    /// Twice the integral of the peanut's fin-side (`right`) and inner-side (`left`) half
    /// widths over lobe B's half, `0 ≤ vp ≤ a + r`, closed form arc by arc; and the inner
    /// side's integral over `0 ≤ vp ≤ a` only (`left_to_centre`).
    fn halves(&self) -> (f64, f64, f64) {
        let (a, r) = (self.a, self.r);
        let span = |f: &dyn Fn(f64) -> f64, x0: f64, x1: f64| f(x1) - f(x0);
        let waist = |x: f64| a * x - sq(x, 0.0, self.rw);
        let arc =
            |c: [f64; 2], rad: f64, sign: f64| move |x: f64| sign * c[0] * x + sq(x, c[1] + a, rad);
        let (jv, pv, tv) = (self.j[1] + a, self.p[1] + a, self.t[1] + a);
        let (g2v, g1v, top) = (self.g2[1] + a, self.g1[1] + a, a + r);
        let right = span(&waist, 0.0, jv)
            + span(&arc(self.cf, self.rho, 1.0), jv, pv)
            + span(&arc(self.ce, self.re, 1.0), pv, tv)
            + span(&arc([0.0, 0.0], r, 0.0), tv, top);
        let near = span(&waist, 0.0, g2v) + span(&arc(self.cg, self.rg, -1.0), g2v, g1v);
        let left = near + span(&arc([0.0, 0.0], r, 0.0), g1v, top);
        let left_to_centre = near + span(&arc([0.0, 0.0], r, 0.0), g1v, a);
        (right, left, left_to_centre)
    }

    /// Peanut (finned pair) area.
    fn peanut_area(&self) -> f64 {
        let (right, left, _) = self.halves();
        2.0 * (right + left)
    }

    /// The area a triple adds to a pair: its middle lobe from waist line to waist line, the
    /// inner side's shape on both sides of it, mirrored about its centre.
    fn middle_area(&self) -> f64 {
        4.0 * self.halves().2
    }

    /// A finless pair: the inner side's shape on both sides.
    fn plain_pair_area(&self) -> f64 {
        4.0 * self.halves().1
    }
}

/// Areas by counting the centres of 2 µm cells: (slot ∩ peanut, slot ∖ pocket, pocket).
fn sampled_areas(h: &Hand) -> (f64, f64, f64) {
    let step = 0.002;
    let (lo, hi) = ([-0.5, -4.0], [3.6, 2.8]);
    let nx = ((hi[0] - lo[0]) / step) as i32;
    let ny = ((hi[1] - lo[1]) / step) as i32;
    let (mut sp, mut so, mut po) = (0u64, 0u64, 0u64);
    for i in 0..nx {
        for j in 0..ny {
            let q = [
                lo[0] + (f64::from(i) + 0.5) * step,
                lo[1] + (f64::from(j) + 0.5) * step,
            ];
            let pocket = h.pocket(q);
            if pocket {
                po += 1;
            }
            if h.slot(q) {
                if h.peanut(q) {
                    sp += 1;
                }
                if !pocket {
                    so += 1;
                }
            }
        }
    }
    let cell = step * step;
    (sp as f64 * cell, so as f64 * cell, po as f64 * cell)
}

fn tube(hgt: f64) -> f64 {
    PI * (3.255f64.powi(2) - 2.4f64.powi(2)) * hgt
}

fn chamfer_rect(l: f64, d: f64) -> f64 {
    let c: f64 = 0.2;
    c * c * (l + d) - 4.0 * c.powi(3) / 3.0
}

fn chamfer_tube() -> f64 {
    let c: f64 = 0.2;
    PI * c * c * (3.255 + 2.4)
}

fn rib(hollow: f64) -> f64 {
    let (ro, mid): (f64, f64) = (3.255, 2.8275);
    let chord = 0.5 * (ro * ro - 0.25f64).sqrt() + ro * ro * (0.5 / ro).asin();
    2.0 * ((8.0 - mid - 1.3) - (chord - mid)) * (hollow - 2.2)
}

/// The stage deltas of a built part, by name.
fn deltas(part: &CustomPart) -> Vec<(String, f64)> {
    let mut before = 0.0;
    part.stage_volumes_mm3
        .iter()
        .map(|(n, v)| {
            let d = v - before;
            before = *v;
            (n.clone(), d)
        })
        .collect()
}

fn delta(part: &CustomPart, name: &str) -> f64 {
    deltas(part)
        .into_iter()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("no stage {name}"))
        .1
}

fn close(name: &str, got: f64, want: f64, rel: f64) {
    let e = (got - want) / want;
    println!(
        "  {name}: measured {got:.4} mm³, hand {want:.4} mm³ ({:+.4} %)",
        e * 100.0
    );
    assert!(e.abs() < rel, "{name}: {got} vs {want}");
}

/// How many units of each kind a brick gets.
#[derive(Clone, Copy)]
struct Units {
    pairs: usize,
    triples: usize,
    plain_pairs: usize,
    plain_triples: usize,
}

/// Builds a fused `w × d` brick (sharp corners) and checks it stage by stage against
/// `units`, with `ribs` centre ribs.
fn check_v2(w: u32, d: u32, u: Units, ribs: usize) -> CustomPart {
    let h = Hand::new();
    let (slot_peanut, slot_out, pocket) = sampled_areas(&h);
    let shape = CustomShape::Box { w, d, plates: 3 };
    let part = build(&shape, &StudProfile::slant_v2_experimental())
        .unwrap_or_else(|e| panic!("v2 {w}x{d}: {e}"));
    let name = format!("v2_box_{w}x{d}x3");
    println!("{name}: {:?}", part.stage_volumes_mm3);
    let lobes = 2.0 * (u.pairs + u.triples) as f64;
    let (l, dd) = (f64::from(w) * 8.0 - 0.2, f64::from(d) * 8.0 - 0.2);
    let tubes = f64::from((w - 1) * (d - 1));
    // Top 1.4 mm: the hollow is 8.2 mm tall.
    let shell = l * dd * 9.6 - (l - 2.4) * (dd - 2.4) * 8.2;
    let pockets = pocket * 0.6 * lobes;
    let (pair, mid, plain) = (h.peanut_area(), h.middle_area(), h.plain_pair_area());
    let studs = (pair * u.pairs as f64
        + (pair + mid) * u.triples as f64
        + plain * u.plain_pairs as f64
        + (plain + mid) * u.plain_triples as f64)
        * 1.8;
    // The slot grooves the top as deep as the pocket (0.6), so the relief floor is flat.
    let slots = (slot_peanut * 1.8 + slot_out * 0.6) * lobes;
    let chamfer = chamfer_rect(l, dd) + tubes * chamfer_tube();
    let hand =
        shell + tubes * tube(8.2) + ribs as f64 * rib(8.2) - chamfer - pockets + studs - slots;
    close("Studs", delta(&part, "Studs"), studs, 1e-6);
    if lobes > 0.0 {
        close("Pockets", -delta(&part, "Pockets"), pockets, 5e-4);
        close("Slots", -delta(&part, "Slots"), slots, 3e-3);
    }
    close("Chamfer", -delta(&part, "Chamfer"), chamfer, 1e-6);
    if ribs > 0 {
        close("Ribs", delta(&part, "Ribs"), ribs as f64 * rib(8.2), 1e-6);
    }
    close("total", part.volume_mm3, hand, 1e-4);
    let mesh = analyze_stl(&part.stl).unwrap();
    assert!(mesh.is_watertight(), "{name}: {mesh:?}");
    for k in 0..3 {
        let (lo, hi) = ([0.1, 0.1, 0.0], [l + 0.1, dd + 0.1, 11.4]);
        assert!(
            (part.bbox_mm.0[k] - lo[k]).abs() < 1e-3,
            "{:?}",
            part.bbox_mm
        );
        assert!(
            (part.bbox_mm.1[k] - hi[k]).abs() < 1e-3,
            "{:?}",
            part.bbox_mm
        );
        assert!(
            (mesh.bbox_mm.1[k] - hi[k]).abs() < 1e-3,
            "{:?}",
            mesh.bbox_mm
        );
    }
    std::fs::create_dir_all(OUT).unwrap();
    std::fs::write(format!("{OUT}/{name}.stl"), &part.stl).unwrap();
    part
}

fn units(pairs: usize, triples: usize, plain_pairs: usize, plain_triples: usize) -> Units {
    Units {
        pairs,
        triples,
        plain_pairs,
        plain_triples,
    }
}

#[test]
fn v2_2x4_two_peanut_rows_with_centre_rib() {
    check_v2(2, 4, units(4, 0, 0, 0), 1);
}

#[test]
fn v2_2x2_one_peanut_per_row() {
    check_v2(2, 2, units(2, 0, 0, 0), 0);
}

#[test]
fn v2_2x3_one_triple_per_row() {
    check_v2(2, 3, units(0, 2, 0, 0), 1);
}

#[test]
fn v2_1x5_a_pair_and_a_triple() {
    check_v2(1, 5, units(1, 1, 0, 0), 0);
}

#[test]
fn v2_3x5_middle_row_fused_without_fins() {
    // Rows along Y: the outer rows get a finned pair and triple each, the middle row a
    // finless pair and triple.
    check_v2(3, 5, units(2, 2, 1, 1), 0);
}

#[test]
fn lone_studs_are_refused_not_made_round() {
    for shape in [
        CustomShape::Box {
            w: 1,
            d: 1,
            plates: 3,
        },
        CustomShape::Box {
            w: 1,
            d: 1,
            plates: 1,
        },
        // A T: a lone stud whichever way the rows run.
        CustomShape::Footprint {
            cells: vec![(0, 0), (1, 0), (2, 0), (1, 1)],
            plates: 3,
        },
    ] {
        let e = build(&shape, &StudProfile::fused())
            .unwrap_err()
            .to_string();
        assert!(e.contains("alone"), "{}: {e}", shape.label());
    }
    // A 1-wide slope keeps one stud on its back row: refused too.
    let e = build(
        &CustomShape::Slope45 {
            w: 1,
            d: 2,
            plates: 3,
        },
        &StudProfile::fused(),
    )
    .unwrap_err();
    assert!(e.to_string().contains("alone"));
}

#[test]
fn v2_fins_hang_only_from_their_roots() {
    // Cut the root of every fin of a 2×2 (a box over the waist on the fin side, from the
    // pocket floor up): each fin then falls away as its own body.
    let mut batch = commands(
        &CustomShape::Box {
            w: 2,
            d: 2,
            plates: 3,
        },
        &StudProfile::slant_v2_experimental(),
    )
    .unwrap();
    let steps = batch["commands"].as_array_mut().unwrap();
    // Row 0's fins face −X from x = 4 − u, row 1's face +X from x = 12 + u; the waist is at
    // y = 8 and the slots end 2.22 mm either side of it.
    for (x0, x1) in [(0.5, 3.5), (12.5, 15.5)] {
        steps.push(json!({"command": "solid.box", "params": {
            "length": x1 - x0, "width": 4.8, "height": 3.0,
            "corner": [x0, 5.6, 8.9], "operation": "cut"}}));
    }
    batch["expect"] = json!({"bodies": 5});
    let part = build_from_commands(&batch).unwrap_or_else(|e| panic!("{e}"));
    assert!(analyze_stl(&part.stl).unwrap().is_watertight());
    // The same boxes on the inner side of each peanut cut nothing free.
    let mut batch = commands(
        &CustomShape::Box {
            w: 2,
            d: 2,
            plates: 3,
        },
        &StudProfile::slant_v2_experimental(),
    )
    .unwrap();
    let steps = batch["commands"].as_array_mut().unwrap();
    for (x0, x1) in [(4.25, 7.0), (9.0, 11.75)] {
        steps.push(json!({"command": "solid.box", "params": {
            "length": x1 - x0, "width": 4.8, "height": 3.0,
            "corner": [x0, 5.6, 8.9], "operation": "cut"}}));
    }
    batch["expect"] = json!({"bodies": 1});
    build_from_commands(&batch).unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn standard_bricks_get_a_rib_every_second_tube() {
    for (d, ribs) in [(2u32, 0usize), (3, 1), (4, 1), (6, 2), (8, 3)] {
        let shape = CustomShape::Box { w: 2, d, plates: 3 };
        let plan = sf_custom::plan(&shape, &StudProfile::standard_fdm()).unwrap();
        let rib_stage = plan.stages.iter().find(|s| s.name == "Ribs");
        let bars = rib_stage.map_or(0, |s| {
            s.commands
                .iter()
                .filter(|c| c["command"] == "sketch.rectangle.two_point")
                .count()
        });
        assert_eq!(bars, 2 * ribs, "2x{d}");
    }
    // Rotated (long side along X) and switched off.
    let plan = sf_custom::plan(
        &CustomShape::Box {
            w: 6,
            d: 2,
            plates: 3,
        },
        &StudProfile::standard_fdm(),
    )
    .unwrap();
    assert!(plan.stages.iter().any(|s| s.name == "Ribs"));
    let off = StudProfile {
        center_ribs: false,
        ..StudProfile::standard_fdm()
    };
    let plan = sf_custom::plan(
        &CustomShape::Box {
            w: 2,
            d: 6,
            plates: 3,
        },
        &off,
    )
    .unwrap();
    assert!(!plan.stages.iter().any(|s| s.name == "Ribs"));
    // A plate is too low for a rib.
    let plan = sf_custom::plan(
        &CustomShape::Box {
            w: 2,
            d: 6,
            plates: 1,
        },
        &StudProfile::standard_fdm(),
    )
    .unwrap();
    assert!(!plan.stages.iter().any(|s| s.name == "Ribs"));
    // A 6×2 builds (ribs along Y).
    let part = build(
        &CustomShape::Box {
            w: 6,
            d: 2,
            plates: 3,
        },
        &StudProfile::standard_fdm(),
    )
    .unwrap();
    assert!(analyze_stl(&part.stl).unwrap().is_watertight());
}

#[test]
fn v2_on_slopes_footprints_and_single_rows() {
    let v2 = StudProfile::fused();
    for shape in [
        CustomShape::Box {
            w: 1,
            d: 3,
            plates: 3,
        },
        CustomShape::Box {
            w: 3,
            d: 3,
            plates: 3,
        },
        CustomShape::Curve {
            w: 3,
            d: 3,
            plates: 3,
        },
        CustomShape::Box {
            w: 1,
            d: 4,
            plates: 3,
        },
        CustomShape::Box {
            w: 4,
            d: 1,
            plates: 3,
        },
        CustomShape::Slope45 {
            w: 2,
            d: 3,
            plates: 3,
        },
        CustomShape::Footprint {
            cells: vec![(0, 0), (1, 0), (2, 0), (0, 1), (1, 1), (0, 2), (1, 2)],
            plates: 3,
        },
        CustomShape::Box {
            w: 2,
            d: 2,
            plates: 1,
        },
    ] {
        let part = build(&shape, &v2).unwrap_or_else(|e| panic!("{}: {e}", shape.label()));
        assert!(
            analyze_stl(&part.stl).unwrap().is_watertight(),
            "{}",
            shape.label()
        );
    }
}

#[test]
#[allow(deprecated)]
fn old_name_is_the_same_profile() {
    assert_eq!(
        StudProfile::compliant_v2_experimental(),
        StudProfile::slant_v2_experimental()
    );
    // A pocket needs 0.8 mm of floor: the standard 1.0 mm top is too thin for v2 fins.
    let thin = StudProfile {
        top_thickness: 1.0,
        ..StudProfile::slant_v2_experimental()
    };
    assert!(
        build(
            &CustomShape::Box {
                w: 2,
                d: 2,
                plates: 3
            },
            &thin
        )
        .is_err()
    );
}
