//! Detail kits: every kit builds; each part is watertight and its supports flag is what its
//! overhang measures; the assembled mesh is the parts placed by their poses; the sub-build
//! steps attach every copy exactly once; the clips fit their bars within strain.

use serde_json::json;
use sf_custom::{
    ClipSpec, Detail, KITS, MAX_STRAIN, SUPPORT_AREA_MM2, analyze_stl, assemble_kit, clip_fit,
    detail_template, overhang_area, peg_fit,
};
use std::collections::BTreeSet;

fn check(name: &str) -> Detail {
    let t = std::time::Instant::now();
    let d = detail_template(name, name, name, &json!({})).unwrap_or_else(|e| panic!("{name}: {e}"));
    let kit = d.design.kit.as_ref().unwrap_or_else(|| panic!("{name}: not a kit"));
    assert_eq!(kit.parts.len(), d.kit_prints.len());
    // Parts: watertight, the flag as measured, the display mesh the print's shape.
    for (p, pr) in kit.parts.iter().zip(&d.kit_prints) {
        let m = analyze_stl(&pr.stl).unwrap();
        assert!(m.is_watertight(), "{name}/{}: not watertight", p.key);
        let above = if pr.bbox_mm.0[2].abs() < 1e-6 && matches!(p.key.as_str(), "hub" | "mount" | "stem" | "eye") {
            1.85
        } else {
            0.3
        };
        assert_eq!(p.supports, overhang_area(&pr.stl, above) > SUPPORT_AREA_MM2, "{name}/{}", p.key);
        // The mesh (LDU) spans the print's box (mm × 2.5), give or take the display decimation.
        let (lo, hi) = pr.bbox_mm;
        let ext: Vec<f64> = (0..3).map(|i| (hi[i] - lo[i]) * 2.5).collect();
        let (mut a, mut b) = ([f32::MAX; 3], [f32::MIN; 3]);
        for v in &p.mesh.positions {
            for i in 0..3 {
                a[i] = a[i].min(v[i]);
                b[i] = b[i].max(v[i]);
            }
        }
        // LDU axes: x = x, y = −z, z = y.
        let mesh_ext = [f64::from(b[0] - a[0]), f64::from(b[2] - a[2]), f64::from(b[1] - a[1])];
        for i in 0..3 {
            assert!((mesh_ext[i] - ext[i]).abs() < 0.08 * ext[i] + 3.0, "{name}/{}: axis {i} {:.1} vs {:.1}", p.key, mesh_ext[i], ext[i]);
        }
        assert!(!p.copies.is_empty(), "{name}/{}: no copies", p.key);
    }
    // The assembled mesh is the parts placed by their poses.
    let asm = assemble_kit(kit);
    assert_eq!(asm, d.design.mesh, "{name}: assembled mesh differs from the poses");
    // Steps: every copy exactly once.
    let mut seen = BTreeSet::new();
    for s in &kit.steps {
        for &(pi, ci) in &s.parts {
            assert!(pi < kit.parts.len() && ci < kit.parts[pi].copies.len(), "{name}: step names ({pi}, {ci})");
            assert!(seen.insert((pi, ci)), "{name}: ({pi}, {ci}) attached twice");
        }
    }
    let all: usize = kit.parts.iter().map(|p| p.copies.len()).sum();
    assert_eq!(seen.len(), all, "{name}: steps miss some copies");
    // Roles: every part's role is listed, "main" among them.
    let roles: Vec<&str> = kit.roles.iter().map(|r| r.0.as_str()).collect();
    assert!(roles.contains(&"main"));
    for p in &kit.parts {
        assert!(roles.contains(&p.role.as_str()), "{name}/{}: role {} not listed", p.key, p.role);
    }
    assert!(!d.design.sockets.is_empty());
    eprintln!(
        "{name}: {:.1} s, {} parts, {} pieces, supports {:?}",
        t.elapsed().as_secs_f64(),
        kit.parts.len(),
        all,
        kit.parts.iter().filter(|p| p.supports).map(|p| p.key.as_str()).collect::<Vec<_>>()
    );
    d
}

fn no_supports(d: &Detail, name: &str) {
    let kit = d.design.kit.as_ref().unwrap();
    let flagged: Vec<&str> = kit.parts.iter().filter(|p| p.supports).map(|p| p.key.as_str()).collect();
    assert!(flagged.is_empty(), "{name}: {flagged:?} need supports");
}

#[test]
fn every_kit_is_listed() {
    let names: Vec<&str> = KITS.iter().map(|k| k.0).collect();
    for n in ["rose", "daisy", "aster", "gerbera", "poppy", "cornflower", "spike", "umbel", "fern", "leaf", "stem", "eye", "beak", "ear", "horn"] {
        assert!(names.contains(&n), "{n}");
    }
}

#[test]
fn clips_and_pegs_fit_within_strain() {
    for i in [0.0, 0.05, 0.10] {
        let f = clip_fit(&ClipSpec { interference: i, ..ClipSpec::default() });
        assert!(f.ok() && f.snap_strain <= MAX_STRAIN, "{i}: {f:?}");
    }
    assert!(peg_fit().strain < MAX_STRAIN);
}

#[test]
fn rose() {
    let d = check("rose");
    no_supports(&d, "rose");
}
#[test]
fn daisy() {
    let d = check("daisy");
    no_supports(&d, "daisy");
}
#[test]
fn aster() {
    check("aster");
}
#[test]
fn gerbera() {
    check("gerbera");
}
#[test]
fn poppy() {
    let d = check("poppy");
    no_supports(&d, "poppy");
}
#[test]
fn cornflower() {
    let d = check("cornflower");
    no_supports(&d, "cornflower");
}
#[test]
fn spike() {
    let d = check("spike");
    no_supports(&d, "spike");
}
#[test]
fn umbel() {
    let d = check("umbel");
    no_supports(&d, "umbel");
}
#[test]
fn small_kits() {
    for n in ["fern", "leaf", "stem", "eye", "beak", "ear", "horn"] {
        let d = check(n);
        no_supports(&d, n);
    }
}

#[test]
fn a_flower_on_a_stem_and_one_piece_still_works() {
    let d = detail_template("r", "r", "rose", &json!({"stem_height": 10, "petals": 3})).unwrap();
    let hub = &d.design.kit.as_ref().unwrap().parts[0];
    assert_eq!(hub.key, "hub");
    assert!(!hub.supports, "the calyx is steeper than 45°");
    let one = detail_template("r1", "r1", "rose", &json!({"one_piece": true})).unwrap();
    assert!(one.design.kit.is_none() && one.kit_prints.is_empty());
}
