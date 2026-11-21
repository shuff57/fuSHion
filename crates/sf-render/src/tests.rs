use super::*;
use sf_core::{Cell, Rot};

const RED: ColorId = 4;
const BLUE: ColorId = 1;

fn opts(size: u32) -> RenderOptions {
    RenderOptions { view_size: size, edges: false, background: [0, 0, 0] }
}

fn model(ps: Vec<Placement>) -> Model {
    Model { placements: ps, ..Default::default() }
}

/// Bounding box `(x0, y0, x1, y1)` of pixels whose colour is "mostly" `rgb`.
fn bbox_of(img: &RgbaImage, pred: impl Fn([u8; 3]) -> bool) -> Option<(u32, u32, u32, u32)> {
    let mut b: Option<(u32, u32, u32, u32)> = None;
    for (x, y, p) in img.enumerate_pixels() {
        if pred([p[0], p[1], p[2]]) {
            b = Some(match b {
                None => (x, y, x, y),
                Some((a, c, d, e)) => (a.min(x), c.min(y), d.max(x), e.max(y)),
            });
        }
    }
    b
}

fn reddish(c: [u8; 3]) -> bool {
    c[0] > 40 && c[0] as u32 > 3 * (c[1] as u32 + 1) && c[0] > c[2]
}
fn bluish(c: [u8; 3]) -> bool {
    c[2] > 40 && c[2] > c[0] && c[2] > c[1]
}
fn cx(b: (u32, u32, u32, u32)) -> f32 {
    (b.0 + b.2) as f32 / 2.0
}
fn cy(b: (u32, u32, u32, u32)) -> f32 {
    (b.1 + b.3) as f32 / 2.0
}

#[test]
fn single_box_renders_in_the_middle() {
    let lib = MeshLibrary::procedural();
    let m = model(vec![Placement::new("3005", RED, Cell::new(0, 0, 0), Rot(0))]);
    let img = render_view(&m, &Palette::builtin(), &lib, View::Front, &opts(64));
    assert_eq!(img.dimensions(), (64, 64));
    let b = bbox_of(&img, |c| c != [0, 0, 0]).expect("something drawn");
    // Centred, with a margin, never touching the border.
    assert!(b.0 > 0 && b.1 > 0 && b.2 < 63 && b.3 < 63, "{b:?}");
    assert!((cx(b) - 31.5).abs() <= 2.0 && (cy(b) - 31.5).abs() <= 2.0, "{b:?}");
    assert!(reddish(img.get_pixel(32, 36).0[..3].try_into().unwrap()));
    // A 1x1 brick is 20 wide and 24 (+4 stud) tall: taller than wide.
    assert!(b.3 - b.1 > b.2 - b.0, "{b:?}");
}

#[test]
fn empty_model_is_background() {
    let lib = MeshLibrary::procedural();
    let o = RenderOptions { background: [9, 8, 7], ..opts(16) };
    let img = render_view(&Model::default(), &Palette::builtin(), &lib, View::Iso, &o);
    assert!(img.pixels().all(|p| p.0 == [9, 8, 7, 255]));
}

#[test]
fn view_conventions() {
    let lib = MeshLibrary::procedural();
    let pal = Palette::builtin();
    // Red at the origin, blue at +x (front row) and another blue at +z (back).
    let m = model(vec![
        Placement::new("3005", RED, Cell::new(0, 0, 0), Rot(0)),
        Placement::new("3005", BLUE, Cell::new(4, 0, 0), Rot(0)),
    ]);
    let front = render_view(&m, &pal, &lib, View::Front, &opts(96));
    let back = render_view(&m, &pal, &lib, View::Back, &opts(96));
    let (r, b) = (bbox_of(&front, reddish).unwrap(), bbox_of(&front, bluish).unwrap());
    assert!(cx(r) < cx(b), "front: +x is to the right");
    let (r, b) = (bbox_of(&back, reddish).unwrap(), bbox_of(&back, bluish).unwrap());
    assert!(cx(r) > cx(b), "back: +x is to the left");

    let m = model(vec![
        Placement::new("3005", RED, Cell::new(0, 0, 0), Rot(0)),
        Placement::new("3005", BLUE, Cell::new(0, 0, 4), Rot(0)),
    ]);
    let left = render_view(&m, &pal, &lib, View::Left, &opts(96));
    let (r, b) = (bbox_of(&left, reddish).unwrap(), bbox_of(&left, bluish).unwrap());
    assert!(cx(r) > cx(b), "left view (camera at -x): the front (-z) is on the right");
    let right = render_view(&m, &pal, &lib, View::Right, &opts(96));
    let (r, b) = (bbox_of(&right, reddish).unwrap(), bbox_of(&right, bluish).unwrap());
    assert!(cx(r) < cx(b), "right view: the front is on the left");
    let top = render_view(&m, &pal, &lib, View::Top, &opts(96));
    let (r, b) = (bbox_of(&top, reddish).unwrap(), bbox_of(&top, bluish).unwrap());
    assert!(cy(r) > cy(b), "top view: the front is at the bottom");

    // Stacked: blue above red is higher in the front view.
    let m = model(vec![
        Placement::new("3005", RED, Cell::new(0, 0, 0), Rot(0)),
        Placement::new("3005", BLUE, Cell::new(0, 3, 0), Rot(0)),
    ]);
    let front = render_view(&m, &pal, &lib, View::Front, &opts(96));
    let (r, b) = (bbox_of(&front, reddish).unwrap(), bbox_of(&front, bluish).unwrap());
    assert!(cy(b) < cy(r), "y is up");
}

#[test]
fn rotation_swaps_footprint_in_top_view() {
    let lib = MeshLibrary::procedural();
    let pal = Palette::builtin();
    // Include a 1x1 marker far away so both renders share the same fit.
    let mk = |rot| {
        model(vec![
            Placement::new("3010", RED, Cell::new(0, 0, 0), Rot(rot)),
            Placement::new("3024", BLUE, Cell::new(7, 0, 7), Rot(0)),
        ])
    };
    let a = bbox_of(&render_view(&mk(0), &pal, &lib, View::Top, &opts(128)), reddish).unwrap();
    let b = bbox_of(&render_view(&mk(1), &pal, &lib, View::Top, &opts(128)), reddish).unwrap();
    assert!(a.2 - a.0 > 3 * (a.3 - a.1), "rot 0: long along x {a:?}");
    assert!(b.3 - b.1 > 3 * (b.2 - b.0), "rot 1: long along z {b:?}");
}

#[test]
fn sheet_layout() {
    let lib = MeshLibrary::procedural();
    let m = model(vec![Placement::new("3001", RED, Cell::new(0, 0, 0), Rot(0))]);
    let img = render_sheet(&m, &Palette::builtin(), &lib, &View::ALL, &opts(50));
    assert_eq!(img.dimensions(), (3 * 50 + 4 * GAP, 2 * (50 + LABEL_H) + 3 * GAP));
}

#[test]
fn grid_render_draws_voxels() {
    let mut g = VoxelGrid::new([3, 3, 3]);
    g.set(Cell::new(1, 1, 1), Fill::Solid(RED));
    let img = render_grid(&g, &Palette::builtin(), View::Front, &opts(32));
    let b = bbox_of(&img, reddish).expect("voxel drawn");
    assert!((cx(b) - 15.5).abs() <= 1.5 && (cy(b) - 15.5).abs() <= 1.5, "{b:?}");
    // A cell is 20 wide, 8 tall.
    assert!(b.2 - b.0 > 2 * (b.3 - b.1));
}

fn printed(def: sf_core::CustomDef, color: ColorId, at: Cell) -> Placement {
    Placement::printed(def, color, at, Rot(0))
}

fn l_brick() -> sf_core::CustomDef {
    sf_core::CustomDef {
        id: "B001".into(),
        kind: sf_core::CustomKind::Brick,
        // An L: a row of four along x and one more at (0, 1).
        colors: Default::default(),
        cells: vec![(0, 0), (1, 0), (2, 0), (3, 0), (0, 1)],
        plates: 3,
        studs: vec![(0, 0), (1, 0), (2, 0), (3, 0)],
        units: vec![vec![(0, 0), (1, 0)], vec![(2, 0), (3, 0)]],
        sockets: vec![(0, 0), (1, 0), (2, 0), (3, 0), (0, 1)],
    }
}

/// A box detail 2 × 1 studs, 2 plates tall, in the catalog frame (top centre, -y up).
fn box_detail() -> (sf_core::CustomDef, sf_core::DetailMesh) {
    let (hx, hz, h) = (20.0f32, 10.0f32, 16.0f32);
    let p = |x: f32, y: f32, z: f32| [x * hx, y * h, z * hz];
    let positions = vec![
        p(-1., 0., -1.), p(1., 0., -1.), p(1., 1., -1.), p(-1., 1., -1.),
        p(-1., 0., 1.), p(1., 0., 1.), p(1., 1., 1.), p(-1., 1., 1.),
    ];
    let indices = vec![0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 2, 3, 7, 2, 7, 6, 1, 2, 6, 1, 6, 5, 0, 4, 7, 0, 7, 3];
    let design = sf_core::DetailDesign {
        name: "boxy".into(),
        label: "box".into(),
        cells: vec![(0, 0), (1, 0)],
        plates: 2,
        sockets: vec![(0, 0), (1, 0)],
        studs: vec![],
        mesh: sf_core::DetailMesh { positions, indices },
        kit: None,
        source: sf_core::DetailSource { source: serde_json::Value::Null },
    };
    (design.instance("D001"), design.mesh)
}

#[test]
fn printed_bricks_and_details_render() {
    let lib = MeshLibrary::procedural();
    let (detail, mesh) = box_detail();
    let mut m = model(vec![
        printed(l_brick(), RED, Cell::new(0, 0, 0)),
        // The detail sits on the studs of the L's row, at x 2..4.
        printed(detail, BLUE, Cell::new(2, 3, 0)),
    ]);
    m.meshes.insert("boxy".into(), mesh);
    let top = render_view(&m, &Palette::builtin(), &lib, View::Top, &opts(96));
    let red = bbox_of(&top, reddish).expect("brick drawn");
    let blue = bbox_of(&top, bluish).expect("detail drawn");
    // From the top (front at the bottom), the detail covers the right half of the row.
    assert!(cx(blue) > cx(red), "{blue:?} vs {red:?}");
    // The L's notch at (1..4, 1) is empty: no red there.
    let (x0, y0, x1, y1) = red;
    let notch = (x0 + (x1 - x0) * 5 / 8, y0 + (y1 - y0) / 6);
    assert!(!reddish(top.get_pixel(notch.0, notch.1).0[..3].try_into().unwrap()), "notch filled at {notch:?} in {red:?}");
    // The front view shows the detail standing on the brick.
    let front = render_view(&m, &Palette::builtin(), &lib, View::Front, &opts(96));
    let (r, b) = (bbox_of(&front, reddish).unwrap(), bbox_of(&front, bluish).unwrap());
    assert!(cy(b) < cy(r), "detail above the brick: {b:?} {r:?}");
    // A detail without its mesh is skipped, not a panic.
    let mut bare = m.clone();
    bare.meshes.clear();
    let img = render_view(&bare, &Palette::builtin(), &lib, View::Top, &opts(32));
    assert!(bbox_of(&img, bluish).is_none());
    // Thumbnails of a detail find the model's mesh.
    let thumb = render_part_in(&m, &m.placements[1], &Palette::builtin(), &lib, View::Iso, &opts(48));
    assert!(bbox_of(&thumb, bluish).is_some());
}

#[test]
fn printed_brick_sheet_png() {
    // Not an assertion test: writes a review image when FUSHION_RENDER_PNG is set.
    let Ok(path) = std::env::var("FUSHION_RENDER_PNG") else { return };
    let lib = MeshLibrary::procedural();
    let (detail, mesh) = box_detail();
    let mut tri = l_brick();
    tri.id = "B002".into();
    tri.cells = (0..5).map(|x| (x, 0)).collect();
    tri.studs = tri.cells.clone();
    tri.units = vec![vec![(0, 0), (1, 0)], vec![(2, 0), (3, 0), (4, 0)]];
    tri.sockets = tri.cells.clone();
    let mut m = model(vec![
        printed(l_brick(), RED, Cell::new(0, 0, 0)),
        printed(detail, BLUE, Cell::new(2, 3, 0)),
        printed(tri, 14, Cell::new(0, 0, 3)),
    ]);
    m.meshes.insert("boxy".into(), mesh);
    let o = RenderOptions { view_size: 320, edges: true, background: [236, 238, 241] };
    render_sheet(&m, &Palette::builtin(), &lib, &[View::Iso, View::Top, View::Front, View::Right], &o).save(path).unwrap();
}

// ---- Kits -------------------------------------------------------------------------------

/// A box mesh from `lo` to `hi` (LDU), outward faces.
fn box_mesh(lo: [f32; 3], hi: [f32; 3]) -> sf_core::DetailMesh {
    let c = |i: u8| [if i & 1 == 0 { lo[0] } else { hi[0] }, if i & 2 == 0 { lo[1] } else { hi[1] }, if i & 4 == 0 { lo[2] } else { hi[2] }];
    let positions = (0..8).map(c).collect();
    // Corner index bits: x = 1, y = 2, z = 4.
    let indices = vec![0, 2, 1, 1, 2, 3, 4, 5, 6, 5, 7, 6, 0, 1, 4, 1, 5, 4, 2, 6, 3, 3, 6, 7, 0, 4, 2, 2, 4, 6, 1, 3, 5, 3, 7, 5];
    sf_core::DetailMesh { positions, indices }
}

fn yaw(deg: f32) -> [[f32; 3]; 3] {
    let (s, c) = deg.to_radians().sin_cos();
    [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]]
}

/// A flower: a yellow hub ("centre") and four petals ("main") around it, clipped on in two
/// steps after the hub.
fn flower_kit() -> sf_core::Kit {
    let src = sf_core::DetailSource { source: serde_json::Value::Null };
    let hub = sf_core::KitPart {
        key: "hub".into(),
        label: "hub".into(),
        role: "centre".into(),
        mesh: box_mesh([-8.0, -6.0, -8.0], [8.0, 0.0, 8.0]),
        source: src.clone(),
        supports: false,
        copies: vec![sf_core::KitPose { rotation: yaw(0.0), position: [0.0, 0.0, 0.0] }],
    };
    // A petal points along +x from its clip at the origin.
    let petal = sf_core::KitPart {
        key: "petal".into(),
        label: "petal".into(),
        role: "main".into(),
        mesh: box_mesh([0.0, -3.0, -6.0], [24.0, 0.0, 6.0]),
        source: src,
        supports: false,
        copies: (0..4)
            .map(|k| {
                let r = yaw(90.0 * k as f32);
                let p = [8.0 * r[0][0], -2.0, 8.0 * r[2][0]];
                sf_core::KitPose { rotation: r, position: p }
            })
            .collect(),
    };
    sf_core::Kit {
        parts: vec![hub, petal],
        steps: vec![
            sf_core::KitStep { parts: vec![(0, 0)], note: "hub".into() },
            sf_core::KitStep { parts: vec![(1, 0), (1, 2)], note: String::new() },
            sf_core::KitStep { parts: vec![(1, 1), (1, 3)], note: String::new() },
        ],
        roles: vec![("main".into(), "Red".into()), ("centre".into(), "Yellow".into())],
    }
}

fn yellowish(c: [u8; 3]) -> bool {
    c[0] > 120 && c[1] > 100 && c[2] < 80
}

fn count(img: &RgbaImage, pred: impl Fn([u8; 3]) -> bool) -> usize {
    img.pixels().filter(|p| pred([p[0], p[1], p[2]])).count()
}

#[test]
fn kit_details_draw_their_parts_in_role_colours() {
    let lib = MeshLibrary::procedural();
    let kit = flower_kit();
    let design = sf_core::DetailDesign {
        name: "flower".into(),
        label: "flower".into(),
        cells: vec![(0, 0), (1, 0), (0, 1), (1, 1)],
        plates: 1,
        sockets: vec![(0, 0), (1, 0)],
        studs: vec![],
        mesh: box_mesh([-20.0, -8.0, -20.0], [20.0, 8.0, 20.0]),
        source: sf_core::DetailSource { source: serde_json::Value::Null },
        kit: Some(kit.clone()),
    };
    let mut m = model(vec![printed(design.instance("D001"), RED, Cell::new(0, 0, 0))]);
    m.meshes.insert("flower".into(), design.mesh.clone());
    m.kits.insert("flower".into(), kit);
    let top = render_view(&m, &Palette::builtin(), &lib, View::Top, &opts(96));
    assert!(count(&top, reddish) > 50, "petals in the placement's colour");
    assert!(count(&top, yellowish) > 20, "hub in its role's default colour");
    // An override recolours the hub's role.
    if let Some(d) = m.placements[0].custom.as_deref_mut() {
        d.colors.insert("centre".into(), BLUE);
    }
    let top = render_view(&m, &Palette::builtin(), &lib, View::Top, &opts(96));
    assert_eq!(count(&top, yellowish), 0);
    assert!(count(&top, bluish) > 20);
    // Without its kit the detail falls back to the assembled mesh, all in the main colour.
    m.kits.clear();
    let top = render_view(&m, &Palette::builtin(), &lib, View::Top, &opts(96));
    assert!(count(&top, bluish) == 0 && count(&top, reddish) > 50);
}

#[test]
fn kit_steps_ghost_earlier_parts_and_hide_later_ones() {
    let lib = MeshLibrary::procedural();
    let kit = flower_kit();
    let pal = Palette::builtin();
    let colors = kit_role_colors(&kit, RED, &BTreeMap::new(), &pal);
    let o = opts(96);
    let s0 = render_kit_step(&kit, 0, &colors, &pal, &lib, View::Top, &o, true);
    assert!(count(&s0, yellowish) > 20 && count(&s0, reddish) == 0, "step 0: the hub only");
    let s1 = render_kit_step(&kit, 1, &colors, &pal, &lib, View::Top, &o, true);
    let s2 = render_kit_step(&kit, 2, &colors, &pal, &lib, View::Top, &o, true);
    assert_eq!(count(&s1, yellowish), 0, "the hub is ghosted after its step");
    let (r1, r2) = (count(&s1, reddish), count(&s2, reddish));
    assert!(r1 > 50 && r2 > 50 && (r1 as i64 - r2 as i64).abs() < r1 as i64 / 4, "two new petals each step: {r1} {r2}");
    // Every step is framed on the whole kit: the hub sits in the same place.
    let pale = |c: [u8; 3]| c.iter().all(|&v| v > 150) && c.iter().max().unwrap() - c.iter().min().unwrap() < 40;
    let g1 = bbox_of(&s1, pale).expect("ghosted hub");
    let g2 = bbox_of(&s2, pale).expect("ghosted hub");
    // The ghost is pale and greyish, not a lighter yellow.
    let gh = ghost_rgb(pal.rgb(14));
    assert!(pale(gh), "{gh:?}");
    assert!((cx(g1) - cx(g2)).abs() < 2.0 && (cy(g1) - cy(g2)).abs() < 2.0);
    // Without ghosting earlier parts keep their colour.
    let plain = render_kit_step(&kit, 2, &colors, &pal, &lib, View::Top, &o, false);
    assert!(count(&plain, yellowish) > 0 && count(&plain, reddish) > r2);
}

#[test]
fn exploded_kits_spread_and_parts_render_alone() {
    let lib = MeshLibrary::procedural();
    let kit = flower_kit();
    let pal = Palette::builtin();
    let colors = kit_role_colors(&kit, RED, &BTreeMap::new(), &pal);
    let o = opts(128);
    let assembled = render_kit_exploded(&kit, &colors, &pal, &lib, View::Top, &o, 0.0);
    let exploded = render_kit_exploded(&kit, &colors, &pal, &lib, View::Top, &o, 1.5);
    // Exploded, the picture is refitted, so the parts get smaller and gaps open between the
    // hub and the petals: fewer red pixels, and background shows between hub and petals.
    let (ra, re) = (count(&assembled, reddish), count(&exploded, reddish));
    assert!(re < ra, "{re} {ra}");
    let hub = bbox_of(&exploded, yellowish).unwrap();
    let right_of_hub = (hub.2 + 2, ((hub.1 + hub.3) / 2));
    assert_eq!(exploded.get_pixel(right_of_hub.0, right_of_hub.1).0[..3], [0, 0, 0], "a gap beside the hub");
    let thumb = render_kit_part(&kit.parts[1], RED, &pal, &lib, View::Iso, &opts(48));
    assert!(count(&thumb, reddish) > 100);
}

#[test]
fn kit_sample_png() {
    let Ok(path) = std::env::var("FUSHION_KIT_PNG") else { return };
    let lib = MeshLibrary::procedural();
    let kit = flower_kit();
    let pal = Palette::builtin();
    let colors = kit_role_colors(&kit, RED, &BTreeMap::new(), &pal);
    let o = RenderOptions { view_size: 256, edges: true, background: [255, 255, 255] };
    let imgs = [
        render_kit_exploded(&kit, &colors, &pal, &lib, View::Iso, &o, 1.2),
        render_kit_step(&kit, 0, &colors, &pal, &lib, View::Iso, &o, true),
        render_kit_step(&kit, 1, &colors, &pal, &lib, View::Iso, &o, true),
        render_kit_step(&kit, 2, &colors, &pal, &lib, View::Iso, &o, true),
        render_kit_part(&kit.parts[1], RED, &pal, &lib, View::Iso, &o),
    ];
    let mut sheet = RgbaImage::from_pixel(256 * imgs.len() as u32, 256, image::Rgba([255, 255, 255, 255]));
    for (i, img) in imgs.iter().enumerate() {
        image::imageops::overlay(&mut sheet, img, 256 * i as i64, 0);
    }
    sheet.save(path).unwrap();
}
