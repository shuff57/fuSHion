//! Fit of the Slant v2 stud against standard bricks pushed on, in plane geometry only (no
//! SolidCraft): every stud's outline is sampled and measured against the walls and tubes of
//! the brick on top, for the brick aligned, shifted one stud along the row, shifted one row
//! sideways over the fin side, and turned 90° across the row.
//!
//! The part's wall ribs are checked the other way up: the part pushed onto a brick below, at
//! every offset, its ribs must press each stud they meet by exactly the configured
//! interference while its walls and tubes touch nothing but at the designed tangency.
//!
//! Run with `--nocapture` to see the table.

use sf_custom::{CustomShape, FitReport, StudProfile, TopBrick, fit, fit_below, slant_dims};

/// How far a fin may press into a wall or tube: its 0.5 mm slot less a 0.05 mm margin.
const FLEX: f64 = 0.45;
/// A round stud is tangent to a tube when within this of touching.
const TANGENT: f64 = 0.01;

/// Whether a case can be met at all by fused studs (see [`cases`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    /// Must fit: no overlap but fins within their flex.
    Fits,
    /// The brick on top ends (or its side runs) between the two lobes of a peanut, so its
    /// wall lands on the waist. No outline can avoid this while the two studs are fused.
    WallOnWaist,
}

struct Case {
    name: String,
    top: TopBrick,
    kind: Kind,
}

/// Whether a brick on top whose walls run across the row at grid lines `y0` and `y1` (in
/// studs) lands one on the waist of a `2 × d` part's peanuts (pairs `(0, 1)`, `(2, 3)`, …).
fn on_waist(d: u32, y0: i32, y1: i32) -> Kind {
    let pairs = i32::try_from(d / 2).unwrap();
    if (0..pairs).any(|k| y0 == 2 * k + 1 || y1 == 2 * k + 1) {
        Kind::WallOnWaist
    } else {
        Kind::Fits
    }
}

/// The placements tried on a `2 × d` part (rows along Y, peanuts pair along Y, row 0's fins
/// face −X and row 1's +X).
fn cases(d: u32) -> Vec<Case> {
    let c = |name: &str, w: u32, dd: u32, at: (i32, i32)| {
        let top = TopBrick { w, d: dd, at };
        let ends = (at.1, at.1 + i32::try_from(dd).unwrap());
        Case {
            name: name.into(),
            top,
            kind: on_waist(d, ends.0, ends.1),
        }
    };
    let d1 = i32::try_from(d).unwrap();
    let mut out = vec![
        c("aligned", 2, d, (0, 0)),
        // Tubes repeat every stud along the row, so a shift along it only moves the ends.
        c("along row +1, ends off the part", 2, d + 2, (0, -1)),
        c("along row -1, ends off the part", 2, d + 4, (0, -3)),
        c("sideways -X (row 0 fin side)", 2, d, (-1, 0)),
        c("sideways +X (row 1 fin side)", 2, d, (1, 0)),
        c("wider brick over both fin sides", 4, d, (-1, 0)),
        c("rotated 90°, 4x2 across", 4, 2, (-1, 0)),
        c("rotated 90°, 3x2 ending on row 0", 3, 2, (0, 0)),
        c("along row +1, same length", 2, d, (0, 1)),
        c("along row -1, same length", 2, d, (0, -1)),
    ];
    if d >= 3 {
        out.push(c("rotated 90°, 4x2 one stud up", 4, 2, (-1, 1)));
        out.push(c("rotated 90°, 4x2 at the far end", 4, 2, (-1, d1 - 2)));
    }
    out
}

fn row(part: &str, case: &Case, r: &FitReport) {
    let round = r
        .round_tube_clearance
        .map_or("-".to_string(), |c| format!("{c:+.4}"));
    println!(
        "{part:<14} {:<34} body/tube {:+.4}  body/wall {:+.4}  fin→wall {:.3}  fin→tube {:.3}  round/tube {round}  {}",
        case.name,
        r.body_tube_clearance,
        r.body_wall_clearance,
        r.fin_wall_preload,
        r.fin_tube_preload,
        if r.crossings.is_empty() {
            String::new()
        } else {
            format!("[{}]", r.crossings.join(", "))
        }
    );
}

fn check(part: &str, shape: &CustomShape, profile: &StudProfile, slant: bool) {
    let d = match *shape {
        CustomShape::Box { d, .. } => d,
        _ => unreachable!(),
    };
    for case in cases(d) {
        let r = fit(shape, profile, &case.top).unwrap();
        row(part, &case, &r);
        // Tubes never touch anything but a fin.
        assert!(r.body_tube_clearance >= 0.0, "{part} {}: {r:?}", case.name);
        // Fins press only as far as their slots let them.
        assert!(
            r.fin_wall_preload <= FLEX && r.fin_tube_preload <= FLEX,
            "{r:?}"
        );
        if let Some(c) = r.round_tube_clearance {
            assert!(c >= 0.0, "{part} {}: round stud overlaps a tube", case.name);
        }
        if !slant || case.kind == Kind::Fits {
            assert!(r.body_wall_clearance >= 0.0, "{part} {}: {r:?}", case.name);
            assert!(r.crossings.is_empty(), "{part} {}: {r:?}", case.name);
        } else {
            // The wall lands on a waist and nothing else: the tubes are clear (above) and the
            // wall's middle runs through a peanut.
            assert!(r.body_wall_clearance < 0.0, "{part} {}: {r:?}", case.name);
            assert_eq!(r.crossings, vec!["wall across a stud".to_string()]);
        }
        if !slant {
            // A standard brick only ever touches at the designed tangency.
            assert_eq!(r.fin_wall_preload, 0.0);
            assert_eq!(r.fin_tube_preload, 0.0);
        }
        if case.name == "aligned" {
            if slant {
                assert!(
                    (0.10..=0.20).contains(&r.fin_wall_preload),
                    "{part}: aligned preload {}",
                    r.fin_wall_preload
                );
            }
            // Studs beside tubes sit tangent to them, as LEGO's own do.
            if let Some(c) = r.round_tube_clearance {
                assert!(c < TANGENT, "{part}: round studs {c} off the tubes");
            }
        }
    }
}

fn shape(d: u32) -> CustomShape {
    CustomShape::Box { w: 2, d, plates: 3 }
}

#[test]
fn v2_studs_fit_bricks_pushed_on() {
    let v2 = StudProfile::slant_v2_experimental();
    let dims = slant_dims(&v2).unwrap();
    println!(
        "v2: waist {:.3}, fin reach {:.3}, root {:.3} wide, slot {:.3}–{:.3} from the waist line, least section {:.3}",
        dims.waist_width,
        dims.fin_reach,
        dims.root_width,
        dims.root_length,
        dims.slot_v.1 + 4.0,
        dims.min_section
    );
    for d in [2, 3, 4] {
        check(&format!("v2 2x{d}"), &shape(d), &v2, true);
    }
}

#[test]
fn v2_lobes_keep_the_stud_tangency_on_their_inner_side() {
    // The aligned brick's tubes sit beside the inner side of every lobe: the lobe there is
    // the plain stud circle, so it is tangent like a round stud (4√2 − 3.255 − 2.4 mm).
    let v2 = StudProfile::slant_v2_experimental();
    let r = fit(
        &shape(4),
        &v2,
        &TopBrick {
            w: 2,
            d: 4,
            at: (0, 0),
        },
    )
    .unwrap();
    let tangent = 4.0 * 2f64.sqrt() - 3.255 - 2.4;
    assert!(
        (r.body_tube_clearance - tangent).abs() < 1e-6,
        "{} vs {tangent}",
        r.body_tube_clearance
    );
    // The waist clears its tube by the 0.05 mm asked for, one row to the side.
    let r = fit(
        &shape(4),
        &v2,
        &TopBrick {
            w: 2,
            d: 4,
            at: (-1, 0),
        },
    )
    .unwrap();
    assert!(r.body_tube_clearance >= tangent - 1e-6);
}

#[test]
fn standard_studs_touch_only_at_the_designed_tangency() {
    let std = StudProfile::standard_fdm();
    for d in [2, 3, 4] {
        check(&format!("std 2x{d}"), &shape(d), &std, false);
    }
    let r = fit(
        &shape(4),
        &std,
        &TopBrick {
            w: 2,
            d: 4,
            at: (0, 0),
        },
    )
    .unwrap();
    let tangent = 4.0 * 2f64.sqrt() - 3.255 - 2.4;
    assert!((r.round_tube_clearance.unwrap() - tangent).abs() < 1e-6);
}

/// Bricks below a ribbed `2 × d` part: aligned, larger and offset, smaller, 1-wide, turned.
fn below_cases(d: u32) -> Vec<(&'static str, TopBrick)> {
    let b = |w, dd, at| TopBrick { w, d: dd, at };
    vec![
        ("aligned", b(2, d, (0, 0))),
        ("4x6 under, part centred", b(4, d + 2, (-1, -1))),
        ("4x6 under, part in a corner", b(4, d + 2, (0, 0))),
        ("shifted one row sideways", b(2, d, (1, 0))),
        ("shifted one stud along", b(2, d, (0, 1))),
        ("2x2 under one end", b(2, 2, (0, 0))),
        ("1x1 under a corner", b(1, 1, (0, 0))),
        ("1xd under one row", b(1, d, (1, 0))),
        ("turned 90°, 4x2 across", b(4, 2, (-1, 1))),
    ]
}

#[test]
fn wall_ribs_grip_studs_pushed_in_from_below() {
    for (name, base) in [
        ("std", StudProfile::standard_fdm()),
        ("v2", StudProfile::slant_v2_experimental()),
    ] {
        for d in [2, 4] {
            let part = shape(d);
            let p = StudProfile {
                wall_ribs: true,
                ..base
            };
            for (case, below) in below_cases(d) {
                // A standard brick below: its round Ø4.8 studs.
                let r = fit_below(&part, &p, &below, &StudProfile::standard_fdm()).unwrap();
                let rib = r.rib_clearance.unwrap();
                println!(
                    "{name} 2x{d} on {case:<28} rib {rib:+.4}  wall {:+.4}  tube {:+.4}",
                    r.body_wall_clearance, r.body_tube_clearance
                );
                // Every case puts a stud against a wall, so against a rib.
                assert!(
                    (rib + p.wall_rib_interference).abs() < 1e-9,
                    "{name} 2x{d} {case}: rib presses {} mm",
                    -rib
                );
                // Nothing else touches: the wall faces 0.3 mm off, tubes tangent.
                assert!(
                    (r.body_wall_clearance - 0.3).abs() < 1e-9,
                    "{name} {case}: {r:?}"
                );
                assert!(r.body_tube_clearance >= -1e-9, "{name} {case}: {r:?}");
                assert!(r.crossings.is_empty(), "{name} {case}: {r:?}");
                assert_eq!(r.fin_rib_preload, 0.0);
            }
            // Without ribs the same part touches nothing.
            let r = fit_below(
                &part,
                &base,
                &below_cases(d)[0].1,
                &StudProfile::standard_fdm(),
            )
            .unwrap();
            assert!(r.rib_clearance.is_none() && (r.body_wall_clearance - 0.3).abs() < 1e-9);
        }
    }
    // Firmer grip: the contact follows the setting.
    let p = StudProfile {
        wall_ribs: true,
        wall_rib_interference: 0.1,
        ..StudProfile::standard_fdm()
    };
    let r = fit_below(
        &shape(4),
        &p,
        &below_cases(4)[0].1,
        &StudProfile::standard_fdm(),
    )
    .unwrap();
    assert!((r.rib_clearance.unwrap() + 0.1).abs() < 1e-9);
}

#[test]
fn v2_fins_below_a_ribbed_part_press_past_their_slot() {
    // A Slant v2 brick below a ribbed part: its fins reach 2.85 mm from their lobe centre, and
    // a rib face stands 2.35 mm off (2.7 − 0.35), so the fins press about 0.5 mm into the
    // ribs: more than their 0.45 mm flex. Ribs are for standard studs; this pins the number
    // the docs quote.
    let v2 = StudProfile::slant_v2_experimental();
    let ribbed = StudProfile {
        wall_ribs: true,
        ..StudProfile::standard_fdm()
    };
    let r = fit_below(&shape(4), &ribbed, &below_cases(4)[0].1, &v2).unwrap();
    println!(
        "v2 below ribbed 2x4: fin→rib {:.3}, fin→wall {:.3}",
        r.fin_rib_preload, r.fin_wall_preload
    );
    assert!(
        r.fin_rib_preload > FLEX && r.fin_rib_preload < 0.52,
        "{r:?}"
    );
    // Without ribs the fins press the plain wall by their designed 0.15 mm.
    let r = fit_below(
        &shape(4),
        &StudProfile::standard_fdm(),
        &below_cases(4)[0].1,
        &v2,
    )
    .unwrap();
    assert!((0.10..=0.20).contains(&r.fin_wall_preload) && r.fin_rib_preload == 0.0);
}
