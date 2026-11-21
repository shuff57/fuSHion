//! Curved printed bricks: a [`CustomShape::Printed`] with a [`Trim`] keeps exactly
//! `block ∩ (S ∪ P)`, where `S` is the model's solid (the trim's shapes, applied in order)
//! and `P` the protect boxes.
//!
//! # How it is built
//! 1. The plain brick, with studs only on cells whose top survives the trim (fused units
//!    whole), checked against its closed-form volume and box. (Measuring after every stage
//!    costs more than the rest of the build, so stage-by-stage checks run only when the
//!    whole is off, to name the stage at fault.)
//! 2. **Fill**: the hollow stays open only
//!    * under a column that keeps its top, inside `offset(S, −wall)` and up to the hollow's
//!      ceiling or a flat level under the lowest place that offset closes over an open point,
//!    * under a column in `sockets`: the column's hollow up to a stud's height plus
//!      clearance, whose walls and a top's thickness of ceiling the trim keeps even outside
//!      `S`.
//!
//!    The rest of the hollow inside the kept solid is filled, so no cut breaks through, and
//!    every cavity has upright sides and a flat ceiling that bridges.
//! 3. **Trim**: the brick is intersected with `S ∪ P` (plus a slab over the kept studs and
//!    the sockets' keep boxes).
//!
//! Only what reaches the brick is built: shapes whose box misses the brick's are left out,
//! boxes are clipped to it, those surely inside `S` dropped and the rest merged into as few
//! boxes as simply found, and the trim is skipped when `S` holds the whole brick.
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
use crate::run::{finish, finish_with, run_stages, run_step, run_unchecked};
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
/// How far past the plain brick's box shapes and boxes still count, mm.
const REGION_PAD: f64 = 0.5;
/// Grid step (across, up) of the search for a cavity's flat ceiling, mm.
const LEVEL_STEP: f64 = 0.4;
const LEVEL_DZ: f64 = 0.2;
/// A kept column's cavity lower than this is filled instead, mm.
const MIN_CAVITY: f64 = 0.6;

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
    /// Seconds spent in each step of the build, in order.
    pub timings: Vec<(&'static str, f64)>,
    /// The slowest trim commands: (seconds, command name, index).
    pub slowest: Vec<(f64, String, usize)>,
}

// ---------------------------------------------------------------- point tests

/// `p` in a shape's local frame.
fn to_local(s: &TrimShape, p: [f64; 3]) -> [f64; 3] {
    let d = [
        p[0] - s.translation[0],
        p[1] - s.translation[1],
        p[2] - s.translation[2],
    ];
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
            q[2].abs() <= height / 2.0
                && (q[0] / radii[0]).powi(2) + (q[1] / radii[1]).powi(2) <= 1.0
        }
        TrimKind::Cone {
            radii,
            top_scale,
            height,
        } => {
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
            let t = if l2 > 0.0 {
                ((aq[0] * ab[0] + aq[1] * ab[1] + aq[2] * ab[2]) / l2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let c = [
                a[0] + t * ab[0] - q[0],
                a[1] + t * ab[1] - q[1],
                a[2] + t * ab[2] - q[2],
            ];
            c[0] * c[0] + c[1] * c[1] + c[2] * c[2] <= radius * radius
        }
        TrimKind::Prism { points, height } => {
            q[2].abs() <= height / 2.0 && in_polygon(points, [q[0], q[1]])
        }
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
        TrimKind::Ellipsoid { radii } => TrimKind::Ellipsoid {
            radii: [pos(radii[0] + d)?, pos(radii[1] + d)?, pos(radii[2] + d)?],
        },
        TrimKind::Box { size } => TrimKind::Box {
            size: [
                pos(size[0] + 2.0 * d)?,
                pos(size[1] + 2.0 * d)?,
                pos(size[2] + 2.0 * d)?,
            ],
        },
        TrimKind::Cylinder { radii, height } => TrimKind::Cylinder {
            radii: [pos(radii[0] + d)?, pos(radii[1] + d)?],
            height: pos(height + 2.0 * d)?,
        },
        TrimKind::Cone {
            radii,
            top_scale,
            height,
        } => {
            // The side's slope stretches a normal offset `d` to `d·√(1 + k²)` across.
            let k = radii[0].max(radii[1]) * (1.0 - top_scale).abs() / height;
            let dd = d * (1.0 + k * k).sqrt();
            let r = [pos(radii[0] + dd)?, pos(radii[1] + dd)?];
            let top = (radii[0] * top_scale + dd).max(0.0) / r[0];
            TrimKind::Cone {
                radii: r,
                top_scale: top,
                height: pos(height + 2.0 * d)?,
            }
        }
        TrimKind::Capsule { a, b, radius } => TrimKind::Capsule {
            a: *a,
            b: *b,
            radius: pos(radius + d)?,
        },
        TrimKind::Prism { points, height } => {
            // Each vertex moved along its corner's bisector so both edges move by `d`.
            let n = points.len();
            let area: f64 = (0..n)
                .map(|i| {
                    points[i][0] * points[(i + 1) % n][1] - points[(i + 1) % n][0] * points[i][1]
                })
                .sum::<f64>()
                / 2.0;
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
            TrimKind::Prism {
                points: pts,
                height: pos(height + 2.0 * d)?,
            }
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
                    to_local(
                        s,
                        [
                            if k & 1 == 0 { block.0[0] } else { block.1[0] },
                            if k & 2 == 0 { block.0[1] } else { block.1[1] },
                            if k & 4 == 0 { block.0[2] } else { block.1[2] },
                        ],
                    )
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
                    TrimKind::Cylinder { height, .. }
                    | TrimKind::Cone { height, .. }
                    | TrimKind::Prism { height, .. } => height / 2.0,
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
                let (dm, dp) = (
                    if minus_out { -d } else { 0.0 },
                    if plus_out { -d } else { 0.0 },
                );
                grow[i] = dm + dp;
                shift[i] = (dp - dm) / 2.0;
            }
            let kind = offset_kind(&s.kind, d)?;
            let kind = match kind {
                TrimKind::Box { size } => TrimKind::Box {
                    size: [size[0] + grow[0], size[1] + grow[1], size[2] + grow[2]],
                },
                TrimKind::Cylinder { radii, height } => TrimKind::Cylinder {
                    radii,
                    height: height + grow[2],
                },
                TrimKind::Cone {
                    radii,
                    top_scale,
                    height,
                } => TrimKind::Cone {
                    radii,
                    top_scale,
                    height: height + grow[2],
                },
                TrimKind::Prism { points, height } => TrimKind::Prism {
                    points,
                    height: height + grow[2],
                },
                k => k,
            };
            let r = &s.rotation;
            let t = [0, 1, 2].map(|i| {
                s.translation[i] + r[i][0] * shift[0] + r[i][1] * shift[1] + r[i][2] * shift[2]
            });
            Some(TrimShape {
                kind,
                translation: t,
                ..s.clone()
            })
        })
        .collect()
}

// ---------------------------------------------------------------- SolidCraft bodies

/// Commands making body `name` from the shapes (in order) and boxes (unioned after), each
/// nudged by a few µm. Returns `None` if nothing is added.
fn body_cmds(
    name: &str,
    shapes: &[TrimShape],
    boxes: &[TrimBox],
    jitter: f64,
) -> Option<Vec<Value>> {
    let mut c = Vec::new();
    let mut have = false;
    for (k, s) in shapes.iter().enumerate() {
        if s.op == TrimOp::Subtract && !have {
            continue;
        }
        let nudge = 1.3e-3 * jitter * (k as f64 + 1.0);
        let piece = if have {
            format!("{name}_{k}")
        } else {
            name.to_string()
        };
        c.extend(primitive(&piece, &s.kind));
        let r = &s.rotation;
        let t = [
            s.translation[0] + nudge,
            s.translation[1] + 0.7 * nudge,
            s.translation[2] + 0.4 * nudge,
        ];
        let m: Vec<Value> = (0..3)
            .map(|i| json!([r[i][0], r[i][1], r[i][2], r6(t[i])]))
            .collect();
        c.push(cmd("solid.move", json!({"bodies": [piece], "matrix": m})));
        if have {
            let op = if s.op == TrimOp::Add { "join" } else { "cut" };
            c.push(cmd(
                "solid.combine",
                json!({"target": name, "tools": [piece], "operation": op, "touching": "one_body"}),
            ));
        }
        have = true;
    }
    for (k, b) in boxes.iter().enumerate() {
        let piece = if have {
            format!("{name}_p{k}")
        } else {
            name.to_string()
        };
        let e = 0.9e-3 * jitter * (k as f64 + 1.0);
        let size = [
            b.max[0] - b.min[0] + e,
            b.max[1] - b.min[1] + e,
            b.max[2] - b.min[2] + e,
        ];
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
    let scale = |f: [f64; 3]| {
        cmd(
            "solid.scale",
            json!({"bodies": [name], "factors": f.map(r6), "origin": [0, 0, 0]}),
        )
    };
    let equal = |a: f64, b: f64| (a - b).abs() < 1e-9;
    match k {
        TrimKind::Ellipsoid { radii } => {
            if equal(radii[0], radii[1]) && equal(radii[1], radii[2]) {
                vec![cmd(
                    "solid.sphere",
                    new(json!({"radius": r6(radii[0]), "center": [0, 0, 0]})),
                )]
            } else {
                vec![
                    cmd(
                        "solid.sphere",
                        new(json!({"radius": 1.0, "center": [0, 0, 0]})),
                    ),
                    scale(*radii),
                ]
            }
        }
        TrimKind::Box { size } => vec![cmd(
            "solid.box",
            new(
                json!({"length": r6(size[0]), "width": r6(size[1]), "height": r6(size[2]), "center": [0, 0, 0]}),
            ),
        )],
        TrimKind::Cylinder { radii, height } => {
            let mut c = vec![cmd(
                "solid.cylinder",
                new(
                    json!({"radius": 1.0, "height": r6(*height), "base": [0, 0, r6(-height / 2.0)]}),
                ),
            )];
            c.push(scale([radii[0], radii[1], 1.0]));
            c
        }
        TrimKind::Cone {
            radii,
            top_scale,
            height,
        } => {
            let mut p =
                json!({"radius": 1.0, "height": r6(*height), "base": [0, 0, r6(-height / 2.0)]});
            if *top_scale > 1e-6 {
                p["top_radius"] = json!(r6(*top_scale));
            }
            vec![cmd("solid.cone", new(p)), scale([radii[0], radii[1], 1.0])]
        }
        TrimKind::Capsule { a, b, radius } => {
            let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let l = (ab[0] * ab[0] + ab[1] * ab[1] + ab[2] * ab[2]).sqrt();
            if l < 1e-6 {
                return vec![cmd(
                    "solid.sphere",
                    new(json!({"radius": r6(*radius), "center": p3(*a)})),
                )];
            }
            let axis = ab.map(|v| v / l);
            let ball = |c: [f64; 3], i: usize| {
                cmd(
                    "solid.sphere",
                    json!({"radius": r6(radius + 0.002), "center": p3(c), "operation": "join", "targets": [name], "name": format!("{name}End{i}")}),
                )
            };
            vec![
                cmd(
                    "solid.cylinder",
                    new(
                        json!({"radius": r6(*radius), "height": r6(l), "base": p3(*a), "axis": p3(axis)}),
                    ),
                ),
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
                cmd(
                    "solid.extrude",
                    new(json!({"sketch": sk, "distance": r6(*height)})),
                ),
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
    let f = |o: usize| {
        f64::from(f32::from_le_bytes([
            stl[o],
            stl[o + 1],
            stl[o + 2],
            stl[o + 3],
        ]))
    };
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
        let ray = |i: usize, j: usize| {
            [
                lo[0] + (i as f64 + 0.5) * step + 1.7e-7,
                lo[1] + (j as f64 + 0.5) * step + 2.3e-7,
            ]
        };
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
                    let l0 =
                        ((p1[1] - p2[1]) * (q[0] - p2[0]) + (p2[0] - p1[0]) * (q[1] - p2[1])) / d;
                    let l1 =
                        ((p2[1] - p0[1]) * (q[0] - p2[0]) + (p0[0] - p2[0]) * (q[1] - p2[1])) / d;
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
        self.hits[j * self.nx + i]
            .iter()
            .filter(|&&h| h < z)
            .count()
            % 2
            == 1
    }
}

/// Inside the solid a binary STL encloses, at `p`: a ray up from `p` crossing it an odd
/// number of times. (For tests and checks; slow for many points, see the sampler.)
pub fn inside_mesh(stl: &[u8], p: [f64; 3]) -> bool {
    let r = Rays::cast(
        stl,
        [p[0] - 0.05, p[1] - 0.05],
        [p[0] + 0.05, p[1] + 0.05],
        0.1,
    );
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
        let c = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ];
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

/// The names of every body, in order.
fn body_names(s: &mut Session) -> Result<Vec<String>> {
    let r = s
        .execute("inspect.measure", &json!({"names_only": true}))
        .map_err(|e| CustomError::Mismatch {
            stage: "Trim".into(),
            details: format!("measure: {e}"),
        })?;
    Ok(r["bodies"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|b| b["name"].as_str().map(str::to_string))
        .collect())
}

/// The bodies not in `known` (what a group of commands made, however it was split).
fn new_bodies(s: &mut Session, known: &[String]) -> Result<Vec<String>> {
    Ok(body_names(s)?
        .into_iter()
        .filter(|n| !known.contains(n))
        .collect())
}

/// A box command, grown by `e` all round: a new body `name`, or `op` ("cut", "join",
/// "intersect") into `target`.
fn box_cmd(name: &str, b: &TrimBox, e: f64, op: &str, target: Option<&str>) -> Value {
    let size = [0, 1, 2].map(|k| b.max[k] - b.min[k] + e);
    let mut p = json!({"length": r6(size[0]), "width": r6(size[1]), "height": r6(size[2]), "corner": p3(b.min.map(|v| v - e / 2.0)), "operation": op, "name": format!("{name}Box")});
    match target {
        Some(t) => p["targets"] = json!([t]),
        None => p["body_name"] = json!(name),
    }
    cmd("solid.box", p)
}

/// A shape's axis-aligned box in the brick frame.
fn shape_aabb(s: &TrimShape) -> ([f64; 3], [f64; 3]) {
    let (lo, hi) = match &s.kind {
        TrimKind::Ellipsoid { radii } => (radii.map(|r| -r), *radii),
        TrimKind::Box { size } => (size.map(|v| -v / 2.0), size.map(|v| v / 2.0)),
        TrimKind::Cylinder { radii, height } => (
            [-radii[0], -radii[1], -height / 2.0],
            [radii[0], radii[1], height / 2.0],
        ),
        TrimKind::Cone {
            radii,
            top_scale,
            height,
        } => {
            let f = top_scale.max(1.0);
            (
                [-radii[0] * f, -radii[1] * f, -height / 2.0],
                [radii[0] * f, radii[1] * f, height / 2.0],
            )
        }
        TrimKind::Capsule { a, b, radius } => (
            [0, 1, 2].map(|k| a[k].min(b[k]) - radius),
            [0, 1, 2].map(|k| a[k].max(b[k]) + radius),
        ),
        TrimKind::Prism { points, height } => {
            let (mut lo, mut hi) = (
                [f64::MAX, f64::MAX, -height / 2.0],
                [f64::MIN, f64::MIN, height / 2.0],
            );
            for q in points {
                for k in 0..2 {
                    lo[k] = lo[k].min(q[k]);
                    hi[k] = hi[k].max(q[k]);
                }
            }
            (lo, hi)
        }
    };
    let (mut a, mut b) = ([f64::MAX; 3], [f64::MIN; 3]);
    for k in 0..8 {
        let l = [
            if k & 1 == 0 { lo[0] } else { hi[0] },
            if k & 2 == 0 { lo[1] } else { hi[1] },
            if k & 4 == 0 { lo[2] } else { hi[2] },
        ];
        for i in 0..3 {
            let v = s.translation[i]
                + s.rotation[i][0] * l[0]
                + s.rotation[i][1] * l[1]
                + s.rotation[i][2] * l[2];
            a[i] = a[i].min(v);
            b[i] = b[i].max(v);
        }
    }
    (a, b)
}

fn overlap(a: ([f64; 3], [f64; 3]), b: ([f64; 3], [f64; 3])) -> bool {
    (0..3).all(|k| a.0[k] < b.1[k] && b.0[k] < a.1[k])
}

/// The shapes that reach `region` (the others change nothing in it), leading subtractions
/// (from nothing) left out.
fn shapes_near(shapes: &[TrimShape], region: ([f64; 3], [f64; 3])) -> Vec<TrimShape> {
    let mut out: Vec<TrimShape> = Vec::new();
    for s in shapes {
        if overlap(shape_aabb(s), region) && (s.op == TrimOp::Add || !out.is_empty()) {
            out.push(s.clone());
        }
    }
    out
}

fn convex(k: &TrimKind) -> bool {
    match k {
        TrimKind::Prism { points, .. } => {
            let n = points.len();
            let turns: Vec<f64> = (0..n)
                .map(|i| {
                    let (a, b, c) = (points[i], points[(i + 1) % n], points[(i + 2) % n]);
                    (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0])
                })
                .collect();
            turns.iter().all(|&t| t >= -1e-12) || turns.iter().all(|&t| t <= 1e-12)
        }
        _ => true,
    }
}

/// Whether the box `(lo, hi)` is surely inside `S`: inside one convex added shape (its
/// corners are), with nothing subtracted after it near the box.
fn box_in_s(shapes: &[TrimShape], lo: [f64; 3], hi: [f64; 3]) -> bool {
    shapes.iter().enumerate().any(|(k, s)| {
        s.op == TrimOp::Add
            && convex(&s.kind)
            && (0..8).all(|c| {
                let q = [
                    if c & 1 == 0 { lo[0] } else { hi[0] },
                    if c & 2 == 0 { lo[1] } else { hi[1] },
                    if c & 4 == 0 { lo[2] } else { hi[2] },
                ];
                inside_kind(&s.kind, to_local(s, q))
            })
            && shapes[k + 1..]
                .iter()
                .all(|t| t.op == TrimOp::Add || !overlap(shape_aabb(t), (lo, hi)))
    })
}

/// The same union of boxes inside `region`, in as few boxes as simply found: each clipped
/// to `region`, those surely inside `S` (when given) or inside another box dropped, and
/// pairs whose union is a box merged.
fn merge_boxes(
    boxes: Vec<TrimBox>,
    region: ([f64; 3], [f64; 3]),
    inside: Option<&[TrimShape]>,
) -> Vec<TrimBox> {
    const E: f64 = 1e-6;
    let mut v: Vec<TrimBox> = boxes
        .into_iter()
        .map(|b| TrimBox {
            min: [0, 1, 2].map(|k| b.min[k].max(region.0[k])),
            max: [0, 1, 2].map(|k| b.max[k].min(region.1[k])),
        })
        .filter(|b| (0..3).all(|k| b.max[k] - b.min[k] > E))
        .filter(|b| inside.is_none_or(|sh| !box_in_s(sh, b.min, b.max)))
        .collect();
    let contains = |a: &TrimBox, b: &TrimBox| {
        (0..3).all(|k| a.min[k] <= b.min[k] + E && a.max[k] >= b.max[k] - E)
    };
    loop {
        let mut changed = false;
        'outer: for i in 0..v.len() {
            for j in 0..v.len() {
                if i == j {
                    continue;
                }
                if contains(&v[j], &v[i]) {
                    v.remove(i);
                    changed = true;
                    break 'outer;
                }
                let same: Vec<usize> = (0..3)
                    .filter(|&k| {
                        (v[i].min[k] - v[j].min[k]).abs() < E
                            && (v[i].max[k] - v[j].max[k]).abs() < E
                    })
                    .collect();
                if same.len() == 2 {
                    let k = (0..3).find(|k| !same.contains(k)).unwrap_or(0);
                    if v[i].min[k] <= v[j].max[k] + E && v[j].min[k] <= v[i].max[k] + E {
                        let u = TrimBox {
                            min: [0, 1, 2].map(|a| v[i].min[a].min(v[j].min[a])),
                            max: [0, 1, 2].map(|a| v[i].max[a].max(v[j].max[a])),
                        };
                        v[i] = u;
                        v.remove(j);
                        changed = true;
                        break 'outer;
                    }
                }
            }
        }
        if !changed {
            return v;
        }
    }
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
    let CustomShape::Printed {
        cells,
        plates,
        units,
        label,
        trim: Some(trim),
        sockets,
    } = shape
    else {
        return Err(CustomError::InvalidShape(
            "build_trimmed takes a printed shape with a trim".into(),
        ));
    };
    let bad = |why: String| CustomError::Mismatch {
        stage: "Trim".into(),
        details: why,
    };
    let mut clock = std::time::Instant::now();
    let mut timings: Vec<(&'static str, f64)> = Vec::new();
    let mut lap = |name: &'static str, timings: &mut Vec<(&'static str, f64)>| {
        timings.push((name, clock.elapsed().as_secs_f64()));
        clock = std::time::Instant::now();
    };
    // Cells moved to start at 0 (the trim's frame).
    let i0 = cells.iter().map(|c| c.0).min().unwrap_or(0);
    let j0 = cells.iter().map(|c| c.1).min().unwrap_or(0);
    let norm: Vec<(i32, i32)> = cells.iter().map(|c| (c.0 - i0, c.1 - j0)).collect();
    let units: Vec<Vec<(i32, i32)>> = units
        .iter()
        .map(|u| u.iter().map(|c| (c.0 - i0, c.1 - j0)).collect())
        .collect();
    let h = p.body_height(*plates);
    // Studs stay where the top under them is kept: every sample of the cell's square 0.3 mm
    // under the top is inside the trim.
    let mut keep = BTreeSet::new();
    let mut dropped = Vec::new();
    for &(i, j) in &norm {
        let (x0, y0) = (f64::from(i) * p.pitch, f64::from(j) * p.pitch);
        let ok = (0..7).all(|a| {
            (0..7).all(|b| {
                let q = [
                    x0 + 1.0 + 6.0 * f64::from(a) / 6.0,
                    y0 + 1.0 + 6.0 * f64::from(b) / 6.0,
                    h - 0.3,
                ];
                inside_trim(trim, q)
            })
        });
        if ok {
            keep.insert((i, j));
        } else {
            dropped.push((i, j));
        }
    }
    let plain = CustomShape::Printed {
        cells: norm.clone(),
        plates: *plates,
        units,
        label: label.clone(),
        trim: None,
        sockets: None,
    };
    let plan = plan_with(&plain, p, Some(&keep))?;
    // 1. The plain brick, checked against its closed-form volume and box (its mesh is only
    // for the volume estimate). Measuring after every stage costs more than the rest of the
    // build, so the stages are checked one by one only when the whole is off, to name the
    // stage at fault.
    let last = plan.stages.last().map_or("(empty)", |st| st.name.as_str());
    let mut s = Session::default();
    let quick = run_unchecked(&mut s, &plan).and_then(|()| {
        finish_with(
            &mut s,
            last,
            Some(plan.volume_mm3),
            Some(plan.bbox_mm),
            false,
            crate::MAX_PART_MM,
        )
    });
    let untrimmed = match quick {
        Ok(u) => u,
        Err(_) => {
            s = Session::default();
            run_stages(&mut s, &plan)?;
            finish_with(
                &mut s,
                last,
                Some(plan.volume_mm3),
                Some(plan.bbox_mm),
                false,
                crate::MAX_PART_MM,
            )?
        }
    };
    let part_name = body_names(&mut s)?
        .into_iter()
        .next()
        .unwrap_or_else(|| "Body1".into());
    lap("plain brick", &mut timings);
    let mut commands: Vec<Value> = plan
        .stages
        .iter()
        .flat_map(|st| st.commands.clone())
        .collect();
    let mut index = 0usize;
    let mut slowest: Vec<(f64, String, usize)> = Vec::new();
    let mut run = |s: &mut Session, c: &Value| -> Result<()> {
        let t = std::time::Instant::now();
        run_step(s, "Trim", index, c)?;
        let p = &c["params"];
        let what = format!(
            "{} {}",
            c["command"].as_str().unwrap_or("?"),
            [&p["target"], &p["tools"], &p["body_name"], &p["targets"]]
                .iter()
                .filter(|v| !v.is_null())
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join(" ")
        );
        slowest.push((t.elapsed().as_secs_f64(), what, index));
        commands.push(c.clone());
        index += 1;
        Ok(())
    };
    let (blo, bhi) = untrimmed.bbox_mm;
    // Only what reaches the brick matters: its box, a little grown.
    let region = (blo.map(|v| v - REGION_PAD), bhi.map(|v| v + REGION_PAD));
    let shapes = shapes_near(&trim.shapes, region);
    if shapes.is_empty() && merge_boxes(trim.protect.clone(), region, None).is_empty() {
        return Err(bad("the trim keeps nothing of the brick".into()));
    }
    let f = Footprint::new(&norm)?;
    let hollow_h = h - p.top_thickness;
    let inner = f.offset_loops(p.pitch, p.clearance + p.wall - 0.01);
    let inner2: Vec<Vec<[f64; 2]>> = inner
        .iter()
        .map(|l| l.iter().map(|v| [v[0], v[1]]).collect())
        .collect();
    let in_hollow_xy =
        |x: f64, y: f64| inner2.iter().filter(|l| in_polygon(l, [x, y])).count() % 2 == 1;
    let in_hollow = |q: [f64; 3]| q[2] > 0.0 && q[2] < hollow_h && in_hollow_xy(q[0], q[1]);
    // The block: the footprint's box, from the bottom to the top of the body.
    let block = (
        [p.clearance, p.clearance, 0.0],
        [
            f64::from(norm.iter().map(|c| c.0).max().unwrap_or(0) + 1) * p.pitch - p.clearance,
            f64::from(norm.iter().map(|c| c.1).max().unwrap_or(0) + 1) * p.pitch - p.clearance,
            h,
        ],
    );
    let col_box = |(i, j): (i32, i32), grow: f64, z0: f64, z1: f64| {
        let (x0, y0) = (f64::from(i) * p.pitch, f64::from(j) * p.pitch);
        TrimBox {
            min: [x0 - grow, y0 - grow, z0],
            max: [x0 + p.pitch + grow, y0 + p.pitch + grow, z1],
        }
    };
    let col_of = |q: [f64; 3]| {
        (
            (q[0] / p.pitch).floor() as i32,
            (q[1] / p.pitch).floor() as i32,
        )
    };

    // 2. Which of the hollow stays open. Everything else of it inside the kept solid is filled,
    // so every cavity has upright sides and a flat ceiling, and none comes within a wall of
    // the cut surface:
    // * under a column that keeps its top: the hollow inside offset(S, −wall), up to its own
    //   ceiling, or up to a flat level under the lowest place that offset closes over an open
    //   point (so no curved ceiling is left);
    // * under a column in `sockets`: the socket, the column's hollow up to a stud's height
    //   plus clearance, always (a stud below goes there); its walls and a top's thickness of
    //   ceiling are kept even outside `S` (a keep box in the trim).
    let keep_s = offset_in(&shapes, p.wall, block);
    let socket_cols: BTreeSet<(i32, i32)> = sockets
        .iter()
        .flatten()
        .map(|c| (c.0 - i0, c.1 - j0))
        .filter(|c| norm.contains(c))
        .collect();
    let socket_h = p.stud_height + 0.4;
    let mut open_h: std::collections::BTreeMap<(i32, i32), f64> = std::collections::BTreeMap::new();
    for &c in &keep {
        let (x0, y0) = (f64::from(c.0) * p.pitch, f64::from(c.1) * p.pitch);
        let mut top = hollow_h + 0.5;
        let mut any = false;
        let n = (p.pitch / LEVEL_STEP).round() as i32;
        let nz = (hollow_h / LEVEL_DZ).ceil() as i32;
        for a in 0..n {
            for b in 0..n {
                let (x, y) = (
                    x0 + (f64::from(a) + 0.5) * LEVEL_STEP,
                    y0 + (f64::from(b) + 0.5) * LEVEL_STEP,
                );
                if !in_hollow_xy(x, y) {
                    continue;
                }
                let mut prev = inside_s(&keep_s, [x, y, LEVEL_DZ / 2.0]);
                any |= prev;
                for k in 1..nz {
                    let z = (f64::from(k) + 0.5) * LEVEL_DZ;
                    let cur = inside_s(&keep_s, [x, y, z]);
                    any |= cur;
                    if prev && !cur {
                        top = top.min(z - 2.0 * LEVEL_DZ);
                    }
                    prev = cur;
                }
            }
        }
        if any && top >= MIN_CAVITY {
            open_h.insert(c, top);
        }
    }
    let in_cavity = |q: [f64; 3]| {
        let c = col_of(q);
        (socket_cols.contains(&c) && q[2] <= socket_h)
            || open_h
                .get(&c)
                .is_some_and(|&t| q[2] <= t && inside_s(&keep_s, q))
    };
    // What the trim keeps besides `S`: the protect boxes, a slab over each kept stud, and
    // each socket's walls and ceiling; clipped to the brick, those inside `S` dropped and the
    // rest merged.
    let mut tboxes: Vec<TrimBox> = trim.protect.clone();
    tboxes.extend(
        keep.iter()
            .map(|&c| col_box(c, 0.0, h - 0.3, h + p.stud_height + 2.0)),
    );
    for &c in &socket_cols {
        let zk = socket_h + p.top_thickness;
        tboxes.push(col_box(
            c,
            p.wall,
            -1.0,
            if zk > h - 0.3 { h + 0.5 } else { zk },
        ));
    }
    let tboxes = merge_boxes(tboxes, region, Some(&shapes));
    let kept_t = |q: [f64; 3]| inside_s(&shapes, q) || tboxes.iter().any(|b| in_box(b, q));
    let mut needs_fill = false;
    'scan: for a in 0..((bhi[0] - blo[0]) / 0.5) as i32 {
        for b in 0..((bhi[1] - blo[1]) / 0.5) as i32 {
            for c in 0..(hollow_h / 0.5) as i32 {
                let q = [
                    blo[0] + 0.25 + 0.5 * f64::from(a),
                    blo[1] + 0.25 + 0.5 * f64::from(b),
                    0.25 + 0.5 * f64::from(c),
                ];
                if in_hollow(q) && kept_t(q) && !in_cavity(q) {
                    needs_fill = true;
                    break 'scan;
                }
            }
        }
    }
    lap("plan fill", &mut timings);
    let big = |name: &str| {
        cmd(
            "solid.box",
            json!({"length": r6(bhi[0] - blo[0] + 6.0), "width": r6(bhi[1] - blo[1] + 6.0), "height": r6(bhi[2] - blo[2] + 6.0), "corner": p3([blo[0] - 3.0, blo[1] - 3.0, blo[2] - 3.0]), "operation": "new", "body_name": name, "name": format!("{name}Box")}),
        )
    };
    if needs_fill {
        let known = body_names(&mut s)?;
        let mut c = vec![sketch_xy("TrimFillSketch", 0.003)];
        c.extend(inner.iter().map(|l| closed(l)));
        c.push(cmd("sketch.finish", json!({})));
        c.push(cmd(
            "solid.extrude",
            json!({"sketch": "TrimFillSketch", "name": "TrimFillPrism", "distance": r6(hollow_h + 0.01 - 0.003), "operation": "new", "body_name": "TrimFill"}),
        ));
        for x in &c {
            run(&mut s, x)?;
        }
        // The open part of the kept columns: offset(S, −wall) less (box − their columns).
        let open_boxes = merge_boxes(
            open_h
                .iter()
                .map(|(&c, &t)| col_box(c, 0.0, -1.0, t))
                .collect(),
            region,
            None,
        );
        let mut cavity_known = known.clone();
        cavity_known.push("TrimFill".into());
        if !open_boxes.is_empty()
            && let Some(kc) = body_cmds("TrimKeep", &keep_s, &[], jitter)
        {
            for x in &kc {
                run(&mut s, x)?;
            }
            run(&mut s, &big("TrimShut"))?;
            for (k, b) in open_boxes.iter().enumerate() {
                run(
                    &mut s,
                    &box_cmd(
                        &format!("TrimOpen{k}"),
                        b,
                        0.7e-3 * jitter * (k as f64 + 1.0),
                        "cut",
                        Some("TrimShut"),
                    ),
                )?;
            }
            let keeps = new_bodies(&mut s, &cavity_known)?
                .into_iter()
                .filter(|n| n != "TrimShut")
                .collect::<Vec<_>>();
            for (k, piece) in keeps.iter().enumerate() {
                let last = k + 1 == keeps.len();
                run(
                    &mut s,
                    &cmd(
                        "solid.combine",
                        json!({"target": piece, "tools": ["TrimShut"], "operation": "cut", "keep_tools": !last}),
                    ),
                )?;
            }
        }
        // The sockets.
        let socket_boxes = merge_boxes(
            socket_cols
                .iter()
                .map(|&c| col_box(c, 0.0, -1.0, socket_h))
                .collect(),
            region,
            None,
        );
        if let Some(sc) = body_cmds("TrimSockets", &[], &socket_boxes, jitter) {
            for x in &sc {
                run(&mut s, x)?;
            }
        }
        // Fill = hollow − cavity, then joined on.
        for cav in new_bodies(&mut s, &cavity_known)? {
            let fills = new_bodies(&mut s, &known)?
                .into_iter()
                .filter(|n| {
                    n == "TrimFill" || n.starts_with("TrimFill ") || n.starts_with("TrimFill(")
                })
                .collect::<Vec<_>>();
            for (k, fp) in fills.iter().enumerate() {
                let last = k + 1 == fills.len();
                run(
                    &mut s,
                    &cmd(
                        "solid.combine",
                        json!({"target": fp, "tools": [cav], "operation": "cut", "keep_tools": !last}),
                    ),
                )?;
            }
        }
        let fills = new_bodies(&mut s, &known)?;
        if !fills.is_empty() {
            run(
                &mut s,
                &cmd(
                    "solid.combine",
                    json!({"target": part_name, "tools": fills, "operation": "join", "touching": "one_body"}),
                ),
            )?;
        }
    }
    lap("fill booleans", &mut timings);
    // 3. Trim to S ∪ (the kept boxes), unless that holds the whole brick.
    let whole = box_in_s(&shapes, region.0, region.1)
        || tboxes
            .iter()
            .any(|b| (0..3).all(|k| b.min[k] <= region.0[k] && b.max[k] >= region.1[k]));
    if !whole {
        let known = body_names(&mut s)?;
        // `S` and the boxes' union each on its own, then one join: joining box after box onto
        // a curved body costs a curved boolean each.
        let s_cmds = body_cmds("TrimSolid", &shapes, &[], jitter);
        let b_cmds = body_cmds(
            if s_cmds.is_some() {
                "TrimBoxes"
            } else {
                "TrimSolid"
            },
            &[],
            &tboxes,
            jitter,
        );
        if s_cmds.is_none() && b_cmds.is_none() {
            return Err(bad("the trim keeps nothing".into()));
        }
        for x in s_cmds.iter().chain(b_cmds.iter()).flatten() {
            run(&mut s, x)?;
        }
        if s_cmds.is_some() && b_cmds.is_some() {
            run(
                &mut s,
                &cmd(
                    "solid.combine",
                    json!({"target": "TrimSolid", "tools": ["TrimBoxes"], "operation": "join", "touching": "one_body"}),
                ),
            )?;
        }
        let solid = new_bodies(&mut s, &known)?;
        if solid.len() == 1 {
            run(
                &mut s,
                &cmd(
                    "solid.combine",
                    json!({"target": part_name, "tools": solid, "operation": "intersect"}),
                ),
            )?;
        } else {
            // Keep part ∩ T as part − (box − T): right however many pieces T is in.
            run(&mut s, &big("TrimAway"))?;
            for piece in &solid {
                run(
                    &mut s,
                    &cmd(
                        "solid.combine",
                        json!({"target": "TrimAway", "tools": [piece], "operation": "cut"}),
                    ),
                )?;
            }
            let away = new_bodies(&mut s, &known)?;
            run(
                &mut s,
                &cmd(
                    "solid.combine",
                    json!({"target": part_name, "tools": away, "operation": "cut"}),
                ),
            )?;
        }
    }
    lap("trim booleans", &mut timings);
    let names = body_names(&mut s)?;
    if names.len() != 1 {
        let r = s.execute("inspect.measure", &json!({})).ok();
        let vols: Vec<String> = r
            .and_then(|r| r["bodies"].as_array().cloned())
            .unwrap_or_default()
            .iter()
            .map(|b| {
                format!(
                    "{} {:.2} mm³ at {}",
                    b["name"].as_str().unwrap_or("?"),
                    b["volume_mm3"].as_f64().unwrap_or(0.0),
                    b["bbox"]["min"]
                )
            })
            .collect();
        return Err(bad(format!(
            "{} bodies after the trim, expected 1 (the trim splits the brick): {}",
            names.len(),
            vols.join("; ")
        )));
    }
    let part = finish(&mut s, "Trim", None, None)?;
    lap("mesh + export", &mut timings);
    if !analyze_stl(&part.stl)?.is_watertight() {
        return Err(bad("the trimmed mesh is not watertight".into()));
    }
    // Inside the plain brick's box.
    for k in 0..3 {
        if part.bbox_mm.0[k] < blo[k] - 0.05 || part.bbox_mm.1[k] > bhi[k] + 0.05 {
            return Err(bad(format!(
                "the trimmed brick pokes out of the block's box on axis {k}"
            )));
        }
    }
    lap("watertight + bbox", &mut timings);
    // The independent estimate.
    let rays = Rays::cast(
        &untrimmed.stl,
        [blo[0], blo[1]],
        [bhi[0], bhi[1]],
        SAMPLE_STEP,
    );
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
                if !(whole || kept_t(q)) {
                    continue;
                }
                if rays.inside(i, j, z) || (needs_fill && in_hollow(q) && !in_cavity(q)) {
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
    lap("sampled volume", &mut timings);
    let overhang = brick_overhang(&part.stl);
    lap("overhang", &mut timings);
    slowest.sort_by(|a, b| b.0.total_cmp(&a.0));
    slowest.truncate(12);
    Ok(Trimmed {
        part,
        untrimmed,
        sampled_volume: sampled,
        overhang_mm2: overhang,
        dropped_studs: dropped,
        filled: needs_fill,
        commands,
        timings,
        slowest,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id() -> [[f64; 3]; 3] {
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
    }

    #[test]
    fn point_tests_follow_the_shapes() {
        let e = TrimShape {
            op: TrimOp::Add,
            kind: TrimKind::Ellipsoid {
                radii: [2.0, 1.0, 1.0],
            },
            rotation: id(),
            translation: [5.0, 0.0, 0.0],
        };
        assert!(inside_s(std::slice::from_ref(&e), [6.9, 0.0, 0.0]));
        assert!(!inside_s(std::slice::from_ref(&e), [5.0, 1.1, 0.0]));
        let hole = TrimShape {
            op: TrimOp::Subtract,
            kind: TrimKind::Box {
                size: [1.0, 1.0, 1.0],
            },
            rotation: id(),
            translation: [5.0, 0.0, 0.0],
        };
        assert!(!inside_s(&[e.clone(), hole], [5.0, 0.0, 0.0]));
        let c = TrimKind::Cone {
            radii: [2.0, 2.0],
            top_scale: 0.0,
            height: 4.0,
        };
        assert!(inside_kind(&c, [1.9, 0.0, -1.99]) && !inside_kind(&c, [1.1, 0.0, 0.0]));
        let cap = TrimKind::Capsule {
            a: [0.0, 0.0, 0.0],
            b: [4.0, 0.0, 0.0],
            radius: 1.0,
        };
        assert!(inside_kind(&cap, [4.9, 0.0, 0.0]) && !inside_kind(&cap, [2.0, 1.1, 0.0]));
        let pr = TrimKind::Prism {
            points: vec![[0.0, 0.0], [4.0, 0.0], [0.0, 4.0]],
            height: 2.0,
        };
        assert!(inside_kind(&pr, [1.0, 1.0, 0.9]) && !inside_kind(&pr, [3.0, 3.0, 0.0]));
        // A shrunk prism is inside the prism, a wall away from its edges.
        let TrimKind::Prism { points, .. } = offset_kind(&pr, -0.5).unwrap() else {
            panic!()
        };
        assert!(points.iter().all(|q| q[0] >= 0.49 && q[1] >= 0.49));
    }

    #[test]
    fn merged_boxes_cover_the_same_points() {
        let b = |min: [f64; 3], max: [f64; 3]| TrimBox { min, max };
        // A row of touching column boxes, one inside another, one sticking out of the region.
        let boxes = vec![
            b([0.0, 0.0, 0.0], [8.0, 8.0, 2.2]),
            b([8.0, 0.0, 0.0], [16.0, 8.0, 2.2]),
            b([16.0, 0.0, 0.0], [24.0, 8.0, 2.2]),
            b([1.0, 1.0, 0.5], [7.0, 7.0, 1.0]),
            b([0.0, 8.0, -5.0], [8.0, 16.0, 1.6]),
        ];
        let region = ([-0.5, -0.5, -0.5], [24.5, 16.5, 10.0]);
        let m = merge_boxes(boxes.clone(), region, None);
        assert_eq!(m.len(), 2, "{m:?}");
        for x in 0..50 {
            for y in 0..34 {
                for z in 0..21 {
                    let q = [
                        -0.25 + 0.5 * f64::from(x),
                        -0.25 + 0.5 * f64::from(y),
                        -0.25 + 0.5 * f64::from(z),
                    ];
                    let before = boxes.iter().any(|b| in_box(b, q));
                    assert_eq!(before, m.iter().any(|b| in_box(b, q)), "{q:?}");
                }
            }
        }
        // Boxes inside the shape are dropped; one poking out stays.
        let big = TrimShape {
            op: TrimOp::Add,
            kind: TrimKind::Cylinder {
                radii: [40.0, 40.0],
                height: 60.0,
            },
            rotation: id(),
            translation: [0.0, 0.0, 0.0],
        };
        let kept = merge_boxes(
            vec![
                b([0.0, 0.0, 0.0], [8.0, 8.0, 2.0]),
                b([30.0, 0.0, 0.0], [45.0, 8.0, 2.0]),
            ],
            ([-1.0; 3], [50.0, 50.0, 50.0]),
            Some(std::slice::from_ref(&big)),
        );
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].min[0], 30.0);
        // Not inside when something is subtracted near it afterwards.
        let hole = TrimShape {
            op: TrimOp::Subtract,
            kind: TrimKind::Box {
                size: [2.0, 2.0, 2.0],
            },
            rotation: id(),
            translation: [4.0, 4.0, 1.0],
        };
        assert!(!box_in_s(
            &[big.clone(), hole],
            [0.0, 0.0, 0.0],
            [8.0, 8.0, 2.0]
        ));
        assert!(box_in_s(&[big], [0.0, 0.0, 0.0], [8.0, 8.0, 2.0]));
    }

    #[test]
    fn far_shapes_are_left_out() {
        let near = TrimShape {
            op: TrimOp::Add,
            kind: TrimKind::Ellipsoid {
                radii: [5.0, 5.0, 5.0],
            },
            rotation: id(),
            translation: [0.0, 0.0, 0.0],
        };
        let far = TrimShape {
            translation: [100.0, 0.0, 0.0],
            ..near.clone()
        };
        let far_hole = TrimShape {
            op: TrimOp::Subtract,
            ..far.clone()
        };
        let lead_hole = TrimShape {
            op: TrimOp::Subtract,
            ..near.clone()
        };
        let kept = shapes_near(
            &[lead_hole, far.clone(), near.clone(), far_hole],
            ([-10.0; 3], [10.0; 3]),
        );
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].translation, near.translation);
    }
}
