//! Bouquet stems in the instructions: grouping identical stems, where they join the build,
//! the pictures of a stem being dressed with its head and leaves, and the numbered hub
//! holes.
//!
//! Stems are drawn by sf-render (`render_view` draws `Model::stems`; `render_stem` lays one
//! flat). Steps show stems by carrying a subset of them, recoloured, in a derived model.

use crate::view::{self, New, ScratchPalette, paint_stem};
use image::RgbaImage;
use sf_core::stem::LDU_PER_MM;
use sf_core::{Cell, ColorId, CustomDef, CustomKind, DetailMesh, KitPose, Model, Palette, Placement, Rot, StemAttach, StemDef, StemPlacement};
use sf_ldraw::MeshLibrary;
use sf_render::{RenderOptions, View, render_stem, render_view};
use std::collections::BTreeMap;

type M3 = [[f32; 3]; 3];

pub const IDENTITY: KitPose = KitPose { rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], position: [0.0; 3] };

fn mat_vec(m: &M3, v: [f32; 3]) -> [f32; 3] {
    [0, 1, 2].map(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2])
}

fn mat_mul(a: &M3, b: &M3) -> M3 {
    let mut r = [[0.0; 3]; 3];
    for (i, row) in r.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    r
}

fn transpose(m: &M3) -> M3 {
    [0, 1, 2].map(|i| [m[0][i], m[1][i], m[2][i]])
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn norm(a: [f32; 3]) -> [f32; 3] {
    let l = dot(a, a).sqrt().max(1e-9);
    a.map(|c| c / l)
}

/// `p` applied to the point `v`.
pub fn apply(p: &KitPose, v: [f32; 3]) -> [f32; 3] {
    let r = mat_vec(&p.rotation, v);
    [r[0] + p.position[0], r[1] + p.position[1], r[2] + p.position[2]]
}

/// `a ∘ b`: first `b`, then `a`.
pub fn compose(a: &KitPose, b: &KitPose) -> KitPose {
    KitPose { rotation: mat_mul(&a.rotation, &b.rotation), position: apply(a, b.position) }
}

pub fn inverse(a: &KitPose) -> KitPose {
    let rt = transpose(&a.rotation);
    let t = mat_vec(&rt, a.position);
    KitPose { rotation: rt, position: t.map(|c| -c) }
}

/// Rotation about the vertical axis (LDraw Y), degrees.
pub fn rot_y(deg: f32) -> M3 {
    let (s, c) = deg.to_radians().sin_cos();
    [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]]
}

/// Rotation about Z, degrees: turns `-Y` (up) toward `+X`.
pub fn rot_z(deg: f32) -> M3 {
    let (s, c) = deg.to_radians().sin_cos();
    [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]]
}

/// A mesh moved by `p`.
pub fn transformed(mesh: &DetailMesh, p: &KitPose) -> DetailMesh {
    DetailMesh { positions: mesh.positions.iter().map(|&v| apply(p, v)).collect(), indices: mesh.indices.clone() }
}

fn bbox(mesh: &DetailMesh) -> Option<([f32; 3], [f32; 3])> {
    let mut it = mesh.positions.iter();
    let first = *it.next()?;
    let (mut lo, mut hi) = (first, first);
    for v in it {
        for i in 0..3 {
            lo[i] = lo[i].min(v[i]);
            hi[i] = hi[i].max(v[i]);
        }
    }
    Some((lo, hi))
}

/// Adds `mesh` (model space, LDU) to `m` as a detail placement keyed `key`, its cells
/// covering the mesh so cameras fit it (for drawing a design on its own).
pub fn bake(m: &mut Model, key: &str, mesh: &DetailMesh, color: ColorId) {
    let Some((lo, hi)) = bbox(mesh) else { return };
    let x0 = (lo[0] / 20.0).floor() as i32;
    let x1 = ((hi[0] / 20.0).ceil() as i32).max(x0 + 1);
    let z0 = (lo[2] / 20.0).floor() as i32;
    let z1 = ((hi[2] / 20.0).ceil() as i32).max(z0 + 1);
    // Plate p spans LDraw y from -(p + 1)·8 to -p·8.
    let y0 = (-hi[1] / 8.0).floor() as i32;
    let y1 = ((-lo[1] / 8.0).ceil() as i32).max(y0 + 1);
    bake_at(m, key, mesh, color, Cell::new(x0, y0, z0), [x1 - x0, z1 - z0], y1 - y0);
}

/// As [`bake`], with the placement's cells given: `size` studs from `origin`, `plates` high.
fn bake_at(m: &mut Model, key: &str, mesh: &DetailMesh, color: ColorId, origin: Cell, size: [i32; 2], plates: i32) {
    let [w, d] = size.map(|s| s.clamp(1, 255) as u8);
    let def = CustomDef {
        id: key.to_string(),
        kind: CustomKind::Detail { mesh: key.to_string(), name: "stem part".into() },
        cells: (0..w).flat_map(|x| (0..d).map(move |z| (x, z))).collect(),
        colors: BTreeMap::new(),
        plates: plates.clamp(1, 255) as u8,
        studs: Vec::new(),
        units: Vec::new(),
        sockets: Vec::new(),
        trim: None,
        mesh: None,
    };
    bake_as(m, Placement::printed(def, color, origin, Rot(0)), key, mesh);
}

/// Adds `mesh` (model space) to `m` drawn by `p` (a detail placement keyed `key`): the mesh
/// is stored in `p`'s frame so it lands where it was.
fn bake_as(m: &mut Model, p: Placement, key: &str, mesh: &DetailMesh) {
    let (pos, r) = p.ldraw_transform();
    let rt = transpose(&r);
    let local = DetailMesh { positions: mesh.positions.iter().map(|&v| mat_vec(&rt, sub(v, pos))).collect(), indices: mesh.indices.clone() };
    m.meshes.insert(key.to_string(), local);
    m.placements.push(p);
}

/// An empty model carrying `model`'s designs (meshes, kits, details) but no parts or stems.
pub fn blank(model: &Model) -> Model {
    Model { placements: Vec::new(), steps: Vec::new(), stems: Vec::new(), ..model.clone() }
}

/// A stem's heads and leaves: the head on its tip, heads on branch tips, then leaves.
pub fn attachments(sp: &StemPlacement) -> impl Iterator<Item = &StemAttach> {
    sp.head.iter().chain(&sp.branch_heads).chain(&sp.leaves)
}

/// A rounded pose, for comparing stems' heads and leaves.
fn round_pose(p: &KitPose) -> String {
    let r = |v: f32| (v * 20.0).round() as i32;
    format!("{:?}{:?}", p.rotation.map(|row| row.map(r)), p.position.map(r))
}

/// What makes two stems the same build: definition, colour, and each attachment's design,
/// colours and pose relative to the stem.
fn signature(sp: &StemPlacement) -> String {
    let inv = inverse(&sp.pose);
    let att = |a: &StemAttach| {
        format!("{}|{}|{:?}|{:?}|{}", a.design, a.color, a.colors, a.bar, round_pose(&compose(&inv, &a.pose)))
    };
    let leaves: Vec<String> = sp.leaves.iter().map(att).collect();
    let branches: Vec<String> = sp.branch_heads.iter().map(|a| format!("{:?}{}", a.branch, att(a))).collect();
    format!("{:?}|{}|{:?}|{}|{}", sp.def, sp.color, sp.head.as_ref().map(att), branches.join(";"), leaves.join(";"))
}

/// Stems grouped into identical builds (indices into `Model::stems`), in order of first use.
pub fn groups(model: &Model) -> Vec<Vec<usize>> {
    let mut out: Vec<(String, Vec<usize>)> = Vec::new();
    for (i, sp) in model.stems.iter().enumerate() {
        let sig = signature(sp);
        match out.iter_mut().find(|(s, _)| *s == sig) {
            Some((_, v)) => v.push(i),
            None => out.push((sig, vec![i])),
        }
    }
    out.into_iter().map(|(_, v)| v).collect()
}

/// One main build step: a step of the model's placements, or stems plugged into the hub.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MainStep {
    /// `Model::steps[k]`.
    Parts(usize),
    /// Indices into `Model::stems`.
    Stems(Vec<usize>),
}

/// The placement index of the hub a stem plugs into.
pub fn hub_index(model: &Model, sp: &StemPlacement) -> Option<usize> {
    model.placements.iter().position(|p| p.custom.as_deref().is_some_and(|c| c.id == sp.hub))
}

/// The main build sequence: the model's steps, with the stems of each hub plugged in (in
/// hole order, `per_step` at a time) right after the step that places the hub. Stems whose
/// hub isn't placed come last.
pub fn sequence(model: &Model, steps: &[Vec<usize>], per_step: usize) -> Vec<MainStep> {
    let mut after: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    let mut tail = Vec::new();
    for (si, sp) in model.stems.iter().enumerate() {
        match hub_index(model, sp).and_then(|h| steps.iter().position(|s| s.contains(&h))) {
            Some(k) => after.entry(k).or_default().push(si),
            None => tail.push(si),
        }
    }
    let by_hole = |v: &mut Vec<usize>| v.sort_by_key(|&si| (model.stems[si].hole, si));
    let mut out = Vec::new();
    for k in 0..steps.len() {
        out.push(MainStep::Parts(k));
        if let Some(v) = after.get_mut(&k) {
            by_hole(v);
            out.extend(v.chunks(per_step.max(1)).map(|c| MainStep::Stems(c.to_vec())));
        }
    }
    by_hole(&mut tail);
    out.extend(tail.chunks(per_step.max(1)).map(|c| MainStep::Stems(c.to_vec())));
    out
}

/// The model as built up to main step `j`: earlier parts and stems pale (when `ghost`), step
/// `j`'s in colour. Returns the model and what step `j` adds to it.
pub fn scene(
    model: &Model,
    steps: &[Vec<usize>],
    seq: &[MainStep],
    j: usize,
    pal: &mut ScratchPalette,
    palette: &Palette,
    ghost: bool,
) -> (Model, New) {
    let mut out = blank(model);
    let mut new = New::default();
    for (i, ms) in seq.iter().enumerate().take(j + 1) {
        let current = i == j;
        let mut color = |c: ColorId| if !current && ghost { pal.ghost(c) } else { c };
        match ms {
            MainStep::Parts(k) => {
                for &pi in steps.get(*k).map(Vec::as_slice).unwrap_or(&[]) {
                    let Some(p) = model.placements.get(pi) else { continue };
                    let mut p = p.clone();
                    p.color = color(p.color);
                    if current {
                        new.placements.push(out.placements.len());
                    }
                    out.placements.push(p);
                }
            }
            MainStep::Stems(list) => {
                for &si in list {
                    let mut sp = model.stems[si].clone();
                    paint_stem(&mut sp, model, palette, &mut color);
                    if current {
                        new.stems.push(out.stems.len());
                    }
                    out.stems.push(sp);
                }
            }
        }
    }
    (out, new)
}

/// HSV hue (degrees) of a colour, or `None` for greys and near-black.
fn hue(p: &image::Rgba<u8>) -> Option<f32> {
    let [r, g, b] = [p[0], p[1], p[2]].map(f32::from);
    let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
    if mx < 40.0 || mx - mn < 0.35 * mx {
        return None;
    }
    let d = mx - mn;
    let h = if mx == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if mx == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    Some(h)
}

fn from_hue(h: f32) -> [u8; 3] {
    let x = 1.0 - ((h / 60.0) % 2.0 - 1.0).abs();
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (1.0, x, 0.0),
        1 => (x, 1.0, 0.0),
        2 => (0.0, 1.0, x),
        3 => (0.0, x, 1.0),
        4 => (x, 0.0, 1.0),
        _ => (1.0, 0.0, x),
    };
    [r, g, b].map(|c: f32| (c * 255.0) as u8)
}

/// Where each hole (`(number, mouth in LDU)`) of the hub at `scene.placements[hub]` lands in
/// an iso render of `scene` turned `turns` quarter turns at `size` pixels: `(number, x, y)`
/// in that render's pixels, for the holes that show.
///
/// The scene is drawn again in grey with a flat marker on each hole in its own hue. Each
/// marker is drawn by a copy of the hub's placement, so the scene's bounds, and with them
/// the camera, stay exactly the same.
pub fn hole_points(scene: &Model, hub: usize, holes: &[(u32, [f32; 3])], turns: u8, meshes: &MeshLibrary, size: u32) -> Vec<(u32, f32, f32)> {
    let Some(hub_p) = scene.placements.get(hub) else { return Vec::new() };
    if holes.is_empty() {
        return Vec::new();
    }
    let mut pal = ScratchPalette::new(&Palette { colors: Vec::new() });
    let grey = pal.add([90, 90, 90]);
    let mut mask = scene.clone();
    for p in &mut mask.placements {
        p.color = grey;
    }
    let mut stems = std::mem::take(&mut mask.stems);
    for sp in &mut stems {
        paint_stem(sp, scene, &pal.palette.clone(), &mut |_| grey);
    }
    mask.stems = stems;
    let n = holes.len();
    let step = 300.0 / n as f32;
    for (k, (_, at)) in holes.iter().enumerate() {
        let code = pal.add(from_hue(k as f32 * step));
        let mut cube = DetailMesh::default();
        // A flat square on the hub's top, wider than a stem, so it shows around one.
        crate::sample::box_mesh(&mut cube, 22.0, 22.0, -1.5, 0.0);
        let cube = transformed(&cube, &KitPose { position: *at, ..IDENTITY });
        let key = format!("~hole{k}");
        let mut p = hub_p.clone();
        if let Some(def) = p.custom.as_deref_mut() {
            def.kind = CustomKind::Detail { mesh: key.clone(), name: "hole".into() };
            def.trim = None;
            def.mesh = None;
        }
        p.color = code;
        bake_as(&mut mask, p, &key, &cube);
    }
    let opts = RenderOptions { view_size: size, edges: false, background: [0, 0, 0] };
    let img = render_view(&view::turn_model(&mask, turns), &pal.palette, meshes, View::Iso, &opts);
    let mut acc = vec![(0.0f64, 0.0f64, 0usize); n];
    for (x, y, p) in img.enumerate_pixels() {
        let Some(h) = hue(p) else { continue };
        let k = (h / step).round() as usize;
        if k < n && (h - k as f32 * step).abs() < step * 0.3 {
            acc[k].0 += x as f64;
            acc[k].1 += y as f64;
            acc[k].2 += 1;
        }
    }
    holes
        .iter()
        .zip(acc)
        .filter(|(_, a)| a.2 > 0)
        .map(|((num, _), (sx, sy, c))| (*num, (sx / c as f64) as f32, (sy / c as f64) as f32))
        .collect()
}

/// A stem as shown on its build page: standing in its own frame, turned 30° about the
/// vertical so it has some depth. Its bend points right, its flat side away from you.
pub fn presented(sp: &StemPlacement) -> StemPlacement {
    let turn = KitPose { rotation: rot_y(30.0), position: [0.0; 3] };
    let frame = compose(&turn, &inverse(&sp.pose));
    let mut out = sp.clone();
    out.pose = turn;
    for a in out.head.iter_mut().chain(out.branch_heads.iter_mut()).chain(out.leaves.iter_mut()) {
        a.pose = compose(&frame, &a.pose);
    }
    out
}

fn render_one(model: &Model, sp: StemPlacement, palette: &Palette, meshes: &MeshLibrary, size: u32) -> RgbaImage {
    let mut m = blank(model);
    m.stems.push(sp);
    let opts = RenderOptions { view_size: size, edges: true, background: [255, 255, 255] };
    render_view(&m, palette, meshes, View::Front, &opts)
}

/// A head or leaf on a stem.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    /// The head on the main tip.
    Head,
    /// `branch_heads[i]`.
    Branch(usize),
    /// `leaves[i]`.
    Leaf(usize),
}

pub fn slot(sp: &StemPlacement, s: Slot) -> Option<&StemAttach> {
    match s {
        Slot::Head => sp.head.as_ref(),
        Slot::Branch(i) => sp.branch_heads.get(i),
        Slot::Leaf(i) => sp.leaves.get(i),
    }
}

/// The steps of dressing a stem: identical heads (the main tip's and the branches') go on
/// together, one step per kind; then each leaf.
pub fn plan(sp: &StemPlacement) -> Vec<Vec<Slot>> {
    let mut heads: Vec<(String, Vec<Slot>)> = Vec::new();
    let all = sp.head.iter().map(|a| (Slot::Head, a)).chain(sp.branch_heads.iter().enumerate().map(|(i, a)| (Slot::Branch(i), a)));
    for (s, a) in all {
        let key = format!("{}|{}|{:?}", a.design, a.color, a.colors);
        match heads.iter_mut().find(|(k, _)| *k == key) {
            Some((_, v)) => v.push(s),
            None => heads.push((key, vec![s])),
        }
    }
    let mut out: Vec<Vec<Slot>> = heads.into_iter().map(|(_, v)| v).collect();
    out.extend((0..sp.leaves.len()).map(|i| vec![Slot::Leaf(i)]));
    out
}

/// The pictures for a stem's build page, all cropped alike: one per step of [`plan`] (what
/// goes on in colour, what is already on pale, the stem pale), then the finished stem.
pub fn build_pictures(model: &Model, sp: &StemPlacement, palette: &Palette, meshes: &MeshLibrary, size: u32) -> Vec<RgbaImage> {
    let shown = presented(sp);
    let mut pal = ScratchPalette::new(palette);
    let steps = plan(&shown);
    let mut stages: Vec<StemPlacement> = Vec::new();
    for k in 0..steps.len() {
        let on: Vec<Slot> = steps[..=k].iter().flatten().copied().collect();
        let mut ghost = |c: ColorId| pal.ghost(c);
        let mut take = |s: Slot| -> Option<StemAttach> {
            let mut a = slot(&shown, s)?.clone();
            if !steps[k].contains(&s) {
                view::paint_attach(&mut a, model, palette, &mut ghost);
            }
            Some(a)
        };
        let head = if on.contains(&Slot::Head) { take(Slot::Head) } else { None };
        let branch_heads = (0..shown.branch_heads.len()).filter(|&i| on.contains(&Slot::Branch(i))).filter_map(|i| take(Slot::Branch(i))).collect();
        let leaves = (0..shown.leaves.len()).filter(|&i| on.contains(&Slot::Leaf(i))).filter_map(|i| take(Slot::Leaf(i))).collect();
        let color = pal.ghost(shown.color);
        stages.push(StemPlacement { color, head, branch_heads, leaves, ..shown.clone() });
    }
    stages.push(shown);
    let imgs: Vec<RgbaImage> = stages.into_iter().map(|st| render_one(model, st, &pal.palette, meshes, size)).collect();
    view::trim_all(&imgs, 6)
}

/// A stem's name: "S03 daisy sprig", or "S03 stem" without a style.
pub fn title(def: &StemDef) -> String {
    let style = def.style.trim();
    format!("{} {}", def.id, if style.is_empty() { "stem" } else { style })
}

/// A finished stem, upright as on its build page.
pub fn finished_thumb(model: &Model, sp: &StemPlacement, palette: &Palette, meshes: &MeshLibrary, size: u32) -> RgbaImage {
    view::trim(&render_one(model, presented(sp), palette, meshes, size), 4)
}

/// A stem lying flat as it prints, seen from above, base on the left.
pub fn flat_thumb(def: &StemDef, color: ColorId, palette: &Palette, meshes: &MeshLibrary, size: u32) -> RgbaImage {
    let opts = RenderOptions { view_size: size, edges: true, background: [255, 255, 255] };
    // sf-render draws it base down; turn the picture so it runs left to right.
    image::imageops::rotate90(&view::trim(&render_stem(def, color, palette, meshes, View::Top, &opts), 4))
}

/// A design (head or leaf) on its own, iso.
pub fn design_thumb(model: &Model, design: &str, color: ColorId, palette: &Palette, meshes: &MeshLibrary, size: u32) -> RgbaImage {
    let mut m = blank(model);
    if let Some(mesh) = model.meshes.get(design) {
        bake(&mut m, "~design", mesh, color);
    }
    let opts = RenderOptions { view_size: size, edges: true, background: [255, 255, 255] };
    view::trim(&render_view(&m, palette, meshes, View::Iso, &opts), 4)
}

/// "180 mm, bent 20°, 2 leaf bars, 2 branches" for the parts list (the style is in the title).
pub fn describe(def: &StemDef) -> String {
    let mut s = format!("{:.0} mm", def.length);
    if def.bend.abs() >= 1.0 {
        s.push_str(&format!(", bent {:.0}°", def.bend.abs()));
    }
    match def.bars.len() {
        0 => {}
        1 => s.push_str(", 1 leaf bar"),
        n => s.push_str(&format!(", {n} leaf bars")),
    }
    match def.branches.len() {
        0 => {}
        1 => s.push_str(", 1 branch"),
        n => s.push_str(&format!(", {n} branches")),
    }
    match def.tip {
        Some(sf_core::stem::StemTip::Bud { .. }) => s.push_str(", bud tip"),
        Some(sf_core::stem::StemTip::Blade { .. }) => s.push_str(", blade tip"),
        None => {}
    }
    s
}

/// A direction in the build-page picture for a vector in the stem's own frame: its bend is
/// to the right, `-Z` toward the viewer.
fn direction(v: [f32; 3]) -> String {
    let words = [
        ([1.0, 0.0, 0.0], "right"),
        ([-1.0, 0.0, 0.0], "left"),
        ([0.0, -1.0, 0.0], "up"),
        ([0.0, 1.0, 0.0], "down"),
        ([0.0, 0.0, -1.0], "toward you"),
        ([0.0, 0.0, 1.0], "away from you"),
    ];
    let v = norm(v);
    let mut scored: Vec<(f32, &str)> = words.iter().map(|(d, w)| (dot(v, *d), *w)).collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    if scored[1].0 > 0.4 { format!("{} and {}", scored[0].1, scored[1].1) } else { scored[0].1.to_string() }
}

/// Which way a stem's head faces (its design's `-Y`), in build-page words.
pub fn head_facing(sp: &StemPlacement) -> Option<String> {
    sp.head.as_ref().map(|h| facing(sp, h))
}

/// Which way a head on `sp` faces, in build-page words.
pub fn facing(sp: &StemPlacement, h: &StemAttach) -> String {
    let rel = compose(&inverse(&sp.pose), &h.pose);
    direction(mat_vec(&rel.rotation, [0.0, -1.0, 0.0]))
}

/// Where leaf `i` sits: (1-based bar number, the bar's height up the stem in mm, the side in
/// build-page words).
pub fn leaf_place(model: &Model, sp: &StemPlacement, i: usize) -> (Option<usize>, Option<f32>, String) {
    let leaf = &sp.leaves[i];
    // The middle of the leaf's mesh, in the stem's frame.
    let rel = compose(&inverse(&sp.pose), &leaf.pose);
    let c = model.meshes.get(&leaf.design).and_then(bbox).map(|(lo, hi)| [0, 1, 2].map(|k| (lo[k] + hi[k]) / 2.0)).unwrap_or([0.0; 3]);
    let c = apply(&rel, c);
    let bar = leaf.bar.and_then(|b| sp.def.bars.get(b).map(|bar| (b, bar.at)));
    let s = bar.map(|(_, at)| at).unwrap_or_else(|| (-c[1] / LDU_PER_MM).clamp(0.0, sp.def.length));
    let (p, t) = (sp.def.point(s), sp.def.tangent(s));
    let d = sub(c, p);
    let d = sub(d, t.map(|x| x * dot(d, t)));
    let side = if dot(d, d) < 1e-6 {
        "around it".to_string()
    } else {
        match direction(d).as_str() {
            "toward you" => "on the side toward you".to_string(),
            "away from you" => "on the far side".to_string(),
            w => format!("on the {w} side"),
        }
    };
    (bar.map(|(b, _)| b + 1), bar.map(|(_, at)| at), side)
}

/// A design key as words: `"leaf_small"` → "leaf small".
pub fn humanize(design: &str) -> String {
    design.replace(['_', '-'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::bouquet_model;

    #[test]
    fn poses_compose_and_invert() {
        let a = KitPose { rotation: mat_mul(&rot_y(35.0), &rot_z(20.0)), position: [3.0, -7.0, 11.0] };
        let b = KitPose { rotation: rot_z(-50.0), position: [1.0, 2.0, 3.0] };
        let v = [4.0, -5.0, 6.0];
        let ab = apply(&compose(&a, &b), v);
        let want = apply(&a, apply(&b, v));
        assert!((0..3).all(|i| (ab[i] - want[i]).abs() < 1e-4));
        let back = apply(&inverse(&a), apply(&a, v));
        assert!((0..3).all(|i| (back[i] - v[i]).abs() < 1e-4));
    }

    #[test]
    fn baked_meshes_land_where_they_were() {
        // Rendering goes through a placement's transform: baking must undo it exactly.
        let mut m = Model::default();
        let mut mesh = DetailMesh::default();
        crate::sample::box_mesh(&mut mesh, 10.0, 10.0, -5.0, 5.0);
        let mesh = transformed(&mesh, &KitPose { position: [37.0, -101.0, -13.0], ..IDENTITY });
        bake(&mut m, "x", &mesh, 4);
        let p = &m.placements[0];
        let (pos, r) = p.ldraw_transform();
        for (local, world) in m.meshes["x"].positions.iter().zip(&mesh.positions) {
            let w = mat_vec(&r, *local);
            assert!((0..3).all(|i| (w[i] + pos[i] - world[i]).abs() < 1e-3));
        }
        // The cells cover the mesh.
        let (lo, hi) = m.bounds().unwrap();
        assert!(lo.x as f32 * 20.0 <= 32.0 && hi.x as f32 * 20.0 >= 42.0);
        assert!(-(hi.y as f32) * 8.0 <= -106.0 && -(lo.y as f32) * 8.0 >= -96.0);
    }

    #[test]
    fn identical_stems_group_and_join_after_the_hub() {
        let m = bouquet_model();
        assert_eq!(groups(&m), vec![vec![0], vec![1, 2]]);
        assert_eq!(plan(&m.stems[0]), vec![vec![Slot::Head], vec![Slot::Leaf(0)], vec![Slot::Leaf(1)]]);
        let steps = crate::steps_of(&m);
        let seq = sequence(&m, &steps, 1);
        let n = steps.len();
        assert_eq!(seq.len(), n + 3);
        // The hub is placed in the last model step; the stems follow it in hole order.
        assert_eq!(&seq[n..], &[MainStep::Stems(vec![0]), MainStep::Stems(vec![1]), MainStep::Stems(vec![2])]);
        assert_eq!(sequence(&m, &steps, 2)[n..], [MainStep::Stems(vec![0, 1]), MainStep::Stems(vec![2])]);
    }

    #[test]
    fn words_for_leaves_and_heads() {
        let m = bouquet_model();
        // S01's leaves: bar 1 on the right, bar 2 on the left; its rose faces up.
        let (bar, at, side) = leaf_place(&m, &m.stems[0], 0);
        assert_eq!((bar, side.as_str()), (Some(1), "on the right side"));
        assert_eq!(at, Some(50.0));
        assert_eq!(leaf_place(&m, &m.stems[0], 1).2, "on the left side");
        assert_eq!(head_facing(&m.stems[0]).as_deref(), Some("up"));
        // S02 bends 20°: its bud faces up, leaning a little right.
        assert_eq!(head_facing(&m.stems[1]).as_deref(), Some("up"));
    }

    #[test]
    fn hole_numbers_are_found_in_the_picture() {
        let m = bouquet_model();
        let pal = Palette::builtin();
        let steps = crate::steps_of(&m);
        let seq = sequence(&m, &steps, 1);
        let mut sp = ScratchPalette::new(&pal);
        let (scene, _) = scene(&m, &steps, &seq, seq.len() - 1, &mut sp, &pal, true);
        let holes: Vec<(u32, [f32; 3])> = m.stems.iter().map(|s| (s.hole, s.pose.position)).collect();
        let hub = scene.placements.iter().position(|p| p.part == "D010").unwrap();
        let pts = hole_points(&scene, hub, &holes, 0, &MeshLibrary::procedural(), 300);
        assert_eq!(pts.len(), 3, "{pts:?}");
        // Hole 1 is at the hub's centre, hole 3 toward +x −z (screen right in iso), hole 2
        // toward −x +z.
        let x = |n: u32| pts.iter().find(|p| p.0 == n).unwrap().1;
        assert!(x(2) < x(1) && x(1) < x(3), "{pts:?}");
    }
}
