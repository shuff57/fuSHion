//! Runs command batches in an in-process SolidCraft session and checks what they built.

use crate::mesh::analyze_stl;
use crate::plan::{BBOX_TOL_MM, Plan, VOLUME_TOL, plan};
use crate::{CustomError, CustomPart, CustomShape, MAX_PART_MM, Result, StudProfile};
use serde_json::{Value, json};
use solidcraft_engine::Session;

/// Most commands a freeform batch may hold.
pub const MAX_COMMANDS: usize = 2000;
/// Largest literal number a freeform command may carry (mm, degrees or a count).
const MAX_LITERAL: f64 = 1000.0;
/// Largest literal pattern count in a freeform command.
const MAX_COUNT: f64 = 64.0;
/// How far the exported mesh's volume may stray from the exact one (chordal error).
const MESH_VOLUME_TOL: f64 = 0.01;

/// Builds `shape` with `profile`: runs [`plan`]'s batch, checks each stage's volume and the
/// finished bounding box against their closed forms, and exports a checked STL and 3MF.
pub fn build(shape: &CustomShape, profile: &StudProfile) -> Result<CustomPart> {
    build_plan(&plan(shape, profile)?)
}

/// Runs a [`Plan`] (see [`build`]).
pub fn build_plan(plan: &Plan) -> Result<CustomPart> {
    let mut s = Session::default();
    let mut index = 0;
    for stage in &plan.stages {
        for c in &stage.commands {
            run_step(&mut s, &stage.name, index, c)?;
            index += 1;
        }
        let m = measure(&mut s, &stage.name)?;
        if m.bodies != 1 {
            return mismatch(&stage.name, format!("{} bodies, expected 1", m.bodies));
        }
        if let Some(why) = volume_off(m.volume, stage.volume_mm3) {
            return mismatch(&stage.name, why);
        }
    }
    let last = plan.stages.last().map_or("(empty)", |s| s.name.as_str());
    finish(&mut s, last, Some(plan.volume_mm3), Some(plan.bbox_mm))
}

/// Builds a freeform part from a SolidCraft batch Claude (or anyone) wrote: `{"commands":
/// [...], "expect"?: {...}}` or a bare array of `{"command", "params"}` steps.
///
/// Before anything runs, the batch is refused when it has more than [`MAX_COMMANDS`]
/// steps, a command outside modelling (`sketch.*`, `solid.*`, `parameters.*`, `body.rename`),
/// one that reads or writes files, a literal number beyond ±1000 or a pattern count above 64.
/// Afterwards the part must be one or more closed bodies no larger than [`MAX_PART_MM`] on a
/// side, its STL watertight, and it must meet the batch's own `expect` (`volume_mm3` with
/// `tol`, `bodies`, `bbox`) when it has one.
pub fn build_from_commands(batch: &Value) -> Result<CustomPart> {
    let (steps, expect) = match batch {
        Value::Array(a) => (a, None),
        Value::Object(o) => match o.get("commands") {
            Some(Value::Array(a)) => (a, o.get("expect").filter(|e| !e.is_null())),
            _ => return refused("a batch is {\"commands\": [...]} or an array of steps"),
        },
        _ => return refused("a batch is {\"commands\": [...]} or an array of steps"),
    };
    if steps.is_empty() {
        return refused("the batch has no steps");
    }
    if steps.len() > MAX_COMMANDS {
        return refused(&format!("{} steps; at most {MAX_COMMANDS}", steps.len()));
    }
    for (i, step) in steps.iter().enumerate() {
        check_step(step).map_err(|why| CustomError::Refused(format!("step {i}: {why}")))?;
    }
    let expect = expect.map(Expect::parse).transpose()?;

    let mut s = Session::default();
    for (i, step) in steps.iter().enumerate() {
        run_step(&mut s, "freeform", i, step)?;
    }
    let m = measure(&mut s, "freeform")?;
    if let Some(n) = expect.as_ref().and_then(|e| e.bodies)
        && m.bodies != n
    {
        return mismatch("freeform", format!("{} bodies, expected {n}", m.bodies));
    }
    let volume = expect.as_ref().and_then(|e| e.volume.map(|v| (v, e.tol)));
    if let Some((v, tol)) = volume
        && (m.volume - v).abs() > tol * v.abs()
    {
        return mismatch(
            "freeform",
            format!(
                "volume {:.3} mm³, expected {v:.3} mm³ (±{:.2} %)",
                m.volume,
                tol * 100.0
            ),
        );
    }
    finish(&mut s, "freeform", None, expect.and_then(|e| e.bbox))
}

/// The batch's own `expect`, as SolidCraft's MCP `batch` takes it.
struct Expect {
    volume: Option<f64>,
    tol: f64,
    bodies: Option<usize>,
    bbox: Option<([f64; 3], [f64; 3])>,
}

impl Expect {
    fn parse(v: &Value) -> Result<Self> {
        let bad = |why: &str| Err(CustomError::Refused(format!("expect: {why}")));
        let Some(o) = v.as_object() else {
            return bad("must be an object");
        };
        for k in o.keys() {
            if !["volume_mm3", "tol", "bodies", "bbox"].contains(&k.as_str()) {
                return bad(&format!("unknown key `{k}`"));
            }
        }
        let num = |k: &str| o.get(k).map(|x| x.as_f64().filter(|f| f.is_finite()));
        let volume = match num("volume_mm3") {
            None => None,
            Some(Some(v)) => Some(v),
            Some(None) => return bad("volume_mm3 must be a number"),
        };
        let tol = match num("tol") {
            None => VOLUME_TOL,
            Some(Some(t)) if t >= 0.0 => t,
            Some(_) => return bad("tol must be a number ≥ 0"),
        };
        let bodies = match o.get("bodies") {
            None => None,
            Some(b) => match b.as_u64().and_then(|n| usize::try_from(n).ok()) {
                Some(n) => Some(n),
                None => return bad("bodies must be a count"),
            },
        };
        let corner = |k: &str| -> Option<[f64; 3]> {
            let a = o.get("bbox")?.get(k)?.as_array()?;
            if a.len() != 3 {
                return None;
            }
            let v: Vec<f64> = a
                .iter()
                .filter_map(Value::as_f64)
                .filter(|f| f.is_finite())
                .collect();
            <[f64; 3]>::try_from(v).ok()
        };
        let bbox = match o.get("bbox") {
            None => None,
            Some(_) => match (corner("min"), corner("max")) {
                (Some(lo), Some(hi)) => Some((lo, hi)),
                _ => return bad("bbox must be {min: [x,y,z], max: [x,y,z]}"),
            },
        };
        Ok(Self {
            volume,
            tol,
            bodies,
            bbox,
        })
    }
}

/// Command families a freeform batch may use.
const ALLOWED_PREFIXES: &[&str] = &["sketch.", "solid.", "parameters."];
const ALLOWED_EXACT: &[&str] = &["body.rename"];
/// Commands in those families that read or write files, or pull in other designs.
const DENIED: &[&str] = &[
    "sketch.text",
    "sketch.edit_text",
    "sketch.insert_dxf",
    "sketch.insert_svg",
    "sketch.drawing_bounds",
    "sketch.export_dxf",
    "sketch.replace",
    "parameters.export",
    "parameters.import",
];
/// Parameter keys that name files.
const FILE_KEYS: &[&str] = &[
    "path",
    "file",
    "font",
    "url",
    "from_file",
    "directory",
    "dir",
];

fn check_step(step: &Value) -> std::result::Result<(), String> {
    let Some(o) = step.as_object() else {
        return Err("a step is {\"command\": id, \"params\": {...}}".into());
    };
    for k in o.keys() {
        if k != "command" && k != "params" {
            return Err(format!("unknown key `{k}`"));
        }
    }
    let Some(id) = o.get("command").and_then(Value::as_str) else {
        return Err("missing `command`".into());
    };
    let allowed = (ALLOWED_PREFIXES.iter().any(|p| id.starts_with(p))
        || ALLOWED_EXACT.contains(&id))
        && !DENIED.contains(&id);
    if !allowed {
        return Err(format!(
            "`{id}` is not a modelling command a custom part may use"
        ));
    }
    if let Some(params) = o.get("params") {
        if !(params.is_object() || params.is_null()) {
            return Err("`params` must be an object".into());
        }
        check_value(params, "", 0)?;
    }
    Ok(())
}

fn check_value(v: &Value, key: &str, depth: usize) -> std::result::Result<(), String> {
    if depth > 16 {
        return Err("parameters nest too deep".into());
    }
    match v {
        Value::Number(n) => {
            let f = n.as_f64().unwrap_or(f64::INFINITY);
            if !f.is_finite() || f.abs() > MAX_LITERAL {
                return Err(format!("`{key}` = {n} is out of range (±{MAX_LITERAL})"));
            }
            if key.starts_with("count") && f > MAX_COUNT {
                return Err(format!("`{key}` = {n}: at most {MAX_COUNT} copies"));
            }
        }
        Value::String(s) => {
            if s.len() > 256 {
                return Err(format!("`{key}` is a string over 256 bytes"));
            }
            if key.starts_with("count") && s.trim().parse::<f64>().map_or(true, |f| f > MAX_COUNT) {
                return Err(format!(
                    "`{key}` must be a literal count of at most {MAX_COUNT}"
                ));
            }
        }
        Value::Array(a) => {
            if a.len() > 4096 {
                return Err(format!("`{key}` lists over 4096 items"));
            }
            for x in a {
                check_value(x, key, depth + 1)?;
            }
        }
        Value::Object(o) => {
            for (k, x) in o {
                if FILE_KEYS.contains(&k.as_str()) {
                    return Err(format!("`{k}`: custom parts may not touch files"));
                }
                check_value(x, k, depth + 1)?;
            }
        }
        Value::Bool(_) | Value::Null => {}
    }
    Ok(())
}

fn refused<T>(why: &str) -> Result<T> {
    Err(CustomError::Refused(why.into()))
}

fn mismatch<T>(stage: &str, details: String) -> Result<T> {
    Err(CustomError::Mismatch {
        stage: stage.into(),
        details,
    })
}

/// `None` when `measured` is within [`VOLUME_TOL`] of `expected`, else what differs.
fn volume_off(measured: f64, expected: f64) -> Option<String> {
    let rel = (measured - expected) / expected.abs().max(1e-9);
    (rel.abs() > VOLUME_TOL).then(|| {
        format!(
            "volume {measured:.3} mm³, expected {expected:.3} mm³ ({:+.3} %)",
            rel * 100.0
        )
    })
}

fn run_step(s: &mut Session, stage: &str, index: usize, step: &Value) -> Result<Value> {
    let id = step
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let params = step
        .get("params")
        .filter(|p| !p.is_null())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let fail = |message: String| CustomError::Engine {
        stage: stage.into(),
        index,
        command: id.into(),
        message,
    };
    let reply = s.execute(id, &params).map_err(|e| fail(e.to_string()))?;
    // A feature that evaluated with an error is reported in its reply rather than as `Err`.
    if let Some(err) = reply.get("error").and_then(Value::as_str) {
        return Err(fail(err.to_string()));
    }
    Ok(reply)
}

/// What `inspect.measure` reports for the whole design.
struct Measured {
    bodies: usize,
    open: usize,
    volume: f64,
    bbox: ([f64; 3], [f64; 3]),
}

fn measure(s: &mut Session, stage: &str) -> Result<Measured> {
    let r = s
        .execute("inspect.measure", &json!({}))
        .map_err(|e| CustomError::Engine {
            stage: stage.into(),
            index: usize::MAX,
            command: "inspect.measure".into(),
            message: e.to_string(),
        })?;
    let bodies = r
        .get("bodies")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let open = bodies
        .iter()
        .filter(|b| b.get("closed").and_then(Value::as_bool) == Some(false))
        .count();
    let volume = r
        .pointer("/total/volume_mm3")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
    for b in &bodies {
        for (corner, out, pick) in [
            ("min", &mut lo, f64::min as fn(f64, f64) -> f64),
            ("max", &mut hi, f64::max),
        ] {
            if let Some(c) = b
                .get("bbox")
                .and_then(|bb| bb.get(corner))
                .and_then(Value::as_array)
            {
                for (k, x) in c.iter().take(3).enumerate() {
                    if let Some(x) = x.as_f64() {
                        out[k] = pick(out[k], x);
                    }
                }
            }
        }
    }
    Ok(Measured {
        bodies: bodies.len(),
        open,
        volume,
        bbox: (lo, hi),
    })
}

fn bbox_off(got: ([f64; 3], [f64; 3]), want: ([f64; 3], [f64; 3]), tol: f64) -> Option<String> {
    let fmt = |b: ([f64; 3], [f64; 3])| {
        format!(
            "[{:.3}, {:.3}, {:.3}]–[{:.3}, {:.3}, {:.3}]",
            b.0[0], b.0[1], b.0[2], b.1[0], b.1[1], b.1[2]
        )
    };
    let off =
        (0..3).any(|k| (got.0[k] - want.0[k]).abs() > tol || (got.1[k] - want.1[k]).abs() > tol);
    off.then(|| format!("bounding box {} mm, expected {} mm", fmt(got), fmt(want)))
}

/// The common end of a build: closed bodies, size limit, expected box, export and mesh checks.
fn finish(
    s: &mut Session,
    stage: &str,
    volume: Option<f64>,
    bbox: Option<([f64; 3], [f64; 3])>,
) -> Result<CustomPart> {
    let m = measure(s, stage)?;
    if m.bodies == 0 {
        return mismatch(stage, "the batch made no body".into());
    }
    if m.open > 0 {
        return mismatch(
            stage,
            format!("{} of {} bodies are not closed solids", m.open, m.bodies),
        );
    }
    if !(m.volume.is_finite() && m.volume > 0.0) {
        return mismatch(stage, format!("volume {} mm³ is not a solid's", m.volume));
    }
    let size = [
        m.bbox.1[0] - m.bbox.0[0],
        m.bbox.1[1] - m.bbox.0[1],
        m.bbox.1[2] - m.bbox.0[2],
    ];
    if size
        .iter()
        .any(|x| !(x.is_finite() && *x <= MAX_PART_MM + BBOX_TOL_MM))
    {
        return Err(CustomError::TooLarge(format!(
            "the part measures {:.1} × {:.1} × {:.1} mm; at most {MAX_PART_MM} mm a side",
            size[0], size[1], size[2]
        )));
    }
    if let Some(why) = volume.and_then(|v| volume_off(m.volume, v)) {
        return mismatch(stage, why);
    }
    if let Some(why) = bbox.and_then(|want| bbox_off(m.bbox, want, BBOX_TOL_MM)) {
        return mismatch(stage, why);
    }

    let state = s
        .world_state()
        .map_err(|e| CustomError::Export(e.to_string()))?;
    let stl = solidcraft_io::export(
        &state,
        &[],
        solidcraft_io::Format::StlBinary,
        "fushion-custom",
    )
    .map_err(|e| CustomError::Export(e.to_string()))?;
    let three_mf = solidcraft_io::export(
        &state,
        &[],
        solidcraft_io::Format::ThreeMf,
        "fushion-custom",
    )
    .map_err(|e| CustomError::Export(e.to_string()))?;

    let mesh = analyze_stl(&stl)?;
    if !mesh.is_watertight() {
        return Err(CustomError::Export(format!(
            "the STL is not watertight: {} open, {} non-manifold and {} misoriented edges",
            mesh.open_edges, mesh.non_manifold_edges, mesh.misoriented_edges
        )));
    }
    let rel = (mesh.volume_mm3 - m.volume) / m.volume;
    if rel.abs() > MESH_VOLUME_TOL {
        return Err(CustomError::Export(format!(
            "the STL encloses {:.3} mm³ but the solid is {:.3} mm³",
            mesh.volume_mm3, m.volume
        )));
    }
    // Units: the mesh must span the solid's box in mm, not in m or inches.
    if let Some(why) = bbox_off(mesh.bbox_mm, m.bbox, BBOX_TOL_MM) {
        return Err(CustomError::Export(format!("STL {why}")));
    }
    Ok(CustomPart {
        stl,
        three_mf,
        volume_mm3: m.volume,
        bbox_mm: m.bbox,
    })
}
