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
