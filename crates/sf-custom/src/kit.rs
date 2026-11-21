//! Detail kits: decorations built from separately printed parts joined with LEGO's own bar
//! and clip (bar Ø3.18 mm), and press-fit pegs, like the flowers of LEGO's Botanical
//! bouquets. See [`crate::kits`] for the templates.
//!
//! # Printing compliant features
//! A clip's jaws, a peg's prongs and a collar's crush ribs must bend **within** print layers
//! (in the bed's XY plane), never across them: a clip whose jaws opened in Z would split
//! between layers. So:
//! * a **clip** is always printed with its C in the bed plane (its bar axis vertical);
//! * a **bar** is printed vertical (a round Ø3.18 rod), or, where it must lie level, as a
//!   **teardrop** (its underside a 45° V, which prints without support and sits in the mouth
//!   of the clip that grips it);
//! * a **peg** is printed vertical, split down its length so its two halves close in XY;
//!   its **hole** is vertical in an upright part.
//!
//! Every part is drawn in its **print frame** (millimetres, z up, standing on the bed at
//! z = 0, laid out as it prints best) and placed in the assembled detail by a rigid
//! transform per copy. A part's display mesh is in LDraw units about its **connection
//! point** (a clip's centre at its foot, a peg's tip, a socket base's corner).

use crate::detail::{
    BASE_HEIGHT, DISPLAY_TRIANGLES, OCCUPY_TOL, SUPPORT_AREA_MM2, arc3, build_on_base, closed, cmd,
    decimate, line, overhang_area, p3, r6, sketch_on, sketch_xy, stl_to_ldu,
};
use crate::run::{finish, measure, run_step};
use crate::shape::P2;
use crate::{CustomError, CustomPart, Result, StudProfile};
use serde::Serialize;
use serde_json::{Value, json};
use sf_core::{DetailDesign, DetailMesh, DetailSource, Kit, KitPart, KitPose, KitStep};
use solidcraft_engine::Session;
use std::collections::BTreeSet;
use std::f64::consts::PI;

/// LEGO's bar diameter, mm.
pub const BAR_D: f64 = 3.18;
/// Largest bending strain allowed in a snap or press fit (PLA yields near 4–5 %).
pub const MAX_STRAIN: f64 = 0.03;
/// Diameter of a split press-fit peg, mm.
pub const PEG_D: f64 = 3.3;
/// Diameter of the hole a peg presses into, mm (0.1 mm interference).
pub const HOLE_D: f64 = 3.2;
/// Width of the slot that splits a peg, mm.
pub const PEG_SLOT: f64 = 0.8;
/// Length of a peg, mm.
pub const PEG_LEN: f64 = 3.5;
/// Smallest wall, slot or gap a 0.4 mm nozzle prints, mm.
pub const MIN_FEATURE: f64 = 0.6;

/// A C-clip that grips a [`BAR_D`] bar, printed with its C in the bed plane. In its own
/// frame the bar axis is z, the clip stands on z = 0, its mouth faces `+x`, and parts join it
/// only through a neck on its back (`−x`), within `neck_half_angle` of the back, so the jaws
/// bend freely.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct ClipSpec {
    /// How much smaller the bore is than the bar, mm (on the diameter).
    pub interference: f64,
    /// Jaw thickness, mm.
    pub wall: f64,
    /// Gap between the jaw tips, mm (less than the bar, so the clip snaps on and holds).
    pub mouth: f64,
    /// Height of the C (its grip along the bar), mm.
    pub height: f64,
    /// Half the angle of the back the neck may cover, degrees.
    pub neck_half_angle: f64,
}

impl Default for ClipSpec {
    /// 0.1 mm interference, 0.75 mm jaws, a 2.55 mm mouth (0.63 mm narrower than the bar, so
    /// it holds), 3 mm tall, a neck over ±20° of the back: 2.8 % strain snapping on, and
    /// under 3 % up to 0.15 mm interference (see [`clip_fit`]).
    fn default() -> Self {
        Self {
            interference: 0.10,
            wall: 0.75,
            mouth: 2.55,
            height: 3.0,
            neck_half_angle: 20.0,
        }
    }
}

impl ClipSpec {
    /// Bore radius, mm.
    pub fn r_in(&self) -> f64 {
        (BAR_D - self.interference) / 2.0
    }
    /// Outside radius, mm.
    pub fn r_out(&self) -> f64 {
        self.r_in() + self.wall
    }
    /// Width of the neck on the back, mm.
    pub fn neck_width(&self) -> f64 {
        2.0 * self.r_out() * self.neck_half_angle.to_radians().sin()
    }
}

/// How a clip fits on a bar: the numbers behind "it snaps on, holds, and doesn't crack".
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct ClipFit {
    /// Bar minus bore, mm (positive: the clip grips).
    pub interference: f64,
    /// Bar minus mouth, mm (positive: the clip holds the bar in).
    pub retention: f64,
    /// How far each jaw tip moves to let the bar through the mouth, mm.
    pub snap_deflection: f64,
    /// Each jaw's free length (along its middle, from the neck to the tip), mm.
    pub jaw_length: f64,
    /// Bending strain at the jaw root while snapping on.
    pub snap_strain: f64,
    /// Bending strain at the jaw root while gripping the bar.
    pub grip_strain: f64,
    /// Thinnest feature (jaw or mouth), mm.
    pub thinnest: f64,
}

impl ClipFit {
    /// Grips, holds, prints, and stays under [`MAX_STRAIN`].
    pub fn ok(&self) -> bool {
        self.interference >= 0.0
            && self.retention > 0.3
            && self.snap_strain <= MAX_STRAIN
            && self.thinnest >= MIN_FEATURE
    }
}

/// The fit of `c` on a [`BAR_D`] bar. Each jaw is a curved cantilever from the edge of the
/// neck to its tip, treated as straight with the length of its middle arc: a tip deflection
/// `δ` bends its root to a strain `ε = 3·δ·t / (2·L²)`. Snapping on, each tip moves
/// `(bar − mouth)/2`; gripping, the bore opens by the interference, which opens the mouth
/// by `π·interference`, half on each jaw.
pub fn clip_fit(c: &ClipSpec) -> ClipFit {
    let (ri, t) = (c.r_in(), c.wall);
    let r_mid = ri + t / 2.0;
    // Half-angle of the mouth (from the +x axis) on the bore.
    let alpha = (c.mouth / 2.0 / ri).clamp(-1.0, 1.0).asin();
    let arc = PI - alpha - c.neck_half_angle.to_radians();
    let len = arc * r_mid;
    let strain = |d: f64| 3.0 * d * t / (2.0 * len * len);
    let snap = (BAR_D - c.mouth) / 2.0;
    let grip = PI * c.interference.max(0.0) / 2.0;
    ClipFit {
        interference: c.interference,
        retention: BAR_D - c.mouth,
        snap_deflection: snap,
        jaw_length: len,
        snap_strain: strain(snap),
        grip_strain: strain(grip),
        thinnest: c.wall.min(c.mouth),
    }
}

/// How a split peg presses into its hole: each half is a cantilever closed by half the
/// interference.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct PegFit {
    /// Peg minus hole, mm.
    pub interference: f64,
    /// Bending strain at each half's root when pressed in.
    pub strain: f64,
    /// Thinnest feature (the slot, or a half's thickness), mm.
    pub thinnest: f64,
}

/// The fit of a [`PEG_D`] split peg in a [`HOLE_D`] hole.
pub fn peg_fit() -> PegFit {
    let i = PEG_D - HOLE_D;
    let t = (PEG_D - PEG_SLOT) / 2.0;
    let len = PEG_LEN - 0.4;
    PegFit {
        interference: i,
        strain: 3.0 * (i / 2.0) * t / (2.0 * len * len),
        thinnest: PEG_SLOT.min(t),
    }
}

// ---------------------------------------------------------------- command builder

/// Commands for one part, in its print frame. The first solid made is the body `Part`;
/// everything after joins or cuts it.
#[derive(Default)]
pub(crate) struct Cmds {
    pub c: Vec<Value>,
    n: usize,
    /// The body `Part` exists (a based part starts with it).
    started: bool,
}

impl Cmds {
    pub fn based() -> Self {
        Self {
            started: true,
            ..Self::default()
        }
    }

    pub fn name(&mut self, prefix: &str) -> String {
        self.n += 1;
        format!("{prefix}{}", self.n)
    }

    /// `"operation"` (and body or targets) for the next solid: new body first, then join.
    fn op(&mut self, p: &mut Value) {
        if self.started {
            p["operation"] = json!("join");
            p["targets"] = json!(["Part"]);
        } else {
            p["operation"] = json!("new");
            p["body_name"] = json!("Part");
            self.started = true;
        }
    }

    fn cut(p: &mut Value) {
        p["operation"] = json!("cut");
        p["targets"] = json!(["Part"]);
    }

    /// A closed outline in a sketch on `plane` extruded `dist` (or, `symmetric`, `dist` in
    /// all centred on the plane).
    pub fn prism(&mut self, plane: Value, outline: Vec<Value>, dist: f64, symmetric: bool, cut: bool) {
        let n = self.name("Sk");
        let mut s = plane;
        s["params"]["name"] = json!(n);
        self.c.push(s);
        self.c.extend(outline);
        self.c.push(cmd("sketch.finish", json!({})));
        let mut p = json!({"sketch": n, "name": format!("{n}Solid"), "distance": r6(dist)});
        if symmetric {
            p["direction"] = json!("symmetric");
        }
        if cut {
            Self::cut(&mut p);
        } else {
            self.op(&mut p);
        }
        self.c.push(cmd("solid.extrude", p));
    }

    pub fn cylinder(&mut self, base: [f64; 3], axis: [f64; 3], len: f64, r: f64, cut: bool) {
        let n = self.name("Cyl");
        let mut p = json!({"radius": r6(r), "height": r6(len), "base": p3(base), "axis": p3(axis), "name": n});
        if cut { Self::cut(&mut p) } else { self.op(&mut p) }
        self.c.push(cmd("solid.cylinder", p));
    }

    /// A cone or frustum (`r1` > 0).
    pub fn cone(&mut self, base: [f64; 3], axis: [f64; 3], len: f64, r0: f64, r1: f64, cut: bool) {
        let n = self.name("Cone");
        let mut p = json!({"radius": r6(r0), "height": r6(len), "base": p3(base), "axis": p3(axis), "name": n});
        if r1 > 0.0 {
            p["top_radius"] = json!(r6(r1));
        }
        if cut { Self::cut(&mut p) } else { self.op(&mut p) }
        self.c.push(cmd("solid.cone", p));
    }

    pub fn boxy(&mut self, lo: [f64; 3], size: [f64; 3], cut: bool) {
        let n = self.name("Box");
        let mut p = json!({"length": r6(size[0]), "width": r6(size[1]), "height": r6(size[2]), "corner": p3(lo), "name": n});
        if cut { Self::cut(&mut p) } else { self.op(&mut p) }
        self.c.push(cmd("solid.box", p));
    }

    /// A profile in the vertical plane through `at` (sketch x radial along `dir`, y up)
    /// revolved round the vertical through `at`, `angle` degrees (360 for all round).
    pub fn revolve(&mut self, at: P2, dir: f64, outline: Vec<Value>, angle: f64) {
        let n = self.name("Rev");
        let (s, c) = dir.sin_cos();
        self.c.push(sketch_on(&n, [at[0], at[1], 0.0], [c, s, 0.0], [0.0, 0.0, 1.0]));
        self.c.extend(outline);
        self.c.push(cmd("sketch.finish", json!({})));
        let mut p = json!({"sketch": n, "axis": {"origin": p3([at[0], at[1], 0.0]), "dir": [0, 0, 1]}, "name": format!("{n}Solid")});
        if angle < 359.99 {
            p["angle"] = json!(format!("{} deg", r6(angle)));
        }
        self.op(&mut p);
        self.c.push(cmd("solid.revolve", p));
    }

    /// A clip (see [`ClipSpec`]) standing on `z0`, centred at `at`, its mouth facing
    /// `mouth` (radians in XY), with a neck `neck` mm long out of its back. Returns the
    /// middle of the neck's far end.
    pub fn clip(&mut self, at: P2, z0: f64, mouth: f64, spec: &ClipSpec, neck: f64) -> [f64; 3] {
        let (ri, ro, m) = (spec.r_in(), spec.r_out(), spec.mouth / 2.0);
        let (s, c) = mouth.sin_cos();
        let rot = |p: P2| -> P2 { [at[0] + p[0] * c - p[1] * s, at[1] + p[0] * s + p[1] * c] };
        let (xi, xo) = ((ri * ri - m * m).sqrt(), (ro * ro - m * m).sqrt());
        let a = rot([xi, m]);
        let b = rot([xo, m]);
        let cc = rot([xo, -m]);
        let d = rot([xi, -m]);
        let outline = vec![
            line(a, b),
            arc3(b, rot([-ro, 0.0]), cc),
            line(cc, d),
            arc3(d, rot([-ri, 0.0]), a),
        ];
        self.prism(sketch_xy("", z0), outline, spec.height, false, false);
        // The neck: from inside the back wall straight out, `neck_width` wide.
        let hw = spec.neck_width() / 2.0;
        let n0 = -(ro - 0.3);
        let n1 = -(ro + neck);
        let corners = [rot([n0, -hw]), rot([n0, hw]), rot([n1, hw]), rot([n1, -hw])];
        self.prism(sketch_xy("", z0), vec![closed(&corners)], spec.height, false, false);
        let far = rot([n1, 0.0]);
        [far[0], far[1], z0 + spec.height / 2.0]
    }

    /// A vertical bar Ø[`BAR_D`] from `base`, `len` long.
    pub fn bar(&mut self, base: [f64; 3], len: f64) {
        self.cylinder(base, [0.0, 0.0, 1.0], len, BAR_D / 2.0, false);
    }

    /// A level teardrop bar along `axis` (unit, level) from `start`, `len` long: round on
    /// top, a 45° V underneath (prints without support; the V sits in the gripping clip's
    /// mouth).
    pub fn teardrop_bar(&mut self, start: [f64; 3], axis: [f64; 2], len: f64) {
        let r = BAR_D / 2.0;
        // A V from 1.6·r below the centre, tangent to the circle: flanks at 51° (a 45° V
        // sits right on the overhang limit).
        let apex = 1.6 * r;
        let (tx, ty) = (r * (1.0 - (r / apex).powi(2)).sqrt(), -r * r / apex);
        // Sketch in the plane square to the bar: x across (axis turned left), y up.
        let across = [-axis[1], axis[0], 0.0];
        let outline = vec![
            arc3([tx, ty], [0.0, r], [-tx, ty]),
            line([-tx, ty], [0.0, -apex]),
            line([0.0, -apex], [tx, ty]),
        ];
        let plane = sketch_on("", start, across, [0.0, 0.0, 1.0]);
        // The plane's normal is across × up = the bar's axis, so extrude forward.
        self.prism(plane, outline, len, false, false);
    }

    /// A split press-fit peg Ø[`PEG_D`] from `base` (its root) along `up` (±z) to its tip.
    pub fn peg(&mut self, base: [f64; 3], up: f64) {
        let tip_z = base[2] + up * PEG_LEN;
        let z0 = base[2].min(tip_z);
        self.cylinder([base[0], base[1], z0], [0.0, 0.0, 1.0], PEG_LEN, PEG_D / 2.0, false);
        // The slot, from the tip back to 0.4 mm short of the root, its inner end a 45° roof
        // (a pointing-down peg prints the slot's end as a ceiling, which must not be flat).
        let depth = PEG_LEN - 0.4;
        // Roof sides at 50° (45° sits right on the overhang limit).
        let h = PEG_SLOT / 2.0;
        let rh = h * 1.2;
        let (z_open, z_end) = if up > 0.0 { (tip_z + 0.5, tip_z - depth) } else { (tip_z - 0.5, tip_z + depth) };
        let profile = if up > 0.0 {
            vec![[-h, z_open], [-h, z_end + rh], [0.0, z_end], [h, z_end + rh], [h, z_open]]
        } else {
            vec![[-h, z_open], [h, z_open], [h, z_end - rh], [0.0, z_end], [-h, z_end - rh]]
        };
        let plane = sketch_on("", [base[0], base[1], 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
        self.prism(plane, vec![closed(&profile)], 2.0 * PEG_D, true, true);
    }

    /// A vertical hole Ø[`HOLE_D`] down from `top`, `depth` deep.
    pub fn hole(&mut self, top: [f64; 3], depth: f64) {
        self.cylinder([top[0], top[1], top[2] - depth], [0.0, 0.0, 1.0], depth + 1.0, HOLE_D / 2.0, true);
    }

    /// A collar that slides down a [`BAR_D`] bar and stays put: a ring with 0.15 mm
    /// clearance and three 0.6 mm crush ribs standing 0.2 mm into the bore (0.05 mm
    /// interference), from `z0` up `h`; the ribs press in XY. `blind` closes its top under a
    /// 50° roof. Make the solid collar with this, join what stands on it, then cut the bore
    /// with [`Cmds::collar_bore`]: the other order (florets joined to a finished hollow
    /// collar) made SolidCraft's mesh export hang. Returns its outside radius.
    pub fn collar(&mut self, at: P2, z0: f64, h: f64, wall: f64) -> f64 {
        let rb = BAR_D / 2.0 + 0.15;
        let ro = rb + wall;
        self.cylinder([at[0], at[1], z0], [0.0, 0.0, 1.0], h, ro, false);
        ro
    }

    /// The collar's bore and ribs (after everything joined to its outside).
    pub fn collar_bore(&mut self, at: P2, z0: f64, h: f64, blind: bool) {
        let rb = BAR_D / 2.0 + 0.15;
        let depth = if blind { h - 1.0 - rb * 1.2 } else { h + 1.0 };
        self.cylinder([at[0], at[1], z0 - 0.5], [0.0, 0.0, 1.0], depth + 0.5, rb, true);
        if blind {
            // A 50° roof over the bore, not a flat ceiling.
            self.cone([at[0], at[1], z0 + depth - 0.01], [0.0, 0.0, 1.0], rb * 1.2, rb, 0.0, true);
        }
        for k in 0..3 {
            let a = 2.0 * PI * k as f64 / 3.0 + 0.3;
            let (s, c) = a.sin_cos();
            // A rib from 0.2 mm inside the bore wall out into it.
            let inner = rb - 0.2;
            let pts: Vec<P2> = [[inner, -0.3], [rb + 0.3, -0.3], [rb + 0.3, 0.3], [inner, 0.3]]
                .iter()
                .map(|p| [at[0] + p[0] * c - p[1] * s, at[1] + p[0] * s + p[1] * c])
                .collect();
            let top = if blind { h - 1.0 } else { h };
            self.prism(sketch_xy("", z0), vec![closed(&pts)], top, false, false);
        }
    }
}

// ---------------------------------------------------------------- parts and kits

/// A rigid map of the print frame into the assembled detail (mm, z up): `R·p + t`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Iso {
    pub r: [[f64; 3]; 3],
    pub t: [f64; 3],
}

impl Iso {
    pub fn translate(t: [f64; 3]) -> Self {
        Self { r: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], t }
    }
    /// Turned `a` about the vertical, then moved by `t`.
    pub fn rz(a: f64, t: [f64; 3]) -> Self {
        let (s, c) = a.sin_cos();
        Self { r: [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]], t }
    }
    pub fn apply(&self, p: [f64; 3]) -> [f64; 3] {
        let r = &self.r;
        [
            r[0][0] * p[0] + r[0][1] * p[1] + r[0][2] * p[2] + self.t[0],
            r[1][0] * p[0] + r[1][1] * p[1] + r[1][2] * p[2] + self.t[1],
            r[2][0] * p[0] + r[2][1] * p[1] + r[2][2] * p[2] + self.t[2],
        ]
    }
    /// This, then `other`.
    #[cfg_attr(not(test), allow(dead_code))]
    #[allow(clippy::needless_range_loop)]
    pub fn then(&self, other: &Iso) -> Iso {
        let mut r = [[0.0; 3]; 3];
        for i in 0..3 {
            for j in 0..3 {
                r[i][j] = (0..3).map(|k| other.r[i][k] * self.r[k][j]).sum();
            }
        }
        Iso { r, t: other.apply(self.t) }
    }
}

/// One kind of part, before it is built.
pub(crate) struct PartDef {
    pub key: String,
    pub label: String,
    pub role: String,
    pub cmds: Vec<Value>,
    /// Built on a socket base on these cells (the part that sits on the model).
    pub base: Option<Vec<(i32, i32)>>,
    /// The connection point in the print frame (the mesh's origin).
    pub connect: [f64; 3],
    /// Print frame → assembled detail, one per copy.
    pub copies: Vec<Iso>,
    /// Why it prints the way it does (kept in the source for the record).
    pub orientation: String,
}

/// A kit, before it is built.
pub(crate) struct KitDef {
    pub parts: Vec<PartDef>,
    pub steps: Vec<(Vec<(usize, usize)>, String)>,
    pub roles: Vec<(&'static str, &'static str)>,
    /// The socket base's cells (in the assembled frame).
    pub sockets: Vec<(i32, i32)>,
}

/// A part's print, from its recorded source (`{"kit_part": {"base": [...]|null,
/// "commands": [...]}}`).
pub fn build_kit_part(source: &Value) -> Result<CustomPart> {
    let kp = source.get("kit_part").ok_or_else(|| CustomError::InvalidShape("not a kit part source".into()))?;
    let cmds: Vec<Value> = kp.get("commands").and_then(Value::as_array).cloned().unwrap_or_default();
    match kp.get("base").filter(|b| !b.is_null()) {
        Some(b) => {
            let base: Vec<(i32, i32)> = serde_json::from_value(b.clone())
                .map_err(|e| CustomError::InvalidShape(format!("kit part base: {e}")))?;
            build_on_base(&base, &cmds, false)
        }
        None => build_plain(&cmds),
    }
}

/// Runs a part's commands and checks it: one closed body, a watertight mesh.
pub(crate) fn build_plain(cmds: &[Value]) -> Result<CustomPart> {
    let mut s = Session::default();
    let trace = std::env::var_os("FUSHION_TRACE").is_some();
    for (i, c) in cmds.iter().enumerate() {
        let t = std::time::Instant::now();
        if trace {
            eprintln!("[{i}] {c}");
        }
        run_step(&mut s, "Kit part", i, c)?;
        if trace {
            eprintln!("    {:.2} s", t.elapsed().as_secs_f64());
        }
    }
    let m = measure(&mut s, "Kit part")?;
    if m.bodies != 1 {
        return Err(CustomError::Mismatch {
            stage: "Kit part".into(),
            details: format!("{} bodies, expected 1", m.bodies),
        });
    }
    if trace {
        eprintln!("exporting a part of {} commands, volume {:.1} mm³", cmds.len(), m.volume);
    }
    let t = std::time::Instant::now();
    let part = finish(&mut s, "Kit part", None, None);
    if trace {
        eprintln!("exported ({} commands) in {:.2} s", cmds.len(), t.elapsed().as_secs_f64());
    }
    part
}

/// mm, z up → LDU, −Y up (as LDraw): x → X, y → Z, z → −Y, ×2.5.
fn to_ldu(v: [f64; 3]) -> [f64; 3] {
    [v[0] * 2.5, -v[2] * 2.5, v[1] * 2.5]
}

/// The rotation part of the same map, applied to a rotation: `Q·R·Qᵀ`.
#[allow(clippy::needless_range_loop)]
fn rot_to_ldu(r: &[[f64; 3]; 3]) -> [[f32; 3]; 3] {
    // Q maps (x, y, z) → (x, −z, y).
    let q = [[1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]];
    let mut qr = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            qr[i][j] = (0..3).map(|k| q[i][k] * r[k][j]).sum::<f64>();
        }
    }
    let mut out = [[0.0f32; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            // (Q R Qᵀ)[i][j] = Σk (QR)[i][k] Q[j][k]
            out[i][j] = (0..3).map(|k| qr[i][k] * q[j][k]).sum::<f64>() as f32;
        }
    }
    out
}

/// A built kit: the design (with its kit) and every part's print, in `kit.parts` order.
#[derive(Clone, Debug)]
pub struct BuiltKit {
    /// The design, its kit filled in.
    pub design: DetailDesign,
    /// Each part's checked print, in `kit.parts` order.
    pub prints: Vec<CustomPart>,
    /// Each part's downward area flatter than 45° off the bed (mm²), in `kit.parts` order.
    pub overhangs: Vec<f64>,
}

/// Builds every part (in parallel), checks them, and assembles the design.
pub(crate) fn build_kit(name: &str, label: &str, def: KitDef, source: Value) -> Result<BuiltKit> {
    let p = StudProfile::fused();
    // Build.
    let results: Vec<Result<CustomPart>> = std::thread::scope(|scope| {
        let handles: Vec<_> = def
            .parts
            .iter()
            .map(|pd| {
                scope.spawn(move || match &pd.base {
                    Some(b) => build_on_base(b, &pd.cmds, false),
                    None => build_plain(&pd.cmds),
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap_or_else(|_| Err(CustomError::Export("a part's build panicked".into())))).collect()
    });
    let mut prints = Vec::new();
    for (pd, r) in def.parts.iter().zip(results) {
        prints.push(r.map_err(|e| prefix(&pd.key, e))?);
    }
    // Where everything is, assembled: every vertex of every copy.
    let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
    for (pd, pr) in def.parts.iter().zip(&prints) {
        let verts = stl_to_ldu(&pr.stl, |v| [v[0] as f32, v[1] as f32, v[2] as f32])?;
        for copy in &pd.copies {
            for v in &verts.positions {
                let w = copy.apply([f64::from(v[0]), f64::from(v[1]), f64::from(v[2])]);
                for i in 0..3 {
                    lo[i] = lo[i].min(w[i]);
                    hi[i] = hi[i].max(w[i]);
                }
            }
        }
    }
    // Occupied cells (as for one-piece details) and the frame of the assembled mesh.
    let col = |v: f64| (v / p.pitch).floor() as i32;
    let (i0, i1) = (col(lo[0] + OCCUPY_TOL), col(hi[0] - OCCUPY_TOL));
    let (j0, j1) = (col(lo[1] + OCCUPY_TOL), col(hi[1] - OCCUPY_TOL));
    let plates = ((hi[2] - 0.05) / p.plate_height).ceil().max(1.0) as i32;
    let cells: Vec<(i32, i32)> = (i0..=i1)
        .flat_map(|i| (j0..=j1).map(move |j| (i, j)))
        .chain(def.sockets.iter().copied())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let si = cells.iter().map(|c| c.0).min().unwrap_or(0);
    let sj = cells.iter().map(|c| c.1).min().unwrap_or(0);
    let to_u8 = |c: &(i32, i32)| -> Result<(u8, u8)> {
        let (a, b) = (c.0 - si, c.1 - sj);
        u8::try_from(a)
            .ok()
            .zip(u8::try_from(b).ok())
            .ok_or_else(|| CustomError::InvalidShape("kit too large".into()))
    };
    let cells_u8: Vec<(u8, u8)> = cells.iter().map(to_u8).collect::<Result<_>>()?;
    let sockets_u8: Vec<(u8, u8)> = def.sockets.iter().map(to_u8).collect::<Result<_>>()?;
    let w = f64::from(cells_u8.iter().map(|c| c.0).max().unwrap_or(0)) + 1.0;
    let d = f64::from(cells_u8.iter().map(|c| c.1).max().unwrap_or(0)) + 1.0;
    let top = f64::from(plates) * p.plate_height;
    // The assembled frame's origin: the top centre of the occupied box.
    let origin = [f64::from(si) * p.pitch + w * 4.0, f64::from(sj) * p.pitch + d * 4.0, top];
    // Meshes: each part about its connection point, cut to its share of the budget.
    // Display meshes: the part on the base up to 6000 triangles, every other part its share
    // of 30 000 (1500 to 5000), so an assembled flower stays under about 35 000.
    let loose: usize = def.parts.iter().filter(|p| p.base.is_none()).map(|p| p.copies.len()).sum();
    let based_budget = 6000.min(DISPLAY_TRIANGLES);
    let loose_budget = (30_000 / loose.max(1)).clamp(1500, 5000);
    let mut kit_parts = Vec::new();
    let mut overhangs = Vec::new();
    let mut assembled = DetailMesh::default();
    for (pd, pr) in def.parts.iter().zip(&prints) {
        let c = pd.connect;
        let mesh = stl_to_ldu(&pr.stl, |v| {
            let l = to_ldu([v[0] - c[0], v[1] - c[1], v[2] - c[2]]);
            [l[0] as f32, l[1] as f32, l[2] as f32]
        })?;
        let mesh = decimate(&mesh, if pd.base.is_some() { based_budget } else { loose_budget });
        // Overhang above the bed (a based part: above its socket's ceiling, whose bridges
        // print like any plate's).
        let above = if pd.base.is_some() { BASE_HEIGHT - p.top_thickness + 0.05 } else { 0.3 };
        let over = overhang_area(&pr.stl, above);
        overhangs.push(over);
        let mut copies = Vec::new();
        for iso in &pd.copies {
            // world_ldu = Q·R·Qᵀ·local + to_ldu(R·c + t − origin)
            let rc = iso.apply(c);
            let pos = to_ldu([rc[0] - origin[0], rc[1] - origin[1], rc[2] - origin[2]]);
            let pose = KitPose {
                rotation: rot_to_ldu(&iso.r),
                position: [pos[0] as f32, pos[1] as f32, pos[2] as f32],
            };
            append_posed(&mut assembled, &mesh, &pose);
            copies.push(pose);
        }
        kit_parts.push(KitPart {
            key: pd.key.clone(),
            label: pd.label.clone(),
            role: pd.role.clone(),
            mesh,
            source: DetailSource {
                source: json!({"kit_part": {"base": pd.base, "commands": pd.cmds, "orientation": pd.orientation}}),
            },
            supports: over > SUPPORT_AREA_MM2,
            copies,
        });
    }
    let kit = Kit {
        parts: kit_parts,
        steps: def
            .steps
            .into_iter()
            .map(|(parts, note)| KitStep { parts, note })
            .collect(),
        roles: def.roles.iter().map(|(r, c)| (r.to_string(), c.to_string())).collect(),
    };
    let design = DetailDesign {
        name: name.into(),
        label: label.into(),
        cells: cells_u8,
        plates: u8::try_from(plates).map_err(|_| CustomError::InvalidShape("kit too tall".into()))?,
        sockets: sockets_u8,
        studs: Vec::new(),
        mesh: assembled,
        source: DetailSource { source },
        kit: Some(kit),
    };
    Ok(BuiltKit { design, prints, overhangs })
}

fn prefix(key: &str, e: CustomError) -> CustomError {
    match e {
        CustomError::InvalidShape(s) => CustomError::InvalidShape(format!("{key}: {s}")),
        CustomError::Mismatch { stage, details } => CustomError::Mismatch { stage: format!("{key}: {stage}"), details },
        CustomError::Engine { stage, index, command, message } => {
            CustomError::Engine { stage: format!("{key}: {stage}"), index, command, message }
        }
        CustomError::Export(s) => CustomError::Export(format!("{key}: {s}")),
        other => other,
    }
}

/// Appends `mesh` moved by `pose` to `out`.
pub fn append_posed(out: &mut DetailMesh, mesh: &DetailMesh, pose: &KitPose) {
    let base = out.positions.len() as u32;
    let r = &pose.rotation;
    for v in &mesh.positions {
        out.positions.push([
            r[0][0] * v[0] + r[0][1] * v[1] + r[0][2] * v[2] + pose.position[0],
            r[1][0] * v[0] + r[1][1] * v[1] + r[1][2] * v[2] + pose.position[1],
            r[2][0] * v[0] + r[2][1] * v[1] + r[2][2] * v[2] + pose.position[2],
        ]);
    }
    out.indices.extend(mesh.indices.iter().map(|i| i + base));
}

/// The kit's parts placed by their poses, as one mesh (what `DetailDesign::mesh` is).
pub fn assemble_kit(kit: &Kit) -> DetailMesh {
    let mut out = DetailMesh::default();
    for p in &kit.parts {
        for pose in &p.copies {
            append_posed(&mut out, &p.mesh, pose);
        }
    }
    out
}

/// A test coupon for clip fit: a strip of five vertical bars, and clips with 0.00, 0.05,
/// 0.10 and 0.15 mm interference on a common spine (each clip has 1–4 dots on the spine to
/// tell them apart). Both print flat on the bed: bars upright, clips' C in the bed plane.
/// Returns `(name, commands)` for the bar strip, the clip strip and each single clip.
pub fn coupon() -> Vec<(String, Vec<Value>)> {
    let mut out = Vec::new();
    // Bar strip: a 2 mm plate 52 × 10 with five bars 8 mm tall, 10 mm apart.
    let mut b = Cmds::default();
    b.boxy([0.0, 0.0, 0.0], [52.0, 10.0, 2.0], false);
    for k in 0..5 {
        b.bar([6.0 + 10.0 * k as f64, 5.0, 1.7], 8.3);
    }
    out.push(("bar_strip".to_string(), b.c));
    // Clip strip: a spine along y = 0 and four clips, mouths facing +y.
    let mut c = Cmds::default();
    c.boxy([0.0, -3.0, 0.0], [48.0, 3.0, 2.0], false);
    for (k, i) in [0.0, 0.05, 0.10, 0.15].iter().enumerate() {
        let spec = ClipSpec { interference: *i, ..ClipSpec::default() };
        let x = 6.0 + 12.0 * k as f64;
        let centre = [x, spec.r_out() + 1.0];
        c.clip(centre, 0.0, PI / 2.0, &spec, 1.0 + 0.3);
        for dot in 0..=k {
            c.cylinder([x - 3.0 + 1.6 * dot as f64, -1.5, 1.9], [0.0, 0.0, 1.0], 0.6, 0.45, false);
        }
    }
    out.push(("clip_strip".to_string(), c.c));
    for i in [0.0, 0.05, 0.10, 0.15] {
        let spec = ClipSpec { interference: i, ..ClipSpec::default() };
        let mut s = Cmds::default();
        s.clip([0.0, 0.0], 0.0, PI / 2.0, &spec, 3.0);
        out.push((format!("clip_i{:03}", (i * 100.0).round() as i32), s.c));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_clip_snaps_on_within_strain() {
        let f = clip_fit(&ClipSpec::default());
        assert!(f.ok(), "{f:?}");
        assert!(f.snap_strain <= MAX_STRAIN && f.snap_strain > 0.02, "{f:?}");
        assert!(f.grip_strain < f.snap_strain);
        // The coupon's tightest clip (0.15 mm) is there to find the limit: just over 3 %.
        let tight = clip_fit(&ClipSpec { interference: 0.15, ..ClipSpec::default() });
        assert!(tight.snap_strain > MAX_STRAIN && tight.snap_strain < 0.035, "{tight:?}");
        // A thick, short-jawed clip would crack.
        let stiff = clip_fit(&ClipSpec { wall: 1.4, neck_half_angle: 60.0, ..ClipSpec::default() });
        assert!(!stiff.ok());
    }

    #[test]
    fn peg_presses_in_within_strain() {
        let f = peg_fit();
        assert!(f.strain < MAX_STRAIN && f.interference > 0.0 && f.thinnest >= MIN_FEATURE, "{f:?}");
    }

    #[test]
    fn poses_compose() {
        let a = Iso::rz(PI / 2.0, [1.0, 0.0, 0.0]);
        let p = a.apply([1.0, 0.0, 0.0]);
        assert!((p[0] - 1.0).abs() < 1e-12 && (p[1] - 1.0).abs() < 1e-12);
        let b = Iso::translate([0.0, 0.0, 2.0]);
        let q = a.then(&b).apply([1.0, 0.0, 0.0]);
        assert!((q[2] - 2.0).abs() < 1e-12);
        // to_ldu and rot_to_ldu agree: rotating then mapping = mapping then rotating.
        let r = rot_to_ldu(&a.r);
        let v = [0.3, -0.7, 1.1];
        let lhs = to_ldu(a.then(&Iso::translate([-1.0, 0.0, 0.0])).apply(v));
        let lv = to_ldu(v);
        let rhs: Vec<f64> = (0..3).map(|i| (0..3).map(|k| f64::from(r[i][k]) * lv[k]).sum()).collect();
        for i in 0..3 {
            assert!((lhs[i] - rhs[i]).abs() < 1e-5, "{lhs:?} {rhs:?}");
        }
    }
}
