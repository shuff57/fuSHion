//! Display mesh of a printed bouquet stem ([`sf_core::stem::StemDef`]): a round rod swept
//! along its centreline, stepping down to LEGO bar size on its bar sections and the tip
//! bar, its side branches swept the same way, buds and blades on bare ends, and the keyed
//! peg below the base shoulder. Smooth shaded, with edge lines only at
//! the steps and ends. The flat underside it prints on is left out: it is out of sight on
//! the model and too small to matter in a thumbnail.

use crate::parse::{MAIN_COLOR, PartMesh};
use sf_core::stem::{BAR_D, LDU_PER_MM, PEG_D, PEG_LEN, StemDef, StemTip};

/// Sides of the rod's cross-section.
const SIDES: usize = 12;
/// Longest distance between rings along the rod, mm.
const RING_STEP: f32 = 4.0;

type V = [f32; 3];

fn add(a: V, b: V) -> V {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn mul(a: V, s: f32) -> V {
    [a[0] * s, a[1] * s, a[2] * s]
}

/// One cross-section ring: centre, the two unit vectors spanning its plane, the unit
/// tangent (the rod's direction there), and the radius (LDU).
#[derive(Clone, Copy)]
struct Ring {
    c: V,
    n: V,
    b: V,
    t: V,
    r: f32,
}

impl Ring {
    fn at(&self, k: usize) -> (V, V) {
        let a = k as f32 / SIDES as f32 * std::f32::consts::TAU;
        let dir = add(mul(self.n, a.cos()), mul(self.b, a.sin()));
        (add(self.c, mul(dir, self.r)), dir)
    }
}

/// Push a triangle with per-vertex normals.
fn tri(m: &mut PartMesh, p: [V; 3], n: [V; 3]) {
    let base = m.positions.len() as u32;
    m.positions.extend_from_slice(&p);
    m.normals.extend_from_slice(&n);
    m.colors.extend_from_slice(&[MAIN_COLOR; 3]);
    m.indices.extend_from_slice(&[base, base + 1, base + 2]);
}

/// The rod's surface between two rings of the same radius (smooth), or the flat step
/// between two rings at the same place with different radii.
fn band(m: &mut PartMesh, a: &Ring, b: &Ring) {
    let step = (a.r - b.r).abs() > 1e-4 && {
        let d = [b.c[0] - a.c[0], b.c[1] - a.c[1], b.c[2] - a.c[2]];
        d.iter().map(|v| v * v).sum::<f32>() < 1e-6
    };
    // A step faces back down the rod where it narrows toward the tip, up where it widens.
    let face = if a.r > b.r { a.t } else { mul(a.t, -1.0) };
    for k in 0..SIDES {
        let (a0, na0) = a.at(k);
        let (a1, na1) = a.at(k + 1);
        let (b0, nb0) = b.at(k);
        let (b1, nb1) = b.at(k + 1);
        if step {
            tri(m, [a0, a1, b1], [face; 3]);
            tri(m, [a0, b1, b0], [face; 3]);
        } else {
            tri(m, [a0, a1, b1], [na0, na1, nb1]);
            tri(m, [a0, b1, b0], [na0, nb1, nb0]);
        }
    }
    if step {
        edge_ring(m, a);
        edge_ring(m, b);
    }
}

fn edge_ring(m: &mut PartMesh, r: &Ring) {
    for k in 0..SIDES {
        m.edges.push([r.at(k).0, r.at(k + 1).0]);
    }
}

/// A flat end cap on a ring, facing `out`.
fn cap(m: &mut PartMesh, r: &Ring, out: V) {
    for k in 0..SIDES {
        tri(m, [r.c, r.at(k).0, r.at(k + 1).0], [out; 3]);
    }
    edge_ring(m, r);
}

/// One rod to sweep: a centreline of `len` mm from `point(0)`, `diameter` mm thick,
/// thinning to [`BAR_D`] on `bars` (mm ranges along it) and on a `tip_bar` at its end.
struct Rod<'a> {
    point: &'a dyn Fn(f32) -> V,
    tangent: &'a dyn Fn(f32) -> V,
    len: f32,
    diameter: f32,
    bars: Vec<(f32, f32)>,
    tip_bar: f32,
}

/// Sweep a rod. Returns its rings (first: the base, last: the end), with the end left open.
fn sweep(m: &mut PartMesh, rod: &Rod) -> Vec<Ring> {
    let len = rod.len.max(1.0);
    let tip_start = (len - rod.tip_bar.max(0.0)).max(0.0);
    let bars: Vec<(f32, f32)> = rod
        .bars
        .iter()
        .map(|&(a, b)| (a.max(0.0), b.min(len)))
        .filter(|(a, b)| b > a)
        .chain((rod.tip_bar > 0.0).then_some((tip_start, len)))
        .collect();
    let radius = |s: f32| {
        let thin = bars.iter().any(|&(a, b)| s > a + 1e-4 && s < b - 1e-4);
        if thin { BAR_D.min(rod.diameter) / 2.0 } else { rod.diameter / 2.0 }
    };
    // Stations along the rod: regular rings plus every bar boundary, each boundary twice
    // (both radii) so the step is a sharp flat ring.
    let mut stations: Vec<f32> = (0..=((len / RING_STEP).ceil() as usize))
        .map(|i| (i as f32 * RING_STEP).min(len))
        .collect();
    for &(a, b) in &bars {
        stations.extend([a, b]);
    }
    stations.sort_by(f32::total_cmp);
    stations.dedup_by(|a, b| (*a - *b).abs() < 1e-3);
    let ring = |s: f32, r: f32| {
        let t = (rod.tangent)(s);
        // Normal in the stem's plane, square to the tangent; binormal along local +Z.
        Ring { c: (rod.point)(s), n: [-t[1], t[0], 0.0], b: [0.0, 0.0, 1.0], t, r: r * LDU_PER_MM }
    };
    let mut rings: Vec<Ring> = Vec::new();
    for &s in &stations {
        let (before, after) = (radius(s - 1e-3), radius(s + 1e-3));
        if s >= len - 1e-3 {
            rings.push(ring(s, before));
            continue;
        }
        if s > 0.0 && (before - after).abs() > 1e-4 {
            rings.push(ring(s, before));
        }
        rings.push(ring(s, after));
    }
    for w in rings.windows(2) {
        band(m, &w[0], &w[1]);
    }
    rings
}

/// Close a rod's end: a flat cap, or the tip shape printed with it.
fn finish(m: &mut PartMesh, end: &Ring, tip: Option<StemTip>) {
    match tip {
        None => cap(m, end, end.t),
        Some(StemTip::Bud { length, width }) => {
            cap(m, end, end.t);
            bud(m, end, length * LDU_PER_MM, width * LDU_PER_MM);
        }
        Some(StemTip::Blade { length, width }) => {
            cap(m, end, end.t);
            blade(m, end, length * LDU_PER_MM, width * LDU_PER_MM);
        }
    }
}

/// A closed bud on a rod's end: a pointed oval of revolution, widest a third of the way
/// up, starting a little inside the rod so it sits on it.
fn bud(m: &mut PartMesh, end: &Ring, len: f32, width: f32) {
    const STEPS: usize = 14;
    let start = add(end.c, mul(end.t, -0.1 * len));
    // r(x) ∝ √x·(1 - x): round at the base, pointed at the top; max at x = 1/3.
    let k = width / 2.0 / ((1.0f32 / 3.0).sqrt() * (2.0 / 3.0));
    let prof = |x: f32| k * x.max(0.0).sqrt() * (1.0 - x);
    let slope = |x: f32| {
        let e = 1e-3;
        (prof((x + e).min(1.0)) - prof((x - e).max(0.0))) / (((x + e).min(1.0) - (x - e).max(0.0)) * len)
    };
    let mut prev: Option<Vec<(V, V)>> = None;
    for i in 0..=STEPS {
        let x = i as f32 / STEPS as f32;
        let r = Ring { c: add(start, mul(end.t, x * len)), r: prof(x), ..*end };
        let ds = slope(x.clamp(0.02, 0.98));
        let ring: Vec<(V, V)> = (0..=SIDES)
            .map(|k| {
                let (p, dir) = r.at(k);
                let n = add(dir, mul(end.t, -ds));
                let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-6);
                (p, mul(n, 1.0 / l))
            })
            .collect();
        if let Some(pr) = &prev {
            for k in 0..SIDES {
                let (a0, a1, b0, b1) = (pr[k], pr[k + 1], ring[k], ring[k + 1]);
                tri(m, [a0.0, a1.0, b1.0], [a0.1, a1.1, b1.1]);
                tri(m, [a0.0, b1.0, b0.0], [a0.1, b1.1, b0.1]);
            }
        }
        prev = Some(ring);
    }
}

/// A flat blade on a rod's end, in the stem's plane, 1.6 mm thick: full width near its
/// base, tapering to a point.
fn blade(m: &mut PartMesh, end: &Ring, len: f32, width: f32) {
    const STEPS: usize = 12;
    let half_t = 0.8 * LDU_PER_MM;
    let start = add(end.c, mul(end.t, -0.5 * end.r));
    let half_w = |x: f32| width / 2.0 * (1.0 - x).powf(0.8) * (0.35 + 0.65 * (x * 5.0).min(1.0));
    // Outline points along both edges, for z = ±half_t.
    let edge = |x: f32, side: f32, z: f32| add(add(add(start, mul(end.t, x * len)), mul(end.n, side * half_w(x))), mul(end.b, z));
    for i in 0..STEPS {
        let (x0, x1) = (i as f32 / STEPS as f32, (i + 1) as f32 / STEPS as f32);
        for z in [-half_t, half_t] {
            let face = mul(end.b, z.signum());
            let q = [edge(x0, -1.0, z), edge(x0, 1.0, z), edge(x1, 1.0, z), edge(x1, -1.0, z)];
            tri(m, [q[0], q[1], q[2]], [face; 3]);
            tri(m, [q[0], q[2], q[3]], [face; 3]);
            m.edges.push([q[0], q[3]]);
            m.edges.push([q[1], q[2]]);
        }
        for side in [-1.0f32, 1.0] {
            let out = mul(end.n, side);
            let q = [edge(x0, side, -half_t), edge(x1, side, -half_t), edge(x1, side, half_t), edge(x0, side, half_t)];
            tri(m, [q[0], q[1], q[2]], [out; 3]);
            tri(m, [q[0], q[2], q[3]], [out; 3]);
        }
    }
}

/// The display mesh of a stem in its own frame (LDU, `-Y` up, base shoulder at the origin,
/// bending toward `+X`; see `sf_core::stem`): the main rod with its bars, tip bar or tip,
/// each branch with its own, and the peg below the shoulder.
pub fn stem_mesh(def: &StemDef) -> PartMesh {
    let mut m = PartMesh::default();
    let point = |s: f32| def.point(s);
    let tangent = |s: f32| def.tangent(s);
    let main = Rod {
        point: &point,
        tangent: &tangent,
        len: def.length,
        diameter: def.diameter,
        bars: def.bars.iter().map(|b| (b.at - b.length / 2.0, b.at + b.length / 2.0)).collect(),
        tip_bar: def.tip_bar,
    };
    let rings = sweep(&mut m, &main);
    if let Some(last) = rings.last() {
        finish(&mut m, last, if def.tip_bar > 0.0 { None } else { def.tip });
    }
    for (i, b) in def.branches.iter().enumerate() {
        let bp = |u: f32| def.branch_point(i, u);
        let bt = |u: f32| def.branch_tangent(i, u);
        let rod = Rod {
            point: &bp,
            tangent: &bt,
            len: b.length,
            diameter: def.branch_diameter(i),
            bars: Vec::new(),
            tip_bar: b.tip_bar,
        };
        let rings = sweep(&mut m, &rod);
        // Its base sits inside the main stem; close it anyway.
        if let Some(first) = rings.first() {
            cap(&mut m, first, mul(first.t, -1.0));
        }
        if let Some(last) = rings.last() {
            finish(&mut m, last, if b.tip_bar > 0.0 { None } else { b.tip });
        }
    }
    // The peg below the shoulder: a step in to the peg's radius, the peg, its end.
    let down = [0.0, 1.0, 0.0];
    let peg = |y: f32, r: f32| Ring {
        c: [0.0, y * LDU_PER_MM, 0.0],
        n: [1.0, 0.0, 0.0],
        b: [0.0, 0.0, 1.0],
        t: [0.0, -1.0, 0.0],
        r: r * LDU_PER_MM,
    };
    let (top, bottom) = (peg(0.0, PEG_D / 2.0), peg(PEG_LEN, PEG_D / 2.0));
    if let Some(shoulder) = rings.first() {
        band(&mut m, &top, shoulder);
    }
    band(&mut m, &top, &bottom);
    cap(&mut m, &bottom, down);
    m
}

/// A content key for a stem's display mesh: stems with equal shapes share a mesh (the id
/// and style name don't change the shape).
pub fn stem_key(def: &StemDef) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let f = |v: f32, h: &mut std::collections::hash_map::DefaultHasher| v.to_bits().hash(h);
    let tip = |t: &Option<StemTip>, h: &mut std::collections::hash_map::DefaultHasher| match t {
        None => 0u8.hash(h),
        Some(StemTip::Bud { length, width }) => (1u8, length.to_bits(), width.to_bits()).hash(h),
        Some(StemTip::Blade { length, width }) => (2u8, length.to_bits(), width.to_bits()).hash(h),
    };
    for v in [def.length, def.bend, def.tip_bar, def.diameter] {
        f(v, &mut h);
    }
    tip(&def.tip, &mut h);
    for b in &def.bars {
        (b.at.to_bits(), b.length.to_bits()).hash(&mut h);
    }
    for b in &def.branches {
        for v in [b.at, b.angle, b.length, b.bend, b.tip_bar, b.diameter.unwrap_or(-1.0)] {
            f(v, &mut h);
        }
        tip(&b.tip, &mut h);
    }
    format!("stem:{:016x}", h.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sf_core::stem::{STEM_D, StemBar, StemBranch};

    fn def(bend: f32) -> StemDef {
        StemDef {
            id: "S01".into(),
            length: 100.0,
            bend,
            tip_bar: 8.0,
            bars: vec![StemBar { at: 50.0, length: 10.0 }],
            diameter: STEM_D,
            tip: None,
            branches: vec![],
            style: String::new(),
        }
    }

    fn radial(m: &PartMesh, up_mm: f32) -> f32 {
        // Largest distance from the axis among vertices at this height (straight stem).
        let y = -up_mm * LDU_PER_MM;
        m.positions
            .iter()
            .filter(|p| (p[1] - y).abs() < 0.5)
            .map(|p| (p[0] * p[0] + p[2] * p[2]).sqrt())
            .fold(0.0, f32::max)
    }

    #[test]
    fn straight_stem_steps_down_on_bars_and_has_a_peg() {
        let m = stem_mesh(&def(0.0));
        let (lo, hi) = m.bounds().unwrap();
        assert!((lo[1] + 100.0 * LDU_PER_MM).abs() < 0.01, "tip at 100 mm: {lo:?}");
        assert!((hi[1] - PEG_LEN * LDU_PER_MM).abs() < 0.01, "peg 10 mm below: {hi:?}");
        let rod = STEM_D / 2.0 * LDU_PER_MM;
        let bar = BAR_D / 2.0 * LDU_PER_MM;
        assert!((radial(&m, 20.0) - rod).abs() < 0.01);
        assert!((radial(&m, 48.0) - bar).abs() < 0.01, "leaf bar is bar-thin");
        assert!((radial(&m, 96.0) - bar).abs() < 0.01, "tip bar is bar-thin");
        assert!((radial(&m, -PEG_LEN) - PEG_D / 2.0 * LDU_PER_MM).abs() < 0.01, "peg");
        // Edge rings at the steps and ends only (not along the rod).
        assert!(!m.edges.is_empty() && m.edges.len() <= SIDES * 12, "{}", m.edges.len());
        // Every triangle has unit normals.
        assert!(m.normals.iter().all(|n| ((n[0] * n[0] + n[1] * n[1] + n[2] * n[2]) - 1.0).abs() < 1e-3));
    }

    #[test]
    fn bent_stem_leans_toward_plus_x_and_keys_follow_shape() {
        let m = stem_mesh(&def(40.0));
        let (lo, hi) = m.bounds().unwrap();
        assert!(hi[0] > 50.0 && lo[0] > -10.0, "{lo:?} {hi:?}");
        let mut other = def(40.0);
        other.id = "S99".into();
        assert_eq!(stem_key(&def(40.0)), stem_key(&other));
        assert_ne!(stem_key(&def(40.0)), stem_key(&def(0.0)));
    }

    #[test]
    fn branches_buds_and_blades() {
        let mut d = def(0.0);
        d.tip_bar = 0.0;
        d.tip = Some(StemTip::Bud { length: 12.0, width: 6.0 });
        let branch = |angle: f32, tip: Option<StemTip>, tip_bar: f32| StemBranch {
            at: 60.0,
            angle,
            length: 30.0,
            bend: 0.0,
            diameter: None,
            tip_bar,
            tip,
        };
        d.branches = vec![
            branch(40.0, None, 6.0),
            branch(-40.0, Some(StemTip::Blade { length: 20.0, width: 5.0 }), 0.0),
        ];
        let m = stem_mesh(&d);
        let (lo, hi) = m.bounds().unwrap();
        // The bud carries the top past the 100 mm rod; branches reach out to both sides.
        assert!(lo[1] < -(100.0 + 8.0) * LDU_PER_MM, "{lo:?}");
        let reach = 30.0 * (40.0f32).to_radians().sin() * LDU_PER_MM;
        assert!(hi[0] > reach && lo[0] < -reach, "{lo:?} {hi:?}");
        // Nothing is thicker than the 6 mm bud; the blade (left, past x < -reach/2) is 1.6 mm.
        assert!(m.positions.iter().all(|p| p[2].abs() <= 3.0 * LDU_PER_MM + 0.01));
        let blade_z = m.positions.iter().filter(|p| p[0] < -reach * 1.1).map(|p| p[2].abs()).fold(0.0, f32::max);
        assert!((blade_z - 0.8 * LDU_PER_MM).abs() < 0.01, "blade half thickness {blade_z}");
        // Shape changes change the key.
        let mut e = d.clone();
        e.branches[1].tip = Some(StemTip::Bud { length: 20.0, width: 5.0 });
        assert_ne!(stem_key(&d), stem_key(&e));
        e = d.clone();
        e.diameter = 4.0;
        assert_ne!(stem_key(&d), stem_key(&e));
        e = d.clone();
        e.style = "daisy sprig".into();
        assert_eq!(stem_key(&d), stem_key(&e));
    }
}
