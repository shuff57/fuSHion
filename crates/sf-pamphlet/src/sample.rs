//! A small hand-made model with every kind of part, for tests and the example.

use sf_core::{Cell, CustomDef, CustomKind, DetailMesh, Model, Placement, Rot};

fn brick(id: &str, cells: Vec<(u8, u8)>, plates: u8) -> CustomDef {
    CustomDef {
        id: id.to_string(),
        kind: CustomKind::Brick,
        studs: cells.clone(),
        units: Vec::new(),
        sockets: cells.clone(),
        cells,
        colors: Default::default(),
        plates,
    }
}

/// A box mesh in the part frame (LDU, -Y up, origin at the top centre of the bounding
/// box): `w × d` LDU, from `top` (negative: above the bounding box) down to `bottom`.
fn box_mesh(m: &mut DetailMesh, w: f32, d: f32, top: f32, bottom: f32) {
    let (x0, x1, z0, z1) = (-w / 2.0, w / 2.0, -d / 2.0, d / 2.0);
    let b = m.positions.len() as u32;
    for &(x, y, z) in &[
        (x0, top, z0),
        (x1, top, z0),
        (x1, top, z1),
        (x0, top, z1),
        (x0, bottom, z0),
        (x1, bottom, z0),
        (x1, bottom, z1),
        (x0, bottom, z1),
    ] {
        m.positions.push([x, y, z]);
    }
    for q in [[0, 1, 2, 3], [7, 6, 5, 4], [0, 4, 5, 1], [1, 5, 6, 2], [2, 6, 7, 3], [3, 7, 4, 0]] {
        m.indices.extend([b + q[0], b + q[1], b + q[2], b + q[0], b + q[2], b + q[3]]);
    }
}

/// A flower pot: a grey plate, a ring of red LEGO bricks, printed orange bricks on top and
/// a printed pink flower head. Four steps.
pub fn sample_model() -> Model {
    let p = |id: &str, c: u16, x: i32, y: i32, z: i32, r: u8| Placement::new(id, c, Cell::new(x, y, z), Rot(r));
    let mut m = Model { title: "Flower pot (sample)".into(), printable: true, ..Default::default() };
    m.placements = vec![
        // Step 1
        p("3958", 72, 0, 0, 0, 0),
        // Step 2
        p("3001", 4, 0, 1, 0, 0),
        p("3001", 4, 4, 1, 0, 1),
        p("3001", 4, 0, 1, 2, 0),
        p("3001", 4, 0, 1, 4, 0),
        p("3003", 4, 4, 1, 4, 0),
        // Step 3
        Placement::printed(brick("B001", vec![(0, 0), (1, 0), (2, 0), (0, 1), (0, 2)], 3), 25, Cell::new(0, 4, 0), Rot(0)),
        Placement::printed(brick("B002", vec![(0, 0), (1, 0), (0, 1), (1, 1), (0, 2), (1, 2)], 3), 25, Cell::new(3, 4, 3), Rot(0)),
        Placement::printed(brick("B002", vec![(0, 0), (1, 0), (0, 1), (1, 1), (0, 2), (1, 2)], 3), 25, Cell::new(1, 4, 3), Rot(1)),
        p("3039", 14, 3, 4, 0, 0),
    ];
    let mut flower = DetailMesh::default();
    box_mesh(&mut flower, 40.0, 20.0, 0.0, 8.0);
    box_mesh(&mut flower, 12.0, 12.0, -10.0, 0.0);
    box_mesh(&mut flower, 56.0, 36.0, -18.0, -10.0);
    m.meshes.insert("flower_head".into(), flower);
    let head = CustomDef {
        id: "D001".into(),
        kind: CustomKind::Detail { mesh: "flower_head".into(), name: "flower head".into() },
        colors: Default::default(),
        cells: vec![(0, 0), (1, 0)],
        plates: 3,
        studs: Vec::new(),
        units: Vec::new(),
        sockets: vec![(0, 0), (1, 0)],
    };
    m.placements.push(Placement::printed(head, 29, Cell::new(2, 7, 2), Rot(0)));
    m.steps = vec![vec![0], vec![1, 2, 3, 4, 5], vec![6, 7, 8, 9], vec![10]];
    m
}
