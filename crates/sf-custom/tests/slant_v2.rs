//! Builds Slant 3D's v2 stud through SolidCraft and checks it against hand calculations made
//! here, independently of the crate's own geometry: the peanut in closed form, the slot and
//! pocket areas by sampling a fine grid with inside tests written from the stud's dimensions.
//!
//! STLs are written to `/tmp/fushion-custom/` for inspection.

use serde_json::json;
use sf_custom::{
    CustomPart, CustomShape, StudProfile, analyze_stl, build, build_from_commands, commands,
    slant_dims,
};
use std::f64::consts::PI;

const OUT: &str = "/tmp/fushion-custom";

/// The v2 lobe geometry, written out from the documented construction.
struct Hand {
    r: f64,
    a: f64,
    big_r: f64,
    c: f64,
    tv: f64,
    uc: f64,
    hw: f64,
    v0: f64,
    vs: f64,
    hc: [f64; 2],
    rh: f64,
    tail: f64,
    pocket_out: f64,
}

const S45: f64 = std::f64::consts::FRAC_1_SQRT_2;

impl Hand {
    fn new() -> Self {
        let d = slant_dims(&StudProfile::slant_v2_experimental()).unwrap();
        let (r, a, w) = (2.64, 4.0, 1.6);
        assert!((d.lobe_diameter - 2.0 * r).abs() < 1e-12);
        // Concave sides of radius R about (±c, 0) tangent to both lobes.
        let big_r = (w * w + a * a - r * r) / (2.0 * (r - w));
        assert!((d.waist_radius - big_r).abs() < 1e-9);
        let c = big_r + w;
        let tv = a * big_r / (big_r + r);
        // Slot: 0.5 wide, inner edge 0.25 outboard; root 0.8; hook radius 1.2 whose outer edge
        // peaks 0.75 below the lobe's far edge.
        let (uc, hw, rh) = (0.5, 0.25, 1.2);
        let vs = r - 0.75 - hw - rh;
        assert!((d.slot_center - uc).abs() < 1e-12);
        assert!((d.slot_v.0 + 3.2).abs() < 1e-12 && (d.slot_v.1 - vs).abs() < 1e-12);
        Self {
            r,
            a,
            big_r,
            c,
            tv,
            uc,
            hw,
            v0: -3.2,
            vs,
            hc: [uc + rh, vs],
            rh,
            tail: d.tail,
            pocket_out: d.pocket_out,
        }
    }

    /// Inside the peanut (lobe B's frame: origin at the lobe centre, `u` to the fin side).
    fn peanut(&self, q: [f64; 2]) -> bool {
        let (u, v) = (q[0], q[1] + self.a); // peanut frame
        let disk = |cv: f64| u * u + (v - cv) * (v - cv) <= self.r * self.r;
        disk(self.a)
            || disk(-self.a)
            || (v.abs() <= self.tv && u.abs() <= self.c - (self.big_r.powi(2) - v * v).sqrt())
    }

    /// Angle of `q` about the hook centre, degrees.
    fn hook_angle(&self, q: [f64; 2]) -> f64 {
        (q[1] - self.hc[1]).atan2(q[0] - self.hc[0]).to_degrees()
    }

    fn m_end(&self) -> [f64; 2] {
        [self.hc[0] + self.rh * S45, self.hc[1] + self.rh * S45]
    }

    /// Inside the slot: within 0.25 mm of its centre line (straight, 135° hook, tail).
    fn slot(&self, q: [f64; 2]) -> bool {
        let straight = (q[0] - self.uc).abs() <= self.hw && q[1] >= self.v0 && q[1] <= self.vs;
        let t = self.hook_angle(q);
        let d = (q[0] - self.hc[0]).hypot(q[1] - self.hc[1]);
        let hook = q[1] >= self.vs && (45.0..=180.0).contains(&t) && (d - self.rh).abs() <= self.hw;
        let m = self.m_end();
        let (dx, dy) = (q[0] - m[0], q[1] - m[1]);
        let along = dx * S45 - dy * S45;
        let across = dx * S45 + dy * S45;
        let tail = (0.0..=self.tail).contains(&along) && across.abs() <= self.hw;
        straight || hook || tail
    }

    /// Inside the pocket: on the fin side of the slot's centre line, from 0.2 mm before the
    /// slot's end to the hook, out to `pocket_out`.
    fn pocket(&self, q: [f64; 2]) -> bool {
        if q[0] < self.uc || q[0] > self.pocket_out || q[1] < self.v0 - 0.2 {
            return false;
        }
        if q[1] <= self.vs {
            return true;
        }
        let t = self.hook_angle(q);
        if t >= 45.0 {
            return (q[0] - self.hc[0]).hypot(q[1] - self.hc[1]) <= self.rh;
        }
        let m = self.m_end();
        let e2 = [
            m[0] + (self.tail + 0.2) * S45,
            m[1] - (self.tail + 0.2) * S45,
        ];
        // Below the tail's centre line (the hook centre's side), or below its end.
        let across = (q[0] - m[0]) * S45 + (q[1] - m[1]) * S45;
        across <= 0.0 || q[1] <= e2[1]
    }

    /// Peanut area, closed form: two disks, plus the waist band between the concave arcs, less
    /// the disks' caps inside the band.
    fn peanut_area(&self) -> f64 {
        let (r, br, tv) = (self.r, self.big_r, self.tv);
        let band = 4.0 * self.c * tv
            - 2.0 * (tv * (br * br - tv * tv).sqrt() + br * br * (tv / br).asin());
        let d = self.a - tv;
        let cap = r * r * (d / r).acos() - d * (r * r - d * d).sqrt();
        2.0 * PI * r * r + band - 2.0 * cap
    }
}

/// Areas by counting the centres of 2 µm cells: (slot ∩ peanut, slot ∖ pocket, pocket).
fn sampled_areas(h: &Hand) -> (f64, f64, f64) {
    let step = 0.002;
    let (lo, hi) = ([-0.5, -4.0], [3.6, 2.8]);
    let nx = ((hi[0] - lo[0]) / step) as i32;
    let ny = ((hi[1] - lo[1]) / step) as i32;
    let (mut sp, mut so, mut po) = (0u64, 0u64, 0u64);
    for i in 0..nx {
        for j in 0..ny {
            let q = [
                lo[0] + (f64::from(i) + 0.5) * step,
                lo[1] + (f64::from(j) + 0.5) * step,
            ];
            let pocket = h.pocket(q);
            if pocket {
                po += 1;
            }
            if h.slot(q) {
                if h.peanut(q) {
                    sp += 1;
                }
                if !pocket {
                    so += 1;
                }
            }
        }
    }
    let cell = step * step;
    (sp as f64 * cell, so as f64 * cell, po as f64 * cell)
}

fn tube(hgt: f64) -> f64 {
    PI * (3.255f64.powi(2) - 2.4f64.powi(2)) * hgt
}

fn chamfer_rect(l: f64, d: f64) -> f64 {
    let c: f64 = 0.2;
    c * c * (l + d) - 4.0 * c.powi(3) / 3.0
}

fn chamfer_tube() -> f64 {
    let c: f64 = 0.2;
    PI * c * c * (3.255 + 2.4)
}

fn rib(hollow: f64) -> f64 {
    let (ro, mid): (f64, f64) = (3.255, 2.8275);
    let chord = 0.5 * (ro * ro - 0.25f64).sqrt() + ro * ro * (0.5 / ro).asin();
    2.0 * ((8.0 - mid - 1.3) - (chord - mid)) * (hollow - 2.2)
}

/// The stage deltas of a built part, by name.
fn deltas(part: &CustomPart) -> Vec<(String, f64)> {
    let mut before = 0.0;
    part.stage_volumes_mm3
        .iter()
        .map(|(n, v)| {
            let d = v - before;
            before = *v;
            (n.clone(), d)
        })
        .collect()
}

fn delta(part: &CustomPart, name: &str) -> f64 {
    deltas(part)
        .into_iter()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("no stage {name}"))
        .1
}

fn close(name: &str, got: f64, want: f64, rel: f64) {
    let e = (got - want) / want;
    println!(
        "  {name}: measured {got:.4} mm³, hand {want:.4} mm³ ({:+.4} %)",
        e * 100.0
    );
    assert!(e.abs() < rel, "{name}: {got} vs {want}");
}

/// Builds a v2 `w × d` brick and checks it stage by stage. `peanuts` and `round` are how
/// many of each the brick should get.
fn check_v2(w: u32, d: u32, peanuts: usize, round: usize, ribs: usize) -> CustomPart {
    let h = Hand::new();
    let (slot_peanut, slot_out, pocket) = sampled_areas(&h);
    let shape = CustomShape::Box { w, d, plates: 3 };
    let part = build(&shape, &StudProfile::slant_v2_experimental())
        .unwrap_or_else(|e| panic!("v2 {w}x{d}: {e}"));
    let name = format!("v2_box_{w}x{d}x3");
    println!("{name}: {:?}", part.stage_volumes_mm3);
    let lobes = 2.0 * peanuts as f64;
    let (l, dd) = (f64::from(w) * 8.0 - 0.2, f64::from(d) * 8.0 - 0.2);
    let tubes = f64::from((w - 1) * (d - 1));
    // Top 1.4 mm: the hollow is 8.2 mm tall.
    let shell = l * dd * 9.6 - (l - 2.4) * (dd - 2.4) * 8.2;
    let pockets = pocket * 0.6 * lobes;
    let studs = (h.peanut_area() * peanuts as f64 + PI * 2.4 * 2.4 * round as f64) * 1.8;
    let slots = (slot_peanut * 1.8 + slot_out * 0.3) * lobes;
    let chamfer = chamfer_rect(l, dd) + tubes * chamfer_tube();
    let hand =
        shell + tubes * tube(8.2) + ribs as f64 * rib(8.2) - chamfer - pockets + studs - slots;
    close("Studs", delta(&part, "Studs"), studs, 1e-6);
    close("Pockets", -delta(&part, "Pockets"), pockets, 2e-3);
    close("Slots", -delta(&part, "Slots"), slots, 3e-3);
    close("Chamfer", -delta(&part, "Chamfer"), chamfer, 1e-6);
    if ribs > 0 {
        close("Ribs", delta(&part, "Ribs"), ribs as f64 * rib(8.2), 1e-6);
    }
    close("total", part.volume_mm3, hand, 1e-4);
    let mesh = analyze_stl(&part.stl).unwrap();
    assert!(mesh.is_watertight(), "{name}: {mesh:?}");
    for k in 0..3 {
        let (lo, hi) = ([0.1, 0.1, 0.0], [l + 0.1, dd + 0.1, 11.4]);
        assert!(
            (part.bbox_mm.0[k] - lo[k]).abs() < 1e-3,
            "{:?}",
            part.bbox_mm
        );
        assert!(
            (part.bbox_mm.1[k] - hi[k]).abs() < 1e-3,
            "{:?}",
            part.bbox_mm
        );
        assert!(
            (mesh.bbox_mm.1[k] - hi[k]).abs() < 1e-3,
            "{:?}",
            mesh.bbox_mm
        );
    }
    std::fs::create_dir_all(OUT).unwrap();
    std::fs::write(format!("{OUT}/{name}.stl"), &part.stl).unwrap();
    part
}

#[test]
fn v2_2x4_two_peanut_rows_with_centre_rib() {
    check_v2(2, 4, 4, 0, 1);
}

#[test]
fn v2_2x2_one_peanut_per_row() {
    check_v2(2, 2, 2, 0, 0);
}

#[test]
fn v2_2x3_leaves_the_odd_studs_round() {
    check_v2(2, 3, 2, 2, 1);
}

#[test]
fn v2_fins_hang_only_from_their_roots() {
    // Cut the root of every fin of a 2×2 (a box over the waist on the fin side, from the
    // pocket floor up): each fin then falls away as its own body.
    let mut batch = commands(
        &CustomShape::Box {
            w: 2,
            d: 2,
            plates: 3,
        },
        &StudProfile::slant_v2_experimental(),
    )
    .unwrap();
    let steps = batch["commands"].as_array_mut().unwrap();
    // Row 0's fins face −X from x = 4 − u, row 1's face +X from x = 12 + u; the waist is at
    // y = 8 and the slots end 0.8 mm either side of it.
    for (x0, x1) in [(0.5, 3.5), (12.5, 15.5)] {
        steps.push(json!({"command": "solid.box", "params": {
            "length": x1 - x0, "width": 2.0, "height": 3.0,
            "corner": [x0, 7.0, 8.9], "operation": "cut"}}));
    }
    batch["expect"] = json!({"bodies": 5});
    let part = build_from_commands(&batch).unwrap_or_else(|e| panic!("{e}"));
    assert!(analyze_stl(&part.stl).unwrap().is_watertight());
    // The same boxes on the inner side of each peanut cut nothing free.
    let mut batch = commands(
        &CustomShape::Box {
            w: 2,
            d: 2,
            plates: 3,
        },
        &StudProfile::slant_v2_experimental(),
    )
    .unwrap();
    let steps = batch["commands"].as_array_mut().unwrap();
    for (x0, x1) in [(4.25, 7.0), (9.0, 11.75)] {
        steps.push(json!({"command": "solid.box", "params": {
            "length": x1 - x0, "width": 2.0, "height": 3.0,
            "corner": [x0, 7.0, 8.9], "operation": "cut"}}));
    }
    batch["expect"] = json!({"bodies": 1});
    build_from_commands(&batch).unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn standard_bricks_get_a_rib_every_second_tube() {
    for (d, ribs) in [(2u32, 0usize), (3, 1), (4, 1), (6, 2), (8, 3)] {
        let shape = CustomShape::Box { w: 2, d, plates: 3 };
        let plan = sf_custom::plan(&shape, &StudProfile::standard_fdm()).unwrap();
        let rib_stage = plan.stages.iter().find(|s| s.name == "Ribs");
        let bars = rib_stage.map_or(0, |s| {
            s.commands
                .iter()
                .filter(|c| c["command"] == "sketch.rectangle.two_point")
                .count()
        });
        assert_eq!(bars, 2 * ribs, "2x{d}");
    }
    // Rotated (long side along X) and switched off.
    let plan = sf_custom::plan(
        &CustomShape::Box {
            w: 6,
            d: 2,
            plates: 3,
        },
        &StudProfile::standard_fdm(),
    )
    .unwrap();
    assert!(plan.stages.iter().any(|s| s.name == "Ribs"));
    let off = StudProfile {
        center_ribs: false,
        ..StudProfile::standard_fdm()
    };
    let plan = sf_custom::plan(
        &CustomShape::Box {
            w: 2,
            d: 6,
            plates: 3,
        },
        &off,
    )
    .unwrap();
    assert!(!plan.stages.iter().any(|s| s.name == "Ribs"));
    // A plate is too low for a rib.
    let plan = sf_custom::plan(
        &CustomShape::Box {
            w: 2,
            d: 6,
            plates: 1,
        },
        &StudProfile::standard_fdm(),
    )
    .unwrap();
    assert!(!plan.stages.iter().any(|s| s.name == "Ribs"));
    // A 6×2 builds (ribs along Y).
    let part = build(
        &CustomShape::Box {
            w: 6,
            d: 2,
            plates: 3,
        },
        &StudProfile::standard_fdm(),
    )
    .unwrap();
    assert!(analyze_stl(&part.stl).unwrap().is_watertight());
}

#[test]
fn v2_on_slopes_footprints_and_single_rows() {
    let v2 = StudProfile::slant_v2_experimental();
    for shape in [
        CustomShape::Box {
            w: 1,
            d: 4,
            plates: 3,
        },
        CustomShape::Box {
            w: 4,
            d: 1,
            plates: 3,
        },
        CustomShape::Slope45 {
            w: 2,
            d: 3,
            plates: 3,
        },
        CustomShape::Footprint {
            cells: vec![(0, 0), (1, 0), (2, 0), (0, 1), (1, 1), (0, 2), (1, 2)],
            plates: 3,
        },
        CustomShape::Box {
            w: 2,
            d: 2,
            plates: 1,
        },
    ] {
        let part = build(&shape, &v2).unwrap_or_else(|e| panic!("{}: {e}", shape.label()));
        assert!(
            analyze_stl(&part.stl).unwrap().is_watertight(),
            "{}",
            shape.label()
        );
    }
}

#[test]
#[allow(deprecated)]
fn old_name_is_the_same_profile() {
    assert_eq!(
        StudProfile::compliant_v2_experimental(),
        StudProfile::slant_v2_experimental()
    );
    // A pocket needs 0.8 mm of floor: the standard 1.0 mm top is too thin for v2 fins.
    let thin = StudProfile {
        top_thickness: 1.0,
        ..StudProfile::slant_v2_experimental()
    };
    assert!(
        build(
            &CustomShape::Box {
                w: 2,
                d: 2,
                plates: 3
            },
            &thin
        )
        .is_err()
    );
}
