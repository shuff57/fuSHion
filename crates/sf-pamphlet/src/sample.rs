//! A small hand-made model with every kind of part, for tests and the example.

use sf_core::{Cell, CustomDef, CustomKind, DetailMesh, DetailSource, Kit, KitPart, KitPose, KitStep, Model, Placement, Rot};

fn brick(id: &str, cells: Vec<(u8, u8)>, plates: u8) -> CustomDef {
    CustomDef {
        id: id.to_string(),
        kind: CustomKind::Brick,
        studs: cells.clone(),
        units: Vec::new(),
        sockets: cells.clone(),
        cells,
        colors: Default::default(),
        trim: None,
        mesh: None,
        plates,
    }
}

/// A box mesh in the part frame (LDU, -Y up, origin at the top centre of the bounding
/// box): `w × d` LDU, from `top` (negative: above the bounding box) down to `bottom`.
pub(crate) fn box_mesh(m: &mut DetailMesh, w: f32, d: f32, top: f32, bottom: f32) {
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
        trim: None,
        mesh: None,
        cells: vec![(0, 0), (1, 0)],
        plates: 3,
        studs: Vec::new(),
        units: Vec::new(),
        sockets: vec![(0, 0), (1, 0)],
    };
    m.placements.push(Placement::printed(head, 29, Cell::new(2, 7, 2), Rot(0)));
    // Two roses: a kit of a hub, four petals and a centre, in two colour schemes.
    let kit = rose_kit();
    m.meshes.insert("rose".into(), assembled_mesh(&kit));
    m.kits.insert("rose".into(), kit);
    let rose = |id: &str, colors: &[(&str, u16)]| CustomDef {
        id: id.into(),
        kind: CustomKind::Detail { mesh: "rose".into(), name: "rose".into() },
        cells: vec![(0, 0), (1, 0)],
        colors: colors.iter().map(|&(r, c)| (r.to_string(), c)).collect(),
        plates: 3,
        studs: Vec::new(),
        units: Vec::new(),
        sockets: vec![(0, 0), (1, 0)],
        trim: None,
        mesh: None,
    };
    m.placements.push(Placement::printed(rose("D002", &[]), 4, Cell::new(0, 7, 0), Rot(0)));
    m.placements.push(Placement::printed(rose("D003", &[("centre", 25)]), 14, Cell::new(3, 7, 5), Rot(0)));
    m.steps = vec![vec![0], vec![1, 2, 3, 4, 5], vec![6, 7, 8, 9], vec![10], vec![11, 12]];
    m
}

fn about_y(deg: f32, position: [f32; 3]) -> KitPose {
    let (s, c) = deg.to_radians().sin_cos();
    KitPose { rotation: [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]], position }
}

/// A rose kit: a green hub, four petals ("main") and a yellow centre.
pub fn rose_kit() -> Kit {
    let part = |key: &str, label: &str, role: &str, mesh: DetailMesh, supports: bool, copies: Vec<KitPose>| KitPart {
        key: key.into(),
        label: label.into(),
        role: role.into(),
        mesh,
        source: DetailSource { source: serde_json::Value::Null },
        supports,
        copies,
    };
    let mut hub = DetailMesh::default();
    box_mesh(&mut hub, 40.0, 40.0, 0.0, 12.0);
    let mut petal = DetailMesh::default();
    // A petal leaning out along +x from its clip at the origin.
    box_mesh(&mut petal, 30.0, 16.0, -6.0, 0.0);
    for v in &mut petal.positions {
        v[0] += 13.0;
        v[1] -= v[0] * 0.25;
    }
    let mut centre = DetailMesh::default();
    box_mesh(&mut centre, 14.0, 14.0, -12.0, 0.0);
    // Each petal clips onto the hub's rim, 16 LDU out.
    let petals = (0..4)
        .map(|k| {
            let (s, c) = (90.0f32 * k as f32).to_radians().sin_cos();
            about_y(90.0 * k as f32, [16.0 * c, -2.0, -16.0 * s])
        })
        .collect();
    Kit {
        parts: vec![
            part("hub", "hub", "hub", hub, false, vec![about_y(0.0, [0.0; 3])]),
            part("petal", "petal", "main", petal, true, petals),
            part("centre", "centre", "centre", centre, false, vec![about_y(0.0, [0.0, -2.0, 0.0])]),
        ],
        steps: vec![
            KitStep { parts: vec![(0, 0)], note: String::new() },
            KitStep { parts: (0..4).map(|c| (1, c)).collect(), note: "Clip the four petals onto the hub.".into() },
            KitStep { parts: vec![(2, 0)], note: "Press the centre in.".into() },
        ],
        roles: vec![("main".into(), "Red".into()), ("hub".into(), "Green".into()), ("centre".into(), "Yellow".into())],
    }
}

/// The kit put together, as one mesh (the detail's quick-view mesh).
fn assembled_mesh(kit: &Kit) -> DetailMesh {
    let mut out = DetailMesh::default();
    for part in &kit.parts {
        for pose in &part.copies {
            let base = out.positions.len() as u32;
            for &v in &part.mesh.positions {
                let r = pose.rotation;
                out.positions.push([0, 1, 2].map(|i| r[i][0] * v[0] + r[i][1] * v[1] + r[i][2] * v[2] + pose.position[i]));
            }
            out.indices.extend(part.mesh.indices.iter().map(|i| i + base));
        }
    }
    out
}

/// A small bouquet: a brick vase, a printed hub on top, and three stems plugged into it. S01
/// is straight with a rose (a kit) and two leaves; S02 and S03 are the same bent stem with a
/// yellow bud and one leaf, so they share a build page.
pub fn bouquet_model() -> Model {
    use crate::stem::{compose, rot_y, rot_z};
    use sf_core::{StemAttach, StemBar, StemDef, StemPlacement};

    let p = |id: &str, c: u16, x: i32, y: i32, z: i32, r: u8| Placement::new(id, c, Cell::new(x, y, z), Rot(r));
    let mut m = Model { title: "Little bouquet (sample)".into(), printable: true, ..Default::default() };
    m.placements = vec![
        p("3958", 72, 0, 0, 0, 0),
        p("3001", 26, 0, 1, 0, 0),
        p("3001", 26, 4, 1, 0, 1),
        p("3001", 26, 0, 1, 2, 0),
        p("3001", 26, 0, 1, 4, 0),
        p("3003", 26, 4, 1, 4, 0),
        p("3001", 26, 0, 4, 0, 0),
        p("3001", 26, 4, 4, 0, 1),
        p("3001", 26, 0, 4, 2, 0),
        p("3001", 26, 0, 4, 4, 0),
        p("3003", 26, 4, 4, 4, 0),
    ];
    // The hub: a 4 × 4 puck, two plates thick, on top of the vase.
    let mut puck = DetailMesh::default();
    box_mesh(&mut puck, 80.0, 80.0, 0.0, 16.0);
    m.meshes.insert("hub".into(), puck);
    let hole = |x: f32, y: f32, dir: [f32; 3]| serde_json::json!({ "top": [x, y, 6.4], "dir": dir, "key": [1.0, 0.0, 0.0] });
    let cells: Vec<(u8, u8)> = (0..4).flat_map(|x| (0..4).map(move |z| (x, z))).collect();
    m.details.insert(
        "hub".into(),
        DetailSource {
            source: serde_json::json!({ "hub": {
                "cells": cells,
                "height": 6.4,
                "holes": [hole(16.0, 16.0, [0.0, 0.0, 1.0]), hole(6.0, 26.0, [-0.2, 0.2, 0.95]), hole(26.0, 6.0, [0.2, -0.2, 0.95])],
            }}),
        },
    );
    let hub = CustomDef {
        id: "D010".into(),
        kind: CustomKind::Detail { mesh: "hub".into(), name: "hub".into() },
        cells: cells.clone(),
        colors: Default::default(),
        plates: 2,
        studs: Vec::new(),
        units: Vec::new(),
        sockets: cells,
        trim: None,
        mesh: None,
    };
    m.placements.push(Placement::printed(hub, 0, Cell::new(1, 7, 1), Rot(0)));
    m.steps = vec![vec![0], (1..6).collect(), (6..11).collect(), vec![11]];

    // Heads and leaves.
    let kit = rose_kit();
    m.meshes.insert("rose".into(), assembled_mesh(&kit));
    m.kits.insert("rose".into(), kit);
    let mut bud = DetailMesh::default();
    box_mesh(&mut bud, 14.0, 14.0, -22.0, 0.0);
    box_mesh(&mut bud, 22.0, 22.0, -16.0, -6.0);
    m.meshes.insert("bud".into(), bud);
    let mut leaf = DetailMesh::default();
    box_mesh(&mut leaf, 30.0, 14.0, -2.0, 2.0);
    for v in &mut leaf.positions {
        v[0] += 19.0;
        v[1] -= v[0] * 0.35;
    }
    m.meshes.insert("leaf".into(), leaf);

    // Stems, in their own frames: heads on the tip bar, leaves on bar sections.
    let local = |rotation: [[f32; 3]; 3], position: [f32; 3]| KitPose { rotation, position };
    let tip = |def: &StemDef, below: f32| {
        let s = def.length - def.tip_bar;
        let a = def.bend * s / def.length;
        let b = def.point(s);
        // `below`: how far the design reaches under its origin (LDU), so it sits on the bar.
        local(rot_z(a), [b[0] - a.to_radians().sin() * below, b[1] + a.to_radians().cos() * below, b[2]])
    };
    let on_bar = |def: &StemDef, bar: usize, side: f32| {
        let at = def.bars[bar].at;
        let a = def.bend * at / def.length;
        let r = rot_z(a);
        let ry = rot_y(side);
        let rot = [0, 1, 2].map(|i| [0, 1, 2].map(|j| (0..3).map(|k| r[i][k] * ry[k][j]).sum::<f32>()));
        local(rot, def.point(at))
    };
    let att = |design: &str, color: u16, pose: KitPose, bar: Option<usize>| StemAttach {
        design: design.into(),
        color,
        colors: Default::default(),
        pose,
        bar,
        branch: None,
    };
    let place = |def: StemDef, hole: u32, at: [f32; 3], yaw: f32, tilt: f32, head: (&str, u16, f32), leaves: Vec<(usize, f32)>| {
        let rot = {
            let (a, b) = (rot_y(yaw), rot_z(tilt));
            [0, 1, 2].map(|i| [0, 1, 2].map(|j| (0..3).map(|k| a[i][k] * b[k][j]).sum::<f32>()))
        };
        let pose = local(rot, at);
        let head = Some(att(head.0, head.1, compose(&pose, &tip(&def, head.2)), None));
        let leaves = leaves.into_iter().map(|(b, side)| att("leaf", 10, compose(&pose, &on_bar(&def, b, side)), Some(b))).collect();
        StemPlacement { def, color: 2, pose, hub: "D010".into(), hole, head, branch_heads: Vec::new(), leaves }
    };
    let s01 = StemDef {
        id: "S01".into(),
        length: 140.0,
        bend: 0.0,
        tip_bar: 8.0,
        bars: vec![StemBar { at: 50.0, length: 10.0 }, StemBar { at: 85.0, length: 10.0 }],
        diameter: sf_core::stem::STEM_D,
        tip: None,
        branches: Vec::new(),
        style: "rose stem".into(),
    };
    let s02 = StemDef {
        id: "S02".into(),
        length: 110.0,
        bend: 20.0,
        tip_bar: 8.0,
        bars: vec![StemBar { at: 45.0, length: 10.0 }],
        diameter: sf_core::stem::STEM_D,
        tip: None,
        branches: Vec::new(),
        style: String::new(),
    };
    m.stems = vec![
        place(s01, 1, [60.0, -72.0, 60.0], 0.0, 0.0, ("rose", 4, 12.0), vec![(0, 0.0), (1, 180.0)]),
        place(s02.clone(), 2, [35.0, -72.0, 85.0], 225.0, 18.0, ("bud", 14, 0.0), vec![(0, 180.0)]),
        place(s02, 3, [85.0, -72.0, 35.0], 45.0, 18.0, ("bud", 14, 0.0), vec![(0, 180.0)]),
    ];
    m
}
