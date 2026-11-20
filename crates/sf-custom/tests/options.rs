//! The two optional underside/outline features: thin wall ribs and rounded vertical edges.
//!
//! Each is checked by building the part with and without it through SolidCraft and comparing
//! the measured change in volume with a hand calculation made here (closed forms, or a
//! brute-force sum where the roof is curved), independent of the crate's own formulas. The
//! build itself also checks every stage's change against the plan.

use sf_custom::{CustomError, CustomPart, CustomShape, StudProfile, build, plan};
use std::f64::consts::{PI, SQRT_2};

/// Rib protrusion: 4 − 0.1 − 1.2 − 2.4 = 0.3 mm to touch a Ø4.8 stud, plus 0.05 interference.
const T: f64 = 0.35;
/// Rib width.
const RW: f64 = 0.6;
/// Rib lead-in chamfer (0.2 mm, under ⅔ of 0.35).
const LEAD: f64 = 0.2;
/// Bottom chamfer.
const C: f64 = 0.2;
/// Demo edge radius.
const R: f64 = 0.5;

fn ribbed(p: StudProfile) -> StudProfile {
    StudProfile {
        wall_ribs: true,
        ..p
    }
}

fn rounded(p: StudProfile) -> StudProfile {
    StudProfile {
        edge_radius: R,
        ..p
    }
}

fn both(p: StudProfile) -> StudProfile {
    ribbed(rounded(p))
}

fn vol(name: &str, shape: &CustomShape, p: &StudProfile) -> CustomPart {
    build(shape, p).unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// The measured change from `base` to `with` must be `hand` (to 0.02 mm³: SolidCraft's own
/// volumes of curved faces are good to a few µm³, sketch coordinates are rounded to 1 µm).
fn check_delta(name: &str, base: &CustomPart, with: &CustomPart, hand: f64) {
    let got = with.volume_mm3 - base.volume_mm3;
    println!(
        "{name}: {:.3} → {:.3} mm³, change {got:+.4} mm³, hand {hand:+.4} mm³",
        base.volume_mm3, with.volume_mm3
    );
    assert!(
        (got - hand).abs() < 0.02,
        "{name}: measured change {got} vs hand {hand}"
    );
}

/// One rib under a flat ceiling `hollow` mm up: its protruding 0.35 × 0.6 section, full
/// height (the bit above the ceiling is inside the top), less its 0.2 mm lead-in.
fn rib(hollow: f64) -> f64 {
    T * RW * hollow - LEAD * LEAD / 2.0 * RW
}

/// What rounding the vertical edges of a rectilinear outline (`convex`, `concave` corners)
/// of height `h` changes: the corner slivers `(1 − π/4)·r²·h`, and the bottom chamfer, which
/// runs a shorter path (`(2 − π/2)·r` less per corner) and turns as an arc (`π/4` per corner
/// instead of 1 in `perimeter·c²/2 − turns·c³/3`).
fn rounding(convex: f64, concave: f64, h: f64) -> f64 {
    let sliver = (1.0 - PI / 4.0) * R * R;
    let corners = convex + concave;
    -(convex - concave) * sliver * h + corners * (2.0 - PI / 2.0) * R * C * C / 2.0
        - (1.0 - PI / 4.0) * (convex - concave) * C * C * C / 3.0
}

fn box2x4() -> CustomShape {
    CustomShape::Box {
        w: 2,
        d: 4,
        plates: 3,
    }
}

#[test]
fn brick_2x4_options_a_to_d() {
    let std = StudProfile::standard_fdm();
    let s = box2x4();
    let a = vol("a", &s, &std);
    let b = vol("b", &s, &ribbed(std));
    let c = vol("c", &s, &rounded(std));
    let d = vol("d", &s, &both(std));
    // 4 ribs along each long wall, 2 along each end wall, under an 8.6 mm hollow.
    let ribs = 12.0 * rib(8.6);
    let round = rounding(4.0, 0.0, 9.6);
    check_delta("2x4 (b) ribs", &a, &b, ribs);
    check_delta("2x4 (c) r 0.5", &a, &c, round);
    check_delta("2x4 (d) both", &a, &d, ribs + round);
    println!(
        "volumes: (a) {:.3}  (b) {:.3}  (c) {:.3}  (d) {:.3} mm³",
        a.volume_mm3, b.volume_mm3, c.volume_mm3, d.volume_mm3
    );
    // The stages and their order.
    let names: Vec<_> = d.stage_volumes_mm3.iter().map(|s| s.0.as_str()).collect();
    assert_eq!(
        names,
        [
            "Block",
            "Hollow",
            "Fillet",
            "Tubes",
            "Ribs",
            "Chamfer",
            "WallRibs",
            "RibLeadIns",
            "Studs"
        ]
    );
    // Same outside box: the rounds don't change the extents.
    for k in 0..3 {
        assert!((a.bbox_mm.0[k] - d.bbox_mm.0[k]).abs() < 1e-6);
        assert!((a.bbox_mm.1[k] - d.bbox_mm.1[k]).abs() < 1e-6);
    }
}

#[test]
fn ribs_sit_on_the_stud_centres() {
    // Every rib of a 2×4 is centred on a stud's row or column, 0.35 mm proud of the wall.
    let p = ribbed(StudProfile::standard_fdm());
    let plan = plan(&box2x4(), &p).unwrap();
    let stage = plan.stages.iter().find(|s| s.name == "WallRibs").unwrap();
    let mut centres = Vec::new();
    for c in &stage.commands {
        if c["command"] == "sketch.rectangle.two_point" {
            let q = |k: &str, i: usize| c["params"][k][i].as_f64().unwrap();
            let lo = [q("p0", 0), q("p0", 1)];
            let hi = [q("p1", 0), q("p1", 1)];
            centres.push([(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0]);
            let (wx, wy) = (hi[0] - lo[0], hi[1] - lo[1]);
            // 0.6 along the wall; from half-way into the 1.2 wall to 0.35 past its face.
            let (along, across) = if wx < wy { (wx, wy) } else { (wy, wx) };
            assert!((along - RW).abs() < 1e-6 && (across - (0.6 + T)).abs() < 1e-6);
        }
    }
    assert_eq!(centres.len(), 12);
    let long: Vec<_> = centres
        .iter()
        .filter(|c| c[0] < 2.0 || c[0] > 14.0)
        .collect();
    let ends: Vec<_> = centres
        .iter()
        .filter(|c| c[1] < 2.0 || c[1] > 30.0)
        .collect();
    assert_eq!((long.len(), ends.len()), (8, 4));
    for c in long {
        assert!(((c[1] - 4.0) / 8.0).fract().abs() < 1e-9, "{c:?}");
    }
    for c in ends {
        assert!(c[0] == 4.0 || c[0] == 12.0, "{c:?}");
    }
    // The derived clearances (see GUIDE): to the nearest tube and to a centre rib.
    let face = 0.1 + 1.2 + T;
    let tube = (8.0 - face).hypot(4.0 - RW / 2.0) - 6.51 / 2.0;
    let centre_rib = 4.0 - RW / 2.0 - 1.0 / 2.0;
    println!("rib → tube {tube:.3} mm, rib → centre rib {centre_rib:.3} mm");
    assert!(tube > 4.0 && centre_rib > 3.0);
}

#[test]
fn v2_2x4_with_both_options() {
    let v2 = StudProfile::slant_v2_experimental();
    let s = box2x4();
    let a = vol("v2", &s, &v2);
    let d = vol("v2 both", &s, &both(v2));
    // The v2 top is 1.4 mm thick: an 8.2 mm hollow.
    check_delta(
        "v2 2x4 both",
        &a,
        &d,
        12.0 * rib(8.2) + rounding(4.0, 0.0, 9.6),
    );
}

#[test]
fn slopes_take_both_options() {
    let std = StudProfile::standard_fdm();
    // 45° 2×2: lip 1.7, run 7.9, ceiling y − 0.1 + 1.7 − 1.2√2 under the roof.
    let s = CustomShape::Slope45 {
        w: 2,
        d: 2,
        plates: 3,
    };
    let a = vol("slope", &s, &std);
    let b = vol("slope ribs", &s, &ribbed(std));
    let c = vol("slope r", &s, &rounded(std));
    let ceil = |y: f64| y - 0.1 + 1.7 - 1.2 * SQRT_2;
    // Side walls, row 0: under the roof (mean ceiling at y = 4). Row 1 and the back wall:
    // flat. The front wall's ceiling (1.2–1.55 mm) is lower than a stud: no ribs there.
    let ribs = 2.0 * (T * RW * ceil(4.0) - LEAD * LEAD / 2.0 * RW) + 4.0 * rib(8.6);
    check_delta("slope45 2x2 ribs", &a, &b, ribs);
    // The roof cut no longer takes the slivers of the two front rounds: over each,
    // ∬(h − roof) = run·(1 − π/4)·r² − r³·(5/6 − π/4).
    let front = 2.0 * (7.9 * (1.0 - PI / 4.0) * R * R - R.powi(3) * (5.0 / 6.0 - PI / 4.0));
    check_delta("slope45 2x2 r 0.5", &a, &c, rounding(4.0, 0.0, 9.6) + front);

    // Curved 2×3: lip 1.0, run 8.6, roof z = 1 + √(8.6² − (y − 8.7)²) in front of the ridge.
    let s = CustomShape::Curve {
        w: 2,
        d: 3,
        plates: 3,
    };
    let a = vol("curve", &s, &std);
    let d = vol("curve both", &s, &both(std));
    let roof = |y: f64| 1.0 + (8.6f64 * 8.6 - (y - 8.7) * (y - 8.7)).max(0.0).sqrt();
    // Brute force over the sliver between the square corner (0.1, 0.1) and the round.
    let step = 0.0004;
    let n = (R / step) as i32;
    let mut sliver = 0.0;
    for i in 0..n {
        for j in 0..n {
            let (x, y) = (
                0.1 + (f64::from(i) + 0.5) * step,
                0.1 + (f64::from(j) + 0.5) * step,
            );
            if (x - 0.6).hypot(y - 0.6) > R && x < 0.6 && y < 0.6 {
                sliver += (9.6 - roof(y)) * step * step;
            }
        }
    }
    // Ribs: the hollow's ceiling in front of the ridge is 1 + √(7.4² − (y − 8.7)²); row 0's
    // side ribs (y = 3.7…4.3) sit under it; the front wall's (ceiling 1.0–3.3 mm, lower than
    // a stud) are left out.
    let arc = |y: f64| (1.0 + (7.4f64 * 7.4 - (y - 8.7) * (y - 8.7)).sqrt()).min(8.6);
    let k = 600;
    let mean = (0..k)
        .map(|i| arc(3.7 + RW * (f64::from(i) + 0.5) / f64::from(k)))
        .sum::<f64>()
        / f64::from(k);
    // Side ribs: row 0 under the arc; rows 1 and 2 behind the ridge (y ≥ 8.7) under the flat
    // ceiling, as are the 2 on the back wall.
    let ribs = 2.0 * (T * RW * mean - LEAD * LEAD / 2.0 * RW) + 6.0 * rib(8.6);
    check_delta(
        "curve 2x3 both",
        &a,
        &d,
        ribs + rounding(4.0, 0.0, 9.6) + 2.0 * sliver,
    );
}

#[test]
fn footprints_round_concave_corners_too() {
    // The L of 7 cells: 5 convex corners and 1 concave; 12 cell edges on the outline.
    let std = StudProfile::standard_fdm();
    let s = CustomShape::Footprint {
        cells: vec![(0, 0), (1, 0), (2, 0), (0, 1), (1, 1), (0, 2), (1, 2)],
        plates: 3,
    };
    let a = vol("L", &s, &std);
    let d = vol("L both", &s, &both(std));
    check_delta(
        "footprint L both",
        &a,
        &d,
        12.0 * rib(8.6) + rounding(5.0, 1.0, 9.6),
    );
    // The concave corner alone adds material: a 3-cell L has 5 convex, 1 concave.
    let s = CustomShape::Footprint {
        cells: vec![(0, 0), (1, 0), (0, 1)],
        plates: 1,
    };
    let a = vol("L3", &s, &std);
    let c = vol("L3 r", &s, &rounded(std));
    check_delta("footprint L3 r 0.5", &a, &c, rounding(5.0, 1.0, 3.2));
}

#[test]
fn plates_and_1x1_take_both_options() {
    let std = StudProfile::standard_fdm();
    for (w, d, ribs) in [(1u32, 1u32, 4.0), (1, 4, 10.0), (2, 2, 8.0)] {
        let s = CustomShape::Box { w, d, plates: 1 };
        let a = vol("plate", &s, &std);
        let both = vol("plate both", &s, &both(std));
        check_delta(
            &format!("plate {w}x{d} both"),
            &a,
            &both,
            ribs * rib(2.2) + rounding(4.0, 0.0, 3.2),
        );
    }
}

#[test]
fn profiles_with_bad_options_are_refused() {
    let std = StudProfile::standard_fdm();
    let s = box2x4();
    for (p, why) in [
        (
            StudProfile {
                edge_radius: 1.5,
                ..std
            },
            "more than the wall",
        ),
        (
            StudProfile {
                edge_radius: 0.15,
                ..std
            },
            "bottom chamfer",
        ),
        (
            StudProfile {
                edge_radius: -0.1,
                ..std
            },
            "edge_radius",
        ),
        (
            StudProfile {
                wall_rib_interference: -0.1,
                ..ribbed(std)
            },
            "wall_rib_interference",
        ),
        (
            StudProfile {
                wall_rib_interference: 2.0,
                ..ribbed(std)
            },
            "proud",
        ),
    ] {
        match plan(&s, &p) {
            Err(CustomError::InvalidProfile(m)) => assert!(m.contains(why), "{m}"),
            other => panic!("expected an invalid profile ({why}), got {other:?}"),
        }
    }
    // Off by default, and old JSON profiles (without the fields) read as off.
    assert!(!std.wall_ribs && std.edge_radius == 0.0);
    let mut v = serde_json::to_value(std).unwrap();
    let o = v.as_object_mut().unwrap();
    o.remove("wall_ribs");
    o.remove("wall_rib_interference");
    o.remove("edge_radius");
    let back: StudProfile = serde_json::from_value(v).unwrap();
    assert_eq!(back, std);
}
