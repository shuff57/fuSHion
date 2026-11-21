//! Display meshes for printed parts: merged body bricks ([`custom_mesh`]) and detail
//! parts Claude designed in SolidCraft ([`detail_mesh`]).
//!
//! Both are in the catalog frame used by `Placement::ldraw_transform`: LDU, LDraw axes
//! (`-Y` up), origin at the top centre of the part's local bounding box, the body filling
//! `0 ≤ y ≤ plates·8` below it. Printed parts have no yaw or offset of their own.

use crate::parse::{MAIN_COLOR, PartMesh};
use sf_core::print::stud_layout;
use sf_core::{CustomDef, DetailMesh};
use std::collections::{HashMap, HashSet};

const STUD_R: f32 = 6.0;
const STUD_H: f32 = 4.0;
const STUD_SEGMENTS: usize = 16;
/// Half width of the waist joining two fused studs (a real waist is 1.39 mm of a 4.8 mm
/// stud, about 0.3 of its diameter).
const WAIST_HALF: f32 = 1.8;
/// Feature edges are drawn where neighbouring faces of a detail mesh meet at more than this.
const CREASE_DEG: f32 = 35.0;

type V = [f32; 3];
/// A welded vertex: a position rounded to 0.01 LDU.
type Weld = (i64, i64, i64);

fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: V, b: V) -> V {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn dot(a: V, b: V) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// A triangle whose normal points along `out` (flipped if needed).
fn tri_out(m: &mut PartMesh, a: V, b: V, c: V, out: V) {
    if dot(cross(sub(b, a), sub(c, a)), out) >= 0.0 {
        m.push_tri(a, b, c, MAIN_COLOR);
    } else {
        m.push_tri(a, c, b, MAIN_COLOR);
    }
}

fn quad_out(m: &mut PartMesh, a: V, b: V, c: V, d: V, out: V) {
    tri_out(m, a, b, c, out);
    tri_out(m, a, c, d, out);
}

/// The display mesh of a printed body brick: its cells extruded `plates` plates (no faces
/// between neighbouring cells), outline edges, and its studs drawn as fused units (rows of
/// two or three studs joined by narrow waists). Columns left flat show no stud.
pub fn custom_mesh(def: &CustomDef) -> PartMesh {
    let [sx, sz] = def.size();
    let (hx, hz) = (sx as f32 * 10.0, sz as f32 * 10.0);
    let h = def.plates.max(1) as f32 * 8.0;
    let cells: HashSet<(i32, i32)> = def.cells.iter().map(|&(x, z)| (x as i32, z as i32)).collect();
    let x_of = |lx: i32| -hx + 20.0 * lx as f32;
    let z_of = |lz: i32| -hz + 20.0 * lz as f32;
    let mut m = PartMesh::default();

    for &(cx, cz) in &cells {
        let (x0, x1, z0, z1) = (x_of(cx), x_of(cx + 1), z_of(cz), z_of(cz + 1));
        // Top (y = 0, facing -y) and bottom (y = h, facing +y).
        quad_out(&mut m, [x0, 0.0, z0], [x1, 0.0, z0], [x1, 0.0, z1], [x0, 0.0, z1], [0.0, -1.0, 0.0]);
        quad_out(&mut m, [x0, h, z0], [x1, h, z0], [x1, h, z1], [x0, h, z1], [0.0, 1.0, 0.0]);
        // Sides where the neighbour is empty, with their top and bottom outline edges.
        let sides: [((i32, i32), V, V, V); 4] = [
            ((cx - 1, cz), [x0, 0.0, z0], [x0, 0.0, z1], [-1.0, 0.0, 0.0]),
            ((cx + 1, cz), [x1, 0.0, z0], [x1, 0.0, z1], [1.0, 0.0, 0.0]),
            ((cx, cz - 1), [x0, 0.0, z0], [x1, 0.0, z0], [0.0, 0.0, -1.0]),
            ((cx, cz + 1), [x0, 0.0, z1], [x1, 0.0, z1], [0.0, 0.0, 1.0]),
        ];
        for (nb, a, b, out) in sides {
            if cells.contains(&nb) {
                continue;
            }
            let (ad, bd) = ([a[0], h, a[2]], [b[0], h, b[2]]);
            quad_out(&mut m, a, b, bd, ad, out);
            m.edges.push([a, b]);
            m.edges.push([ad, bd]);
        }
    }
    // Vertical edges at the outline's corners: where the four cells around a grid point
    // aren't a straight wall (one or three filled, or two diagonal).
    let mut corners: HashSet<(i32, i32)> = HashSet::new();
    for &(cx, cz) in &cells {
        for (px, pz) in [(cx, cz), (cx + 1, cz), (cx, cz + 1), (cx + 1, cz + 1)] {
            corners.insert((px, pz));
        }
    }
    for (px, pz) in corners {
        let f = |x: i32, z: i32| cells.contains(&(x, z));
        let (a, b, c, d) = (f(px - 1, pz - 1), f(px, pz - 1), f(px - 1, pz), f(px, pz));
        let n = [a, b, c, d].iter().filter(|&&v| v).count();
        let corner = n == 1 || n == 3 || (n == 2 && a == d);
        if corner {
            let (x, z) = (x_of(px), z_of(pz));
            m.edges.push([[x, 0.0, z], [x, h, z]]);
        }
    }

    // Studs, as fused units.
    let units: Vec<Vec<(i32, i32)>> = if def.units.is_empty() {
        let studs: Vec<(i32, i32)> = def.studs.iter().map(|&(x, z)| (x as i32, z as i32)).collect();
        stud_layout(&studs, sx > sz, |_, _| None).0
    } else {
        def.units.iter().map(|u| u.iter().map(|&(x, z)| (x as i32, z as i32)).collect()).collect()
    };
    let centre = |(lx, lz): (i32, i32)| (x_of(lx) + 10.0, z_of(lz) + 10.0);
    for unit in &units {
        for &c in unit {
            let (cx, cz) = centre(c);
            stud(&mut m, cx, cz);
        }
        for w in unit.windows(2) {
            waist(&mut m, centre(w[0]), centre(w[1]));
        }
    }
    m
}

fn stud(m: &mut PartMesh, cx: f32, cz: f32) {
    let ring = |k: usize| {
        let a = k as f32 / STUD_SEGMENTS as f32 * std::f32::consts::TAU;
        (cx + STUD_R * a.cos(), cz + STUD_R * a.sin())
    };
    let top = [cx, -STUD_H, cz];
    for k in 0..STUD_SEGMENTS {
        let (x0, z0) = ring(k);
        let (x1, z1) = ring(k + 1);
        let mid = ((x0 + x1) / 2.0 - cx, (z0 + z1) / 2.0 - cz);
        quad_out(m, [x0, 0.0, z0], [x1, 0.0, z1], [x1, -STUD_H, z1], [x0, -STUD_H, z0], [mid.0, 0.0, mid.1]);
        tri_out(m, top, [x0, -STUD_H, z0], [x1, -STUD_H, z1], [0.0, -1.0, 0.0]);
        m.edges.push([[x0, -STUD_H, z0], [x1, -STUD_H, z1]]);
    }
}

/// The waist between two neighbouring stud centres: a low bar, as tall as the studs,
/// running between their cylinders.
fn waist(m: &mut PartMesh, a: (f32, f32), b: (f32, f32)) {
    let along_x = (a.1 - b.1).abs() < 1e-3;
    // The bar's ends sit inside the cylinders, where the cylinder is wider than the bar.
    let inset = (STUD_R * STUD_R - WAIST_HALF * WAIST_HALF).sqrt();
    let (lo, hi) = if along_x { (a.0.min(b.0), a.0.max(b.0)) } else { (a.1.min(b.1), a.1.max(b.1)) };
    let (s0, s1) = (lo + inset * 0.9, hi - inset * 0.9);
    let mid = if along_x { a.1 } else { a.0 };
    let p = |s: f32, t: f32, y: f32| if along_x { [s, y, mid + t] } else { [mid + t, y, s] };
    let w = WAIST_HALF;
    let side = |t: f32| if along_x { [0.0, 0.0, t.signum()] } else { [t.signum(), 0.0, 0.0] };
    // Top.
    quad_out(m, p(s0, -w, -STUD_H), p(s1, -w, -STUD_H), p(s1, w, -STUD_H), p(s0, w, -STUD_H), [0.0, -1.0, 0.0]);
    // Two long sides.
    for t in [-w, w] {
        quad_out(m, p(s0, t, 0.0), p(s1, t, 0.0), p(s1, t, -STUD_H), p(s0, t, -STUD_H), side(t));
        m.edges.push([p(s0, t, -STUD_H), p(s1, t, -STUD_H)]);
    }
}

/// The display mesh of a detail part. Flat shaded, with feature edges along open edges and
/// creases sharper than 35°. Vertices are welded by position for the crease test, so an
/// STL-style mesh (three vertices per triangle) gets the same edges as an indexed one.
pub fn detail_mesh(d: &DetailMesh) -> PartMesh {
    let mut m = PartMesh::default();
    let n = d.positions.len() as u32;
    let key = |p: V| ((p[0] * 100.0).round() as i64, (p[1] * 100.0).round() as i64, (p[2] * 100.0).round() as i64);
    // Undirected welded edge → (first endpoint positions, normals of the faces on it).
    let mut edges: HashMap<(Weld, Weld), ([V; 2], Vec<V>)> = HashMap::new();
    for t in d.indices.as_chunks::<3>().0 {
        if t.iter().any(|&i| i >= n) {
            continue;
        }
        let [a, b, c] = [d.positions[t[0] as usize], d.positions[t[1] as usize], d.positions[t[2] as usize]];
        let nrm = cross(sub(b, a), sub(c, a));
        let len = dot(nrm, nrm).sqrt();
        if len < 1e-9 || !len.is_finite() {
            continue;
        }
        let nrm = [nrm[0] / len, nrm[1] / len, nrm[2] / len];
        m.push_tri(a, b, c, MAIN_COLOR);
        for (p, q) in [(a, b), (b, c), (c, a)] {
            let (kp, kq) = (key(p), key(q));
            let k = if kp <= kq { (kp, kq) } else { (kq, kp) };
            edges.entry(k).or_insert_with(|| ([p, q], Vec::new())).1.push(nrm);
        }
    }
    let cos_max = CREASE_DEG.to_radians().cos();
    for (_, (seg, normals)) in edges {
        let sharp = match normals.as_slice() {
            [a, b] => dot(*a, *b) < cos_max,
            _ => true,
        };
        if sharp {
            m.edges.push(seg);
        }
    }
    m
}

/// A content key for a printed part's display mesh: equal keys mean equal meshes. Ids like
/// `B001` repeat across models, so caches that outlive a model must use this instead.
pub fn brick_key(def: &CustomDef) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (&def.cells, def.plates, &def.studs, &def.units).hash(&mut h);
    format!("printed-brick:{:016x}", h.finish())
}

/// A content key for a detail mesh (see [`brick_key`]).
pub fn detail_key(d: &DetailMesh) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for p in &d.positions {
        for v in p {
            v.to_bits().hash(&mut h);
        }
    }
    d.indices.hash(&mut h);
    format!("printed-detail:{:016x}", h.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sf_core::CustomKind;

    fn l_brick() -> CustomDef {
        CustomDef {
            id: "B001".into(),
            kind: CustomKind::Brick,
            colors: Default::default(),
            trim: None,
            mesh: None,
            cells: vec![(0, 0), (1, 0), (2, 0), (0, 1)],
            plates: 3,
            studs: vec![(0, 0), (1, 0), (2, 0), (0, 1)],
            units: vec![vec![(0, 0), (1, 0), (2, 0)]],
            sockets: vec![(0, 0), (1, 0), (2, 0), (0, 1)],
        }
    }

    #[test]
    fn brick_mesh_fills_its_bbox_frame_and_skips_flat_columns() {
        let m = custom_mesh(&l_brick());
        let (lo, hi) = m.bounds().unwrap();
        // 3 × 2 studs, 3 plates, studs 4 LDU proud.
        assert!((lo[0] + 30.0).abs() < 1e-4 && (hi[0] - 30.0).abs() < 1e-4, "{lo:?} {hi:?}");
        assert!((lo[2] + 20.0).abs() < 1e-4 && (hi[2] - 20.0).abs() < 1e-4);
        assert!((lo[1] + STUD_H).abs() < 1e-4 && (hi[1] - 24.0).abs() < 1e-4);
        // (0, 1) is a stud but no unit: no stud geometry over that cell's centre.
        let over = |x: f32, z: f32| m.positions.iter().any(|p| p[1] < -0.5 && (p[0] - x).abs() < 7.0 && (p[2] - z).abs() < 7.0);
        assert!(over(-20.0, -10.0), "unit stud missing");
        assert!(!over(-20.0, 10.0), "flat column has a stud");
        // Outward normals: the top faces point up (-y).
        let up = m.normals.iter().filter(|n| n[1] < -0.99).count();
        assert!(up > 0);
        assert!(!m.edges.is_empty());
    }

    #[test]
    fn no_inner_walls_between_neighbouring_cells() {
        let m = custom_mesh(&l_brick());
        // No side face at x = -10 inside the run (between cells 0 and 1 of row 0).
        let inner = m.positions.chunks(3).any(|t| {
            t.iter().all(|p| (p[0] + 10.0).abs() < 1e-4 && p[1] >= -1e-4 && p[2] < 0.0)
        });
        assert!(!inner);
    }

    #[test]
    fn detail_mesh_edges_on_creases_only() {
        // A unit cube (two triangles per face, indexed): 12 creased edges, no diagonals.
        let p = |x: f32, y: f32, z: f32| [x * 10.0, y * 10.0, z * 10.0];
        let positions = vec![p(0., 0., 0.), p(1., 0., 0.), p(1., 1., 0.), p(0., 1., 0.), p(0., 0., 1.), p(1., 0., 1.), p(1., 1., 1.), p(0., 1., 1.)];
        let indices = vec![0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 2, 3, 7, 2, 7, 6, 1, 2, 6, 1, 6, 5, 0, 4, 7, 0, 7, 3];
        let m = detail_mesh(&DetailMesh { positions, indices });
        assert_eq!(m.triangle_count(), 12);
        assert_eq!(m.edges.len(), 12);
    }

    #[test]
    fn keys_follow_content() {
        let a = l_brick();
        let mut b = l_brick();
        b.id = "B999".into();
        assert_eq!(brick_key(&a), brick_key(&b));
        b.units.clear();
        assert_ne!(brick_key(&a), brick_key(&b));
    }
}
