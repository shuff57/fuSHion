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
//! Contacts are trimmed to the shape too: a stud contact between two trimmed bricks keeps
//! its protect boxes only where `S`, grown by [`CONTACT_MARGIN`], covers them (see
//! [`contacts`]). Elsewhere the stud and the socket go and the region is cut like the rest,
//! so a sloped side doesn't step at every course. The engine restores the best contacts of
//! any part this would cut loose (see `print_pack`).
//!
//! The base follows the outline too (see [`ground_foot`]): where `S` reaches the floor it
//! stands on its own flat bottom; only where it doesn't (a ball, a rounded belly) does a
//! ground column get a flat pad, the size of `S`'s section at the pad's top.
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
/// How high the socket hollow is protected: a stud's height (1.8 mm) plus clearance. Above
/// it the hollow may close (sf-custom fills it wherever the cut surface comes near), so a
/// curved brick keeps its sockets without staying a full-height box.
const SOCKET_DEPTH: f64 = 2.2;
/// Spec primitives this close to a brick (mm) are kept in its trim.
const NEAR: f64 = 8.0;
/// A contact's protect boxes are kept when `S` grown by this much covers them, mm.
pub const CONTACT_MARGIN: f64 = 0.4;
/// Sample spacing when testing boxes against `S`, mm.
const CONTACT_STEP: f64 = 1.0;
/// Grid of a ground pad (twice the contact margin, so its edge stays near `S`'s outline), mm.
const PAD_STEP: f64 = 0.8;
/// Height just above the floor where `S` counts as standing on it, mm.
const FLOOR_Z: f64 = 0.05;
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
    surface
        .iter()
        .filter(|s| !(0..3).any(|k| s.aabb.0[k] > hi[k] + NEAR || s.aabb.1[k] < lo[k] - NEAR))
        .filter_map(|s| place(s, lo))
        .collect()
}

/// `s` in the frame of a brick whose bottom corner is at grid-mm `lo` (X, Y up, Z).
fn place(s: &SurfaceShape, lo: [f64; 3]) -> Option<TrimShape> {
    {
        // Brick frame: x = X − 8·ox, y = Z − 8·oz, z = Y − 3.2·oy.
        let l: M3 = [s.m[0], s.m[2], s.m[1]];
        let t = [s.t[0] - lo[0], s.t[2] - lo[2], s.t[1] - lo[1]];
        let d = det(&l);
        if d.abs() < 1e-12 {
            return None;
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
            TrimKind::Prism { points, height } => {
                TrimKind::Prism { points: points.iter().map(|p| [p[0] * sigma * fx, p[1] * sigma]).collect(), height: height * sigma }
            }
        };
        Some(TrimShape { op: s.op, kind, rotation: r, translation: t })
    }
}

/// The model's solid `S` in the model frame: mm, `x = 8·gx`, `y = 8·gz`, `z = 3.2·gy` for
/// grid position `(gx, gy, gz)` (a brick's print frame, moved to the grid origin).
pub struct Solid {
    /// Each shape with its bounding box in the model frame.
    shapes: Vec<(TrimShape, [f64; 3], [f64; 3])>,
}

impl Solid {
    pub fn new(surface: &[SurfaceShape]) -> Self {
        let shapes = surface
            .iter()
            .filter_map(|s| {
                let t = place(s, [0.0; 3])?;
                let (lo, hi) = s.aabb;
                Some((t, [lo[0], lo[2], lo[1]], [hi[0], hi[2], hi[1]]))
            })
            .collect();
        Solid { shapes }
    }

    /// Samples the part of brick-frame box `b` inside placement `p`'s block (every
    /// [`CONTACT_STEP`] mm, faces included). Returns `(samples, within margin of S, inside S)`.
    pub fn sample_box(&self, p: &Placement, b: &TrimBox, margin: f64) -> (usize, usize, usize) {
        self.sample(p, b, margin, false)
    }

    /// Does some of brick-frame box `b`, in placement `p`'s block, lie outside `S` grown by
    /// `margin` (a ledge)?
    pub fn reaches_outside(&self, p: &Placement, b: &TrimBox, margin: f64) -> bool {
        let (n, within, _) = self.sample(p, b, margin, true);
        within < n
    }

    /// `S`'s signed distance at model-frame point `q` (mm).
    pub fn sd(&self, q: [f64; 3]) -> f64 {
        solid_sd(self.shapes.iter().map(|(s, _, _)| s), q)
    }

    /// `S`'s bounding box in the model frame (mm), over its added shapes.
    pub fn bounds(&self) -> Option<([f64; 3], [f64; 3])> {
        let mut it = self.shapes.iter().filter(|(s, _, _)| s.op == TrimOp::Add).map(|(_, a, b)| (*a, *b));
        let (mut lo, mut hi) = it.next()?;
        for (a, b) in it {
            for k in 0..3 {
                lo[k] = lo[k].min(a[k]);
                hi[k] = hi[k].max(b[k]);
            }
        }
        Some((lo, hi))
    }

    /// `S`'s signed distance at brick-frame points of `p`, using only shapes within `margin`
    /// of the box `(lo, hi)` around them (exact for "within margin" and "inside").
    fn sd_points(&self, p: &Placement, lo: [f64; 3], hi: [f64; 3], margin: f64, pts: &[[f64; 3]]) -> Vec<f64> {
        let off = [8.0 * p.origin.x as f64, 8.0 * p.origin.z as f64, 3.2 * p.origin.y as f64];
        let near: Vec<&TrimShape> = self
            .shapes
            .iter()
            .filter(|(_, a, z)| (0..3).all(|k| a[k] <= hi[k] + off[k] + margin && z[k] >= lo[k] + off[k] - margin))
            .map(|(s, _, _)| s)
            .collect();
        pts.iter().map(|q| solid_sd(near.iter().copied(), [q[0] + off[0], q[1] + off[1], q[2] + off[2]])).collect()
    }

    /// How far (mm) the part of brick-frame box `b` in `p`'s block reaches outside `S` at most.
    pub fn max_outside(&self, p: &Placement, b: &TrimBox) -> f64 {
        let mut worst = f64::NEG_INFINITY;
        // Cull generously so distances outside are measured, not just their sign.
        self.each_sample(p, b, 20.0, &mut |d| {
            worst = worst.max(d);
            true
        });
        worst
    }

    fn sample(&self, p: &Placement, b: &TrimBox, margin: f64, stop_outside: bool) -> (usize, usize, usize) {
        let (mut total, mut within, mut inside) = (0, 0, 0);
        self.each_sample(p, b, margin, &mut |d| {
            total += 1;
            within += (d <= margin) as usize;
            inside += (d <= 0.0) as usize;
            !(stop_outside && d > margin)
        });
        (total, within, inside)
    }

    /// Calls `f` with `S`'s signed distance at each sample of `b` in `p`'s block, until it
    /// returns false. Shapes farther than `margin` from the box are left out (they can't
    /// change whether a sample is within `margin`).
    fn each_sample(&self, p: &Placement, b: &TrimBox, margin: f64, f: &mut dyn FnMut(f64) -> bool) {
        let Some(def) = p.custom.as_deref() else { return };
        let cols: HashSet<(i32, i32)> = def.cells.iter().map(|&(x, z)| (x as i32, z as i32)).collect();
        let [sx, sz] = def.size();
        let h = 3.2 * def.plates as f64;
        let lo = [b.min[0].max(0.0), b.min[1].max(0.0), b.min[2].max(0.0)];
        let hi = [b.max[0].min(8.0 * sx as f64), b.max[1].min(8.0 * sz as f64), b.max[2].min(h)];
        if (0..3).any(|k| hi[k] < lo[k]) {
            return;
        }
        let off = [8.0 * p.origin.x as f64, 8.0 * p.origin.z as f64, 3.2 * p.origin.y as f64];
        let (mlo, mhi) = ([0, 1, 2].map(|k| lo[k] + off[k]), [0, 1, 2].map(|k| hi[k] + off[k]));
        // Shapes farther than the margin can't change "within margin" or "inside".
        let near: Vec<&TrimShape> = self
            .shapes
            .iter()
            .filter(|(_, a, z)| (0..3).all(|k| a[k] <= mhi[k] + margin && z[k] >= mlo[k] - margin))
            .map(|(s, _, _)| s)
            .collect();
        let n = [0, 1, 2].map(|k| ((hi[k] - lo[k]) / CONTACT_STEP).ceil().max(1.0) as usize + 1);
        for i in 0..n[0] {
            for j in 0..n[1] {
                for k in 0..n[2] {
                    let q = [0, 1, 2].map(|a| {
                        let t = [i, j, k][a] as f64 / (n[a] - 1) as f64;
                        lo[a] + (hi[a] - lo[a]) * t
                    });
                    // Only points in the block (a polyomino's bbox has gaps).
                    let col = (((q[0] - 1e-9).max(0.0) / 8.0) as i32, ((q[1] - 1e-9).max(0.0) / 8.0) as i32);
                    if !cols.contains(&col) {
                        continue;
                    }
                    let d = solid_sd(near.iter().copied(), [q[0] + off[0], q[1] + off[1], q[2] + off[2]]);
                    if !f(d) {
                        return;
                    }
                }
            }
        }
    }
}

/// `S` from shapes in order: `Add` unions, `Subtract` removes, from empty.
fn solid_sd<'a>(shapes: impl Iterator<Item = &'a TrimShape>, p: [f64; 3]) -> f64 {
    let mut d = f64::INFINITY;
    for s in shapes {
        let ds = sd_shape(s, p);
        d = match s.op {
            TrimOp::Add => d.min(ds),
            TrimOp::Subtract => d.max(-ds),
        };
    }
    d
}

/// The protect box of a column's top (a part sits on it): the top slab and the stud room,
/// reaching past the brick's outer faces. Brick frame; `cols` are the brick's local columns.
fn top_box(cols: &HashSet<(i32, i32)>, x: i32, z: i32, h: f64) -> TrimBox {
    let [x0, x1, y0, y1, ex0, ex1, ey0, ey1] = column_frame(cols, x, z);
    TrimBox { min: [x0 - ex0, y0 - ey0, h - TOP_KEEP], max: [x1 + ex1, y1 + ey1, h + STUD_ROOM] }
}

/// The protect boxes of a column's bottom (it rests on something): the bottom band and the
/// socket hollow.
fn bottom_boxes(cols: &HashSet<(i32, i32)>, x: i32, z: i32, h: f64) -> [TrimBox; 2] {
    let [x0, x1, y0, y1, ex0, ex1, ey0, ey1] = column_frame(cols, x, z);
    let inset = |e: f64| if e > 0.0 { WALL } else { 0.0 };
    let top = (h - TOP).clamp(REACH, SOCKET_DEPTH);
    [
        TrimBox { min: [x0 - ex0, y0 - ey0, -REACH], max: [x1 + ex1, y1 + ey1, REACH] },
        TrimBox { min: [x0 + inset(ex0), y0 + inset(ey0), 0.0], max: [x1 - inset(ex1), y1 - inset(ey1), top] },
    ]
}

/// How a ground column of a trimmed brick meets the floor (see [`ground_foot`]).
#[derive(Clone, Debug, Default)]
pub struct GroundFoot {
    /// Protect boxes to keep, brick frame.
    pub boxes: Vec<TrimBox>,
    /// The column stands on the floor (`S` reaches it there, or a pad does): it keeps its
    /// socket and counts toward the footprint the model balances on.
    pub touches: bool,
    /// A flat pad was added where `S` doesn't reach the floor.
    pub pad: bool,
}

/// The plain bottom protection of local column `col` of brick `p` (band and socket hollow).
pub fn plain_bottom(p: &Placement, col: (i32, i32)) -> Vec<TrimBox> {
    let h = 3.2 * p.custom.as_ref().map_or(0, |d| d.plates) as f64;
    bottom_boxes(&local_cols(p), col.0, col.1, h).to_vec()
}

/// How ground column `col` (local) of trimmed brick `p` stands on the floor:
/// * `S` covers the plain bottom: keep it (a flat bottom inside the shape, with its socket);
/// * otherwise no box where `S` reaches the floor (it is flat there by itself, so a
///   cylinder's rim follows the circle), and a flat pad, from the floor up to the band's
///   top, only where `S` is present at that height but not at the floor (a ball's
///   underside). The pad is a union of [`PAD_STEP`] squares inside the column.
pub fn ground_foot(solid: &Solid, p: &Placement, col: (i32, i32)) -> GroundFoot {
    let plain = plain_bottom(p, col);
    if plain.iter().all(|b| !solid.reaches_outside(p, b, CONTACT_MARGIN)) {
        return GroundFoot { boxes: plain, touches: true, pad: false };
    }
    let cols = local_cols(p);
    let [x0, _, y0, _, ex0, ex1, ey0, ey1] = column_frame(&cols, col.0, col.1);
    let n = (8.0 / PAD_STEP).round() as usize;
    let centre = |i: usize| (i as f64 + 0.5) * PAD_STEP;
    let mut pts = Vec::with_capacity(2 * n * n);
    for j in 0..n {
        for i in 0..n {
            pts.push([x0 + centre(i), y0 + centre(j), FLOOR_Z]);
            pts.push([x0 + centre(i), y0 + centre(j), REACH]);
        }
    }
    let d = solid.sd_points(p, [x0, y0, 0.0], [x0 + 8.0, y0 + 8.0, REACH], CONTACT_MARGIN, &pts);
    let (mut touches, mut runs) = (false, Vec::<(usize, usize, usize, usize)>::new());
    for j in 0..n {
        let mut run: Option<usize> = None;
        for i in 0..=n {
            let pad = i < n && {
                let (floor, top) = (d[2 * (j * n + i)], d[2 * (j * n + i) + 1]);
                touches |= floor <= 0.0;
                top <= 0.0 && floor > CONTACT_MARGIN
            };
            match (pad, run) {
                (true, None) => run = Some(i),
                (false, Some(a)) => {
                    // Rows `j0..=j1` of squares `a..i`; grow the previous row's rectangle when
                    // it spans the same squares.
                    match runs.iter_mut().find(|r| r.0 == a && r.1 == i && r.3 + 1 == j) {
                        Some(r) => r.3 = j,
                        None => runs.push((a, i, j, j)),
                    }
                    run = None;
                }
                _ => {}
            }
        }
    }
    // Reach past the brick's outer faces like other protect boxes.
    let boxes: Vec<TrimBox> = runs
        .iter()
        .map(|&(a, b, j0, j1)| {
            let xa = x0 + a as f64 * PAD_STEP - if a == 0 { ex0 } else { 0.0 };
            let xb = x0 + b as f64 * PAD_STEP + if b == n { ex1 } else { 0.0 };
            let ya = y0 + j0 as f64 * PAD_STEP - if j0 == 0 { ey0 } else { 0.0 };
            let yb = y0 + (j1 + 1) as f64 * PAD_STEP + if j1 + 1 == n { ey1 } else { 0.0 };
            TrimBox { min: [xa, ya, -REACH], max: [xb, yb, REACH] }
        })
        .collect();
    let pad = !boxes.is_empty();
    GroundFoot { boxes, touches: touches || pad, pad }
}

/// A column's extent (mm) and how far boxes reach past each side (0 where the brick goes on).
fn column_frame(cols: &HashSet<(i32, i32)>, x: i32, z: i32) -> [f64; 8] {
    let ext = |dx: i32, dz: i32| if cols.contains(&(x + dx, z + dz)) { 0.0 } else { REACH };
    let (x0, y0) = (8.0 * x as f64, 8.0 * z as f64);
    [x0, x0 + 8.0, y0, y0 + 8.0, ext(-1, 0), ext(1, 0), ext(0, -1), ext(0, 1)]
}

fn local_cols(p: &Placement) -> HashSet<(i32, i32)> {
    p.custom.as_deref().map(|d| d.cells.iter().map(|&(x, z)| (x as i32, z as i32)).collect()).unwrap_or_default()
}

/// One stud contact: part `upper` rests on part `lower` over grid column `col`.
#[derive(Clone, Debug)]
pub struct Contact {
    pub lower: usize,
    pub upper: usize,
    pub col: (i32, i32),
    /// One side isn't a trimmed brick (a detail, a hidden brick): the contact always stays.
    pub fixed: bool,
    /// `S` grown by [`CONTACT_MARGIN`] covers both sides' protect boxes.
    pub covered: bool,
    /// Fraction of the protect boxes inside `S` (to pick contacts to restore).
    pub score: f64,
}

/// `f` over `items` on all cores, results in order.
pub(crate) fn par_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 16);
    if items.len() < 2 * threads {
        return items.iter().map(&f).collect();
    }
    let f = &f;
    std::thread::scope(|s| {
        let handles: Vec<_> =
            items.chunks(items.len().div_ceil(threads)).map(|c| s.spawn(move || c.iter().map(f).collect::<Vec<R>>())).collect();
        handles.into_iter().flat_map(|h| h.join().expect("worker panicked")).collect()
    })
}

/// Every vertical contact between parts. `trimmed[i]`: part `i` is a brick that gets trimmed.
pub fn contacts(ps: &[Placement], occ: &Occupancy, trimmed: &[bool], solid: &Solid) -> Vec<Contact> {
    let mut found = Vec::new();
    let mut cells: Vec<(&Cell, &usize)> = occ.owner.iter().collect();
    cells.sort_unstable();
    for (c, &lower) in cells {
        let Some(&upper) = occ.owner.get(&Cell::new(c.x, c.y + 1, c.z)) else { continue };
        if upper == lower || ps[upper].origin.y != c.y + 1 {
            continue;
        }
        found.push((*c, lower, upper));
    }
    par_map(&found, |&(c, lower, upper)| {
        let fixed = !(trimmed[lower] && trimmed[upper]);
        let (mut covered, mut score) = (true, 1.0);
        if !fixed {
            let (lp, up) = (&ps[lower], &ps[upper]);
            let (lc, uc) = (local_cols(lp), local_cols(up));
            let lh = 3.2 * lp.custom.as_ref().map_or(0, |d| d.plates) as f64;
            let uh = 3.2 * up.custom.as_ref().map_or(0, |d| d.plates) as f64;
            let (lx, lz) = (c.x - lp.origin.x, c.z - lp.origin.z);
            let (ux, uz) = (c.x - up.origin.x, c.z - up.origin.z);
            let mut acc = (0, 0, 0);
            let mut add = |s: (usize, usize, usize)| acc = (acc.0 + s.0, acc.1 + s.1, acc.2 + s.2);
            add(solid.sample_box(lp, &top_box(&lc, lx, lz, lh), CONTACT_MARGIN));
            for b in bottom_boxes(&uc, ux, uz, uh) {
                add(solid.sample_box(up, &b, CONTACT_MARGIN));
            }
            covered = acc.1 == acc.0;
            score = if acc.0 == 0 { 0.0 } else { acc.2 as f64 / acc.0 as f64 };
        }
        Contact { lower, upper, col: (c.x, c.z), fixed, covered, score }
    })
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
                !cols.contains(&(x + dx, z + dz)) && (0..plates).any(|k| !occ.other(Cell::new(o.x + x + dx, o.y + k, o.z + z + dz), i))
            })
    })
}

/// Which contacts of a brick keep their protect boxes, by local column.
pub struct KeptContacts<'a> {
    /// The column's top keeps its slab and stud room (a part sits there and stays held).
    pub top: &'a dyn Fn((i32, i32)) -> bool,
    /// The column's bottom keeps its band and socket hollow.
    pub bottom: &'a dyn Fn((i32, i32)) -> bool,
    /// The protect boxes of a ground column (a brick on the floor).
    pub ground: &'a dyn Fn((i32, i32)) -> Vec<TrimBox>,
}

/// The trim for printed brick `i` (at `Rot(0)`), or `None` when nothing of it is exposed or
/// no spec shape reaches it. `trimmed(j)` says whether part `j` is a brick that is trimmed
/// too (no side slab is kept against it); `kept` which top and bottom contacts keep their
/// protect boxes (only asked about columns with a part above or below, or on the ground).
pub fn brick_trim(
    i: usize,
    p: &Placement,
    occ: &Occupancy,
    surface: &[SurfaceShape],
    trimmed: &dyn Fn(usize) -> bool,
    kept: &KeptContacts,
) -> Option<Trim> {
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
    let mut cols_sorted: Vec<(i32, i32)> = cols.iter().copied().collect();
    cols_sorted.sort_unstable();
    for &(x, z) in &cols_sorted {
        let (x0, x1) = (8.0 * x as f64, 8.0 * x as f64 + 8.0);
        let (y0, y1) = (8.0 * z as f64, 8.0 * z as f64 + 8.0);
        let (ex0, ex1, ey0, ey1) = (ext(x, z, -1, 0), ext(x, z, 1, 0), ext(x, z, 0, -1), ext(x, z, 0, 1));
        // Top.
        if occ.other(cell(x, plates, z), i) {
            if (kept.top)((x, z)) {
                protect.push(top_box(&cols, x, z, h));
            }
        } else {
            exposed = true;
        }
        // Bottom.
        if o.y == 0 {
            protect.extend((kept.ground)((x, z)));
        } else if occ.other(cell(x, -1, z), i) {
            if (kept.bottom)((x, z)) {
                protect.extend(bottom_boxes(&cols, x, z, h));
            }
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
        solid_sd(self.trim.shapes.iter(), p)
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
    /// Body triangles (no studs), and how many are slivers (an angle under 5°).
    pub triangles: usize,
    pub slivers: usize,
    /// Triangles whose centre lies more than [`CHAMFER_MM`] off the true surface (they cut
    /// a crease or corner: notches and gaps where bricks meet), and the worst distance, mm.
    pub chamfered: usize,
    pub max_off_mm: f64,
}

/// See [`TrimmedMesh::chamfered`].
pub const CHAMFER_MM: f64 = 0.2;

/// Slivers, chamfered triangles and the worst centre distance off the surface.
fn quality(verts: &[[f64; 3]], tris: &[[u32; 3]], field: &Field) -> (usize, usize, f64) {
    let (mut slivers, mut chamfered, mut worst) = (0, 0, 0.0f64);
    let sin5 = 5f64.to_radians().sin();
    for &t in tris {
        let p = t.map(|i| verts[i as usize]);
        let e = [0, 1, 2].map(|k| {
            let (a, b) = (p[k], p[(k + 1) % 3]);
            len3([b[0] - a[0], b[1] - a[1], b[2] - a[2]])
        });
        // The smallest angle's sine: 2·area / (product of its two sides), sides around it the two longest.
        let area2 = len3(cross(verts, t));
        let mut s = e;
        s.sort_by(f64::total_cmp);
        if s[1] * s[2] <= 0.0 || area2 / (s[1] * s[2]) < sin5 {
            slivers += 1;
        }
        let c = [0, 1, 2].map(|k| (p[0][k] + p[1][k] + p[2][k]) / 3.0);
        let off = field.eval(c).abs();
        worst = worst.max(off);
        chamfered += (off > CHAMFER_MM) as usize;
    }
    (slivers, chamfered, worst)
}

/// Dual contouring of a [`Field`] on a grid of samples at `(i − ½)·step` (block faces fall
/// halfway between samples, and every brick samples the same lattice, so neighbours agree).
/// One vertex per cell the surface crosses, placed where the tangent planes at its edge
/// crossings meet (least squares, pulled a little toward their centroid, kept in the cell):
/// a brick face meeting the curved surface keeps its sharp crease, so neighbouring bricks
/// meet without notches. One quad per crossed edge, so the mesh is closed.
struct Dual<'a> {
    field: &'a Field<'a>,
    step: f64,
    n: [usize; 3],
    values: Vec<f64>,
    /// Crossed edges by (lower sample, axis): the crossing point and the surface normal.
    crossings: HashMap<(usize, usize), ([f64; 3], [f64; 3])>,
    cells: HashMap<usize, u32>,
    verts: Vec<[f64; 3]>,
}

impl<'a> Dual<'a> {
    fn new(field: &'a Field<'a>, size: [u8; 2], h: f64, step: f64) -> Self {
        let n = [
            (8.0 * size[0] as f64 / step).round() as usize + 2,
            (8.0 * size[1] as f64 / step).round() as usize + 2,
            (h / step).round() as usize + 2,
        ];
        Dual {
            field,
            step,
            n,
            values: vec![f64::NAN; n[0] * n[1] * n[2]],
            crossings: HashMap::new(),
            cells: HashMap::new(),
            verts: Vec::new(),
        }
    }

    fn idx(&self, s: [usize; 3]) -> usize {
        (s[2] * self.n[1] + s[1]) * self.n[0] + s[0]
    }

    fn pos(&self, s: [usize; 3]) -> [f64; 3] {
        s.map(|v| (v as f64 - 0.5) * self.step)
    }

    fn value(&mut self, s: [usize; 3]) -> f64 {
        let i = self.idx(s);
        if self.values[i].is_nan() {
            self.values[i] = self.field.eval(self.pos(s));
        }
        self.values[i]
    }

    fn normal(&self, p: [f64; 3]) -> [f64; 3] {
        let e = 0.02;
        let g = [0, 1, 2].map(|k| {
            let (mut a, mut b) = (p, p);
            a[k] += e;
            b[k] -= e;
            self.field.eval(a) - self.field.eval(b)
        });
        let l = len3(g);
        if l < 1e-12 { [0.0; 3] } else { g.map(|v| v / l) }
    }

    /// The crossing on the edge from sample `s` along `axis`, if the surface crosses it:
    /// found by bisection on the field (not linear interpolation).
    fn crossing(&mut self, s: [usize; 3], axis: usize) -> Option<([f64; 3], [f64; 3])> {
        let mut t = s;
        t[axis] += 1;
        if t[axis] >= self.n[axis] {
            return None;
        }
        let (va, vb) = (self.value(s), self.value(t));
        if (va < 0.0) == (vb < 0.0) {
            return None;
        }
        let key = (self.idx(s), axis);
        if let Some(c) = self.crossings.get(&key) {
            return Some(*c);
        }
        let (pa, pb) = (self.pos(s), self.pos(t));
        let (mut lo, mut hi) = (0.0f64, 1.0f64); // lo on va's side
        let at = |u: f64| [0, 1, 2].map(|k| pa[k] + (pb[k] - pa[k]) * u);
        for _ in 0..10 {
            let mid = (lo + hi) / 2.0;
            if (self.field.eval(at(mid)) < 0.0) == (va < 0.0) { lo = mid } else { hi = mid }
        }
        let p = at((lo + hi) / 2.0);
        let c = (p, self.normal(p));
        self.crossings.insert(key, c);
        Some(c)
    }

    /// The vertex of the cell whose lowest sample is `c`.
    fn cell_vertex(&mut self, c: [usize; 3]) -> u32 {
        let key = self.idx(c);
        if let Some(&v) = self.cells.get(&key) {
            return v;
        }
        let mut pts: Vec<([f64; 3], [f64; 3])> = Vec::new();
        for axis in 0..3 {
            let (b, d) = ((axis + 1) % 3, (axis + 2) % 3);
            for (db, dd) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let mut s = c;
                s[b] += db;
                s[d] += dd;
                if let Some(x) = self.crossing(s, axis) {
                    pts.push(x);
                }
            }
        }
        let lo = self.pos(c);
        let hi = lo.map(|v| v + self.step);
        let m = pts.iter().fold([0.0; 3], |a, (p, _)| [a[0] + p[0], a[1] + p[1], a[2] + p[2]]).map(|v| v / pts.len().max(1) as f64);
        // Minimise Σ (nᵢ·(x − pᵢ))² + λ|x − m|², relative to m.
        let lambda = 0.05;
        let mut a = [[0.0f64; 3]; 3];
        let mut r = [0.0f64; 3];
        for (p, nrm) in &pts {
            let d = nrm[0] * (p[0] - m[0]) + nrm[1] * (p[1] - m[1]) + nrm[2] * (p[2] - m[2]);
            for i in 0..3 {
                for j in 0..3 {
                    a[i][j] += nrm[i] * nrm[j];
                }
                r[i] += nrm[i] * d;
            }
        }
        for (i, row) in a.iter_mut().enumerate() {
            row[i] += lambda;
        }
        let x = solve3(&a, r).map_or(m, |dx| [m[0] + dx[0], m[1] + dx[1], m[2] + dx[2]]);
        let v = [0, 1, 2].map(|k| x[k].clamp(lo[k], hi[k]));
        self.verts.push(v);
        let id = (self.verts.len() - 1) as u32;
        self.cells.insert(key, id);
        id
    }

    fn contour(mut self) -> (Vec<[f64; 3]>, Vec<[u32; 3]>) {
        let n = self.n;
        let mut tris = Vec::new();
        const B: usize = 6; // block side in samples: skip blocks the surface can't reach
        let half_diag = (B as f64) * self.step * 3f64.sqrt() / 2.0;
        for bk in (0..n[2] - 1).step_by(B) {
            for bj in (0..n[1] - 1).step_by(B) {
                for bi in (0..n[0] - 1).step_by(B) {
                    let centre = [(bi + B / 2).min(n[0] - 1), (bj + B / 2).min(n[1] - 1), (bk + B / 2).min(n[2] - 1)];
                    if self.value(centre).abs() > 2.0 * half_diag {
                        continue;
                    }
                    for k in bk..(bk + B).min(n[2]) {
                        for j in bj..(bj + B).min(n[1]) {
                            for i in bi..(bi + B).min(n[0]) {
                                for axis in 0..3 {
                                    self.quad([i, j, k], axis, &mut tris);
                                }
                            }
                        }
                    }
                }
            }
        }
        (self.verts, tris)
    }

    /// The quad of the edge from sample `s` along `axis`, if it is crossed.
    fn quad(&mut self, s: [usize; 3], axis: usize, tris: &mut Vec<[u32; 3]>) {
        let (b, d) = ((axis + 1) % 3, (axis + 2) % 3);
        if s[b] == 0 || s[d] == 0 || s[b] + 1 >= self.n[b] || s[d] + 1 >= self.n[d] {
            return; // the outer layer is outside the block: never crossed
        }
        if self.crossing(s, axis).is_none() {
            return;
        }
        let inside_first = self.value(s) < 0.0;
        // Cells around the edge, counter-clockwise seen from +axis.
        let mut q = [0u32; 4];
        for (slot, (db, dd)) in [(0, 0), (1, 0), (1, 1), (0, 1)].into_iter().enumerate() {
            let mut c = s;
            c[b] = s[b] - 1 + db;
            c[d] = s[d] - 1 + dd;
            q[slot] = self.cell_vertex(c);
        }
        if !inside_first {
            q.swap(1, 3);
        }
        let p = q.map(|v| self.verts[v as usize]);
        let dist = |a: [f64; 3], b: [f64; 3]| len3([a[0] - b[0], a[1] - b[1], a[2] - b[2]]);
        // Split along the shorter diagonal.
        if dist(p[0], p[2]) <= dist(p[1], p[3]) {
            tris.push([q[0], q[1], q[2]]);
            tris.push([q[0], q[2], q[3]]);
        } else {
            tris.push([q[0], q[1], q[3]]);
            tris.push([q[1], q[2], q[3]]);
        }
    }
}

/// Solves `a·x = r` for a 3 × 3 system, `None` if singular.
fn solve3(a: &[[f64; 3]; 3], r: [f64; 3]) -> Option<[f64; 3]> {
    let d = det(a);
    if d.abs() < 1e-12 {
        return None;
    }
    let col = |k: usize| {
        let mut m = *a;
        for i in 0..3 {
            m[i][k] = r[i];
        }
        det(&m) / d
    };
    Some([col(0), col(1), col(2)])
}

/// A display mesh of `block ∩ (S ∪ P)` for a trimmed brick (see [`TrimmedMesh`]).
pub fn trimmed_mesh(def: &CustomDef, trim: &Trim) -> TrimmedMesh {
    let cols: HashSet<(i32, i32)> = def.cells.iter().map(|&(x, z)| (x as i32, z as i32)).collect();
    let [sx, sz] = def.size();
    let h = 3.2 * def.plates as f64;
    let field = Field { trim, cols, h };
    let step = MESH_STEP;
    let (mut verts, mut tris) = Dual::new(&field, [sx, sz], h, step).contour();
    collapse_slivers(&verts, &mut tris, step / 2.0);
    simplify(&mut verts, &mut tris, step);
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
    let (slivers, chamfered, max_off_mm) = quality(&verts, &tris, &field);
    let triangles = tris.len();
    // Studs: plain round ones in the display (the print fuses them).
    for &(lx, lz) in &def.studs {
        stud(&mut verts, &mut tris, [8.0 * lx as f64 + 4.0, 8.0 * lz as f64 + 4.0], h);
    }
    // mm, print frame → LDU catalog frame: (x, y, z) ↦ 2.5·(x − W/2, H − z, y − D/2).
    let (w, d) = (8.0 * sx as f64, 8.0 * sz as f64);
    let positions =
        verts.iter().map(|v| [((v[0] - w / 2.0) * 2.5) as f32, ((h - v[2]) * 2.5) as f32, ((v[1] - d / 2.0) * 2.5) as f32]).collect();
    // The map has determinant +1 times a positive scale, so windings stay outward.
    let indices = tris.iter().flatten().copied().collect();
    TrimmedMesh { mesh: DetailMesh { positions, indices }, overhang, volume_mm3, triangles, slivers, chamfered, max_off_mm }
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

/// Collapses the shortest edge (at most `max_len`) of every sliver where that keeps the
/// mesh closed and nothing flips or becomes a sliver itself (dual contouring leaves a few
/// where two cell vertices land close together).
fn collapse_slivers(verts: &[[f64; 3]], tris: &mut Vec<[u32; 3]>, max_len: f64) {
    for _pass in 0..4 {
        let mut vt: Vec<Vec<usize>> = vec![Vec::new(); verts.len()];
        for (ti, t) in tris.iter().enumerate() {
            for &v in t {
                vt[v as usize].push(ti);
            }
        }
        let mut alive = vec![true; tris.len()];
        let mut touched = vec![false; verts.len()];
        let mut changed = 0;
        for ti in 0..tris.len() {
            if !alive[ti] || !is_sliver(verts, tris[ti]) {
                continue;
            }
            let t = tris[ti];
            let (a, b) = (0..3)
                .map(|k| (t[k] as usize, t[(k + 1) % 3] as usize))
                .min_by(|&(a, b), &(c, d)| dist3(verts[a], verts[b]).total_cmp(&dist3(verts[c], verts[d])))
                .expect("three edges");
            if dist3(verts[a], verts[b]) > max_len || touched[a] || touched[b] {
                continue;
            }
            for (from, to) in [(a, b), (b, a)] {
                let ring: Vec<usize> = vt[from].iter().copied().filter(|&r| alive[r]).collect();
                let nb = |v: usize| -> HashSet<u32> {
                    vt[v].iter().filter(|&&r| alive[r]).flat_map(|&r| tris[r]).filter(|&w| w as usize != v).collect()
                };
                let (nf, nt) = (nb(from), nb(to));
                if nf.iter().filter(|w| nt.contains(w)).count() != 2 {
                    continue;
                }
                let ok = ring.iter().all(|&r| {
                    let tri = tris[r];
                    if tri.contains(&(to as u32)) {
                        return true;
                    }
                    let moved = tri.map(|w| if w as usize == from { to as u32 } else { w });
                    let c1 = cross(verts, moved);
                    len3(c1) > 1e-9 && dot(normal(verts, tri), c1) / len3(c1) >= 0.5 && !is_sliver(verts, moved)
                });
                if !ok {
                    continue;
                }
                for &r in &ring {
                    if tris[r].contains(&(to as u32)) {
                        alive[r] = false;
                    } else {
                        for w in tris[r].iter_mut() {
                            if *w as usize == from {
                                *w = to as u32;
                            }
                        }
                    }
                    for &w in &tris[r] {
                        touched[w as usize] = true;
                    }
                }
                touched[from] = true;
                changed += 1;
                break;
            }
        }
        let mut keep = alive.iter();
        tris.retain(|_| *keep.next().expect("one flag per triangle"));
        if changed == 0 {
            break;
        }
    }
}

fn dist3(a: [f64; 3], b: [f64; 3]) -> f64 {
    len3([b[0] - a[0], b[1] - a[1], b[2] - a[2]])
}

/// Does triangle `t` have an angle under 5°?
fn is_sliver(verts: &[[f64; 3]], t: [u32; 3]) -> bool {
    let p = t.map(|i| verts[i as usize]);
    let mut e = [0, 1, 2].map(|k| {
        let (a, b) = (p[k], p[(k + 1) % 3]);
        len3([b[0] - a[0], b[1] - a[1], b[2] - a[2]])
    });
    e.sort_by(f64::total_cmp);
    // The smallest angle is opposite the shortest side, between the two longer ones.
    e[1] * e[2] <= 0.0 || len3(cross(verts, t)) / (e[1] * e[2]) < 5f64.to_radians().sin()
}

/// Collapses edges where the surface is flat (any length) or gently curved (short edges
/// only), keeping the mesh closed and its faces' orientation.
fn simplify(verts: &mut [[f64; 3]], tris: &mut Vec<[u32; 3]>, step: f64) {
    const FLAT: f64 = 0.99995;
    // Gently curved: an edge of 4.8 mm on a 25 mm radius sags 0.1 mm, invisible in a view.
    const GENTLE: f64 = 0.99;
    let gentle_edge = 3.0 * step;
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
            let min_dot = cs.iter().filter(|c| len3(**c) > 1e-3 * total).map(|c| dot(c.map(|x| x / len3(*c)), avg)).fold(1.0f64, f64::min);
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
                    if len3(c1) < 1e-9 || dot(n0, c1) / len3(c1) < 0.5 || is_sliver(verts, moved) {
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

/// A round stud on the top at `c` (mm): a closed cylinder standing on the body, so the
/// display mesh has no open edges.
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
    verts.push([c[0], c[1], h]);
    let centre = base + 2 * SEG as u32;
    let bottom = centre + 1;
    for k in 0..SEG as u32 {
        let n = (k + 1) % SEG as u32;
        let (b0, t0, b1, t1) = (base + 2 * k, base + 2 * k + 1, base + 2 * n, base + 2 * n + 1);
        tris.push([b0, b1, t1]);
        tris.push([b0, t1, t0]);
        tris.push([t0, t1, centre]);
        tris.push([b1, b0, bottom]);
    }
}

/// Edges used by exactly one triangle (0 for a closed, manifold mesh).
pub fn open_edges(m: &DetailMesh) -> usize {
    let mut count: HashMap<(u32, u32), i32> = HashMap::new();
    for t in m.indices.as_chunks::<3>().0 {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            *count.entry((a.min(b), a.max(b))).or_default() += 1;
        }
    }
    count.values().filter(|&&c| c == 1).count()
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
        let shape_grid = sf_spec::compile(
            &Spec::from_json(SNOWMAN).unwrap(),
            &sf_core::Palette::builtin(),
            &sf_spec::CompileOptions { resolution: Some(20), image: None },
        )
        .unwrap()
        .grid;
        assert_eq!(shape_grid.dims, r.compiled.grid.dims);
        let mut repaired = 0;
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
            assert_eq!(open_edges(&r.model.meshes[def.mesh.as_deref().unwrap()]), 0, "{}: display mesh not closed", def.id);
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
            // Volume: no more than the block, at least half of what the shape's own voxels
            // said (cells the repair added to hold a feature lie outside the shape and are
            // trimmed away where no contact needs them).
            let m = trimmed_mesh(def, trim);
            let block = def.cells.len() as f64 * 64.0 * h;
            let mut voxels = 0.0;
            for c in p.cells() {
                if shape_grid.get(c) != Fill::Empty {
                    voxels += 64.0 * 3.2;
                }
            }
            if r.compiled.grid.get(p.origin) != shape_grid.get(p.origin) {
                repaired += 1;
            }
            assert!(m.volume_mm3 <= block * 1.02, "{}: {:.0} > block {block:.0}", def.id, m.volume_mm3);
            assert!(m.volume_mm3 >= 0.5 * voxels, "{}: {:.0} < half of {voxels:.0}", def.id, m.volume_mm3);
        }
        // (Every snowman brick touches the surface somewhere; the cylinder test covers hidden ones.)
        let _ = hidden;
        println!("{repaired} curved bricks start on a repaired cell");
        assert!(s.curved > 0);
        assert!(r.report.to_text().contains("Curved: "));
        assert!(r.analysis.overlaps.is_empty());
        // About 0.6 s on an idle machine; generous so a loaded CI box doesn't fail it.
        assert!(took.as_secs_f64() < 10.0, "too slow: {took:?}");
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
                let q =
                    [8.0 * (c.x - p.origin.x) as f64 + 4.0, 8.0 * (c.z - p.origin.z) as f64 + 4.0, 3.2 * (c.y - p.origin.y) as f64 + 1.6];
                let d = shapes.iter().map(|s| sd_shape(s, q)).fold(f64::INFINITY, f64::min);
                assert!(d < 4.0, "filled cell {c:?} is {d:.2} mm outside the sphere");
            }
        }
    }

    /// Protect boxes of a curved build that reach outside `S` grown by the contact margin,
    /// leaving out a ground brick's bottom (kept flat on purpose).
    fn ledges(r: &BuildResult) -> Vec<String> {
        let solid = Solid::new(&r.compiled.surface);
        let mut out = Vec::new();
        for (_, p) in bricks(r) {
            let def = p.custom.as_deref().unwrap();
            let Some(t) = def.trim.as_deref() else { continue };
            let h = 3.2 * def.plates as f64;
            for b in &t.protect {
                let ground_bottom = p.origin.y == 0 && b.min[2] <= 0.0 && b.max[2] <= h - TOP + 1e-6;
                if !ground_bottom && solid.reaches_outside(p, b, CONTACT_MARGIN) {
                    out.push(format!("{} at {:?}: {:?}", def.id, p.origin, b));
                }
            }
        }
        out
    }

    #[test]
    fn a_sloped_side_has_no_ledges() {
        // A dome and a cone: every course is narrower than the one below.
        for (what, json) in [
            (
                "dome",
                r#"{"shapes":[{"shape":"ellipsoid","center":[0,0,0],"size":[10,10,10],"color":"Red"},
                {"shape":"box","center":[0,-3,0],"size":[12,6,12],"op":"subtract"}]}"#,
            ),
            ("cone", r#"{"shapes":[{"shape":"cone","center":[0,3,0],"size":[10,6,10],"color":"Blue"}]}"#),
        ] {
            let r = printed(json, 14);
            let c = r.report.printed.as_ref().unwrap().contacts.clone().unwrap();
            println!("{what}: {c:?}");
            assert!(c.contacts > 0 && c.covered < c.contacts, "{what}: expected some contacts outside the shape");
            assert!(c.studs_after < c.studs_before, "{what}: no stud was dropped");
            let l = ledges(&r);
            // Only contacts restored to hold a part may stick out (3 boxes each at most).
            assert!(l.len() <= 3 * c.restored, "{what}: {} ledges with {} contacts restored: {l:#?}", l.len(), c.restored);
            assert!(c.ledges_after < c.ledges_before, "{what}: {c:?}");
        }
    }

    #[test]
    fn trimmed_contacts_keep_every_part_held() {
        for (what, json, res) in [("snowman", SNOWMAN, 16), ("duck", DUCK, 16), ("ball", BALL, 12)] {
            let spec = Spec::from_json(json).unwrap();
            let plain =
                build(&spec, &BuildOptions { resolution: Some(res), print: true, curves: false, ..Default::default() }, None).unwrap();
            let r = printed(json, res);
            let c = r.report.printed.as_ref().unwrap().contacts.clone().unwrap();
            // Against the same packing before its contacts were trimmed: nothing cut loose.
            assert_eq!((c.cut_loose, c.newly_floating), (0, 0), "{what}: {c:?}");
            // Every part still has a connection (or stands on the ground) …
            let mut held: Vec<bool> = r.model.placements.iter().map(|p| p.origin.y == 0).collect();
            for &(a, b, _) in &r.analysis.connections {
                held[a] = true;
                held[b] = true;
            }
            for (i, h) in held.iter().enumerate() {
                let p = &r.model.placements[i];
                assert!(h, "{what}: part {} at {:?} has no connection", p.part, p.origin);
            }
            // … and curving never leaves more floating than plain bricks do.
            assert!(r.analysis.floating.len() <= plain.analysis.floating.len(), "{what}");
            assert!(sf_core::print::fused_splits(&r.model.placements).is_empty(), "{what}");
            // Studs, sockets and protect boxes agree: every stud on a curved brick has its top
            // protected, every socket off the ground has a stud under it.
            let studs: HashSet<(i32, i32, i32)> =
                r.model.placements.iter().flat_map(|p| p.top_studs().into_iter().map(move |(x, z)| (x, p.top(), z))).collect();
            for (_, p) in bricks(&r) {
                let def = p.custom.as_deref().unwrap();
                let Some(t) = def.trim.as_deref() else { continue };
                let h = 3.2 * def.plates as f64;
                for &(x, z) in &def.studs {
                    let c = [8.0 * x as f64 + 4.0, 8.0 * z as f64 + 4.0, h - 0.5];
                    assert!(
                        t.protect.iter().any(|b| (0..3).all(|k| b.min[k] <= c[k] && c[k] <= b.max[k])),
                        "{what}: {} stud {x},{z} unprotected",
                        def.id
                    );
                }
                if p.origin.y > 0 {
                    for (x, z) in p.bottom_sockets() {
                        assert!(studs.contains(&(x, p.origin.y, z)), "{what}: {} socket over no stud", def.id);
                    }
                }
            }
            let c = r.report.printed.as_ref().unwrap().contacts.clone().unwrap();
            println!("{what}: {c:?}");
            assert!(r.report.to_text().contains("Contacts: "), "{}", r.report.to_text());
        }
    }

    /// Ground protect boxes of a curved build: `(all, reaching outside S + margin)`.
    fn ground_boxes(r: &BuildResult) -> (usize, Vec<String>) {
        let solid = Solid::new(&r.compiled.surface);
        let (mut n, mut out) = (0, Vec::new());
        for (_, p) in bricks(r) {
            let def = p.custom.as_deref().unwrap();
            let Some(t) = def.trim.as_deref() else { continue };
            let h = 3.2 * def.plates as f64;
            for b in t.protect.iter().filter(|b| p.origin.y == 0 && b.min[2] <= 0.0 && b.max[2] <= h - TOP + 1e-6) {
                n += 1;
                if solid.reaches_outside(p, b, CONTACT_MARGIN) {
                    out.push(format!("{} at {:?}: {b:?} reaches {:.1} mm out", def.id, p.origin, solid.max_outside(p, b)));
                }
            }
        }
        (n, out)
    }

    #[test]
    fn a_cylinder_stands_on_its_own_round_bottom() {
        let r = printed(r#"{"shapes":[{"shape":"cylinder","center":[0,3,0],"size":[8,6,8],"color":"Reddish Brown"}]}"#, 12);
        let (n, out) = ground_boxes(&r);
        assert!(n > 0, "the inner ground columns keep their plain bottoms");
        assert!(out.is_empty(), "ground boxes past the cylinder: {out:#?}");
        let c = r.report.printed.as_ref().unwrap().contacts.clone().unwrap();
        assert_eq!(c.pads, 0, "{c:?}");
        assert!(!c.ground_fallback && c.ground_touching == c.ground_columns, "{c:?}");
        assert!(r.analysis.balance.ok && r.analysis.floating.is_empty());
        assert!(r.report.to_text().contains("Base: follows the outline; 0 flat pad"), "{}", r.report.to_text());
    }

    #[test]
    fn a_ball_keeps_a_small_flat_pad_and_stands() {
        for (what, json, res) in [("snowman", SNOWMAN, 20), ("ball", BALL, 12)] {
            let r = printed(json, res);
            let c = r.report.printed.as_ref().unwrap().contacts.clone().unwrap();
            assert!(c.pads > 0 && !c.ground_fallback, "{what}: {c:?}");
            assert!(r.analysis.balance.ok, "{what}: {:?}", r.analysis.balance);
            assert!(r.analysis.floating.is_empty(), "{what}");
            assert_eq!((c.cut_loose, c.newly_floating), (0, 0), "{what}");
            // The pad sits under the shape: it reaches out of `S` only below the band's top,
            // never a full column's square past the outline.
            let (_, out) = ground_boxes(&r);
            let solid = Solid::new(&r.compiled.surface);
            for (_, p) in bricks(&r) {
                let def = p.custom.as_deref().unwrap();
                let Some(t) = def.trim.as_deref() else { continue };
                for b in t.protect.iter().filter(|b| p.origin.y == 0 && b.min[2] < 0.0) {
                    assert!(solid.max_outside(p, b) <= REACH + 0.2, "{what}: {} pad {b:?} reaches far out", def.id);
                }
            }
            assert!(!out.is_empty(), "{what}: expected a pad under the ball");
        }
    }

    /// Share of the true shape (sampled every 0.5 mm) that no part's cells cover.
    fn missing(r: &BuildResult) -> f64 {
        let solid = Solid::new(&r.compiled.surface);
        let cells: HashSet<Cell> = r.model.placements.iter().flat_map(|p| p.cells()).collect();
        let (lo, hi) = solid.bounds().unwrap();
        let (mut inside, mut missing) = (0usize, 0usize);
        let st = 0.5;
        let mut x = lo[0] + st / 2.0;
        while x < hi[0] {
            let mut y = lo[1] + st / 2.0;
            while y < hi[1] {
                let mut z = lo[2] + st / 2.0;
                while z < hi[2] {
                    if solid.sd([x, y, z]) <= 0.0 {
                        inside += 1;
                        missing += !cells.contains(&Cell::new((x / 8.0).floor() as i32, (z / 3.2).floor() as i32, (y / 8.0).floor() as i32))
                            as usize;
                    }
                    z += st;
                }
                y += st;
            }
            x += st;
        }
        missing as f64 / inside.max(1) as f64
    }

    #[test]
    fn a_cylinder_has_no_flats_and_clean_meshes() {
        let json = r#"{"shapes":[{"shape":"cylinder","center":[0,3,0],"size":[8,6,8],"color":"Reddish Brown"}]}"#;
        let plain = build(
            &Spec::from_json(json).unwrap(),
            &BuildOptions { resolution: Some(12), print: true, curves: false, ..Default::default() },
            None,
        )
        .unwrap();
        let r = printed(json, 12);
        let (before, after) = (missing(&plain), missing(&r));
        println!("missing: plain {:.1}%, curved {:.1}%", 100.0 * before, 100.0 * after);
        assert!(before > 0.03 && after < 0.015, "the curve should bulge past the voxels: {before:.3} → {after:.3}");
        // Plain print mode keeps the shape's own voxels.
        for p in &plain.model.placements {
            for c in p.cells() {
                assert!(plain.compiled.grid.get(c) != Fill::Empty || plain.report.warnings.iter().any(|w| w.contains("small feature")));
            }
        }
        let c = r.report.printed.as_ref().unwrap().contacts.clone().unwrap();
        println!("{c:?}");
        assert!(c.slivers * 100 < c.mesh_triangles, "slivers: {c:?}");
        assert!(c.max_off_um < 400, "a crease is cut off: {c:?}");
        assert!(c.chamfered * 100 < c.mesh_triangles, "{c:?}");
        assert!(r.analysis.floating.is_empty() && r.analysis.balance.ok);
        // Bricks inside the cylinder touch nothing exposed: they stay plain, and only they do.
        let occ = Occupancy::new(&r.model.placements, &r.model.meshes);
        let all = bricks(&r);
        let plain_bricks = all.iter().filter(|(_, p)| p.custom.as_ref().unwrap().trim.is_none()).count();
        assert!(plain_bricks > 0, "expected some fully hidden bricks");
        for (i, p) in all {
            assert_eq!(p.custom.as_ref().unwrap().trim.is_some(), exposed(&r, &occ, i, p), "{}", p.part);
        }
    }

    const DUCK: &str = r#"{"title":"duck","shapes":[
        {"shape":"ellipsoid","center":[0,4,1],"size":[9,7,13],"color":"Yellow"},
        {"shape":"ellipsoid","center":[0,10,-3],"size":[6,6,6],"color":"Yellow"},
        {"shape":"box","center":[0,9.3,-6.6],"size":[3,1.4,2.6],"color":"Orange"},
        {"shape":"ellipsoid","center":[1.6,11,-5.4],"size":[1,1,1],"color":"Black","op":"paint","mirror_x":true}]}"#;
    const BALL: &str = r#"{"shapes":[{"shape":"ellipsoid","center":[0,4,0],"size":[8,8,8],"color":"Red"}]}"#;
}
