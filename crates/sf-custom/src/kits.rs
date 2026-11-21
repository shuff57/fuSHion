//! Kit templates: decorations built from separate printed parts, after LEGO's Botanical
//! bouquets (see [`crate::kit`] for the connectors and how parts print).
//!
//! # How the flowers go together
//! The **hub** sits on the model (it carries the socket base, and a Ø3.18 bar stem when
//! `stem_height` > 0, widening to the hub through a cone steeper than 45°). Round its top
//! stand short vertical **bars**; for a second ring of petals every other bar stands on a
//! pedestal so its clip sits higher. Each **petal** (or fan of petals, or floret) has a clip
//! at its root and snaps onto a bar from the inside; it can swing round the bar. A **centre**
//! presses into a hole in the hub's top with a split peg.
//!
//! # How each part prints (no supports where the shape allows)
//! * **Hub**: upright on its socket base; bars and pedestals vertical; the hole vertical.
//! * **Petals and florets**: upright on the clip's foot, exactly as they sit in the flower
//!   (the clip's C in the bed plane, so its jaws bend within layers). A petal rising at 45° or
//!   more needs no support; flat petals lie on the bed; petals between 0° and 45° are flagged.
//! * **Centres** that are a flat disc print face down (peg up); domed centres and buds print
//!   upright on their peg, widening from it through a cone steeper than 45°.
//! * **Spike flowers**: the stem is a tall Ø3.18 bar; each ring of florets is a collar that
//!   slides down it (three crush ribs press in the bed plane), printed upright.
//! * **Fronds and leaves**: flat on the bed, the clip in their plane, gripping a level
//!   teardrop bar on an upright mount (the bar's 45° V prints without support).
//! * **Beak, ear, horn**: on an upright mount's vertical bar: the beak lies flat, the ear
//!   stands on its edge, the horn stands on its base, each clip in the bed plane.

use crate::detail::{
    BASE_HEIGHT, Detail, SINK, SUPPORT_AREA_MM2, arc3, base_centre, base_param, closed, cmd, dims,
    dir_param, line, num, p3, r6, sketch_on, sketch_xy,
};
use crate::kit::{BAR_D, BuiltKit, ClipSpec, Cmds, Iso, KitDef, PEG_LEN, PartDef, build_kit};
use crate::shape::P2;
use crate::{CustomError, Result};
use serde_json::{Value, json};
use std::f64::consts::PI;

/// The kit templates and their parameters (mm and degrees; `base` `[w, d]` in studs; every
/// flower takes `stem_height` 0–80 mm; `"one_piece": true` builds the old single print).
pub const KITS: &[(&str, &str)] = &[
    (
        "rose",
        r#"{"size": 36 (head width in mm, also "diameter"; 64 mm is 8 studs across; 24-64), "petals": 4 (per ring, 3-5), "open": 0.4 (0-1), "stem_height": 0, "mount": true (a bar hole up the hub for a printed stem's tip bar; lifts the hub about 6 mm; only with stem_height 0), "base": [2, 2]} — hub, 4 outer + 4 middle petals, a curled bud; stem: plain Dark Green Ø5, 2 leaf places; colours: main (outer petals), inner (the inner cup, default Tan), centre (the bud, default White)"#,
    ),
    (
        "daisy",
        r#"{"size": 40 (head width in mm, also "diameter"; 64 mm is 8 studs across; 28-64), "petals": 6 (5-8), "petal_width": 0.7 (0.3-1 of the room), "centre": 0.5 (centre disc / petal ring, 0.3-0.8), "stem_height": 0, "mount": true (a bar hole up the hub for a printed stem's tip bar; lifts the hub about 6 mm; only with stem_height 0), "base": [2, 2]} — hub, flat petals, a centre disc; stem: a forked Dark Green Ø4 sprig, 3 heads at different heights; colours: main (petals), centre (default Yellow)"#,
    ),
    (
        "aster",
        r#"{"size": 34 (head width in mm, also "diameter"; 64 mm is 8 studs across; 26-64), "fans": 4 (per ring, 3-4), "per_fan": 4 (2-5), "cup": 25 (inner ring, 0-60), "stem_height": 0, "mount": true (a bar hole up the hub for a printed stem's tip bar; lifts the hub about 6 mm; only with stem_height 0), "base": [2, 2]} — hub, two rings of petal fans, a domed centre; stem: plain Dark Green Ø5, 1 leaf; colours: main (both rings of fans), centre (default Yellow)"#,
    ),
    (
        "gerbera",
        r#"{"size": 36 (head width in mm, also "diameter"; 64 mm is 8 studs across; 26-64), "fans": 4 (3-4), "per_fan": 4 (2-5), "stem_height": 0, "mount": true (a bar hole up the hub for a printed stem's tip bar; lifts the hub about 6 mm; only with stem_height 0), "base": [2, 2]} — hub, two rings of petal fans, a dark centre; stem: plain, long, Dark Green Ø5; colours: main (both rings of fans), centre (default Reddish Brown)"#,
    ),
    (
        "poppy",
        r#"{"size": 32 (head width in mm, also "diameter"; 64 mm is 8 studs across; 20-64), "petals": 4 (3-5), "open": 0.3 (0-1), "stem_height": 0, "mount": true (a bar hole up the hub for a printed stem's tip bar; lifts the hub about 6 mm; only with stem_height 0), "base": [2, 2]} — hub, cupped petals, a spoked centre; stem: plain Dark Green Ø4.5 with a bud branch; colours: main (petals), centre (default Black)"#,
    ),
    (
        "cornflower",
        r#"{"size": 26 (head width in mm, also "diameter"; 18-48), "florets": 6 (5-6), "angle": 60 (45-85), "stem_height": 0, "mount": true (a bar hole up the hub for a printed stem's tip bar; lifts the hub about 6 mm; only with stem_height 0), "base": [2, 2]} — hub, trumpet florets, a spiky tuft; stem: thin Dark Green Ø3.5, 2 small bud branches; colours: main (florets), centre (default Dark Blue)"#,
    ),
    (
        "spike",
        r#"{"kind": "lupin" (snapdragon|lupin|lavender|larkspur; also "species"), "height": 36 (12-48), "rings": 6 (2-10), "florets_per_ring": 5 (3-6), "taper": 0.3 (0-0.8), "floret_size": 6 (3-8), "stem_height": 6 (0-24), "mount": true (a bar hole up a foot under the stem, for a printed stem's tip bar; needs stem_height ≥ 4.8), "base": [2, 1]} — a bar stem, floret collars in three sizes; stem: snapdragon/lupin/larkspur a straight Dark Green Ø5 stalk with 2 bud branches near the top; lavender a Reddish Brown Ø3.5 sprig branching into 3 more heads"#,
    ),
    (
        "umbel",
        r#"{"rays": 8 (5-10), "spread": 35 (15-40), "ray_length": 8 (4-16), "cluster": 1.6 (1-3), "stem_height": 20 (4-40), "mount": true (a bar hole up a foot under the stem; needs stem_height ≥ 8.8), "base": [2, 1]} — a bar stem, a head of rays; stem: thin Dark Green Ø3.5, 3 branches fanning to more heads"#,
    ),
    (
        "fern",
        r#"{"length": 36 (12-48), "pairs": 7 (3-12), "curl": 50 (0-120), "leaflet_length": 9 (3-20), "leaflet_width": 3 (1.5-6), "angle": 45 (25-75), "thickness": 1.6 (1.2-3), "direction": "+x", "base": [2, 1]} — a mount, a flat frond on a clip (on a stem: only the frond, its clip on a bar section); stem: a Dark Green fan of blades, no head"#,
    ),
    (
        "leaf",
        r#"{"length": 18 (8-40), "width": 8 (3-20), "thickness": 1.6 (1.2-3), "tilt": 0 (lean from upright, -60-60), "direction": "+x", "base": [2, 1]} — a mount, a flat leaf on a clip (on a stem: only the leaf, its clip snapped on one of the stem's bar sections; the mount stays off)"#,
    ),
    (
        "stem",
        r#"{"height": 6 (plates above the base, 1-18), "angle": 0 (0-30), "direction": "+x", "top": "flat" (flat|round), "base": [2, 1]} — a Ø3.18 bar on a base"#,
    ),
    (
        "eye",
        r#"{"diameter": 7 (5-14), "pupil": 0.45 (pupil / eye, 0.25-0.7), "base": [2, 1]} — a domed eye, a pupil pressed in"#,
    ),
    (
        "beak",
        r#"{"length": 9 (4-20), "width": 7 (3-14), "height": 3 (2-6), "direction": "+x", "base": [2, 1]} — a mount, a flat beak on a clip"#,
    ),
    (
        "ear",
        r#"{"width": 10 (5-20), "height": 12 (5-28), "thickness": 1.6 (1.2-3), "direction": "+y", "base": [2, 1]} — a mount, an upright ear on a clip"#,
    ),
    (
        "horn",
        r#"{"length": 12 (5-30), "diameter": 5 (3-10), "lean": 0 (0-30; leaning needs a little support), "direction": "+x", "base": [2, 1]} — a mount, a horn on a clip"#,
    ),
];

/// Builds kit template `template`.
pub(crate) fn kit_template(
    name: &str,
    label: &str,
    template: &str,
    params: &Value,
) -> Result<Detail> {
    if !(params.is_object() || params.is_null()) {
        return Err(CustomError::InvalidShape(
            "template params must be an object".into(),
        ));
    }
    let (def, full) = match template {
        "rose" => rose(params)?,
        "daisy" => daisy(params)?,
        "aster" => fans_flower(params, false)?,
        "gerbera" => fans_flower(params, true)?,
        "poppy" => poppy(params)?,
        "cornflower" => cornflower(params)?,
        "spike" => spike(params)?,
        "umbel" => umbel(params)?,
        "fern" => flat_on_mount(params, true)?,
        "leaf" => flat_on_mount(params, false)?,
        "stem" => stem(params)?,
        "eye" => eye(params)?,
        "beak" => beak(params)?,
        "ear" => ear(params)?,
        "horn" => horn(params)?,
        other => return Err(CustomError::InvalidShape(format!("no kit `{other}`"))),
    };
    let style = crate::stem::stem_style(template, &full);
    let BuiltKit {
        mut design,
        prints,
        overhangs,
    } = build_kit(name, label, def, json!({"kit": template, "params": full}))?;
    design.stem = style;
    let kit = design.kit.as_ref().expect("a kit");
    let based = kit
        .parts
        .iter()
        .position(|p| p.key == "hub" || p.key == "mount" || p.key == "stem" || p.key == "eye")
        .unwrap_or(0);
    let total: f64 = overhangs.iter().sum();
    Ok(Detail {
        supports: overhangs.iter().any(|o| *o > SUPPORT_AREA_MM2),
        overhang_mm2: total,
        part: prints[based].clone(),
        kit_prints: prints,
        design,
    })
}

fn clip() -> ClipSpec {
    ClipSpec::default()
}

/// Neck from a clip's back to the part it holds, mm.
const NECK: f64 = 1.4;
/// From a mount post's middle to where a clip on its level bar starts, mm.
const POST_GAP: f64 = 2.4;

// ---------------------------------------------------------------- hub and petals

/// The hub's bars.
struct Hub {
    stem_height: f64,
    /// A bar hole up into it for a stem's tip bar (only with no `stem_height` of its own).
    mount: bool,
    /// Bar circle radius.
    r_b: f64,
    /// Angles of the low bars and of the high bars (on pedestals).
    low: Vec<f64>,
    high: Vec<f64>,
}

/// The hub part (on the socket base, upright), its top's height, and its stem mount.
fn hub(at: P2, h: &Hub, base: &[(i32, i32)]) -> (PartDef, f64, Option<crate::stem::MountMm>) {
    let c = clip();
    let mut k = Cmds::based();
    let r_hub = h.r_b + c.r_out() + 0.6;
    let hm = (h.mount && h.stem_height <= 0.0).then(|| crate::stem::head_mount(at, base, true));
    // The hub's top without a mount: 2 mm over the base's top, or over its cone.
    let r0 = r_hub.min(7.0);
    let cone_rise = if r_hub > r0 + 0.05 {
        (r_hub - r0) * 1.15 + SINK
    } else {
        0.0
    };
    let plain_top = BASE_HEIGHT - SINK + cone_rise + 2.0;
    // On a stem: a column lifts the hub until its bar hole reaches the centre's hole.
    let lift = hm.as_ref().map_or(0.0, |m| (m.top - plain_top).max(0.0));
    let z_b = BASE_HEIGHT - SINK + lift;
    if lift > 0.0 {
        k.cylinder(
            [at[0], at[1], BASE_HEIGHT - SINK],
            [0.0, 0.0, 1.0],
            lift + 0.01,
            r0,
            false,
        );
    }
    let z = if h.stem_height > 0.0 {
        let zs = BASE_HEIGHT + h.stem_height;
        k.bar([at[0], at[1], z_b], zs - z_b + 0.3);
        let rise = (r_hub - BAR_D / 2.0) * 1.15;
        k.cone(
            [at[0], at[1], zs],
            [0.0, 0.0, 1.0],
            rise + 0.01,
            BAR_D / 2.0,
            r_hub,
            false,
        );
        zs + rise
    } else {
        // From the base's top out to the hub, steeper than 45° (no ledge over air).
        let r0 = r_hub.min(7.0);
        if r_hub > r0 + 0.05 {
            let rise = (r_hub - r0) * 1.15;
            k.cone(
                [at[0], at[1], z_b],
                [0.0, 0.0, 1.0],
                rise + SINK,
                r0,
                r_hub,
                false,
            );
            z_b + SINK + rise
        } else {
            z_b
        }
    };
    let top = z + 2.0;
    k.cylinder(
        [at[0], at[1], z - 0.01],
        [0.0, 0.0, 1.0],
        top - z + 0.01,
        r_hub,
        false,
    );
    let hc = c.height;
    for &a in &h.low {
        let (s, co) = a.sin_cos();
        k.bar([at[0] + h.r_b * co, at[1] + h.r_b * s, top - 0.3], hc + 0.8);
    }
    for &a in &h.high {
        let (s, co) = a.sin_cos();
        let (x, y) = (at[0] + h.r_b * co, at[1] + h.r_b * s);
        k.cylinder([x, y, top - 0.3], [0.0, 0.0, 1.0], hc + 0.7, 2.3, false);
        k.bar([x, y, top + hc + 0.1], hc + 0.8);
    }
    k.hole([at[0], at[1], top], PEG_LEN + 0.5);
    let mut post = Vec::new();
    let mount = hm.map(|m| {
        k.c.extend(m.cmds);
        // A hub already taller than the bar hole needs: a Ø3 hole on up to the centre's.
        let bore_top = m.mount.origin[2]
            + crate::stem::BAR_MOUNT_DEPTH
            + (crate::stem::BAR_BORE_D / 2.0 - 1.5) * 1.2
            - 0.02;
        let hole_bottom = top - PEG_LEN - 0.5;
        if hole_bottom > bore_top {
            k.cylinder(
                [at[0], at[1], bore_top],
                [0.0, 0.0, 1.0],
                hole_bottom - bore_top + 0.1,
                1.5,
                true,
            );
        }
        post = m.post;
        m.mount
    });
    (
        PartDef {
            key: "hub".into(),
            label: "hub".into(),
            role: "stem".into(),
            cmds: k.c,
            base: Some(base.to_vec()),
            connect: [0.0, 0.0, 0.0],
            copies: vec![Iso::translate([0.0, 0.0, 0.0])],
            orientation: "upright on its socket base: bars, pedestals and the centre hole vertical; on a stem, the bar hole up the middle under a 50° roof".into(),
            post,
        },
        top,
        mount,
    )
}

/// Biggest flower head the kits make, mm across (a petal part stays well under
/// [`crate::MAX_PART_MM`]).
pub const MAX_HEAD: f64 = 64.0;

/// The bar circle for a head `size` mm across that is `default` mm at `r`: as big as `r`
/// for smaller heads, growing with the head above it, so a big flower's hub, centre and
/// petal roots scale with its petals (clips and bars keep their size).
fn grown(r: f64, size: f64, default: f64) -> f64 {
    r * (size / default).max(1.0)
}

/// The smallest bar circle for `n` low bars and `m` high bars (alternating when both).
fn bar_radius(n: usize, m: usize) -> f64 {
    let c = clip();
    let total = n + m;
    // Clips on bars of one height must clear each other; a low clip must clear the
    // neighbouring high bar's pedestal.
    let same = 2.0 * c.r_out() + 0.6;
    let mixed = c.r_out() + 2.3 + 0.3;
    let need_same = same / (2.0 * (PI / n.max(1) as f64).sin());
    let need_mixed = if m > 0 {
        mixed / (2.0 * (PI / total as f64).sin())
    } else {
        0.0
    };
    need_same.max(need_mixed).max(3.0)
}

/// A ring of petals' shape.
#[derive(Clone, Copy)]
struct Petals {
    /// Petals on one clip, and the angle between neighbouring petals.
    per: usize,
    step: f64,
    /// Slope at the root and at the rim, degrees.
    b0: f64,
    b1: f64,
    /// Radius from the flower's axis the tips reach.
    reach: f64,
    t: f64,
    /// Widest petal (less where there is no room).
    w: f64,
}

/// A petal part: a clip (mouth toward the flower's axis), its neck, and `per` petals cut
/// from a cupped band round the flower's axis, which lies at `(−r_b, 0)` in its print frame.
fn petal_part(key: &str, label: &str, r_b: f64, p: &Petals) -> Result<Vec<Value>> {
    let c = clip();
    let mut k = Cmds::default();
    let neck_end = k.clip([0.0, 0.0], 0.0, PI, &c, NECK);
    let axis = [-r_b, 0.0];
    // Radii from the flower's axis: the root strip from u0 to us, the band from r0.
    let u0 = neck_end[0] + r_b - 0.5;
    let us = u0 + 1.2;
    let r0 = u0 - 0.6;
    let (b0, b1) = (p.b0.to_radians(), p.b1.to_radians());
    let dr = (p.reach - r0).max(1.0);
    let len = if (b1 - b0).abs() < 1e-3 {
        dr / b0.cos()
    } else {
        dr * (b1 - b0) / (b1.sin() - b0.sin())
    };
    let pt = |s: f64, off: f64| -> P2 {
        let (x, z, beta) = if (b1 - b0).abs() < 1e-3 {
            (r0 + s * b0.cos(), s * b0.sin(), b0)
        } else {
            let kk = (b1 - b0) / len;
            let beta = b0 + kk * s;
            (
                r0 + (beta.sin() - b0.sin()) / kk,
                -(beta.cos() - b0.cos()) / kk,
                beta,
            )
        };
        [x - off * beta.sin(), z + off * beta.cos()]
    };
    let (q0, qm, q1) = (pt(0.0, 0.0), pt(len / 2.0, 0.0), pt(len, 0.0));
    let (s0, sm, s1) = (pt(0.0, p.t), pt(len / 2.0, p.t), pt(len, p.t));
    let band = format!("Band{key}");
    let sk = format!("BandSk{key}");
    k.c.push(sketch_on(
        &sk,
        [axis[0], axis[1], 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
    ));
    if (b1 - b0).abs() < 1e-3 {
        k.c.push(closed(&[q0, q1, s1, s0]));
    } else {
        k.c.extend([
            arc3(q0, qm, q1),
            line(q1, s1),
            arc3(s1, sm, s0),
            line(s0, q0),
        ]);
    }
    k.c.push(cmd("sketch.finish", json!({})));
    k.c.push(cmd(
        "solid.revolve",
        json!({"sketch": sk, "axis": {"origin": p3([axis[0], axis[1], 0.0]), "dir": [0, 0, 1]}, "operation": "new", "body_name": band, "name": format!("{sk}Solid")}),
    ));
    // The petals' outline from above, round the flower's axis.
    let rim = q1[0].min(s1[0]);
    let u1 = rim - 0.25;
    let kk = (p.step / 2.0).sin() * 0.92;
    let w = p.w.min(2.0 * u1 * kk / (1.0 + kk));
    let wr = (w * 0.45)
        .min(2.0 * us * (p.step / 2.0).sin() * 0.85)
        .max(1.0)
        .min(w);
    let um = (u1 - w / 2.0).max(us + 0.3);
    if w < 1.2 || u1 <= us + 1.0 {
        return Err(CustomError::InvalidShape(format!(
            "{label}: no room for petals between the clip (out to {us:.1} mm) and the rim ({u1:.1} mm); make the flower bigger"
        )));
    }
    let ol = format!("Outline{key}");
    let zlo = q0[1].min(q1[1]).min(s0[1]) - 2.0;
    k.c.push(sketch_xy(&ol, zlo));
    let ang = |j: usize| (j as f64 - (p.per as f64 - 1.0) / 2.0) * p.step + 1e-4 * j as f64;
    let at = |phi: f64, u: f64, v: f64| -> P2 {
        let (s, co) = phi.sin_cos();
        [axis[0] + u * co - v * s, axis[1] + u * s + v * co]
    };
    for j in 0..p.per {
        let phi = ang(j);
        let a = at(phi, us, -wr / 2.0);
        let b = at(phi, um, -w / 2.0);
        let tip = at(phi, u1, 0.0);
        let cc = at(phi, um, w / 2.0);
        let d = at(phi, us, wr / 2.0);
        if j == 0 {
            // From the root strip's inner edge to the first petal.
            let start = at(ang(0) - 0.6 * p.step / 2.0, u0, 0.0);
            k.c.push(line(start, a));
        }
        k.c.push(line(a, b));
        k.c.push(arc3(b, tip, cc));
        k.c.push(line(cc, d));
        if j + 1 < p.per {
            k.c.push(line(d, at(ang(j + 1), us, -wr / 2.0)));
        } else {
            let end = at(phi + 0.6 * p.step / 2.0, u0, 0.0);
            let start = at(ang(0) - 0.6 * p.step / 2.0, u0, 0.0);
            k.c.push(line(d, end));
            k.c.push(line(end, start));
        }
    }
    k.c.push(cmd("sketch.finish", json!({})));
    let ztop = q1[1].max(s1[1]).max(qm[1]).max(sm[1]) + 4.0;
    k.c.push(cmd(
        "solid.extrude",
        json!({"sketch": ol, "name": format!("{ol}Cut"), "distance": r6(ztop - zlo), "operation": "intersect", "targets": [band]}),
    ));
    k.c.push(cmd(
        "solid.combine",
        json!({"target": "Part", "tools": [band], "operation": "join"}),
    ));
    Ok(k.c)
}

/// The copies of a petal part on bars at `angles`, clip feet at height `z`.
fn ring_copies(at: P2, r_b: f64, angles: &[f64], z: f64) -> Vec<Iso> {
    angles
        .iter()
        .map(|&a| {
            let (s, c) = a.sin_cos();
            Iso::rz(a, [at[0] + r_b * c, at[1] + r_b * s, z])
        })
        .collect()
}

fn petal_def(
    key: &str,
    label: &str,
    role: &str,
    cmds: Vec<Value>,
    copies: Vec<Iso>,
    why: &str,
) -> PartDef {
    PartDef {
        key: key.into(),
        label: label.into(),
        role: role.into(),
        cmds,
        base: None,
        connect: [0.0, 0.0, 0.0],
        copies,
        orientation: why.into(),
        post: Vec::new(),
    }
}

const PETAL_WHY: &str = "upright on its clip's foot, as it sits in the flower: the clip's C in the bed plane so the jaws bend within layers; petals rising at 45° or more need no support";

fn angles(n: usize, offset: f64) -> Vec<f64> {
    (0..n)
        .map(|i| offset + 2.0 * PI * i as f64 / n as f64)
        .collect()
}

/// A centre that prints upright on a split peg (pointing down), widening from it through
/// a cone steeper than 45° to `r` at `z_top` above the peg's tip; `extra` adds what stands
/// on top. Returns the part and the height of its platform above the peg tip.
fn upright_centre(r: f64, extra: impl FnOnce(&mut Cmds, f64)) -> (Vec<Value>, f64) {
    let mut k = Cmds::default();
    k.peg([0.0, 0.0, PEG_LEN], -1.0);
    let rise = ((r - crate::kit::PEG_D / 2.0) * 1.15).max(0.4);
    k.cone(
        [0.0, 0.0, PEG_LEN - 0.2],
        [0.0, 0.0, 1.0],
        rise + 0.2,
        crate::kit::PEG_D / 2.0,
        r,
        false,
    );
    // A short platform of radius `r`, so what stands on it starts inside it.
    k.cylinder(
        [0.0, 0.0, PEG_LEN + rise - 0.01],
        [0.0, 0.0, 1.0],
        0.61,
        r,
        false,
    );
    let z = PEG_LEN + rise + 0.6;
    extra(&mut k, z);
    (k.c, z)
}

/// The copy of a peg-down centre pressed fully into the hub's hole.
fn centre_copy(at: P2, hub_top: f64) -> Iso {
    Iso::translate([at[0], at[1], hub_top - PEG_LEN])
}

/// A flat centre printed face down with its peg up; returns the part and its peg tip height.
fn face_down_centre(
    r: f64,
    thickness: f64,
    column: f64,
    extra: impl FnOnce(&mut Cmds),
) -> (Vec<Value>, f64) {
    let mut k = Cmds::default();
    k.cylinder([0.0, 0.0, 0.0], [0.0, 0.0, 1.0], thickness, r, false);
    extra(&mut k);
    let mut z = thickness;
    if column > 0.0 {
        k.cylinder(
            [0.0, 0.0, z - 0.2],
            [0.0, 0.0, 1.0],
            column + 0.2,
            2.0,
            false,
        );
        z += column;
    }
    k.peg([0.0, 0.0, z - 0.01], 1.0);
    (k.c, z + PEG_LEN)
}

/// The copy of a face-down centre turned over and pressed into the hub's hole.
fn flipped_copy(at: P2, hub_top: f64, tip: f64) -> Iso {
    // Turned over about x: (x, y, z) → (x, −y, −z); the peg's tip goes to the hole.
    Iso {
        r: [[1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, -1.0]],
        t: [at[0], at[1], hub_top - PEG_LEN + tip],
    }
}

fn stem_height(p: &Value) -> Result<f64> {
    num(p, "stem_height", 0.0, 0.0, 80.0)
}

fn rose(p: &Value) -> Result<(KitDef, Value)> {
    let size = num(p, "size", 36.0, 24.0, MAX_HEAD)?;
    let n = num(p, "petals", 4.0, 3.0, 5.0)?.round() as usize;
    let open = num(p, "open", 0.4, 0.0, 1.0)?;
    let sh = stem_height(p)?;
    let base = base_param(p, [2, 2])?;
    let at = base_centre(&base);
    let r_b = grown(bar_radius(n, n), size, 36.0);
    let step = 2.0 * PI / n as f64;
    let h = Hub {
        stem_height: sh,
        mount: crate::stem::wants_mount(p) && sh == 0.0,
        r_b,
        low: angles(n, 0.0),
        high: angles(n, step / 2.0),
    };
    let (hub_part, top, hub_mount) = hub(at, &h, &base);
    let c = clip();
    let r = size / 2.0;
    let outer = petal_part(
        "o",
        "outer petal",
        r_b,
        &Petals {
            per: 1,
            step,
            b0: 50.0,
            b1: 80.0 - 40.0 * open,
            reach: r,
            t: 1.2,
            w: size,
        },
    )?;
    let middle = petal_part(
        "m",
        "middle petal",
        r_b,
        &Petals {
            per: 1,
            step,
            b0: 66.0,
            b1: 86.0 - 22.0 * open,
            reach: 0.78 * r,
            t: 1.2,
            w: size,
        },
    )?;
    // The bud: spiral walls round a domed core, printed upright on its peg.
    let r_bud = (r_b - c.r_out() - 0.5).max(2.4);
    let (bud, _) = upright_centre(r_bud, |k, z| {
        let walls = 4;
        for i in 0..walls {
            let f = i as f64 / walls as f64;
            let ro = r_bud * (1.0 - 0.5 * f);
            let hgt = r * (0.5 - 0.12 * f);
            k.revolve(
                [0.0, 0.0],
                (137.5 * i as f64).to_radians(),
                vec![closed(&[
                    [ro - 0.9, z - 0.3],
                    [ro, z - 0.3],
                    [ro, z + hgt],
                    [ro - 0.9, z + hgt],
                ])],
                230.0,
            );
        }
        let rc = (r_bud * 0.5 - 0.15).max(0.8);
        k.cylinder(
            [0.0, 0.0, z - 0.3],
            [0.0, 0.0, 1.0],
            r * 0.3 + 0.3,
            rc,
            false,
        );
        k.revolve(
            [0.0, 0.0],
            0.0,
            cap_profile(rc - 0.05, z + r * 0.3 - 0.05, rc * 0.8),
            360.0,
        );
    });
    let hc = c.height;
    let parts = vec![
        hub_part,
        petal_def(
            "outer_petal",
            "outer petal",
            "main",
            outer,
            ring_copies(at, r_b, &h.low, top),
            PETAL_WHY,
        ),
        petal_def(
            "middle_petal",
            "middle petal",
            "inner",
            middle,
            ring_copies(at, r_b, &h.high, top + hc + 0.4),
            PETAL_WHY,
        ),
        petal_def(
            "bud",
            "bud",
            "centre",
            bud,
            vec![centre_copy(at, top)],
            "upright on its peg, widening through a cone steeper than 45°; the peg's halves close in the bed plane",
        ),
    ];
    let steps = vec![
        (vec![(0, 0)], "the hub".to_string()),
        (
            (0..n).map(|i| (1, i)).collect(),
            format!("clip {n} outer petals onto the low bars"),
        ),
        (
            (0..n).map(|i| (2, i)).collect(),
            format!("clip {n} middle petals onto the high bars"),
        ),
        (vec![(3, 0)], "press the bud into the middle".to_string()),
    ];
    Ok((
        KitDef {
            parts,
            steps,
            roles: vec![("main", "Red"), ("inner", "Tan"), ("centre", "White"), ("stem", "Green")],
            sockets: base.clone(),
            mount: hub_mount,
        },
        json!({"size": size, "petals": n, "open": open, "stem_height": sh, "mount": h.mount, "base": dims(&base)}),
    ))
}

/// A spherical cap profile (rim `r` at `zb`, `h` tall) for [`Cmds::revolve`].
fn cap_profile(r: f64, zb: f64, h: f64) -> Vec<Value> {
    let c = zb + (h * h - r * r) / (2.0 * h);
    let rho = zb + h - c;
    let a0 = (zb - c).atan2(r);
    let am = (a0 + PI / 2.0) / 2.0;
    vec![
        line([0.0, zb], [r, zb]),
        arc3([r, zb], [rho * am.cos(), c + rho * am.sin()], [0.0, zb + h]),
        line([0.0, zb + h], [0.0, zb]),
    ]
}

fn daisy(p: &Value) -> Result<(KitDef, Value)> {
    let size = num(p, "size", 40.0, 28.0, MAX_HEAD)?;
    let n = num(p, "petals", 6.0, 5.0, 8.0)?.round() as usize;
    let pw = num(p, "petal_width", 0.7, 0.3, 1.0)?;
    let centre = num(p, "centre", 0.5, 0.3, 0.8)?;
    let sh = stem_height(p)?;
    let base = base_param(p, [2, 2])?;
    let at = base_centre(&base);
    let r_b = grown(bar_radius(n, 0), size, 40.0);
    let step = 2.0 * PI / n as f64;
    let h = Hub {
        stem_height: sh,
        mount: crate::stem::wants_mount(p) && sh == 0.0,
        r_b,
        low: angles(n, 0.0),
        high: vec![],
    };
    let (hub_part, top, hub_mount) = hub(at, &h, &base);
    let c = clip();
    // As wide as `petal_width` of the room at the petal's middle.
    let room = 2.0 * (r_b + size / 2.0) / 2.0 * (step / 2.0).sin();
    let petal = petal_part(
        "p",
        "petal",
        r_b,
        &Petals {
            per: 1,
            step,
            b0: 0.0,
            b1: 0.0,
            reach: size / 2.0,
            t: 1.2,
            w: pw * room,
        },
    )?;
    // A disc over the clips, on a column down to the hole.
    let rc = (r_b + c.r_out()) * (0.6 + 0.5 * centre);
    let (disc, tip) = face_down_centre(rc, 1.6, c.height + 0.8, |_| {});
    let parts = vec![
        hub_part,
        petal_def(
            "petal",
            "petal",
            "main",
            petal,
            ring_copies(at, r_b, &h.low, top),
            "flat on the bed, its clip's foot level with it: the C in the bed plane",
        ),
        petal_def(
            "centre",
            "centre",
            "centre",
            disc,
            vec![flipped_copy(at, top, tip)],
            "face down, peg up: the disc needs no support and the peg's halves close in the bed plane",
        ),
    ];
    let steps = vec![
        (vec![(0, 0)], "the hub".into()),
        (
            (0..n).map(|i| (1, i)).collect(),
            format!("clip {n} petals onto the bars"),
        ),
        (vec![(2, 0)], "press the centre in".into()),
    ];
    Ok((
        KitDef {
            parts,
            steps,
            roles: vec![("main", "White"), ("centre", "Yellow"), ("stem", "Green")],
            sockets: base.clone(),
            mount: hub_mount,
        },
        json!({"size": size, "petals": n, "petal_width": pw, "centre": centre, "stem_height": sh, "mount": h.mount, "base": dims(&base)}),
    ))
}

/// Aster (narrow petals, a domed centre) or gerbera (wider petals, a dark disc centre):
/// two rings of petal fans, the inner ring on high bars and cupped.
fn fans_flower(p: &Value, gerbera: bool) -> Result<(KitDef, Value)> {
    let size = num(
        p,
        "size",
        if gerbera { 36.0 } else { 34.0 },
        26.0,
        MAX_HEAD,
    )?;
    let n = num(p, "fans", 4.0, 3.0, 4.0)?.round() as usize;
    let per = num(p, "per_fan", 4.0, 2.0, 5.0)?.round() as usize;
    let cup = num(p, "cup", 25.0, 0.0, 60.0)?;
    let sh = stem_height(p)?;
    let base = base_param(p, [2, 2])?;
    let at = base_centre(&base);
    let r_b = grown(bar_radius(n, n), size, if gerbera { 36.0 } else { 34.0 });
    let fan_step = 2.0 * PI / n as f64;
    let step = fan_step / per as f64;
    let h = Hub {
        stem_height: sh,
        mount: crate::stem::wants_mount(p) && sh == 0.0,
        r_b,
        low: angles(n, 0.0),
        high: angles(n, fan_step / 2.0),
    };
    let (hub_part, top, hub_mount) = hub(at, &h, &base);
    let c = clip();
    let r = size / 2.0;
    let wf = if gerbera { 0.8 } else { 0.55 };
    let outer = petal_part(
        "o",
        "outer fan",
        r_b,
        &Petals {
            per,
            step,
            b0: 0.0,
            b1: 0.0,
            reach: r,
            t: 1.0,
            w: wf * 2.0 * r * (step / 2.0).sin(),
        },
    )?;
    let inner = petal_part(
        "i",
        "inner fan",
        r_b,
        &Petals {
            per,
            step,
            b0: cup,
            b1: cup + 8.0,
            reach: 0.82 * r,
            t: 1.0,
            w: wf * 2.0 * r * (step / 2.0).sin(),
        },
    )?;
    let r_c = (r_b - c.r_out() - 0.4).max(2.4);
    let (centre, _) = upright_centre(r_c, |k, z| {
        if gerbera {
            k.cylinder([0.0, 0.0, z - 0.2], [0.0, 0.0, 1.0], 1.6, r_c, false);
            k.revolve(
                [0.0, 0.0],
                0.0,
                cap_profile(r_c * 0.7, z + 1.2, r_c * 0.35),
                360.0,
            );
        } else {
            k.revolve(
                [0.0, 0.0],
                0.0,
                cap_profile(r_c - 0.05, z - 0.2, r_c * 0.6),
                360.0,
            );
        }
    });
    let flat_why = "upright on its clip's foot as it sits in the flower: the C in the bed plane. Outer fans lie flat on the bed; the cupped inner fans rise at less than 45° and are flagged";
    let parts = vec![
        hub_part,
        petal_def(
            "outer_fan",
            "outer petal fan",
            "main",
            outer,
            ring_copies(at, r_b, &h.low, top),
            flat_why,
        ),
        petal_def(
            "inner_fan",
            "inner petal fan",
            "main",
            inner,
            ring_copies(at, r_b, &h.high, top + c.height + 0.4),
            flat_why,
        ),
        petal_def(
            "centre",
            "centre",
            "centre",
            centre,
            vec![centre_copy(at, top)],
            "upright on its peg, widening through a cone steeper than 45°",
        ),
    ];
    let steps = vec![
        (vec![(0, 0)], "the hub".into()),
        (
            (0..n).map(|i| (1, i)).collect(),
            format!("clip {n} outer fans onto the low bars"),
        ),
        (
            (0..n).map(|i| (2, i)).collect(),
            format!("clip {n} inner fans onto the high bars"),
        ),
        (vec![(3, 0)], "press the centre in".into()),
    ];
    let roles = if gerbera {
        vec![
            ("main", "Orange"),
            ("centre", "Reddish Brown"),
            ("stem", "Green"),
        ]
    } else {
        vec![
            ("main", "Medium Lavender"),
            ("centre", "Yellow"),
            ("stem", "Green"),
        ]
    };
    Ok((
        KitDef {
            parts,
            steps,
            roles,
            sockets: base.clone(),
            mount: hub_mount,
        },
        json!({"size": size, "fans": n, "per_fan": per, "cup": cup, "stem_height": sh, "mount": h.mount, "base": dims(&base)}),
    ))
}

fn poppy(p: &Value) -> Result<(KitDef, Value)> {
    let size = num(p, "size", 32.0, 20.0, MAX_HEAD)?;
    let n = num(p, "petals", 4.0, 3.0, 5.0)?.round() as usize;
    let open = num(p, "open", 0.3, 0.0, 1.0)?;
    let sh = stem_height(p)?;
    let base = base_param(p, [2, 2])?;
    let at = base_centre(&base);
    let r_b = grown(bar_radius(n, 0).max(4.5), size, 32.0);
    let step = 2.0 * PI / n as f64;
    let h = Hub {
        stem_height: sh,
        mount: crate::stem::wants_mount(p) && sh == 0.0,
        r_b,
        low: angles(n, PI / 4.0),
        high: vec![],
    };
    let (hub_part, top, hub_mount) = hub(at, &h, &base);
    let c = clip();
    let petal = petal_part(
        "p",
        "petal",
        r_b,
        &Petals {
            per: 1,
            step,
            b0: 48.0,
            b1: 74.0 - 28.0 * open,
            reach: size / 2.0,
            t: 1.2,
            w: size,
        },
    )?;
    // A flat centre with eight spokes, face down; its column clears the clips.
    let rc = (r_b + c.r_out()) * 0.75;
    // A flat disc, face down. (Spokes cut into the face would print as bridges.)
    let (centre, tip) = face_down_centre(rc, 1.2, c.height + 0.8, |_| {});
    let parts = vec![
        hub_part,
        petal_def(
            "petal",
            "petal",
            "main",
            petal,
            ring_copies(at, r_b, &h.low, top),
            PETAL_WHY,
        ),
        petal_def(
            "centre",
            "centre",
            "centre",
            centre,
            vec![flipped_copy(at, top, tip)],
            "face down, peg up: a flat disc, the peg's halves close in the bed plane",
        ),
    ];
    let steps = vec![
        (vec![(0, 0)], "the hub".into()),
        (
            (0..n).map(|i| (1, i)).collect(),
            format!("clip {n} petals onto the bars"),
        ),
        (vec![(2, 0)], "press the centre in".into()),
    ];
    Ok((
        KitDef {
            parts,
            steps,
            roles: vec![("main", "Red"), ("centre", "Black"), ("stem", "Green")],
            sockets: base.clone(),
            mount: hub_mount,
        },
        json!({"size": size, "petals": n, "open": open, "stem_height": sh, "mount": h.mount, "base": dims(&base)}),
    ))
}

fn cornflower(p: &Value) -> Result<(KitDef, Value)> {
    let size = num(p, "size", 26.0, 18.0, 48.0)?;
    let n = num(p, "florets", 6.0, 5.0, 6.0)?.round() as usize;
    let angle = num(p, "angle", 60.0, 45.0, 85.0)?;
    let sh = stem_height(p)?;
    let base = base_param(p, [2, 2])?;
    let at = base_centre(&base);
    let r_b = grown(bar_radius(n, 0), size, 26.0);
    let h = Hub {
        stem_height: sh,
        mount: crate::stem::wants_mount(p) && sh == 0.0,
        r_b,
        low: angles(n, 0.0),
        high: vec![],
    };
    let (hub_part, top, hub_mount) = hub(at, &h, &base);
    let c = clip();
    // A floret: the clip, then a trumpet rising outward from the neck, hollow at the mouth.
    let mut k = Cmds::default();
    let ne = k.clip([0.0, 0.0], 0.0, PI, &c, NECK);
    let e = angle.to_radians();
    let axis = [e.cos(), 0.0, e.sin()];
    let len = (size / 2.0 - r_b - 1.0).max(5.0) / e.cos().max(0.3);
    let len = len.min(0.8 * size);
    let mouth = 0.14 * size;
    // The root circle (tilted with the trumpet) inside the neck, its low edge above the bed.
    let from = [ne[0] - 0.4, 0.0, 0.75 * e.cos() + 0.05];
    k.cone(from, axis, len, 0.75, mouth, false);
    let out = 0.5;
    let start = [
        from[0] + axis[0] * (len + out),
        0.0,
        from[2] + axis[2] * (len + out),
    ];
    k.cone(
        start,
        [-axis[0], 0.0, -axis[2]],
        out + 0.45 * len,
        mouth * 0.8,
        0.25,
        true,
    );
    let floret = k.c;
    let rc = (r_b - c.r_out() - 0.4).max(2.4);
    let (tuft, _) = upright_centre(rc, |k, z| {
        let mut star = Vec::new();
        for i in 0..16 {
            let a = PI * i as f64 / 8.0 + 0.05;
            let rr = if i % 2 == 0 { rc * 0.95 } else { rc * 0.45 };
            star.push([rr * a.cos(), rr * a.sin()]);
        }
        k.prism(
            sketch_xy("", z - 0.3),
            vec![closed(&star)],
            0.12 * size + 0.3,
            false,
            false,
        );
    });
    let parts = vec![
        hub_part,
        petal_def(
            "floret",
            "floret",
            "main",
            floret,
            ring_copies(at, r_b, &h.low, top),
            "upright on its clip's foot as it sits in the flower; the trumpet rises steeper than 45°",
        ),
        petal_def(
            "tuft",
            "centre tuft",
            "centre",
            tuft,
            vec![centre_copy(at, top)],
            "upright on its peg",
        ),
    ];
    let steps = vec![
        (vec![(0, 0)], "the hub".into()),
        (
            (0..n).map(|i| (1, i)).collect(),
            format!("clip {n} florets onto the bars"),
        ),
        (vec![(2, 0)], "press the tuft in".into()),
    ];
    Ok((
        KitDef {
            parts,
            steps,
            roles: vec![("main", "Blue"), ("centre", "Dark Blue"), ("stem", "Green")],
            sockets: base.clone(),
            mount: hub_mount,
        },
        json!({"size": size, "florets": n, "angle": angle, "stem_height": sh, "mount": h.mount, "base": dims(&base)}),
    ))
}

fn spike(p: &Value) -> Result<(KitDef, Value)> {
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
    let height = num(p, "height", 36.0, 12.0, 48.0)?;
    let rings = num(p, "rings", 6.0, 2.0, 10.0)?.round() as usize;
    let per = num(p, "florets_per_ring", 5.0, 3.0, 6.0)?.round() as usize;
    let taper = num(p, "taper", 0.3, 0.0, 0.8)?;
    let fs = num(p, "floret_size", 6.0, 3.0, 8.0)?;
    let sh = num(p, "stem_height", 6.0, 0.0, 24.0)?;
    let base = base_param(p, [2, 1])?;
    let at = base_centre(&base);
    let dz = height / rings as f64;
    if dz < 3.0 {
        return Err(CustomError::InvalidShape(
            "rings closer than 3 mm: fewer rings or more height".into(),
        ));
    }
    // The stem: a bar with a stop under the first collar, tapering at the top.
    let mut k = Cmds::based();
    let z_b = BASE_HEIGHT - SINK;
    let zf = BASE_HEIGHT + sh;
    // On a stem: a foot round a bar hole, inside the stop under the first collar.
    let hm = crate::stem::wants_mount(p)
        .then(|| crate::stem::head_mount(at, &base, false))
        .filter(|m| m.top <= zf + 0.9);
    if let Some(m) = &hm {
        k.cylinder(
            [at[0], at[1], z_b],
            [0.0, 0.0, 1.0],
            m.top - z_b,
            crate::stem::MOUNT_COLUMN_R,
            false,
        );
    }
    k.bar([at[0], at[1], z_b], zf + height + 1.0 - z_b);
    k.cone(
        [at[0], at[1], zf - 1.8],
        [0.0, 0.0, 1.0],
        1.8,
        BAR_D / 2.0,
        3.2,
        false,
    );
    k.cylinder([at[0], at[1], zf - 0.01], [0.0, 0.0, 1.0], 1.0, 3.2, false);
    let z_tip = zf + 1.0 + height;
    k.cone(
        [at[0], at[1], z_tip - 0.01],
        [0.0, 0.0, 1.0],
        2.5,
        BAR_D / 2.0,
        0.6,
        false,
    );
    let mut stem_post = Vec::new();
    let spike_mount = hm.map(|m| {
        k.c.extend(m.cmds);
        stem_post = m.post;
        m.mount
    });
    if BASE_HEIGHT + sh + height + 4.0 > crate::MAX_PART_MM {
        return Err(CustomError::InvalidShape(
            "stem_height + height too tall for one print".into(),
        ));
    }
    let stem_part = PartDef {
        key: "stem".into(),
        label: "stem".into(),
        role: "stem".into(),
        cmds: k.c,
        base: Some(base.clone()),
        connect: [0.0, 0.0, 0.0],
        copies: vec![Iso::translate([0.0, 0.0, 0.0])],
        orientation: "upright on its socket base: the bar vertical, the stop's underside a cone steeper than 45°".into(),
        post: stem_post,
    };
    // Three collar sizes, largest at the bottom.
    let size_of = |i: usize| fs * (1.0 - taper * i as f64 / (rings - 1).max(1) as f64);
    let classes = [
        size_of(0),
        (size_of(0) + size_of(rings - 1)) / 2.0,
        size_of(rings - 1),
    ];
    let class_of = |i: usize| {
        let s = size_of(i);
        (0..3)
            .min_by(|&a, &b| (classes[a] - s).abs().total_cmp(&(classes[b] - s).abs()))
            .unwrap_or(0)
    };
    let mut parts = vec![stem_part];
    let names = ["large", "medium", "small"];
    let mut class_part = [usize::MAX; 3];
    let mut steps = vec![(vec![(0, 0)], "the stem".to_string())];
    for i in 0..rings {
        let cl = class_of(i);
        if class_part[cl] == usize::MAX {
            let s = classes[cl];
            let mut k = Cmds::default();
            let ro = k.collar([0.0, 0.0], 0.0, dz - 0.1, 1.0);
            let zm = (dz - 0.1) / 2.0;
            for j in 0..per {
                let phi = 2.0 * PI * j as f64 / per as f64 + 1e-4 * j as f64;
                let (sp, cp) = phi.sin_cos();
                // Rooted deep in the collar (the bore, cut afterwards, trims what reaches into
                // it), so no floret's tilted foot shows below it.
                let from = [
                    (ro - 1.4) * cp,
                    (ro - 1.4) * sp,
                    zm - 0.5 + 0.002 * j as f64,
                ];
                let dirn = |e: f64| [e.cos() * cp, e.cos() * sp, e.sin()];
                // (Lengthening the florets by the depth they are rooted at made the mesh
                // non-manifold where neighbouring cones met; `floret_size` sets their size.)
                let deep = 0.0;
                match kind.as_str() {
                    "lupin" => k.cone(
                        from,
                        dirn(66f64.to_radians()),
                        s * 1.2 + deep,
                        (s * 0.34).min(1.4),
                        0.0,
                        false,
                    ),
                    "snapdragon" => k.cone(
                        from,
                        dirn(68f64.to_radians()),
                        s * 1.1 + deep,
                        0.7,
                        s * 0.42,
                        false,
                    ),
                    "larkspur" => k.cone(
                        from,
                        dirn(66f64.to_radians()),
                        s * 0.9 + deep,
                        0.7,
                        s * 0.48,
                        false,
                    ),
                    _ => k.cone(
                        from,
                        dirn(75f64.to_radians()),
                        s * 0.8 + deep,
                        s * 0.22,
                        0.0,
                        false,
                    ),
                }
            }
            k.collar_bore([0.0, 0.0], 0.0, dz - 0.1, false);
            class_part[cl] = parts.len();
            parts.push(PartDef {
                key: format!("florets_{}", names[cl]),
                label: format!("{} floret ring", names[cl]),
                role: "main".into(),
                cmds: k.c,
                base: None,
                connect: [0.0, 0.0, 0.0],
                copies: Vec::new(),
                orientation: "upright as it sits on the stem: the collar's crush ribs press in the bed plane; florets rise steeper than 45°".into(),
                post: Vec::new(),
            });
        }
        let pi = class_part[cl];
        let rot = if i % 2 == 1 { PI / per as f64 } else { 0.0 };
        let ci = parts[pi].copies.len();
        parts[pi]
            .copies
            .push(Iso::rz(rot, [at[0], at[1], zf + 1.0 + i as f64 * dz]));
        steps.push((
            vec![(pi, ci)],
            format!("slide on a {} floret ring", names[cl]),
        ));
    }
    Ok((
        KitDef {
            parts,
            steps,
            roles: vec![("main", "Medium Lavender"), ("stem", "Green")],
            sockets: base.clone(),
            mount: spike_mount,
        },
        json!({"kind": kind, "height": height, "rings": rings, "florets_per_ring": per, "taper": taper, "floret_size": fs, "stem_height": sh, "mount": spike_mount.is_some(), "base": dims(&base)}),
    ))
}

fn umbel(p: &Value) -> Result<(KitDef, Value)> {
    let rays = num(p, "rays", 8.0, 5.0, 10.0)?.round() as usize;
    let spread = num(p, "spread", 35.0, 15.0, 40.0)?;
    let len = num(p, "ray_length", 8.0, 4.0, 16.0)?;
    let cl = num(p, "cluster", 1.6, 1.0, 3.0)?;
    let sh = num(p, "stem_height", 20.0, 4.0, 40.0)?;
    let base = base_param(p, [2, 1])?;
    let at = base_centre(&base);
    let mut k = Cmds::based();
    let z_b = BASE_HEIGHT - SINK;
    let z_top = BASE_HEIGHT + sh;
    // On a stem: a foot round a bar hole, under the head's collar.
    let hm = crate::stem::wants_mount(p)
        .then(|| crate::stem::head_mount(at, &base, false))
        .filter(|m| m.top <= z_top - 3.1);
    if let Some(m) = &hm {
        k.cylinder(
            [at[0], at[1], z_b],
            [0.0, 0.0, 1.0],
            m.top - z_b,
            crate::stem::MOUNT_COLUMN_R,
            false,
        );
    }
    k.bar([at[0], at[1], z_b], z_top - z_b);
    let mut stem_post = Vec::new();
    let umbel_mount = hm.map(|m| {
        k.c.extend(m.cmds);
        stem_post = m.post;
        m.mount
    });
    let stem_part = PartDef {
        key: "stem".into(),
        label: "stem".into(),
        role: "stem".into(),
        cmds: k.c,
        base: Some(base.clone()),
        connect: [0.0, 0.0, 0.0],
        copies: vec![Iso::translate([0.0, 0.0, 0.0])],
        orientation: "upright on its socket base".into(),
        post: stem_post,
    };
    // The head: a blind collar, rays from its top ring leaning out, each ending in a cluster
    // that widens from the ray through a 45°+ cone.
    let mut k = Cmds::default();
    let ch = 4.0;
    let ro = k.collar([0.0, 0.0], 0.0, ch, 1.2);
    let r_start = ro - 0.8;
    for i in 0..rays {
        let phi = 2.0 * PI * i as f64 / rays as f64 + 1e-4 * i as f64;
        let l = (spread * if i % 2 == 0 { 1.0 } else { 0.6 }).to_radians();
        let axis = [l.sin() * phi.cos(), l.sin() * phi.sin(), l.cos()];
        let from = [r_start * phi.cos(), r_start * phi.sin(), ch - 0.3];
        let rl = len * if i % 2 == 0 { 1.0 } else { 0.8 };
        k.cylinder(from, axis, rl, 0.6, false);
        let end = [
            from[0] + axis[0] * rl,
            from[1] + axis[1] * rl,
            from[2] + axis[2] * rl,
        ];
        // From a thin start inside the ray's end, widening through a cone steeper than 45°.
        let rise = (cl - 0.3) * 1.15;
        k.cone(
            [end[0], end[1], end[2] - 0.6],
            [0.0, 0.0, 1.0],
            rise + 0.6,
            0.3,
            cl,
            false,
        );
        k.cylinder(
            [end[0], end[1], end[2] + rise - 0.01],
            [0.0, 0.0, 1.0],
            0.8,
            cl,
            false,
        );
    }
    k.collar_bore([0.0, 0.0], 0.0, ch, true);
    let head = PartDef {
        key: "head".into(),
        label: "flower head".into(),
        role: "main".into(),
        cmds: k.c,
        base: None,
        connect: [0.0, 0.0, 0.0],
        copies: vec![Iso::translate([at[0], at[1], z_top - (ch - 1.0)])],
        orientation: "upright as it sits on the stem: rays lean less than 45°, clusters widen through cones steeper than 45°, the collar's ribs press in the bed plane".into(),
        post: Vec::new(),
    };
    Ok((
        KitDef {
            parts: vec![stem_part, head],
            steps: vec![
                (vec![(0, 0)], "the stem".into()),
                (vec![(1, 0)], "push the head onto the stem".into()),
            ],
            roles: vec![("main", "White"), ("stem", "Green")],
            sockets: base.clone(),
            mount: umbel_mount,
        },
        json!({"rays": rays, "spread": spread, "ray_length": len, "cluster": cl, "stem_height": sh, "mount": umbel_mount.is_some(), "base": dims(&base)}),
    ))
}

/// A mount: the socket base with a vertical bar (for beak, ear, horn) or a post with a level
/// teardrop bar along `teardrop` (for fronds and leaves). Returns the part, the bar's foot
/// or axis point, and the top of the base.
fn mount(at: P2, base: &[(i32, i32)], teardrop: Option<[f64; 2]>) -> (PartDef, [f64; 3]) {
    let c = clip();
    let mut k = Cmds::based();
    let z_b = BASE_HEIGHT - SINK;
    let point = match teardrop {
        None => {
            k.bar([at[0], at[1], z_b], c.height + 1.2);
            [at[0], at[1], BASE_HEIGHT]
        }
        Some(dir) => {
            // A post up to the bar; the bar runs out of it along `dir`, the clip's place
            // starting POST_GAP from the post's middle.
            let zb = BASE_HEIGHT + 4.0;
            k.cylinder(
                [at[0], at[1], z_b],
                [0.0, 0.0, 1.0],
                zb - z_b + 0.5,
                2.0,
                false,
            );
            let l = POST_GAP + c.height + 0.8;
            k.teardrop_bar([at[0], at[1], zb], dir, l);
            [at[0], at[1], zb]
        }
    };
    let why = match teardrop {
        None => "upright on its socket base, the bar vertical",
        Some(_) => {
            "upright on its socket base; the level bar is a teardrop (45° V underneath), so it prints without support"
        }
    };
    (
        PartDef {
            key: "mount".into(),
            label: "mount".into(),
            role: "mount".into(),
            cmds: k.c,
            base: Some(base.to_vec()),
            connect: [0.0, 0.0, 0.0],
            copies: vec![Iso::translate([0.0, 0.0, 0.0])],
            orientation: why.into(),
            post: Vec::new(),
        },
        point,
    )
}

/// The rotation taking a flat print (in the bed plane, its clip's bar axis +z) to stand
/// upright with its plane facing `normal` (level) and its print +y pointing up.
fn stand_up(normal: [f64; 2]) -> [[f64; 3]; 3] {
    // Columns: print x → the level direction in the plane, print y → up, print z → normal
    // (along × up = normal, so it is a rotation, not a mirror).
    let along = [-normal[1], normal[0], 0.0];
    [
        [along[0], 0.0, normal[0]],
        [along[1], 0.0, normal[1]],
        [along[2], 1.0, 0.0],
    ]
}

/// A fern frond or a leaf: flat on the bed with a clip in its plane (mouth −y), on a mount's
/// teardrop bar.
#[allow(clippy::needless_late_init)] // each branch sets them after reading its own parameters
fn flat_on_mount(p: &Value, fern: bool) -> Result<(KitDef, Value)> {
    let (dname, dv) = dir_param(p, "+x", false)?;
    let base = base_param(p, [2, 1])?;
    let at = base_centre(&base);
    let c = clip();
    // The frond's plane faces across `direction`: the bar runs across it.
    let normal = [-dv[1], dv[0]];
    let (mount_part, bar_at) = mount(at, &base, Some(normal));
    let mut k = Cmds::default();
    let ne = k.clip([0.0, 0.0], 0.0, -PI / 2.0, &c, NECK);
    let y0 = ne[1] - 0.4;
    let full;
    let t;
    if fern {
        let length = num(p, "length", 36.0, 12.0, 48.0)?;
        let pairs = num(p, "pairs", 7.0, 3.0, 12.0)?.round() as usize;
        let curl = num(p, "curl", 50.0, 0.0, 120.0)?;
        let ll = num(p, "leaflet_length", 9.0, 3.0, 20.0)?;
        let lw = num(p, "leaflet_width", 3.0, 1.5, 6.0)?;
        let angle = num(p, "angle", 45.0, 25.0, 75.0)?;
        t = num(p, "thickness", 1.6, 1.2, 3.0)?;
        // The rib up +y from the neck, curling toward +x.
        const N: usize = 24;
        let kc = curl.to_radians();
        let ds = length / N as f64;
        let mut pts = vec![[0.0, y0]];
        let mut tang = vec![0.0f64];
        for i in 0..N {
            let s = (i as f64 + 0.5) * ds;
            let th = kc * (s / length).powi(2);
            let q = pts[i];
            pts.push([q[0] + ds * th.sin(), q[1] + ds * th.cos()]);
            tang.push(kc * ((i + 1) as f64 * ds / length).powi(2));
        }
        let rib_w = 2.0;
        let mut left = Vec::new();
        let mut right = Vec::new();
        for (i, q) in pts.iter().enumerate() {
            let th = tang[i];
            let w = rib_w / 2.0 * (1.0 - 0.4 * i as f64 / N as f64);
            let nl = [-th.cos(), th.sin()];
            left.push([q[0] + w * nl[0], q[1] + w * nl[1]]);
            right.push([q[0] - w * nl[0], q[1] - w * nl[1]]);
        }
        right.reverse();
        let outline: Vec<P2> = left.into_iter().chain(right).collect();
        k.prism(sketch_xy("", 0.0), vec![closed(&outline)], t, false, false);
        let a = angle.to_radians();
        for i in 0..pairs {
            let f = i as f64 / pairs as f64;
            let s = length * (0.15 + 0.78 * f);
            let idx = ((s / ds) as usize).min(N - 1);
            let (q, th) = (pts[idx], tang[idx]);
            let l = ll * (1.0 - 0.6 * f);
            let w = lw * (1.0 - 0.4 * f);
            for side in [-1.0, 1.0] {
                let dir = th + side * a;
                let (sd, cd) = dir.sin_cos();
                let root = [q[0] + 0.3 * sd, q[1] + 0.3 * cd];
                let tip = [root[0] + l * sd, root[1] + l * cd];
                let mid = [(root[0] + tip[0]) / 2.0, (root[1] + tip[1]) / 2.0];
                let nrm = [-cd, sd];
                k.prism(
                    sketch_xy("", 0.0),
                    vec![
                        arc3(
                            root,
                            [mid[0] + nrm[0] * w / 2.0, mid[1] + nrm[1] * w / 2.0],
                            tip,
                        ),
                        arc3(
                            tip,
                            [mid[0] - nrm[0] * w / 2.0, mid[1] - nrm[1] * w / 2.0],
                            root,
                        ),
                    ],
                    t * 0.8,
                    false,
                    false,
                );
            }
        }
        full = json!({"length": length, "pairs": pairs, "curl": curl, "leaflet_length": ll, "leaflet_width": lw, "angle": angle, "thickness": t, "direction": dname, "base": dims(&base)});
    } else {
        let length = num(p, "length", 18.0, 8.0, 40.0)?;
        let width = num(p, "width", 8.0, 3.0, 20.0)?;
        t = num(p, "thickness", 1.6, 1.2, 3.0)?;
        let tilt = num(p, "tilt", 0.0, -60.0, 60.0)?;
        // A stalk from the neck, then a pointed leaf with a midrib groove on top.
        let stalk = 2.0;
        k.prism(
            sketch_xy("", 0.0),
            vec![closed(&[
                [-0.9, y0],
                [0.9, y0],
                [0.9, y0 + stalk + 0.5],
                [-0.9, y0 + stalk + 0.5],
            ])],
            t,
            false,
            false,
        );
        let (u0, u1) = (y0 + stalk, y0 + stalk + length);
        let m = (u0 + u1) / 2.0;
        k.prism(
            sketch_xy("", 0.0),
            vec![
                arc3([0.0, u0], [width / 2.0, m], [0.0, u1]),
                arc3([0.0, u1], [-width / 2.0, m], [0.0, u0]),
            ],
            t,
            false,
            false,
        );
        if length > 8.0 {
            k.prism(
                sketch_xy("", t - 0.4),
                vec![closed(&[
                    [-0.3, u0 + 2.0],
                    [0.3, u0 + 2.0],
                    [0.3, u1 - 2.5],
                    [-0.3, u1 - 2.5],
                ])],
                1.0,
                false,
                true,
            );
        }
        full = json!({"length": length, "width": width, "thickness": t, "tilt": tilt, "direction": dname, "base": dims(&base)});
    }
    // Assembled: standing up, its plane facing `normal`, the clip round the bar (its C
    // spans print z 0..height, which becomes −normal..0 across the bar).
    let r = stand_up(normal);
    let tilt = full
        .get("tilt")
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
        .to_radians();
    // Lean round the bar (the clip's axis) by `tilt`.
    let lean = rot_about(normal, tilt);
    let r = matmul(&lean, &r);
    // The clip's C spans print z 0..height, which stands along +normal from the pose's origin.
    let pose = Iso {
        r,
        t: [
            bar_at[0] + normal[0] * POST_GAP,
            bar_at[1] + normal[1] * POST_GAP,
            bar_at[2],
        ],
    };
    let (key, label) = if fern {
        ("frond", "frond")
    } else {
        ("leaf", "leaf")
    };
    // On a stem: its clip goes round one of the stem's bar sections instead (the mount part
    // is left out). The clip's middle, along the bar, the leaf pointing out of it.
    let mv = |v: [f64; 3]| [0, 1, 2].map(|i| r[i][0] * v[0] + r[i][1] * v[1] + r[i][2] * v[2]);
    let clip_mount = crate::stem::MountMm {
        kind: sf_core::MountKind::Clip,
        origin: pose.apply([0.0, 0.0, c.height / 2.0]),
        axis: mv([0.0, 0.0, 1.0]),
        up: mv([0.0, 1.0, 0.0]),
    };
    let part = petal_def(
        key,
        label,
        "main",
        k.c,
        vec![pose],
        "flat on the bed with its clip's C in the same plane: no supports, the jaws bend within layers",
    );
    let _ = t;
    Ok((
        KitDef {
            parts: vec![mount_part, part],
            steps: vec![
                (vec![(0, 0)], "the mount".into()),
                (vec![(1, 0)], format!("clip the {label} onto the bar")),
            ],
            roles: vec![("main", "Green"), ("mount", "Green")],
            sockets: base.clone(),
            mount: Some(clip_mount),
        },
        full,
    ))
}

fn matmul(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut r = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            r[i][j] = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    r
}

/// Rotation by `a` about the level unit axis `n`.
fn rot_about(n: [f64; 2], a: f64) -> [[f64; 3]; 3] {
    let (s, c) = a.sin_cos();
    let (x, y, z) = (n[0], n[1], 0.0);
    let t = 1.0 - c;
    [
        [t * x * x + c, t * x * y - s * z, t * x * z + s * y],
        [t * x * y + s * z, t * y * y + c, t * y * z - s * x],
        [t * x * z - s * y, t * y * z + s * x, t * z * z + c],
    ]
}

fn stem(p: &Value) -> Result<(KitDef, Value)> {
    let plates = num(p, "height", 6.0, 1.0, 18.0)?.round();
    let angle = num(p, "angle", 0.0, 0.0, 30.0)?;
    let (dname, dv) = dir_param(p, "+x", false)?;
    let top = p
        .get("top")
        .and_then(Value::as_str)
        .unwrap_or("flat")
        .to_string();
    if !["flat", "round"].contains(&top.as_str()) {
        return Err(CustomError::InvalidShape("`top` is flat or round".into()));
    }
    let base = base_param(p, [2, 1])?;
    let at = base_centre(&base);
    let mut k = Cmds::based();
    let l = angle.to_radians();
    let axis = [l.sin() * dv[0], l.sin() * dv[1], l.cos()];
    let len = plates * 3.2 / l.cos() + SINK;
    let from = [at[0], at[1], BASE_HEIGHT - SINK];
    k.cylinder(from, axis, len, BAR_D / 2.0, false);
    if top == "round" && angle < 1e-6 {
        let zt = from[2] + len;
        k.revolve(
            at,
            0.0,
            cap_profile(BAR_D / 2.0 - 0.05, zt - 0.05, BAR_D / 2.0 - 0.15),
            360.0,
        );
    }
    let part = PartDef {
        key: "stem".into(),
        label: "stem".into(),
        role: "main".into(),
        cmds: k.c,
        base: Some(base.clone()),
        connect: [0.0, 0.0, 0.0],
        copies: vec![Iso::translate([0.0, 0.0, 0.0])],
        orientation: "upright on its socket base: a Ø3.18 bar (leaning at most 30°)".into(),
        post: Vec::new(),
    };
    Ok((
        KitDef {
            parts: vec![part],
            steps: vec![(vec![(0, 0)], "the stem".into())],
            roles: vec![("main", "Green")],
            sockets: base.clone(),
            mount: None,
        },
        json!({"height": plates, "angle": angle, "direction": dname, "top": top, "base": dims(&base)}),
    ))
}

fn eye(p: &Value) -> Result<(KitDef, Value)> {
    let d = num(p, "diameter", 7.0, 5.0, 14.0)?;
    let pr = num(p, "pupil", 0.45, 0.25, 0.7)?;
    let base = base_param(p, [2, 1])?;
    let at = base_centre(&base);
    let r = d / 2.0;
    let rp = (pr * r).max(crate::kit::PEG_D / 2.0 + 0.6);
    // The dome, flat on top where the pupil sits, with the pupil's hole down its middle.
    let h = (r * 0.9).max(PEG_LEN + 1.0);
    let zb = BASE_HEIGHT - SINK;
    let zt = zb + h;
    let rf = rp + 0.3;
    let mut k = Cmds::based();
    // Profile: base rim, a quarter-ellipse-ish arc up to the flat, the flat to the axis.
    let mid = [r * 0.82, zb + h * 0.62];
    k.revolve(
        at,
        0.0,
        vec![
            line([0.0, zb], [r, zb]),
            arc3([r, zb], mid, [rf, zt]),
            line([rf, zt], [0.0, zt]),
            line([0.0, zt], [0.0, zb]),
        ],
        360.0,
    );
    k.hole([at[0], at[1], zt], PEG_LEN + 0.5);
    let dome = PartDef {
        key: "eye".into(),
        label: "eye".into(),
        role: "main".into(),
        cmds: k.c,
        base: Some(base.clone()),
        connect: [0.0, 0.0, 0.0],
        copies: vec![Iso::translate([0.0, 0.0, 0.0])],
        orientation: "upright on its socket base; the pupil's hole vertical".into(),
        post: Vec::new(),
    };
    let (pupil, tip) = face_down_centre(rp, 1.0, 0.0, |_| {});
    let parts = vec![
        dome,
        petal_def(
            "pupil",
            "pupil",
            "pupil",
            pupil,
            vec![flipped_copy(at, zt, tip)],
            "face down, peg up: the peg's halves close in the bed plane",
        ),
    ];
    Ok((
        KitDef {
            parts,
            steps: vec![
                (vec![(0, 0)], "the eye".into()),
                (vec![(1, 0)], "press the pupil in".into()),
            ],
            roles: vec![("main", "White"), ("pupil", "Black")],
            sockets: base.clone(),
            mount: None,
        },
        json!({"diameter": d, "pupil": pr, "base": dims(&base)}),
    ))
}

/// A clipped part on a mount's vertical bar, turned to face `direction`.
fn on_vertical_bar(
    at: P2,
    base: &[(i32, i32)],
    dv: [f64; 2],
    part: (&str, &str, Vec<Value>, &str),
    roles: Vec<(&'static str, &'static str)>,
    full: Value,
) -> (KitDef, Value) {
    let (mount_part, foot) = mount(at, base, None);
    let a = dv[1].atan2(dv[0]);
    let (key, label, cmds, why) = part;
    let pose = Iso::rz(a, foot);
    (
        KitDef {
            parts: vec![
                mount_part,
                petal_def(key, label, "main", cmds, vec![pose], why),
            ],
            steps: vec![
                (vec![(0, 0)], "the mount".into()),
                (vec![(1, 0)], format!("clip the {label} onto the bar")),
            ],
            roles,
            sockets: base.to_vec(),
            mount: None,
        },
        full,
    )
}

fn beak(p: &Value) -> Result<(KitDef, Value)> {
    let len = num(p, "length", 9.0, 4.0, 20.0)?;
    let w = num(p, "width", 7.0, 3.0, 14.0)?;
    let h = num(p, "height", 3.0, 2.0, 6.0)?;
    let (dname, dv) = dir_param(p, "+x", false)?;
    let base = base_param(p, [2, 1])?;
    let at = base_centre(&base);
    let c = clip();
    let mut k = Cmds::default();
    // Mouth toward −x (snaps on from behind); the beak points +x.
    let ne = k.clip([0.0, 0.0], 0.0, PI, &c, NECK);
    let x0 = ne[0] - 0.4;
    // The beak: its outline tapering to a rounded tip, its sides drafted 30° in (no overhang).
    let tip = x0 + len;
    let outline = vec![
        line([x0, -w / 2.0], [tip - 1.0, -0.8]),
        arc3([tip - 1.0, -0.8], [tip, 0.0], [tip - 1.0, 0.8]),
        line([tip - 1.0, 0.8], [x0, w / 2.0]),
        line([x0, w / 2.0], [x0, -w / 2.0]),
    ];
    // Lofted from that outline on the bed to one drawn in by h·tan 30° at the top, so the
    // sides lean in 60° from level and nothing overhangs.
    let inset = h * 30f64.to_radians().tan();
    let top = vec![
        line(
            [x0 + inset * 0.3, -(w / 2.0 - inset)],
            [tip - 1.0 - inset, -0.5],
        ),
        arc3(
            [tip - 1.0 - inset, -0.5],
            [tip - inset * 0.9, 0.0],
            [tip - 1.0 - inset, 0.5],
        ),
        line(
            [tip - 1.0 - inset, 0.5],
            [x0 + inset * 0.3, w / 2.0 - inset],
        ),
        line(
            [x0 + inset * 0.3, w / 2.0 - inset],
            [x0 + inset * 0.3, -(w / 2.0 - inset)],
        ),
    ];
    let (a, b) = (k.name("BeakFoot"), k.name("BeakTop"));
    k.c.push(cmd(
        "sketch.create",
        json!({"plane": "XY", "offset": "0 mm", "name": a}),
    ));
    k.c.extend(outline);
    k.c.push(cmd("sketch.finish", json!({})));
    k.c.push(cmd(
        "sketch.create",
        json!({"plane": "XY", "offset": format!("{} mm", r6(h)), "name": b}),
    ));
    k.c.extend(top);
    k.c.push(cmd("sketch.finish", json!({})));
    k.c.push(cmd("solid.loft", json!({"sections": [{"sketch": a}, {"sketch": b}], "name": "Beak", "operation": "join", "targets": ["Part"]})));
    let full =
        json!({"length": len, "width": w, "height": h, "direction": dname, "base": dims(&base)});
    Ok(on_vertical_bar(
        at,
        &base,
        dv,
        (
            "beak",
            "beak",
            k.c,
            "flat on the bed, its clip's C in the same plane; the sides drafted 30° so nothing overhangs",
        ),
        vec![("main", "Orange"), ("mount", "")],
        full,
    ))
}

fn ear(p: &Value) -> Result<(KitDef, Value)> {
    let w = num(p, "width", 10.0, 5.0, 20.0)?;
    let hgt = num(p, "height", 12.0, 5.0, 28.0)?;
    let t = num(p, "thickness", 1.6, 1.2, 3.0)?;
    let (dname, dv) = dir_param(p, "+y", false)?;
    let base = base_param(p, [2, 1])?;
    let at = base_centre(&base);
    let c = clip();
    let mut k = Cmds::default();
    // Mouth toward −x; the ear plate stands behind the neck, its face toward +x.
    let ne = k.clip([0.0, 0.0], 0.0, PI, &c, NECK);
    let x = ne[0] + t / 2.0 - 0.3;
    let plate = sketch_on("", [x, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]);
    k.prism(
        plate,
        vec![closed(&[[-w / 2.0, 0.0], [w / 2.0, 0.0], [0.0, hgt]])],
        t,
        true,
        false,
    );
    let full =
        json!({"width": w, "height": hgt, "thickness": t, "direction": dname, "base": dims(&base)});
    Ok(on_vertical_bar(
        at,
        &base,
        dv,
        (
            "ear",
            "ear",
            k.c,
            "standing on its edge as it sits: a vertical plate narrowing upward, the clip's C in the bed plane",
        ),
        vec![("main", "Reddish Brown"), ("mount", "")],
        full,
    ))
}

fn horn(p: &Value) -> Result<(KitDef, Value)> {
    let len = num(p, "length", 12.0, 5.0, 30.0)?;
    let d = num(p, "diameter", 5.0, 3.0, 10.0)?;
    let lean = num(p, "lean", 0.0, 0.0, 30.0)?;
    let (dname, dv) = dir_param(p, "+x", false)?;
    let base = base_param(p, [2, 1])?;
    let at = base_centre(&base);
    let c = clip();
    let mut k = Cmds::default();
    let ne = k.clip([0.0, 0.0], 0.0, PI, &c, NECK);
    let r = d / 2.0;
    let l = lean.to_radians();
    // Its base circle tilts with the lean: lift it so its low edge is on the bed (a leaning
    // horn's tilted underside overhangs a little and is flagged; upright it needs nothing).
    let from = [ne[0] + r - 0.6, 0.0, r * l.sin() + 0.01];
    k.cone(from, [l.sin(), 0.0, l.cos()], len, r, 0.5, false);
    let full = json!({"length": len, "diameter": d, "lean": lean, "direction": dname, "base": dims(&base)});
    Ok(on_vertical_bar(
        at,
        &base,
        dv,
        (
            "horn",
            "horn",
            k.c,
            "standing on its base as it sits (leaning at most 30°), the clip's C in the bed plane",
        ),
        vec![("main", "White"), ("mount", "")],
        full,
    ))
}
