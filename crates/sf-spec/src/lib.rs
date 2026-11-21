//! The shape language Claude writes ("spec"), and its compiler to a coloured
//! voxel grid.
//!
//! A spec describes the subject in its own units with real proportions (one
//! unit up is as long as one unit across). The compiler scales it so the
//! longest dimension spans `resolution` studs, then samples it into grid cells
//! of 1 stud × 1 plate × 1 stud.
//!
//! Orientation: `+y` is up. The subject's **front faces `-z`** (the viewer
//! stands at `-z` looking toward `+z`), so `+x` is the viewer's right.
//!
//! See [`GUIDE`] for the text given to Claude.

mod geom;
mod relief;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use sf_core::{Cell, ColorId, Fill, ModelMode, Palette, VoxelGrid};

pub use geom::{Axis, Geom, Plane};

/// The guide shown to Claude (also served by the MCP server).
pub const GUIDE: &str = include_str!("../GUIDE.md");

/// Sub-samples per cell along x, y and z. 3 × 2 × 3 = 18 samples.
pub const SX: usize = 3;
pub const SY: usize = 2;
pub const SZ: usize = 3;
pub const SAMPLES: usize = SX * SY * SZ;

/// Plate height in studs (3.2 mm / 8 mm).
pub const PLATE_IN_STUDS: f32 = 0.4;

/// Largest grid the compiler will allocate (about 0.3 GB of working memory).
pub const MAX_CELLS: i64 = 16_000_000;
/// Caps on size fields, so a careless spec can't ask for a huge grid.
pub const MAX_BASE_MARGIN: u32 = 16;
pub const MAX_BASE_PLATES: u32 = 9;
pub const MAX_RELIEF_PLATES: u32 = 36;
pub const MAX_RELIEF_SIDE: usize = 512;

/// Refuse grids larger than [`MAX_CELLS`] before allocating them.
pub(crate) fn check_dims(dims: [i32; 3]) -> Result<()> {
    let n = dims.iter().try_fold(1i64, |acc, &d| if d < 0 { None } else { acc.checked_mul(d as i64) });
    match n {
        Some(n) if n <= MAX_CELLS => Ok(()),
        _ => bail!(
            "the model would need a {} × {} × {} grid, which is too large; lower the resolution or the sizes",
            dims[0], dims[1], dims[2]
        ),
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Spec {
    #[serde(default = "one")]
    pub version: u32,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub mode: ModelMode,
    /// Longest dimension of the model in studs. If absent, the engine picks
    /// the largest size that fits `budget`.
    #[serde(default)]
    pub resolution: Option<u32>,
    /// Maximum number of parts.
    #[serde(default)]
    pub budget: Option<u32>,
    /// Colour for shapes that don't name one.
    #[serde(default)]
    pub default_color: Option<String>,
    #[serde(default)]
    pub shapes: Vec<ShapeSpec>,
    #[serde(default)]
    pub relief: Option<relief::ReliefSpec>,
    /// Optional baseplate under the model.
    #[serde(default)]
    pub base: Option<BaseSpec>,
    /// Printed detail parts (flower heads, petals, eyes, beaks…) placed on the model.
    /// Each names a design made beforehand (`make_detail`); the engine rests it on the
    /// body under `at`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub details: Vec<DetailUse>,
    /// Free-form notes Claude keeps between rounds (ignored by the compiler).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// One use of a detail design in a spec.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DetailUse {
    /// The design's name.
    pub design: String,
    /// Where the centre of the design's socket base should sit, in spec units (relief:
    /// `[column, plates, row]` counted from the picture's top left). The engine rests the
    /// detail on the highest body surface under its base at or below this height.
    pub at: [f32; 3],
    /// Quarter turns about the vertical axis (0–3).
    #[serde(default)]
    pub turn: u8,
    /// Colour (default: the spec's `default_color`): the design's "main" role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Colours of the design's other roles, e.g. `{"centre": "Yellow", "leaf": "Green"}`.
    /// Roles left out keep the design's defaults.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub colors: std::collections::BTreeMap<String, String>,
}

/// A detail use mapped onto the grid: `x`, `z` in studs and `y` in plates, as fractional
/// grid coordinates (cell `(x, y, z)` spans `[x, x + 1)` and so on).
#[derive(Clone, Debug, PartialEq)]
pub struct GridDetail {
    pub design: String,
    pub at: [f32; 3],
    pub turn: u8,
    pub color: ColorId,
    /// Colours by role (besides "main", which is `color`), resolved.
    pub colors: std::collections::BTreeMap<String, ColorId>,
}

/// Maps spec coordinates onto the grid, kept through cropping and the base.
#[derive(Clone, Debug, PartialEq)]
pub enum GridMap {
    /// Shapes: `grid = (p − origin) / (units per stud, per plate, per stud) + shift`.
    Shapes { origin: [f32; 3], ups: f32, upp: f32, shift: [i32; 3] },
    /// Reliefs: `[column, plates, row]` → `(column, plates, depth − 1 − row) + shift`.
    Relief { depth: i32, shift: [i32; 3] },
}

impl GridMap {
    /// The grid position of spec point `p`.
    pub fn to_grid(&self, p: [f32; 3]) -> [f32; 3] {
        match self {
            GridMap::Shapes { origin, ups, upp, shift } => [
                (p[0] - origin[0]) / ups + shift[0] as f32,
                (p[1] - origin[1]) / upp + shift[1] as f32,
                (p[2] - origin[2]) / ups + shift[2] as f32,
            ],
            GridMap::Relief { depth, shift } => [
                p[0] + 0.5 + shift[0] as f32,
                p[1] + shift[1] as f32,
                (*depth - 1) as f32 - p[2] + 0.5 + shift[2] as f32,
            ],
        }
    }

    fn shift_mut(&mut self) -> &mut [i32; 3] {
        match self {
            GridMap::Shapes { shift, .. } | GridMap::Relief { shift, .. } => shift,
        }
    }
}

fn one() -> u32 {
    1
}

pub use relief::{ReliefHeights, ReliefSource, ReliefSpec};

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Op {
    #[default]
    Add,
    Subtract,
    /// Recolour cells that are already filled, without adding any.
    Paint,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShapeSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(flatten)]
    pub geom: Geom,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default)]
    pub op: Op,
    /// Euler rotation in degrees (applied X, then Y, then Z) about the shape's
    /// centre. Ignored by `capsule` and `extrude`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotate: Option<[f32; 3]>,
    /// Also add a copy mirrored across `x = 0`.
    #[serde(default)]
    pub mirror_x: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BaseSpec {
    #[serde(default = "base_color")]
    pub color: String,
    /// Studs of margin around the model's footprint.
    #[serde(default = "base_margin")]
    pub margin: u32,
    /// Thickness in plates.
    #[serde(default = "one")]
    pub plates: u32,
}

fn base_color() -> String {
    "Dark Bluish Gray".into()
}
fn base_margin() -> u32 {
    1
}

impl Spec {
    pub fn from_json(s: &str) -> Result<Self> {
        let spec: Spec = serde_json::from_str(s).map_err(|e| anyhow::anyhow!("spec is not valid JSON for the spec format: {e}"))?;
        if spec.shapes.is_empty() && spec.relief.is_none() {
            bail!("spec has no `shapes` and no `relief`; nothing to build");
        }
        Ok(spec)
    }
}

/// Output of [`compile`].
#[derive(Clone, Debug)]
pub struct Compiled {
    pub grid: VoxelGrid,
    /// Per-cell sample masks (bit `i` set = sample `i` inside), parallel to the
    /// grid in `[y][z][x]` order. Used to fit slopes.
    pub samples: Vec<u32>,
    pub warnings: Vec<String>,
    /// Model units per stud (0 for reliefs, which are in studs already).
    pub units_per_stud: f32,
    /// Spec coordinates → grid.
    pub map: GridMap,
    /// The spec's detail uses, mapped onto the grid.
    pub details: Vec<GridDetail>,
}

impl Compiled {
    pub fn sample_mask(&self, c: Cell) -> u32 {
        if !self.grid.in_bounds(c) {
            return 0;
        }
        let [dx, _, dz] = self.grid.dims;
        self.samples[((c.y * dz + c.z) * dx + c.x) as usize]
    }
}

/// Bit index of sample `(i, j, k)` (x, y, z) inside a cell.
pub const fn sample_bit(i: usize, j: usize, k: usize) -> u32 {
    1 << ((j * SZ + k) * SX + i)
}

#[derive(Clone, Debug, Default)]
pub struct CompileOptions<'a> {
    /// Override the spec's resolution.
    pub resolution: Option<u32>,
    /// The source image, for `relief` specs with `"source": "image"`.
    pub image: Option<&'a image::RgbaImage>,
}

/// Compile a spec into a voxel grid.
pub fn compile(spec: &Spec, palette: &Palette, opts: &CompileOptions) -> Result<Compiled> {
    let mut warnings = Vec::new();
    let default_color = match &spec.default_color {
        Some(name) => resolve_color(palette, name, &mut warnings),
        None => sf_core::color::FILLER,
    };
    let mut out = if let Some(relief) = &spec.relief {
        relief::compile(relief, palette, opts, default_color, &mut warnings)?
    } else {
        let res = opts.resolution.or(spec.resolution).unwrap_or(24).clamp(2, 160);
        compile_shapes(&spec.shapes, palette, res, default_color, &mut warnings)?
    };
    if let Some(base) = &spec.base {
        let color = resolve_color(palette, &base.color, &mut warnings);
        if base.margin > MAX_BASE_MARGIN || base.plates > MAX_BASE_PLATES {
            warnings.push(format!(
                "base margin is capped at {MAX_BASE_MARGIN} studs and thickness at {MAX_BASE_PLATES} plates"
            ));
        }
        add_base(&mut out, base, color)?;
    }
    for d in &spec.details {
        let mut color = match &d.color {
            Some(name) => resolve_color(palette, name, &mut warnings),
            None => default_color,
        };
        let mut colors = std::collections::BTreeMap::new();
        for (role, name) in &d.colors {
            let c = resolve_color(palette, name, &mut warnings);
            if role == "main" {
                color = c;
            } else {
                colors.insert(role.clone(), c);
            }
        }
        if !d.at.iter().all(|v| v.is_finite()) {
            bail!("detail `{}`: `at` must be three finite numbers", d.design);
        }
        let at = out.map.to_grid(d.at);
        out.details.push(GridDetail { design: d.design.clone(), at, turn: d.turn % 4, color, colors });
    }
    out.grid.mark_interior();
    out.warnings = warnings;
    Ok(out)
}

pub(crate) fn resolve_color(palette: &Palette, name: &str, warnings: &mut Vec<String>) -> ColorId {
    match palette.resolve(name) {
        Some(c) => c,
        None => {
            warnings.push(format!("unknown colour `{name}`; used Light Bluish Gray. Use a palette name or #RRGGBB."));
            sf_core::color::FILLER
        }
    }
}

fn compile_shapes(
    shapes: &[ShapeSpec],
    palette: &Palette,
    resolution: u32,
    default_color: ColorId,
    warnings: &mut Vec<String>,
) -> Result<Compiled> {
    let prims = geom::flatten(shapes, palette, default_color, warnings)?;
    let Some((lo, hi)) = geom::bounds(&prims) else {
        bail!("the spec adds no volume (every shape is subtract or paint)");
    };
    let ext = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
    let longest = ext[0].max(ext[1]).max(ext[2]);
    if longest <= 0.0 || !longest.is_finite() {
        bail!("shapes have zero or invalid size");
    }
    let ups = longest / resolution as f32; // units per stud
    let upp = ups * PLATE_IN_STUDS; // units per plate
    let dims = [
        ((ext[0] / ups).ceil() as i32).max(1),
        ((ext[1] / upp).ceil() as i32).max(1),
        ((ext[2] / ups).ceil() as i32).max(1),
    ];
    check_dims(dims)?;
    // Centre the model in the grid horizontally; sit it on y = 0.
    let origin = [
        lo[0] - (dims[0] as f32 * ups - ext[0]) / 2.0,
        lo[1],
        lo[2] - (dims[2] as f32 * ups - ext[2]) / 2.0,
    ];
    let n = (dims[0] * dims[1] * dims[2]) as usize;
    let mut masks = vec![0u32; n];
    let mut colors = vec![default_color; n];
    let mut cover = vec![0u8; n];
    let idx = |x: i32, y: i32, z: i32| ((y * dims[2] + z) * dims[0] + x) as usize;
    let half = (SAMPLES / 2) as u32;

    for p in &prims {
        let (plo, phi) = p.aabb();
        // Paint is dilated by half a cell so thin details (eyes, stripes) survive.
        let pad = if p.op == Op::Paint { 0.5 } else { 0.0 };
        let x0 = (((plo[0] - origin[0]) / ups - pad).floor() as i32).max(0);
        let x1 = (((phi[0] - origin[0]) / ups + pad).ceil() as i32).min(dims[0]);
        let y0 = (((plo[1] - origin[1]) / upp - pad).floor() as i32).max(0);
        let y1 = (((phi[1] - origin[1]) / upp + pad).ceil() as i32).min(dims[1]);
        let z0 = (((plo[2] - origin[2]) / ups - pad).floor() as i32).max(0);
        let z1 = (((phi[2] - origin[2]) / ups + pad).ceil() as i32).min(dims[2]);
        let mut touched = 0usize;
        // Best partial coverage, so shapes smaller than a cell still show.
        let mut partial: Vec<(usize, u32)> = Vec::new();
        for y in y0..y1 {
            for z in z0..z1 {
                for x in x0..x1 {
                    let i = idx(x, y, z);
                    let m = if p.op == Op::Paint {
                        // Dilated test: any of the cell centre ± half a cell.
                        let c = [
                            origin[0] + (x as f32 + 0.5) * ups,
                            origin[1] + (y as f32 + 0.5) * upp,
                            origin[2] + (z as f32 + 0.5) * ups,
                        ];
                        let hit = [-0.5f32, 0.0, 0.5].iter().any(|&d| {
                            [-0.5f32, 0.0, 0.5].iter().any(|&e| {
                                [-0.5f32, 0.0, 0.5].iter().any(|&f| p.contains([c[0] + d * ups, c[1] + e * upp, c[2] + f * ups]))
                            })
                        });
                        if hit { u32::MAX } else { 0 }
                    } else {
                        let mut m = 0u32;
                        for j in 0..SY {
                            for k in 0..SZ {
                                for ii in 0..SX {
                                    let pt = [
                                        origin[0] + (x as f32 + (ii as f32 + 0.5) / SX as f32) * ups,
                                        origin[1] + (y as f32 + (j as f32 + 0.5) / SY as f32) * upp,
                                        origin[2] + (z as f32 + (k as f32 + 0.5) / SZ as f32) * ups,
                                    ];
                                    if p.contains(pt) {
                                        m |= sample_bit(ii, j, k);
                                    }
                                }
                            }
                        }
                        m
                    };
                    if m == 0 {
                        continue;
                    }
                    let cov = m.count_ones();
                    match p.op {
                        Op::Add => {
                            masks[i] |= m;
                            if cov >= half || cov as u8 >= cover[i] {
                                colors[i] = p.color;
                                cover[i] = cov as u8;
                            }
                            if cov >= half {
                                touched += 1;
                            } else {
                                partial.push((i, cov));
                            }
                        }
                        Op::Subtract => {
                            masks[i] &= !m;
                            touched += 1;
                        }
                        Op::Paint => {
                            if masks[i].count_ones() >= half {
                                colors[i] = p.color;
                                touched += 1;
                            }
                        }
                    }
                }
            }
        }
        if touched == 0 && p.op == Op::Add && !partial.is_empty() {
            // Smaller than a cell: draw it as the cells it covers most.
            let best = partial.iter().map(|&(_, c)| c).max().unwrap_or(0);
            let full = u32::MAX >> (32 - SAMPLES);
            let mut n = 0;
            for &(i, c) in &partial {
                if c * 2 >= best && masks[i].count_ones() < half {
                    masks[i] = full;
                    colors[i] = p.color;
                    n += 1;
                }
            }
            warnings.push(format!(
                "shape `{}` is smaller than one stud at {resolution} studs; drawn as {n} cell(s). Make it bigger for more detail",
                p.label
            ));
        } else if touched == 0 {
            warnings.push(format!(
                "shape `{}` has no effect at {resolution} studs; it is too small or misses the model",
                p.label
            ));
        }
    }

    let mut grid = VoxelGrid::new(dims);
    for y in 0..dims[1] {
        for z in 0..dims[2] {
            for x in 0..dims[0] {
                let i = idx(x, y, z);
                if masks[i].count_ones() >= half {
                    grid.set(Cell::new(x, y, z), Fill::Solid(colors[i]));
                }
            }
        }
    }
    let mut out = Compiled {
        grid,
        samples: masks,
        warnings: Vec::new(),
        units_per_stud: ups,
        map: GridMap::Shapes { origin, ups, upp, shift: [0; 3] },
        details: Vec::new(),
    };
    crop(&mut out);
    Ok(out)
}

/// Shrink the grid to the bounding box of filled cells (keeping y = 0 as the
/// lowest filled layer).
fn crop(c: &mut Compiled) {
    let [dx, dy, dz] = c.grid.dims;
    let (mut lo, mut hi) = ([i32::MAX; 3], [i32::MIN; 3]);
    for (cell, _) in c.grid.filled() {
        lo = [lo[0].min(cell.x), lo[1].min(cell.y), lo[2].min(cell.z)];
        hi = [hi[0].max(cell.x), hi[1].max(cell.y), hi[2].max(cell.z)];
    }
    if lo[0] == i32::MAX {
        return;
    }
    let nd = [hi[0] - lo[0] + 1, hi[1] - lo[1] + 1, hi[2] - lo[2] + 1];
    if nd == [dx, dy, dz] {
        return;
    }
    let mut g = VoxelGrid::new(nd);
    let mut s = vec![0u32; (nd[0] * nd[1] * nd[2]) as usize];
    for y in 0..nd[1] {
        for z in 0..nd[2] {
            for x in 0..nd[0] {
                let src = Cell::new(x + lo[0], y + lo[1], z + lo[2]);
                g.set(Cell::new(x, y, z), c.grid.get(src));
                s[((y * nd[2] + z) * nd[0] + x) as usize] = c.sample_mask(src);
            }
        }
    }
    c.grid = g;
    c.samples = s;
    let shift = c.map.shift_mut();
    for k in 0..3 {
        shift[k] -= lo[k];
    }
}

fn add_base(c: &mut Compiled, base: &BaseSpec, color: ColorId) -> Result<()> {
    let m = base.margin.min(MAX_BASE_MARGIN) as i32;
    let t = base.plates.clamp(1, MAX_BASE_PLATES) as i32;
    let [dx, dy, dz] = c.grid.dims;
    let nd = [dx + 2 * m, dy + t, dz + 2 * m];
    check_dims(nd)?;
    let mut g = VoxelGrid::new(nd);
    let mut s = vec![0u32; (nd[0] * nd[1] * nd[2]) as usize];
    for y in 0..nd[1] {
        for z in 0..nd[2] {
            for x in 0..nd[0] {
                let i = ((y * nd[2] + z) * nd[0] + x) as usize;
                if y < t {
                    g.set(Cell::new(x, y, z), Fill::Solid(color));
                    s[i] = u32::MAX >> (32 - SAMPLES);
                } else {
                    let src = Cell::new(x - m, y - t, z - m);
                    g.set(Cell::new(x, y, z), c.grid.get(src));
                    s[i] = c.sample_mask(src);
                }
            }
        }
    }
    c.grid = g;
    c.samples = s;
    let shift = c.map.shift_mut();
    shift[0] += m;
    shift[1] += t;
    shift[2] += m;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(json: &str) -> Spec {
        Spec::from_json(json).unwrap()
    }

    #[test]
    fn box_fills_expected_cells() {
        let s = spec(r#"{"shapes":[{"shape":"box","center":[0,0,0],"size":[4,2,2],"color":"Red"}]}"#);
        let c = compile(&s, &Palette::builtin(), &CompileOptions { resolution: Some(4), image: None }).unwrap();
        // 4 units → 4 studs; 2 units high → 2 studs → 5 plates; 2 deep → 2 studs.
        assert_eq!(c.grid.dims, [4, 5, 2]);
        assert_eq!(c.grid.count_filled(), 40);
        assert_eq!(c.grid.get(Cell::new(0, 0, 0)), Fill::Solid(4));
    }

    #[test]
    fn sphere_is_round_and_hollow_inside_marked() {
        let s = spec(r#"{"shapes":[{"shape":"ellipsoid","center":[0,5,0],"size":[10,10,10],"color":"Orange"}]}"#);
        let c = compile(&s, &Palette::builtin(), &CompileOptions { resolution: Some(10), image: None }).unwrap();
        assert_eq!(c.grid.dims[0], 10);
        assert!(c.grid.get(Cell::new(0, 0, 0)) == Fill::Empty, "corner should be empty");
        assert_eq!(c.grid.get(Cell::new(5, 12, 5)), Fill::Interior);
    }

    #[test]
    fn subtract_and_paint() {
        let s = spec(
            r#"{"shapes":[
            {"shape":"box","center":[0,0,0],"size":[6,6,6],"color":"White"},
            {"shape":"box","center":[0,0,-3],"size":[2,2,2],"op":"subtract"},
            {"shape":"ellipsoid","center":[2,2,-3],"size":[0.6,0.6,0.6],"color":"Black","op":"paint"}
        ]}"#,
        );
        let c = compile(&s, &Palette::builtin(), &CompileOptions { resolution: Some(6), image: None }).unwrap();
        // Hole in the front face centre.
        assert_eq!(c.grid.get(Cell::new(3, 7, 0)), Fill::Empty);
        // Small painted eye survives thanks to dilation.
        assert!(c.grid.filled().any(|(_, f)| f == Fill::Solid(0)), "eye paint vanished");
        assert!(c.warnings.is_empty(), "{:?}", c.warnings);
    }

    #[test]
    fn mirror_and_groups() {
        let s = spec(
            r#"{"shapes":[
            {"shape":"box","center":[0,0.5,0],"size":[8,1,2],"color":"Tan"},
            {"shape":"box","center":[3.5,2,0],"size":[1,2,2],"color":"Red","mirror_x":true},
            {"shape":"group","at":[0,1,0],"children":[{"shape":"box","center":[0,0.5,0],"size":[2,1,2],"color":"Blue"}]}
        ]}"#,
        );
        let c = compile(&s, &Palette::builtin(), &CompileOptions { resolution: Some(8), image: None }).unwrap();
        let reds: Vec<_> = c.grid.filled().filter(|(_, f)| *f == Fill::Solid(4)).map(|(c, _)| c.x).collect();
        assert!(reds.contains(&0) && reds.contains(&7), "mirrored copy missing: {reds:?}");
        assert!(c.grid.filled().any(|(_, f)| f == Fill::Solid(1)));
    }

    #[test]
    fn tiny_shape_warns() {
        let s = spec(
            r#"{"shapes":[{"shape":"box","center":[0,0,0],"size":[10,10,10]},
            {"id":"speck","shape":"box","center":[5.3,0,0],"size":[0.6,0.6,0.6],"color":"Red"}]}"#,
        );
        let c = compile(&s, &Palette::builtin(), &CompileOptions { resolution: Some(10), image: None }).unwrap();
        assert!(c.warnings.iter().any(|w| w.contains("speck")), "{:?}", c.warnings);
        // A tiny add still shows as at least one cell.
        assert!(c.grid.filled().any(|(_, f)| f == Fill::Solid(4)), "tiny shape vanished");
    }

    #[test]
    fn base_adds_plates() {
        let s = spec(
            r#"{"shapes":[{"shape":"box","center":[0,0,0],"size":[2,2,2],"color":"Red"}],
            "base":{"color":"Green","margin":1,"plates":1}}"#,
        );
        let c = compile(&s, &Palette::builtin(), &CompileOptions { resolution: Some(2), image: None }).unwrap();
        assert_eq!(c.grid.dims[0], 4);
        assert_eq!(c.grid.get(Cell::new(0, 0, 0)), Fill::Solid(2));
        assert_eq!(c.grid.get(Cell::new(1, 1, 1)), Fill::Solid(4));
    }

    #[test]
    fn hostile_sizes_are_capped_not_fatal() {
        let p = Palette::builtin();
        let opts = CompileOptions { resolution: Some(8), image: None };
        for json in [
            r#"{"shapes":[{"shape":"box","center":[0,0,0],"size":[2,2,2]}],"base":{"margin":100000,"plates":4294967295}}"#,
            r#"{"mode":"relief","relief":{"rows":["ab"],"legend":{"a":"Red","b":"Blue"},"base_plates":4294967295}}"#,
            r#"{"mode":"relief","relief":{"rows":["ab"],"legend":{"a":"Red","b":"Blue"},"heights":{"luminance":{"max_plates":2000000000}}}}"#,
        ] {
            let c = compile(&Spec::from_json(json).unwrap(), &p, &opts).unwrap();
            let n: i64 = c.grid.dims.iter().map(|&d| d as i64).product();
            assert!(n < 100_000, "grid too big: {:?}", c.grid.dims);
        }
        let wide = format!(r#"{{"mode":"relief","relief":{{"rows":["{}"],"legend":{{"a":"Red"}}}}}}"#, "a".repeat(5000));
        assert!(compile(&Spec::from_json(&wide).unwrap(), &p, &opts).is_err());
    }


    #[test]
    fn details_map_onto_the_grid_through_crop_and_base() {
        // A 4 × 2 × 4 box at the origin, resolution 4: one unit per stud, 0.4 units per plate.
        let s = spec(
            r#"{"shapes":[{"shape":"box","center":[0,1,0],"size":[4,2,4],"color":"Red"}],
            "base":{"margin":1,"plates":1},
            "details":[{"design":"flower","at":[0,2,0],"turn":5,"color":"Yellow","colors":{"centre":"Orange"}}]}"#,
        );
        let c = compile(&s, &Palette::builtin(), &CompileOptions { resolution: Some(4), image: None }).unwrap();
        assert_eq!(c.details.len(), 1);
        let d = &c.details[0];
        assert_eq!(d.turn, 1);
        assert_eq!(d.color, 14);
        assert_eq!(d.colors.get("centre"), Some(&25));
        // Box spans x −2..2 → grid 0..4, + 1 margin: centre at 3. Top at 2 units = 5 plates,
        // + 1 base plate = 6.
        assert!((d.at[0] - 3.0).abs() < 1e-4 && (d.at[2] - 3.0).abs() < 1e-4, "{:?}", d.at);
        assert!((d.at[1] - 6.0).abs() < 1e-4, "{:?}", d.at);
        // Relief: column 2, row 0 of a 3-row picture → x 2.5, z 2.5.
        let r = spec(
            r#"{"mode":"relief","relief":{"rows":["abc","abc","abc"],"legend":{"a":"Red","b":"Blue","c":"Red"}},
            "details":[{"design":"x","at":[2,1,0]}]}"#,
        );
        let c = compile(&r, &Palette::builtin(), &CompileOptions::default()).unwrap();
        assert_eq!(c.details[0].at, [2.5, 1.0, 2.5]);
    }
}
