//! Bouquet stems, hubs and stem mounts: volumes against the closed form, Ø3.18 bars, the
//! peg's key, the hub's holes where `HubHole` puts them, and every flower template mounting
//! on a stem.

use serde_json::json;
use sf_core::{Hub, HubHole, MountKind, StemBar, StemBranch, StemDef, StemTip};
use sf_custom::stem::{
    BAR_BORE_D, PEG_HOLE_D, PEG_HOLE_FLAT, STEM_FLAT, main_arc, stem_commands, stem_volume_bounds,
};
use sf_custom::{
    BAR_D, BAR_MOUNT_DEPTH, analyze_stl, build_hub, build_stem, detail_template, inside_mesh,
};

fn stem(id: &str, length: f32, bend: f32) -> StemDef {
    StemDef {
        id: id.into(),
        length,
        bend,
        tip_bar: BAR_MOUNT_DEPTH as f32,
        bars: vec![StemBar {
            at: length / 2.0,
            length: 5.0,
        }],
        diameter: 5.0,
        tip: None,
        branches: vec![],
        style: String::new(),
    }
}

/// The half-width of the solid at height `z` across the centreline at `s`, found by probing
/// the mesh outward along the in-plane normal.
fn half_width(stl: &[u8], def: &StemDef, s: f64, z: f64) -> f64 {
    let a = main_arc(def);
    let (p, t) = (a.point(s), a.tangent(s));
    let n = [-t[1], t[0]];
    let (mut lo, mut hi) = (0.0, 6.0);
    for _ in 0..30 {
        let m = (lo + hi) / 2.0;
        if inside_mesh(stl, [p[0] + n[0] * m, p[1] + n[1] * m, z]) {
            lo = m;
        } else {
            hi = m;
        }
    }
    lo
}

#[test]
fn stem_volume_matches_the_closed_form() {
    // Straight, a tight bend (revolved), and a gentle one (lofted).
    for (id, len, bend) in [
        ("S11", 80.0, 0.0),
        ("S12", 70.0, 60.0),
        ("S13", 150.0, 15.0),
        ("S14", 120.0, -25.0),
    ] {
        let d = stem(id, len, bend);
        let (lo, hi) = stem_volume_bounds(&d).unwrap();
        assert!(
            (hi - lo).abs() < 1e-9,
            "{id}: no branches, an exact closed form"
        );
        let (_, engraved) = stem_commands(&d).unwrap();
        let want = lo - engraved * sf_custom::LABEL_DEPTH;
        let p = build_stem(&d).unwrap();
        let rel = (p.volume_mm3 - want) / want;
        eprintln!(
            "{id}: {:.2} mm³ vs {want:.2} ({:+.4} %)",
            p.volume_mm3,
            rel * 100.0
        );
        assert!(rel.abs() < sf_custom::VOLUME_TOL, "{id}: {rel}");
        assert!(analyze_stl(&p.stl).unwrap().is_watertight());
        // Flat on the bed, nothing below it.
        assert!(p.bbox_mm.0[2].abs() < 1e-6, "{id}: {:?}", p.bbox_mm);
    }
}

#[test]
fn bars_are_bar_d_and_the_rod_its_diameter() {
    let d = stem("S21", 90.0, 30.0);
    let p = build_stem(&d).unwrap();
    // At the axis's height the solid is as wide as the round section there (the STL's facets
    // lie a hair inside the true circle).
    let r_bar = half_width(&p.stl, &d, 45.0, STEM_FLAT);
    let r_tip = half_width(&p.stl, &d, 90.0 - BAR_MOUNT_DEPTH / 2.0, STEM_FLAT);
    let r_rod = half_width(&p.stl, &d, 20.0, STEM_FLAT);
    for (what, got, want) in [
        ("leaf bar", r_bar, BAR_D / 2.0),
        ("tip bar", r_tip, BAR_D / 2.0),
        ("rod", r_rod, 2.5),
    ] {
        assert!(
            got <= want + 1e-3 && got > want - 0.03,
            "{what}: radius {got:.3}, want {want:.3}"
        );
    }
}

#[test]
fn the_peg_key_is_on_the_bed_side() {
    let d = stem("S31", 60.0, 20.0);
    let p = build_stem(&d).unwrap();
    // The peg runs down −y from the shoulder; its flat is the bed (z = 0), STEM_FLAT under
    // its axis: wide there (3.75 mm), round on top (its full Ø4.8 at the axis's height).
    let y = -8.5;
    assert!(
        inside_mesh(&p.stl, [0.0, y, 0.05])
            && inside_mesh(&p.stl, [1.7, y, 0.05])
            && !inside_mesh(&p.stl, [2.05, y, 0.05])
    );
    assert!(
        inside_mesh(&p.stl, [2.35, y, STEM_FLAT]) && !inside_mesh(&p.stl, [2.45, y, STEM_FLAT])
    );
    assert!(
        inside_mesh(&p.stl, [0.0, y, STEM_FLAT + 2.35])
            && !inside_mesh(&p.stl, [0.0, y, STEM_FLAT + 2.45])
    );
}

#[test]
fn hub_holes_go_where_hubhole_says_keyed_toward_key() {
    let height = 14.0f32;
    let lean = |x: f32, y: f32, deg: f32, key: [f32; 3]| {
        let a = deg.to_radians();
        HubHole {
            top: [x, y, height],
            dir: [a.sin(), 0.0, a.cos()],
            key,
        }
    };
    let holes = vec![
        lean(7.0, 8.0, 0.0, [0.0, 1.0, 0.0]),
        lean(20.0, 8.0, -20.0, [0.0, -1.0, 0.0]),
    ];
    let hub = Hub {
        cells: (0..4u8)
            .flat_map(|i| (0..2u8).map(move |j| (i, j)))
            .collect(),
        height,
        holes: holes.clone(),
    };
    let p = build_hub(&hub).unwrap();
    assert!(analyze_stl(&p.stl).unwrap().is_watertight());
    for h in &holes {
        let d = h.dir.map(f64::from);
        let k = h.key.map(f64::from);
        let top = h.top.map(f64::from);
        let at = |t: f64, off: [f64; 3]| {
            [
                top[0] - d[0] * t + off[0],
                top[1] - d[1] * t + off[1],
                top[2] - d[2] * t + off[2],
            ]
        };
        // Empty down its axis to its depth, solid under its floor.
        for t in [1.0, 4.0, 8.0, 10.2] {
            assert!(
                !inside_mesh(&p.stl, at(t, [0.0; 3])),
                "{h:?}: solid in the hole at {t}"
            );
        }
        assert!(inside_mesh(&p.stl, at(11.2, [0.0; 3])), "{h:?}: no floor");
        // The key: solid just past the flat on the key side; open as far on the other side.
        let past = PEG_HOLE_FLAT + 0.15;
        assert!(
            inside_mesh(&p.stl, at(5.0, k.map(|v| v * past))),
            "{h:?}: no flat toward the key"
        );
        assert!(
            !inside_mesh(&p.stl, at(5.0, k.map(|v| -v * past))),
            "{h:?}: the round side is closed"
        );
        // Solid past the hole's radius all round.
        let side = [
            k[1] * d[2] - k[2] * d[1],
            k[2] * d[0] - k[0] * d[2],
            k[0] * d[1] - k[1] * d[0],
        ];
        assert!(inside_mesh(
            &p.stl,
            at(5.0, side.map(|v| v * (PEG_HOLE_D / 2.0 + 0.3)))
        ));
    }
}

#[test]
fn a_hub_too_thin_for_its_pegs_is_refused() {
    let hub = Hub {
        cells: vec![(0, 0), (1, 0)],
        height: 12.0,
        holes: vec![HubHole {
            top: [8.0, 4.0, 12.0],
            dir: [0.0, 0.0, 1.0],
            key: [1.0, 0.0, 0.0],
        }],
    };
    let e = build_hub(&hub).unwrap_err().to_string();
    assert!(e.contains("thicker"), "{e}");
}

#[test]
fn branched_sprig_builds_in_one_piece() {
    let mut d = stem("S41", 80.0, -10.0);
    d.bars.clear();
    d.diameter = 4.0;
    d.branches = vec![
        StemBranch {
            at: 35.0,
            angle: -35.0,
            length: 24.0,
            bend: 10.0,
            diameter: None,
            tip_bar: 4.5,
            tip: None,
        },
        StemBranch {
            at: 50.0,
            angle: 30.0,
            length: 18.0,
            bend: -8.0,
            diameter: None,
            tip_bar: 0.0,
            tip: Some(StemTip::Bud {
                length: 8.0,
                width: 4.5,
            }),
        },
    ];
    d.tip_bar = 0.0;
    d.tip = Some(StemTip::Blade {
        length: 20.0,
        width: 5.0,
    });
    let p = build_stem(&d).unwrap();
    assert!(analyze_stl(&p.stl).unwrap().is_watertight());
    let (lo, hi) = stem_volume_bounds(&d).unwrap();
    assert!(
        p.volume_mm3 > lo * 0.99 && p.volume_mm3 < hi,
        "{} not in {lo:.0}–{hi:.0}",
        p.volume_mm3
    );
    // Branches too close together are refused.
    let mut bad = d.clone();
    bad.branches[1].at = 36.0;
    bad.branches[1].angle = -33.0;
    assert!(build_stem(&bad).is_err());
}

#[test]
fn flower_templates_carry_mounts_and_stems() {
    // A kit flower: a bar hole up its hub, open from the mouth up the bar's depth.
    let daisy = detail_template("d", "daisy", "daisy", &json!({})).unwrap();
    let m = daisy.design.mount.expect("a daisy goes on a stem");
    assert_eq!(m.kind, MountKind::Hole);
    assert_eq!(m.axis, [0.0, 1.0, 0.0], "the hole opens downward");
    let style = daisy
        .design
        .stem
        .as_ref()
        .expect("a daisy suggests its stem");
    assert_eq!(
        style.branches.iter().filter(|b| b.head).count(),
        2,
        "a daisy sprig carries 3 heads"
    );
    let kit = daisy.design.kit.as_ref().unwrap();
    let hub = kit.parts.iter().position(|p| p.key == "hub").unwrap();
    let stl = &daisy.kit_prints[hub].stl;
    let (lo, hi) = daisy.kit_prints[hub].bbox_mm;
    let (cx, cy) = ((lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0);
    for z in [1.0, 2.5, 4.0, 1.8 + BAR_MOUNT_DEPTH - 0.2] {
        assert!(
            !inside_mesh(stl, [cx, cy, z]),
            "the bar hole is closed at z {z}"
        );
    }
    assert!(
        inside_mesh(stl, [cx + BAR_BORE_D / 2.0 + 0.4, cy, 4.0]),
        "no wall round the bar hole"
    );
    // A leaf on a stem: a clip.
    let leaf = detail_template("l", "leaf", "leaf", &json!({})).unwrap();
    assert_eq!(leaf.design.mount.map(|m| m.kind), Some(MountKind::Clip));
    assert!(leaf.design.stem.is_none());
    // One-piece heads too, and `"mount": false` leaves it off.
    let fh = detail_template("f", "flower head", "flower_head", &json!({})).unwrap();
    assert_eq!(fh.design.mount.map(|m| m.kind), Some(MountKind::Hole));
    let plain = detail_template(
        "p",
        "daisy",
        "daisy",
        &json!({"one_piece": true, "mount": false}),
    )
    .unwrap();
    assert!(plain.design.mount.is_none());
}
