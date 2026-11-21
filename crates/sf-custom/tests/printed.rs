//! Printed body parts: any footprint, studs only in the given fused units, an engraved label.

use sf_custom::{CustomShape, LABEL_DEPTH, StudProfile, analyze_stl, build, plan};

fn printed(
    cells: &[(i32, i32)],
    plates: u32,
    units: &[&[(i32, i32)]],
    label: Option<&str>,
) -> CustomShape {
    CustomShape::Printed {
        cells: cells.to_vec(),
        plates,
        units: units.iter().map(|u| u.to_vec()).collect(),
        label: label.map(str::to_string), trim: None, sockets: None
    }
}

/// Builds (every stage's volume checked against its closed form, watertight mesh) and
/// returns the part's volume and the engraved text.
fn check(shape: &CustomShape) -> (f64, Option<String>) {
    let p = StudProfile::fused();
    let plan = plan(shape, &p).unwrap_or_else(|e| panic!("{}: {e}", shape.label()));
    let t = std::time::Instant::now();
    let part = build(shape, &p).unwrap_or_else(|e| panic!("{}: {e}", shape.label()));
    eprintln!(
        "{}: {:.1} mm³ in {:.2} s, label {:?}",
        shape.label(),
        part.volume_mm3,
        t.elapsed().as_secs_f64(),
        plan.label
    );
    assert!(analyze_stl(&part.stl).unwrap().is_watertight());
    (part.volume_mm3, plan.label)
}

#[test]
fn l_shape_brick_with_a_triple_and_a_pair() {
    // ###
    // #
    // #      a triple along X, a pair along Y up the stem.
    let cells = [(0, 0), (1, 0), (2, 0), (0, 1), (0, 2)];
    let shape = printed(
        &cells,
        3,
        &[&[(0, 0), (1, 0), (2, 0)], &[(0, 1), (0, 2)]],
        Some("B017"),
    );
    let (_, label) = check(&shape);
    assert_eq!(label.as_deref(), Some("017"));
}

#[test]
fn t_shape_plate_with_a_flat_column() {
    // ###      a pair on the bar, (2, 0) left flat, a pair down the stem.
    //  #
    //  #
    let cells = [(0, 0), (1, 0), (2, 0), (1, 1), (1, 2)];
    let shape = printed(&cells, 1, &[&[(0, 0), (1, 0)], &[(1, 1), (1, 2)]], Some("B2"));
    check(&shape);
}

#[test]
fn two_by_four_with_units_split_for_the_bricks_above() {
    // Row j = 0: two pairs; row j = 1: a triple and a flat end. Centre ribs, tubes.
    let cells: Vec<(i32, i32)> = (0..4).flat_map(|i| (0..2).map(move |j| (i, j))).collect();
    let shape = printed(
        &cells,
        3,
        &[
            &[(0, 0), (1, 0)],
            &[(2, 0), (3, 0)],
            &[(0, 1), (1, 1), (2, 1)],
        ],
        Some("B123"),
    );
    let (_, label) = check(&shape);
    assert_eq!(label.as_deref(), Some("123"));
}

#[test]
fn two_plate_part_and_offset_cells() {
    // Cells away from the origin are moved to it.
    let cells: Vec<(i32, i32)> = (5..8).flat_map(|i| (3..5).map(move |j| (i, j))).collect();
    let shape = printed(&cells, 2, &[&[(5, 3), (6, 3), (7, 3)], &[(5, 4), (6, 4)]], None);
    check(&shape);
}

#[test]
fn the_label_takes_exactly_its_engraving() {
    let cells: Vec<(i32, i32)> = (0..3).flat_map(|i| (0..2).map(move |j| (i, j))).collect();
    let units: [&[(i32, i32)]; 2] = [&[(0, 0), (1, 0), (2, 0)], &[(0, 1), (1, 1), (2, 1)]];
    let (with, text) = check(&printed(&cells, 3, &units, Some("88")));
    let (without, _) = check(&printed(&cells, 3, &units, None));
    assert_eq!(text.as_deref(), Some("88"));
    // "88": 2 × 13 pixels; the pixel size is whichever fitted.
    let removed = without - with;
    assert!(removed > 0.0, "the label removed nothing");
    let px = (removed / (26.0 * LABEL_DEPTH)).sqrt();
    assert!(
        [0.6, 0.5, 0.45, 0.4].iter().any(|s| (px - s).abs() < 0.01),
        "pixel {px:.3} mm"
    );
}

#[test]
fn bad_units_are_refused() {
    let cells: Vec<(i32, i32)> = (0..3).flat_map(|i| (0..2).map(move |j| (i, j))).collect();
    let p = StudProfile::fused();
    for (why, units) in [
        ("lone", vec![vec![(0, 0)]]),
        ("four", vec![vec![(0, 0), (1, 0), (2, 0), (3, 0)]]),
        ("diagonal", vec![vec![(0, 0), (1, 1)]]),
        ("gap", vec![vec![(0, 0), (2, 0)]]),
        ("outside", vec![vec![(2, 1), (3, 1)]]),
        ("overlap", vec![vec![(0, 0), (1, 0)], vec![(1, 0), (2, 0)]]),
    ] {
        let shape = CustomShape::Printed {
            cells: cells.clone(),
            plates: 3,
            units,
            label: None, trim: None, sockets: None
        };
        assert!(plan(&shape, &p).is_err(), "{why} should be refused");
    }
}
