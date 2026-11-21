//! Step pictures: the model so far with earlier parts ghosted, seen from whichever iso
//! corner shows the new parts best.
//!
//! sf-render has one iso view (from the front-left). The other corners come from turning
//! the whole model a quarter turn at a time about the vertical axis before rendering.
//! Ghosting and the visibility test only change colours, through extra palette entries.

use image::RgbaImage;
use sf_core::{Cell, ColorId, Model, Palette, Placement, Rot, color::Color};
use sf_ldraw::MeshLibrary;
use sf_render::{RenderOptions, View, render_view};
use std::collections::HashMap;

/// First colour code used for made-up colours (ghosts, masks). LDraw codes stay far below.
const SCRATCH_CODE: ColorId = 50_000;

/// How pale earlier parts are: 0 = their own colour, 1 = the ghost grey.
const GHOST: f32 = 0.6;
/// How much of its hue a ghost keeps (the rest goes grey), so a pale red can't pass for pink.
const GHOST_HUE: f32 = 0.22;
const GHOST_GREY: [u8; 3] = [236, 237, 240];

/// A copy of `p` turned `turns` quarter turns about the vertical axis through the grid
/// origin, the same way `Rot` turns a part: grid cell `(x, z)` goes to `(z, -x - 1)` per
/// turn.
pub fn turn_placement(p: &Placement, turns: u8) -> Placement {
    let mut q = p.clone();
    for _ in 0..turns % 4 {
        let [fx, _] = q.footprint();
        let o = q.origin;
        q.origin = Cell::new(o.z, o.y, -(o.x + fx));
        q.rot = q.rot.then(Rot(1));
    }
    q
}

/// The model turned `turns` quarter turns (placements only; steps and meshes kept).
pub fn turn_model(m: &Model, turns: u8) -> Model {
    let mut out = m.clone();
    out.placements = m.placements.iter().map(|p| turn_placement(p, turns)).collect();
    out
}

/// A palette with extra entries, handing out fresh codes for made-up colours.
pub struct ScratchPalette {
    pub palette: Palette,
    next: ColorId,
    ghosts: HashMap<ColorId, ColorId>,
}

impl ScratchPalette {
    pub fn new(base: &Palette) -> Self {
        Self { palette: base.clone(), next: SCRATCH_CODE, ghosts: HashMap::new() }
    }

    /// A new colour with this RGB.
    pub fn add(&mut self, rgb: [u8; 3]) -> ColorId {
        let code = self.next;
        self.next += 1;
        self.palette.colors.push(Color { code, name: format!("scratch {code}"), rgb, bricklink: None });
        code
    }

    /// The pale stand-in for `code`.
    pub fn ghost(&mut self, code: ColorId) -> ColorId {
        if let Some(&g) = self.ghosts.get(&code) {
            return g;
        }
        let c = self.palette.rgb(code).map(f32::from);
        let lum = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        let rgb = [0, 1, 2].map(|i| {
            let desat = lum + (c[i] - lum) * GHOST_HUE;
            (desat * (1.0 - GHOST) + GHOST_GREY[i] as f32 * GHOST).round() as u8
        });
        let g = self.add(rgb);
        self.ghosts.insert(code, g);
        g
    }
}

/// The model up to and including step `k`, with parts from earlier steps ghosted.
/// Returns the model and the indices (into the new model) of this step's parts.
pub fn step_model(m: &Model, steps: &[Vec<usize>], k: usize, pal: &mut ScratchPalette, ghost: bool) -> (Model, Vec<usize>) {
    let mut out = Model { title: m.title.clone(), mode: m.mode, printable: m.printable, meshes: m.meshes.clone(), details: m.details.clone(), ..Default::default() };
    for s in &steps[..k] {
        for &i in s {
            if let Some(p) = m.placements.get(i) {
                let mut p = p.clone();
                if ghost {
                    p.color = pal.ghost(p.color);
                }
                out.placements.push(p);
            }
        }
    }
    let first_new = out.placements.len();
    out.placements.extend(steps[k].iter().filter_map(|&i| m.placements.get(i).cloned()));
    let new = (first_new..out.placements.len()).collect();
    (out, new)
}

/// Pixels of the `new` parts visible in an iso render of `m` turned `turns` times.
pub fn visible_new(m: &Model, new: &[usize], turns: u8, meshes: &MeshLibrary, size: u32) -> usize {
    let mut pal = ScratchPalette::new(&Palette { colors: Vec::new() });
    let red = pal.add([255, 0, 0]);
    let green = pal.add([0, 255, 0]);
    let mut mask = m.clone();
    for (i, p) in mask.placements.iter_mut().enumerate() {
        p.color = if new.contains(&i) { red } else { green };
    }
    let img = render_view(&turn_model(&mask, turns), &pal.palette, meshes, View::Iso, &mask_opts(size));
    count_red(&img)
}

fn mask_opts(size: u32) -> RenderOptions {
    RenderOptions { view_size: size, edges: false, background: [0, 0, 255] }
}

fn count_red(img: &RgbaImage) -> usize {
    img.pixels().filter(|p| p[0] > 60 && p[1] < 30 && p[2] < 30).count()
}

/// The quarter turn (0–3) whose iso view shows the most of the `new` parts. Turn 0
/// (front-left) is kept unless another shows clearly more (25 %).
pub fn best_turn(m: &Model, new: &[usize], meshes: &MeshLibrary) -> u8 {
    if new.is_empty() || m.placements.is_empty() {
        return 0;
    }
    let seen: Vec<usize> = (0..4).map(|t| visible_new(m, new, t, meshes, 160)).collect();
    let (best, &most) = seen.iter().enumerate().max_by_key(|(_, v)| **v).unwrap_or((0, &0));
    if most as f32 > seen[0] as f32 * 1.25 { best as u8 } else { 0 }
}

/// Render `m` from iso after turning it.
pub fn render_turned(m: &Model, palette: &Palette, meshes: &MeshLibrary, turns: u8, size: u32) -> RgbaImage {
    let opts = RenderOptions { view_size: size, edges: true, background: [255, 255, 255] };
    render_view(&turn_model(m, turns), palette, meshes, View::Iso, &opts)
}

/// Crop the white border around a render, keeping a small margin.
pub fn trim(img: &RgbaImage, margin: u32) -> RgbaImage {
    let (w, h) = img.dimensions();
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for (x, y, p) in img.enumerate_pixels() {
        if p[0] < 250 || p[1] < 250 || p[2] < 250 {
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
    }
    if x0 > x1 {
        return img.clone();
    }
    let (x0, y0) = (x0.saturating_sub(margin), y0.saturating_sub(margin));
    let (x1, y1) = ((x1 + margin).min(w - 1), (y1 + margin).min(h - 1));
    image::imageops::crop_imm(img, x0, y0, x1 - x0 + 1, y1 - y0 + 1).to_image()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sf_core::{CustomDef, CustomKind};
    use std::collections::BTreeSet;

    fn cells(p: &Placement) -> BTreeSet<(i32, i32, i32)> {
        p.cells().map(|c| (c.x, c.y, c.z)).collect()
    }

    #[test]
    fn turning_moves_cells_like_rot() {
        let l = CustomDef {
            id: "B001".into(),
            kind: CustomKind::Brick,
            colors: Default::default(),
            cells: vec![(0, 0), (1, 0), (2, 0), (0, 1)],
            plates: 3,
            studs: vec![],
            units: vec![],
            sockets: vec![],
        };
        let parts = [
            Placement::new("3001", 4, Cell::new(3, 0, 5), Rot(0)),
            Placement::new("3040b", 4, Cell::new(-2, 3, 1), Rot(1)),
            Placement::printed(l, 4, Cell::new(1, 0, 2), Rot(3)),
        ];
        for p in &parts {
            for t in 1..4u8 {
                let q = turn_placement(p, t);
                let mut want: BTreeSet<(i32, i32, i32)> = BTreeSet::new();
                for c in cells(p) {
                    let (mut x, mut z) = (c.0, c.2);
                    for _ in 0..t {
                        (x, z) = (z, -x - 1);
                    }
                    want.insert((x, c.1, z));
                }
                assert_eq!(cells(&q), want, "{} turned {t}", p.part);
                // Studs turn with it too.
                let mut studs: Vec<(i32, i32)> = p
                    .top_studs()
                    .into_iter()
                    .map(|(mut x, mut z)| {
                        for _ in 0..t {
                            (x, z) = (z, -x - 1);
                        }
                        (x, z)
                    })
                    .collect();
                let mut got = q.top_studs();
                studs.sort();
                got.sort();
                assert_eq!(got, studs, "{} studs turned {t}", p.part);
            }
            assert_eq!(turn_placement(p, 4), *p);
        }
    }

    #[test]
    fn ghosts_are_paler_and_cached() {
        let mut pal = ScratchPalette::new(&Palette::builtin());
        let g = pal.ghost(0);
        assert_eq!(pal.ghost(0), g);
        let (black, ghost) = (pal.palette.rgb(0), pal.palette.rgb(g));
        let sum = |c: [u8; 3]| c.iter().map(|&v| v as u32).sum::<u32>();
        assert!(sum(ghost) > sum(black) + 300, "{ghost:?} vs {black:?}");
        // A ghost red is greyish, not pink: its channels sit close together.
        let r = pal.ghost(4);
        let c = pal.palette.rgb(r);
        let spread = c.iter().max().unwrap() - c.iter().min().unwrap();
        assert!(spread < 40, "{c:?}");
    }
}
