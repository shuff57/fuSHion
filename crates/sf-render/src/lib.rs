//! Headless CPU software renderer for studforge models.
//!
//! Orthographic, z-buffered, flat-shaded (Lambert + ambient), with optional
//! LDraw edge lines. No GPU, no windowing: it runs in tests and on servers.
//!
//! # Views
//! The subject's front faces **-z**. [`View::Front`] looks from -z toward +z
//! with +x to the right and up = +y (LDraw -Y). [`View::Top`] looks down with
//! the front at the bottom of the image. [`View::Iso`] looks from
//! front-left-above. The key light is fixed relative to the camera (upper
//! left, behind the viewer) so every view is lit the same way.

mod font;
mod raster;

use image::{Rgba, RgbaImage};
use raster::{SVert, Target};
use sf_core::{ColorId, Fill, Model, Palette, Placement, VoxelGrid};
use sf_ldraw::{MAIN_COLOR, MeshLibrary, PartMesh};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct RenderOptions {
    /// Pixels per view (square). Default 384.
    pub view_size: u32,
    /// Draw LDraw type-2 edges, darkened. Default true.
    pub edges: bool,
    pub background: [u8; 3],
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self { view_size: 384, edges: true, background: [236, 238, 241] }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum View {
    Front,
    Back,
    Left,
    Right,
    Top,
    Iso,
}

impl View {
    /// All views in sheet order.
    pub const ALL: [View; 6] = [View::Front, View::Back, View::Left, View::Right, View::Top, View::Iso];

    pub fn label(self) -> &'static str {
        match self {
            View::Front => "FRONT",
            View::Back => "BACK",
            View::Left => "LEFT",
            View::Right => "RIGHT",
            View::Top => "TOP",
            View::Iso => "ISO",
        }
    }

    /// `(right, up, dir)` unit vectors in LDraw axes; `dir` points from the
    /// camera into the scene.
    fn basis(self) -> ([f32; 3], [f32; 3], [f32; 3]) {
        // Work in a y-up frame, then flip y into LDraw (-Y up).
        let (r, u, d): ([f32; 3], [f32; 3], [f32; 3]) = match self {
            View::Front => ([1., 0., 0.], [0., 1., 0.], [0., 0., 1.]),
            View::Back => ([-1., 0., 0.], [0., 1., 0.], [0., 0., -1.]),
            View::Left => ([0., 0., -1.], [0., 1., 0.], [1., 0., 0.]),
            View::Right => ([0., 0., 1.], [0., 1., 0.], [-1., 0., 0.]),
            View::Top => ([1., 0., 0.], [0., 0., 1.], [0., -1., 0.]),
            View::Iso => {
                let d = norm([1.0, -0.8, 1.0]);
                let up = [0.0, 1.0, 0.0];
                let u = norm(sub(up, mul(d, dot(up, d))));
                (cross(u, d), u, d)
            }
        };
        let f = |v: [f32; 3]| [v[0], -v[1], v[2]];
        (f(r), f(u), f(d))
    }
}

/// An orthographic camera fitted to a box.
#[derive(Copy, Clone, Debug)]
struct Camera {
    r: [f32; 3],
    u: [f32; 3],
    d: [f32; 3],
    /// Pixels per LDU.
    scale: f32,
    /// Screen offset added after the linear part.
    off: [f32; 2],
    light: [f32; 3],
}

impl Camera {
    fn fit(view: View, lo: [f32; 3], hi: [f32; 3], size: u32) -> Self {
        let (r, u, d) = view.basis();
        let (mut a0, mut a1, mut b0, mut b1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
        for i in 0..8 {
            let p = [
                if i & 1 == 0 { lo[0] } else { hi[0] },
                if i & 2 == 0 { lo[1] } else { hi[1] },
                if i & 4 == 0 { lo[2] } else { hi[2] },
            ];
            let (a, b) = (dot(p, r), dot(p, u));
            (a0, a1, b0, b1) = (a0.min(a), a1.max(a), b0.min(b), b1.max(b));
        }
        let s = size as f32;
        let extent = (a1 - a0).max(b1 - b0).max(1.0);
        let scale = s * 0.88 / extent;
        let off = [s * 0.5 - (a0 + a1) * 0.5 * scale, s * 0.5 + (b0 + b1) * 0.5 * scale];
        let light = norm(add(add(mul(r, -0.35), mul(u, 0.55)), mul(d, -0.75)));
        Self { r, u, d, scale, off, light }
    }

    /// Linear part of the projection (no screen offset).
    #[inline]
    fn lin(&self, p: [f32; 3]) -> SVert {
        [dot(p, self.r) * self.scale, -dot(p, self.u) * self.scale, dot(p, self.d)]
    }

    #[inline]
    fn project(&self, p: [f32; 3]) -> SVert {
        let v = self.lin(p);
        [v[0] + self.off[0], v[1] + self.off[1], v[2]]
    }

    /// Lambert + ambient for a (double-sided) face normal.
    #[inline]
    fn shade(&self, n: [f32; 3]) -> f32 {
        let n = if dot(n, self.d) > 0.0 { mul(n, -1.0) } else { n };
        // Camera-facing faces land near 0.93 so colours stay recognisable.
        (0.55 + 0.5 * dot(n, self.light).max(0.0)).min(1.0)
    }

    /// Edge visibility tolerance in depth units (about two pixels).
    fn edge_bias(&self) -> f32 {
        (2.0 / self.scale).max(0.75)
    }

    /// Edge opacity: fade edges when parts get small on screen.
    fn edge_alpha(&self) -> f32 {
        // Pixels per LDU: below ~0.2 the edges are denser than the pixels and
        // only darken the picture, so they fade out completely.
        ((self.scale - 0.2) / 0.7).clamp(0.0, 0.9)
    }
}

/// A mesh pre-projected for one (part, rotation) under one camera, relative
/// to the placement's anchor.
struct Projected {
    /// Three vertices, shade factor, colour code.
    tris: Vec<([SVert; 3], f32, u16)>,
    edges: Vec<[SVert; 2]>,
}

fn project_mesh(cam: &Camera, mesh: &PartMesh, m: [[f32; 3]; 3], with_edges: bool) -> Projected {
    let xf = |v: [f32; 3]| mat_vec(m, v);
    let mut tris = Vec::with_capacity(mesh.indices.len() / 3);
    for t in mesh.indices.as_chunks::<3>().0 {
        let [i0, i1, i2] = t.map(|i| i as usize);
        let (Some(&a), Some(&b), Some(&c)) = (mesh.positions.get(i0), mesh.positions.get(i1), mesh.positions.get(i2)) else {
            continue;
        };
        let n = mesh.normals.get(i0).copied().unwrap_or([0.0, -1.0, 0.0]);
        let color = mesh.colors.get(i0).copied().unwrap_or(MAIN_COLOR);
        tris.push(([cam.lin(xf(a)), cam.lin(xf(b)), cam.lin(xf(c))], cam.shade(mat_vec(m, n)), color));
    }
    let edges =
        if with_edges { mesh.edges.iter().map(|e| [cam.lin(xf(e[0])), cam.lin(xf(e[1]))]).collect() } else { Vec::new() };
    Projected { tris, edges }
}

/// Model bounds in LDU (studs included), or `None` for an empty model.
fn model_bounds(model: &Model) -> Option<([f32; 3], [f32; 3])> {
    let (lo, hi) = model.bounds()?;
    Some((
        [lo.x as f32 * 20.0, -(hi.y as f32) * 8.0 - 4.0, lo.z as f32 * 20.0],
        [hi.x as f32 * 20.0, -(lo.y as f32) * 8.0, hi.z as f32 * 20.0],
    ))
}

#[inline]
fn shaded(rgb: [u8; 3], s: f32) -> [u8; 3] {
    rgb.map(|c| (c as f32 * s).min(255.0) as u8)
}

/// Darker (or, for very dark colours, lighter) outline colour.
fn edge_color(rgb: [u8; 3]) -> [u8; 3] {
    let lum = 0.2126 * rgb[0] as f32 + 0.7152 * rgb[1] as f32 + 0.0722 * rgb[2] as f32;
    if lum < 60.0 { rgb.map(|c| (c as f32 * 0.5 + 100.0) as u8) } else { rgb.map(|c| (c as f32 * 0.4) as u8) }
}

/// Resolves colour codes with a small cache (palette lookups are linear).
struct Colors<'a> {
    palette: &'a Palette,
    cache: HashMap<ColorId, Option<[u8; 3]>>,
}

impl<'a> Colors<'a> {
    fn new(palette: &'a Palette) -> Self {
        Self { palette, cache: HashMap::new() }
    }

    /// RGB for `code`; codes the palette doesn't know (e.g. printed details)
    /// fall back to `main`.
    fn get(&mut self, code: ColorId, main: [u8; 3]) -> [u8; 3] {
        let p = self.palette;
        self.cache.entry(code).or_insert_with(|| p.get(code).map(|c| c.rgb)).unwrap_or(main)
    }
}

/// Render one view of a model, auto-fitted with a margin.
///
/// Placements whose part has no mesh are skipped (with a warning once per
/// part id).
pub fn render_view(model: &Model, palette: &Palette, meshes: &MeshLibrary, view: View, opts: &RenderOptions) -> RgbaImage {
    let size = opts.view_size.max(8);
    let mut target = Target::new(size as usize, size as usize, opts.background);
    if let Some((lo, hi)) = model_bounds(model) {
        let cam = Camera::fit(view, lo, hi, size);
        draw_model(&mut target, &cam, model, palette, meshes, opts.edges);
    }
    to_image(&target)
}

fn draw_model(target: &mut Target, cam: &Camera, model: &Model, palette: &Palette, meshes: &MeshLibrary, edges: bool) {
    let mut projected: HashMap<(&str, u8), Option<Arc<Projected>>> = HashMap::new();
    let mut mesh_cache: HashMap<&str, Option<Arc<PartMesh>>> = HashMap::new();
    let mut colors = Colors::new(palette);
    let mut warned: Vec<&str> = Vec::new();
    // Edges are drawn after all faces so later parts can't overwrite them.
    let mut edge_queue: Vec<(Arc<Projected>, SVert, [u8; 3])> = Vec::new();

    for pl in &model.placements {
        let (pos, m) = pl.ldraw_transform();
        let key = (pl.part.as_str(), pl.rot.turns());
        let proj = projected
            .entry(key)
            .or_insert_with(|| {
                let mesh = mesh_cache.entry(pl.part.as_str()).or_insert_with(|| meshes.mesh(&pl.part)).clone()?;
                Some(Arc::new(project_mesh(cam, &mesh, m, edges)))
            })
            .clone();
        let Some(proj) = proj else {
            if !warned.contains(&pl.part.as_str()) {
                eprintln!("sf-render: no mesh for part {}; skipped", pl.part);
                warned.push(pl.part.as_str());
            }
            continue;
        };
        let o = cam.project(pos);
        let main = palette.rgb(pl.color);
        draw_projected(target, &proj, o, main, &mut colors);
        if edges && !proj.edges.is_empty() {
            edge_queue.push((proj, o, edge_color(main)));
        }
    }
    let (alpha, bias) = (cam.edge_alpha(), cam.edge_bias());
    for (proj, o, rgb) in &edge_queue {
        for e in &proj.edges {
            target.line(add(e[0], *o), add(e[1], *o), *rgb, alpha, bias);
        }
    }
}

#[inline]
fn draw_projected(target: &mut Target, proj: &Projected, o: SVert, main: [u8; 3], colors: &mut Colors) {
    let mut last = (MAIN_COLOR, main);
    for (v, s, code) in &proj.tris {
        let rgb = if *code == MAIN_COLOR {
            main
        } else if *code == last.0 {
            last.1
        } else {
            let c = colors.get(*code, main);
            last = (*code, c);
            c
        };
        target.tri(add(v[0], o), add(v[1], o), add(v[2], o), shaded(rgb, *s));
    }
}

const LABEL_H: u32 = 18;
const GAP: u32 = 2;

/// Render several views into one sheet.
///
/// Layout: `ceil(sqrt(n))` columns, filled row by row in the order given
/// (for [`View::ALL`]: FRONT BACK LEFT / RIGHT TOP ISO). Each cell has an
/// 18 px label strip on top with the view name in a small bitmap font.
/// Views are rendered in parallel threads.
pub fn render_sheet(model: &Model, palette: &Palette, meshes: &MeshLibrary, views: &[View], opts: &RenderOptions) -> RgbaImage {
    let imgs: Vec<RgbaImage> = std::thread::scope(|s| {
        let handles: Vec<_> =
            views.iter().map(|&v| s.spawn(move || render_view(model, palette, meshes, v, opts))).collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or_else(|_| RgbaImage::from_pixel(opts.view_size, opts.view_size, Rgba([255, 0, 255, 255]))))
            .collect()
    });
    compose_sheet(&imgs, views, opts)
}

fn compose_sheet(imgs: &[RgbaImage], views: &[View], opts: &RenderOptions) -> RgbaImage {
    let n = imgs.len().max(1) as u32;
    let cols = (n as f32).sqrt().ceil() as u32;
    let rows = n.div_ceil(cols);
    let vs = opts.view_size.max(8);
    let (cw, ch) = (vs, vs + LABEL_H);
    let w = cols * cw + (cols + 1) * GAP;
    let h = rows * ch + (rows + 1) * GAP;
    let bg = opts.background;
    let lum = bg[0] as u32 + bg[1] as u32 + bg[2] as u32;
    let (sep, strip, ink) = if lum > 384 {
        ([150, 150, 150], bg.map(|c| c.saturating_sub(22)), [40, 40, 40])
    } else {
        ([90, 90, 90], bg.map(|c| c.saturating_add(22)), [230, 230, 230])
    };
    let mut t = Target::new(w as usize, h as usize, sep);
    for (i, (img, view)) in imgs.iter().zip(views).enumerate() {
        let (cx, cy) = ((i as u32 % cols), (i as u32 / cols));
        let x0 = GAP + cx * (cw + GAP);
        let y0 = GAP + cy * (ch + GAP);
        t.rect(x0 as usize, y0 as usize, cw as usize, LABEL_H as usize, strip);
        font::draw_text(&mut t, x0 as usize + 6, y0 as usize + 2, view.label(), 2, ink);
        for (x, y, p) in img.enumerate_pixels() {
            let (px, py) = ((x0 + x) as usize, (y0 + LABEL_H + y) as usize);
            if px < t.w && py < t.h {
                t.color[py * t.w + px] = [p[0], p[1], p[2]];
            }
        }
    }
    to_image(&t)
}

/// (neighbour offset, outward normal, corners) of one voxel face.
type Face = ([i32; 3], [f32; 3], [[f32; 3]; 4]);

/// Debug render of a voxel grid: every filled cell is a 20×8×20 LDU box
/// (only faces next to empty cells are drawn). Interior cells are grey.
pub fn render_grid(grid: &VoxelGrid, palette: &Palette, view: View, opts: &RenderOptions) -> RgbaImage {
    let size = opts.view_size.max(8);
    let mut target = Target::new(size as usize, size as usize, opts.background);
    let [dx, dy, dz] = grid.dims;
    if grid.count_filled() > 0 {
        // Bounds of filled cells.
        let (mut lo, mut hi) = ([i32::MAX; 3], [i32::MIN; 3]);
        for (c, _) in grid.filled() {
            for (i, v) in [c.x, c.y, c.z].into_iter().enumerate() {
                lo[i] = lo[i].min(v);
                hi[i] = hi[i].max(v + 1);
            }
        }
        let blo = [lo[0] as f32 * 20.0, -(hi[1] as f32) * 8.0, lo[2] as f32 * 20.0];
        let bhi = [hi[0] as f32 * 20.0, -(lo[1] as f32) * 8.0, hi[2] as f32 * 20.0];
        let cam = Camera::fit(view, blo, bhi, size);
        let mut lines = Vec::new();
        let mut colors = Colors::new(palette);
        let interior = [150, 150, 150];
        for (c, f) in grid.filled() {
            let rgb = match f {
                Fill::Solid(code) => colors.get(code, [255, 0, 255]),
                _ => interior,
            };
            let x0 = c.x as f32 * 20.0;
            let z0 = c.z as f32 * 20.0;
            let ytop = -(c.y as f32 + 1.0) * 8.0;
            let ybot = -(c.y as f32) * 8.0;
            let (x1, z1) = (x0 + 20.0, z0 + 20.0);
            let faces: [Face; 6] = [
                ([1, 0, 0], [1., 0., 0.], [[x1, ytop, z0], [x1, ytop, z1], [x1, ybot, z1], [x1, ybot, z0]]),
                ([-1, 0, 0], [-1., 0., 0.], [[x0, ytop, z0], [x0, ybot, z0], [x0, ybot, z1], [x0, ytop, z1]]),
                ([0, 1, 0], [0., -1., 0.], [[x0, ytop, z0], [x0, ytop, z1], [x1, ytop, z1], [x1, ytop, z0]]),
                ([0, -1, 0], [0., 1., 0.], [[x0, ybot, z0], [x1, ybot, z0], [x1, ybot, z1], [x0, ybot, z1]]),
                ([0, 0, 1], [0., 0., 1.], [[x0, ytop, z1], [x0, ybot, z1], [x1, ybot, z1], [x1, ytop, z1]]),
                ([0, 0, -1], [0., 0., -1.], [[x0, ytop, z0], [x1, ytop, z0], [x1, ybot, z0], [x0, ybot, z0]]),
            ];
            for (nb, n, q) in faces {
                let (nx, ny, nz) = (c.x + nb[0], c.y + nb[1], c.z + nb[2]);
                let inside = nx >= 0 && ny >= 0 && nz >= 0 && nx < dx && ny < dy && nz < dz;
                if inside && grid.get(sf_core::Cell::new(nx, ny, nz)).is_filled() {
                    continue;
                }
                let col = shaded(rgb, cam.shade(n));
                let p = q.map(|v| cam.project(v));
                target.tri(p[0], p[1], p[2], col);
                target.tri(p[0], p[2], p[3], col);
                if opts.edges {
                    for k in 0..4 {
                        lines.push((p[k], p[(k + 1) % 4], edge_color(rgb)));
                    }
                }
            }
        }
        let (alpha, bias) = (cam.edge_alpha() * 0.7, cam.edge_bias());
        for (a, b, rgb) in lines {
            target.line(a, b, rgb, alpha, bias);
        }
    }
    to_image(&target)
}

/// Render a single placement at the origin (handy for part thumbnails).
pub fn render_part(placement: &Placement, palette: &Palette, meshes: &MeshLibrary, view: View, opts: &RenderOptions) -> RgbaImage {
    let mut p = placement.clone();
    p.origin = sf_core::Cell::new(0, 0, 0);
    let model = Model { placements: vec![p], ..Default::default() };
    render_view(&model, palette, meshes, view, opts)
}

fn to_image(t: &Target) -> RgbaImage {
    let mut buf = Vec::with_capacity(t.w * t.h * 4);
    for c in &t.color {
        buf.extend_from_slice(&[c[0], c[1], c[2], 255]);
    }
    RgbaImage::from_raw(t.w as u32, t.h as u32, buf).unwrap_or_else(|| RgbaImage::new(t.w as u32, t.h as u32))
}

#[inline]
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
#[inline]
fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
#[inline]
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
#[inline]
fn mul(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn norm(a: [f32; 3]) -> [f32; 3] {
    let l = dot(a, a).sqrt().max(1e-12);
    mul(a, 1.0 / l)
}
#[inline]
fn mat_vec(m: [[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    [dot(m[0], v), dot(m[1], v), dot(m[2], v)]
}

#[cfg(test)]
mod tests;
