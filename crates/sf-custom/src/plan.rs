//! Turns a [`CustomShape`] into SolidCraft commands, with the volume each stage must reach.
//!
//! Every part is built the same way, so each stage's volume has a closed form:
//!
//! 1. **Block**: the solid outline, full height.
//! 2. **Hollow**: the underside cut out, leaving walls and a top (for slopes, a sloped roof).
//! 3. **Fillet** (with an [`edge_radius`](StudProfile::edge_radius)): the vertical outside
//!    edges rounded, each convex corner losing `(tan(φ/2) − φ/2)·r²·h` and each concave one
//!    gaining as much (`(1 − π/4)·r²·h` for the square corners every outline here has).
//! 4. **Tubes** (where there are any): rings standing in the hollow, full height; the part of a
//!    tube that reaches above the hollow's ceiling lies in solid material and adds nothing.
//! 5. **Ribs** (2-wide boxes): a cross-wall through every second tube.
//! 6. **Chamfer**: the bottom outer edges (straight and rounded) and tube bottoms.
//! 7. **WallRibs** and **RibLeadIns** (with [`wall_ribs`](StudProfile::wall_ribs)): thin ribs
//!    on the inner faces of the walls (every wall at each stud for round studs, the end walls
//!    either side of each stud for fused ones), then a lead-in chamfer on each rib's lower
//!    inner edge.
//! 8. **Pockets** (fused studs): a pocket in the top under each fin.
//! 9. **Studs** on top (fused studs: pairs and triples); then **Slots** that cut the fins
//!    free.
//! 10. **Roof** (slopes and curves only): the wedge above the slope cut away (less the bits of
//!     it the rounded front corners had already taken).

use crate::geom::{self, Loop, Seg};
use crate::profile::{
    FUSED_RIB_OFFSET, RIB_GAP, RIB_THICKNESS, StudProfile, StudStyle, WALL_RIB_WIDTH,
};
use crate::shape::{CustomShape, Footprint, P2, signed_area};
use crate::label::{self, Keepout, LABEL_DEPTH};
use crate::slant::{self, Lobe, Unit};
use crate::{CustomError, MAX_PART_MM, Result};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::f64::consts::{FRAC_PI_2, PI, SQRT_2};

/// Relative volume tolerance used for every stage check (0.2 %, as SolidCraft's MCP `expect`).
pub const VOLUME_TOL: f64 = 0.002;
/// Bounding-box tolerance per coordinate, in mm (as SolidCraft's MCP `expect`).
pub const BBOX_TOL_MM: f64 = 0.05;
/// The lowest front lip a slope or curve keeps, in mm.
const MIN_LIP: f64 = 1.0;
/// How far cutters reach past the faces they cut, in mm, so no cut ends flush on a face.
const OVERRUN: f64 = 1.0;
/// How far a wall rib reaches up into the top above the hollow, in mm (a Slant v2 pocket
/// leaves at least 0.8 mm of floor, so this stays clear of it).
const WALL_RIB_TOP_EMBED: f64 = 0.3;

/// Stud-grid cells or grid points `(i, j)`.
type Cells = Vec<(i32, i32)>;
/// A plan-view extent `(min, max)`, mm.
type Extent2 = ([f64; 2], [f64; 2]);

/// One stage of a build: its commands and the total volume the part must have after them.
#[derive(Clone, Debug, Serialize)]
pub struct Stage {
    /// What the stage makes ("Block", "Hollow", "Fillet", "Tubes", "Ribs", "Chamfer",
    /// "WallRibs", "RibLeadIns", "Pockets", "Studs", "Slots", "Roof").
    pub name: String,
    /// SolidCraft commands, each `{"command": id, "params": {...}}`.
    pub commands: Vec<Value>,
    /// Total part volume after this stage, mm³.
    pub volume_mm3: f64,
}

/// A complete build: its stages, and the finished part's volume and bounding box.
#[derive(Clone, Debug, Serialize)]
pub struct Plan {
    /// The stages in order.
    pub stages: Vec<Stage>,
    /// Finished volume, mm³ (the last stage's).
    pub volume_mm3: f64,
    /// Finished bounding box `(min, max)`, mm.
    pub bbox_mm: ([f64; 3], [f64; 3]),
    /// The text engraved under the top (a [`CustomShape::Printed`] label), if any.
    pub label: Option<String>,
}

impl Plan {
    /// The whole build as one SolidCraft batch, `{"commands": [...], "expect": {...}}`: the
    /// arguments SolidCraft's MCP `batch` tool takes, with the finished volume, body count
    /// and bounding box as its `expect`.
    pub fn batch(&self) -> Value {
        let commands: Vec<Value> = self
            .stages
            .iter()
            .flat_map(|s| s.commands.iter().cloned())
            .collect();
        json!({
            "commands": commands,
            "expect": {
                "volume_mm3": round6(self.volume_mm3),
                "tol": VOLUME_TOL,
                "bodies": 1,
                "bbox": {"min": self.bbox_mm.0.map(round6), "max": self.bbox_mm.1.map(round6)},
            },
        })
    }
}

fn round6(v: f64) -> f64 {
    (v * 1e6).round() / 1e6
}

/// Collects stages, keeping the running volume.
struct Builder {
    stages: Vec<Stage>,
    volume: f64,
}

impl Builder {
    fn stage(&mut self, name: &str, commands: Vec<Value>, delta: f64) {
        self.volume += delta;
        self.stages.push(Stage {
            name: name.into(),
            commands,
            volume_mm3: self.volume,
        });
    }
}

fn cmd(id: &str, params: Value) -> Value {
    json!({"command": id, "params": params})
}

/// `sketch.create` on a principal plane moved `offset` mm along its normal.
fn sketch(name: &str, plane: &str, offset: f64) -> Value {
    cmd(
        "sketch.create",
        json!({"plane": plane, "offset": format!("{} mm", round6(offset)), "name": name}),
    )
}

fn finish() -> Value {
    cmd("sketch.finish", json!({}))
}

fn extrude(sketch: &str, name: &str, distance: f64, operation: &str) -> Value {
    cmd(
        "solid.extrude",
        json!({"sketch": sketch, "name": name, "distance": round6(distance), "operation": operation}),
    )
}

fn polyline(points: &[P2]) -> Value {
    let pts: Vec<Value> = points
        .iter()
        .map(|p| json!([round6(p[0]), round6(p[1])]))
        .collect();
    cmd("sketch.line", json!({"points": pts, "closed": true}))
}

fn open_polyline(points: &[P2]) -> Value {
    let pts: Vec<Value> = points
        .iter()
        .map(|p| json!([round6(p[0]), round6(p[1])]))
        .collect();
    cmd("sketch.line", json!({"points": pts}))
}

/// A circular arc from `start` to `end` about `center` (both ends at the same radius), drawn
/// through its midpoint so it passes exactly through both ends.
fn arc(start: P2, end: P2, center: P2) -> Value {
    let r = (start[0] - center[0]).hypot(start[1] - center[1]);
    let a0 = (start[1] - center[1]).atan2(start[0] - center[0]);
    let a1 = (end[1] - center[1]).atan2(end[0] - center[0]);
    // The shorter way round (roof arcs are quarter circles).
    let mut da = a1 - a0;
    while da > PI {
        da -= 2.0 * PI;
    }
    while da < -PI {
        da += 2.0 * PI;
    }
    let mid = a0 + da / 2.0;
    let through = [center[0] + r * mid.cos(), center[1] + r * mid.sin()];
    cmd(
        "sketch.arc.three_point",
        json!({"start": [round6(start[0]), round6(start[1])], "end": [round6(end[0]), round6(end[1])], "through": [round6(through[0]), round6(through[1])]}),
    )
}

fn circle(c: P2, diameter: f64) -> Value {
    cmd(
        "sketch.circle.center",
        json!({"center": [round6(c[0]), round6(c[1])], "diameter": round6(diameter)}),
    )
}

/// Area of a disk of radius `r` inside the strip `|y| ≤ a`.
fn disk_in_strip(r: f64, a: f64) -> f64 {
    if a >= r {
        return PI * r * r;
    }
    2.0 * (a * (r * r - a * a).sqrt() + r * r * (a / r).asin())
}

/// `∬ f(y) dA` over a disk of radius `r` centred at `y = y0` (Simpson's rule on
/// `y = y0 + r·sin θ`, which takes away the square-root ends of the chords).
fn disk_integral(r: f64, y0: f64, f: &dyn Fn(f64) -> f64) -> f64 {
    const N: usize = 4000;
    let h = PI / N as f64;
    let g = |k: usize| {
        let t = -FRAC_PI_2 + h * k as f64;
        let c = t.cos();
        f(y0 + r * t.sin()) * 2.0 * r * r * c * c
    };
    let mut s = g(0) + g(N);
    for k in 1..N {
        s += g(k) * if k % 2 == 1 { 4.0 } else { 2.0 };
    }
    s * h / 3.0
}

/// `∫ f` over `[a, b]` by Simpson's rule on `n` (even) intervals.
fn simpson(f: &dyn Fn(f64) -> f64, a: f64, b: f64, n: usize) -> f64 {
    let h = (b - a) / n as f64;
    let mut s = f(a) + f(b);
    for k in 1..n {
        s += f(a + h * k as f64) * if k % 2 == 1 { 4.0 } else { 2.0 };
    }
    s * h / 3.0
}

/// One corner of an outline: where it is, the unit vectors from it along its two edges, its
/// turning angle `φ` and whether it is convex (material inside the turn).
struct Corner {
    at: P2,
    u_prev: P2,
    u_next: P2,
    phi: f64,
    convex: bool,
}

fn corners(l: &[P2]) -> Vec<Corner> {
    let n = l.len();
    (0..n)
        .map(|k| {
            let (a, q, b) = (l[(k + n - 1) % n], l[k], l[(k + 1) % n]);
            let unit = |p: P2| {
                let d = [p[0] - q[0], p[1] - q[1]];
                let len = d[0].hypot(d[1]).max(f64::MIN_POSITIVE);
                [d[0] / len, d[1] / len]
            };
            let (u0, u1) = (unit(a), unit(b));
            // Travel directions in and out of the corner.
            let (din, dout) = ([-u0[0], -u0[1]], u1);
            let cross = din[0] * dout[1] - din[1] * dout[0];
            let dot = din[0] * dout[0] + din[1] * dout[1];
            Corner {
                at: q,
                u_prev: u0,
                u_next: u1,
                phi: cross.atan2(dot).abs(),
                convex: cross > 0.0,
            }
        })
        .collect()
}

impl Corner {
    /// The middle of a round of radius `r` in this corner.
    fn arc_mid(&self, r: f64) -> P2 {
        let b = [
            self.u_prev[0] + self.u_next[0],
            self.u_prev[1] + self.u_next[1],
        ];
        let bl = b[0].hypot(b[1]);
        let b = [b[0] / bl, b[1] / bl];
        // The round's centre is r / cos(φ/2) from the corner along the bisector.
        let d = r / (self.phi / 2.0).cos() - r;
        [self.at[0] + b[0] * d, self.at[1] + b[1] * d]
    }

    /// Area a round of radius `r` takes off a convex corner (or adds to a concave one).
    fn round_area(&self, r: f64) -> f64 {
        r * r * ((self.phi / 2.0).tan() - self.phi / 2.0)
    }
}

/// The fused units on top of a box or 45° slope, as stud cells `(i, j)` (X, Y) in row order:
/// what the part's studs group into (pairs, and one triple for an odd run). Other shapes are
/// refused. `sf-core` keeps its own copy of this rule for the packer; a test in `sf-agent`
/// checks the two agree.
pub fn fused_units(shape: &CustomShape, profile: &StudProfile) -> Result<Vec<Vec<(i32, i32)>>> {
    let (cells, studs): (BTreeSet<(i32, i32)>, Cells) = match shape {
        CustomShape::Box { w, d, .. } => {
            let (w, d) = dims(*w, *d, 1)?;
            let studs = grid(0..w, 0..d);
            (studs.iter().copied().collect(), studs)
        }
        CustomShape::Slope45 { w, d, plates } => {
            let (w, d) = dims(*w, *d, 2)?;
            let roof = Roof::new(profile, d, profile.body_height(*plates), false)?;
            let studs = grid(0..w, 0..d)
                .into_iter()
                .filter(|&(_, j)| f64::from(j) * profile.pitch >= roof.y_top - 1e-9)
                .collect();
            (grid(0..w, 0..d).into_iter().collect(), studs)
        }
        _ => {
            return Err(CustomError::InvalidShape(
                "fused_units: only boxes and 45° slopes".into(),
            ));
        }
    };
    Ok(slant::layout(&cells, &studs)?
        .into_iter()
        .map(|u| {
            (0..u.n as i32)
                .map(|k| (u.first.0 + k * u.step.0, u.first.1 + k * u.step.1))
                .collect()
        })
        .collect())
}

/// The plan for `shape` under `profile`: commands, stage volumes and bounding box.
pub fn plan(shape: &CustomShape, profile: &StudProfile) -> Result<Plan> {
    profile.validate()?;
    let plates = shape.plates();
    if plates == 0 {
        return Err(CustomError::InvalidShape(
            "a part must be at least one plate tall".into(),
        ));
    }
    let p = profile;
    let h = p.body_height(plates);
    let hollow_h = h - p.top_thickness;
    let mut b = Builder {
        stages: Vec::new(),
        volume: 0.0,
    };

    // Cells with studs, tube points (grid units), the plan's footprint extents, every cell of
    // the part, the outline loops (mm, material on the left) and the centre ribs.
    let mut cells: BTreeSet<(i32, i32)> = BTreeSet::new();
    let mut outline: Vec<Vec<P2>> = Vec::new();
    let mut ribs: Option<Ribs> = None;
    // Units given by the caller (printed parts), the hollow's outline (for placing a label)
    // and the label to engrave.
    let mut given: Option<Vec<Unit>> = None;
    let mut hollow_loops: Vec<Vec<P2>> = Vec::new();
    let mut label_text: Option<&str> = None;
    let (studs, tubes, bbox_xy, roof): (Cells, Cells, Extent2, Option<Roof>) = match shape {
        CustomShape::Box { w, d, .. } => {
            let (w, d) = dims(*w, *d, 1)?;
            block_and_hollow(&mut b, p, w, d, h, hollow_h);
            let studs = grid(0..w, 0..d);
            let tubes = grid(1..w, 1..d);
            let ext = rect_extent(p, w, d);
            cells.extend(studs.iter().copied());
            outline.push(rect_loop(ext));
            if p.center_ribs && w.min(d) == 2 && w.max(d) >= 3 && hollow_h - RIB_GAP >= 0.4 {
                ribs = Some(Ribs {
                    along_y: d > w,
                    long: w.max(d),
                });
            }
            (studs, tubes, ext, None)
        }
        CustomShape::Slope45 { w, d, .. } | CustomShape::Curve { w, d, .. } => {
            let (w, d) = dims(*w, *d, 2)?;
            let curved = matches!(shape, CustomShape::Curve { .. });
            let roof = Roof::new(p, d, h, curved)?;
            // Block.
            let len = f64::from(w) * p.pitch - 2.0 * p.clearance;
            let depth = f64::from(d) * p.pitch - 2.0 * p.clearance;
            b.stage(
                "Block",
                vec![cmd("solid.box", json!({"length": round6(len), "width": round6(depth), "height": round6(h), "corner": [round6(p.clearance), round6(p.clearance), 0.0], "name": "Block", "body_name": "Part"}))],
                len * depth * h,
            );
            // Hollow: a prism along X whose section follows the roof.
            let inner_len = len - 2.0 * p.wall;
            let section = roof.hollow_section(p, d);
            let mut hollow = vec![sketch("HollowSketch", "YZ", p.clearance + p.wall)];
            hollow.extend(roof.section_commands(&section));
            hollow.push(finish());
            hollow.push(extrude("HollowSketch", "Hollow", inner_len, "cut"));
            b.stage("Hollow", hollow, -roof.hollow_area(p, d) * inner_len);
            let tubes = grid(1..w, 1..d);
            let studs: Vec<(i32, i32)> = grid(0..w, 0..d)
                .into_iter()
                .filter(|&(_, j)| f64::from(j) * p.pitch >= roof.y_top - 1e-9)
                .collect();
            cells.extend(grid(0..w, 0..d));
            let ext = rect_extent(p, w, d);
            outline.push(rect_loop(ext));
            (studs, tubes, ext, Some(roof))
        }
        CustomShape::Footprint { cells: list, .. } => {
            let f = Footprint::new(list)?;
            let outer = f.offset_loops(p.pitch, p.clearance);
            let inner = f.offset_loops(p.pitch, p.clearance + p.wall);
            let area: f64 = outer.iter().map(|l| signed_area(l)).sum();
            let inner_area: f64 = inner.iter().map(|l| signed_area(l)).sum();
            let mut block = vec![sketch("BlockSketch", "XY", 0.0)];
            block.extend(outer.iter().map(|l| polyline(l)));
            block.push(finish());
            block.push(extrude("BlockSketch", "Block", h, "new"));
            b.stage("Block", block, area * h);
            let mut hollow = vec![sketch("HollowSketch", "XY", -OVERRUN)];
            hollow.extend(inner.iter().map(|l| polyline(l)));
            hollow.push(finish());
            hollow.push(extrude("HollowSketch", "Hollow", hollow_h + OVERRUN, "cut"));
            b.stage("Hollow", hollow, -inner_area * hollow_h);
            let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
            for q in outer.iter().flatten() {
                for k in 0..2 {
                    lo[k] = lo[k].min(q[k]);
                    hi[k] = hi[k].max(q[k]);
                }
            }
            let studs: Vec<(i32, i32)> = f.cells.iter().copied().collect();
            cells.clone_from(&f.cells);
            outline = outer;
            (studs, f.full_corners(), (lo, hi), None)
        }
        CustomShape::Printed {
            cells: list,
            units,
            label,
            ..
        } => {
            // Move the cells (and units) so the smallest i and j are 0.
            let i0 = list.iter().map(|c| c.0).min().unwrap_or(0);
            let j0 = list.iter().map(|c| c.1).min().unwrap_or(0);
            let shift = |c: &(i32, i32)| (c.0 - i0, c.1 - j0);
            let list: Vec<(i32, i32)> = list.iter().map(shift).collect();
            let units: Vec<Vec<(i32, i32)>> =
                units.iter().map(|u| u.iter().map(shift).collect()).collect();
            let f = Footprint::new(&list)?;
            let outer = f.offset_loops(p.pitch, p.clearance);
            let inner = f.offset_loops(p.pitch, p.clearance + p.wall);
            let area: f64 = outer.iter().map(|l| signed_area(l)).sum();
            let inner_area: f64 = inner.iter().map(|l| signed_area(l)).sum();
            let mut block = vec![sketch("BlockSketch", "XY", 0.0)];
            block.extend(outer.iter().map(|l| polyline(l)));
            block.push(finish());
            block.push(extrude("BlockSketch", "Block", h, "new"));
            b.stage("Block", block, area * h);
            let mut hollow = vec![sketch("HollowSketch", "XY", -OVERRUN)];
            hollow.extend(inner.iter().map(|l| polyline(l)));
            hollow.push(finish());
            hollow.push(extrude("HollowSketch", "Hollow", hollow_h + OVERRUN, "cut"));
            b.stage("Hollow", hollow, -inner_area * hollow_h);
            let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
            for q in outer.iter().flatten() {
                for k in 0..2 {
                    lo[k] = lo[k].min(q[k]);
                    hi[k] = hi[k].max(q[k]);
                }
            }
            cells.clone_from(&f.cells);
            let given_units = slant::units_from(&f.cells, &units)?;
            let studs: Vec<(i32, i32)> = given_units.iter().flat_map(Unit::cells).collect();
            // A 2-wide rectangle gets centre ribs, as a box does.
            let (w, d) = (
                f.cells.iter().map(|c| c.0).max().unwrap_or(0) + 1,
                f.cells.iter().map(|c| c.1).max().unwrap_or(0) + 1,
            );
            let rect = f.cells.len() == (w * d) as usize;
            if rect && p.center_ribs && w.min(d) == 2 && w.max(d) >= 3 && hollow_h - RIB_GAP >= 0.4
            {
                ribs = Some(Ribs {
                    along_y: d > w,
                    long: w.max(d),
                });
            }
            given = Some(given_units);
            hollow_loops = inner;
            label_text = label.as_deref();
            outline = outer;
            (studs, f.full_corners(), (lo, hi), None)
        }
    };

    // Fillet: the vertical outside edges, picked half-way up (before any roof is cut, every
    // one of them runs the full height).
    let er = p.edge_radius;
    if er > 0.0 {
        let mut picks = Vec::new();
        let mut delta = 0.0;
        for l in &outline {
            for k in corners(l) {
                picks.push(json!([round6(k.at[0]), round6(k.at[1]), round6(h / 2.0)]));
                let sign = if k.convex { -1.0 } else { 1.0 };
                delta += sign * k.round_area(er) * h;
            }
        }
        b.stage(
            "Fillet",
            vec![cmd(
                "solid.fillet",
                json!({"edges": picks, "radius": round6(er), "name": "EdgeRounds"}),
            )],
            delta,
        );
    }

    let size = [
        bbox_xy.1[0] - bbox_xy.0[0],
        bbox_xy.1[1] - bbox_xy.0[1],
        h + p.stud_height,
    ];
    if size.iter().any(|s| *s > MAX_PART_MM) {
        return Err(CustomError::TooLarge(format!(
            "{} would be {:.1} × {:.1} × {:.1} mm; custom parts are limited to {MAX_PART_MM} mm a side",
            shape.label(),
            size[0],
            size[1],
            size[2]
        )));
    }

    // Tubes: rings from the floor to the underside of the top.
    let (ro, ri) = (p.tube_outer_diameter / 2.0, p.tube_inner_diameter / 2.0);
    let tube_centres: Vec<P2> = tubes
        .iter()
        .map(|&(i, j)| [f64::from(i) * p.pitch, f64::from(j) * p.pitch])
        .collect();
    if !tubes.is_empty() {
        let mut c = vec![sketch("TubeSketch", "XY", 0.0)];
        let mut added = 0.0;
        for &at in &tube_centres {
            c.push(circle(at, p.tube_outer_diameter));
            c.push(circle(at, p.tube_inner_diameter));
            added += match &roof {
                // Only the part below the hollow's ceiling is new material.
                Some(roof) => {
                    let ceil = |y: f64| roof.ceiling(p, y).clamp(0.0, hollow_h);
                    disk_integral(ro, at[1], &ceil) - disk_integral(ri, at[1], &ceil)
                }
                None => PI * (ro * ro - ri * ri) * hollow_h,
            };
        }
        c.push(finish());
        c.push(extrude("TubeSketch", "Tubes", hollow_h, "join"));
        b.stage("Tubes", c, added);
    }

    if let Some(ribs) = ribs {
        ribs.stage(&mut b, p, hollow_h);
    }

    if p.bottom_chamfer > 0.0 {
        let c = p.bottom_chamfer;
        let mut picks = Vec::new();
        let mut removed = 0.0;
        for l in &outline {
            let n = l.len();
            for k in 0..n {
                let (a, q) = (l[k], l[(k + 1) % n]);
                // A round shortens both ends of a straight edge alike: its middle stays put.
                picks.push(json!([
                    round6((a[0] + q[0]) / 2.0),
                    round6((a[1] + q[1]) / 2.0),
                    0.0
                ]));
            }
            if er > 0.0 {
                for k in corners(l) {
                    let m = k.arc_mid(er);
                    picks.push(json!([round6(m[0]), round6(m[1]), 0.0]));
                }
            }
            removed += chamfer_loop_volume(l, c, er);
        }
        for at in &tube_centres {
            picks.push(json!([round6(at[0] + ro), round6(at[1]), 0.0]));
            picks.push(json!([round6(at[0] + ri), round6(at[1]), 0.0]));
            // Pappus: a c × c triangle turned about the tube axis.
            removed += PI * c * c * (ro - c / 3.0) + PI * c * c * (ri + c / 3.0);
        }
        b.stage(
            "Chamfer",
            vec![cmd(
                "solid.chamfer",
                json!({"edges": picks, "distance": round6(c), "name": "BottomChamfer"}),
            )],
            -removed,
        );
    }

    let along_y = match &given {
        Some(units) => {
            let ys = units.iter().filter(|u| u.step == (0, 1)).count();
            if p.wall_ribs && ys != 0 && ys != units.len() {
                return Err(CustomError::InvalidShape(
                    "wall ribs need every fused unit to run the same way".into(),
                ));
            }
            Some(ys == units.len())
        }
        None => None,
    };
    if p.wall_ribs {
        wall_ribs(&mut b, p, &cells, &studs, along_y, hollow_h, roof.as_ref())?;
    }

    // The stud units, the stud profile's lobe and its finned lobes' frames.
    let slant_parts = match p.stud_style {
        StudStyle::SlantV2 { .. } => {
            let lobe = Lobe::new(p)?;
            let units = match &given {
                Some(u) => u.clone(),
                None => slant::layout(&cells, &studs)?,
            };
            let lobes: Vec<_> = units.iter().flat_map(|q| q.lobe_frames(p.pitch)).collect();
            Some((lobe, units, lobes))
        }
        StudStyle::Solid => None,
    };

    // Label: engraved into the ceiling where it is bare (see `label`).
    let mut engraved = None;
    if let Some(text) = label_text {
        let tube_centres: Vec<P2> = tubes
            .iter()
            .map(|&(i, j)| [f64::from(i) * p.pitch, f64::from(j) * p.pitch])
            .collect();
        let mut rects: Vec<(P2, P2)> = ribs.map(|r| r.bars(p)).unwrap_or_default();
        if p.wall_ribs {
            rects.extend(
                wall_rib_sites(p, &cells, &studs, along_y)?
                    .into_iter()
                    .map(|s| (s.lo, s.hi)),
            );
        }
        let cuts: Vec<Loop> = slant_parts
            .as_ref()
            .map(|(lobe, _, lobes)| {
                lobes
                    .iter()
                    .flat_map(|f| [f.map_loop(&lobe.pocket), f.map_loop(&lobe.slot)])
                    .collect()
            })
            .unwrap_or_default();
        let keep = Keepout {
            hollow: &hollow_loops,
            tubes: &tube_centres,
            tube_radius: p.tube_outer_diameter / 2.0,
            rects: &rects,
            cuts: &cuts,
        };
        if let Some(e) = label::engrave(text, &keep) {
            let mut c = vec![sketch("LabelSketch", "XY", hollow_h - OVERRUN)];
            c.extend(e.loops.iter().map(|l| polyline(l)));
            c.push(finish());
            c.push(extrude("LabelSketch", "Label", OVERRUN + LABEL_DEPTH, "cut"));
            b.stage("Label", c, -e.area * LABEL_DEPTH);
            engraved = Some(e.text);
        }
    }

    let rs = p.stud_diameter / 2.0;
    let centre = |&(i, j): &(i32, i32)| {
        [
            (f64::from(i) + 0.5) * p.pitch,
            (f64::from(j) + 0.5) * p.pitch,
        ]
    };
    match p.stud_style {
        StudStyle::Solid if studs.is_empty() => {}
        StudStyle::Solid => {
            let mut c = vec![sketch("StudSketch", "XY", h)];
            c.extend(studs.iter().map(|q| circle(centre(q), p.stud_diameter)));
            c.push(finish());
            c.push(extrude("StudSketch", "Studs", p.stud_height, "join"));
            b.stage(
                "Studs",
                c,
                PI * rs * rs * p.stud_height * studs.len() as f64,
            );
        }
        StudStyle::SlantV2 { .. } => {
            let Some((lobe, units, lobes)) = slant_parts else {
                unreachable!("worked out above for fused studs")
            };
            if units.is_empty() {
                // A printed part with no studs at all (a flat top).
            } else {
            if !lobes.is_empty() {
                // Pockets first, into the bare top, so nothing above them is cut.
                let mut c = vec![sketch("PocketSketch", "XY", h - lobe.pocket_depth)];
                for f in &lobes {
                    c.extend(loop_commands(&f.map_loop(&lobe.pocket)));
                }
                c.push(finish());
                c.push(extrude(
                    "PocketSketch",
                    "Pockets",
                    lobe.pocket_depth + OVERRUN,
                    "cut",
                ));
                b.stage(
                    "Pockets",
                    c,
                    -lobe.pocket_area() * lobe.pocket_depth * lobes.len() as f64,
                );
            }
            let mut c = vec![sketch("StudSketch", "XY", h)];
            let mut area = 0.0;
            for q in &units {
                let outline = lobe.outline(q.n, q.fin.is_some());
                c.extend(loop_commands(&q.frame(p.pitch).map_loop(outline)));
                area += geom::area(outline);
            }
            c.push(finish());
            c.push(extrude("StudSketch", "Studs", p.stud_height, "join"));
            b.stage("Studs", c, area * p.stud_height);
            if !lobes.is_empty() {
                let mut c = vec![sketch("SlotSketch", "XY", h - lobe.groove)];
                for f in &lobes {
                    c.extend(loop_commands(&f.map_loop(&lobe.slot)));
                }
                c.push(finish());
                c.push(extrude(
                    "SlotSketch",
                    "Slots",
                    lobe.groove + p.stud_height + OVERRUN,
                    "cut",
                ));
                b.stage(
                    "Slots",
                    c,
                    -lobe.slot_volume(p.stud_height) * lobes.len() as f64,
                );
            }
            }
        }
    }

    if let (Some(roof), CustomShape::Slope45 { w, .. } | CustomShape::Curve { w, .. }) =
        (&roof, shape)
    {
        let len = f64::from(*w) * p.pitch - 2.0 * p.clearance;
        let mut c = vec![sketch("RoofSketch", "YZ", p.clearance - OVERRUN)];
        c.extend(roof.wedge_commands(h));
        c.push(finish());
        c.push(extrude("RoofSketch", "Roof", len + 2.0 * OVERRUN, "cut"));
        // The two rounded front corners had already lost their share of the wedge.
        let rounded = if er > 0.0 {
            2.0 * roof.wedge_over_round(er)
        } else {
            0.0
        };
        b.stage("Roof", c, -roof.wedge_area() * len + rounded);
    }

    let top = if studs.is_empty() { h } else { h + p.stud_height };
    let bbox = (
        [bbox_xy.0[0], bbox_xy.0[1], 0.0],
        [bbox_xy.1[0], bbox_xy.1[1], top],
    );
    let volume = b.volume;
    Ok(Plan {
        stages: b.stages,
        volume_mm3: volume,
        bbox_mm: bbox,
        label: engraved,
    })
}

/// The commands drawing a closed outline of lines and arcs.
fn loop_commands(l: &Loop) -> Vec<Value> {
    l.iter()
        .map(|s| match *s {
            Seg::Line { a, b } => open_polyline(&[a, b]),
            Seg::Arc { a, b, .. } => {
                let m = s.at(0.5);
                cmd(
                    "sketch.arc.three_point",
                    json!({"start": [round6(a[0]), round6(a[1])], "end": [round6(b[0]), round6(b[1])], "through": [round6(m[0]), round6(m[1])]}),
                )
            }
        })
        .collect()
}

/// A rectangle's outline, counter-clockwise.
fn rect_loop((lo, hi): Extent2) -> Vec<P2> {
    vec![lo, [hi[0], lo[1]], hi, [lo[0], hi[1]]]
}

/// Volume an equal-distance chamfer `c` takes off the bottom edges of a prism with this
/// outline (material on the left, no collinear points), its corners rounded to radius `r`
/// (0 for sharp). At height `z < c` the section is the outline moved `δ = c − z` into the
/// material, which loses `perimeter·δ − K·δ²`: a sharp corner turning by `φ` adds
/// `tan(φ/2)` to `K` (mitred), a rounded one `φ/2` (its arc shrinks with `δ`, which needs
/// `r ≥ c`), negative for concave corners. A round of radius `r` shortens the perimeter by
/// `2·r·tan(φ/2) − r·φ` per corner. Integrated over `z`: `perimeter·c²/2 − K·c³/3`.
fn chamfer_loop_volume(l: &[P2], c: f64, r: f64) -> f64 {
    let n = l.len();
    let mut perimeter = 0.0;
    let mut k_sum = 0.0;
    for k in 0..n {
        let (a, q) = (l[k], l[(k + 1) % n]);
        perimeter += (q[0] - a[0]).hypot(q[1] - a[1]);
    }
    for k in corners(l) {
        let sign = if k.convex { 1.0 } else { -1.0 };
        let half = k.phi / 2.0;
        if r > 0.0 {
            perimeter -= 2.0 * r * half.tan() - r * k.phi;
            k_sum += sign * half;
        } else {
            k_sum += sign * half.tan();
        }
    }
    perimeter * c * c / 2.0 - k_sum * c * c * c / 3.0
}

/// One wall rib: the rectangle it stands on (from half-way into the wall to its face), the
/// unit normal of its face (into the hollow), the middle of its lower inner edge, and how
/// far it stands proud of the wall.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RibSite {
    pub lo: P2,
    pub hi: P2,
    pub n: P2,
    pub edge: P2,
    pub proud: f64,
}

impl RibSite {
    /// The protruding part's extent along Y (the only direction a roof's ceiling varies).
    fn y_span(&self) -> (f64, f64) {
        if self.n[1] > 0.0 {
            (self.edge[1] - self.proud, self.edge[1])
        } else if self.n[1] < 0.0 {
            (self.edge[1], self.edge[1] + self.proud)
        } else {
            (self.lo[1], self.hi[1])
        }
    }

    /// The protruding part's width across Y.
    fn x_width(&self) -> f64 {
        if self.n[1] == 0.0 {
            self.proud
        } else {
            WALL_RIB_WIDTH
        }
    }
}

/// Where the wall ribs of a part made of `cells` go, by its stud style.
///
/// * **Round studs** (LEGO's layout): on the inner face of the wall along every edge of a
///   cell that has no neighbour, one rib centred on the cell's stud, so a stud pushed in
///   from below meets it square on.
/// * **Fused studs**: only on the **end walls**, the cell edges whose normal runs along the
///   rows (see [`slant::layout`]), so no rib stands on the long walls the fins of a brick
///   below press. A fused brick below turned 90° puts its fins against the end walls, so the
///   ribs keep off the stud centre line, where the fins reach past the stud circle: two per
///   cell edge, centred [`FUSED_RIB_OFFSET`] either side of it, where the fin side of a lobe
///   is within about 0.01 mm of the stud circle. See
///   [`StudProfile::wall_rib_protrusion`] for how proud they stand.
///
/// Every rib lies on a straight run of wall: a cell edge's middle is at least
/// `pitch/2 − clearance − wall` (2.7 mm) from any corner of the hollow, more than a rib
/// reaches from it, so on footprints ribs keep clear of convex and concave corners alike.
pub(crate) fn wall_rib_sites(
    p: &StudProfile,
    cells: &BTreeSet<(i32, i32)>,
    studs: &[(i32, i32)],
    along: Option<bool>,
) -> Result<Vec<RibSite>> {
    let t = p.wall_rib_protrusion();
    let hw = WALL_RIB_WIDTH / 2.0;
    let face = p.clearance + p.wall;
    let back = p.clearance + p.wall / 2.0;
    // Offsets of the ribs along each cell edge, and which edges get them (normal along X,
    // along Y).
    let (offsets, edges): (Vec<f64>, [bool; 2]) = match p.stud_style {
        StudStyle::Solid => (vec![0.0], [true, true]),
        StudStyle::SlantV2 { .. } => {
            let along_y = match along {
                Some(a) => a,
                None => slant::layout(cells, studs)?
                    .first()
                    .is_none_or(|u| u.step == (0, 1)),
            };
            (
                vec![-FUSED_RIB_OFFSET, FUSED_RIB_OFFSET],
                [!along_y, along_y],
            )
        }
    };
    let mut out = Vec::new();
    for &(i, j) in cells {
        let (x0, y0) = (f64::from(i) * p.pitch, f64::from(j) * p.pitch);
        let (xc, yc) = (x0 + p.pitch / 2.0, y0 + p.pitch / 2.0);
        let (x1, y1) = (x0 + p.pitch, y0 + p.pitch);
        for o in &offsets {
            let (xo, yo) = (xc + o, yc + o);
            // (neighbour, rib rectangle, unit normal, middle of its lower inner edge, normal
            // along Y?)
            let sides = [
                (
                    (i - 1, j),
                    [x0 + back, yo - hw],
                    [x0 + face + t, yo + hw],
                    [1.0, 0.0],
                    [x0 + face + t, yo],
                ),
                (
                    (i + 1, j),
                    [x1 - face - t, yo - hw],
                    [x1 - back, yo + hw],
                    [-1.0, 0.0],
                    [x1 - face - t, yo],
                ),
                (
                    (i, j - 1),
                    [xo - hw, y0 + back],
                    [xo + hw, y0 + face + t],
                    [0.0, 1.0],
                    [xo, y0 + face + t],
                ),
                (
                    (i, j + 1),
                    [xo - hw, y1 - face - t],
                    [xo + hw, y1 - back],
                    [0.0, -1.0],
                    [xo, y1 - face - t],
                ),
            ];
            for (nb, lo, hi, n, edge) in sides {
                let along_y = n[1] != 0.0;
                if cells.contains(&nb) || !edges[usize::from(along_y)] {
                    continue;
                }
                out.push(RibSite {
                    lo,
                    hi,
                    n,
                    edge,
                    proud: t,
                });
            }
        }
    }
    Ok(out)
}

/// Wall ribs (see [`wall_rib_sites`] for where): each stands
/// [`StudProfile::wall_rib_protrusion`] proud of the face, [`WALL_RIB_WIDTH`] wide, from the
/// floor into the top (embedded half a wall deep in the wall and [`WALL_RIB_TOP_EMBED`] into
/// the top), with a lead-in chamfer on its lower inner edge. Under a slope's roof a rib is
/// left out where the hollow's ceiling over it is lower than a stud, which could not reach
/// it.
fn wall_ribs(
    b: &mut Builder,
    p: &StudProfile,
    cells: &BTreeSet<(i32, i32)>,
    studs: &[(i32, i32)],
    along: Option<bool>,
    hollow_h: f64,
    roof: Option<&Roof>,
) -> Result<()> {
    let lead = p.wall_rib_lead_in();
    let ceil = |y: f64| roof.map_or(hollow_h, |r| r.ceiling(p, y).clamp(0.0, hollow_h));
    let mut sk = vec![sketch("WallRibSketch", "XY", 0.0)];
    let mut picks = Vec::new();
    let (mut added, mut count) = (0.0, 0usize);
    for site in wall_rib_sites(p, cells, studs, along)? {
        let (ya, yb) = site.y_span();
        let low = ceil(ya).min(ceil(yb)).min(ceil((ya + yb) / 2.0));
        if low < p.stud_height {
            continue;
        }
        sk.push(cmd(
            "sketch.rectangle.two_point",
            json!({"p0": [round6(site.lo[0]), round6(site.lo[1])], "p1": [round6(site.hi[0]), round6(site.hi[1])]}),
        ));
        picks.push(json!([round6(site.edge[0]), round6(site.edge[1]), 0.0]));
        added += site.x_width() * simpson(&ceil, ya, yb, 2000);
        count += 1;
    }
    if count == 0 {
        return Ok(());
    }
    sk.push(finish());
    sk.push(extrude(
        "WallRibSketch",
        "WallRibs",
        hollow_h + WALL_RIB_TOP_EMBED,
        "join",
    ));
    b.stage("WallRibs", sk, added);
    b.stage(
        "RibLeadIns",
        vec![cmd(
            "solid.chamfer",
            json!({"edges": picks, "distance": round6(lead), "name": "RibLeadIns"}),
        )],
        -(lead * lead / 2.0) * WALL_RIB_WIDTH * count as f64,
    );
    Ok(())
}

/// Centre ribs under a 2-wide box: one through every second tube along the long side.
#[derive(Clone, Copy, Debug)]
struct Ribs {
    /// The long side runs along Y (the ribs run along X).
    along_y: bool,
    /// Length of the long side, in studs.
    long: i32,
}

impl Ribs {
    /// The bars' rectangles in plan, `(lo, hi)` mm.
    fn bars(&self, p: &StudProfile) -> Vec<(P2, P2)> {
        let t = RIB_THICKNESS;
        let mid = (p.tube_outer_diameter + p.tube_inner_diameter) / 4.0;
        let x0 = p.clearance + p.wall / 2.0;
        let x1 = 2.0 * p.pitch - x0;
        let mut out = Vec::new();
        for k in (2..self.long).step_by(2) {
            let y = f64::from(k) * p.pitch;
            for (s0, s1) in [(x0, p.pitch - mid), (p.pitch + mid, x1)] {
                out.push(if self.along_y {
                    ([s0, y - t / 2.0], [s1, y + t / 2.0])
                } else {
                    ([y - t / 2.0, s0], [y + t / 2.0, s1])
                });
            }
        }
        out
    }

    /// Each rib is two bars, from inside each long wall to the middle of the tube wall, from
    /// [`RIB_GAP`] above the floor into the top.
    fn stage(&self, b: &mut Builder, p: &StudProfile, hollow_h: f64) {
        let t = RIB_THICKNESS;
        let (ro, ri) = (p.tube_outer_diameter / 2.0, p.tube_inner_diameter / 2.0);
        let mid = (ro + ri) / 2.0;
        let wall_in = p.clearance + p.wall;
        let x0 = p.clearance + p.wall / 2.0;
        let x1 = 2.0 * p.pitch - x0;
        let mut c = vec![sketch("RibSketch", "XY", RIB_GAP)];
        let mut bars = 0;
        for k in (2..self.long).step_by(2) {
            let y = f64::from(k) * p.pitch;
            for (s0, s1) in [(x0, p.pitch - mid), (p.pitch + mid, x1)] {
                let (p0, p1) = if self.along_y {
                    ([s0, y - t / 2.0], [s1, y + t / 2.0])
                } else {
                    ([y - t / 2.0, s0], [y + t / 2.0, s1])
                };
                c.push(cmd(
                    "sketch.rectangle.two_point",
                    json!({"p0": [round6(p0[0]), round6(p0[1])], "p1": [round6(p1[0]), round6(p1[1])]}),
                ));
                bars += 1;
            }
        }
        c.push(finish());
        c.push(extrude(
            "RibSketch",
            "Ribs",
            hollow_h - RIB_GAP + p.top_thickness / 2.0,
            "join",
        ));
        // New material: from the wall's inside face to the tube, less the bit inside the
        // tube's outer circle, up to the hollow's ceiling.
        let in_tube = disk_in_strip(ro, t / 2.0) / 2.0 - mid * t;
        let per_bar = ((p.pitch - mid - wall_in) * t - in_tube) * (hollow_h - RIB_GAP);
        b.stage("Ribs", c, per_bar * f64::from(bars));
    }
}

/// Checks a `w × d` size: at least 1 (or `min_d` deep), at most 64 studs.
fn dims(w: u32, d: u32, min_d: u32) -> Result<(i32, i32)> {
    if w == 0 || d < min_d.max(1) {
        return Err(CustomError::InvalidShape(format!(
            "size {w}x{d} is too small (at least 1 wide and {} deep)",
            min_d.max(1)
        )));
    }
    let w = i32::try_from(w.min(64)).unwrap_or(64);
    let d = i32::try_from(d.min(64)).unwrap_or(64);
    Ok((w, d))
}

fn grid(is: std::ops::Range<i32>, js: std::ops::Range<i32>) -> Vec<(i32, i32)> {
    is.flat_map(|i| js.clone().map(move |j| (i, j))).collect()
}

fn rect_extent(p: &StudProfile, w: i32, d: i32) -> ([f64; 2], [f64; 2]) {
    (
        [p.clearance, p.clearance],
        [
            f64::from(w) * p.pitch - p.clearance,
            f64::from(d) * p.pitch - p.clearance,
        ],
    )
}

/// Block and rectangular hollow of a plain brick.
fn block_and_hollow(b: &mut Builder, p: &StudProfile, w: i32, d: i32, h: f64, hollow_h: f64) {
    let len = f64::from(w) * p.pitch - 2.0 * p.clearance;
    let depth = f64::from(d) * p.pitch - 2.0 * p.clearance;
    b.stage(
        "Block",
        vec![cmd("solid.box", json!({"length": round6(len), "width": round6(depth), "height": round6(h), "corner": [round6(p.clearance), round6(p.clearance), 0.0], "name": "Block", "body_name": "Part"}))],
        len * depth * h,
    );
    let (il, id) = (len - 2.0 * p.wall, depth - 2.0 * p.wall);
    let inset = p.clearance + p.wall;
    b.stage(
        "Hollow",
        vec![cmd(
            "solid.box",
            json!({"length": round6(il), "width": round6(id), "height": round6(hollow_h + OVERRUN), "corner": [round6(inset), round6(inset), -OVERRUN], "operation": "cut", "name": "Hollow"}),
        )],
        -il * id * hollow_h,
    );
}

/// The sloped (or curved) roof of a slope brick, in the brick's Y–Z section.
///
/// The roof rises from the front lip `(clearance, lip)` to the ridge `(y_top, h)`, a run and
/// rise of `run` each: a 45° line, or a convex quarter circle of radius `run` about
/// `(y_top, lip)`. Studs stand only where the top is flat (`y ≥ y_top`).
#[derive(Clone, Copy, Debug)]
struct Roof {
    curved: bool,
    lip: f64,
    run: f64,
    y_top: f64,
    h: f64,
    front: f64,
}

impl Roof {
    fn new(p: &StudProfile, d: i32, h: f64, curved: bool) -> Result<Self> {
        // The roof covers every row but the back one, unless the brick is too low for that.
        let run = (f64::from(d - 1) * p.pitch - p.clearance).min(h - MIN_LIP);
        if run <= p.wall * 2.0 {
            return Err(CustomError::InvalidShape(format!(
                "a {}-plate slope is too low for a roof (needs a body over {:.1} mm tall)",
                (h / p.plate_height).round(),
                MIN_LIP + p.wall * 2.0
            )));
        }
        let lip = h - run;
        let roof = Self {
            curved,
            lip,
            run,
            y_top: p.clearance + run,
            h,
            front: p.clearance,
        };
        // The underside of the roof must stay above the floor at the front wall.
        if roof.ceiling(p, p.clearance + p.wall) <= 0.2 {
            return Err(CustomError::InvalidShape(
                "the slope's front lip is too low for its wall thickness".into(),
            ));
        }
        // The hollow must keep a flat stretch behind the roof, inside the back wall.
        let back_wall = f64::from(d) * p.pitch - p.clearance - p.wall;
        if roof.knee(p).max(roof.y_top) >= back_wall - 0.2 {
            return Err(CustomError::InvalidShape(
                "the slope leaves no room for the hollow behind its roof".into(),
            ));
        }
        Ok(roof)
    }

    /// Height of the hollow's ceiling at `y` (a wall's thickness under the roof, and the top's
    /// thickness under the flat top).
    fn ceiling(&self, p: &StudProfile, y: f64) -> f64 {
        let flat = self.h - p.top_thickness;
        if self.curved {
            // Under the arc, then a step up to the flat ceiling behind the ridge.
            if y >= self.y_top {
                return flat;
            }
            let r = self.run - p.wall;
            let dy = y - self.y_top;
            (self.lip + (r * r - dy * dy).max(0.0).sqrt()).min(flat)
        } else {
            // A wall's thickness (measured square to the roof) under the 45° line, until it
            // meets the flat ceiling a little behind the ridge.
            (self.lip + (y - self.front) - p.wall * SQRT_2).min(flat)
        }
    }

    /// The hollow's section (Y, Z) as a polygon, or for a curve the points of its outline with
    /// the arc running from `arc.0` to `arc.1` about `arc.2`.
    fn hollow_section(&self, p: &StudProfile, d: i32) -> Section {
        let y0 = self.front + p.wall;
        let y1 = f64::from(d) * p.pitch - p.clearance - p.wall;
        let flat = self.h - p.top_thickness;
        if self.curved {
            let r = self.run - p.wall;
            // Arc from the ridge side (y_top, lip + r) down to the front (y0, lip).
            Section::Curved {
                lines: vec![
                    [y0, self.lip],
                    [y0, -OVERRUN],
                    [y1, -OVERRUN],
                    [y1, flat],
                    [self.y_top, flat],
                    [self.y_top, self.lip + r],
                ],
                center: [self.y_top, self.lip],
            }
        } else {
            Section::Polygon(vec![
                [y0, -OVERRUN],
                [y1, -OVERRUN],
                [y1, flat],
                [self.knee(p), flat],
                [y0, self.ceiling(p, y0)],
            ])
        }
    }

    /// Area of the hollow's section above the floor (`z ≥ 0`).
    fn hollow_area(&self, p: &StudProfile, d: i32) -> f64 {
        let y0 = self.front + p.wall;
        let y1 = f64::from(d) * p.pitch - p.clearance - p.wall;
        let flat = self.h - p.top_thickness;
        if self.curved {
            // Behind the ridge: flat. In front: a rectangle up to the lip and a quarter disk.
            let r = self.run - p.wall;
            (y1 - self.y_top) * flat + (self.y_top - y0) * self.lip + PI * r * r / 4.0
        } else {
            // A trapezoid under the 45° line up to the knee, then flat.
            let c0 = self.ceiling(p, y0);
            let knee = self.knee(p);
            (knee - y0) * (c0 + flat) / 2.0 + (y1 - knee) * flat
        }
    }

    /// Where the underside of a 45° roof meets the flat ceiling.
    fn knee(&self, p: &StudProfile) -> f64 {
        self.h - p.top_thickness - self.lip + self.front + p.wall * SQRT_2
    }

    /// The commands drawing a hollow section.
    fn section_commands(&self, s: &Section) -> Vec<Value> {
        match s {
            Section::Polygon(pts) => vec![polyline(pts)],
            Section::Curved { lines, center } => {
                // The open chain of lines, then the arc closing it from its last point to its first.
                let (Some(first), Some(last)) = (lines.first(), lines.last()) else {
                    return Vec::new();
                };
                vec![open_polyline(lines), arc(*last, *first, *center)]
            }
        }
    }

    /// Height of the roof's outer surface at `y` (in front of the ridge).
    fn surface(&self, y: f64) -> f64 {
        if self.curved {
            let dy = y - self.y_top;
            self.lip + (self.run * self.run - dy * dy).max(0.0).sqrt()
        } else {
            self.lip + (y - self.front)
        }
    }

    /// Volume of the wedge above the roof over one front corner's round of radius `r` (the
    /// sliver between the square corner and the round, which the fillet already took away):
    /// `∬ (h − surface(y)) dA` over it. With `u = r·sin θ` measured back from the round's
    /// centre line, the sliver is `r·(1 − cos θ)` wide, which takes away the square-root end.
    fn wedge_over_round(&self, r: f64) -> f64 {
        let f = |t: f64| {
            let y = self.front + r - r * t.sin();
            r * (1.0 - t.cos()) * (self.h - self.surface(y)) * r * t.cos()
        };
        simpson(&f, 0.0, FRAC_PI_2, 4000)
    }

    /// Area of the wedge the roof cut takes off the block's section.
    fn wedge_area(&self) -> f64 {
        if self.curved {
            self.run * self.run * (1.0 - PI / 4.0)
        } else {
            self.run * self.run / 2.0
        }
    }

    /// The commands drawing the wedge above the roof, reaching past the block at the front
    /// and over the top.
    fn wedge_commands(&self, h: f64) -> Vec<Value> {
        let front = self.front - OVERRUN;
        if self.curved {
            let lip_pt = [self.front, self.lip];
            let ridge = [self.y_top, h];
            let chain = [
                ridge,
                [self.y_top, h + OVERRUN],
                [front, h + OVERRUN],
                [front, self.lip],
                lip_pt,
            ];
            vec![
                open_polyline(&chain),
                arc(lip_pt, ridge, [self.y_top, self.lip]),
            ]
        } else {
            vec![polyline(&[
                [front, self.lip - OVERRUN],
                [self.y_top, h],
                [self.y_top, h + OVERRUN],
                [front, h + OVERRUN],
            ])]
        }
    }
}

/// A hollow section: a polygon, or lines closed by an arc.
enum Section {
    Polygon(Vec<P2>),
    Curved { lines: Vec<P2>, center: P2 },
}
