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
        trim: None,
        mesh: None,
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
        mount: None,
        stem: None,
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
        mount: None,
        stem: None,
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

// ---- Trimmed (curved) printed bricks ----------------------------------------------------

/// A trimmed 2 × 2 brick, 3 plates: vertical sides from the bottom (y = 24) up to a domed
/// top that rises 8 LDU at the centre and meets the top plane (y = 0) at the rim. Indexed,
/// outward faces, in the catalog frame (top centre, -y up).
fn dome_mesh(n: usize) -> sf_core::DetailMesh {
    let (h, bottom) = (8.0f32, 24.0f32);
    let at = |i: usize| -20.0 + 40.0 * i as f32 / n as f32;
    let top = |x: f32, z: f32| -h * (1.0 - (x / 20.0).powi(2)) * (1.0 - (z / 20.0).powi(2));
    let mut positions = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    // Top grid.
    for j in 0..=n {
        for i in 0..=n {
            positions.push([at(i), top(at(i), at(j)), at(j)]);
        }
    }
    let id = |i: usize, j: usize| (j * (n + 1) + i) as u32;
    for j in 0..n {
        for i in 0..n {
            // Outward = -y (up): wind so (b-a)×(c-a) points to -y.
            indices.extend([id(i, j), id(i, j + 1), id(i + 1, j), id(i + 1, j), id(i, j + 1), id(i + 1, j + 1)]);
        }
    }
    // Sides: a strip along each edge of the grid down to the bottom.
    let side = |rim: Vec<[f32; 3]>, out: [f32; 3], positions: &mut Vec<[f32; 3]>, indices: &mut Vec<u32>| {
        for w in rim.windows(2) {
            let (a, b) = (w[0], w[1]);
            let (ad, bd) = ([a[0], bottom, a[2]], [b[0], bottom, b[2]]);
            let base = positions.len() as u32;
            positions.extend([a, b, bd, ad]);
            let nrm = {
                let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                let v = [bd[0] - a[0], bd[1] - a[1], bd[2] - a[2]];
                [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]]
            };
            let flip = nrm[0] * out[0] + nrm[1] * out[1] + nrm[2] * out[2] < 0.0;
            if flip {
                indices.extend([base, base + 2, base + 1, base, base + 3, base + 2]);
            } else {
                indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
            }
        }
    };
    let row = |pick: &dyn Fn(usize) -> (usize, usize), positions: &Vec<[f32; 3]>| -> Vec<[f32; 3]> {
        (0..=n).map(|k| { let (i, j) = pick(k); positions[id(i, j) as usize] }).collect()
    };
    let rims = [
        (row(&|k| (k, 0), &positions), [0.0, 0.0, -1.0]),
        (row(&|k| (k, n), &positions), [0.0, 0.0, 1.0]),
        (row(&|k| (0, k), &positions), [-1.0, 0.0, 0.0]),
        (row(&|k| (n, k), &positions), [1.0, 0.0, 0.0]),
    ];
    for (rim, out) in rims {
        side(rim, out, &mut positions, &mut indices);
    }
    // Bottom.
    let base = positions.len() as u32;
    positions.extend([[-20.0, bottom, -20.0], [20.0, bottom, -20.0], [20.0, bottom, 20.0], [-20.0, bottom, 20.0]]);
    indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    sf_core::DetailMesh { positions, indices }
}

fn trimmed_brick(mesh: Option<&str>) -> sf_core::CustomDef {
    let cells = vec![(0, 0), (1, 0), (0, 1), (1, 1)];
    sf_core::CustomDef {
        id: "B007".into(),
        kind: sf_core::CustomKind::Brick,
        cells: cells.clone(),
        colors: Default::default(),
        plates: 3,
        studs: vec![],
        units: vec![],
        sockets: cells,
        trim: None,
        mesh: mesh.map(str::to_string),
    }
}

#[test]
fn trimmed_bricks_draw_their_curved_mesh() {
    let lib = MeshLibrary::procedural();
    let mut m = model(vec![printed(trimmed_brick(Some("dome")), RED, Cell::new(0, 0, 0))]);
    m.meshes.insert("dome".into(), dome_mesh(16));
    let front = render_view(&m, &Palette::builtin(), &lib, View::Front, &opts(96));
    let b = bbox_of(&front, reddish).expect("drawn");
    // The silhouette is domed: the top edge is higher in the middle than at the sides.
    let top_at = |x: u32| (b.1..=b.3).find(|&y| reddish(front.get_pixel(x, y).0[..3].try_into().unwrap()));
    let (mid, side) = (top_at((b.0 + b.2) / 2).unwrap(), top_at(b.0 + 2).unwrap());
    assert!(mid + 4 < side, "dome top at {mid}, sides at {side}");
    // Smooth shading: the iso view of the dome shows many shades of red, not a few facets.
    let iso = render_view(&m, &Palette::builtin(), &lib, View::Iso, &opts(128));
    let shades: std::collections::HashSet<[u8; 3]> =
        iso.pixels().map(|p| [p[0], p[1], p[2]]).filter(|&c| reddish(c)).collect();
    assert!(shades.len() > 30, "only {} shades", shades.len());
    // Without the mesh it is the plain 2 × 2 block (flat top).
    let plain = model(vec![printed(trimmed_brick(None), RED, Cell::new(0, 0, 0))]);
    let pf = render_view(&plain, &Palette::builtin(), &lib, View::Front, &opts(96));
    let pb = bbox_of(&pf, reddish).unwrap();
    let ptop = |x: u32| (pb.1..=pb.3).find(|&y| reddish(pf.get_pixel(x, y).0[..3].try_into().unwrap())).unwrap();
    assert!((ptop((pb.0 + pb.2) / 2) as i32 - ptop(pb.0 + 2) as i32).abs() <= 1);
    // A missing mesh falls back to the plain block rather than vanishing.
    let mut missing = m.clone();
    missing.meshes.clear();
    let img = render_view(&missing, &Palette::builtin(), &lib, View::Front, &opts(48));
    assert!(bbox_of(&img, reddish).is_some());
    // Thumbnails carry the mesh along.
    let thumb = render_part_in(&m, &m.placements[0], &Palette::builtin(), &lib, View::Front, &opts(96));
    let tb = bbox_of(&thumb, reddish).unwrap();
    let ttop = |x: u32| (tb.1..=tb.3).find(|&y| reddish(thumb.get_pixel(x, y).0[..3].try_into().unwrap())).unwrap();
    assert!(ttop((tb.0 + tb.2) / 2) + 4 < ttop(tb.0 + 2), "thumbnail lost the dome");
}

#[test]
fn trimmed_meshes_are_cached_by_content() {
    let lib = MeshLibrary::procedural();
    let mut m = model(vec![printed(trimmed_brick(Some("dome")), RED, Cell::new(0, 0, 0))]);
    m.meshes.insert("dome".into(), dome_mesh(8));
    let (k1, a) = lib.placement_mesh(&m.placements[0], &m).unwrap();
    // Another model reusing the key name with the same content shares the cached mesh...
    let mut m2 = m.clone();
    m2.placements[0].part = "B099".into();
    let (k2, b) = lib.placement_mesh(&m2.placements[0], &m2).unwrap();
    assert!(k1.starts_with("printed-trimmed:") && k1 == k2 && Arc::ptr_eq(&a, &b));
    // ...and different content under the same name gets its own.
    m2.meshes.insert("dome".into(), dome_mesh(12));
    let (k3, c) = lib.placement_mesh(&m2.placements[0], &m2).unwrap();
    assert!(k3 != k1 && !Arc::ptr_eq(&a, &c));
}

#[test]
fn curved_sample_png() {
    let Ok(path) = std::env::var("FUSHION_CURVED_PNG") else { return };
    let lib = MeshLibrary::procedural();
    let mut m = model(vec![
        printed(trimmed_brick(Some("dome")), RED, Cell::new(0, 0, 0)),
        printed(trimmed_brick(None), BLUE, Cell::new(3, 0, 0)),
    ]);
    m.meshes.insert("dome".into(), dome_mesh(24));
    let o = RenderOptions { view_size: 300, edges: true, background: [236, 238, 241] };
    render_sheet(&m, &Palette::builtin(), &lib, &[View::Iso, View::Front], &o).save(path).unwrap();
}

// ---- Stems (bouquets) --------------------------------------------------------------------

const GREEN: ColorId = 2;
const YELLOW: ColorId = 14;

fn rot_z(deg: f32) -> [[f32; 3]; 3] {
    let (s, c) = deg.to_radians().sin_cos();
    [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]]
}

fn mat3(a: [[f32; 3]; 3], b: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| a[i][k] * b[k][j]).sum()))
}

/// The design's lowest point (largest y) below its origin, LDU.
fn bottom_y(d: &sf_core::DetailMesh) -> f32 {
    d.positions.iter().map(|p| p[1]).fold(f32::MIN, f32::max)
}

/// A head on the stem's tip (`branch` None) or a branch's: the design's bottom on the tip,
/// its -Y along the tip.
fn head_at(stem: &sf_core::StemPlacement, branch: Option<usize>, design: &str, mesh: &sf_core::DetailMesh, color: ColorId) -> sf_core::StemAttach {
    let d = &stem.def;
    let (tip, t) = match branch {
        None => (d.point(d.length), d.tangent(d.length)),
        Some(i) => (d.branch_point(i, d.branches[i].length), d.branch_tangent(i, d.branches[i].length)),
    };
    let r = rot_z(t[0].atan2(-t[1]).to_degrees());
    let local = KitPose { rotation: r, position: sub(tip, mat_vec(r, [0.0, bottom_y(mesh), 0.0])) };
    sf_core::StemAttach {
        design: design.into(),
        color,
        colors: Default::default(),
        pose: pose_then(&stem.pose, &local),
        bar: None,
        branch,
    }
}

fn head_on(stem: &sf_core::StemPlacement, design: &str, mesh: &sf_core::DetailMesh, color: ColorId) -> sf_core::StemAttach {
    head_at(stem, None, design, mesh, color)
}

/// A leaf clipped on bar `bar`, turned `side` degrees about the stem and tilted out.
fn leaf_on(stem: &sf_core::StemPlacement, bar: usize, side: f32, design: &str, color: ColorId) -> sf_core::StemAttach {
    let d = &stem.def;
    let s = d.bars[bar].at;
    let along = rot_z(d.bend * s / d.length);
    let spin = yaw(side);
    let tilt = rot_z(-30.0);
    let local = KitPose { rotation: mat3(along, mat3(spin, tilt)), position: d.point(s) };
    sf_core::StemAttach {
        design: design.into(),
        color,
        colors: Default::default(),
        pose: pose_then(&stem.pose, &local),
        bar: Some(bar),
        branch: None,
    }
}

struct Designs {
    /// (name, assembled mesh, kit) for two heads and a leaf.
    heads: [(String, sf_core::DetailMesh, Option<sf_core::Kit>); 2],
    leaf: (String, sf_core::DetailMesh, Option<sf_core::Kit>),
}

fn stub_designs() -> Designs {
    // A plain box head, the flower kit (assembled mesh: a disc-ish box), and a flat leaf
    // reaching out along +x from its clip.
    let flower = flower_kit();
    Designs {
        heads: [
            ("bud".into(), box_mesh([-12.0, -4.0, -12.0], [12.0, 20.0, 12.0]), None),
            ("flower".into(), box_mesh([-32.0, -6.0, -32.0], [32.0, 2.0, 32.0]), Some(flower)),
        ],
        leaf: ("leaf".into(), box_mesh([0.0, -2.0, -8.0], [50.0, 2.0, 8.0]), None),
    }
}

/// A brick vase (three 2 × 4 bricks), a printed hub on it, and three stems: straight with
/// a box head, bent with a kit head, and one with two leaves.
fn bouquet(ds: &Designs) -> Model {
    let mut m = model(
        (0..3).map(|i| Placement::new("3001", BLUE, Cell::new(0, 3 * i, 0), Rot(0))).collect(),
    );
    let hub_mesh = box_mesh([-20.0, 0.0, -20.0], [20.0, 8.0, 20.0]);
    let hub = sf_core::DetailDesign {
        name: "hub".into(),
        label: "hub".into(),
        cells: vec![(0, 0), (1, 0), (0, 1), (1, 1)],
        plates: 1,
        sockets: vec![(0, 0), (1, 0), (0, 1), (1, 1)],
        studs: vec![],
        mesh: hub_mesh.clone(),
        source: sf_core::DetailSource { source: serde_json::Value::Null },
        kit: None,
        mount: None,
        stem: None,
    };
    let hub_pl = printed(hub.instance("D001"), GREEN, Cell::new(1, 9, 0));
    let (top, _) = hub_pl.ldraw_transform();
    m.placements.push(hub_pl);
    m.meshes.insert("hub".into(), hub_mesh);
    for (name, mesh, kit) in ds.heads.iter().chain([&ds.leaf]) {
        m.meshes.insert(name.clone(), mesh.clone());
        if let Some(k) = kit {
            m.kits.insert(name.clone(), k.clone());
        }
    }
    let stem = |def: sf_core::StemDef, lean: f32, turn: f32, hole: u32| sf_core::StemPlacement {
        def,
        color: GREEN,
        pose: KitPose { rotation: mat3(yaw(turn), rot_z(lean)), position: top },
        hub: "D001".into(),
        hole,
        head: None,
        branch_heads: vec![],
        leaves: vec![],
    };
    let def = |id: &str, length: f32, bend: f32, bars: Vec<sf_core::StemBar>| sf_core::StemDef {
        id: id.into(),
        length,
        bend,
        tip_bar: 8.0,
        bars,
        diameter: sf_core::stem::STEM_D,
        tip: None,
        branches: vec![],
        style: String::new(),
    };
    let mut straight = stem(def("S01", 120.0, 0.0, vec![]), 0.0, 0.0, 1);
    straight.head = Some(head_on(&straight, &ds.heads[0].0, &ds.heads[0].1, YELLOW));
    let mut bent = stem(def("S02", 140.0, 35.0, vec![]), 10.0, 0.0, 2);
    bent.head = Some(head_on(&bent, &ds.heads[1].0, &ds.heads[1].1, RED));
    let bars = vec![sf_core::StemBar { at: 40.0, length: 8.0 }, sf_core::StemBar { at: 70.0, length: 8.0 }];
    let mut leafy = stem(def("S03", 110.0, 15.0, bars), 12.0, 180.0, 3);
    leafy.leaves = vec![leaf_on(&leafy, 0, 0.0, &ds.leaf.0, GREEN), leaf_on(&leafy, 1, 180.0, &ds.leaf.0, GREEN)];
    let branch = |at: f32, angle: f32, length: f32, bend: f32, tip_bar: f32, tip: Option<sf_core::StemTip>| sf_core::StemBranch {
        at,
        angle,
        length,
        bend,
        diameter: None,
        tip_bar,
        tip,
    };
    // A daisy sprig: a bare main stem forking into three branches, each with a daisy.
    let mut sprig_def = def("S04", 90.0, -10.0, vec![]);
    sprig_def.tip_bar = 6.0;
    sprig_def.style = "daisy sprig".into();
    sprig_def.branches = vec![branch(55.0, 35.0, 50.0, -10.0, 6.0, None), branch(70.0, -40.0, 40.0, 10.0, 6.0, None)];
    let mut sprig = stem(sprig_def, -18.0, 60.0, 4);
    let (name, mesh) = (&ds.heads[1].0, &ds.heads[1].1);
    sprig.head = Some(head_on(&sprig, name, mesh, WHITE));
    sprig.branch_heads = (0..2).map(|i| head_at(&sprig, Some(i), name, mesh, WHITE)).collect();
    // Lavender: a thin brown stem branching into sprigs, every end a bud.
    let bud = Some(sf_core::StemTip::Bud { length: 16.0, width: 4.5 });
    let mut lav_def = def("S05", 130.0, 8.0, vec![]);
    lav_def.tip_bar = 0.0;
    lav_def.diameter = 4.0;
    lav_def.tip = bud;
    lav_def.style = "lavender sprig".into();
    lav_def.branches = vec![
        branch(80.0, 25.0, 40.0, -8.0, 0.0, bud),
        branch(90.0, -25.0, 35.0, 8.0, 0.0, bud),
        branch(100.0, 20.0, 25.0, -5.0, 0.0, bud),
        branch(108.0, -20.0, 20.0, 5.0, 0.0, bud),
    ];
    let mut lavender = stem(lav_def, 15.0, -120.0, 5);
    lavender.color = BROWN;
    // Grass: a fan of blades, no head.
    let blade = |l: f32| Some(sf_core::StemTip::Blade { length: l, width: 6.0 });
    let mut grass_def = def("S06", 50.0, 0.0, vec![]);
    grass_def.tip_bar = 0.0;
    grass_def.tip = blade(60.0);
    grass_def.style = "grass".into();
    grass_def.branches = vec![
        branch(10.0, 30.0, 30.0, 20.0, 0.0, blade(50.0)),
        branch(10.0, -30.0, 30.0, -20.0, 0.0, blade(50.0)),
        branch(20.0, 50.0, 20.0, 25.0, 0.0, blade(35.0)),
        branch(20.0, -50.0, 20.0, -25.0, 0.0, blade(35.0)),
    ];
    let grass = stem(grass_def, 8.0, 150.0, 6);
    m.stems = vec![straight, bent, leafy, sprig, lavender, grass];
    m
}

const WHITE: ColorId = 15;
const BROWN: ColorId = 70;

fn greenish(c: [u8; 3]) -> bool {
    c[1] > 40 && c[1] as u32 > c[0] as u32 + 20 && c[1] as u32 > c[2] as u32 + 10
}

#[test]
fn stems_render_with_their_heads_and_leaves_above_the_vase() {
    let lib = MeshLibrary::procedural();
    let ds = stub_designs();
    let m = bouquet(&ds);
    let pal = Palette::builtin();
    let front = render_view(&m, &pal, &lib, View::Front, &opts(160));
    // The vase alone fills the frame; with stems it is framed small at the bottom.
    let vase = bbox_of(&front, bluish).expect("vase");
    let all = bbox_of(&front, |c| c != [0, 0, 0]).unwrap();
    assert!(all.1 + 40 < vase.1, "stems reach well above the vase: {all:?} {vase:?}");
    assert!(all.1 > 0 && all.3 < 159, "everything framed: {all:?}");
    assert!(count(&front, greenish) > 100, "stems, hub and leaves drawn");
    assert!(count(&front, yellowish) > 30, "the box head");
    let brownish = |c: [u8; 3]| c[0] > 50 && c[0] as u32 > c[1] as u32 + 15 && c[1] > c[2] && c[0] < 200;
    assert!(count(&front, brownish) > 30, "the lavender stem");
    // The kit head draws its parts in their role colours: red petals, a yellow hub part.
    let top = render_view(&m, &pal, &lib, View::Top, &opts(160));
    assert!(count(&top, reddish) > 30);
    // Without the designs' meshes the stems still draw, the heads don't.
    let mut bare = m.clone();
    bare.meshes.retain(|k, _| k == "hub");
    bare.kits.clear();
    let img = render_view(&bare, &pal, &lib, View::Front, &opts(160));
    assert_eq!(count(&img, yellowish), 0);
    assert!(count(&img, greenish) > 50);
    // stems_bounds covers the heads, far above the vase's top (y = -72 at the hub's foot).
    let (lo, _) = stems_bounds(&m).unwrap();
    assert!(lo[1] < -72.0 - 120.0 * 2.5, "{lo:?}");
}

#[test]
fn stem_thumbnails() {
    let lib = MeshLibrary::procedural();
    let ds = stub_designs();
    let m = bouquet(&ds);
    let pal = Palette::builtin();
    // Laid flat as printed, seen from the top: a long thin green shape, base at the bottom.
    let flat = render_stem(&m.stems[0].def, GREEN, &pal, &lib, View::Top, &opts(128));
    let b = bbox_of(&flat, greenish).expect("stem drawn");
    assert!(b.3 - b.1 > 8 * (b.2 - b.0), "long and thin from above: {b:?}");
    // ... and lying flat: from the side it is long and low.
    let side = render_stem(&m.stems[0].def, GREEN, &pal, &lib, View::Left, &opts(128));
    let sb = bbox_of(&side, greenish).unwrap();
    assert!(sb.2 - sb.0 > 8 * (sb.3 - sb.1), "{sb:?}");
    // The assembly stands upright in the stem's own frame: head above the stem.
    let asm = render_stem_assembly(&m.stems[0], &m, &pal, &lib, View::Front, &opts(128));
    let (head, stem) = (bbox_of(&asm, yellowish).unwrap(), bbox_of(&asm, greenish).unwrap());
    assert!(head.3 <= stem.1 + 4 && cy(head) < cy(stem), "{head:?} {stem:?}");
    let leafy = render_stem_assembly(&m.stems[2], &m, &pal, &lib, View::Front, &opts(128));
    let lb = bbox_of(&leafy, greenish).unwrap();
    let mut bare = m.stems[2].clone();
    bare.leaves.clear();
    let plain = render_stem_assembly(&bare, &m, &pal, &lib, View::Front, &opts(128));
    let pb = bbox_of(&plain, greenish).unwrap();
    assert!(lb.2 - lb.0 > pb.2 - pb.0 + 6, "leaves reach out: {lb:?} vs {pb:?}");
}

#[test]
fn pose_helpers_invert() {
    let p = KitPose { rotation: mat3(yaw(30.0), rot_z(20.0)), position: [5.0, -7.0, 3.0] };
    let id = pose_then(&p, &pose_inverse(&p));
    for i in 0..3 {
        assert!(id.position[i].abs() < 1e-4);
        for j in 0..3 {
            assert!((id.rotation[i][j] - if i == j { 1.0 } else { 0.0 }).abs() < 1e-5);
        }
    }
}

/// Writes the bouquet renders when FUSHION_STEMS_DIR is set, with the real flower designs
/// from FUSHION_STEMS_DESIGNS (a session's `details/` folder) if given.
#[test]
fn stems_sample_pngs() {
    let Ok(dir) = std::env::var("FUSHION_STEMS_DIR") else { return };
    let mut ds = stub_designs();
    if let Ok(src) = std::env::var("FUSHION_STEMS_DESIGNS") {
        let load = |n: &str| {
            let d: sf_core::DetailDesign =
                serde_json::from_str(&std::fs::read_to_string(format!("{src}/{n}.json")).unwrap()).unwrap();
            (d.name, d.mesh, d.kit)
        };
        ds.heads = [load("magenta_rose"), load("daisy")];
        ds.leaf = load("big_leaf");
    }
    let m = bouquet(&ds);
    let lib = MeshLibrary::open_default().unwrap_or_else(|_| MeshLibrary::procedural());
    let pal = Palette::builtin();
    let o = RenderOptions { view_size: 480, edges: true, background: [236, 238, 241] };
    std::fs::create_dir_all(&dir).unwrap();
    for (v, name) in [(View::Front, "front"), (View::Iso, "iso"), (View::Top, "top")] {
        render_view(&m, &pal, &lib, v, &o).save(format!("{dir}/{name}.png")).unwrap();
    }
    render_sheet(&m, &pal, &lib, &[View::Front, View::Iso, View::Top, View::Left], &o).save(format!("{dir}/sheet.png")).unwrap();
    // A session folder with the model, for looking at it in the app.
    std::fs::create_dir_all(format!("{dir}/session")).unwrap();
    std::fs::write(format!("{dir}/session/model.json"), serde_json::to_string(&m).unwrap()).unwrap();
    let t = RenderOptions { view_size: 256, ..o.clone() };
    for (i, s) in m.stems.iter().enumerate() {
        render_stem(&s.def, s.color, &pal, &lib, View::Top, &t).save(format!("{dir}/stem{}-flat.png", i + 1)).unwrap();
        render_stem_assembly(s, &m, &pal, &lib, View::Iso, &t).save(format!("{dir}/stem{}-assembly.png", i + 1)).unwrap();
    }
}
