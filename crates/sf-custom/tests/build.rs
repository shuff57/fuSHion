//! Builds every shape through SolidCraft and checks it against hand calculations.
//!
//! STLs are written to `/tmp/studforge-custom/` for inspection.

use serde_json::json;
use sf_custom::{
    CustomError, CustomPart, CustomShape, StudProfile, analyze_stl, build, build_from_commands,
    commands,
};
use std::f64::consts::PI;

const OUT: &str = "/tmp/studforge-custom";

fn stud() -> f64 {
    PI * 2.4 * 2.4 * 1.8
}

/// One Ø6.51/Ø4.8 tube of height `h`.
fn tube(h: f64) -> f64 {
    PI * (3.255f64.powi(2) - 2.4f64.powi(2)) * h
}

/// A tube centred at `y0` cut off by a ceiling `ceil(y)`, by brute-force summation on a
/// 0.005 mm grid (independent of the crate's own quadrature).
fn tube_under(y0: f64, ceil: impl Fn(f64) -> f64) -> f64 {
    let step = 0.005;
    let n = (3.3 / step) as i32;
    let mut v = 0.0;
    for i in -n..=n {
        for j in -n..=n {
            let (x, y) = (f64::from(i) * step, f64::from(j) * step);
            let r2 = x * x + y * y;
            if (2.4 * 2.4..=3.255 * 3.255).contains(&r2) {
                v += ceil(y0 + y) * step * step;
            }
        }
    }
    v
}

fn build_and_check(name: &str, shape: CustomShape, profile: StudProfile, hand: f64) -> CustomPart {
    let part = build(&shape, &profile).unwrap_or_else(|e| panic!("{name}: {e}"));
    let rel = (part.volume_mm3 - hand) / hand;
    println!(
        "{name}: measured {:.3} mm³, hand {:.3} mm³ ({:+.4} %), bbox {:?}",
        part.volume_mm3,
        hand,
        rel * 100.0,
        part.bbox_mm
    );
    assert!(
        rel.abs() < 0.002,
        "{name}: measured {} vs hand {hand}",
        part.volume_mm3
    );
    let mesh = analyze_stl(&part.stl).unwrap();
    assert!(mesh.is_watertight(), "{name}: {mesh:?}");
    assert!(
        (mesh.volume_mm3 - part.volume_mm3).abs() / part.volume_mm3 < 0.01,
        "{name}: mesh volume {}",
        mesh.volume_mm3
    );
    assert!(
        part.three_mf.starts_with(b"PK"),
        "{name}: 3MF is a zip package"
    );
    std::fs::create_dir_all(OUT).unwrap();
    std::fs::write(format!("{OUT}/{name}.stl"), &part.stl).unwrap();
    part
}

fn assert_bbox(part: &CustomPart, min: [f64; 3], max: [f64; 3]) {
    for k in 0..3 {
        assert!(
            (part.bbox_mm.0[k] - min[k]).abs() < 1e-3,
            "min {:?} vs {min:?}",
            part.bbox_mm.0
        );
        assert!(
            (part.bbox_mm.1[k] - max[k]).abs() < 1e-3,
            "max {:?} vs {max:?}",
            part.bbox_mm.1
        );
    }
}

#[test]
fn brick_2x4_matches_shell_plus_studs_plus_tubes() {
    // Shell: 15.8 × 31.8 × 9.6 block less a 13.4 × 29.4 × 8.6 hollow; 8 studs; 3 tubes.
    let hand = 15.8 * 31.8 * 9.6 - 13.4 * 29.4 * 8.6 + 8.0 * stud() + 3.0 * tube(8.6);
    let part = build_and_check(
        "box_2x4x3",
        CustomShape::Box {
            w: 2,
            d: 4,
            plates: 3,
        },
        StudProfile::standard_fdm(),
        hand,
    );
    // Units are mm: 2×4 studs of 8 mm pitch, 9.6 mm body + 1.8 mm studs.
    assert_bbox(&part, [0.1, 0.1, 0.0], [15.9, 31.9, 11.4]);
    let mesh = analyze_stl(&part.stl).unwrap();
    assert!((mesh.bbox_mm.1[2] - 11.4).abs() < 1e-3);
}

#[test]
fn plate_1x1_and_2x2() {
    let hand = 7.8 * 7.8 * 3.2 - 5.4 * 5.4 * 2.2 + stud();
    let part = build_and_check(
        "plate_1x1x1",
        CustomShape::Box {
            w: 1,
            d: 1,
            plates: 1,
        },
        StudProfile::standard_fdm(),
        hand,
    );
    assert_bbox(&part, [0.1, 0.1, 0.0], [7.9, 7.9, 5.0]);
    let hand = 15.8 * 15.8 * 3.2 - 13.4 * 13.4 * 2.2 + 4.0 * stud() + tube(2.2);
    build_and_check(
        "plate_2x2x1",
        CustomShape::Box {
            w: 2,
            d: 2,
            plates: 1,
        },
        StudProfile::standard_fdm(),
        hand,
    );
}

#[test]
fn slope45_2x2() {
    // Roof: run 7.9 mm from the 1.7 mm front lip up to the ridge at y = 8.0.
    let s2 = 2f64.sqrt();
    let c0 = 1.7 + 1.2 - 1.2 * s2; // hollow ceiling at the inner front wall (y = 1.3)
    let knee = 8.6 - 1.7 + 0.1 + 1.2 * s2;
    let hollow = ((knee - 1.3) * (c0 + 8.6) / 2.0 + (14.7 - knee) * 8.6) * 13.4;
    let ceil = |y: f64| (1.7 + (y - 0.1) - 1.2 * s2).min(8.6);
    let hand =
        15.8 * 15.8 * 9.6 - hollow + tube_under(8.0, ceil) + 2.0 * stud() - 7.9 * 7.9 / 2.0 * 15.8;
    let part = build_and_check(
        "slope45_2x2x3",
        CustomShape::Slope45 {
            w: 2,
            d: 2,
            plates: 3,
        },
        StudProfile::standard_fdm(),
        hand,
    );
    assert_bbox(&part, [0.1, 0.1, 0.0], [15.9, 15.9, 11.4]);
}

#[test]
fn slope45_3x3() {
    // Too low to cover two rows at 45°: the roof runs 8.6 mm from a 1.0 mm lip.
    let s2 = 2f64.sqrt();
    let c0 = 1.0 + 1.2 - 1.2 * s2;
    let knee = 8.6 - 1.0 + 0.1 + 1.2 * s2;
    let hollow = ((knee - 1.3) * (c0 + 8.6) / 2.0 + (22.7 - knee) * 8.6) * 21.4;
    let ceil = |y: f64| (1.0 + (y - 0.1) - 1.2 * s2).min(8.6);
    let tubes = 2.0 * (tube_under(8.0, ceil) + tube(8.6));
    let hand = 23.8 * 23.8 * 9.6 - hollow + tubes + 3.0 * stud() - 8.6 * 8.6 / 2.0 * 23.8;
    build_and_check(
        "slope45_3x3x3",
        CustomShape::Slope45 {
            w: 3,
            d: 3,
            plates: 3,
        },
        StudProfile::standard_fdm(),
        hand,
    );
}

#[test]
fn curve_2x3() {
    // Quarter circle of radius 8.6 about (8.7, 1.0); the hollow follows it at radius 7.4.
    let hollow = ((22.7 - 8.7) * 8.6 + (8.7 - 1.3) * 1.0 + PI * 7.4 * 7.4 / 4.0) * 13.4;
    let ceil = |y: f64| {
        if y >= 8.7 {
            8.6
        } else {
            1.0 + (7.4f64.powi(2) - (y - 8.7).powi(2)).max(0.0).sqrt()
        }
    };
    let tubes = tube_under(8.0, ceil) + tube(8.6);
    let hand =
        15.8 * 23.8 * 9.6 - hollow + tubes + 2.0 * stud() - 8.6 * 8.6 * (1.0 - PI / 4.0) * 15.8;
    let part = build_and_check(
        "curve_2x3x3",
        CustomShape::Curve {
            w: 2,
            d: 3,
            plates: 3,
        },
        StudProfile::standard_fdm(),
        hand,
    );
    assert_bbox(&part, [0.1, 0.1, 0.0], [15.9, 23.9, 11.4]);
}

#[test]
fn footprint_l_shape() {
    // A 2 × 3 block with one extra cell at (2, 0).
    let cells = vec![(0, 0), (1, 0), (2, 0), (0, 1), (1, 1), (0, 2), (1, 2)];
    let outer = 23.8 * 7.8 + 15.8 * 16.0;
    let inner = 21.4 * 5.4 + 13.4 * 16.0;
    let hand = outer * 9.6 - inner * 8.6 + 7.0 * stud() + 2.0 * tube(8.6);
    let part = build_and_check(
        "footprint_l_x3",
        CustomShape::Footprint { cells, plates: 3 },
        StudProfile::standard_fdm(),
        hand,
    );
    assert_bbox(&part, [0.1, 0.1, 0.0], [23.9, 23.9, 11.4]);
}

#[test]
fn footprint_ring_with_hole() {
    let cells: Vec<_> = (0..3)
        .flat_map(|i| (0..3).map(move |j| (i, j)))
        .filter(|c| *c != (1, 1))
        .collect();
    let outer = 23.8 * 23.8 - 8.2 * 8.2;
    let inner = 21.4 * 21.4 - 10.6 * 10.6;
    let hand = outer * 9.6 - inner * 8.6 + 8.0 * stud();
    build_and_check(
        "footprint_ring_x3",
        CustomShape::Footprint { cells, plates: 3 },
        StudProfile::standard_fdm(),
        hand,
    );
}

#[test]
fn compliant_v2_stud_box() {
    // Hollow Ø5.28/Ø3.28 studs, each with a 0.5 mm bar cut 1.5 mm deep across it.
    let strip = |r: f64, a: f64| 2.0 * (a * (r * r - a * a).sqrt() + r * r * (a / r).asin());
    let ring = PI * (2.64f64.powi(2) - 1.64f64.powi(2)) * 1.8;
    let slot = (strip(2.64, 0.25) - strip(1.64, 0.25)) * 1.5;
    let hand = 15.8 * 15.8 * 9.6 - 13.4 * 13.4 * 8.6 + tube(8.6) + 4.0 * (ring - slot);
    let part = build_and_check(
        "v2_box_2x2x3",
        CustomShape::Box {
            w: 2,
            d: 2,
            plates: 3,
        },
        StudProfile::compliant_v2_experimental(),
        hand,
    );
    assert_bbox(&part, [0.1, 0.1, 0.0], [15.9, 15.9, 11.4]);
}

#[test]
fn commands_carry_an_expect_check() {
    let batch = commands(
        &CustomShape::Box {
            w: 2,
            d: 4,
            plates: 3,
        },
        &StudProfile::standard_fdm(),
    )
    .unwrap();
    let hand = 15.8 * 31.8 * 9.6 - 13.4 * 29.4 * 8.6 + 8.0 * stud() + 3.0 * tube(8.6);
    let v = batch["expect"]["volume_mm3"].as_f64().unwrap();
    assert!((v - hand).abs() < 1e-3, "{v} vs {hand}");
    assert_eq!(batch["expect"]["bodies"], 1);
    assert_eq!(batch["expect"]["bbox"]["max"], json!([15.9, 31.9, 11.4]));
    assert!(batch["commands"].as_array().unwrap().len() > 5);
    // The same batch runs through the freeform path too, with its own expect.
    let part = build_from_commands(&batch).unwrap();
    assert!((part.volume_mm3 - hand).abs() / hand < 0.002);
}

#[test]
fn bad_shapes_are_errors_not_panics() {
    let p = StudProfile::standard_fdm();
    for s in [
        CustomShape::Box {
            w: 0,
            d: 2,
            plates: 3,
        },
        CustomShape::Box {
            w: 2,
            d: 2,
            plates: 0,
        },
        CustomShape::Box {
            w: 9,
            d: 2,
            plates: 3,
        },
        CustomShape::Box {
            w: 2,
            d: 2,
            plates: 40,
        },
        CustomShape::Slope45 {
            w: 2,
            d: 1,
            plates: 3,
        },
        CustomShape::Curve {
            w: 2,
            d: 2,
            plates: 1,
        },
        CustomShape::Footprint {
            cells: vec![(0, 0), (1, 1)],
            plates: 3,
        },
        CustomShape::Footprint {
            cells: vec![],
            plates: 3,
        },
        CustomShape::Box {
            w: u32::MAX,
            d: u32::MAX,
            plates: u32::MAX,
        },
    ] {
        assert!(build(&s, &p).is_err(), "{s:?}");
    }
    let mut bad = StudProfile::standard_fdm();
    bad.wall = f64::NAN;
    assert!(matches!(
        build(
            &CustomShape::Box {
                w: 1,
                d: 1,
                plates: 1
            },
            &bad
        ),
        Err(CustomError::InvalidProfile(_))
    ));
}

#[test]
fn freeform_batches_are_bounded() {
    let cube =
        |s: f64| json!({"command": "solid.box", "params": {"length": s, "width": s, "height": s}});
    // A plain freeform part works.
    let part =
        build_from_commands(&json!({"commands": [cube(20.0)], "expect": {"volume_mm3": 8000.0}}))
            .unwrap();
    assert!((part.volume_mm3 - 8000.0).abs() < 0.01);
    // Too large.
    assert!(matches!(
        build_from_commands(&json!([cube(100.0)])),
        Err(CustomError::TooLarge(_))
    ));
    // A wrong expect names the mismatch.
    assert!(matches!(
        build_from_commands(&json!({"commands": [cube(20.0)], "expect": {"volume_mm3": 9000.0}})),
        Err(CustomError::Mismatch { .. })
    ));
    // Files, other command families, too many steps, huge literals.
    for refused in [
        json!([{"command": "file.export", "params": {"path": "/tmp/x.stl"}}]),
        json!([{"command": "sketch.text", "params": {"text": "hi"}}]),
        json!([cube(10.0), {"command": "solid.pattern.rectangular", "params": {"bodies": ["Body1"], "dir1": [1, 0, 0], "count1": 100000, "spacing1": 1}}]),
        json!([{"command": "solid.box", "params": {"length": 1e9, "width": 1, "height": 1}}]),
        json!({"commands": vec![cube(1.0); 2001]}),
        json!({"commands": []}),
        json!("solid.box"),
    ] {
        assert!(
            matches!(build_from_commands(&refused), Err(CustomError::Refused(_))),
            "{refused}"
        );
    }
    // An engine failure names the step.
    let err = build_from_commands(&json!([cube(10.0), {"command": "solid.fillet", "params": {"edges": [[500, 500, 500]], "radius": 1}}])).unwrap_err();
    assert!(
        matches!(&err, CustomError::Engine { index: 1, .. }),
        "{err}"
    );
}
