//! Curved surfaces for printed bricks: where a brick's outside is exposed, it follows the
//! true shape Claude described instead of stair-stepping cell by cell.
//!
//! A trimmed brick keeps `block ∩ (S ∪ P)` (see [`sf_core::Trim`]): `S` is the spec's
//! solid near the brick, moved into the brick's print frame; `P` are protect boxes that
//! keep everything touching another part as the plain brick:
//! * the top slab and stud area of every column with a part (or detail) on it;
//! * a slab 1.6 mm either side of every side face against another part, except a brick
//!   that is trimmed too: both are cut by the same surface, so their shared face is flush
//!   wherever there is material, and a slab would only leave a fin outside the surface
//!   (neighbours in a course hold nothing sideways);
//! * the bottom 1.6 mm and the socket hollow of every column resting on something.
//!
//! The display mesh is built by marching tetrahedra on a signed-distance field sampled
//! every [`MESH_STEP`] mm (lazily, only in blocks the surface can pass through), then
//! flat and gently curved areas are simplified by edge collapses. Its downward faces set
//! [`sf_core::Trim::overhang`].

use sf_core::{Cell, CustomDef, DetailMesh, Placement, Trim, TrimBox, TrimKind, TrimOp, TrimShape};
use sf_spec::SurfaceShape;
use std::collections::{HashMap, HashSet};

/// Display-mesh sampling step, mm (divides a stud, 8 mm, and a plate, 3.2 mm).
pub const MESH_STEP: f64 = 1.6;
/// How far protect slabs reach past a face (at least a wall), mm.
const REACH: f64 = 1.6;
/// Thickness of the flat top kept under a part above (the top is 1.4 mm), mm.
const TOP_KEEP: f64 = 1.6;
/// Height above the top kept for studs, mm.
const STUD_ROOM: f64 = 2.5;
/// The printed top's thickness: the socket hollow reaches up to `H - TOP`, mm.
const TOP: f64 = 1.4;
/// A side wall plus the outside clearance, mm: the hollow starts this far inside an outer face.
const WALL: f64 = 1.3;
/// Spec primitives this close to a brick (mm) are kept in its trim.
const NEAR: f64 = 8.0;
/// Overhangs: downward faces more than 45° from vertical, above the bottom, this much area,
/// with nothing kept within a 45° cone [`OVERHANG_PROBE`] mm below them (a ledge that
/// short, like a flat top kept a little past a trimmed wall, prints without supports).
const OVERHANG_COS: f64 = std::f64::consts::FRAC_1_SQRT_2;
const OVERHANG_PROBE: f64 = 1.6;
const OVERHANG_MIN_Z: f64 = 0.3;
const OVERHANG_MIN_AREA: f64 = 1.0;

type M3 = [[f64; 3]; 3];

fn det(m: &M3) -> f64 {
    m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
}

/// The spec's primitives near the brick at grid `origin` with size `[sx, sz]` studs and
/// `plates`, in the brick's print frame (x along local x, y along local z, z up, mm).
pub fn shapes_for(surface: &[SurfaceShape], origin: Cell, size: [u8; 2], plates: u8) -> Vec<TrimShape> {
    // Brick bbox in grid-mm (X, Y up, Z).
    let lo = [8.0 * origin.x as f64, 3.2 * origin.y as f64, 8.0 * origin.z as f64];
    let hi = [lo[0] + 8.0 * size[0] as f64, lo[1] + 3.2 * plates as f64, lo[2] + 8.0 * size[1] as f64];
    let mut out = Vec::new();
    for s in surface {
        if (0..3).any(|k| s.aabb.0[k] > hi[k] + NEAR || s.aabb.1[k] < lo[k] - NEAR) {
            continue;
        }
        // Brick frame: x = X − 8·ox, y = Z − 8·oz, z = Y − 3.2·oy.
        let l: M3 = [s.m[0], s.m[2], s.m[1]];
        let t = [s.t[0] - lo[0], s.t[2] - lo[2], s.t[1] - lo[1]];
        let d = det(&l);
        if d.abs() < 1e-12 {
            continue;
        }
        let sigma = d.abs().cbrt();
        let mut r = l.map(|row| row.map(|v| v / sigma));
        let mirror = d < 0.0;
        if mirror {
            for row in r.iter_mut() {
                row[0] = -row[0];
            }
        }
        let fx = if mirror { -1.0 } else { 1.0 };
        let kind = match &s.kind {
            TrimKind::Ellipsoid { radii } => TrimKind::Ellipsoid { radii: radii.map(|v| v * sigma) },
            TrimKind::Box { size } => TrimKind::Box { size: size.map(|v| v * sigma) },
            TrimKind::Cylinder { radii, height } => TrimKind::Cylinder { radii: radii.map(|v| v * sigma), height: height * sigma },
            TrimKind::Cone { radii, top_scale, height } => {
                TrimKind::Cone { radii: radii.map(|v| v * sigma), top_scale: *top_scale, height: height * sigma }
            }
            TrimKind::Capsule { a, b, radius } => TrimKind::Capsule {
                a: [a[0] * sigma * fx, a[1] * sigma, a[2] * sigma],
                b: [b[0] * sigma * fx, b[1] * sigma, b[2] * sigma],
                radius: radius * sigma,
            },
            TrimKind::Prism { points, height } => TrimKind::Prism {
                points: points.iter().map(|p| [p[0] * sigma * fx, p[1] * sigma]).collect(),
                height: height * sigma,
            },
        };
        out.push(TrimShape { op: s.op, kind, rotation: r, translation: t });
    }
    out
}

/// Who fills each cell, for exposure and protection.
pub struct Occupancy {
    pub owner: HashMap<Cell, usize>,
}

impl Occupancy {
    pub fn new(placements: &[Placement], meshes: &std::collections::BTreeMap<String, DetailMesh>) -> Self {
        let mut owner = HashMap::new();
        for (i, p) in placements.iter().enumerate() {
            for c in crate::details::occupied_cells(p, meshes) {
                owner.insert(c, i);
            }
        }
        Occupancy { owner }
    }

    fn other(&self, c: Cell, me: usize) -> bool {
        self.owner.get(&c).is_some_and(|&o| o != me)
    }
}

/// Is printed brick `i` exposed anywhere (a top, a bottom off the ground, or a side with
/// nothing against it)? Only exposed bricks are trimmed.
pub fn is_exposed(i: usize, p: &Placement, occ: &Occupancy) -> bool {
    let Some(def) = p.custom.as_deref() else { return false };
    let o = p.origin;
    let plates = def.plates as i32;
    let cols: HashSet<(i32, i32)> = def.cells.iter().map(|&(x, z)| (x as i32, z as i32)).collect();
    cols.iter().any(|&(x, z)| {
        !occ.other(Cell::new(o.x + x, o.y + plates, o.z + z), i)
            || (o.y > 0 && !occ.other(Cell::new(o.x + x, o.y - 1, o.z + z), i))
            || [(-1, 0), (1, 0), (0, -1), (0, 1)].iter().any(|&(dx, dz)| {
                !cols.contains(&(x + dx, z + dz))
                    && (0..plates).any(|k| !occ.other(Cell::new(o.x + x + dx, o.y + k, o.z + z + dz), i))
            })
    })
}

/// The trim for printed brick `i` (at `Rot(0)`), or `None` when nothing of it is exposed or
/// no spec shape reaches it. `trimmed(j)` says whether part `j` is a brick that is trimmed
/// too (no side slab is kept against it).
pub fn brick_trim(i: usize, p: &Placement, occ: &Occupancy, surface: &[SurfaceShape], trimmed: &dyn Fn(usize) -> bool) -> Option<Trim> {
    let def = p.custom.as_deref()?;
    let o = p.origin;
    let plates = def.plates as i32;
    let h = 3.2 * plates as f64;
    let cols: HashSet<(i32, i32)> = def.cells.iter().map(|&(x, z)| (x as i32, z as i32)).collect();
    let inside = |x: i32, z: i32| cols.contains(&(x, z));
    let cell = |lx: i32, ly: i32, lz: i32| Cell::new(o.x + lx, o.y + ly, o.z + lz);
    let mut exposed = false;
    let mut protect: Vec<TrimBox> = Vec::new();
    // Outward reach for a column's side, 0 where the brick continues.
    let ext = |x: i32, z: i32, dx: i32, dz: i32| if inside(x + dx, z + dz) { 0.0 } else { REACH };
    for &(x, z) in &cols {
        let (x0, x1) = (8.0 * x as f64, 8.0 * x as f64 + 8.0);
        let (y0, y1) = (8.0 * z as f64, 8.0 * z as f64 + 8.0);
        let (ex0, ex1, ey0, ey1) = (ext(x, z, -1, 0), ext(x, z, 1, 0), ext(x, z, 0, -1), ext(x, z, 0, 1));
        // Top.
        if occ.other(cell(x, plates, z), i) {
            protect.push(TrimBox { min: [x0 - ex0, y0 - ey0, h - TOP_KEEP], max: [x1 + ex1, y1 + ey1, h + STUD_ROOM] });
        } else {
            exposed = true;
        }
        // Bottom.
        if o.y == 0 || occ.other(cell(x, -1, z), i) {
            protect.push(TrimBox { min: [x0 - ex0, y0 - ey0, -REACH], max: [x1 + ex1, y1 + ey1, REACH] });
            let inset = |e: f64| if e > 0.0 { WALL } else { 0.0 };
            let top = (h - TOP).max(REACH);
            protect.push(TrimBox {
                min: [x0 + inset(ex0), y0 + inset(ey0), 0.0],
                max: [x1 - inset(ex1), y1 - inset(ey1), top],
            });
        } else {
            exposed = true;
        }
        // Sides.
        for (dx, dz) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            if inside(x + dx, z + dz) {
                continue;
            }
            let mut k = 0;
            while k < plates {
                let held = |k: i32| occ.owner.get(&cell(x + dx, k, z + dz)).is_some_and(|&j| j != i && !trimmed(j));
                if !occ.other(cell(x + dx, k, z + dz), i) {
                    exposed = true;
                }
                if !held(k) {
                    k += 1;
                    continue;
                }
                let k0 = k;
                while k < plates && held(k) {
                    k += 1;
                }
                let (z0, z1) = (3.2 * k0 as f64, 3.2 * k as f64);
                let b = match (dx, dz) {
                    (-1, _) => TrimBox { min: [x0 - REACH, y0 - ey0, z0], max: [x0 + REACH, y1 + ey1, z1] },
                    (1, _) => TrimBox { min: [x1 - REACH, y0 - ey0, z0], max: [x1 + REACH, y1 + ey1, z1] },
                    (_, -1) => TrimBox { min: [x0 - ex0, y0 - REACH, z0], max: [x1 + ex1, y0 + REACH, z1] },
                    _ => TrimBox { min: [x0 - ex0, y1 - REACH, z0], max: [x1 + ex1, y1 + REACH, z1] },
                };
                protect.push(b);
            }
        }
    }
    if !exposed {
        return None;
    }
    let shapes = shapes_for(surface, o, def.size(), def.plates);
    if !shapes.iter().any(|s| s.op == TrimOp::Add) {
        return None;
    }
    Some(Trim { shapes, protect, overhang: false })
}

// ---------------------------------------------------------------------------------------
// Signed distances (negative inside). Exact for boxes, capsules and prisms; close bounds
// for ellipsoids, cylinders and cones. Zero sets are exact either way.

fn len3(p: [f64; 3]) -> f64 {
    (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt()
}

fn sd_box(p: [f64; 3], half: [f64; 3]) -> f64 {
    let q = [p[0].abs() - half[0], p[1].abs() - half[1], p[2].abs() - half[2]];
    let out = len3([q[0].max(0.0), q[1].max(0.0), q[2].max(0.0)]);
    out + q[0].max(q[1]).max(q[2]).min(0.0)
}

fn sd_ellipsoid(p: [f64; 3], r: [f64; 3]) -> f64 {
    let k0 = len3([p[0] / r[0], p[1] / r[1], p[2] / r[2]]);
    let k1 = len3([p[0] / (r[0] * r[0]), p[1] / (r[1] * r[1]), p[2] / (r[2] * r[2])]);
    if k1 < 1e-12 {
        return -r[0].min(r[1]).min(r[2]);
    }
    k0 * (k0 - 1.0) / k1
}

fn sd_ellipse(x: f64, y: f64, r: [f64; 2]) -> f64 {
    if r[0] <= 1e-9 || r[1] <= 1e-9 {
        return (x * x + y * y).sqrt();
    }
    let k0 = ((x / r[0]).powi(2) + (y / r[1]).powi(2)).sqrt();
    let k1 = ((x / (r[0] * r[0])).powi(2) + (y / (r[1] * r[1])).powi(2)).sqrt();
    if k1 < 1e-12 {
        return -r[0].min(r[1]);
    }
    k0 * (k0 - 1.0) / k1
}

/// Combine a radial and an axial distance like a box corner.
fn extrusion(dr: f64, da: f64) -> f64 {
    let out = (dr.max(0.0).powi(2) + da.max(0.0).powi(2)).sqrt();
    out + dr.max(da).min(0.0)
}

fn sd_polygon(pts: &[[f64; 2]], x: f64, y: f64) -> f64 {
    let n = pts.len();
    if n < 3 {
        return f64::INFINITY;
    }
    let mut d = f64::INFINITY;
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (pts[j], pts[i]);
        let e = [b[0] - a[0], b[1] - a[1]];
        let w = [x - a[0], y - a[1]];
        let l2 = e[0] * e[0] + e[1] * e[1];
        let t = if l2 > 0.0 { ((w[0] * e[0] + w[1] * e[1]) / l2).clamp(0.0, 1.0) } else { 0.0 };
        let q = [w[0] - e[0] * t, w[1] - e[1] * t];
        d = d.min(q[0] * q[0] + q[1] * q[1]);
        if (b[1] > y) != (a[1] > y) && x < (a[0] - b[0]) * (y - b[1]) / (a[1] - b[1]) + b[0] {
            inside = !inside;
        }
        j = i;
    }
    let d = d.sqrt();
    if inside { -d } else { d }
}

/// Signed distance to one trim primitive, in the brick frame.
pub fn sd_shape(s: &TrimShape, p: [f64; 3]) -> f64 {
    let v = [p[0] - s.translation[0], p[1] - s.translation[1], p[2] - s.translation[2]];
    let r = &s.rotation;
    // Local = Rᵀ · v.
    let q = [
        r[0][0] * v[0] + r[1][0] * v[1] + r[2][0] * v[2],
        r[0][1] * v[0] + r[1][1] * v[1] + r[2][1] * v[2],
        r[0][2] * v[0] + r[1][2] * v[1] + r[2][2] * v[2],
    ];
    match &s.kind {
        TrimKind::Box { size } => sd_box(q, [size[0] / 2.0, size[1] / 2.0, size[2] / 2.0]),
        TrimKind::Ellipsoid { radii } => sd_ellipsoid(q, *radii),
        TrimKind::Cylinder { radii, height } => extrusion(sd_ellipse(q[0], q[1], *radii), q[2].abs() - height / 2.0),
        TrimKind::Cone { radii, top_scale, height } => {
            let t = ((q[2] + height / 2.0) / height).clamp(0.0, 1.0);
            let k = 1.0 - t * (1.0 - top_scale);
            let rr = [radii[0] * k, radii[1] * k];
            // Scale the radial distance by the side's slope so it approximates the normal one.
            let slope = (radii[0].max(radii[1]) * (1.0 - top_scale)) / height.max(1e-9);
            let dr = if rr[0] < 1e-9 || rr[1] < 1e-9 { (q[0] * q[0] + q[1] * q[1]).sqrt() } else { sd_ellipse(q[0], q[1], rr) };
            extrusion(dr / (1.0 + slope * slope).sqrt(), q[2].abs() - height / 2.0)
        }
        TrimKind::Capsule { a, b, radius } => {
            let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let ap = [q[0] - a[0], q[1] - a[1], q[2] - a[2]];
            let l2 = ab[0] * ab[0] + ab[1] * ab[1] + ab[2] * ab[2];
            let t = if l2 > 0.0 { ((ap[0] * ab[0] + ap[1] * ab[1] + ap[2] * ab[2]) / l2).clamp(0.0, 1.0) } else { 0.0 };
            len3([ap[0] - ab[0] * t, ap[1] - ab[1] * t, ap[2] - ab[2] * t]) - radius
        }
        TrimKind::Prism { points, height } => extrusion(sd_polygon(points, q[0], q[1]), q[2].abs() - height / 2.0),
    }
}

fn sd_trim_box(b: &TrimBox, p: [f64; 3]) -> f64 {
    let c = [0, 1, 2].map(|k| (b.min[k] + b.max[k]) / 2.0);
    let h = [0, 1, 2].map(|k| (b.max[k] - b.min[k]) / 2.0);
    sd_box([p[0] - c[0], p[1] - c[1], p[2] - c[2]], h)
}

/// The kept solid's field: `max(block, min(S, P))`.
struct Field<'a> {
    trim: &'a Trim,
    cols: HashSet<(i32, i32)>,
    h: f64,
}

impl Field<'_> {
    fn solid(&self, p: [f64; 3]) -> f64 {
        let mut d = f64::INFINITY;
        for s in &self.trim.shapes {
            let ds = sd_shape(s, p);
            d = match s.op {
                TrimOp::Add => d.min(ds),
                TrimOp::Subtract => d.max(-ds),
            };
        }
        d
    }

    fn block(&self, p: [f64; 3]) -> f64 {
        let (cx, cy) = ((p[0] / 8.0).floor() as i32, (p[1] / 8.0).floor() as i32);
        let mut d = 8.0f64;
        for dx in -1..=1 {
            for dy in -1..=1 {
                let (x, y) = (cx + dx, cy + dy);
                if self.cols.contains(&(x, y)) {
                    let c = [8.0 * x as f64 + 4.0, 8.0 * y as f64 + 4.0, self.h / 2.0];
                    d = d.min(sd_box([p[0] - c[0], p[1] - c[1], p[2] - c[2]], [4.0, 4.0, self.h / 2.0]));
                }
            }
        }
        d
    }

    fn eval(&self, p: [f64; 3]) -> f64 {
        let b = self.block(p);
        if b > 0.5 {
            return b; // well outside the block: nothing else matters
        }
        let mut keep = self.solid(p);
        for bx in &self.trim.protect {
            keep = keep.min(sd_trim_box(bx, p));
        }
        b.max(keep)
    }
}

/// A trimmed brick's display mesh and what was measured on it.
pub struct TrimmedMesh {
    /// With studs, in the catalog-part frame (LDU, `-Y` up, origin at the top centre of
    /// the local bounding box).
    pub mesh: DetailMesh,
    /// A downward face steeper than 45° from vertical (supports needed).
    pub overhang: bool,
    /// Volume of the kept solid without studs or the socket hollow, mm³.
    pub volume_mm3: f64,
}

/// A display mesh of `block ∩ (S ∪ P)` for a trimmed brick (see [`TrimmedMesh`]).
pub fn trimmed_mesh(def: &CustomDef, trim: &Trim) -> TrimmedMesh {
    let cols: HashSet<(i32, i32)> = def.cells.iter().map(|&(x, z)| (x as i32, z as i32)).collect();
    let [sx, sz] = def.size();
    let h = 3.2 * def.plates as f64;
    let field = Field { trim, cols, h };
    let step = MESH_STEP;
    // Samples at (i − ½)·step: block faces fall halfway between samples.
    let n = [
        (8.0 * sx as f64 / step).round() as usize + 2,
        (8.0 * sz as f64 / step).round() as usize + 2,
        (h / step).round() as usize + 2,
    ];
    let pos = |i: usize, j: usize, k: usize| [(i as f64 - 0.5) * step, (j as f64 - 0.5) * step, (k as f64 - 0.5) * step];
    let idx = |i: usize, j: usize, k: usize| (k * n[1] + j) * n[0] + i;
    let mut values = vec![f64::NAN; n[0] * n[1] * n[2]];
    let value = |values: &mut Vec<f64>, i: usize, j: usize, k: usize| -> f64 {
        let v = &mut values[idx(i, j, k)];
        if v.is_nan() {
            *v = field.eval(pos(i, j, k));
        }
        *v
    };
    let mut tris: Vec<[u32; 3]> = Vec::new();
    let mut verts: Vec<[f64; 3]> = Vec::new();
    let mut edge_vert: HashMap<(usize, usize), u32> = HashMap::new();
    const B: usize = 6; // block side in samples
    let half_diag = (B as f64) * step * 3f64.sqrt() / 2.0;
    // Tetrahedra of a cube (corner bits: x = 1, y = 2, z = 4) around the 0–7 diagonal.
    const TETS: [[usize; 4]; 6] = [[0, 1, 3, 7], [0, 3, 2, 7], [0, 2, 6, 7], [0, 6, 4, 7], [0, 4, 5, 7], [0, 5, 1, 7]];
    for bk in (0..n[2] - 1).step_by(B) {
        for bj in (0..n[1] - 1).step_by(B) {
            for bi in (0..n[0] - 1).step_by(B) {
                let ci = (bi + B / 2).min(n[0] - 1);
                let cj = (bj + B / 2).min(n[1] - 1);
                let ck = (bk + B / 2).min(n[2] - 1);
                if value(&mut values, ci, cj, ck).abs() > 2.0 * half_diag {
                    continue;
                }
                for k in bk..(bk + B).min(n[2] - 1) {
                    for j in bj..(bj + B).min(n[1] - 1) {
                        for i in bi..(bi + B).min(n[0] - 1) {
                            let corner = |c: usize| (i + (c & 1), j + ((c >> 1) & 1), k + ((c >> 2) & 1));
                            let mut v = [0.0; 8];
                            let mut any_in = false;
                            let mut any_out = false;
                            for (c, vc) in v.iter_mut().enumerate() {
                                let (a, b, cc) = corner(c);
                                *vc = value(&mut values, a, b, cc);
                                if *vc < 0.0 { any_in = true } else { any_out = true }
                            }
                            if !(any_in && any_out) {
                                continue;
                            }
                            for t in TETS {
                                let ins: Vec<usize> = t.iter().copied().filter(|&c| v[c] < 0.0).collect();
                                let outs: Vec<usize> = t.iter().copied().filter(|&c| v[c] >= 0.0).collect();
                                if ins.is_empty() || outs.is_empty() {
                                    continue;
                                }
                                let mut vert = |a: usize, b: usize| -> u32 {
                                    let (pa, pb) = (corner(a), corner(b));
                                    let (ka, kb) = (idx(pa.0, pa.1, pa.2), idx(pb.0, pb.1, pb.2));
                                    let key = if ka < kb { (ka, kb) } else { (kb, ka) };
                                    *edge_vert.entry(key).or_insert_with(|| {
                                        let (xa, xb) = (pos(pa.0, pa.1, pa.2), pos(pb.0, pb.1, pb.2));
                                        let t = (v[a] / (v[a] - v[b])).clamp(0.0, 1.0);
                                        verts.push([0, 1, 2].map(|q| xa[q] + (xb[q] - xa[q]) * t));
                                        (verts.len() - 1) as u32
                                    })
                                };
                                // Triangles, then oriented so their normal points out.
                                let mut local: Vec<([u32; 3], usize, usize)> = Vec::new();
                                match (ins.len(), outs.len()) {
                                    (1, 3) => {
                                        let a = ins[0];
                                        local.push(([vert(a, outs[0]), vert(a, outs[1]), vert(a, outs[2])], a, outs[0]));
                                    }
                                    (3, 1) => {
                                        let b = outs[0];
                                        local.push(([vert(ins[0], b), vert(ins[1], b), vert(ins[2], b)], ins[0], b));
                                    }
                                    _ => {
                                        let (a, b, c, d) = (ins[0], ins[1], outs[0], outs[1]);
                                        let (p, q, r, s) = (vert(a, c), vert(a, d), vert(b, d), vert(b, c));
                                        local.push(([p, q, r], a, c));
                                        local.push(([p, r, s], a, c));
                                    }
                                }
                                for (tri, a, b) in local {
                                    let (pa, pb) = (corner(a), corner(b));
                                    let dir = {
                                        let (xa, xb) = (pos(pa.0, pa.1, pa.2), pos(pb.0, pb.1, pb.2));
                                        [xb[0] - xa[0], xb[1] - xa[1], xb[2] - xa[2]]
                                    };
                                    let nrm = normal(&verts, tri);
                                    let t = if nrm[0] * dir[0] + nrm[1] * dir[1] + nrm[2] * dir[2] < 0.0 { [tri[0], tri[2], tri[1]] } else { tri };
                                    if t[0] != t[1] && t[1] != t[2] && t[0] != t[2] {
                                        tris.push(t);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    let before = tris.len();
    simplify(&mut verts, &mut tris, step);
    relax(&mut verts, &tris, &field, 3);
    simplify(&mut verts, &mut tris, step);
    if std::env::var("ZZ_MESH").is_ok() {
        let flat = tris.iter().filter(|&&t| { let n = normal(&verts, t); n[0].abs() > 0.9999 || n[1].abs() > 0.9999 || n[2].abs() > 0.9999 }).count();
        eprintln!("mesh {}: {before} -> {} tris ({flat} axis-flat)", def.id, tris.len());
    }
    // Overhangs, on the trimmed solid alone.
    let mut down = 0.0;
    for &t in &tris {
        let c = cross(&verts, t);
        let area = len3(c) / 2.0;
        if area < 1e-12 {
            continue;
        }
        let centre = [0, 1, 2].map(|k| (verts[t[0] as usize][k] + verts[t[1] as usize][k] + verts[t[2] as usize][k]) / 3.0);
        if centre[2] > OVERHANG_MIN_Z && c[2] / (2.0 * area) < -OVERHANG_COS && !supported(&field, centre) {
            down += area;
        }
    }
    let overhang = down > OVERHANG_MIN_AREA;
    let volume_mm3 = tris
        .iter()
        .map(|t| {
            let (a, b, c) = (verts[t[0] as usize], verts[t[1] as usize], verts[t[2] as usize]);
            (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0]) + a[2] * (b[0] * c[1] - b[1] * c[0])) / 6.0
        })
        .sum::<f64>();
    // Studs: plain round ones in the display (the print fuses them).
    for &(lx, lz) in &def.studs {
        stud(&mut verts, &mut tris, [8.0 * lx as f64 + 4.0, 8.0 * lz as f64 + 4.0], h);
    }
    // mm, print frame → LDU catalog frame: (x, y, z) ↦ 2.5·(x − W/2, H − z, y − D/2).
    let (w, d) = (8.0 * sx as f64, 8.0 * sz as f64);
    let positions = verts.iter().map(|v| [((v[0] - w / 2.0) * 2.5) as f32, ((h - v[2]) * 2.5) as f32, ((v[1] - d / 2.0) * 2.5) as f32]).collect();
    // The map has determinant +1 times a positive scale, so windings stay outward.
    let indices = tris.iter().flatten().copied().collect();
    TrimmedMesh { mesh: DetailMesh { positions, indices }, overhang, volume_mm3 }
}

/// Smooths marching-tetrahedra facet noise: each vertex on a curved, crease-free patch
/// moves to its neighbours' average and is projected back onto the surface along the
/// field's gradient. Vertices on flat faces or creases stay put.
fn relax(verts: &mut [[f64; 3]], tris: &[[u32; 3]], field: &Field, iterations: usize) {
    let mut nbrs: Vec<Vec<u32>> = vec![Vec::new(); verts.len()];
    let mut vt: Vec<Vec<usize>> = vec![Vec::new(); verts.len()];
    for (ti, t) in tris.iter().enumerate() {
        for k in 0..3 {
            let (a, b) = (t[k] as usize, t[(k + 1) % 3]);
            nbrs[a].push(b);
            nbrs[b as usize].push(t[k]);
            vt[a].push(ti);
        }
    }
    for n in nbrs.iter_mut() {
        n.sort_unstable();
        n.dedup();
    }
    // Which vertices may move: their faces differ (curved) but not sharply (no crease).
    let movable: Vec<bool> = (0..verts.len())
        .map(|v| {
            let ns: Vec<[f64; 3]> = vt[v].iter().map(|&t| normal(verts, tris[t])).filter(|n| len3(*n) > 0.5).collect();
            if ns.len() < 3 {
                return false;
            }
            let min = ns.iter().flat_map(|a| ns.iter().map(move |b| dot(*a, *b))).fold(1.0f64, f64::min);
            (0.0..0.99995).contains(&min) && min > 0.5
        })
        .collect();
    let e = 0.05;
    for _ in 0..iterations {
        let snapshot = verts.to_vec();
        for v in 0..verts.len() {
            if !movable[v] || nbrs[v].is_empty() {
                continue;
            }
            let n = nbrs[v].len() as f64;
            let avg = nbrs[v].iter().fold([0.0; 3], |a, &u| {
                let p = snapshot[u as usize];
                [a[0] + p[0] / n, a[1] + p[1] / n, a[2] + p[2] / n]
            });
            // Project back onto the zero set (one Newton step on the field).
            let f = field.eval(avg);
            let g = [
                field.eval([avg[0] + e, avg[1], avg[2]]) - field.eval([avg[0] - e, avg[1], avg[2]]),
                field.eval([avg[0], avg[1] + e, avg[2]]) - field.eval([avg[0], avg[1] - e, avg[2]]),
                field.eval([avg[0], avg[1], avg[2] + e]) - field.eval([avg[0], avg[1], avg[2] - e]),
            ]
            .map(|x| x / (2.0 * e));
            let g2 = dot(g, g);
            if g2 < 1e-6 {
                continue;
            }
            let p = [avg[0] - f * g[0] / g2, avg[1] - f * g[1] / g2, avg[2] - f * g[2] / g2];
            // Don't wander: stay within a step of where the vertex was.
            let d = [p[0] - snapshot[v][0], p[1] - snapshot[v][1], p[2] - snapshot[v][2]];
            if len3(d) < MESH_STEP {
                verts[v] = p;
            }
        }
    }
}

/// Is there kept material (or the print bed) within a 45° cone [`OVERHANG_PROBE`] mm below `p`?
fn supported(field: &Field, p: [f64; 3]) -> bool {
    let z = p[2] - OVERHANG_PROBE;
    if z <= 0.0 {
        return true;
    }
    for r in [0.0, OVERHANG_PROBE / 2.0, OVERHANG_PROBE] {
        let n = if r == 0.0 { 1 } else { 8 };
        for k in 0..n {
            let a = std::f64::consts::TAU * k as f64 / n as f64;
            if field.eval([p[0] + r * a.cos(), p[1] + r * a.sin(), z]) < 0.0 {
                return true;
            }
        }
    }
    false
}

fn cross(v: &[[f64; 3]], t: [u32; 3]) -> [f64; 3] {
    let (a, b, c) = (v[t[0] as usize], v[t[1] as usize], v[t[2] as usize]);
    let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]]
}

fn normal(v: &[[f64; 3]], t: [u32; 3]) -> [f64; 3] {
    let c = cross(v, t);
    let l = len3(c);
    if l < 1e-15 { [0.0; 3] } else { c.map(|x| x / l) }
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Collapses edges where the surface is flat (any length) or gently curved (short edges
/// only), keeping the mesh closed and its faces' orientation.
fn simplify(verts: &mut [[f64; 3]], tris: &mut Vec<[u32; 3]>, step: f64) {
    const FLAT: f64 = 0.99995;
    // Gently curved: an edge of 4 mm on a 20 mm radius sags 0.1 mm, invisible in a view.
    const GENTLE: f64 = 0.99;
    let gentle_edge = 5.0 * step;
    for _pass in 0..60 {
        let mut vt: Vec<Vec<usize>> = vec![Vec::new(); verts.len()];
        for (ti, t) in tris.iter().enumerate() {
            for &v in t {
                vt[v as usize].push(ti);
            }
        }
        let mut alive_t = vec![true; tris.len()];
        let mut touched = vec![false; verts.len()];
        let mut changed = 0;
        for v in 0..verts.len() {
            if touched[v] || vt[v].is_empty() {
                continue;
            }
            let ring: Vec<usize> = vt[v].iter().copied().filter(|&t| alive_t[t]).collect();
            if ring.len() < 3 {
                continue;
            }
            // Compare each face with the ring's area-weighted normal; slivers (marching
            // tetrahedra makes some) have meaningless normals and don't count.
            let cs: Vec<[f64; 3]> = ring.iter().map(|&t| cross(verts, tris[t])).collect();
            let sum = cs.iter().fold([0.0; 3], |a, c| [a[0] + c[0], a[1] + c[1], a[2] + c[2]]);
            let total = len3(sum);
            if total < 1e-12 {
                continue;
            }
            let avg = sum.map(|x| x / total);
            let min_dot = cs
                .iter()
                .filter(|c| len3(**c) > 1e-3 * total)
                .map(|c| dot(c.map(|x| x / len3(*c)), avg))
                .fold(1.0f64, f64::min);
            if min_dot < GENTLE {
                continue;
            }
            let flat = min_dot >= FLAT;
            let mut nbrs: Vec<u32> = ring.iter().flat_map(|&t| tris[t]).filter(|&u| u as usize != v).collect();
            nbrs.sort_unstable();
            nbrs.dedup();
            'cand: for &u in &nbrs {
                let u = u as usize;
                if touched[u] {
                    continue;
                }
                let e = [verts[u][0] - verts[v][0], verts[u][1] - verts[v][1], verts[u][2] - verts[v][2]];
                if !flat && len3(e) > gentle_edge {
                    continue;
                }
                // Link condition: exactly two common neighbours.
                let un: HashSet<u32> = vt[u].iter().filter(|&&t| alive_t[t]).flat_map(|&t| tris[t]).collect();
                let common = nbrs.iter().filter(|&&w| w as usize != u && un.contains(&w)).count();
                if common != 2 {
                    continue;
                }
                // No flipped or degenerate faces after the move.
                for &t in &ring {
                    let tri = tris[t];
                    if tri.contains(&(u as u32)) {
                        continue;
                    }
                    let moved = tri.map(|w| if w as usize == v { u as u32 } else { w });
                    let (n0, c1) = (normal(verts, tri), cross(verts, moved));
                    if len3(c1) < 1e-9 || dot(n0, c1) / len3(c1) < 0.5 {
                        continue 'cand;
                    }
                }
                for &t in &ring {
                    if tris[t].contains(&(u as u32)) {
                        alive_t[t] = false;
                    } else {
                        for w in tris[t].iter_mut() {
                            if *w as usize == v {
                                *w = u as u32;
                            }
                        }
                    }
                }
                for &t in &ring {
                    for &w in &tris[t] {
                        touched[w as usize] = true;
                    }
                }
                touched[v] = true;
                changed += 1;
                break;
            }
        }
        let mut keep = alive_t.iter();
        tris.retain(|_| *keep.next().unwrap());
        if changed == 0 {
            break;
        }
    }
}

/// A round stud on the top at `c` (mm).
fn stud(verts: &mut Vec<[f64; 3]>, tris: &mut Vec<[u32; 3]>, c: [f64; 2], h: f64) {
    const SEG: usize = 12;
    let (r, top) = (2.4, h + 1.8);
    let base = verts.len() as u32;
    for k in 0..SEG {
        let a = std::f64::consts::TAU * k as f64 / SEG as f64;
        let (x, y) = (c[0] + r * a.cos(), c[1] + r * a.sin());
        verts.push([x, y, h]);
        verts.push([x, y, top]);
    }
    verts.push([c[0], c[1], top]);
    let centre = base + 2 * SEG as u32;
    for k in 0..SEG as u32 {
        let n = (k + 1) % SEG as u32;
        let (b0, t0, b1, t1) = (base + 2 * k, base + 2 * k + 1, base + 2 * n, base + 2 * n + 1);
        tris.push([b0, b1, t1]);
        tris.push([b0, t1, t0]);
        tris.push([t0, t1, centre]);
    }
}

/// Volume enclosed by a closed mesh (any frame; units cubed).
pub fn mesh_volume(m: &DetailMesh) -> f64 {
    let p = &m.positions;
    m.indices
        .chunks(3)
        .filter(|t| t.len() == 3)
        .map(|t| {
            let (a, b, c) = (p[t[0] as usize], p[t[1] as usize], p[t[2] as usize]);
            let a = a.map(|v| v as f64);
            let b = b.map(|v| v as f64);
            let c = c.map(|v| v as f64);
            (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0]) + a[2] * (b[0] * c[1] - b[1] * c[0])) / 6.0
        })
        .sum::<f64>()
        .abs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BuildOptions, BuildResult, build};
    use sf_core::Fill;
    use sf_spec::Spec;
    use std::time::Instant;

    fn printed(json: &str, res: u32) -> BuildResult {
        let opts = BuildOptions { resolution: Some(res), print: true, ..Default::default() };
        build(&Spec::from_json(json).unwrap(), &opts, None).unwrap()
    }

    fn bricks(r: &BuildResult) -> Vec<(usize, &Placement)> {
        r.model
            .placements
            .iter()
            .enumerate()
            .filter(|(_, p)| matches!(p.custom.as_deref().map(|c| &c.kind), Some(sf_core::CustomKind::Brick)))
            .collect()
    }

    /// Does any face of brick `i` touch nothing (top, sides, or bottom off the ground)?
    fn exposed(r: &BuildResult, occ: &Occupancy, i: usize, p: &Placement) -> bool {
        let top = p.top();
        p.cells().any(|c| {
            let other = |q: Cell| occ.owner.get(&q).is_some_and(|&o| o != i);
            let free = |q: Cell| !occ.owner.contains_key(&q);
            (c.y == top - 1 && free(Cell::new(c.x, top, c.z)))
                || (c.y == p.origin.y && p.origin.y > 0 && free(Cell::new(c.x, c.y - 1, c.z)))
                || [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().any(|&(dx, dz)| {
                    let q = Cell::new(c.x + dx, c.y, c.z + dz);
                    !p.cells().any(|m| m == q) && !other(q)
                })
        }) && !r.model.placements.is_empty()
    }

    const SNOWMAN: &str = r#"{"title":"snowman","shapes":[
        {"shape":"ellipsoid","center":[0,4,0],"size":[8,8,8],"color":"White"},
        {"shape":"ellipsoid","center":[0,10,0],"size":[5.5,5.5,5.5],"color":"White"},
        {"shape":"cone","center":[0,10,-3.2],"size":[1,1,2],"axis":"z","rotate":[180,0,0],"color":"Orange"}
    ]}"#;

    #[test]
    fn a_curved_snowman() {
        let t = Instant::now();
        let r = printed(SNOWMAN, 20);
        let took = t.elapsed();
        let s = r.report.printed.as_ref().unwrap();
        println!("snowman at 20 studs: {} bricks, {} curved, {} need supports, built in {took:.2?}", s.bricks, s.curved, s.need_supports);
        println!("{}", r.report.to_text());
        let occ = Occupancy::new(&r.model.placements, &r.model.meshes);
        let mut hidden = 0;
        for (i, p) in bricks(&r) {
            let def = p.custom.as_deref().unwrap();
            assert_eq!(def.trim.is_some(), exposed(&r, &occ, i, p), "{} at {:?}", def.id, p.origin);
            if def.trim.is_none() {
                hidden += 1;
                continue;
            }
            let trim = def.trim.as_deref().unwrap();
            // A mesh exists for it.
            assert_eq!(def.mesh.as_deref(), Some(format!("trim:{}", def.id).as_str()));
            assert!(r.model.meshes.contains_key(def.mesh.as_deref().unwrap()));
            // Studs only where something sits; those tops are protected and kept flat.
            let top = p.top();
            let h = 3.2 * def.plates as f64;
            let field = Field { trim, cols: def.cells.iter().map(|&(x, z)| (x as i32, z as i32)).collect(), h };
            for &(lx, lz) in &def.studs {
                assert!(occ.owner.contains_key(&Cell::new(p.origin.x + lx as i32, top, p.origin.z + lz as i32)), "stud on an exposed top");
                for (fx, fy) in [(0.5, 0.5), (0.1, 0.1), (0.9, 0.9), (0.1, 0.9)] {
                    let q = [8.0 * (lx as f64 + fx), 8.0 * (lz as f64 + fy), h - 0.3];
                    assert!(field.eval(q) < 0.0, "{}: covered top at {q:?} was trimmed", def.id);
                }
            }
            for (x, z) in p.top_studs() {
                assert!(occ.owner.contains_key(&Cell::new(x, top, z)));
            }
            // Volume: no more than the block, at least half of what the voxels said.
            let m = trimmed_mesh(def, trim);
            let block = def.cells.len() as f64 * 64.0 * h;
            let mut voxels = 0.0;
            for c in p.cells() {
                if r.compiled.grid.get(c) != Fill::Empty {
                    voxels += 64.0 * 3.2;
                }
            }
            assert!(m.volume_mm3 <= block * 1.02, "{}: {:.0} > block {block:.0}", def.id, m.volume_mm3);
            assert!(m.volume_mm3 >= 0.5 * voxels, "{}: {:.0} < half of {voxels:.0}", def.id, m.volume_mm3);
        }
        assert!(hidden > 0, "expected some fully hidden bricks");
        assert!(s.curved > 0);
        assert!(r.report.to_text().contains("Curved: "));
        assert!(r.analysis.overlaps.is_empty());
        assert!(took.as_secs_f64() < 5.0, "too slow: {took:?}");
    }

    #[test]
    fn a_ball_overhangs_below_a_dome_does_not() {
        let ball = printed(r#"{"shapes":[{"shape":"ellipsoid","center":[0,4,0],"size":[8,8,8],"color":"Red"}]}"#, 12);
        let centre_y = ball.compiled.grid.dims[1] / 2;
        let low = bricks(&ball)
            .into_iter()
            .filter(|(_, p)| p.top() <= centre_y)
            .filter(|(_, p)| p.custom.as_ref().unwrap().trim.as_ref().is_some_and(|t| t.overhang))
            .count();
        assert!(low > 0, "no overhang flagged under a ball\n{}", ball.report.to_text());
        let dome = printed(
            r#"{"shapes":[{"shape":"ellipsoid","center":[0,0,0],"size":[8,8,8],"color":"Red"},
            {"shape":"box","center":[0,-2,0],"size":[10,4,10],"op":"subtract"}]}"#,
            12,
        );
        let flagged: Vec<String> = bricks(&dome)
            .into_iter()
            .filter(|(_, p)| p.custom.as_ref().unwrap().trim.as_ref().is_some_and(|t| t.overhang))
            .map(|(_, p)| format!("{} at {:?}", p.part, p.origin))
            .collect();
        assert!(flagged.is_empty(), "a dome needs no supports: {flagged:?}");
        assert!(dome.report.printed.as_ref().unwrap().curved > 0);
    }

    #[test]
    fn curves_off_or_relief_leave_plain_bricks() {
        let off = build(
            &Spec::from_json(SNOWMAN).unwrap(),
            &BuildOptions { resolution: Some(12), print: true, curves: false, ..Default::default() },
            None,
        )
        .unwrap();
        assert!(off.model.placements.iter().all(|p| p.custom.as_ref().is_none_or(|c| c.trim.is_none())));
        assert!(off.model.meshes.is_empty());
        let relief = printed(r#"{"mode":"relief","relief":{"rows":["aab","abb"],"legend":{"a":"Red","b":"Blue"}}}"#, 4);
        assert!(relief.model.placements.iter().all(|p| p.custom.as_ref().is_none_or(|c| c.trim.is_none())));
    }

    #[test]
    fn trim_shapes_land_where_the_voxels_are() {
        // Every filled cell's centre of a sphere is (nearly) inside the trim's solid.
        let r = printed(r#"{"shapes":[{"shape":"ellipsoid","center":[0,4,0],"size":[8,8,8],"color":"Red"}]}"#, 10);
        for (_, p) in bricks(&r) {
            let def = p.custom.as_deref().unwrap();
            let shapes = shapes_for(&r.compiled.surface, p.origin, def.size(), def.plates);
            assert!(!shapes.is_empty());
            for c in p.cells() {
                if r.compiled.grid.get(c) == Fill::Empty {
                    continue;
                }
                let q = [8.0 * (c.x - p.origin.x) as f64 + 4.0, 8.0 * (c.z - p.origin.z) as f64 + 4.0, 3.2 * (c.y - p.origin.y) as f64 + 1.6];
                let d = shapes.iter().map(|s| sd_shape(s, q)).fold(f64::INFINITY, f64::min);
                assert!(d < 4.0, "filled cell {c:?} is {d:.2} mm outside the sphere");
            }
        }
    }
}

