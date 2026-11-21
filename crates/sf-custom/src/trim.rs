//! Curved printed bricks: a [`CustomShape::Printed`] with a [`Trim`] keeps exactly
//! `block ∩ (S ∪ P)`, where `S` is the model's solid (the trim's shapes, applied in order)
//! and `P` the protect boxes.
//!
//! # How it is built
//! 1. The plain brick, every stage checked against its closed form as always, with studs
//!    only on cells whose top survives the trim (fused units whole).
//! 2. **Fill**: wherever the hollow would come within a wall's thickness of the trimmed
//!    surface, it is filled solid: the hollow's prism less `offset(S, −wall) ∪ P` is joined
//!    on, so no cut ever breaks through into the hollow.
//! 3. **Trim**: the brick is intersected with `S ∪ P` (plus a slab over the kept studs).
//!
//! # How each primitive is made in SolidCraft
//! Each is made as its own body at the origin, turned and moved into place with
//! `solid.move` (a rigid 3×4 matrix), and combined into the trim body in order (`join` for
//! Add, `cut` for Subtract):
//! * ellipsoid: `solid.sphere` of radius 1, then `solid.scale` by its radii (a sphere of the
//!   right radius when they are equal);
//! * box: `solid.box` centred;
//! * elliptic cylinder and cone: `solid.cylinder` / `solid.cone` of radius 1, scaled in x, y;
//! * capsule: a cylinder between the ends and a sphere at each (2 µm bigger, so their
//!   surfaces are never tangent);
//! * prism: its polygon sketched and extruded.
//!
//! Every shape is nudged a few µm (a different amount each), so no face of the trim lies
//! exactly on a face of the brick or of another shape, where SolidCraft's booleans can fail.
//!
//! # Checks
//! No closed form survives a trim, so the result must be one closed body with a watertight
//! mesh inside the block's box, and its volume must match, within [`TRIM_VOLUME_TOL`], an
//! independent estimate: vertical rays cast through the (checked) untrimmed mesh, sampled
//! every [`SAMPLE_STEP`] mm and kept where the trim keeps them.

use crate::detail::{closed, cmd, p3, r6, sketch_xy};
use crate::mesh::analyze_stl;
use crate::plan::plan_with;
use crate::run::{finish, measure, run_stages, run_step};
use crate::shape::{Footprint, P2};
use crate::{CustomError, CustomPart, CustomShape, Result, StudProfile};
use serde_json::{Value, json};
use sf_core::{Trim, TrimBox, TrimKind, TrimOp, TrimShape};
use solidcraft_engine::Session;
use std::collections::BTreeSet;

/// How far a trimmed brick's volume may stray from the sampled estimate.
pub const TRIM_VOLUME_TOL: f64 = 0.06;
/// Grid step of the sampled estimate, mm.
pub const SAMPLE_STEP: f64 = 0.2;

/// A trimmed brick and what its checks found.
#[derive(Clone, Debug)]
pub struct Trimmed {
    /// The trimmed print.
    pub part: CustomPart,
    /// The plain brick it was cut from (studs only where kept).
    pub untrimmed: CustomPart,
    /// The independent volume estimate, mm³.
    pub sampled_volume: f64,
    /// Downward-facing area steeper than 45° (see [`brick_overhang`]), mm².
    pub overhang_mm2: f64,
    /// Cells whose studs were left off (their top is trimmed away).
    pub dropped_studs: Vec<(i32, i32)>,
    /// Whether the hollow had to be partly filled to keep the walls whole.
    pub filled: bool,
    /// Every SolidCraft command run, in order (replay them to export a STEP).
    pub commands: Vec<Value>,
}

// ---------------------------------------------------------------- point tests

/// `p` in a shape's local frame.
fn to_local(s: &TrimShape, p: [f64; 3]) -> [f64; 3] {
    let d = [p[0] - s.translation[0], p[1] - s.translation[1], p[2] - s.translation[2]];
    let r = &s.rotation;
    // Rᵀ·d (R maps local → brick).
    [
        r[0][0] * d[0] + r[1][0] * d[1] + r[2][0] * d[2],
        r[0][1] * d[0] + r[1][1] * d[1] + r[2][1] * d[2],
        r[0][2] * d[0] + r[1][2] * d[1] + r[2][2] * d[2],
    ]
}

fn in_polygon(pts: &[[f64; 2]], q: [f64; 2]) -> bool {
    let mut c = false;
    let n = pts.len();
    for k in 0..n {
        let (a, b) = (pts[k], pts[(k + 1) % n]);
        if (a[1] > q[1]) != (b[1] > q[1]) {
            let x = a[0] + (q[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
            if x > q[0] {
                c = !c;
            }
        }
    }
    c
}

/// Whether the local point `q` is inside `k`.
pub fn inside_kind(k: &TrimKind, q: [f64; 3]) -> bool {
    match k {
        TrimKind::Ellipsoid { radii } => {
            (q[0] / radii[0]).powi(2) + (q[1] / radii[1]).powi(2) + (q[2] / radii[2]).powi(2) <= 1.0
        }
        TrimKind::Box { size } => (0..3).all(|i| q[i].abs() <= size[i] / 2.0),
        TrimKind::Cylinder { radii, height } => {
            q[2].abs() <= height / 2.0 && (q[0] / radii[0]).powi(2) + (q[1] / radii[1]).powi(2) <= 1.0
        }
        TrimKind::Cone { radii, top_scale, height } => {
            if q[2].abs() > height / 2.0 {
                return false;
            }
            let f = (q[2] + height / 2.0) / height;
            let s = 1.0 + (top_scale - 1.0) * f;
            s > 0.0 && (q[0] / (radii[0] * s)).powi(2) + (q[1] / (radii[1] * s)).powi(2) <= 1.0
        }
        TrimKind::Capsule { a, b, radius } => {
            let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let aq = [q[0] - a[0], q[1] - a[1], q[2] - a[2]];
            let l2 = ab[0] * ab[0] + ab[1] * ab[1] + ab[2] * ab[2];
            let t = if l2 > 0.0 { ((aq[0] * ab[0] + aq[1] * ab[1] + aq[2] * ab[2]) / l2).clamp(0.0, 1.0) } else { 0.0 };
            let c = [a[0] + t * ab[0] - q[0], a[1] + t * ab[1] - q[1], a[2] + t * ab[2] - q[2]];
            c[0] * c[0] + c[1] * c[1] + c[2] * c[2] <= radius * radius
        }
        TrimKind::Prism { points, height } => q[2].abs() <= height / 2.0 && in_polygon(points, [q[0], q[1]]),
    }
}

/// Whether `p` (brick frame) is in the trim's solid `S`.
pub fn inside_s(shapes: &[TrimShape], p: [f64; 3]) -> bool {
    let mut inside = false;
    for s in shapes {
        let hit = inside_kind(&s.kind, to_local(s, p));
        match s.op {
            TrimOp::Add => inside |= hit,
            TrimOp::Subtract => inside &= !hit,
        }
    }
    inside
}

fn in_box(b: &TrimBox, p: [f64; 3]) -> bool {
    (0..3).all(|i| p[i] >= b.min[i] && p[i] <= b.max[i])
}

/// Whether `p` is kept: in `S ∪ P`.
pub fn inside_trim(t: &Trim, p: [f64; 3]) -> bool {
    t.protect.iter().any(|b| in_box(b, p)) || inside_s(&t.shapes, p)
}

/// The shape grown by `d` (shrunk for negative `d`), approximately for cones and prisms
/// (always on the safe side for shrinking: never bigger than the true offset); `None` if
/// it shrinks away.
pub fn offset_kind(k: &TrimKind, d: f64) -> Option<TrimKind> {
    let pos = |v: f64| (v > 0.05).then_some(v);
    Some(match k {
        TrimKind::Ellipsoid { radii } => TrimKind::Ellipsoid { radii: [pos(radii[0] + d)?, pos(radii[1] + d)?, pos(radii[2] + d)?] },
        TrimKind::Box { size } => TrimKind::Box { size: [pos(size[0] + 2.0 * d)?, pos(size[1] + 2.0 * d)?, pos(size[2] + 2.0 * d)?] },
        TrimKind::Cylinder { radii, height } => TrimKind::Cylinder { radii: [pos(radii[0] + d)?, pos(radii[1] + d)?], height: pos(height + 2.0 * d)? },
        TrimKind::Cone { radii, top_scale, height } => {
            // The side's slope stretches a normal offset `d` to `d·√(1 + k²)` across.
            let k = radii[0].max(radii[1]) * (1.0 - top_scale).abs() / height;
            let dd = d * (1.0 + k * k).sqrt();
            let r = [pos(radii[0] + dd)?, pos(radii[1] + dd)?];
            let top = (radii[0] * top_scale + dd).max(0.0) / r[0];
            TrimKind::Cone { radii: r, top_scale: top, height: pos(height + 2.0 * d)? }
        }
        TrimKind::Capsule { a, b, radius } => TrimKind::Capsule { a: *a, b: *b, radius: pos(radius + d)? },
        TrimKind::Prism { points, height } => {
            // Each vertex moved along its corner's bisector so both edges move by `d`.
            let n = points.len();
            let area: f64 = (0..n).map(|i| points[i][0] * points[(i + 1) % n][1] - points[(i + 1) % n][0] * points[i][1]).sum::<f64>() / 2.0;
            let out = if area > 0.0 { 1.0 } else { -1.0 };
            let mut pts = Vec::new();
            for i in 0..n {
                let (a, b, c) = (points[(i + n - 1) % n], points[i], points[(i + 1) % n]);
                let nrm = |p: [f64; 2], q: [f64; 2]| {
                    let (dx, dy) = (q[0] - p[0], q[1] - p[1]);
                    let l = dx.hypot(dy).max(1e-12);
                    [dy / l * out, -dx / l * out]
                };
                let (n1, n2) = (nrm(a, b), nrm(b, c));
                let m = [n1[0] + n2[0], n1[1] + n2[1]];
                let dot = 1.0 + n1[0] * n2[0] + n1[1] * n2[1];
                let f = if dot > 1e-6 { d / dot } else { d };
                pts.push([b[0] + m[0] * f, b[1] + m[1] * f]);
            }
            TrimKind::Prism { points: pts, height: pos(height + 2.0 * d)? }
        }
    })
}

/// `offset(S, −w)` as far as it matters inside the block `(lo, hi)`: every Add shape shrunk
/// by `w`, every Subtract shape grown by `w`, except that a flat face lying wholly outside
/// the block (a box's or a cap's) stays put: it is no surface of the kept solid, and moving
/// it would close the hollow's open bottom or sides.
fn offset_in(shapes: &[TrimShape], w: f64, block: ([f64; 3], [f64; 3])) -> Vec<TrimShape> {
    shapes
        .iter()
        .filter_map(|s| {
            let d = match s.op {
                TrimOp::Add => -w,
                TrimOp::Subtract => w,
            };
            // The block's corners in the shape's frame.
            let corners: Vec<[f64; 3]> = (0..8)
                .map(|k| {
                    to_local(s, [
                        if k & 1 == 0 { block.0[0] } else { block.1[0] },
                        if k & 2 == 0 { block.0[1] } else { block.1[1] },
                        if k & 4 == 0 { block.0[2] } else { block.1[2] },
                    ])
                })
                .collect();
            let (cmin, cmax) = (
                [0, 1, 2].map(|i| corners.iter().map(|c| c[i]).fold(f64::MAX, f64::min)),
                [0, 1, 2].map(|i| corners.iter().map(|c| c[i]).fold(f64::MIN, f64::max)),
            );
            // Per axis: how far each face moves (− side, + side), for the axes with flat faces.
            let flat: &[usize] = match &s.kind {
                TrimKind::Box { .. } => &[0, 1, 2],
                TrimKind::Cylinder { .. } | TrimKind::Cone { .. } | TrimKind::Prism { .. } => &[2],
                _ => &[],
            };
            let half = |i: usize| -> f64 {
                match &s.kind {
                    TrimKind::Box { size } => size[i] / 2.0,
                    TrimKind::Cylinder { height, .. } | TrimKind::Cone { height, .. } | TrimKind::Prism { height, .. } => height / 2.0,
                    _ => 0.0,
                }
            };
            let mut shift = [0.0; 3];
            let mut grow = [0.0; 3];
            for &i in flat {
                // A face outside the block (every corner on the shape's side of it) keeps put:
                // undo the offset on that side.
                let plus_out = cmax[i] < half(i);
                let minus_out = cmin[i] > -half(i);
                let (dm, dp) = (if minus_out { -d } else { 0.0 }, if plus_out { -d } else { 0.0 });
                grow[i] = dm + dp;
                shift[i] = (dp - dm) / 2.0;
            }
            let kind = offset_kind(&s.kind, d)?;
            let kind = match kind {
                TrimKind::Box { size } => TrimKind::Box { size: [size[0] + grow[0], size[1] + grow[1], size[2] + grow[2]] },
                TrimKind::Cylinder { radii, height } => TrimKind::Cylinder { radii, height: height + grow[2] },
                TrimKind::Cone { radii, top_scale, height } => TrimKind::Cone { radii, top_scale, height: height + grow[2] },
                TrimKind::Prism { points, height } => TrimKind::Prism { points, height: height + grow[2] },
                k => k,
            };
            let r = &s.rotation;
            let t = [0, 1, 2].map(|i| s.translation[i] + r[i][0] * shift[0] + r[i][1] * shift[1] + r[i][2] * shift[2]);
            Some(TrimShape { kind, translation: t, ..s.clone() })
        })
        .collect()
}

// ---------------------------------------------------------------- SolidCraft bodies

/// Commands making body `name` from the shapes (in order) and boxes (unioned after), each
/// nudged by a few µm. Returns `None` if nothing is added.
fn body_cmds(name: &str, shapes: &[TrimShape], boxes: &[TrimBox], jitter: f64) -> Option<Vec<Value>> {
    let mut c = Vec::new();
    let mut have = false;
    for (k, s) in shapes.iter().enumerate() {
        if s.op == TrimOp::Subtract && !have {
            continue;
        }
        let nudge = 1.3e-3 * jitter * (k as f64 + 1.0);
        let piece = if have { format!("{name}_{k}") } else { name.to_string() };
        c.extend(primitive(&piece, &s.kind));
        let r = &s.rotation;
        let t = [s.translation[0] + nudge, s.translation[1] + 0.7 * nudge, s.translation[2] + 0.4 * nudge];
        let m: Vec<Value> = (0..3).map(|i| json!([r[i][0], r[i][1], r[i][2], r6(t[i])])).collect();
        c.push(cmd("solid.move", json!({"bodies": [piece], "matrix": m})));
        if have {
            let op = if s.op == TrimOp::Add { "join" } else { "cut" };
            c.push(cmd("solid.combine", json!({"target": name, "tools": [piece], "operation": op, "touching": "one_body"})));
        }
        have = true;
    }
    for (k, b) in boxes.iter().enumerate() {
        let piece = if have { format!("{name}_p{k}") } else { name.to_string() };
        let e = 0.9e-3 * jitter * (k as f64 + 1.0);
        let size = [b.max[0] - b.min[0] + e, b.max[1] - b.min[1] + e, b.max[2] - b.min[2] + e];
        c.push(cmd(
            "solid.box",
            json!({"length": r6(size[0]), "width": r6(size[1]), "height": r6(size[2]), "corner": p3([b.min[0] - e / 2.0, b.min[1] - e / 2.0, b.min[2] - e / 2.0]), "operation": "new", "body_name": piece, "name": format!("{piece}Box")}),
        ));
        if have {
            c.push(cmd("solid.combine", json!({"target": name, "tools": [piece], "operation": "join", "touching": "one_body"})));
        }
        have = true;
    }
    have.then_some(c)
}

/// One primitive at the origin as the new body `name`.
fn primitive(name: &str, k: &TrimKind) -> Vec<Value> {
    let new = |mut p: Value| {
        p["operation"] = json!("new");
        p["body_name"] = json!(name);
        p["name"] = json!(format!("{name}Prim"));
        p
    };
    let scale = |f: [f64; 3]| cmd("solid.scale", json!({"bodies": [name], "factors": f.map(r6), "origin": [0, 0, 0]}));
    let equal = |a: f64, b: f64| (a - b).abs() < 1e-9;
    match k {
        TrimKind::Ellipsoid { radii } => {
            if equal(radii[0], radii[1]) && equal(radii[1], radii[2]) {
                vec![cmd("solid.sphere", new(json!({"radius": r6(radii[0]), "center": [0, 0, 0]})))]
            } else {
                vec![cmd("solid.sphere", new(json!({"radius": 1.0, "center": [0, 0, 0]}))), scale(*radii)]
            }
        }
        TrimKind::Box { size } => vec![cmd(
            "solid.box",
            new(json!({"length": r6(size[0]), "width": r6(size[1]), "height": r6(size[2]), "center": [0, 0, 0]})),
        )],
        TrimKind::Cylinder { radii, height } => {
            let mut c = vec![cmd("solid.cylinder", new(json!({"radius": 1.0, "height": r6(*height), "base": [0, 0, r6(-height / 2.0)]})))];
            c.push(scale([radii[0], radii[1], 1.0]));
            c
        }
        TrimKind::Cone { radii, top_scale, height } => {
            let mut p = json!({"radius": 1.0, "height": r6(*height), "base": [0, 0, r6(-height / 2.0)]});
            if *top_scale > 1e-6 {
                p["top_radius"] = json!(r6(*top_scale));
            }
            vec![cmd("solid.cone", new(p)), scale([radii[0], radii[1], 1.0])]
        }
        TrimKind::Capsule { a, b, radius } => {
            let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let l = (ab[0] * ab[0] + ab[1] * ab[1] + ab[2] * ab[2]).sqrt();
            if l < 1e-6 {
                return vec![cmd("solid.sphere", new(json!({"radius": r6(*radius), "center": p3(*a)})))];
            }
            let axis = ab.map(|v| v / l);
            let ball = |c: [f64; 3], i: usize| {
                cmd(
                    "solid.sphere",
                    json!({"radius": r6(radius + 0.002), "center": p3(c), "operation": "join", "targets": [name], "name": format!("{name}End{i}")}),
                )
            };
            vec![
                cmd("solid.cylinder", new(json!({"radius": r6(*radius), "height": r6(l), "base": p3(*a), "axis": p3(axis)}))),
                ball(*a, 0),
                ball(*b, 1),
            ]
        }
        TrimKind::Prism { points, height } => {
            let sk = format!("{name}Sketch");
            let pts: Vec<P2> = points.iter().map(|p| [p[0], p[1]]).collect();
            vec![
                sketch_xy(&sk, -height / 2.0),
                closed(&pts),
                cmd("sketch.finish", json!({})),
                cmd("solid.extrude", new(json!({"sketch": sk, "distance": r6(*height)}))),
            ]
        }
    }
}

// ---------------------------------------------------------------- sampling

/// Vertical rays through a triangle mesh: for each (x, y) on a grid, the z intervals inside.
struct Rays {
    lo: [f64; 2],
    nx: usize,
    ny: usize,
    /// Sorted crossing heights per ray.
    hits: Vec<Vec<f64>>,
}

fn triangles(stl: &[u8]) -> Vec<[[f64; 3]; 3]> {
    if stl.len() < 84 {
        return Vec::new();
    }
    let n = u32::from_le_bytes([stl[80], stl[81], stl[82], stl[83]]) as usize;
    let f = |o: usize| f64::from(f32::from_le_bytes([stl[o], stl[o + 1], stl[o + 2], stl[o + 3]]));
    (0..n)
        .map(|t| {
            let b = 84 + t * 50 + 12;
            [0, 1, 2].map(|k| [f(b + k * 12), f(b + k * 12 + 4), f(b + k * 12 + 8)])
        })
        .collect()
}

impl Rays {
    fn cast(stl: &[u8], lo: [f64; 2], hi: [f64; 2], step: f64) -> Self {
        let nx = ((hi[0] - lo[0]) / step).ceil().max(1.0) as usize;
        let ny = ((hi[1] - lo[1]) / step).ceil().max(1.0) as usize;
        let mut hits = vec![Vec::new(); nx * ny];
        // Ray (i, j) at the cell's middle, nudged off grid lines.
        let ray = |i: usize, j: usize| [lo[0] + (i as f64 + 0.5) * step + 1.7e-7, lo[1] + (j as f64 + 0.5) * step + 2.3e-7];
        for t in triangles(stl) {
            let (mut a, mut b) = ([f64::MAX; 2], [f64::MIN; 2]);
            for v in &t {
                for k in 0..2 {
                    a[k] = a[k].min(v[k]);
                    b[k] = b[k].max(v[k]);
                }
            }
            let i0 = (((a[0] - lo[0]) / step - 0.5).floor().max(0.0)) as usize;
            let i1 = ((((b[0] - lo[0]) / step - 0.5).ceil()).max(0.0) as usize).min(nx - 1);
            let j0 = (((a[1] - lo[1]) / step - 0.5).floor().max(0.0)) as usize;
            let j1 = ((((b[1] - lo[1]) / step - 0.5).ceil()).max(0.0) as usize).min(ny - 1);
            for i in i0..=i1.max(i0) {
                for j in j0..=j1.max(j0) {
                    if i >= nx || j >= ny {
                        continue;
                    }
                    let q = ray(i, j);
                    // Barycentric test in the plane, then the height there.
                    let (p0, p1, p2) = (t[0], t[1], t[2]);
                    let d = (p1[1] - p2[1]) * (p0[0] - p2[0]) + (p2[0] - p1[0]) * (p0[1] - p2[1]);
                    if d.abs() < 1e-15 {
                        continue;
                    }
                    let l0 = ((p1[1] - p2[1]) * (q[0] - p2[0]) + (p2[0] - p1[0]) * (q[1] - p2[1])) / d;
                    let l1 = ((p2[1] - p0[1]) * (q[0] - p2[0]) + (p0[0] - p2[0]) * (q[1] - p2[1])) / d;
                    let l2 = 1.0 - l0 - l1;
                    if l0 >= 0.0 && l1 >= 0.0 && l2 >= 0.0 {
                        hits[j * nx + i].push(l0 * p0[2] + l1 * p1[2] + l2 * p2[2]);
                    }
                }
            }
        }
        for h in &mut hits {
            h.sort_by(f64::total_cmp);
        }
        Self { lo, nx, ny, hits }
    }

    fn inside(&self, i: usize, j: usize, z: f64) -> bool {
        self.hits[j * self.nx + i].iter().filter(|&&h| h < z).count() % 2 == 1
    }

}

/// Inside the solid a binary STL encloses, at `p`: a ray up from `p` crossing it an odd
/// number of times. (For tests and checks; slow for many points, see the sampler.)
pub fn inside_mesh(stl: &[u8], p: [f64; 3]) -> bool {
    let r = Rays::cast(stl, [p[0] - 0.05, p[1] - 0.05], [p[0] + 0.05, p[1] + 0.05], 0.1);
    r.inside(0, 0, p[2])
}

/// Downward-facing area of a brick's STL steeper than 45° from level and not level
/// (level downward faces are bridges: the hollow's ceiling, slot and label roofs), above
/// the bottom 0.3 mm, mm².
pub fn brick_overhang(stl: &[u8]) -> f64 {
    let mut area = 0.0;
    for t in triangles(stl) {
        if t.iter().any(|v| v[2] <= 0.3) {
            continue;
        }
        let e1 = [t[1][0] - t[0][0], t[1][1] - t[0][1], t[1][2] - t[0][2]];
        let e2 = [t[2][0] - t[0][0], t[2][1] - t[0][1], t[2][2] - t[0][2]];
        let c = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
        let l = (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt();
        if l > 0.0 {
            let nz = c[2] / l;
            if nz < -std::f64::consts::FRAC_1_SQRT_2 && nz > -0.999 {
                area += l / 2.0;
            }
        }
    }
    area
}

// ---------------------------------------------------------------- the build

/// Names of the bodies whose names start with `prefix` (a split body's pieces).
fn bodies_named(s: &mut Session, prefix: &str) -> Result<Vec<String>> {
    let r = s.execute("inspect.measure", &json!({})).map_err(|e| CustomError::Mismatch {
        stage: "Trim".into(),
        details: format!("measure: {e}"),
    })?;
    Ok(r["bodies"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|b| b["name"].as_str().map(str::to_string))
        .filter(|n| n == prefix || n.starts_with(&format!("{prefix} ")) || n.starts_with(&format!("{prefix}(")))
        .collect())
}

/// Builds a [`CustomShape::Printed`] with a trim (see the module docs). A boolean that
/// SolidCraft fails is retried with the shapes nudged differently (three tries).
pub fn build_trimmed(shape: &CustomShape, p: &StudProfile) -> Result<Trimmed> {
    let mut last = None;
    for jitter in [1.0, 3.7, 11.3] {
        match build_trimmed_once(shape, p, jitter) {
            Err(e @ CustomError::Engine { .. }) => last = Some(e),
            other => return other,
        }
    }
    Err(last.expect("tried at least once"))
}

fn build_trimmed_once(shape: &CustomShape, p: &StudProfile, jitter: f64) -> Result<Trimmed> {
    let CustomShape::Printed { cells, plates, units, label, trim: Some(trim) } = shape else {
        return Err(CustomError::InvalidShape("build_trimmed takes a printed shape with a trim".into()));
    };
    let bad = |why: String| CustomError::Mismatch { stage: "Trim".into(), details: why };
    // Cells moved to start at 0 (the trim's frame).
    let i0 = cells.iter().map(|c| c.0).min().unwrap_or(0);
    let j0 = cells.iter().map(|c| c.1).min().unwrap_or(0);
    let norm: Vec<(i32, i32)> = cells.iter().map(|c| (c.0 - i0, c.1 - j0)).collect();
    let units: Vec<Vec<(i32, i32)>> = units.iter().map(|u| u.iter().map(|c| (c.0 - i0, c.1 - j0)).collect()).collect();
    let h = p.body_height(*plates);
    // Studs stay where the top under them is kept: every sample of the cell's square 0.3 mm
    // under the top is inside the trim.
    let mut keep = BTreeSet::new();
    let mut dropped = Vec::new();
    for &(i, j) in &norm {
        let (x0, y0) = (f64::from(i) * p.pitch, f64::from(j) * p.pitch);
        let ok = (0..7).all(|a| {
            (0..7).all(|b| {
                let q = [x0 + 1.0 + 6.0 * f64::from(a) / 6.0, y0 + 1.0 + 6.0 * f64::from(b) / 6.0, h - 0.3];
                inside_trim(trim, q)
            })
        });
        if ok {
            keep.insert((i, j));
        } else {
            dropped.push((i, j));
        }
    }
    let plain = CustomShape::Printed { cells: norm.clone(), plates: *plates, units, label: label.clone(), trim: None };
    let plan = plan_with(&plain, p, Some(&keep))?;
    // 1. The plain brick, checked stage by stage.
    let mut s = Session::default();
    run_stages(&mut s, &plan)?;
    let last = plan.stages.last().map_or("(empty)", |st| st.name.as_str());
    let untrimmed = finish(&mut s, last, Some(plan.volume_mm3), Some(plan.bbox_mm))?;
    let m = measure(&mut s, "Trim")?;
    let part_name = {
        let r = s
            .execute("inspect.measure", &json!({}))
            .map_err(|e| bad(format!("measure: {e}")))?;
        r["bodies"][0]["name"].as_str().unwrap_or("Body1").to_string()
    };
    let _ = m;
    let mut commands: Vec<Value> = plan.stages.iter().flat_map(|st| st.commands.clone()).collect();
    let mut index = 0usize;
    let mut run = |s: &mut Session, c: &Value| -> Result<()> {
        run_step(s, "Trim", index, c)?;
        commands.push(c.clone());
        index += 1;
        Ok(())
    };
    // 2. Fill: the hollow's prism less offset(S, −wall) ∪ P.
    let f = Footprint::new(&norm)?;
    let hollow_h = h - p.top_thickness;
    let inner = f.offset_loops(p.pitch, p.clearance + p.wall - 0.01);
    // The block: the footprint's box, from the bottom to the top of the body.
    let block = (
        [p.clearance, p.clearance, 0.0],
        [
            f64::from(norm.iter().map(|c| c.0).max().unwrap_or(0) + 1) * p.pitch - p.clearance,
            f64::from(norm.iter().map(|c| c.1).max().unwrap_or(0) + 1) * p.pitch - p.clearance,
            h,
        ],
    );
    // offset(S ∪ P, −wall), each piece on its own (a protect box is a box like any other:
    // its faces inside the block move in, those outside stay put).
    let as_boxes: Vec<TrimShape> = trim
        .protect
        .iter()
        .map(|b| TrimShape {
            op: TrimOp::Add,
            kind: TrimKind::Box { size: [0, 1, 2].map(|i| b.max[i] - b.min[i]) },
            rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            translation: [0, 1, 2].map(|i| (b.max[i] + b.min[i]) / 2.0),
        })
        .collect();
    let mut keep_in = offset_in(&as_boxes, p.wall, block);
    keep_in.extend(offset_in(&trim.shapes, p.wall, block));
    // Where the hollow may stay open: a wall inside the kept surface (or in P), and only
    // under cells that keep their top (a normal hollow with its flat, bridged ceiling) or in
    // the socket under every cell (a stud from below always fits). The rest is filled, so
    // every cavity has upright sides and a flat ceiling: no curved ceiling to print onto air.
    let socket_r = p.stud_diameter / 2.0 + 0.35;
    let socket_h = p.stud_height + 0.4;
    let dropped_cols: Vec<TrimBox> = dropped
        .iter()
        .map(|&(i, j)| {
            let (x0, y0) = (f64::from(i) * p.pitch, f64::from(j) * p.pitch);
            TrimBox { min: [x0 - 0.01, y0 - 0.01, -1.0], max: [x0 + p.pitch + 0.01, y0 + p.pitch + 0.01, h + 1.0] }
        })
        .collect();
    let in_socket = |q: [f64; 3]| {
        q[2] <= socket_h
            && norm.iter().any(|&(i, j)| {
                let c = [(f64::from(i) + 0.5) * p.pitch, (f64::from(j) + 0.5) * p.pitch];
                (q[0] - c[0]).hypot(q[1] - c[1]) <= socket_r
            })
    };
    let ok_in = |q: [f64; 3]| {
        inside_s(&keep_in, q)
            && (!dropped_cols.iter().any(|b| in_box(b, q)) || in_socket(q))
    };
    let in_hollow = |q: [f64; 3]| {
        q[2] > 0.0 && q[2] < hollow_h && inner.iter().filter(|l| crate::trim::in_polygon(&l.iter().map(|v| [v[0], v[1]]).collect::<Vec<_>>(), [q[0], q[1]])).count() % 2 == 1
    };
    let (blo, bhi) = untrimmed.bbox_mm;
    let mut needs_fill = false;
    'scan: for a in 0..((bhi[0] - blo[0]) / 0.5) as i32 {
        for b in 0..((bhi[1] - blo[1]) / 0.5) as i32 {
            for c in 0..((hollow_h) / 0.5) as i32 {
                let q = [blo[0] + 0.25 + 0.5 * f64::from(a), blo[1] + 0.25 + 0.5 * f64::from(b), 0.25 + 0.5 * f64::from(c)];
                if in_hollow(q) && inside_trim(trim, q) && !ok_in(q) {
                    needs_fill = true;
                    break 'scan;
                }
            }
        }
    }
    if needs_fill {
        let mut c = vec![sketch_xy("TrimFillSketch", 0.003)];
        c.extend(inner.iter().map(|l| closed(l)));
        c.push(cmd("sketch.finish", json!({})));
        c.push(cmd(
            "solid.extrude",
            json!({"sketch": "TrimFillSketch", "name": "TrimFillPrism", "distance": r6(hollow_h + 0.01 - 0.003), "operation": "new", "body_name": "TrimFill"}),
        ));
        if let Some(keep_cmds) = body_cmds("TrimKeep", &keep_in, &[], jitter) {
            c.extend(keep_cmds);
            // Under a dropped cell only the socket may stay open: cut (column − socket) out
            // of the keep region, one cell at a time (single-piece tools only).
            for (k, &(i, j)) in dropped.iter().enumerate() {
                let (x0, y0) = (f64::from(i) * p.pitch, f64::from(j) * p.pitch);
                let col = format!("TrimCol{k}");
                let sock = format!("TrimSock{k}");
                c.push(cmd(
                    "solid.box",
                    json!({"length": r6(p.pitch + 0.02), "width": r6(p.pitch + 0.02), "height": r6(h + 2.0), "corner": p3([x0 - 0.01, y0 - 0.01, -1.0]), "operation": "new", "body_name": col, "name": format!("{col}Box")}),
                ));
                c.push(cmd(
                    "solid.cylinder",
                    json!({"radius": r6(socket_r), "height": r6(socket_h + 1.0), "base": p3([x0 + p.pitch / 2.0 + 1.1e-3, y0 + p.pitch / 2.0 + 0.7e-3, -1.0]), "operation": "new", "body_name": sock, "name": format!("{sock}Cyl")}),
                ));
                c.push(cmd("solid.combine", json!({"target": col, "tools": [sock], "operation": "cut"})));
                c.push(cmd("solid.combine", json!({"target": "TrimKeep", "tools": [col], "operation": "cut"})));
            }
        }
        for x in &c {
            run(&mut s, x)?;
        }
        // A cut that splits a body leaves its pieces as bodies of their own ("TrimKeep",
        // "TrimKeep (2)", ...): use every piece.
        for piece in bodies_named(&mut s, "TrimKeep")? {
            run(&mut s, &cmd("solid.combine", json!({"target": "TrimFill", "tools": [piece], "operation": "cut"})))?;
        }
        for piece in bodies_named(&mut s, "TrimFill")? {
            run(&mut s, &cmd("solid.combine", json!({"target": part_name, "tools": [piece], "operation": "join", "touching": "one_body"})))?;
        }
    }
    // 3. Trim: S ∪ P, plus a slab over the kept studs.
    let slabs: Vec<TrimBox> = keep
        .iter()
        .map(|&(i, j)| {
            let (x0, y0) = (f64::from(i) * p.pitch, f64::from(j) * p.pitch);
            TrimBox { min: [x0 + 0.05, y0 + 0.05, h - 0.3], max: [x0 + p.pitch - 0.05, y0 + p.pitch - 0.05, h + p.stud_height + 2.0] }
        })
        .collect();
    let boxes: Vec<TrimBox> = trim.protect.iter().copied().chain(slabs).collect();
    let Some(t_cmds) = body_cmds("TrimSolid", &trim.shapes, &boxes, jitter) else {
        return Err(bad("the trim keeps nothing".into()));
    };
    for x in &t_cmds {
        run(&mut s, x)?;
    }
    // Keep part ∩ T as part − (box − T): right however many pieces T is in.
    let pad = 3.0;
    run(&mut s, &cmd(
        "solid.box",
        json!({"length": r6(bhi[0] - blo[0] + 2.0 * pad), "width": r6(bhi[1] - blo[1] + 2.0 * pad), "height": r6(bhi[2] - blo[2] + 2.0 * pad), "corner": p3([blo[0] - pad, blo[1] - pad, blo[2] - pad]), "operation": "new", "body_name": "TrimAway", "name": "TrimAwayBox"}),
    ))?;
    for piece in bodies_named(&mut s, "TrimSolid")? {
        run(&mut s, &cmd("solid.combine", json!({"target": "TrimAway", "tools": [piece], "operation": "cut"})))?;
    }
    for piece in bodies_named(&mut s, "TrimAway")? {
        run(&mut s, &cmd("solid.combine", json!({"target": part_name, "tools": [piece], "operation": "cut"})))?;
    }
    let m = measure(&mut s, "Trim")?;
    if m.bodies != 1 {
        let r = s.execute("inspect.measure", &json!({})).ok();
        let vols: Vec<String> = r
            .and_then(|r| r["bodies"].as_array().cloned())
            .unwrap_or_default()
            .iter()
            .map(|b| format!("{:.2} mm³ at {}", b["volume_mm3"].as_f64().unwrap_or(0.0), b["bbox"]["min"]))
            .collect();
        return Err(bad(format!("{} bodies after the trim, expected 1 (the trim splits the brick): {}", m.bodies, vols.join("; "))));
    }
    let part = finish(&mut s, "Trim", None, None)?;
    if !analyze_stl(&part.stl)?.is_watertight() {
        return Err(bad("the trimmed mesh is not watertight".into()));
    }
    // Inside the plain brick's box.
    for k in 0..3 {
        if part.bbox_mm.0[k] < blo[k] - 0.05 || part.bbox_mm.1[k] > bhi[k] + 0.05 {
            return Err(bad(format!("the trimmed brick pokes out of the block's box on axis {k}")));
        }
    }
    // The independent estimate.
    let rays = Rays::cast(&untrimmed.stl, [blo[0], blo[1]], [bhi[0], bhi[1]], SAMPLE_STEP);
    let dz = SAMPLE_STEP / 2.0;
    let nz = ((bhi[2] - blo[2]) / dz).ceil() as usize;
    let mut n = 0usize;
    for i in 0..rays.nx {
        for j in 0..rays.ny {
            let x = rays.lo[0] + (i as f64 + 0.5) * SAMPLE_STEP;
            let y = rays.lo[1] + (j as f64 + 0.5) * SAMPLE_STEP;
            for k in 0..nz {
                let z = blo[2] + (k as f64 + 0.5) * dz;
                let q = [x, y, z];
                let kept = inside_trim(trim, q) || boxes.iter().any(|b| in_box(b, q));
                if !kept {
                    continue;
                }
                let solid = rays.inside(i, j, z) || (needs_fill && in_hollow(q) && !ok_in(q));
                if solid {
                    n += 1;
                }
            }
        }
    }
    let sampled = n as f64 * SAMPLE_STEP * SAMPLE_STEP * dz;
    let rel = (part.volume_mm3 - sampled) / sampled.max(1e-9);
    if rel.abs() > TRIM_VOLUME_TOL {
        return Err(bad(format!(
            "volume {:.1} mm³ but the sampled estimate is {sampled:.1} mm³ ({:+.1} %)",
            part.volume_mm3,
            rel * 100.0
        )));
    }
    let overhang = brick_overhang(&part.stl);
    Ok(Trimmed { part, untrimmed, sampled_volume: sampled, overhang_mm2: overhang, dropped_studs: dropped, filled: needs_fill, commands })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id() -> [[f64; 3]; 3] {
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
    }

    #[test]
    fn point_tests_follow_the_shapes() {
        let e = TrimShape { op: TrimOp::Add, kind: TrimKind::Ellipsoid { radii: [2.0, 1.0, 1.0] }, rotation: id(), translation: [5.0, 0.0, 0.0] };
        assert!(inside_s(std::slice::from_ref(&e), [6.9, 0.0, 0.0]));
        assert!(!inside_s(std::slice::from_ref(&e), [5.0, 1.1, 0.0]));
        let hole = TrimShape { op: TrimOp::Subtract, kind: TrimKind::Box { size: [1.0, 1.0, 1.0] }, rotation: id(), translation: [5.0, 0.0, 0.0] };
        assert!(!inside_s(&[e.clone(), hole], [5.0, 0.0, 0.0]));
        let c = TrimKind::Cone { radii: [2.0, 2.0], top_scale: 0.0, height: 4.0 };
        assert!(inside_kind(&c, [1.9, 0.0, -1.99]) && !inside_kind(&c, [1.1, 0.0, 0.0]));
        let cap = TrimKind::Capsule { a: [0.0, 0.0, 0.0], b: [4.0, 0.0, 0.0], radius: 1.0 };
        assert!(inside_kind(&cap, [4.9, 0.0, 0.0]) && !inside_kind(&cap, [2.0, 1.1, 0.0]));
        let pr = TrimKind::Prism { points: vec![[0.0, 0.0], [4.0, 0.0], [0.0, 4.0]], height: 2.0 };
        assert!(inside_kind(&pr, [1.0, 1.0, 0.9]) && !inside_kind(&pr, [3.0, 3.0, 0.0]));
        // A shrunk prism is inside the prism, a wall away from its edges.
        let TrimKind::Prism { points, .. } = offset_kind(&pr, -0.5).unwrap() else { panic!() };
        assert!(points.iter().all(|q| q[0] >= 0.49 && q[1] >= 0.49));
    }
}
