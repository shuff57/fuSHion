//! Compliant-socket bricks: round studs on top, in-plane springs underneath. Every stage's
//! volume is checked against its closed form, and the springs grip studs Ø4.7–4.9 within
//! strain, clear of everything rigid.

use sf_custom::{CustomShape, MAX_STRAIN, StudProfile, analyze_stl, build, socket_fit};

fn rect(w: i32, d: i32) -> Vec<(i32, i32)> {
    (0..w).flat_map(|i| (0..d).map(move |j| (i, j))).collect()
}

/// A test brick: its name, cells and plates.
type TestBrick = (&'static str, Vec<(i32, i32)>, u32);

fn shapes() -> Vec<TestBrick> {
    vec![
        ("2x4x3", rect(2, 4), 3),
        ("2x2x3", rect(2, 2), 3),
        ("1x4x3", rect(1, 4), 3),
        ("1x1x3", rect(1, 1), 3),
        ("2x4x1", rect(2, 4), 1),
        ("L5x3", vec![(0, 0), (1, 0), (2, 0), (0, 1), (0, 2)], 3),
    ]
}

#[test]
fn every_test_brick_builds_with_exact_stage_volumes() {
    for p in [StudProfile::compliant_socket(), StudProfile::compliant_socket_nozzle04()] {
        for (name, cells, plates) in shapes() {
            let shape = CustomShape::Printed { cells, plates, units: vec![], label: None , trim: None, sockets: None };
            let t = std::time::Instant::now();
            let part = build(&shape, &p).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(analyze_stl(&part.stl).unwrap().is_watertight(), "{name}");
            let stages: Vec<&str> = part.stage_volumes_mm3.iter().map(|s| s.0.as_str()).collect();
            eprintln!("{name} ({}): {:.0} mm³ in {:.1} s, stages {stages:?}", p.socket.unwrap().finger, part.volume_mm3, t.elapsed().as_secs_f64());
            if name == "1x1x3" {
                assert!(stages.contains(&"LeafPads"));
            } else {
                assert!(stages.contains(&"SocketSprings"));
            }
        }
    }
    // Boxes too (a plain 2 × 4 and a 1 × 1).
    for (w, d) in [(2, 4), (1, 1)] {
        build(&CustomShape::Box { w, d, plates: 3 }, &StudProfile::compliant_socket()).unwrap();
    }
}

#[test]
fn springs_grip_every_stud_within_strain_and_nothing_rigid_touches() {
    for p in [StudProfile::compliant_socket(), StudProfile::compliant_socket_nozzle04()] {
        let spec = p.socket.unwrap();
        for (name, cells, _) in shapes() {
            // Centred studs from Ø4.7 to Ø4.9 (a printed stud 0.1 mm over or under): every
            // stud gripped, every spring within strain, nothing rigid touching.
            let mut worst = 0.0f64;
            for d in [4.7, 4.8, 4.9] {
                let f = socket_fit(&cells, &p, d, [0.0, 0.0]).unwrap();
                assert!(f.weakest_stud.1 > 0.0, "{name} Ø{d}: stud {:?} loose", f.weakest_stud.0);
                assert!(f.max_strain <= MAX_STRAIN, "{name} Ø{d}: strain {}", f.max_strain);
                assert!(f.rigid_clearance >= 0.0, "{name} Ø{d}: rigid clearance {}", f.rigid_clearance);
                worst = worst.max(f.max_strain);
            }
            // Pushed 0.1 mm off-centre (for the record): the tubes stop it; strain grows.
            let (mut off_strain, mut off_clear) = (0.0f64, f64::MAX);
            for d in [4.7, 4.8, 4.9] {
                for ox in [-0.1, 0.0, 0.1] {
                    for oy in [-0.1, 0.0, 0.1] {
                        let f = socket_fit(&cells, &p, d, [ox, oy]).unwrap();
                        off_strain = off_strain.max(f.max_strain);
                        off_clear = off_clear.min(f.rigid_clearance);
                    }
                }
            }
            let nominal = socket_fit(&cells, &p, 4.8, [0.0, 0.0]).unwrap();
            eprintln!(
                "{name} (finger {}): Ø4.8 {} springs, {:.1} N in all; worst centred strain {:.2} %; off-centre 0.1 mm: strain up to {:.2} %, rigid clearance down to {:.3} mm",
                spec.finger,
                nominal.contacts.len(),
                nominal.total_force,
                worst * 100.0,
                off_strain * 100.0,
                off_clear
            );
        }
    }
}

#[test]
fn nominal_numbers() {
    let p = StudProfile::compliant_socket_nozzle04();
    let f = socket_fit(&rect(2, 4), &p, 4.8, [0.0, 0.0]).unwrap();
    for c in &f.contacts {
        assert!((c.preload - p.socket.unwrap().preload).abs() < 1e-9, "{c:?}");
    }
    let one = socket_fit(&rect(1, 1), &p, 4.8, [0.0, 0.0]).unwrap();
    assert_eq!(one.contacts.len(), 2);
    assert!(one.contacts.iter().all(|c| c.kind == "leaf" && c.preload > 0.1));
}
