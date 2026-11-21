//! The 3D viewer widget: orbit camera, scene building, picking.
//!
//! World space is y-up and measured in studs: LDraw `(X, Y, Z)` LDU maps to
//! `(X / 20, -Y / 20, Z / 20)`, so a plate is 0.4 units tall. The subject's
//! front faces -z; the camera conventions match `sf-render`'s views.

use super::gpu::{self, Globals, Instance, LineVertex, Scene, ViewerCallback};
use eframe::egui_wgpu;
use egui::{Color32, Pos2, Rect, Sense, Vec2};
use sf_core::{Model, Palette, Placement};
use sf_ldraw::MeshLibrary;
use std::collections::HashMap;
use std::sync::Arc;

pub const PLATE: f32 = 0.4;
const FOV_Y: f32 = 0.55; // radians (~31°)

pub type V3 = [f32; 3];

pub fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub fn mul(a: V3, s: f32) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
pub fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
pub fn norm(a: V3) -> V3 {
    mul(a, 1.0 / dot(a, a).sqrt().max(1e-9))
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Preset {
    Front,
    Right,
    Back,
    Left,
    Top,
    Iso,
}

impl Preset {
    pub const ALL: [Preset; 6] = [
        Preset::Front,
        Preset::Right,
        Preset::Back,
        Preset::Left,
        Preset::Top,
        Preset::Iso,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Preset::Front => "Front",
            Preset::Right => "Right",
            Preset::Back => "Back",
            Preset::Left => "Left",
            Preset::Top => "Top",
            Preset::Iso => "Iso",
        }
    }
    /// (yaw, pitch) in radians.
    fn angles(self) -> (f32, f32) {
        use std::f32::consts::{FRAC_PI_2, PI};
        match self {
            Preset::Front => (0.0, 0.0),
            Preset::Right => (-FRAC_PI_2, 0.0),
            Preset::Back => (PI, 0.0),
            Preset::Left => (FRAC_PI_2, 0.0),
            Preset::Top => (0.0, FRAC_PI_2),
            // sf-render's iso direction (1, -0.8, 1).
            Preset::Iso => (PI / 4.0, (0.8f32 / 2f32.sqrt()).atan()),
        }
    }
}

/// Orbit camera. `yaw = 0, pitch = 0` looks from -z toward +z (front view);
/// positive pitch looks down.
#[derive(Clone, Debug)]
pub struct Camera {
    pub target: V3,
    pub yaw: f32,
    pub pitch: f32,
    pub dist: f32,
}

impl Default for Camera {
    fn default() -> Self {
        let (yaw, pitch) = Preset::Iso.angles();
        Self {
            target: [0.0, 2.0, 0.0],
            yaw,
            pitch,
            dist: 30.0,
        }
    }
}

impl Camera {
    /// (right, up, dir) unit vectors; `dir` points into the scene.
    pub fn basis(&self) -> (V3, V3, V3) {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        let d = [sy * cp, -sp, cy * cp];
        let r = [cy, 0.0, -sy];
        let u = cross(d, r);
        (r, u, d)
    }

    pub fn eye(&self) -> V3 {
        let (_, _, d) = self.basis();
        sub(self.target, mul(d, self.dist))
    }

    pub fn set_preset(&mut self, p: Preset) {
        (self.yaw, self.pitch) = p.angles();
    }

    /// Frame a bounding box (world units).
    pub fn fit(&mut self, lo: V3, hi: V3, aspect: f32) {
        self.target = mul(add(lo, hi), 0.5);
        let r = (dot(sub(hi, lo), sub(hi, lo)).sqrt() * 0.5).max(1.0);
        let half = (FOV_Y * 0.5).tan();
        let half_min = half * aspect.min(1.0);
        self.dist = r / half_min.atan().sin() * 1.08;
    }

    fn near_far(&self) -> (f32, f32) {
        ((self.dist * 0.02).max(0.05), self.dist * 4.0 + 200.0)
    }

    /// Column-major view-projection matrix.
    pub fn view_proj(&self, aspect: f32) -> [[f32; 4]; 4] {
        let (r, u, d) = self.basis();
        let eye = self.eye();
        let f = 1.0 / (FOV_Y * 0.5).tan();
        let (n, fa) = self.near_far();
        let (sx, sy) = (f / aspect, f);
        let a = fa / (fa - n);
        // Rows of the 4×4: clip = M · (p, 1).
        let rows = [
            [r[0] * sx, r[1] * sx, r[2] * sx, -dot(r, eye) * sx],
            [u[0] * sy, u[1] * sy, u[2] * sy, -dot(u, eye) * sy],
            [d[0] * a, d[1] * a, d[2] * a, -dot(d, eye) * a - n * a],
            [d[0], d[1], d[2], -dot(d, eye)],
        ];
        // Transpose into columns.
        let mut c = [[0.0; 4]; 4];
        for (i, row) in rows.iter().enumerate() {
            for (j, v) in row.iter().enumerate() {
                c[j][i] = *v;
            }
        }
        c
    }

    /// World-space ray through a point in normalised device coords.
    pub fn ray(&self, ndc: [f32; 2], aspect: f32) -> (V3, V3) {
        let (r, u, d) = self.basis();
        let t = (FOV_Y * 0.5).tan();
        let dir = norm(add(d, add(mul(r, ndc[0] * t * aspect), mul(u, ndc[1] * t))));
        (self.eye(), dir)
    }
}

/// Model bounds in world units (studs included), or a default box.
pub fn model_bounds(model: Option<&Model>) -> (V3, V3) {
    match model.and_then(|m| m.bounds()) {
        Some((lo, hi)) => (
            [lo.x as f32, lo.y as f32 * PLATE, lo.z as f32],
            [hi.x as f32, hi.y as f32 * PLATE + 0.2, hi.z as f32],
        ),
        None => ([-4.0, 0.0, -4.0], [4.0, 2.0, 4.0]),
    }
}

/// A placement's axis-aligned cell box in world units.
pub fn placement_box(p: &Placement) -> (V3, V3) {
    let [fx, fz] = p.footprint();
    let o = p.origin;
    (
        [o.x as f32, o.y as f32 * PLATE, o.z as f32],
        [
            (o.x + fx) as f32,
            (o.y + p.height()) as f32 * PLATE,
            (o.z + fz) as f32,
        ],
    )
}

/// Ray / AABB slab test. Returns (t, face normal).
pub fn ray_box(o: V3, d: V3, lo: V3, hi: V3) -> Option<(f32, V3)> {
    let (mut t0, mut t1) = (f32::NEG_INFINITY, f32::INFINITY);
    let mut n = [0.0; 3];
    for a in 0..3 {
        if d[a].abs() < 1e-9 {
            if o[a] < lo[a] || o[a] > hi[a] {
                return None;
            }
            continue;
        }
        let inv = 1.0 / d[a];
        let (mut ta, mut tb) = ((lo[a] - o[a]) * inv, (hi[a] - o[a]) * inv);
        let mut na = -1.0;
        if ta > tb {
            std::mem::swap(&mut ta, &mut tb);
            na = 1.0;
        }
        if ta > t0 {
            t0 = ta;
            n = [0.0; 3];
            n[a] = na;
        }
        t1 = t1.min(tb);
        if t0 > t1 {
            return None;
        }
    }
    (t1 >= 0.0 && t0 >= 0.0).then_some((t0, n))
}

/// The first visible placement hit by a ray: (index, hit point, face normal).
pub fn pick(model: &Model, visible: &[bool], o: V3, d: V3) -> Option<(usize, V3, V3)> {
    let mut best: Option<(usize, f32, V3)> = None;
    for (i, p) in model.placements.iter().enumerate() {
        if !visible.get(i).copied().unwrap_or(true) {
            continue;
        }
        let (lo, hi) = placement_box(p);
        if let Some((t, n)) = ray_box(o, d, lo, hi)
            && best.is_none_or(|b| t < b.1)
        {
            best = Some((i, t, n));
        }
    }
    best.map(|(i, t, n)| (i, add(o, mul(d, t)), n))
}

/// Which parts are drawn and how (indexed like `model.placements`).
pub struct Visibility {
    pub visible: Vec<bool>,
    pub flags: Vec<u32>,
}

/// Visibility for build step `step` (0-based; `None` = whole model).
pub fn visibility(model: &Model, step: Option<usize>) -> Visibility {
    let n = model.placements.len();
    let Some(k) = step.filter(|_| !model.steps.is_empty()) else {
        return Visibility {
            visible: vec![true; n],
            flags: vec![0; n],
        };
    };
    let mut visible = vec![false; n];
    let mut flags = vec![0; n];
    for (si, s) in model.steps.iter().enumerate().take(k + 1) {
        for &i in s {
            if i < n {
                visible[i] = true;
                flags[i] = if si == k {
                    gpu::FLAG_HIGHLIGHT
                } else {
                    gpu::FLAG_DIM
                };
            }
        }
    }
    Visibility { visible, flags }
}

/// Column-major model matrix: LDraw part frame → world.
fn instance_matrix(p: &Placement) -> [[f32; 4]; 4] {
    let (pos, m) = p.ldraw_transform();
    let s = 1.0 / 20.0;
    // World = S · (R · v + t) with S = diag(s, -s, s).
    let sy = [s, -s, s];
    let mut c = [[0.0; 4]; 4];
    for j in 0..3 {
        for i in 0..3 {
            c[j][i] = sy[i] * m[i][j];
        }
    }
    c[3] = [pos[0] * s, -pos[1] * s, pos[2] * s, 1.0];
    c
}

pub fn rgba(c: Color32) -> [u8; 4] {
    [c.r(), c.g(), c.b(), c.a()]
}

fn box_lines(out: &mut Vec<LineVertex>, lo: V3, hi: V3, color: [u8; 4]) {
    let c = |i: u8| {
        [
            if i & 1 == 0 { lo[0] } else { hi[0] },
            if i & 2 == 0 { lo[1] } else { hi[1] },
            if i & 4 == 0 { lo[2] } else { hi[2] },
        ]
    };
    for (a, b) in [
        (0, 1),
        (2, 3),
        (4, 5),
        (6, 7),
        (0, 2),
        (1, 3),
        (4, 6),
        (5, 7),
        (0, 4),
        (1, 5),
        (2, 6),
        (3, 7),
    ] {
        out.push(LineVertex { pos: c(a), color });
        out.push(LineVertex { pos: c(b), color });
    }
}

pub struct SceneInput<'a> {
    pub model: Option<&'a Model>,
    pub palette: &'a Palette,
    pub meshes: &'a MeshLibrary,
    pub vis: &'a Visibility,
    pub selected: Option<usize>,
    pub ghost: Option<&'a Placement>,
    pub accent: Color32,
    pub edges: bool,
}

pub fn build_scene(generation: u64, inp: &SceneInput) -> Scene {
    // Batches by mesh key: a catalog part's id, or a printed part's content key (printed
    // ids like B001 repeat across models, and the GPU keeps meshes between scenes).
    let mut batches: HashMap<String, (Arc<sf_ldraw::PartMesh>, Vec<Instance>)> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    // Within one scene: printed brick / detail design → its mesh key.
    let mut printed_keys: HashMap<String, Option<String>> = HashMap::new();
    let mut overlay = Vec::new();
    if let Some(model) = inp.model {
        for (i, p) in model.placements.iter().enumerate() {
            if !inp.vis.visible.get(i).copied().unwrap_or(true) {
                continue;
            }
            let local = p.custom.as_deref().map(|def| match &def.kind {
                sf_core::CustomKind::Brick => sf_ldraw::brick_key(def),
                sf_core::CustomKind::Detail { mesh, .. } => format!("detail:{mesh}"),
            });
            let key = match &local {
                None => p.part.clone(),
                Some(l) => match printed_keys.get(l) {
                    Some(Some(k)) => k.clone(),
                    Some(None) => continue,
                    None => match inp.meshes.placement_mesh(p, model) {
                        Some((k, mesh)) => {
                            printed_keys.insert(l.clone(), Some(k.clone()));
                            if !batches.contains_key(&k) {
                                order.push(k.clone());
                                batches.insert(k.clone(), (mesh, Vec::new()));
                            }
                            k
                        }
                        None => {
                            printed_keys.insert(l.clone(), None);
                            continue;
                        }
                    },
                },
            };
            let entry = match batches.get_mut(&key) {
                Some(e) => e,
                None => {
                    let Some(mesh) = inp.meshes.mesh(&p.part) else {
                        continue;
                    };
                    order.push(key.clone());
                    batches.entry(key).or_insert((mesh, Vec::new()))
                }
            };
            let rgb = inp.palette.rgb(p.color);
            let mut flags = inp.vis.flags.get(i).copied().unwrap_or(0);
            if inp.selected == Some(i) {
                flags = gpu::FLAG_SELECTED;
                let (lo, hi) = placement_box(p);
                let e = 0.04;
                box_lines(
                    &mut overlay,
                    sub(lo, [e; 3]),
                    add(hi, [e; 3]),
                    rgba(inp.accent),
                );
            }
            entry.1.push(Instance {
                model: instance_matrix(p),
                color: [rgb[0], rgb[1], rgb[2], 255],
                flags,
            });
        }
    }
    if let Some(g) = inp.ghost {
        let (lo, hi) = placement_box(g);
        let e = 0.02;
        box_lines(
            &mut overlay,
            sub(lo, [e; 3]),
            add(hi, [e; 3]),
            rgba(inp.accent.gamma_multiply(0.9)),
        );
    }
    // Ground grid around the model.
    let (lo, hi) = model_bounds(inp.model);
    let pad = 6.0;
    let (x0, x1) = ((lo[0] - pad).floor() as i32, (hi[0] + pad).ceil() as i32);
    let (z0, z1) = ((lo[2] - pad).floor() as i32, (hi[2] + pad).ceil() as i32);
    let mut lines = Vec::new();
    for x in x0..=x1 {
        let a = if x % 4 == 0 { 46 } else { 22 };
        let c = [150, 156, 166, a];
        lines.push(LineVertex {
            pos: [x as f32, 0.0, z0 as f32],
            color: c,
        });
        lines.push(LineVertex {
            pos: [x as f32, 0.0, z1 as f32],
            color: c,
        });
    }
    for z in z0..=z1 {
        let a = if z % 4 == 0 { 46 } else { 22 };
        let c = [150, 156, 166, a];
        lines.push(LineVertex {
            pos: [x0 as f32, 0.0, z as f32],
            color: c,
        });
        lines.push(LineVertex {
            pos: [x1 as f32, 0.0, z as f32],
            color: c,
        });
    }
    let batches = order
        .into_iter()
        .filter_map(|id| batches.remove(&id).map(|(m, inst)| (id, m, inst)))
        .collect();
    Scene {
        generation,
        palette: inp.palette.clone(),
        batches,
        lines,
        overlay_lines: overlay,
        edges: inp.edges,
    }
}

/// Result of one frame of viewer interaction.
#[derive(Default)]
pub struct ViewerResponse {
    /// A click without dragging: (ray origin, ray dir).
    pub clicked: Option<(V3, V3)>,
    /// Hover ray, when the pointer is over the viewer.
    pub hover: Option<(V3, V3)>,
}

/// Draw the viewer into `rect` and handle orbit / pan / zoom.
pub fn show(
    ui: &mut egui::Ui,
    rect: Rect,
    cam: &mut Camera,
    scene: &Arc<Scene>,
    accent: Color32,
    time: f64,
    fake_click: Option<Pos2>,
) -> ViewerResponse {
    let resp = ui.interact(rect, ui.scope_id().with("viewer"), Sense::click_and_drag());
    let mut out = ViewerResponse::default();
    let aspect = (rect.width() / rect.height().max(1.0)).max(0.01);

    if resp.dragged_by(egui::PointerButton::Primary) {
        let d = resp.drag_delta();
        cam.yaw += d.x * 0.008;
        cam.pitch = (cam.pitch + d.y * 0.008).clamp(-1.45, std::f32::consts::FRAC_PI_2);
    }
    if resp.dragged_by(egui::PointerButton::Secondary)
        || resp.dragged_by(egui::PointerButton::Middle)
    {
        let d = resp.drag_delta();
        let (r, u, _) = cam.basis();
        let scale = 2.0 * cam.dist * (FOV_Y * 0.5).tan() / rect.height().max(1.0);
        cam.target = add(cam.target, add(mul(r, -d.x * scale), mul(u, d.y * scale)));
    }
    if resp.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y + i.zoom_delta().ln() * 200.0);
        if scroll.abs() > 0.0 {
            cam.dist = (cam.dist * (-scroll * 0.0015).exp()).clamp(1.0, 2000.0);
        }
    }
    let to_ray = |p: Pos2| {
        let ndc = [
            (p.x - rect.left()) / rect.width() * 2.0 - 1.0,
            1.0 - (p.y - rect.top()) / rect.height() * 2.0,
        ];
        cam.ray(ndc, aspect)
    };
    if resp.clicked()
        && let Some(p) = resp.interact_pointer_pos()
    {
        out.clicked = Some(to_ray(p));
    }
    if let Some(p) = fake_click.filter(|p| rect.contains(*p)) {
        out.clicked = Some(to_ray(p));
    }
    if let Some(p) = resp.hover_pos().or(fake_click) {
        out.hover = Some(to_ray(p));
    }

    // Background: soft vertical gradient.
    let painter = ui.painter_at(rect);
    let top = Color32::from_rgb(34, 37, 43);
    let bottom = Color32::from_rgb(20, 22, 26);
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));

    let (r, u, d) = cam.basis();
    let key = norm(add(add(mul(r, -0.35), mul(u, 0.55)), mul(d, -0.75)));
    let fill = norm(add(add(mul(r, 0.6), mul(u, -0.1)), mul(d, -0.5)));
    let lin = |c: u8| {
        let c = c as f32 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let srgb_target = ui
        .ctx()
        .data(|d| d.get_temp::<bool>(egui::Id::unique("fushion_srgb_target")))
        .unwrap_or(false);
    let eye = cam.eye();
    let globals = Globals {
        view_proj: cam.view_proj(aspect),
        eye: [eye[0], eye[1], eye[2], 1.0],
        key_dir: [key[0], key[1], key[2], 0.0],
        fill_dir: [fill[0], fill[1], fill[2], 0.0],
        accent: [lin(accent.r()), lin(accent.g()), lin(accent.b()), 1.0],
        params: [
            if srgb_target { 1.0 } else { 0.0 },
            0.85,
            (0.5 + 0.5 * (time * 4.0).sin()) as f32,
            0.0,
        ],
    };
    painter.add(egui_wgpu::Callback::new_paint_callback(
        rect,
        ViewerCallback {
            scene: scene.clone(),
            globals,
        },
    ));
    out
}

/// Small helper: screen size of the viewer for fitting.
pub fn aspect_of(rect: Rect) -> f32 {
    let s: Vec2 = rect.size();
    (s.x / s.y.max(1.0)).max(0.01)
}
