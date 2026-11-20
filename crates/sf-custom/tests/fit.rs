//! Fit of fused studs (pairs and triples with leaf-spring fins) against bricks pushed on, in
//! plane geometry only (no SolidCraft): every stud's outline is sampled and measured against
//! the walls, tubes and wall ribs of the brick on the other side.
//!
//! Every brick fuSHion makes is fused, so both sides of each joint are fused bricks here: a
//! fused brick's underside is a standard one (walls, tubes, centre ribs above the studs) plus,
//! when it has them, end-wall ribs, so the brick on top is modelled by its profile's walls,
//! tubes and ribs, and the brick below by its fused studs. Placements are enumerated: every
//! size of brick on the other side from a list, at every offset where it overlaps the part,
//! aligned, shifted along and across the rows, and turned 90°.
//!
//! A brick whose end wall lands between two lobes of one unit (on a waist) cannot fit: no
//! outline avoids that while the studs are fused. Those placements are worked out here,
//! independently of the crate (from the documented run layout), and must show exactly that
//! one collision and nothing else.
//!
//! The standard round-stud profile is kept as a regression guard. Run with `--nocapture` to
//! see the tables.

use sf_custom::{
    CustomShape, FitReport, StudProfile, TopBrick, fit, fit_below, fit_under, slant_dims,
};

/// How far a fin may press into a wall, tube or rib: its 0.5 mm slot less a 0.05 mm margin.
const FLEX: f64 = 0.45;
/// A round stud is tangent to a tube when within this of touching.
const TANGENT: f64 = 0.01;
/// Outline sampling: a stud meeting a flat face off its extreme point is found to within this.
const SAMPLED: f64 = 3e-3;

/// The run layout as documented: pairs, and for an odd run one triple with `⌈k/2⌉` of its
/// `k` pairs before it.
fn split_run(len: usize) -> Vec<usize> {
    assert!(len >= 2);
    if len.is_multiple_of(2) {
        return vec![2; len / 2];
    }
    let k = (len - 3) / 2;
    let before = k.div_ceil(2);
    let mut v = vec![2; before];
    v.push(3);
    v.extend(std::iter::repeat_n(2, k - before));
    v
}

/// A box brick `w × d` with its cell `(0, 0)` on the part's cell `at`.
#[derive(Clone, Copy, Debug)]
struct Brick {
    w: i32,
    d: i32,
    at: (i32, i32),
}

impl Brick {
    fn of(t: &TopBrick) -> Self {
        Self {
            w: t.w as i32,
            d: t.d as i32,
            at: t.at,
        }
    }

    /// Whether its rows run along Y (the longer side; ties: Y).
    fn rows_along_y(&self) -> bool {
        self.d >= self.w
    }

    /// The waists of its rows: `(row's cross index, grid line along the row)`, part grid.
    fn waists(&self) -> Vec<(i32, i32)> {
        let along_y = self.rows_along_y();
        let (len, rows) = if along_y {
            (self.d, self.w)
        } else {
            (self.w, self.d)
        };
        let (start, cross0) = if along_y {
            (self.at.1, self.at.0)
        } else {
            (self.at.0, self.at.1)
        };
        let mut out = Vec::new();
        for r in 0..rows {
            let mut k = start;
            for n in split_run(len as usize) {
                for m in 1..n as i32 {
                    out.push((cross0 + r, k + m));
                }
                k += n as i32;
            }
        }
        out
    }
}

/// Whether `upper`, pushed onto the fused `lower`, has a wall on one of `lower`'s waists: a
/// wall across `lower`'s rows at a waist's grid line, over that waist's row.
fn wall_on_waist(lower: &Brick, upper: &Brick) -> bool {
    let along_y = lower.rows_along_y();
    let (ends, span) = if along_y {
        (
            [upper.at.1, upper.at.1 + upper.d],
            (upper.at.0, upper.at.0 + upper.w),
        )
    } else {
        (
            [upper.at.0, upper.at.0 + upper.w],
            (upper.at.1, upper.at.1 + upper.d),
        )
    };
    lower
        .waists()
        .iter()
        .any(|&(row, g)| ends.contains(&g) && (span.0..span.1).contains(&row))
}

/// Every `w × d` from `sizes` at every offset where it overlaps a `pw × pd` part by a cell.
fn placements(pw: i32, pd: i32, sizes: &[(u32, u32)]) -> Vec<TopBrick> {
    let mut out = Vec::new();
    for &(w, d) in sizes {
        let (wi, di) = (w as i32, d as i32);
        for x in (1 - wi)..pw {
            for y in (1 - di)..pd {
                out.push(TopBrick { w, d, at: (x, y) });
            }
        }
    }
    out
}

/// Bricks met on the other side of a part: 1- and 2-wide in both orientations, longer and
/// shorter than the part, and a 4×4. No 1×1 (it can't be fused) or other lone-stud row.
const SIZES: &[(u32, u32)] = &[
    (1, 2),
    (2, 1),
    (1, 3),
    (3, 1),
    (1, 4),
    (4, 1),
    (1, 5),
    (2, 2),
    (2, 3),
    (3, 2),
    (2, 4),
    (4, 2),
    (2, 5),
    (5, 2),
    (2, 6),
    (6, 2),
    (3, 4),
    (4, 4),
];

/// `f` of every item, on all cores (each fit is independent and takes tens of ms).
fn par_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let n = std::thread::available_parallelism().map_or(4, |n| n.get());
    let chunk = items.len().div_ceil(n).max(1);
    std::thread::scope(|s| {
        let handles: Vec<_> = items
            .chunks(chunk)
            .map(|c| s.spawn(|| c.iter().map(&f).collect::<Vec<R>>()))
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap())
            .collect()
    })
}

fn shape(w: u32, d: u32) -> CustomShape {
    CustomShape::Box { w, d, plates: 3 }
}

fn fused() -> StudProfile {
    StudProfile::fused()
}

fn ribbed() -> StudProfile {
    StudProfile {
        wall_ribs: true,
        ..fused()
    }
}

/// The checks every joint must pass, `waist` telling whether a wall must land on a waist.
fn assert_joint(what: &str, r: &FitReport, waist: bool, interference: f64) {
    // Tubes never touch anything but a fin; fins press only as far as their slots let them.
    assert!(r.body_tube_clearance >= -1e-9, "{what}: {r:?}");
    assert!(
        r.fin_wall_preload <= FLEX && r.fin_tube_preload <= FLEX && r.fin_rib_preload <= FLEX,
        "{what}: {r:?}"
    );
    if waist {
        // The wall lands on a waist and nothing else: the tubes are clear (above) and the
        // wall's middle runs through a fused unit.
        assert!(r.body_wall_clearance < 0.0, "{what}: {r:?}");
        assert_eq!(
            r.crossings,
            vec!["wall across a stud".to_string()],
            "{what}"
        );
    } else {
        assert!(r.body_wall_clearance >= -1e-9, "{what}: {r:?}");
        assert!(r.crossings.is_empty(), "{what}: {r:?}");
        // A rib presses solid stud material by no more than its set interference.
        if let Some(c) = r.rib_clearance {
            assert!(c >= -interference - 1e-9, "{what}: rib presses {c}");
        }
    }
}

#[derive(Default)]
struct Tally {
    cases: usize,
    waists: usize,
    fin_wall: f64,
    fin_rib: f64,
    fin_tube: f64,
    rib_body: f64,
}

impl Tally {
    fn add(&mut self, r: &FitReport, waist: bool) {
        self.cases += 1;
        self.waists += usize::from(waist);
        if waist {
            return;
        }
        self.fin_wall = self.fin_wall.max(r.fin_wall_preload);
        self.fin_rib = self.fin_rib.max(r.fin_rib_preload);
        self.fin_tube = self.fin_tube.max(r.fin_tube_preload);
        if let Some(c) = r.rib_clearance {
            self.rib_body = self.rib_body.max(-c);
        }
    }

    fn print(&self, what: &str) {
        println!(
            "{what:<40} {:>4} placements ({:>3} on a waist)  max fin→wall {:.3}  fin→rib {:.3}  fin→tube {:.3}  rib→stud {:.3}",
            self.cases, self.waists, self.fin_wall, self.fin_rib, self.fin_tube, self.rib_body
        );
    }
}

#[test]
fn fused_studs_fit_bricks_pushed_on() {
    let p = fused();
    let dims = slant_dims(&p).unwrap();
    println!(
        "fused: waist {:.3}, fin reach {:.3}, root {:.3} wide, least section {:.3}",
        dims.waist_width, dims.fin_reach, dims.root_width, dims.min_section
    );
    for (w, d) in [
        (2, 2),
        (2, 3),
        (2, 4),
        (2, 5),
        (1, 2),
        (1, 3),
        (1, 5),
        (3, 4),
        (4, 2),
    ] {
        let part = shape(w, d);
        let me = Brick {
            w: w as i32,
            d: d as i32,
            at: (0, 0),
        };
        let mut t = Tally::default();
        let tops = placements(w as i32, d as i32, SIZES);
        let reports = par_map(&tops, |top| fit(&part, &p, top).unwrap());
        for (top, r) in tops.iter().zip(&reports) {
            let r = r.clone();
            let waist = wall_on_waist(&me, &Brick::of(top));
            assert_joint(&format!("{w}x{d} under {top:?}"), &r, waist, 0.0);
            t.add(&r, waist);
        }
        // Aligned: every fin presses the wall of the brick on top by its designed preload.
        let r = fit(&part, &p, &TopBrick { w, d, at: (0, 0) }).unwrap();
        assert!(
            (0.10..=0.20).contains(&r.fin_wall_preload),
            "{w}x{d} aligned: {r:?}"
        );
        t.print(&format!("fused {w}x{d} under plain bricks"));
    }
}

#[test]
fn fused_lobes_keep_the_stud_tangency_on_their_inner_side() {
    // The aligned brick's tubes sit beside the inner side of every end lobe, away from its
    // waist: the stud circle there, so tangent like a round stud (4√2 − 3.255 − 2.4 mm).
    // Every tube beside a triple's middle lobe is beside a waist too, which keeps it the
    // waist clearance (0.05 mm) off: a 2×3 (one triple per row) touches no tube.
    let tangent = 4.0 * 2f64.sqrt() - 3.255 - 2.4;
    for (d, least) in [(3, 0.05), (4, tangent), (5, tangent)] {
        let r = fit(
            &shape(2, d),
            &fused(),
            &TopBrick {
                w: 2,
                d,
                at: (0, 0),
            },
        )
        .unwrap();
        assert!(
            (r.body_tube_clearance - least).abs() < 1e-6,
            "2x{d}: {} vs {least}",
            r.body_tube_clearance
        );
        // One row to the side, the waists clear their tubes.
        let r = fit(
            &shape(2, d),
            &fused(),
            &TopBrick {
                w: 2,
                d,
                at: (-1, 0),
            },
        )
        .unwrap();
        assert!(r.body_tube_clearance >= tangent - 1e-6);
    }
    // A triple's middle lobe (finned or finless) stands 0.3 mm off a wall, like a round
    // stud, on either side.
    let r = fit(
        &shape(3, 3),
        &fused(),
        &TopBrick {
            w: 3,
            d: 3,
            at: (0, 0),
        },
    )
    .unwrap();
    assert!(
        r.crossings.is_empty() && r.body_wall_clearance >= 0.3 - 1e-6,
        "{r:?}"
    );
}

#[test]
fn one_wide_fused_parts_are_gripped_by_the_fins_below() {
    // No pins: the fins of a fused row below press a 1-wide part's long walls.
    let p = fused();
    let cases = [
        (
            "1x4 on a 1x4, aligned",
            4,
            TopBrick {
                w: 1,
                d: 4,
                at: (0, 0),
            },
        ),
        (
            "1x4 on row 1 of a 2x4 (fins +X)",
            4,
            TopBrick {
                w: 2,
                d: 4,
                at: (-1, 0),
            },
        ),
        (
            "1x4 on row 0 of a 2x4 (fins -X), one stud across",
            4,
            TopBrick {
                w: 2,
                d: 4,
                at: (0, 0),
            },
        ),
        (
            "1x2 on a 1x6, one pair along",
            2,
            TopBrick {
                w: 1,
                d: 6,
                at: (0, -2),
            },
        ),
        (
            "1x3 on the triple of a 1x5",
            3,
            TopBrick {
                w: 1,
                d: 5,
                at: (0, -2),
            },
        ),
        (
            "1x2 on the pair of a 1x5",
            2,
            TopBrick {
                w: 1,
                d: 5,
                at: (0, 0),
            },
        ),
        (
            "1x5 on a 1x5, aligned",
            5,
            TopBrick {
                w: 1,
                d: 5,
                at: (0, 0),
            },
        ),
        (
            "1x5 on a 2x5, one stud across",
            5,
            TopBrick {
                w: 2,
                d: 5,
                at: (0, 0),
            },
        ),
        (
            "1x3 on a 2x3",
            3,
            TopBrick {
                w: 2,
                d: 3,
                at: (-1, 0),
            },
        ),
    ];
    for (what, d, below) in cases {
        let r = fit_below(&shape(1, d), &p, &below, &p).unwrap();
        println!(
            "{what:<48} fin→wall {:.3}  body/wall {:+.3}  body/tube {:+.3}  {:?}",
            r.fin_wall_preload, r.body_wall_clearance, r.body_tube_clearance, r.crossings
        );
        assert!((0.10..=0.20).contains(&r.fin_wall_preload), "{what}: {r:?}");
        assert_joint(what, &r, false, 0.0);
    }
    // Shifted one stud along a pair, its end walls land on the waists below.
    let r = fit_below(
        &shape(1, 2),
        &p,
        &TopBrick {
            w: 1,
            d: 6,
            at: (0, -1),
        },
        &p,
    )
    .unwrap();
    assert_joint("1x2 one stud along a 1x6", &r, true, 0.0);
}

#[test]
fn ribbed_fused_parts_on_fused_bricks_below() {
    // The part has end-wall ribs; the bricks below are fused, in every orientation and
    // offset, so their fins meet the long walls (aligned) or the end walls (turned 90°).
    let p = ribbed();
    let below = fused();
    let i = p.wall_rib_interference;
    println!(
        "fused end-wall ribs stand {:.3} mm proud, centred {} mm either side of each stud",
        p.wall_rib_protrusion(),
        sf_custom::FUSED_RIB_OFFSET
    );
    let mut turned = Tally::default();
    for (w, d) in [(2, 4), (2, 3), (2, 2), (1, 4), (1, 5), (3, 4)] {
        let me = Brick {
            w: w as i32,
            d: d as i32,
            at: (0, 0),
        };
        let mut t = Tally::default();
        let bs = placements(w as i32, d as i32, SIZES);
        let reports = par_map(&bs, |b| fit_below(&shape(w, d), &p, b, &below).unwrap());
        for (&b, r) in bs.iter().zip(reports) {
            let lower = Brick::of(&b);
            let waist = wall_on_waist(&lower, &me);
            assert_joint(&format!("ribbed {w}x{d} on {b:?}"), &r, waist, i);
            t.add(&r, waist);
            if lower.rows_along_y() != me.rows_along_y() {
                turned.add(&r, waist);
            }
        }
        t.print(&format!("ribbed fused {w}x{d} on fused bricks"));
        // Aligned, the ribs grip the caps of the end lobes by their interference.
        let r = fit_below(&shape(w, d), &p, &TopBrick { w, d, at: (0, 0) }, &below).unwrap();
        let rib = r.rib_clearance.unwrap();
        assert!((rib + i).abs() < SAMPLED, "{w}x{d}: rib {rib}");
    }
    turned.print("  of which turned 90° to the part");
    // Turned 90°, the fins face the end walls and meet the off-centre ribs about as a round
    // stud would: well inside their flex.
    assert!(
        turned.fin_rib > 0.0 && turned.fin_rib < 0.10,
        "{}",
        turned.fin_rib
    );
}

#[test]
fn ribbed_fused_bricks_on_top_of_fused_parts() {
    // The same joint the other way round: a ribbed fused brick pushed onto the part.
    let top = ribbed();
    let p = fused();
    for (w, d) in [(2, 4), (2, 3), (1, 5)] {
        let me = Brick {
            w: w as i32,
            d: d as i32,
            at: (0, 0),
        };
        let mut t = Tally::default();
        let bs = placements(w as i32, d as i32, SIZES);
        let reports = par_map(&bs, |b| fit_under(&shape(w, d), &p, b, &top).unwrap());
        for (&b, r) in bs.iter().zip(reports) {
            let waist = wall_on_waist(&me, &Brick::of(&b));
            assert_joint(
                &format!("{w}x{d} under ribbed {b:?}"),
                &r,
                waist,
                top.wall_rib_interference,
            );
            t.add(&r, waist);
        }
        t.print(&format!("fused {w}x{d} under ribbed fused bricks"));
    }
}

#[test]
fn ribbed_fused_parts_grip_standard_studs() {
    // Round studs below meet the end-wall ribs at the ribs' inner edges by the interference.
    let p = ribbed();
    for (w, d) in [(2, 4), (1, 3)] {
        for b in [
            TopBrick { w, d, at: (0, 0) },
            TopBrick {
                w: 4,
                d: d + 2,
                at: (-1, -1),
            },
            TopBrick {
                w: 4,
                d: 2,
                at: (-1, 0),
            },
        ] {
            let r = fit_below(&shape(w, d), &p, &b, &StudProfile::standard_fdm()).unwrap();
            let rib = r.rib_clearance.unwrap();
            println!("ribbed fused {w}x{d} on standard {b:?}: rib {rib:+.4}");
            assert!((rib + p.wall_rib_interference).abs() < SAMPLED, "{rib}");
            assert!(r.body_wall_clearance >= 0.3 - 1e-6 && r.crossings.is_empty());
            assert!(r.body_tube_clearance >= -1e-9);
        }
    }
}

#[test]
fn centred_ribs_would_overload_fused_fins() {
    // Why fused bricks keep ribs off the stud centre line: a standard (centred, all-walls)
    // ribbed brick on a fused brick takes about 0.5 mm of fin travel, past the 0.45 flex.
    let ribbed_std = StudProfile {
        wall_ribs: true,
        ..StudProfile::standard_fdm()
    };
    let aligned = TopBrick {
        w: 2,
        d: 4,
        at: (0, 0),
    };
    let r = fit_below(&shape(2, 4), &ribbed_std, &aligned, &fused()).unwrap();
    println!(
        "fused below a centred-ribbed 2x4: fin→rib {:.3}, fin→wall {:.3}",
        r.fin_rib_preload, r.fin_wall_preload
    );
    assert!(
        r.fin_rib_preload > FLEX && r.fin_rib_preload < 0.52,
        "{r:?}"
    );
    // The same on a fused brick's end walls, if its ribs were centred: a fused brick below
    // turned 90° has its fins there.
    let turned = TopBrick {
        w: 4,
        d: 2,
        at: (-1, 0),
    };
    let r = fit_below(&shape(2, 4), &ribbed_std, &turned, &fused()).unwrap();
    assert!(r.fin_rib_preload > FLEX, "{r:?}");
    // Without ribs the fins press the plain wall by their designed 0.15 mm.
    let r = fit_below(
        &shape(2, 4),
        &StudProfile::standard_fdm(),
        &aligned,
        &fused(),
    )
    .unwrap();
    assert!((0.10..=0.20).contains(&r.fin_wall_preload) && r.fin_rib_preload == 0.0);
}

#[test]
fn lone_studs_are_refused() {
    let e = fit(
        &shape(1, 1),
        &fused(),
        &TopBrick {
            w: 1,
            d: 1,
            at: (0, 0),
        },
    )
    .unwrap_err();
    assert!(e.to_string().contains("alone"), "{e}");
}

// ----- The standard profile: a regression guard on the model itself. -----

#[test]
fn standard_studs_touch_only_at_the_designed_tangency() {
    let std = StudProfile::standard_fdm();
    for d in [2, 3, 4] {
        let tops = placements(2, d as i32, SIZES);
        let reports = par_map(&tops, |top| fit(&shape(2, d), &std, top).unwrap());
        for (top, r) in tops.iter().zip(reports) {
            assert_joint(&format!("std 2x{d} {top:?}"), &r, false, 0.0);
            assert_eq!(r.fin_wall_preload, 0.0);
            assert_eq!(r.fin_tube_preload, 0.0);
            assert!(r.round_tube_clearance.is_none_or(|c| c >= 0.0));
        }
        let r = fit(
            &shape(2, d),
            &std,
            &TopBrick {
                w: 2,
                d,
                at: (0, 0),
            },
        )
        .unwrap();
        let tangent = 4.0 * 2f64.sqrt() - 3.255 - 2.4;
        assert!((r.round_tube_clearance.unwrap() - tangent).abs() < 1e-6);
        assert!(r.round_tube_clearance.unwrap() < TANGENT);
    }
}

#[test]
fn standard_wall_ribs_grip_round_studs_pushed_in_from_below() {
    let std = StudProfile::standard_fdm();
    let p = StudProfile {
        wall_ribs: true,
        ..std
    };
    for d in [2, 4] {
        let bs = placements(2, d as i32, SIZES);
        let reports = par_map(&bs, |b| fit_below(&shape(2, d), &p, b, &std).unwrap());
        for (b, r) in bs.iter().zip(reports) {
            let rib = r.rib_clearance.unwrap();
            // Every placement puts a stud against a wall, so square on against a rib.
            assert!(
                (rib + p.wall_rib_interference).abs() < 1e-9,
                "2x{d} {b:?}: {rib}"
            );
            assert!((r.body_wall_clearance - 0.3).abs() < 1e-9, "{b:?}: {r:?}");
            assert!(r.body_tube_clearance >= -1e-9 && r.crossings.is_empty());
            assert_eq!(r.fin_rib_preload, 0.0);
        }
    }
    // Firmer grip: the contact follows the setting.
    let p = StudProfile {
        wall_rib_interference: 0.1,
        ..p
    };
    let r = fit_below(
        &shape(2, 4),
        &p,
        &TopBrick {
            w: 2,
            d: 4,
            at: (0, 0),
        },
        &std,
    )
    .unwrap();
    assert!((r.rib_clearance.unwrap() + 0.1).abs() < 1e-9);
}
