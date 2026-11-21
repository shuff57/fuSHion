//! Flower species after LEGO's Botanical bouquets (10280 Flower Bouquet, 10313 Wildflower
//! Bouquet), as detail templates on the standard socket base (see [`crate::detail`]).
//!
//! How the heads are made, as the sets build them: a **hub** (on a thin **stem** when
//! `stem_height` > 0, through a 45° **calyx** cone so its underside prints without supports),
//! **petals** in one or more rings round it, a later ring turned half a petal step, and a
//! **centre** on top.
//!
//! * **Cupped petals**: each ring is a band revolved round the flower's axis (so the petals
//!   curve round the axis) whose cross-section is a circular arc rising from `b0` to `b1`
//!   degrees above level (so they curve up along their length), cut to the ring's petal
//!   outline seen from above (round-tipped petals, `w` wide, from a narrow root). The band
//!   starts inside the hub. Petals rising at 45° or more everywhere print without supports.
//! * **Rose**: two rings of four big petals, the inner one steeper, then a spiral of short
//!   curved walls (each a revolved strip, turned 137.5° on from the last and a little smaller)
//!   round a bud.
//! * **Spike** flowers: rings of florets up a vertical stem, every other ring turned half a
//!   step, the florets shrinking toward the tip.
//! * **Umbel**: stalks from one point, each ending in a tiny cluster.
//! * **Fern**: a frond standing in a vertical plane: a rib curling over toward `direction`,
//!   with pairs of leaflets shrinking toward the tip.
//! * **Stem**: a plain rod, upright or leaning, optionally with a knob or bud on top.
//!
//! Every geometry is drawn in the base frame (see [`crate::detail`]); joins name the part's
//! body (`Part`) so the working bodies of a petal ring never swallow anything else.

use crate::detail::{
    BASE_HEIGHT, SINK, arc3, base_centre, base_param, closed, cmd, dims, dir_param, line, num, p3,
    r6, sketch_on, sketch_xy,
};
use crate::shape::P2;
use crate::{CustomError, Result};
use serde_json::{Value, json};
use std::f64::consts::PI;

/// The species templates and their parameters (millimetres and degrees; `base` `[w, d]` in
/// studs, 2–16 cells; every head also takes `stem_height`, 0–80 mm, a bare stem under it, and
/// `"mount"` (default true): with no `stem_height`, a bar hole up its hub for a printed stem's
/// tip bar, the hub lifted on a column as far as the hole needs). Each head suggests the stem
/// its kit template does (see `crate::KITS`).
pub const SPECIES: &[(&str, &str)] = &[
    (
        "rose",
        r#"{"size": 32 (head width, 16-60), "petals": 4 (per ring, 3-6), "layers": 2 (1-2), "open": 0.4 (0 closed bud .. 1 wide open; above ~0.7 the rim needs supports), "inner": 5 (spiral petals, 0-8), "stem_height": 0, "base": [2, 2]}"#,
    ),
    (
        "daisy",
        r#"{"size": 16 (8-40), "petals": 6 (4-12), "cup": 8 (degrees the petals rise, 0-60), "petal_width": 0.8 (0.3-1 of the room), "centre": 0.35 (centre disc / size, 0.15-0.6), "stem_height": 0, "base": [2, 2]}"#,
    ),
    (
        "aster",
        r#"{"size": 24 (10-50), "rings": 3 (1-4), "petals": 16 (per ring, 8-30), "cup": 10 (outer ring, 0-50; each inner ring 20 more), "petal_width": 0.6 (0.3-1), "stem_height": 0, "base": [2, 2]}"#,
    ),
    (
        "gerbera",
        r#"{"size": 28 (12-56), "petals": 18 (per ring, 10-30), "cup": 12 (0-50), "petal_width": 0.75 (0.3-1), "centre": 0.32 (0.15-0.6), "stem_height": 0, "base": [2, 2]}"#,
    ),
    (
        "poppy",
        r#"{"size": 30 (14-56), "petals": 4 (3-6), "open": 0.3 (0-1), "centre": 0.3 (0.15-0.5), "stem_height": 0, "base": [2, 2]}"#,
    ),
    (
        "cornflower",
        r#"{"size": 22 (10-44), "florets": 6 (4-10), "angle": 60 (degrees the trumpets rise, 30-85; under 55 they need supports), "stem_height": 0, "base": [2, 2]}"#,
    ),
    (
        "spike",
        r#"{"kind": "lupin" (snapdragon|lupin|lavender|larkspur), "height": 40 (flowering part, 10-56), "rings": 8 (2-20), "florets_per_ring": 5 (3-8), "taper": 0.5 (tip floret size lost, 0-0.9), "floret_size": 4.5 (1.5-7), "stem_height": 6 (bare stem below, 0-40), "base": [2, 1]}"#,
    ),
    (
        "umbel",
        r#"{"rays": 12 (4-24), "spread": 35 (outer rays' lean from upright, 10-60), "ray_length": 8 (3-20), "cluster": 1.4 (floret cluster radius, 0.6-3), "stem_height": 20 (0-50), "base": [2, 1]}"#,
    ),
    (
        "fern",
        r#"{"length": 40 (10-56), "pairs": 8 (3-16), "curl": 60 (degrees the tip turns over, 0-150), "leaflet_length": 10 (3-25), "leaflet_width": 3 (1-8), "angle": 40 (leaflets from the rib, 20-80), "thickness": 1.4 (0.8-3), "direction": "+x", "base": [2, 1]}"#,
    ),
    (
        "stem",
        r#"{"height": 6 (plates above the base, 1-18), "diameter": 3.2 (1.6-6), "angle": 0 (lean from upright, 0-60), "direction": "+x", "top": "none" (none|knob|bud), "base": [2, 1]}"#,
    ),
];

/// A species' base cells, decoration commands, filled-in parameters, post-tube commands and
/// stem mount.
pub(crate) type Species = (
    Vec<(i32, i32)>,
    Vec<Value>,
    Value,
    Vec<Value>,
    Option<crate::stem::MountMm>,
);

/// The species that are flower heads: on a stem (with `"mount": true`, the default, and no
/// `stem_height` of their own) the stem's tip bar goes up a bar hole into their hub.
pub(crate) const HEADS: &[&str] = &["rose", "daisy", "aster", "gerbera", "poppy", "cornflower"];

/// Builds the commands for species `name`.
pub(crate) fn species(name: &str, params: &Value) -> Result<Species> {
    let mut b = Build::default();
    let head = HEADS.contains(&name);
    let with_mount = head && crate::stem::wants_mount(params) && stem_height(params, 0.0)? == 0.0;
    if with_mount {
        // The base is read again by the species; the mount needs it first.
        let base = base_param(params, [2, 2])?;
        b.lift = Some(crate::stem::head_mount(base_centre(&base), &base, false).top);
    }
    let mut full = match name {
        "rose" => rose(&mut b, params)?,
        "daisy" => daisy(&mut b, params)?,
        "aster" => aster(&mut b, params)?,
        "gerbera" => gerbera(&mut b, params)?,
        "poppy" => poppy(&mut b, params)?,
        "cornflower" => cornflower(&mut b, params)?,
        "spike" => spike(&mut b, params)?,
        "umbel" => umbel(&mut b, params)?,
        "fern" => fern(&mut b, params)?,
        "stem" => stem_template(&mut b, params)?,
        other => return Err(CustomError::InvalidShape(format!("no species `{other}`"))),
    };
    let (mut post, mut mount) = (Vec::new(), None);
    if head {
        full["mount"] = json!(with_mount);
    }
    if let (true, Some(at)) = (with_mount, b.mount_at) {
        let m = crate::stem::head_mount(at, &b.base, false);
        b.c.extend(m.cmds);
        post = m.post;
        mount = Some(m.mount);
    }
    Ok((b.base, b.c, full, post, mount))
}

#[derive(Default)]
struct Build {
    base: Vec<(i32, i32)>,
    c: Vec<Value>,
    /// Counter for unique feature names.
    n: usize,
    /// Floret primitives added (for tests).
    florets: usize,
    /// For a stem mount: how high the hub's solid must reach on its axis (it is lifted on a
    /// column if it doesn't), and where the hub went.
    lift: Option<f64>,
    mount_at: Option<P2>,
}

impl Build {
    fn name(&mut self, prefix: &str) -> String {
        self.n += 1;
        format!("{prefix}{}", self.n)
    }

    /// A cylinder from `from` along unit `axis`, `len` long.
    fn rod(&mut self, from: [f64; 3], axis: [f64; 3], len: f64, r: f64) {
        let n = self.name("Rod");
        self.c.push(cmd(
            "solid.cylinder",
            json!({"radius": r6(r), "height": r6(len), "base": p3(from), "axis": p3(axis), "operation": "join", "targets": ["Part"], "name": n}),
        ));
    }

    /// A cone (or frustum, `r1` > 0) from `from` along unit `axis`.
    fn cone(&mut self, from: [f64; 3], axis: [f64; 3], len: f64, r0: f64, r1: f64) {
        let n = self.name("Cone");
        let mut p = json!({"radius": r6(r0), "height": r6(len), "base": p3(from), "axis": p3(axis), "operation": "join", "targets": ["Part"], "name": n});
        if r1 > 0.0 {
            p["top_radius"] = json!(r6(r1));
        }
        self.c.push(cmd("solid.cone", p));
    }

    fn sphere(&mut self, at: [f64; 3], r: f64) {
        let n = self.name("Ball");
        self.c.push(cmd(
            "solid.sphere",
            json!({"radius": r6(r), "center": p3(at), "operation": "join", "targets": ["Part"], "name": n}),
        ));
    }

    /// A spherical cap `h` tall, `r` wide at its rim, rim at height `zb`, on the vertical
    /// through `at` (revolved, so it is one smooth face).
    fn cap(&mut self, at: P2, zb: f64, r: f64, h: f64) {
        let n = self.name("Cap");
        let c = zb + (h * h - r * r) / (2.0 * h);
        let rho = zb + h - c;
        let a0 = (zb - c).atan2(r);
        let am = (a0 + PI / 2.0) / 2.0;
        let mid = [rho * am.cos(), c + rho * am.sin()];
        self.c.extend([
            sketch_on(&n, [at[0], at[1], 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
            line([0.0, zb], [r, zb]),
            arc3([r, zb], mid, [0.0, zb + h]),
            line([0.0, zb + h], [0.0, zb]),
            cmd("sketch.finish", json!({})),
            cmd(
                "solid.revolve",
                json!({"sketch": n, "axis": {"origin": p3([at[0], at[1], 0.0]), "dir": [0, 0, 1]}, "operation": "join", "targets": ["Part"], "name": format!("{n}Solid")}),
            ),
        ]);
    }

    /// A curved wall: the strip `r_in..r_out` × `z0..z1` (radius × height) revolved
    /// `span` degrees round the vertical through `at`, starting at angle `start`.
    #[allow(clippy::too_many_arguments)]
    fn wall(&mut self, at: P2, start: f64, span: f64, r_in: f64, r_out: f64, z0: f64, z1: f64) {
        let n = self.name("Wall");
        let (s, c) = start.sin_cos();
        self.c.extend([
            sketch_on(&n, [at[0], at[1], 0.0], [c, s, 0.0], [0.0, 0.0, 1.0]),
            closed(&[[r_in, z0], [r_out, z0], [r_out, z1], [r_in, z1]]),
            cmd("sketch.finish", json!({})),
            cmd(
                "solid.revolve",
                json!({"sketch": n, "axis": {"origin": p3([at[0], at[1], 0.0]), "dir": [0, 0, 1]}, "angle": format!("{} deg", r6(span)), "operation": "join", "targets": ["Part"], "name": format!("{n}Solid")}),
            ),
        ]);
    }

    /// A stem from the base's top up to `z_top`, `r` thick, on the vertical through `at`.
    fn stem(&mut self, at: P2, z_top: f64, r: f64) {
        let z0 = BASE_HEIGHT - SINK;
        if z_top > z0 + 0.1 {
            self.rod([at[0], at[1], z0], [0.0, 0.0, 1.0], z_top - z0, r);
        }
    }

    /// The hub a head's petals start from: on a stem (stem radius 1.6 mm, then a 45° calyx
    /// widening to `r_hub`) or straight on the base. Returns the hub's top.
    fn hub(&mut self, at: P2, stem_height: f64, r_hub: f64, hub_h: f64) -> f64 {
        const STEM_R: f64 = 1.6;
        let z = if stem_height > 0.0 {
            let zs = BASE_HEIGHT + stem_height;
            self.stem(at, zs + 0.2, STEM_R);
            // Calyx: from the stem out to a little past the hub (so their edges don't
            // coincide), a little steeper than 45°.
            let r_top = r_hub + 0.3;
            let h = ((r_top - STEM_R) * 1.15).max(0.2);
            self.cone([at[0], at[1], zs], [0.0, 0.0, 1.0], h + 0.01, STEM_R, r_top);
            zs + h
        } else if let Some(need) = self.lift {
            // On a stem: a column round the bar hole up to where it needs, the hub on top.
            let z0 = BASE_HEIGHT - SINK;
            self.rod(
                [at[0], at[1], z0],
                [0.0, 0.0, 1.0],
                need - z0,
                r_hub.max(crate::stem::MOUNT_COLUMN_R),
            );
            self.mount_at = Some(at);
            (need - hub_h).max(z0)
        } else {
            BASE_HEIGHT - SINK
        };
        let top = z + hub_h;
        self.rod(
            [at[0], at[1], z - 0.01],
            [0.0, 0.0, 1.0],
            top - z + 0.01,
            r_hub,
        );
        top
    }

    /// One ring of cupped petals (see the module docs).
    fn ring(&mut self, at: P2, r: &Ring) -> Result<()> {
        let step = 2.0 * PI / r.n as f64;
        // The petals' roots stand on a circle inside the hub; the band starts further in,
        // inside the polygon joining the roots, so what is left of it after the petals are
        // cut out is one piece: the petals and a buried ring joining them.
        let u0 = r.hub - 0.4;
        let r0 = (u0 * (step / 2.0).cos() - 0.8).max(0.6);
        let r = &Ring { r0, ..*r };
        // The band's cross-section: a circular arc of length `len` from (r0, z0) heading
        // b0 above level, turning to b1, `t` thick on its upper side.
        let (b0, b1) = (r.b0.to_radians(), r.b1.to_radians());
        let dr = r.reach - r.r0;
        let len = if (b1 - b0).abs() < 1e-3 {
            dr / b0.cos()
        } else {
            dr * (b1 - b0) / (b1.sin() - b0.sin())
        };
        let pt = |s: f64, off: f64| -> P2 {
            // The point `s` along the arc, moved `off` along its upper normal.
            let (x, z, beta) = if (b1 - b0).abs() < 1e-3 {
                (r.r0 + s * b0.cos(), r.z0 + s * b0.sin(), b0)
            } else {
                let k = (b1 - b0) / len;
                let beta = b0 + k * s;
                (
                    r.r0 + (beta.sin() - b0.sin()) / k,
                    r.z0 - (beta.cos() - b0.cos()) / k,
                    beta,
                )
            };
            [x - off * beta.sin(), z + off * beta.cos()]
        };
        let (p0, pm, p1) = (pt(0.0, 0.0), pt(len / 2.0, 0.0), pt(len, 0.0));
        let (q0, qm, q1) = (pt(0.0, r.t), pt(len / 2.0, r.t), pt(len, r.t));
        let n = self.name("Ring");
        let mut c = vec![sketch_on(
            &n,
            [at[0], at[1], 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
        )];
        if (b1 - b0).abs() < 1e-3 {
            c.push(closed(&[p0, p1, q1, q0]));
        } else {
            c.extend([
                arc3(p0, pm, p1),
                line(p1, q1),
                arc3(q1, qm, q0),
                line(q0, p0),
            ]);
        }
        c.push(cmd("sketch.finish", json!({})));
        let body = format!("{n}Band");
        c.push(cmd(
            "solid.revolve",
            json!({"sketch": n, "axis": {"origin": p3([at[0], at[1], 0.0]), "dir": [0, 0, 1]}, "operation": "new", "body_name": body, "name": format!("{n}Solid")}),
        ));
        // The petals' outline from above.
        let rim = p1[0].min(q1[0]);
        let u1 = rim - 0.25;
        // Widest a petal can be without touching its neighbours, with its round tip
        // (radius w/2) ending at u1: w ≤ 2·(u1 − w/2)·sin(step/2)·0.92.
        let k = (step / 2.0).sin() * 0.92;
        let w = r.w.min(2.0 * u1 * k / (1.0 + k));
        let wr = (w * 0.45)
            .min(2.0 * u0 * (step / 2.0).sin() * 0.85)
            .max(0.3);
        let um = (u1 - w / 2.0).max(u0 + 0.3);
        if w < 0.6 || u1 <= u0 + 0.6 {
            return Err(CustomError::InvalidShape(format!(
                "a ring of {} petals out to {:.1} mm leaves no room for them",
                r.n, r.reach
            )));
        }
        let ol = format!("{n}Outline");
        c.push(sketch_xy(&ol, r.z0.min(p1[1]).min(q0[1]) - 2.0));
        for k in 0..r.n {
            // A whisker of difference between petals keeps the kernel off exact symmetries.
            let phi = r.rot + step * k as f64 + 1e-4 * k as f64;
            let (s, co) = phi.sin_cos();
            let m = |u: f64, v: f64| -> P2 { [at[0] + u * co - v * s, at[1] + u * s + v * co] };
            let a = m(u0, -wr / 2.0);
            let bb = m(um, -w / 2.0);
            let tip = m(u1, 0.0);
            let cc = m(um, w / 2.0);
            let d = m(u0, wr / 2.0);
            c.push(line(a, bb));
            c.push(arc3(bb, tip, cc));
            c.push(line(cc, d));
            let phi2 = phi + step + 1e-4;
            let (s2, c2) = phi2.sin_cos();
            let next = [
                at[0] + u0 * c2 + wr / 2.0 * s2,
                at[1] + u0 * s2 - wr / 2.0 * c2,
            ];
            if k + 1 == r.n {
                // Back to the first petal's root.
                let (s0, c0) = r.rot.sin_cos();
                let first = [
                    at[0] + u0 * c0 + wr / 2.0 * s0,
                    at[1] + u0 * s0 - wr / 2.0 * c0,
                ];
                c.push(line(d, first));
            } else {
                c.push(line(d, next));
            }
        }
        c.push(cmd("sketch.finish", json!({})));
        let top = p1[1].max(q1[1]).max(pm[1]).max(qm[1]) + 4.0;
        let bottom = r.z0.min(p1[1]).min(q0[1]) - 2.0;
        c.push(cmd(
            "solid.extrude",
            json!({"sketch": ol, "name": format!("{n}Cut"), "distance": r6(top - bottom), "operation": "intersect", "targets": [body]}),
        ));
        c.push(cmd(
            "solid.combine",
            json!({"target": "Part", "tools": [body], "operation": "join"}),
        ));
        self.c.extend(c);
        Ok(())
    }
}

/// One ring of petals.
#[derive(Clone, Copy)]
struct Ring {
    n: usize,
    /// Angle of the first petal, radians.
    rot: f64,
    /// The hub's radius: the petals' roots and the band's start are inside it.
    hub: f64,
    /// Where the band starts: radius (worked out from `hub`) and height, mm. The height must
    /// be inside the hub.
    r0: f64,
    z0: f64,
    /// Its slope at the start and at the rim, degrees above level.
    b0: f64,
    b1: f64,
    /// Radius the rim reaches, mm.
    reach: f64,
    t: f64,
    /// Widest petal, mm (less where the ring has no room).
    w: f64,
}

fn stem_height(params: &Value, default: f64) -> Result<f64> {
    num(params, "stem_height", default, 0.0, 80.0)
}

fn rose(b: &mut Build, p: &Value) -> Result<Value> {
    let size = num(p, "size", 32.0, 16.0, 60.0)?;
    let petals = num(p, "petals", 4.0, 3.0, 6.0)?.round() as usize;
    let layers = num(p, "layers", 2.0, 1.0, 2.0)?.round() as usize;
    let open = num(p, "open", 0.4, 0.0, 1.0)?;
    let inner = num(p, "inner", 5.0, 0.0, 8.0)?.round() as usize;
    let sh = stem_height(p, 0.0)?;
    b.base = base_param(p, [2, 2])?;
    let at = base_centre(&b.base);
    let r = size / 2.0;
    let r_hub = 0.3 * r;
    let top = b.hub(at, sh, r_hub, 2.5);
    let t = 1.2;
    let step = 2.0 * PI / petals as f64;
    b.ring(
        at,
        &Ring {
            n: petals,
            rot: 0.0,
            hub: r_hub,
            r0: 0.0,
            z0: top - 2.0,
            b0: 50.0,
            b1: 80.0 - 40.0 * open,
            reach: r,
            t,
            w: 2.0 * r,
        },
    )?;
    if layers == 2 {
        b.ring(
            at,
            &Ring {
                n: petals,
                rot: step / 2.0,
                hub: r_hub,
                r0: 0.0,
                z0: top - 1.6,
                b0: 62.0,
                b1: 84.0 - 24.0 * open,
                reach: 0.72 * r,
                t,
                w: 2.0 * r,
            },
        )?;
    }
    // Inner petals: a spiral of curved walls round the bud, each a little smaller and
    // turned on by the golden angle.
    // They stand on the hub inside where the inner ring's band comes up through it.
    let r_wall = r_hub - 1.5;
    let mut r_min = r_wall;
    for k in 0..inner {
        let f = k as f64 / inner.max(1) as f64;
        let ro = r_wall * (1.0 - 0.55 * f);
        let h = r * (0.55 - 0.15 * f);
        b.wall(
            at,
            (137.5 * k as f64).to_radians(),
            220.0,
            ro - 0.9,
            ro,
            top - 0.3,
            top + h,
        );
        r_min = ro - 0.9;
    }
    // The bud: a short column with a round top (a cap taller than it is wide would bulge
    // out over its own neck).
    let rb = (r_min - 0.15).max(0.8);
    let hb = r * 0.3;
    if hb > rb {
        b.rod(
            [at[0], at[1], top - 0.3],
            [0.0, 0.0, 1.0],
            hb - rb + 0.3,
            rb,
        );
    }
    // Not taller than its rim is wide, so it never bulges past the column.
    b.cap(
        at,
        top + (hb - rb).max(0.0) - 0.05,
        rb - 0.05,
        (rb.min(hb) - 0.1).max(0.3),
    );
    Ok(
        json!({"size": size, "petals": petals, "layers": layers, "open": open, "inner": inner, "stem_height": sh, "base": dims(&b.base)}),
    )
}

fn daisy(b: &mut Build, p: &Value) -> Result<Value> {
    let size = num(p, "size", 16.0, 8.0, 40.0)?;
    let petals = num(p, "petals", 6.0, 4.0, 12.0)?.round() as usize;
    let cup = num(p, "cup", 8.0, 0.0, 60.0)?;
    let pw = num(p, "petal_width", 0.8, 0.3, 1.0)?;
    let centre = num(p, "centre", 0.35, 0.15, 0.6)?;
    let sh = stem_height(p, 0.0)?;
    b.base = base_param(p, [2, 2])?;
    let at = base_centre(&b.base);
    let r = size / 2.0;
    let r_hub = (centre * r).max(1.8);
    let top = b.hub(at, sh, r_hub, 2.8);
    let step = 2.0 * PI / petals as f64;
    let um = 0.65 * r;
    b.ring(
        at,
        &Ring {
            n: petals,
            rot: 0.0,
            hub: r_hub,
            r0: 0.0,
            z0: top - 1.4,
            b0: cup,
            b1: cup + 4.0,
            reach: r,
            t: 1.2,
            w: pw * 2.0 * um * (step / 2.0).sin() * 0.95,
        },
    )?;
    // The centre: a round tile, a little raised.
    b.rod([at[0], at[1], top - 0.5], [0.0, 0.0, 1.0], 1.6, centre * r);
    Ok(
        json!({"size": size, "petals": petals, "cup": cup, "petal_width": pw, "centre": centre, "stem_height": sh, "base": dims(&b.base)}),
    )
}

fn aster(b: &mut Build, p: &Value) -> Result<Value> {
    let size = num(p, "size", 24.0, 10.0, 50.0)?;
    let rings = num(p, "rings", 3.0, 1.0, 4.0)?.round() as usize;
    let petals = num(p, "petals", 16.0, 8.0, 30.0)?.round() as usize;
    let cup = num(p, "cup", 10.0, 0.0, 50.0)?;
    let pw = num(p, "petal_width", 0.6, 0.3, 1.0)?;
    let sh = stem_height(p, 0.0)?;
    b.base = base_param(p, [2, 2])?;
    let at = base_centre(&b.base);
    let r = size / 2.0;
    let r_hub = 0.22 * r + 1.6;
    let top = b.hub(at, sh, r_hub, 1.6 + 0.8 * rings as f64);
    let step = 2.0 * PI / petals as f64;
    for i in 0..rings {
        let f = i as f64;
        let reach = r * (1.0 - 0.22 * f);
        let um = 0.7 * reach;
        b.ring(
            at,
            &Ring {
                n: petals,
                rot: f * step / 2.0,
                hub: r_hub,
                r0: 0.0,
                z0: top - 1.4 - 0.8 * (rings as f64 - 1.0 - f),
                b0: (cup + 20.0 * f).min(75.0),
                b1: (cup + 20.0 * f + 6.0).min(80.0),
                reach,
                t: 1.0,
                w: pw * 2.0 * um * (step / 2.0).sin(),
            },
        )?;
    }
    // Raised centre.
    b.cap(at, top - 0.2, r_hub - 0.2, 0.45 * r_hub + 0.6);
    Ok(
        json!({"size": size, "rings": rings, "petals": petals, "cup": cup, "petal_width": pw, "stem_height": sh, "base": dims(&b.base)}),
    )
}

fn gerbera(b: &mut Build, p: &Value) -> Result<Value> {
    let size = num(p, "size", 28.0, 12.0, 56.0)?;
    let petals = num(p, "petals", 18.0, 10.0, 30.0)?.round() as usize;
    let cup = num(p, "cup", 12.0, 0.0, 50.0)?;
    let pw = num(p, "petal_width", 0.75, 0.3, 1.0)?;
    let centre = num(p, "centre", 0.32, 0.15, 0.6)?;
    let sh = stem_height(p, 0.0)?;
    b.base = base_param(p, [2, 2])?;
    let at = base_centre(&b.base);
    let r = size / 2.0;
    let r_hub = (centre * r).max(2.4);
    let top = b.hub(at, sh, r_hub, 3.2);
    let step = 2.0 * PI / petals as f64;
    for (i, reach) in [(0usize, r), (1, 0.82 * r)] {
        let um = 0.7 * reach;
        b.ring(
            at,
            &Ring {
                n: petals,
                rot: i as f64 * step / 2.0,
                hub: r_hub,
                r0: 0.0,
                z0: top - 1.4 - 0.8 * (1 - i) as f64,
                b0: cup + 14.0 * i as f64,
                b1: cup + 4.0 + 14.0 * i as f64,
                reach,
                t: 1.0,
                w: pw * 2.0 * um * (step / 2.0).sin(),
            },
        )?;
    }
    // A dark centre disc with a low dome.
    b.rod([at[0], at[1], top - 0.5], [0.0, 0.0, 1.0], 1.4, r_hub);
    b.cap(at, top + 0.7, r_hub * 0.7, r_hub * 0.35);
    Ok(
        json!({"size": size, "petals": petals, "cup": cup, "petal_width": pw, "centre": centre, "stem_height": sh, "base": dims(&b.base)}),
    )
}

fn poppy(b: &mut Build, p: &Value) -> Result<Value> {
    let size = num(p, "size", 30.0, 14.0, 56.0)?;
    let petals = num(p, "petals", 4.0, 3.0, 6.0)?.round() as usize;
    let open = num(p, "open", 0.3, 0.0, 1.0)?;
    let centre = num(p, "centre", 0.3, 0.15, 0.5)?;
    let sh = stem_height(p, 0.0)?;
    b.base = base_param(p, [2, 2])?;
    let at = base_centre(&b.base);
    let r = size / 2.0;
    let r_hub = (centre * r).max(2.4);
    let top = b.hub(at, sh, r_hub, 2.0);
    b.ring(
        at,
        &Ring {
            n: petals,
            rot: PI / 4.0,
            hub: r_hub,
            r0: 0.0,
            z0: top - 1.6,
            b0: 48.0,
            b1: 72.0 - 30.0 * open,
            reach: r,
            t: 1.2,
            w: 2.0 * r,
        },
    )?;
    // A flat centre with spokes: a disc, then an eight-point star on it.
    b.rod([at[0], at[1], top - 0.5], [0.0, 0.0, 1.0], 1.0, r_hub);
    let n = b.name("Spokes");
    let mut star = Vec::new();
    for k in 0..16 {
        let a = PI * k as f64 / 8.0 + 0.01;
        let rr = if k % 2 == 0 {
            r_hub * 0.95
        } else {
            r_hub * 0.35
        };
        star.push([at[0] + rr * a.cos(), at[1] + rr * a.sin()]);
    }
    b.c.extend([
        sketch_xy(&n, top + 0.4),
        closed(&star),
        cmd("sketch.finish", json!({})),
        cmd(
            "solid.extrude",
            json!({"sketch": n, "name": format!("{n}Solid"), "distance": 0.8, "operation": "join", "targets": ["Part"]}),
        ),
    ]);
    b.cap(at, top + 1.1, r_hub * 0.3, r_hub * 0.25);
    Ok(
        json!({"size": size, "petals": petals, "open": open, "centre": centre, "stem_height": sh, "base": dims(&b.base)}),
    )
}

fn cornflower(b: &mut Build, p: &Value) -> Result<Value> {
    let size = num(p, "size", 22.0, 10.0, 44.0)?;
    let florets = num(p, "florets", 6.0, 4.0, 10.0)?.round() as usize;
    let angle = num(p, "angle", 60.0, 30.0, 85.0)?;
    let sh = stem_height(p, 0.0)?;
    b.base = base_param(p, [2, 2])?;
    let at = base_centre(&b.base);
    let r = size / 2.0;
    let r_hub = 0.2 * r + 1.6;
    let top = b.hub(at, sh, r_hub, 2.0);
    // Trumpets: frustums flaring from a narrow root on the hub out and up.
    let e = angle.to_radians();
    let len = (r - r_hub * 0.5) / e.cos().max(0.25);
    let len = len.min(0.9 * size);
    let mouth = 0.14 * size;
    for k in 0..florets {
        let phi = 2.0 * PI * k as f64 / florets as f64 + 1e-4 * k as f64;
        let axis = [e.cos() * phi.cos(), e.cos() * phi.sin(), e.sin()];
        let from = [
            at[0] + 0.4 * r_hub * phi.cos(),
            at[1] + 0.4 * r_hub * phi.sin(),
            top - 1.3,
        ];
        let l = len + 0.01 * k as f64;
        b.cone(from, axis, l, 0.9, mouth);
        // Hollow the mouth into a trumpet: a cone cut in from just past the rim.
        let out = 0.5;
        let start = [
            from[0] + axis[0] * (l + out),
            from[1] + axis[1] * (l + out),
            from[2] + axis[2] * (l + out),
        ];
        let n = b.name("Mouth");
        b.c.push(cmd(
            "solid.cone",
            json!({"radius": r6(mouth * 0.8), "top_radius": 0.25, "height": r6(out + 0.45 * l), "base": p3(start), "axis": p3([-axis[0], -axis[1], -axis[2]]), "operation": "cut", "targets": ["Part"], "name": n}),
        ));
        b.florets += 1;
    }
    // A spiky centre: an eight-point star crown on the hub. (Thin leaning cones here made
    // SolidCraft's mesh export hang where they cut the trumpets.)
    let n = b.name("Crown");
    let mut star = Vec::new();
    for k in 0..16 {
        let a = PI * k as f64 / 8.0 + 0.05;
        let rr = if k % 2 == 0 {
            r_hub * 0.85
        } else {
            r_hub * 0.4
        };
        star.push([at[0] + rr * a.cos(), at[1] + rr * a.sin()]);
    }
    b.c.extend([
        sketch_xy(&n, top - 0.4),
        closed(&star),
        cmd("sketch.finish", json!({})),
        cmd(
            "solid.extrude",
            json!({"sketch": n, "name": format!("{n}Solid"), "distance": r6(0.12 * size + 0.4), "operation": "join", "targets": ["Part"]}),
        ),
    ]);
    Ok(
        json!({"size": size, "florets": florets, "angle": angle, "stem_height": sh, "base": dims(&b.base)}),
    )
}

fn spike(b: &mut Build, p: &Value) -> Result<Value> {
    let kind = p
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("lupin")
        .to_string();
    if !["snapdragon", "lupin", "lavender", "larkspur"].contains(&kind.as_str()) {
        return Err(CustomError::InvalidShape(
            "`kind` is snapdragon, lupin, lavender or larkspur".into(),
        ));
    }
    let height = num(p, "height", 40.0, 10.0, 56.0)?;
    let rings = num(p, "rings", 8.0, 2.0, 20.0)?.round() as usize;
    let per = num(p, "florets_per_ring", 5.0, 3.0, 8.0)?.round() as usize;
    let taper = num(p, "taper", 0.5, 0.0, 0.9)?;
    let fs = num(p, "floret_size", 4.5, 1.5, 7.0)?;
    let sh = stem_height(p, 6.0)?;
    if BASE_HEIGHT + sh + height + fs > crate::MAX_PART_MM - 1.0 {
        return Err(CustomError::InvalidShape(format!(
            "stem_height + height is {:.0} mm; a part is at most {} mm tall",
            sh + height,
            crate::MAX_PART_MM
        )));
    }
    b.base = base_param(p, [2, 1])?;
    let at = base_centre(&b.base);
    // A LEGO bar's 3.2 mm: thick enough to hide the florets' roots.
    let rs = 1.6;
    let z_first = BASE_HEIGHT + sh;
    let z_top = z_first + height;
    b.stem(at, z_top, rs);
    let dz = height / rings as f64;
    for k in 0..rings {
        let f = if rings > 1 {
            k as f64 / (rings - 1) as f64
        } else {
            0.0
        };
        let s = fs * (1.0 - taper * f);
        let z = z_first + dz * (k as f64 + 0.3);
        let off = if k % 2 == 1 { PI / per as f64 } else { 0.0 };
        for j in 0..per {
            let phi = off + 2.0 * PI * j as f64 / per as f64 + 1e-4 * (k * per + j) as f64;
            let (sp, cp) = phi.sin_cos();
            let out = |rad: f64, up: f64| {
                [
                    at[0] + rad * cp,
                    at[1] + rad * sp,
                    z + up + 0.002 * j as f64,
                ]
            };
            match kind.as_str() {
                // Pods: a cone rising steeply outward from the stem, pointed.
                "lupin" => {
                    let e = 68f64.to_radians();
                    b.cone(
                        out(0.0, 0.0),
                        [e.cos() * cp, e.cos() * sp, e.sin()],
                        s * 1.3 + rs * 0.5,
                        (s * 0.42).min(rs - 0.05),
                        0.0,
                    );
                }
                // Bells: a frustum flaring out and up.
                "snapdragon" => {
                    let e = 68f64.to_radians();
                    b.cone(
                        out(0.0, 0.0),
                        [e.cos() * cp, e.cos() * sp, e.sin()],
                        s * 1.2 + rs,
                        s * 0.18,
                        s * 0.45,
                    );
                }
                // Buds: little balls tight on the stem.
                "lavender" => b.sphere(out(rs + s * 0.15, 0.0), s * 0.35),
                // Open florets: a short, wide frustum facing out and up.
                _ => {
                    let e = 66f64.to_radians();
                    b.cone(
                        out(0.0, 0.0),
                        [e.cos() * cp, e.cos() * sp, e.sin()],
                        s * 0.9 + rs,
                        s * 0.2,
                        s * 0.5,
                    );
                }
            }
            b.florets += 1;
        }
    }
    // A pointed tip.
    b.cone(
        [at[0], at[1], z_top - 0.2],
        [0.0, 0.0, 1.0],
        fs * (1.0 - taper) + 1.0,
        rs,
        0.0,
    );
    Ok(
        json!({"kind": kind, "height": height, "rings": rings, "florets_per_ring": per, "taper": taper, "floret_size": fs, "stem_height": sh, "base": dims(&b.base)}),
    )
}

fn umbel(b: &mut Build, p: &Value) -> Result<Value> {
    let rays = num(p, "rays", 12.0, 4.0, 24.0)?.round() as usize;
    let spread = num(p, "spread", 35.0, 10.0, 60.0)?;
    let len = num(p, "ray_length", 8.0, 3.0, 20.0)?;
    let cl = num(p, "cluster", 1.4, 0.6, 3.0)?;
    let sh = stem_height(p, 20.0)?;
    b.base = base_param(p, [2, 1])?;
    let at = base_centre(&b.base);
    let z = BASE_HEIGHT + sh;
    b.stem(at, z + 0.5, 1.2);
    b.sphere([at[0], at[1], z + 0.5], 1.5);
    let inner = (rays / 3).max(1);
    for k in 0..rays {
        let (lean, phi) = if k < inner {
            (spread * 0.45, 2.0 * PI * k as f64 / inner as f64 + 0.2)
        } else {
            let j = k - inner;
            (spread, 2.0 * PI * j as f64 / (rays - inner) as f64)
        };
        let phi = phi + 1e-4 * k as f64;
        let l = lean.to_radians();
        let axis = [l.sin() * phi.cos(), l.sin() * phi.sin(), l.cos()];
        let from = [at[0], at[1], z + 0.5];
        b.rod(from, axis, len, 0.55);
        let end = [
            from[0] + axis[0] * len,
            from[1] + axis[1] * len,
            from[2] + axis[2] * len,
        ];
        b.sphere(end, cl);
        b.florets += 1;
    }
    Ok(
        json!({"rays": rays, "spread": spread, "ray_length": len, "cluster": cl, "stem_height": sh, "base": dims(&b.base)}),
    )
}

fn fern(b: &mut Build, p: &Value) -> Result<Value> {
    let length = num(p, "length", 40.0, 10.0, 56.0)?;
    let pairs = num(p, "pairs", 8.0, 3.0, 16.0)?.round() as usize;
    let curl = num(p, "curl", 60.0, 0.0, 150.0)?;
    let ll = num(p, "leaflet_length", 10.0, 3.0, 25.0)?;
    let lw = num(p, "leaflet_width", 3.0, 1.0, 8.0)?;
    let angle = num(p, "angle", 40.0, 20.0, 80.0)?;
    let t = num(p, "thickness", 1.4, 0.8, 3.0)?;
    let (dname, dv) = dir_param(p, "+x", false)?;
    b.base = base_param(p, [2, 1])?;
    let at = base_centre(&b.base);
    // The rib's centre line in the frond's plane (u toward `direction`, v up), from the
    // base's top, turning over by `curl` at the tip.
    const N: usize = 32;
    let k_curl = curl.to_radians();
    let mut pts = vec![[0.0, BASE_HEIGHT - SINK]];
    let mut tangents = vec![0.0];
    let ds = length / N as f64;
    for i in 0..N {
        let s = (i as f64 + 0.5) * ds;
        let th = k_curl * (s / length).powi(2);
        let last = pts[i];
        pts.push([last[0] + ds * th.sin(), last[1] + ds * th.cos()]);
        tangents.push(k_curl * ((i + 1) as f64 * ds / length).powi(2));
    }
    // Where on the rib (and which way) at arc length s.
    let at_s = |s: f64| -> (P2, f64) {
        let i = ((s / ds).floor() as usize).min(N - 1);
        let f = s / ds - i as f64;
        let (a, c) = (pts[i], pts[i + 1]);
        (
            [a[0] + f * (c[0] - a[0]), a[1] + f * (c[1] - a[1])],
            tangents[i] + f * (tangents[i + 1] - tangents[i]),
        )
    };
    let rib_w = 1.6;
    let mut left = Vec::new();
    let mut right = Vec::new();
    for (i, q) in pts.iter().enumerate() {
        let th = tangents[i];
        let w = rib_w / 2.0 * (1.0 - 0.5 * i as f64 / N as f64);
        // Normal to the left of the direction of travel (sin th, cos th).
        let nl = [-th.cos(), th.sin()];
        left.push([q[0] + w * nl[0], q[1] + w * nl[1]]);
        right.push([q[0] - w * nl[0], q[1] - w * nl[1]]);
    }
    right.reverse();
    let outline: Vec<P2> = left.into_iter().chain(right).collect();
    let plane = |b: &mut Build, prefix: &str| -> String {
        let n = b.name(prefix);
        b.c.push(sketch_on(
            &n,
            [at[0], at[1], 0.0],
            [dv[0], dv[1], 0.0],
            [0.0, 0.0, 1.0],
        ));
        n
    };
    let n = plane(b, "Rib");
    b.c.push(closed(&outline));
    b.c.push(cmd("sketch.finish", json!({})));
    b.c.push(cmd(
        "solid.extrude",
        json!({"sketch": n, "name": format!("{n}Solid"), "distance": r6(t), "operation": "join", "targets": ["Part"], "direction": "symmetric"}),
    ));
    // Leaflets: pairs along the rib, shrinking toward the tip, a little thinner than the rib.
    let a = angle.to_radians();
    for i in 0..pairs {
        let f = i as f64 / pairs as f64;
        let s = length * (0.18 + 0.75 * f);
        let (q, th) = at_s(s);
        let l = ll * (1.0 - 0.65 * f);
        let w = lw * (1.0 - 0.45 * f);
        for side in [-1.0, 1.0] {
            let dir = th + side * a;
            let (ds_, dc) = dir.sin_cos();
            let root = [q[0] + 0.2 * ds_, q[1] + 0.2 * dc];
            let tip = [root[0] + l * ds_, root[1] + l * dc];
            let mid = [(root[0] + tip[0]) / 2.0, (root[1] + tip[1]) / 2.0];
            let nrm = [-dc, ds_];
            let n = plane(b, "Leaflet");
            b.c.extend([
                arc3(root, [mid[0] + nrm[0] * w / 2.0, mid[1] + nrm[1] * w / 2.0], tip),
                arc3(tip, [mid[0] - nrm[0] * w / 2.0, mid[1] - nrm[1] * w / 2.0], root),
                cmd("sketch.finish", json!({})),
                cmd(
                    "solid.extrude",
                    json!({"sketch": n, "name": format!("{n}Solid"), "distance": r6(t * 0.75), "operation": "join", "targets": ["Part"], "direction": "symmetric"}),
                ),
            ]);
            b.florets += 1;
        }
    }
    Ok(
        json!({"length": length, "pairs": pairs, "curl": curl, "leaflet_length": ll, "leaflet_width": lw, "angle": angle, "thickness": t, "direction": dname, "base": dims(&b.base)}),
    )
}

fn stem_template(b: &mut Build, p: &Value) -> Result<Value> {
    let plates = num(p, "height", 6.0, 1.0, 18.0)?.round();
    let d = num(p, "diameter", 3.2, 1.6, 6.0)?;
    let angle = num(p, "angle", 0.0, 0.0, 60.0)?;
    let (dname, dv) = dir_param(p, "+x", false)?;
    let top = p
        .get("top")
        .and_then(Value::as_str)
        .unwrap_or("none")
        .to_string();
    if !["none", "knob", "bud"].contains(&top.as_str()) {
        return Err(CustomError::InvalidShape(
            "`top` is none, knob or bud".into(),
        ));
    }
    b.base = base_param(p, [2, 1])?;
    let at = base_centre(&b.base);
    let l = angle.to_radians();
    let axis = [l.sin() * dv[0], l.sin() * dv[1], l.cos()];
    let len = plates * 3.2 / l.cos() + SINK;
    let from = [at[0], at[1], BASE_HEIGHT - SINK];
    b.rod(from, axis, len, d / 2.0);
    let end = [
        from[0] + axis[0] * len,
        from[1] + axis[1] * len,
        from[2] + axis[2] * len,
    ];
    match top.as_str() {
        "knob" => b.sphere(end, d * 0.7),
        "bud" => {
            // A 45° collar out to the bud's width (no ledge to print onto air), then a point.
            let back = d * 0.4;
            let s0 = [
                end[0] - axis[0] * back,
                end[1] - axis[1] * back,
                end[2] - axis[2] * back,
            ];
            b.cone(s0, axis, d * 0.32 + 0.01, d / 2.0, d * 0.75);
            let s1 = [
                s0[0] + axis[0] * d * 0.32,
                s0[1] + axis[1] * d * 0.32,
                s0[2] + axis[2] * d * 0.32,
            ];
            b.cone(s1, axis, d * 2.0, d * 0.75, 0.0);
        }
        _ => {}
    }
    Ok(
        json!({"height": plates, "diameter": d, "angle": angle, "direction": dname, "top": top, "base": dims(&b.base)}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spike_has_rings_times_florets() {
        for (rings, per) in [(8, 5), (3, 4), (12, 6)] {
            let mut b = Build::default();
            spike(&mut b, &json!({"rings": rings, "florets_per_ring": per})).unwrap();
            assert_eq!(b.florets, rings * per);
        }
    }
}
