//! Curved (trimmed) printed bricks: watertight, the volume the sampled estimate says,
//! protected regions untouched, walls never thinner than 1 mm where the curve passes, and
//! overhangs flagged.

use sf_core::{Trim, TrimBox, TrimKind, TrimOp, TrimShape};
use sf_custom::{
    CustomShape, SUPPORT_AREA_MM2, StudProfile, TRIM_VOLUME_TOL, Trimmed, analyze_stl,
    build_trimmed, fused_units, inside_mesh, inside_trim,
};

const ID: [[f64; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
const ALONG_Y: [[f64; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0]];

fn add(kind: TrimKind, rotation: [[f64; 3]; 3], translation: [f64; 3]) -> TrimShape {
    TrimShape {
        op: TrimOp::Add,
        kind,
        rotation,
        translation,
    }
}

fn shape(w: u32, d: u32, trim: Trim, p: &StudProfile) -> CustomShape {
    let units = fused_units(&CustomShape::Box { w, d, plates: 3 }, p).unwrap();
    let cells = (0..w as i32)
        .flat_map(|i| (0..d as i32).map(move |j| (i, j)))
        .collect();
    CustomShape::Printed {
        cells,
        plates: 3,
        units,
        label: None,
        trim: Some(trim),
        sockets: None,
    }
}

fn roof() -> Trim {
    Trim {
        shapes: vec![
            add(
                TrimKind::Box {
                    size: [7.4, 40.0, 14.0],
                },
                ID,
                [2.7, 16.0, 5.0],
            ),
            add(
                TrimKind::Cylinder {
                    radii: [9.6, 9.6],
                    height: 40.0,
                },
                ALONG_Y,
                [6.4, 16.0, 0.0],
            ),
        ],
        protect: vec![],
        overhang: false,
    }
}

fn dome() -> Trim {
    Trim {
        shapes: vec![
            add(
                TrimKind::Box {
                    size: [20.0, 20.0, 5.6],
                },
                ID,
                [8.0, 8.0, 2.0],
            ),
            add(
                TrimKind::Ellipsoid {
                    radii: [11.3, 11.3, 4.8],
                },
                ID,
                [8.0, 8.0, 4.8],
            ),
        ],
        protect: vec![],
        overhang: false,
    }
}

fn belly() -> Trim {
    Trim {
        shapes: vec![add(
            TrimKind::Ellipsoid {
                radii: [8.5, 17.0, 9.6],
            },
            ID,
            [8.0, 16.0, 9.6],
        )],
        protect: vec![TrimBox {
            min: [-1.0, -1.0, 6.4],
            max: [17.0, 33.0, 12.0],
        }],
        overhang: true,
    }
}

fn check(name: &str, t: &Trimmed, trim: &Trim, h: f64) {
    assert!(analyze_stl(&t.part.stl).unwrap().is_watertight(), "{name}");
    let rel = (t.part.volume_mm3 - t.sampled_volume) / t.sampled_volume;
    assert!(rel.abs() <= TRIM_VOLUME_TOL, "{name}: {rel}");
    // A pseudo-random spread of points in the block.
    let (lo, hi) = t.untrimmed.bbox_mm;
    let mut seed = 0x1234_5678u64;
    let mut rnd = |a: f64, b: f64| {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        a + (b - a) * ((seed >> 11) as f64 / (1u64 << 53) as f64)
    };
    let (mut prot, mut walls) = (0, 0);
    for _ in 0..400 {
        let q = [rnd(lo[0], hi[0]), rnd(lo[1], hi[1]), rnd(0.05, h - 0.05)];
        let trimmed = inside_mesh(&t.part.stl, q);
        // Protected: whatever was solid in the plain brick stays solid (the hollow there may
        // be filled where the protected region's own inner face is exposed).
        if trim
            .protect
            .iter()
            .any(|b| (0..3).all(|i| q[i] >= b.min[i] && q[i] <= b.max[i]))
        {
            if inside_mesh(&t.untrimmed.stl, q) {
                assert!(trimmed, "{name}: protected solid point {q:?} was cut away");
            }
            prot += 1;
        }
        // An empty point inside the kept solid is in the hollow: a wall of at least 1 mm
        // (or the open bottom) all round it.
        if !trimmed && inside_trim(trim, q) {
            for dx in [-1.0f64, 0.0, 1.0] {
                for dy in [-1.0f64, 0.0, 1.0] {
                    for dz in [-1.0f64, 0.0, 1.0] {
                        let l: f64 = (dx * dx + dy * dy + dz * dz).sqrt();
                        if l == 0.0 {
                            continue;
                        }
                        let r = [q[0] + dx / l, q[1] + dy / l, q[2] + dz / l];
                        assert!(
                            r[2] < 0.0 || inside_trim(trim, r),
                            "{name}: wall under 1 mm near {q:?} (toward {r:?})"
                        );
                    }
                }
            }
            walls += 1;
        }
    }
    eprintln!(
        "{name}: {:.0} mm³ (sampled {:.0}), {prot} protected and {walls} hollow points checked, overhang {:.1} mm²",
        t.part.volume_mm3, t.sampled_volume, t.overhang_mm2
    );
}

#[test]
fn quarter_round_roof() {
    let p = StudProfile::fused();
    let trim = roof();
    let t = build_trimmed(&shape(2, 4, trim.clone(), &p), &p).unwrap();
    check("roof", &t, &trim, 9.6);
    assert_eq!(
        t.dropped_studs.len(),
        4,
        "the rolled-over row loses its studs"
    );
    assert!(
        t.overhang_mm2 <= SUPPORT_AREA_MM2,
        "a roof facing up needs no supports: {}",
        t.overhang_mm2
    );
}

#[test]
fn dome_top_keeps_its_sockets_open() {
    let p = StudProfile::fused();
    let trim = dome();
    let mut sh = shape(2, 2, trim.clone(), &p);
    if let CustomShape::Printed { sockets, cells, .. } = &mut sh {
        *sockets = Some(cells.clone());
    }
    let t = build_trimmed(&sh, &p).unwrap();
    check("dome", &t, &trim, 9.6);
    // A stud from below still fits under every cell.
    for c in [[4.0, 4.0], [12.0, 4.0], [4.0, 12.0], [12.0, 12.0]] {
        assert!(
            !inside_mesh(&t.part.stl, [c[0], c[1], 1.0]),
            "socket at {c:?} closed"
        );
    }
    assert!(t.overhang_mm2 <= SUPPORT_AREA_MM2);
}

#[test]
fn lower_hemisphere_belly_needs_supports() {
    let p = StudProfile::fused();
    let trim = belly();
    let t = build_trimmed(&shape(2, 4, trim.clone(), &p), &p).unwrap();
    check("belly", &t, &trim, 9.6);
    assert!(t.overhang_mm2 > SUPPORT_AREA_MM2, "{}", t.overhang_mm2);
    assert!(
        t.dropped_studs.is_empty(),
        "the protected top keeps every stud"
    );
}

#[test]
fn curved_compliant_socket_brick() {
    let p = StudProfile::compliant_socket();
    let trim = dome();
    let cells = (0..2).flat_map(|i| (0..2).map(move |j| (i, j))).collect();
    let s = CustomShape::Printed {
        cells,
        plates: 3,
        units: vec![],
        label: None,
        trim: Some(trim.clone()),
        sockets: Some(vec![(0, 0), (1, 1)]),
    };
    let t = build_trimmed(&s, &p).unwrap();
    check("socket dome", &t, &trim, 9.6);
}

/// A flower-pot wall: a tall cylinder cuts the outer column row. Only listed columns keep a
/// socket under a cut top; the rest of the hollow there is filled, and a listed socket keeps
/// its walls and ceiling even where they stand outside the shape.
#[test]
fn pot_wall_keeps_only_listed_sockets() {
    let p = StudProfile::fused();
    let trim = Trim {
        shapes: vec![add(
            TrimKind::Cylinder {
                radii: [41.0, 41.0],
                height: 64.0,
            },
            ID,
            [-20.0, 8.0, 20.0],
        )],
        protect: vec![],
        overhang: false,
    };
    let cells: Vec<(i32, i32)> = (0..3).flat_map(|i| (0..2).map(move |j| (i, j))).collect();
    let units = fused_units(
        &CustomShape::Box {
            w: 3,
            d: 2,
            plates: 3,
        },
        &p,
    )
    .unwrap();
    let s = CustomShape::Printed {
        cells,
        plates: 3,
        units,
        label: None,
        trim: Some(trim.clone()),
        sockets: Some(vec![(0, 0), (2, 0)]),
    };
    let t = build_trimmed(&s, &p).unwrap();
    assert_eq!(t.dropped_studs, vec![(2, 0), (2, 1)]);
    assert!(analyze_stl(&t.part.stl).unwrap().is_watertight());
    let rel = (t.part.volume_mm3 - t.sampled_volume) / t.sampled_volume;
    assert!(rel.abs() <= TRIM_VOLUME_TOL, "{rel}");
    // The listed socket under the cut column is open to a stud's height plus clearance...
    for z in [0.5, 1.0, 2.0] {
        assert!(
            !inside_mesh(&t.part.stl, [20.0, 4.0, z]),
            "listed socket closed at z {z}"
        );
    }
    // ... with its outer wall and ceiling kept although they stand outside the cylinder.
    assert!(
        inside_mesh(&t.part.stl, [23.5, 4.0, 1.0]),
        "listed socket's wall trimmed off"
    );
    assert!(
        inside_mesh(&t.part.stl, [20.0, 4.0, 2.8]),
        "listed socket's ceiling trimmed off"
    );
    // The unlisted cut column is filled, the kept columns stay hollow.
    assert!(
        inside_mesh(&t.part.stl, [20.0, 12.0, 1.0]),
        "unlisted socket left open"
    );
    assert!(
        !inside_mesh(&t.part.stl, [4.0, 12.0, 4.0]),
        "kept column's hollow filled"
    );
    assert!(t.overhang_mm2 <= SUPPORT_AREA_MM2, "{}", t.overhang_mm2);
}
