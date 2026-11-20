//! The curated part catalog.
//!
//! Every part is described in **our** frame at rotation 0: `size = [x, z]` in
//! studs and `height` in plates. Slopes fall toward `+x` at rotation 0.
//!
//! `ldraw_yaw` and `ldraw_offset` map that description onto the real LDraw
//! geometry. They are checked against the LDraw library's bounding boxes by
//! the tests in `sf-ldraw` (`catalog_frames_match_ldraw`).

use serde::Serialize;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Shape {
    /// Rectangular brick or plate with a stud on every top cell.
    Box,
    /// Rectangular with a smooth top (no studs).
    Tile,
    /// Slope falling toward `+x` at rotation 0. Columns `x < high` are full
    /// height with studs; the rest are sloped and studless. `high = 0` is a
    /// cheese slope (no studs at all).
    Slope { high: u8 },
    /// Inverted slope: flat studded top, but only columns `x < high` reach the
    /// bottom and connect to studs below; the rest overhang.
    InvertedSlope { high: u8 },
}

#[derive(Clone, Debug, Serialize)]
pub struct PartDef {
    /// LDraw file stem, e.g. `"3001"`.
    pub id: &'static str,
    pub name: &'static str,
    /// Footprint `[x, z]` in studs at rotation 0.
    pub size: [u8; 2],
    /// Height in plates.
    pub height: u8,
    pub shape: Shape,
    /// Quarter turns (about the vertical axis) applied to the LDraw part so it
    /// matches our rotation-0 description.
    pub ldraw_yaw: u8,
    /// Offset in LDU (LDraw axes, -Y up) from the top-centre of the part's
    /// bounding body to its LDraw origin, in the rotation-0 frame.
    pub ldraw_offset: [i32; 3],
}

impl PartDef {
    /// Is there a stud on top of local column `(lx, lz)`?
    pub fn stud_on_top(&self, lx: u8, _lz: u8) -> bool {
        match self.shape {
            Shape::Box | Shape::InvertedSlope { .. } => true,
            Shape::Tile => false,
            Shape::Slope { high } => lx < high,
        }
    }

    /// Does local column `(lx, lz)` accept studs from below?
    pub fn connects_below(&self, lx: u8, _lz: u8) -> bool {
        match self.shape {
            Shape::InvertedSlope { high } => lx < high,
            _ => true,
        }
    }

    /// Fraction of the bounding volume that is solid (used for mass).
    pub fn fill_fraction(&self) -> f32 {
        let sx = self.size[0] as f32;
        match self.shape {
            Shape::Box | Shape::Tile => 1.0,
            Shape::Slope { high } | Shape::InvertedSlope { high } => (high as f32 + (sx - high as f32) * 0.5) / sx,
        }
    }

    pub fn is_slope(&self) -> bool {
        matches!(self.shape, Shape::Slope { .. } | Shape::InvertedSlope { .. })
    }

    /// Number of cells in the bounding box.
    pub fn cell_count(&self) -> u32 {
        self.size[0] as u32 * self.size[1] as u32 * self.height as u32
    }
}

const fn p(id: &'static str, name: &'static str, x: u8, z: u8, h: u8, shape: Shape) -> PartDef {
    PartDef { id, name, size: [x, z], height: h, shape, ldraw_yaw: 0, ldraw_offset: [0, 0, 0] }
}

#[allow(clippy::too_many_arguments)]
const fn pf(
    id: &'static str,
    name: &'static str,
    x: u8,
    z: u8,
    h: u8,
    shape: Shape,
    yaw: u8,
    off: [i32; 3],
) -> PartDef {
    PartDef { id, name, size: [x, z], height: h, shape, ldraw_yaw: yaw, ldraw_offset: off }
}

/// The parts the packer may use.
pub static CATALOG: &[PartDef] = &[
    // Bricks (3 plates tall). LDraw already has the long side along x, the
    // origin at the top centre, so no yaw or offset is needed.
    p("3005", "Brick 1 x 1", 1, 1, 3, Shape::Box),
    p("3004", "Brick 1 x 2", 2, 1, 3, Shape::Box),
    p("3622", "Brick 1 x 3", 3, 1, 3, Shape::Box),
    p("3010", "Brick 1 x 4", 4, 1, 3, Shape::Box),
    p("3009", "Brick 1 x 6", 6, 1, 3, Shape::Box),
    p("3008", "Brick 1 x 8", 8, 1, 3, Shape::Box),
    p("3003", "Brick 2 x 2", 2, 2, 3, Shape::Box),
    p("3002", "Brick 2 x 3", 3, 2, 3, Shape::Box),
    p("3001", "Brick 2 x 4", 4, 2, 3, Shape::Box),
    p("2456", "Brick 2 x 6", 6, 2, 3, Shape::Box),
    p("3007", "Brick 2 x 8", 8, 2, 3, Shape::Box),
    // Plates (1 plate tall).
    p("3024", "Plate 1 x 1", 1, 1, 1, Shape::Box),
    p("3023", "Plate 1 x 2", 2, 1, 1, Shape::Box),
    p("3623", "Plate 1 x 3", 3, 1, 1, Shape::Box),
    p("3710", "Plate 1 x 4", 4, 1, 1, Shape::Box),
    p("3666", "Plate 1 x 6", 6, 1, 1, Shape::Box),
    p("3460", "Plate 1 x 8", 8, 1, 1, Shape::Box),
    p("3022", "Plate 2 x 2", 2, 2, 1, Shape::Box),
    p("3021", "Plate 2 x 3", 3, 2, 1, Shape::Box),
    p("3020", "Plate 2 x 4", 4, 2, 1, Shape::Box),
    p("3795", "Plate 2 x 6", 6, 2, 1, Shape::Box),
    p("3034", "Plate 2 x 8", 8, 2, 1, Shape::Box),
    p("3031", "Plate 4 x 4", 4, 4, 1, Shape::Box),
    p("3032", "Plate 4 x 6", 6, 4, 1, Shape::Box),
    p("3035", "Plate 4 x 8", 8, 4, 1, Shape::Box),
    p("3958", "Plate 6 x 6", 6, 6, 1, Shape::Box),
    // Tiles (smooth tops).
    p("3070b", "Tile 1 x 1", 1, 1, 1, Shape::Tile),
    p("3069b", "Tile 1 x 2", 2, 1, 1, Shape::Tile),
    p("63864", "Tile 1 x 3", 3, 1, 1, Shape::Tile),
    p("2431", "Tile 1 x 4", 4, 1, 1, Shape::Tile),
    p("3068b", "Tile 2 x 2", 2, 2, 1, Shape::Tile),
    // Slopes. LDraw models these falling toward -z with the origin on the
    // high (studded) column, so yaw 3 maps -z onto our +x and the offset moves
    // the origin to that column's centre. Cheese slopes have their origin at
    // the bottom, hence the +16 LDU (two plates) y offset. Verified by
    // `catalog_frames_match_ldraw` in sf-ldraw.
    pf("3040b", "Slope 45 2 x 1", 2, 1, 3, Shape::Slope { high: 1 }, 3, [-10, 0, 0]),
    pf("3039", "Slope 45 2 x 2", 2, 2, 3, Shape::Slope { high: 1 }, 3, [-10, 0, 0]),
    pf("3665b", "Slope 45 2 x 1 Inverted", 2, 1, 3, Shape::InvertedSlope { high: 1 }, 3, [-10, 0, 0]),
    pf("3660b", "Slope 45 2 x 2 Inverted", 2, 2, 3, Shape::InvertedSlope { high: 1 }, 3, [-10, 0, 0]),
    pf("54200", "Slope 30 1 x 1 x 2/3", 1, 1, 2, Shape::Slope { high: 0 }, 3, [0, 16, 0]),
    pf("85984", "Slope 30 1 x 2 x 2/3", 1, 2, 2, Shape::Slope { high: 0 }, 3, [0, 16, 0]),
];

/// Look up a catalog part by LDraw id.
pub fn part(id: &str) -> Option<&'static PartDef> {
    CATALOG.iter().find(|p| p.id == id)
}
