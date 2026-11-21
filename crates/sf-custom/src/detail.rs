//! Detail parts: shaped pieces (a flower head, a petal, a leaf, an eye, a beak, a round disc)
//! that stand on a model's studs, designed in SolidCraft from a template or from a batch of
//! commands Claude wrote.
//!
//! # How a detail is built
//! Every detail stands on a **socket base**: the cells in `sockets` (at least two neighbouring
//! cells in a row, since a fused stud can't stand alone), one plate tall, hollow underneath
//! like a plate so fused studs from below push in (walls 1.2 mm, 0.1 mm side clearance, a tube
//! wherever a full 2×2 block of sockets meets). The build always runs in this order, so the
//! base is right whatever the decoration is:
//!
//! 1. **Base**: the sockets' outline, one plate (3.2 mm) tall. Its volume is checked exactly.
//! 2. **Decoration**: the template's (or Claude's) commands, joined onto the base. Material
//!    may reach [`SINK`] mm down into the base's top so it fuses with it.
//! 3. **Trim**: everything below the base's top outside the base's outline is cut away, so
//!    nothing hangs down beside the base into a neighbour's space.
//! 4. **Hollow**: the base's hollow cut out (1.8 mm, as a plate), which also clears anything
//!    the decoration put there.
//! 5. **Tubes** (2×2 sockets): checked exactly.
//!
//! The finished part must be one closed body with a watertight mesh. Its occupied cells are
//! the grid columns its bounding box reaches (by more than [`OCCUPY_TOL`] mm), and its height
//! the plates its top reaches.
//!
//! # Frames
//! Templates are drawn in the **base frame**: the base's cells start at column `(0, 0)`
//! (millimetres, `x` and `y` across, `z` up, the part standing on `z = 0`, the base's top at
//! `z = 3.2`). The returned [`DetailDesign`] moves the cells so the smallest occupied column
//! is `(0, 0)`, and its display mesh is in LDraw units round the top centre of the occupied
//! box (see [`sf_core::DetailMesh`]). The print file stays in the base frame.

use crate::mesh::analyze_stl;
use crate::run::{check_step, finish, measure, run_step};
use crate::shape::{Footprint, P2, signed_area};
use crate::{CustomError, CustomPart, Result, StudProfile};
use serde_json::{Value, json};
use sf_core::{DetailDesign, DetailMesh, DetailSource};
use solidcraft_engine::Session;
use std::collections::{BTreeSet, HashMap};
use std::f64::consts::PI;

/// Height of the socket base: one plate, mm.
pub const BASE_HEIGHT: f64 = 3.2;
/// How far decoration may sink into the base's top to fuse with it, mm.
pub const SINK: f64 = 0.4;
/// How far past a column's edge a part may reach without occupying it, mm (the side
/// clearance every part keeps).
pub const OCCUPY_TOL: f64 = 0.1;
/// How far past its declared cells and plates a Claude-written detail may reach, mm.
pub const OVERHANG_MARGIN: f64 = 0.5;
/// Cutters reach this far past the faces they cut, mm.
const OVERRUN: f64 = 1.0;

/// The templates, each with its parameters (JSON, millimetres unless named otherwise) and
/// their defaults. `base` is `[w, d]` in studs (at least 2 cells, w·d ≤ 16).
pub const TEMPLATES: &[(&str, &str)] = &[
    (
        "flower_head",
        r#"{"petals": 6 (3-12), "diameter": 15.6 (whole flower, 6-40), "petal_width": 6 (1.5-15), "thickness": 1.4 (0.8-4), "cup": 20 (degrees the petals tilt up, 0-60), "centre_diameter": 5 (2-15), "centre_height": 2 (0-8), "base": [2, 2]}"#,
    ),
    (
        "petal",
        r#"{"length": 10 (3-30), "width": 6 (1.5-15), "thickness": 1.4 (0.8-4), "tilt": 25 (degrees up, 0-80), "direction": "+x" (+x|-x|+y|-y), "base": [2, 1]}"#,
    ),
    (
        "leaf",
        r#"{"length": 14 (4-30), "width": 6 (2-15), "thickness": 1.4 (0.8-4), "tilt": 15 (0-80), "direction": "+x", "midrib": true, "base": [2, 1]}"#,
    ),
    (
        "dome",
        r#"{"diameter": 6 (2-15), "height": 3 (0.5-10, at most the diameter / 2 for a round cap), "base": [2, 1]}"#,
    ),
    (
        "cone",
        r#"{"diameter": 5 (1.5-15), "length": 8 (1-30), "top_diameter": 0 (0 = a point), "direction": "up" (up|+x|-x|+y|-y), "tilt": 0 (degrees up from level for a sideways cone, 0-90), "base": [2, 1]}"#,
    ),
    (
        "round_plate",
        r#"{"diameter": 15.6 (4-30), "thickness": 1.6 (0.8-6), "base": [2, 2]}"#,
    ),
];

/// A detail designed and checked: its design (for the model) and its print.
#[derive(Clone, Debug)]
pub struct Detail {
    /// The design, ready to place: cells, sockets, display mesh (at most
    /// [`DISPLAY_TRIANGLES`]) and print source.
    pub design: DetailDesign,
    /// The checked print (STL and 3MF, in the base frame, full resolution).
    pub part: CustomPart,
    /// Whether printing it standing on its base needs supports: see [`overhang_area`].
    pub supports: bool,
    /// Downward-facing area flatter than 45° above the socket's ceiling, mm² (a kit: the
    /// sum over its parts, each above its own bed).
    pub overhang_mm2: f64,
    /// A kit's part prints, in `design.kit` part order (empty for a one-piece detail).
    pub kit_prints: Vec<CustomPart>,
}

/// Most triangles a detail's display mesh keeps (the print keeps them all).
pub const DISPLAY_TRIANGLES: usize = 15_000;
/// Downward-facing area (mm²) a detail may have before it is said to need supports: a few
/// small facets (the bottom of a bud) print without.
pub const SUPPORT_AREA_MM2: f64 = 1.0;

/// The area of a binary STL's faces that face down more steeply than 45° from level (their
/// outward normal within 45° of straight down) and lie wholly above `above` mm: what a
/// printer standing the part on its base would have to print onto air. For a detail,
/// `above` is the socket's ceiling (1.8 mm), whose bridges print like any plate's.
pub fn overhang_area(stl: &[u8], above: f64) -> f64 {
    if stl.len() < 84 {
        return 0.0;
    }
    let n = u32::from_le_bytes([stl[80], stl[81], stl[82], stl[83]]) as usize;
    let f = |o: usize| f64::from(f32::from_le_bytes([stl[o], stl[o + 1], stl[o + 2], stl[o + 3]]));
    let mut area = 0.0;
    for t in 0..n {
        let b = 84 + t * 50 + 12;
        if b + 36 > stl.len() {
            break;
        }
        let v: Vec<[f64; 3]> = (0..3).map(|k| [f(b + k * 12), f(b + k * 12 + 4), f(b + k * 12 + 8)]).collect();
        if v.iter().any(|p| p[2] <= above) {
            continue;
        }
        let e1 = [v[1][0] - v[0][0], v[1][1] - v[0][1], v[1][2] - v[0][2]];
        let e2 = [v[2][0] - v[0][0], v[2][1] - v[0][1], v[2][2] - v[0][2]];
        let c = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ];
        let len = (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt();
        if len > 0.0 && c[2] / len < -std::f64::consts::FRAC_1_SQRT_2 {
            area += len / 2.0;
        }
    }
    area
}

/// Merges vertices on a grid until the mesh has at most `max` triangles (vertex clustering:
/// each grid cell's vertices become their average; collapsed and repeated triangles go).
/// For display only: the result need not be watertight.
pub fn decimate(mesh: &DetailMesh, max: usize) -> DetailMesh {
    if mesh.indices.len() / 3 <= max {
        return mesh.clone();
    }
    let mut cell = 0.25f32;
    loop {
        let mut keys: HashMap<[i32; 3], u32> = HashMap::new();
        let mut sums: Vec<([f32; 3], f32)> = Vec::new();
        let remap: Vec<u32> = mesh
            .positions
            .iter()
            .map(|p| {
                let k = p.map(|c| (c / cell).floor() as i32);
                *keys.entry(k).or_insert_with(|| {
                    sums.push(([0.0; 3], 0.0));
                    (sums.len() - 1) as u32
                })
            })
            .collect();
        for (p, &i) in mesh.positions.iter().zip(&remap) {
            let s = &mut sums[i as usize];
            for (acc, c) in s.0.iter_mut().zip(p) {
                *acc += c;
            }
            s.1 += 1.0;
        }
        let mut seen = std::collections::HashSet::new();
        let mut indices = Vec::new();
        for t in mesh.indices.as_chunks::<3>().0 {
            let tri = [remap[t[0] as usize], remap[t[1] as usize], remap[t[2] as usize]];
            if tri[0] == tri[1] || tri[1] == tri[2] || tri[0] == tri[2] {
                continue;
            }
            let mut key = tri;
            key.sort_unstable();
            if seen.insert(key) {
                indices.extend(tri);
            }
        }
        if indices.len() / 3 <= max || cell > 50.0 {
            let positions = sums.iter().map(|(s, n)| s.map(|c| c / n)).collect();
            return DetailMesh { positions, indices };
        }
        cell *= 1.4;
    }
}

/// Builds a detail from a template (see [`TEMPLATES`]): `params` may leave anything out,
/// which takes its default. `name` is the design's key, `label` its human name.
///
/// The decorative templates in [`crate::KITS`] are built as **kits** of separately printed
/// parts (see [`crate::kits`]) unless `params` has `"one_piece": true`.
pub fn detail_template(name: &str, label: &str, template: &str, params: &Value) -> Result<Detail> {
    let one_piece = params.get("one_piece").and_then(Value::as_bool).unwrap_or(false);
    if !one_piece && crate::kits::KITS.iter().any(|(n, _)| *n == template) {
        return crate::kits::kit_template(name, label, template, params);
    }
    let (base, deco, full) = template_commands(template, params)?;
    let source = json!({"template": template, "params": full});
    assemble(name, label, &base, deco, false, None, source)
}

/// Builds a detail from Claude's own SolidCraft commands (`{"commands": [...]}` or a bare
/// array), drawn in the detail's own frame: column `(i, j)` spans `x ∈ [8i, 8i + 8]`,
/// `y ∈ [8j, 8j + 8]` mm, the part stands on `z = 0`, and the socket base (built first, from
/// `sockets`) has its top at `z = 3.2`. The commands should join onto it (the default
/// operation does). Afterwards everything below the base's top outside the base is cut
/// away and the base's hollow is cut, so the base is always right.
///
/// Refused unless every command passes the freeform checks (modelling commands only, no
/// files, sane numbers), the result is one closed body with a watertight mesh, and it stays
/// within `cells` and `plates` (plus [`OVERHANG_MARGIN`]); `sockets` must be at least two
/// neighbouring cells in a row, all inside `cells`.
pub fn detail_from_commands(
    batch: &Value,
    cells: &[(i32, i32)],
    plates: u32,
    sockets: &[(i32, i32)],
    name: &str,
    label: &str,
) -> Result<Detail> {
    let steps = batch_steps(batch)?;
    let cell_set: BTreeSet<(i32, i32)> = cells.iter().copied().collect();
    if cells.iter().any(|c| c.0 < 0 || c.1 < 0) {
        return bad("detail cells start at (0, 0): no negative columns");
    }
    if let Some(s) = sockets.iter().find(|s| !cell_set.contains(s)) {
        return bad(&format!("socket {s:?} is not one of the detail's cells"));
    }
    if !(1..=16).contains(&plates) {
        return bad("a detail is 1 to 16 plates tall");
    }
    let limit = Limit { cells: cell_set, plates };
    let source = json!({"commands": steps, "cells": cells, "plates": plates, "sockets": sockets});
    assemble(name, label, sockets, steps, true, Some(limit), source)
}

/// Rebuilds a detail's print from its [`DetailSource`] (a template or a batch, as the
/// design recorded it).
pub fn rebuild_detail(source: &Value) -> Result<CustomPart> {
    Ok(rebuild(source, "rebuilt", "rebuilt")?.part)
}

/// The full [`Detail`] again from a recorded source (template or commands).
pub fn rebuild(source: &Value, name: &str, label: &str) -> Result<Detail> {
    if let Some(t) = source.get("template").and_then(Value::as_str) {
        let mut params = source.get("params").cloned().unwrap_or_else(|| json!({}));
        // A one-piece design recorded from a kit template stays one piece.
        if crate::kits::KITS.iter().any(|(n, _)| *n == t) && params.is_object() {
            params["one_piece"] = json!(true);
        }
        return detail_template(name, label, t, &params);
    }
    if let Some(t) = source.get("kit").and_then(Value::as_str) {
        let params = source.get("params").cloned().unwrap_or_else(|| json!({}));
        return crate::kits::kit_template(name, label, t, &params);
    }
    let pairs = |k: &str| -> Result<Vec<(i32, i32)>> {
        serde_json::from_value(source.get(k).cloned().unwrap_or(Value::Null))
            .map_err(|e| CustomError::InvalidShape(format!("detail source `{k}`: {e}")))
    };
    let plates = source.get("plates").and_then(Value::as_u64).unwrap_or(0) as u32;
    detail_from_commands(
        source.get("commands").unwrap_or(&Value::Null),
        &pairs("cells")?,
        plates,
        &pairs("sockets")?,
        name,
        label,
    )
}

fn bad<T>(why: &str) -> Result<T> {
    Err(CustomError::InvalidShape(why.into()))
}

struct Limit {
    cells: BTreeSet<(i32, i32)>,
    plates: u32,
}

fn batch_steps(batch: &Value) -> Result<Vec<Value>> {
    let steps = match batch {
        Value::Array(a) => a.clone(),
        Value::Object(o) => match o.get("commands") {
            Some(Value::Array(a)) => a.clone(),
            _ => return Err(CustomError::Refused("a batch is {\"commands\": [...]} or an array of steps".into())),
        },
        _ => return Err(CustomError::Refused("a batch is {\"commands\": [...]} or an array of steps".into())),
    };
    if steps.is_empty() || steps.len() > crate::MAX_COMMANDS {
        return Err(CustomError::Refused(format!(
            "{} steps; 1 to {}",
            steps.len(),
            crate::MAX_COMMANDS
        )));
    }
    for (i, s) in steps.iter().enumerate() {
        check_step(s).map_err(|why| CustomError::Refused(format!("step {i}: {why}")))?;
    }
    Ok(steps)
}

// ---------------------------------------------------------------- command helpers

pub(crate) fn r6(v: f64) -> f64 {
    (v * 1e6).round() / 1e6
}
pub(crate) fn p3(p: [f64; 3]) -> Value {
    json!([r6(p[0]), r6(p[1]), r6(p[2])])
}
pub(crate) fn cmd(id: &str, params: Value) -> Value {
    json!({"command": id, "params": params})
}
pub(crate) fn sketch_xy(name: &str, z: f64) -> Value {
    cmd(
        "sketch.create",
        json!({"plane": "XY", "offset": format!("{} mm", r6(z)), "name": name}),
    )
}
/// A sketch on the plane through `origin` spanned by `x` and `y` (unit, square).
pub(crate) fn sketch_on(name: &str, origin: [f64; 3], x: [f64; 3], y: [f64; 3]) -> Value {
    cmd(
        "sketch.create",
        json!({"plane": {"origin": p3(origin), "x_dir": p3(x), "y_dir": p3(y)}, "name": name}),
    )
}
pub(crate) fn finish_sketch() -> Value {
    cmd("sketch.finish", json!({}))
}
pub(crate) fn line(a: P2, b: P2) -> Value {
    cmd(
        "sketch.line",
        json!({"points": [[r6(a[0]), r6(a[1])], [r6(b[0]), r6(b[1])]]}),
    )
}
pub(crate) fn closed(points: &[P2]) -> Value {
    let pts: Vec<Value> = points.iter().map(|p| json!([r6(p[0]), r6(p[1])])).collect();
    cmd("sketch.line", json!({"points": pts, "closed": true}))
}
pub(crate) fn arc3(a: P2, through: P2, b: P2) -> Value {
    cmd(
        "sketch.arc.three_point",
        json!({"start": [r6(a[0]), r6(a[1])], "end": [r6(b[0]), r6(b[1])], "through": [r6(through[0]), r6(through[1])]}),
    )
}
pub(crate) fn circle(c: P2, d: f64) -> Value {
    cmd(
        "sketch.circle.center",
        json!({"center": [r6(c[0]), r6(c[1])], "diameter": r6(d)}),
    )
}
pub(crate) fn extrude(sketch: &str, name: &str, distance: f64, op: &str) -> Value {
    cmd(
        "solid.extrude",
        json!({"sketch": sketch, "name": name, "distance": r6(distance), "operation": op}),
    )
}
pub(crate) fn extrude_sym(sketch: &str, name: &str, thickness: f64) -> Value {
    cmd(
        "solid.extrude",
        json!({"sketch": sketch, "name": name, "distance": r6(thickness), "operation": "join", "direction": "symmetric"}),
    )
}

// ---------------------------------------------------------------- templates

/// Reads a number parameter with a default and a range.
pub(crate) fn num(params: &Value, key: &str, default: f64, lo: f64, hi: f64) -> Result<f64> {
    let v = match params.get(key) {
        None | Some(Value::Null) => default,
        Some(x) => x.as_f64().ok_or_else(|| {
            CustomError::InvalidShape(format!("`{key}` must be a number"))
        })?,
    };
    if !(v.is_finite() && (lo..=hi).contains(&v)) {
        return bad(&format!("`{key}` = {v} is outside {lo}–{hi}"));
    }
    Ok(v)
}

pub(crate) fn base_param(params: &Value, default: [u32; 2]) -> Result<Vec<(i32, i32)>> {
    let [w, d] = match params.get("base") {
        None | Some(Value::Null) => default,
        Some(v) => serde_json::from_value::<[u32; 2]>(v.clone())
            .map_err(|_| CustomError::InvalidShape("`base` is [w, d] in studs".into()))?,
    };
    if w * d < 2 || w * d > 16 || w == 0 || d == 0 {
        return bad("`base` must be 2 to 16 cells, e.g. [2, 1] or [2, 2]");
    }
    Ok((0..w as i32)
        .flat_map(|i| (0..d as i32).map(move |j| (i, j)))
        .collect())
}

pub(crate) fn dir_param(params: &Value, default: &str, allow_up: bool) -> Result<(String, [f64; 2])> {
    let d = params
        .get("direction")
        .and_then(Value::as_str)
        .unwrap_or(default)
        .to_string();
    let v = match d.as_str() {
        "+x" => [1.0, 0.0],
        "-x" => [-1.0, 0.0],
        "+y" => [0.0, 1.0],
        "-y" => [0.0, -1.0],
        "up" if allow_up => [0.0, 0.0],
        _ => {
            return bad(&format!(
                "`direction` must be {}+x, -x, +y or -y",
                if allow_up { "up, " } else { "" }
            ));
        }
    };
    Ok((d, v))
}

/// The centre of the base's top, in the base frame.
pub(crate) fn base_centre(base: &[(i32, i32)]) -> P2 {
    let w = base.iter().map(|c| c.0).max().unwrap_or(0) + 1;
    let d = base.iter().map(|c| c.1).max().unwrap_or(0) + 1;
    [f64::from(w) * 4.0, f64::from(d) * 4.0]
}

/// The plane frame of a blade (petal or leaf) pointing at angle `theta` round Z, tilted
/// `tilt` up: `x` along the blade, `y` across it, both unit.
fn blade_frame(theta: f64, tilt: f64) -> ([f64; 3], [f64; 3]) {
    let (st, ct) = theta.sin_cos();
    let (sp, cp) = tilt.sin_cos();
    ([ct * cp, st * cp, sp], [-st, ct, 0.0])
}

/// A petal outline in its plane: from a root `wr` wide at `u0`, widening to `w` at `um`,
/// round over the tip at `u1`.
pub(crate) fn petal_outline(u0: f64, u1: f64, w: f64) -> Vec<Value> {
    let wr = w * 0.4;
    let um = u0 + 0.6 * (u1 - u0);
    let a = [u0, -wr / 2.0];
    let b = [um, -w / 2.0];
    let c = [um, w / 2.0];
    let d = [u0, wr / 2.0];
    vec![line(a, b), arc3(b, [u1, 0.0], c), line(c, d), line(d, a)]
}

/// A pointed leaf outline: two arcs from `u0` to `u1`, `w` wide in the middle.
pub(crate) fn leaf_outline(u0: f64, u1: f64, w: f64) -> Vec<Value> {
    let m = (u0 + u1) / 2.0;
    vec![
        arc3([u0, 0.0], [m, -w / 2.0], [u1, 0.0]),
        arc3([u1, 0.0], [m, w / 2.0], [u0, 0.0]),
    ]
}

/// One blade: a sketch at the base's top through `at`, along `theta` tilted `tilt`, the
/// outline extruded `t` thick, its underside [`SINK`] below the base's top at the root.
fn blade(
    n: usize,
    at: P2,
    theta: f64,
    tilt: f64,
    t: f64,
    outline: Vec<Value>,
    groove: Option<(f64, f64)>,
) -> Vec<Value> {
    let (x, y) = blade_frame(theta, tilt);
    let normal = [-tilt.sin() * theta.cos(), -tilt.sin() * theta.sin(), tilt.cos()];
    // Mid-plane: SINK below the top's thickness centre, so the blade's underside at the root
    // sits SINK into the base and its top surface t above the base top.
    let zc = BASE_HEIGHT + (t - SINK) / 2.0;
    let o = [at[0], at[1], zc];
    let name = format!("Blade{n}");
    let mut c = vec![sketch_on(&name, o, x, y)];
    c.extend(outline);
    c.push(finish_sketch());
    c.push(extrude_sym(&name, &format!("{name}Solid"), t + SINK));
    if let Some((u0, u1)) = groove {
        // A midrib: a 0.6 mm groove 0.4 mm deep down the blade's top face.
        let top = (t + SINK) / 2.0;
        let og = [o[0] + normal[0] * top, o[1] + normal[1] * top, o[2] + normal[2] * top];
        let gname = format!("Rib{n}");
        c.push(sketch_on(&gname, og, x, y));
        c.push(closed(&[[u0, -0.3], [u1, -0.3], [u1, 0.3], [u0, 0.3]]));
        c.push(finish_sketch());
        c.push(cmd(
            "solid.extrude",
            json!({"sketch": gname, "name": format!("{gname}Cut"), "distance": 0.4, "operation": "cut", "direction": "negative"}),
        ));
    }
    c
}

/// A spherical cap of radius `r`, `h` tall, revolved about the vertical through `at`, from
/// `zb` up.
fn cap(name: &str, at: P2, zb: f64, r: f64, h: f64) -> Vec<Value> {
    let c = zb + (h * h - r * r) / (2.0 * h);
    let rho = zb + h - c;
    let a0 = (zb - c).atan2(r);
    let a1 = PI / 2.0;
    let am = (a0 + a1) / 2.0;
    let mid = [rho * am.cos(), c + rho * am.sin()];
    vec![
        sketch_on(name, [at[0], at[1], 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        line([0.0, zb], [r, zb]),
        arc3([r, zb], mid, [0.0, zb + h]),
        line([0.0, zb + h], [0.0, zb]),
        finish_sketch(),
        cmd(
            "solid.revolve",
            json!({"sketch": name, "axis": {"origin": p3([at[0], at[1], 0.0]), "dir": [0, 0, 1]}, "operation": "join", "name": format!("{name}Solid")}),
        ),
    ]
}

/// A template's base cells, decoration commands, and parameters with defaults filled in.
type TemplateParts = (Vec<(i32, i32)>, Vec<Value>, Value);

/// The base cells, the decoration's commands, and the parameters with defaults filled in.
#[allow(clippy::needless_late_init)] // each arm sets both, after reading its own parameters
fn template_commands(template: &str, params: &Value) -> Result<TemplateParts> {
    if !(params.is_object() || params.is_null()) {
        return bad("template params must be an object");
    }
    let mut deco = Vec::new();
    let full;
    let base;
    match template {
        "flower_head" => {
            let petals = num(params, "petals", 6.0, 3.0, 12.0)?.round() as usize;
            let dia = num(params, "diameter", 15.6, 6.0, 40.0)?;
            let pw = num(params, "petal_width", 6.0, 1.5, 15.0)?;
            let t = num(params, "thickness", 1.4, 0.8, 4.0)?;
            let cup = num(params, "cup", 20.0, 0.0, 60.0)?;
            let cd = num(params, "centre_diameter", 5.0, 2.0, 15.0)?;
            let ch = num(params, "centre_height", 2.0, 0.0, 8.0)?;
            base = base_param(params, [2, 2])?;
            let at = base_centre(&base);
            let tilt = cup.to_radians();
            let rc = cd / 2.0;
            // `dia` is the flower's whole width: a tilted petal's underside reaches out past its
            // mid-plane by (t + SINK)/2 · sin(tilt), so the mid-plane stops that much short.
            let reach = (dia / 2.0 - (t + SINK) / 2.0 * tilt.sin()) / tilt.cos().max(0.2);
            // Petals start inside the centre, which is built first, so where they meet near
            // the middle they meet inside it.
            let u0 = rc * 0.6;
            if reach <= rc + 1.0 {
                return bad("the flower is too small for its centre");
            }
            // The centre: a short cylinder with a domed top.
            deco.push(cmd(
                "solid.cylinder",
                json!({"radius": r6(rc), "height": r6(SINK + t + ch), "base": p3([at[0], at[1], BASE_HEIGHT - SINK]), "operation": "join", "name": "Centre"}),
            ));
            let top = BASE_HEIGHT + t + ch;
            deco.extend(cap("CentreCap", at, top - 0.2, rc - 0.15, (rc - 0.15) * 0.6 + 0.2));
            for k in 0..petals {
                // A whisker of difference between petals (well under a print's resolution)
                // keeps the kernel off the exact coincidences a perfectly symmetric flower
                // gives it, where its booleans can fail.
                let theta = 2.0 * PI * k as f64 / petals as f64 + 1e-3 * k as f64;
                let tk = tilt + 2e-3 * (k % 3) as f64;
                deco.extend(blade(k, at, theta, tk, t + 0.003 * (k % 2) as f64, petal_outline(u0, reach, pw), None));
            }
            full = json!({"petals": petals, "diameter": dia, "petal_width": pw, "thickness": t, "cup": cup, "centre_diameter": cd, "centre_height": ch, "base": dims(&base)});
        }
        "petal" | "leaf" => {
            let leaf = template == "leaf";
            let len = num(params, "length", if leaf { 14.0 } else { 10.0 }, if leaf { 4.0 } else { 3.0 }, 30.0)?;
            let w = num(params, "width", 6.0, if leaf { 2.0 } else { 1.5 }, 15.0)?;
            let t = num(params, "thickness", 1.4, 0.8, 4.0)?;
            let tilt = num(params, "tilt", if leaf { 15.0 } else { 25.0 }, 0.0, 80.0)?;
            let (dname, dv) = dir_param(params, "+x", false)?;
            let midrib = params.get("midrib").and_then(Value::as_bool).unwrap_or(true);
            base = base_param(params, [2, 1])?;
            let at = base_centre(&base);
            let theta = dv[1].atan2(dv[0]);
            let u0 = 0.0;
            let u1 = len;
            let outline = if leaf {
                leaf_outline(u0, u1, w)
            } else {
                petal_outline(u0, u1, w)
            };
            let groove = (leaf && midrib && len > 6.0).then_some((u0 + 2.5, u1 - 2.5));
            deco.extend(blade(0, at, theta, tilt.to_radians(), t, outline, groove));
            full = if leaf {
                json!({"length": len, "width": w, "thickness": t, "tilt": tilt, "direction": dname, "midrib": midrib, "base": dims(&base)})
            } else {
                json!({"length": len, "width": w, "thickness": t, "tilt": tilt, "direction": dname, "base": dims(&base)})
            };
        }
        "dome" => {
            let d = num(params, "diameter", 6.0, 2.0, 15.0)?;
            let h = num(params, "height", 3.0, 0.5, 10.0)?;
            if h > d / 2.0 + 1e-9 {
                return bad("a dome is at most half its diameter tall");
            }
            base = base_param(params, [2, 1])?;
            let at = base_centre(&base);
            // From SINK below the top, so the cap's rim fuses with the base.
            deco.extend(cap("Dome", at, BASE_HEIGHT - SINK, d / 2.0, h + SINK));
            full = json!({"diameter": d, "height": h, "base": dims(&base)});
        }
        "cone" => {
            let d = num(params, "diameter", 5.0, 1.5, 15.0)?;
            let len = num(params, "length", 8.0, 1.0, 30.0)?;
            let top = num(params, "top_diameter", 0.0, 0.0, 15.0)?;
            let (dname, dv) = dir_param(params, "up", true)?;
            let tilt = num(params, "tilt", 0.0, 0.0, 90.0)?.to_radians();
            base = base_param(params, [2, 1])?;
            let at = base_centre(&base);
            let r = d / 2.0;
            let (axis, start) = if dname == "up" {
                ([0.0, 0.0, 1.0], [at[0], at[1], BASE_HEIGHT - SINK])
            } else {
                let (s, c) = tilt.sin_cos();
                // Its base circle stands on the base's top (sunk SINK), centred over the base.
                ([dv[0] * c, dv[1] * c, s], [at[0], at[1], BASE_HEIGHT - SINK + r * c])
            };
            let length = if dname == "up" { len + SINK } else { len };
            let mut p = json!({"radius": r6(r), "height": r6(length), "base": p3(start), "axis": p3(axis), "operation": "join", "name": "Cone"});
            if top > 0.0 {
                p["top_radius"] = json!(r6(top / 2.0));
            }
            deco.push(cmd("solid.cone", p));
            full = json!({"diameter": d, "length": len, "top_diameter": top, "direction": dname, "tilt": tilt.to_degrees(), "base": dims(&base)});
        }
        "round_plate" => {
            let d = num(params, "diameter", 15.6, 4.0, 30.0)?;
            let t = num(params, "thickness", 1.6, 0.8, 6.0)?;
            base = base_param(params, [2, 2])?;
            let at = base_centre(&base);
            deco.push(cmd(
                "solid.cylinder",
                json!({"radius": r6(d / 2.0), "height": r6(t + SINK), "base": p3([at[0], at[1], BASE_HEIGHT - SINK]), "operation": "join", "name": "Disc"}),
            ));
            full = json!({"diameter": d, "thickness": t, "base": dims(&base)});
        }
        other if crate::flower::SPECIES.iter().any(|(n, _)| *n == other) => {
            let (b, d, f) = crate::flower::species(other, params)?;
            base = b;
            deco = d;
            full = f;
        }
        other => {
            let names: Vec<&str> = TEMPLATES
                .iter()
                .chain(crate::flower::SPECIES.iter())
                .map(|t| t.0)
                .collect();
            return bad(&format!("no detail template `{other}` (have {})", names.join(", ")));
        }
    }
    Ok((base, deco, full))
}

pub(crate) fn dims(base: &[(i32, i32)]) -> [i32; 2] {
    [
        base.iter().map(|c| c.0).max().unwrap_or(0) + 1,
        base.iter().map(|c| c.1).max().unwrap_or(0) + 1,
    ]
}

// ---------------------------------------------------------------- assembly

/// Runs base → decoration → trim → hollow → tubes on `sockets` and checks the result: one
/// closed body, watertight mesh. The base frame: cells from the sockets' coordinates, z up.
pub(crate) fn build_on_base(sockets: &[(i32, i32)], deco: &[Value], freeform: bool) -> Result<CustomPart> {
    let p = StudProfile::fused();
    let f = Footprint::new(sockets)?;
    // At least two neighbouring sockets in a row.
    if !sockets
        .iter()
        .any(|&(i, j)| f.cells.contains(&(i + 1, j)) || f.cells.contains(&(i, j + 1)))
    {
        return bad("a detail's socket base needs at least two neighbouring cells in a row");
    }
    let outer = f.offset_loops(p.pitch, p.clearance);
    let inner = f.offset_loops(p.pitch, p.clearance + p.wall);
    let hollow_h = BASE_HEIGHT - p.top_thickness;
    let mut s = Session::default();
    let mut index = 0;
    // FUSHION_TRACE=1 prints each command and how long it took (to find a slow boolean).
    let trace = std::env::var_os("FUSHION_TRACE").is_some();
    let mut run = |s: &mut Session, stage: &str, c: &Value| -> Result<()> {
        let t = std::time::Instant::now();
        if trace {
            eprintln!("[{index}] {stage}: {c}");
        }
        run_step(s, stage, index, c)?;
        if trace {
            eprintln!("    {:.2} s", t.elapsed().as_secs_f64());
        }
        index += 1;
        Ok(())
    };
    // 1. Base.
    let mut base = vec![sketch_xy("BaseSketch", 0.0)];
    base.extend(outer.iter().map(|l| closed(l)));
    base.push(finish_sketch());
    base.push(cmd(
        "solid.extrude",
        json!({"sketch": "BaseSketch", "name": "Base", "distance": BASE_HEIGHT, "operation": "new", "body_name": "Part"}),
    ));
    for c in &base {
        run(&mut s, "Base", c)?;
    }
    let want: f64 = outer.iter().map(|l| signed_area(l)).sum::<f64>() * BASE_HEIGHT;
    let m = measure(&mut s, "Base")?;
    if (m.volume - want).abs() > 1e-3 * want {
        return Err(CustomError::Mismatch {
            stage: "Base".into(),
            details: format!("volume {:.3} mm³, expected {want:.3}", m.volume),
        });
    }
    // 2. Decoration.
    let stage = if freeform { "freeform" } else { "Decoration" };
    for c in deco {
        run(&mut s, stage, c)?;
    }
    // 3. Trim: below the base's top, outside the base. A box well round everything, with
    // the base's outline as its hole.
    let m = measure(&mut s, stage)?;
    let (lo, hi) = m.bbox;
    let pad = 2.0;
    let ring = [
        [lo[0].min(0.0) - pad, lo[1].min(0.0) - pad],
        [hi[0].max(0.0) + pad, lo[1].min(0.0) - pad],
        [hi[0].max(0.0) + pad, hi[1].max(0.0) + pad],
        [lo[0].min(0.0) - pad, hi[1].max(0.0) + pad],
    ];
    let reaches_out = lo[0] < outer_min(&outer, 0) - 1e-6
        || lo[1] < outer_min(&outer, 1) - 1e-6
        || hi[0] > outer_max(&outer, 0) + 1e-6
        || hi[1] > outer_max(&outer, 1) + 1e-6;
    if reaches_out {
        let mut trim = vec![sketch_xy("TrimSketch", lo[2].min(0.0) - OVERRUN)];
        trim.push(closed(&ring));
        trim.extend(outer.iter().map(|l| closed(l)));
        trim.push(finish_sketch());
        trim.push(extrude(
            "TrimSketch",
            "Trim",
            BASE_HEIGHT - lo[2].min(0.0) + OVERRUN,
            "cut",
        ));
        for c in &trim {
            // A trim that finds nothing to cut is fine.
            if let Err(e) = run(&mut s, "Trim", c)
                && !e.to_string().to_lowercase().contains("nothing")
                && !e.to_string().to_lowercase().contains("does not intersect")
                && !e.to_string().to_lowercase().contains("no material")
            {
                return Err(e);
            }
        }
    }
    // 4. Hollow.
    let mut hollow = vec![sketch_xy("HollowSketch", -OVERRUN)];
    hollow.extend(inner.iter().map(|l| closed(l)));
    hollow.push(finish_sketch());
    hollow.push(extrude("HollowSketch", "Hollow", hollow_h + OVERRUN, "cut"));
    for c in &hollow {
        run(&mut s, "Hollow", c)?;
    }
    // 5. Tubes.
    let tubes = f.full_corners();
    if !tubes.is_empty() {
        let before = measure(&mut s, "Hollow")?.volume;
        let (ro, ri) = (p.tube_outer_diameter / 2.0, p.tube_inner_diameter / 2.0);
        let mut c = vec![sketch_xy("TubeSketch", 0.0)];
        for &(i, j) in &tubes {
            let at = [f64::from(i) * p.pitch, f64::from(j) * p.pitch];
            c.push(circle(at, p.tube_outer_diameter));
            c.push(circle(at, p.tube_inner_diameter));
        }
        c.push(finish_sketch());
        c.push(extrude("TubeSketch", "Tubes", hollow_h, "join"));
        for x in &c {
            run(&mut s, "Tubes", x)?;
        }
        let t_m = std::time::Instant::now();
        let got = measure(&mut s, "Tubes")?.volume - before;
        if trace {
            eprintln!("measure after tubes: {:.2} s", t_m.elapsed().as_secs_f64());
        }
        let want = PI * (ro * ro - ri * ri) * hollow_h * tubes.len() as f64;
        if (got - want).abs() > 2e-3 * want + 0.01 {
            return Err(CustomError::Mismatch {
                stage: "Tubes".into(),
                details: format!("tubes added {got:.3} mm³, expected {want:.3} (something filled the hollow)"),
            });
        }
    }
    let t_m = std::time::Instant::now();
    let m = measure(&mut s, "Detail")?;
    if trace {
        eprintln!("measure: {:.2} s", t_m.elapsed().as_secs_f64());
    }
    if m.bodies != 1 {
        return Err(CustomError::Mismatch {
            stage: "Detail".into(),
            details: format!(
                "{} bodies: the decoration must touch the base (overlap its top by up to {SINK} mm)",
                m.bodies
            ),
        });
    }
    let t_export = std::time::Instant::now();
    let part = finish(&mut s, "Detail", None, None)?;
    if trace {
        eprintln!("export and mesh checks: {:.2} s", t_export.elapsed().as_secs_f64());
    }
    Ok(part)
}

/// Builds a detail on its socket base (see [`build_on_base`]) and works out the design.
fn assemble(
    name: &str,
    label: &str,
    sockets: &[(i32, i32)],
    deco: Vec<Value>,
    freeform: bool,
    limit: Option<Limit>,
    source: Value,
) -> Result<Detail> {
    let p = StudProfile::fused();
    let hollow_h = BASE_HEIGHT - p.top_thickness;
    let trace = std::env::var_os("FUSHION_TRACE").is_some();
    let part = build_on_base(sockets, &deco, freeform)?;
    let (lo, hi) = part.bbox_mm;
    // Occupied columns and plates.
    let col = |v: f64| (v / p.pitch).floor() as i32;
    let (i0, i1) = (col(lo[0] + OCCUPY_TOL), col(hi[0] - OCCUPY_TOL));
    let (j0, j1) = (col(lo[1] + OCCUPY_TOL), col(hi[1] - OCCUPY_TOL));
    let plates = ((hi[2] - 0.05) / p.plate_height).ceil().max(1.0) as i32;
    if let Some(l) = &limit {
        let li = l.cells.iter().map(|c| c.0).min().unwrap_or(0);
        let lj = l.cells.iter().map(|c| c.1).min().unwrap_or(0);
        let ui = l.cells.iter().map(|c| c.0).max().unwrap_or(0) + 1;
        let uj = l.cells.iter().map(|c| c.1).max().unwrap_or(0) + 1;
        let m = OVERHANG_MARGIN;
        if lo[0] < f64::from(li) * 8.0 - m
            || lo[1] < f64::from(lj) * 8.0 - m
            || hi[0] > f64::from(ui) * 8.0 + m
            || hi[1] > f64::from(uj) * 8.0 + m
            || hi[2] > f64::from(l.plates) * p.plate_height + m
        {
            return Err(CustomError::Refused(format!(
                "the detail measures [{:.1}, {:.1}, {:.1}]–[{:.1}, {:.1}, {:.1}] mm, outside its cells \
                 [{}, {}]–[{}, {}] studs × {} plates (± {m} mm)",
                lo[0], lo[1], lo[2], hi[0], hi[1], hi[2], li, lj, ui, uj, l.plates
            )));
        }
    }
    let cells: Vec<(i32, i32)> = match &limit {
        Some(l) => l.cells.iter().copied().collect(),
        None => (i0..=i1)
            .flat_map(|i| (j0..=j1).map(move |j| (i, j)))
            .chain(sockets.iter().copied())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
    };
    let plates = match &limit {
        Some(l) => l.plates as i32,
        None => plates,
    };
    // Move so the smallest column is (0, 0).
    let si = cells.iter().map(|c| c.0).min().unwrap_or(0);
    let sj = cells.iter().map(|c| c.1).min().unwrap_or(0);
    let to_u8 = |c: &(i32, i32)| -> Result<(u8, u8)> {
        let (a, b) = (c.0 - si, c.1 - sj);
        if !(0..=255).contains(&a) || !(0..=255).contains(&b) {
            return bad("detail too large");
        }
        Ok((a as u8, b as u8))
    };
    let cells_u8: Vec<(u8, u8)> = cells.iter().map(to_u8).collect::<Result<_>>()?;
    let sockets_u8: Vec<(u8, u8)> = sockets.iter().map(to_u8).collect::<Result<_>>()?;
    let w = cells_u8.iter().map(|c| c.0).max().unwrap_or(0) as f64 + 1.0;
    let d = cells_u8.iter().map(|c| c.1).max().unwrap_or(0) as f64 + 1.0;
    let top = f64::from(plates) * p.plate_height;
    // The mesh: base frame (mm) → local frame (cells from 0) → LDU about the top centre.
    let (ox, oy) = (f64::from(si) * p.pitch + w * 4.0, f64::from(sj) * p.pitch + d * 4.0);
    let mesh = stl_to_ldu(&part.stl, |v| {
        [
            ((v[0] - ox) * 2.5) as f32,
            ((top - v[2]) * 2.5) as f32,
            ((v[1] - oy) * 2.5) as f32,
        ]
    })?;
    let t_dec = std::time::Instant::now();
    let mesh = decimate(&mesh, DISPLAY_TRIANGLES);
    if trace {
        eprintln!("display mesh: {:.2} s, {} triangles", t_dec.elapsed().as_secs_f64(), mesh.indices.len() / 3);
    }
    let design = DetailDesign {
        name: name.into(),
        label: label.into(),
        cells: cells_u8,
        plates: u8::try_from(plates).map_err(|_| CustomError::InvalidShape("detail too tall".into()))?,
        sockets: sockets_u8,
        studs: Vec::new(),
        mesh,
        kit: None,
        mount: None,
        source: DetailSource { source },
    };
    let overhang_mm2 = overhang_area(&part.stl, hollow_h + 0.05);
    Ok(Detail {
        design,
        part,
        supports: overhang_mm2 > SUPPORT_AREA_MM2,
        overhang_mm2,
        kit_prints: Vec::new(),
    })
}

fn outer_min(loops: &[Vec<P2>], k: usize) -> f64 {
    loops.iter().flatten().map(|q| q[k]).fold(f64::MAX, f64::min)
}
fn outer_max(loops: &[Vec<P2>], k: usize) -> f64 {
    loops.iter().flatten().map(|q| q[k]).fold(f64::MIN, f64::max)
}

/// Reads a binary STL into an indexed mesh, each vertex mapped by `map` (shared vertices
/// matched by exact coordinates).
pub(crate) fn stl_to_ldu(stl: &[u8], map: impl Fn([f64; 3]) -> [f32; 3]) -> Result<DetailMesh> {
    analyze_stl(stl)?; // length and finiteness
    let n = u32::from_le_bytes([stl[80], stl[81], stl[82], stl[83]]) as usize;
    let f = |o: usize| f32::from_le_bytes([stl[o], stl[o + 1], stl[o + 2], stl[o + 3]]);
    let mut ids: HashMap<[u32; 3], u32> = HashMap::new();
    let mut mesh = DetailMesh::default();
    for t in 0..n {
        let base = 84 + t * 50 + 12;
        let mut tri = [0u32; 3];
        for (k, slot) in tri.iter_mut().enumerate() {
            let o = base + k * 12;
            let v = [f(o), f(o + 4), f(o + 8)];
            let next = mesh.positions.len() as u32;
            *slot = *ids.entry(v.map(f32::to_bits)).or_insert_with(|| {
                mesh.positions.push(map(v.map(f64::from)));
                next
            });
        }
        if tri[0] != tri[1] && tri[1] != tri[2] && tri[0] != tri[2] {
            mesh.indices.extend(tri);
        }
    }
    Ok(mesh)
}
