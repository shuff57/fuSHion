//! Display meshes for printed parts: merged body bricks ([`custom_mesh`]), curved
//! (trimmed) bricks ([`trimmed_mesh`]) and detail parts Claude designed in SolidCraft
//! ([`detail_mesh`]).
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
/// The same for trimmed bricks' surface meshes: a simplified marching-tetrahedra surface
/// has facets meeting at up to ~60° on its curves, so only sharper corners count as edges.
const TRIM_CREASE_DEG: f32 = 65.0;

type V = [f32; 3];

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

/// The display mesh of a detail part: [`smooth_mesh`] with a 35° crease.
pub fn detail_mesh(d: &DetailMesh) -> PartMesh {
    smooth_mesh(d, CREASE_DEG)
}

/// The display mesh of a trimmed (curved) printed brick from its approximate surface mesh:
/// [`smooth_mesh`] with a 65° crease, so a marching-tetrahedra surface shades as one
/// smooth curve while the brick's own corners stay sharp.
///
/// Only edges beside a flat, axis-aligned face are drawn (the brick's own corners and the
/// line where the curve meets a flat face): creases between two faces of the curve are
/// folds of the approximate surface, not features, and would draw as scratches.
pub fn trimmed_mesh(d: &DetailMesh) -> PartMesh {
    smooth_mesh_with(d, TRIM_CREASE_DEG, true)
}

/// A triangle mesh shaded smoothly: each corner's normal averages the faces around its
/// vertex that meet this face at less than `crease_deg` (area weighted), so curved surfaces
/// look round and sharp edges stay sharp.
///
/// Feature edges (type-2 lines) are kept only where they help read the shape: creases
/// sharper than `crease_deg` and open borders, in connected runs at least
/// [`MIN_FEATURE_LDU`] long. Isolated short creases, which a marching-cubes surface leaves
/// all over its curves, would draw as scratches.
///
/// Vertices within [`WELD_LDU`] of each other are welded first (true distance, not rounding,
/// so float noise on either side of a rounding boundary still welds), so an STL-style mesh
/// (three vertices per triangle) is treated like an indexed one.
pub fn smooth_mesh(d: &DetailMesh, crease_deg: f32) -> PartMesh {
    smooth_mesh_with(d, crease_deg, false)
}

/// [`smooth_mesh`]; with `axis_edges_only`, a feature edge must also have a face whose
/// normal is within ~5° of an axis beside it.
fn smooth_mesh_with(d: &DetailMesh, crease_deg: f32, axis_edges_only: bool) -> PartMesh {
    let n = d.positions.len() as u32;
    let weld = Welder::new(&d.positions);
    // Valid triangles: corners, welded ids, unit normal, area-weighted normal.
    let mut tris: Vec<([V; 3], [u32; 3], V, V)> = Vec::new();
    for t in d.indices.as_chunks::<3>().0 {
        if t.iter().any(|&i| i >= n) {
            continue;
        }
        let ids = t.map(|i| weld.id[i as usize]);
        if ids[0] == ids[1] || ids[1] == ids[2] || ids[0] == ids[2] {
            continue;
        }
        let [a, b, c] = [d.positions[t[0] as usize], d.positions[t[1] as usize], d.positions[t[2] as usize]];
        let w = cross(sub(b, a), sub(c, a));
        let len = dot(w, w).sqrt();
        if len < 1e-9 || !len.is_finite() {
            continue;
        }
        tris.push(([a, b, c], ids, [w[0] / len, w[1] / len, w[2] / len], w));
    }
    // Welded vertex → the triangles around it; welded edge → its endpoints and face normals.
    let mut around: HashMap<u32, Vec<usize>> = HashMap::new();
    let mut edges: HashMap<(u32, u32), ([V; 2], Vec<V>)> = HashMap::new();
    for (i, (v, ids, nrm, _)) in tris.iter().enumerate() {
        for k in 0..3 {
            around.entry(ids[k]).or_default().push(i);
            let (p, q) = (ids[k], ids[(k + 1) % 3]);
            let e = (p.min(q), p.max(q));
            edges.entry(e).or_insert_with(|| ([v[k], v[(k + 1) % 3]], Vec::new())).1.push(*nrm);
        }
    }
    let cos_max = crease_deg.to_radians().cos();
    let mut m = PartMesh::default();
    for (v, ids, nrm, _) in &tris {
        let base = m.positions.len() as u32;
        for k in 0..3 {
            let mut sum = [0.0f32; 3];
            for &j in &around[&ids[k]] {
                let (_, _, nj, wj) = &tris[j];
                if dot(*nj, *nrm) >= cos_max {
                    sum = [sum[0] + wj[0], sum[1] + wj[1], sum[2] + wj[2]];
                }
            }
            let len = dot(sum, sum).sqrt();
            let smooth = if len > 1e-12 { [sum[0] / len, sum[1] / len, sum[2] / len] } else { *nrm };
            m.positions.push(v[k]);
            m.normals.push(smooth);
            m.colors.push(MAIN_COLOR);
        }
        m.indices.extend_from_slice(&[base, base + 1, base + 2]);
    }
    // Candidate feature edges: creases sharper than the threshold (between every pair of
    // faces on a non-manifold edge), and open borders.
    let feature: Vec<((u32, u32), [V; 2])> = edges
        .into_iter()
        .filter(|(_, (_, normals))| {
            let axis = |n: &V| n.iter().any(|c| c.abs() > AXIS_COS);
            if axis_edges_only && !normals.iter().any(axis) {
                return false;
            }
            match normals.as_slice() {
                [_] => true,
                ns => ns.iter().enumerate().any(|(i, a)| ns[i + 1..].iter().any(|b| dot(*a, *b) < cos_max)),
            }
        })
        .map(|(k, (seg, _))| (k, seg))
        .collect();
    // Keep runs (connected through shared welded vertices) at least MIN_FEATURE_LDU long.
    let mut parent: HashMap<u32, u32> = HashMap::new();
    fn find(p: &mut HashMap<u32, u32>, x: u32) -> u32 {
        let mut r = x;
        while let Some(&q) = p.get(&r) {
            if q == r {
                break;
            }
            r = q;
        }
        let mut y = x;
        while let Some(&q) = p.get(&y) {
            if q == r {
                break;
            }
            p.insert(y, r);
            y = q;
        }
        r
    }
    for ((a, b), _) in &feature {
        parent.entry(*a).or_insert(*a);
        parent.entry(*b).or_insert(*b);
        let (ra, rb) = (find(&mut parent, *a), find(&mut parent, *b));
        if ra != rb {
            parent.insert(ra, rb);
        }
    }
    let mut run_len: HashMap<u32, f32> = HashMap::new();
    for ((a, _), seg) in &feature {
        let r = find(&mut parent, *a);
        let d = sub(seg[0], seg[1]);
        *run_len.entry(r).or_default() += dot(d, d).sqrt();
    }
    for ((a, _), seg) in feature {
        if run_len[&find(&mut parent, a)] >= MIN_FEATURE_LDU {
            m.edges.push(seg);
        }
    }
    m
}

/// A normal is axis-aligned when one component exceeds this (about 5°).
const AXIS_COS: f32 = 0.996;
/// Vertices closer than this are one vertex, LDU.
const WELD_LDU: f32 = 0.01;
/// Feature-edge runs shorter than this (in total) are not drawn, LDU.
pub const MIN_FEATURE_LDU: f32 = 4.0;

/// Welds positions within [`WELD_LDU`]: `id[i]` is the welded vertex of position `i`.
struct Welder {
    id: Vec<u32>,
}

impl Welder {
    fn new(ps: &[V]) -> Self {
        let cell = |p: V| p.map(|v| (v / WELD_LDU).floor() as i64);
        let mut grid: HashMap<[i64; 3], Vec<u32>> = HashMap::new();
        let mut reps: Vec<V> = Vec::new();
        let mut id = Vec::with_capacity(ps.len());
        for &p in ps {
            let c = cell(p);
            let mut found = None;
            'search: for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        if let Some(list) = grid.get(&[c[0] + dx, c[1] + dy, c[2] + dz]) {
                            for &r in list {
                                let d = sub(reps[r as usize], p);
                                if dot(d, d) <= WELD_LDU * WELD_LDU {
                                    found = Some(r);
                                    break 'search;
                                }
                            }
                        }
                    }
                }
            }
            let r = found.unwrap_or_else(|| {
                let r = reps.len() as u32;
                reps.push(p);
                grid.entry(c).or_default().push(r);
                r
            });
            id.push(r);
        }
        Self { id }
    }
}

/// A content key for a printed part's display mesh: equal keys mean equal meshes. Ids like
/// `B001` repeat across models, so caches that outlive a model must use this instead.
pub fn brick_key(def: &CustomDef) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (&def.cells, def.plates, &def.studs, &def.units).hash(&mut h);
    format!("printed-brick:{:016x}", h.finish())
}

/// A content key for a trimmed brick's display mesh (see [`brick_key`]); differs from
/// [`detail_key`] of the same mesh, since the two are shaded differently.
pub fn trimmed_key(d: &DetailMesh) -> String {
    format!("printed-trimmed:{}", &detail_key(d)["printed-detail:".len()..])
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
    /// A trimmed 2 × 2 brick, 3 plates: vertical sides from the bottom (y = 24) up to a domed
    /// top that rises 8 LDU at the centre and meets the top plane (y = 0) at the rim. Indexed,
    /// outward faces, in the catalog frame (top centre, -y up).
    fn dome_mesh(n: usize) -> sf_core::DetailMesh {
        let (h, bottom) = (8.0f32, 24.0f32);
        let at = |i: usize| -20.0 + 40.0 * i as f32 / n as f32;
        let top = |x: f32, z: f32| -h * (1.0 - (x / 20.0).powi(2)) * (1.0 - (z / 20.0).powi(2));
        let mut positions = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        // Top grid.
        for j in 0..=n {
            for i in 0..=n {
                positions.push([at(i), top(at(i), at(j)), at(j)]);
            }
        }
        let id = |i: usize, j: usize| (j * (n + 1) + i) as u32;
        for j in 0..n {
            for i in 0..n {
                // Outward = -y (up): wind so (b-a)×(c-a) points to -y.
                indices.extend([id(i, j), id(i, j + 1), id(i + 1, j), id(i + 1, j), id(i, j + 1), id(i + 1, j + 1)]);
            }
        }
        // Sides: a strip along each edge of the grid down to the bottom.
        let side = |rim: Vec<[f32; 3]>, out: [f32; 3], positions: &mut Vec<[f32; 3]>, indices: &mut Vec<u32>| {
            for w in rim.windows(2) {
                let (a, b) = (w[0], w[1]);
                let (ad, bd) = ([a[0], bottom, a[2]], [b[0], bottom, b[2]]);
                let base = positions.len() as u32;
                positions.extend([a, b, bd, ad]);
                let nrm = {
                    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                    let v = [bd[0] - a[0], bd[1] - a[1], bd[2] - a[2]];
                    [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]]
                };
                let flip = nrm[0] * out[0] + nrm[1] * out[1] + nrm[2] * out[2] < 0.0;
                if flip {
                    indices.extend([base, base + 2, base + 1, base, base + 3, base + 2]);
                } else {
                    indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
                }
            }
        };
        let row = |pick: &dyn Fn(usize) -> (usize, usize), positions: &Vec<[f32; 3]>| -> Vec<[f32; 3]> {
            (0..=n).map(|k| { let (i, j) = pick(k); positions[id(i, j) as usize] }).collect()
        };
        let rims = [
            (row(&|k| (k, 0), &positions), [0.0, 0.0, -1.0]),
            (row(&|k| (k, n), &positions), [0.0, 0.0, 1.0]),
            (row(&|k| (0, k), &positions), [-1.0, 0.0, 0.0]),
            (row(&|k| (n, k), &positions), [1.0, 0.0, 0.0]),
        ];
        for (rim, out) in rims {
            side(rim, out, &mut positions, &mut indices);
        }
        // Bottom.
        let base = positions.len() as u32;
        positions.extend([[-20.0, bottom, -20.0], [20.0, bottom, -20.0], [20.0, bottom, 20.0], [-20.0, bottom, 20.0]]);
        indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        sf_core::DetailMesh { positions, indices }
    }

    #[test]
    fn trimmed_meshes_shade_smoothly_with_edges_only_at_the_rim() {
        let d = dome_mesh(16);
        let m = trimmed_mesh(&d);
        assert_eq!(m.triangle_count(), d.indices.len() / 3);
        // No feature edge crosses the dome's interior: every edge has an end on the rim or
        // below it.
        let on_rim = |p: &[f32; 3]| p[0].abs() > 19.9 || p[2].abs() > 19.9 || p[1] > 23.9;
        assert!(!m.edges.is_empty());
        for e in &m.edges {
            assert!(on_rim(&e[0]) && on_rim(&e[1]), "edge inside the dome: {e:?}");
        }
        // Corner normals on the dome vary smoothly: neighbouring triangles' corners at a shared
        // interior vertex get the same normal.
        let mut by_pos: HashMap<(i64, i64, i64), Vec<[f32; 3]>> = HashMap::new();
        for (p, n) in m.positions.iter().zip(&m.normals) {
            if !on_rim(p) {
                by_pos.entry(((p[0] * 100.0) as i64, (p[1] * 100.0) as i64, (p[2] * 100.0) as i64)).or_default().push(*n);
            }
        }
        assert!(by_pos.len() > 100);
        for ns in by_pos.values() {
            for n in ns {
                assert!(dot(*n, ns[0]) > 0.9999, "{ns:?}");
            }
        }
        // The rim stays sharp: a side face's corners keep the side's normal.
        assert!(m.normals.iter().any(|n| (n[0].abs() - 1.0).abs() < 1e-4));
        assert_ne!(trimmed_key(&d), detail_key(&d));
    }
}
