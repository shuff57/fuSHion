//! Shared types for fushion.
//!
//! # Coordinate system
//! The build grid uses integer cells. `x` and `z` are horizontal and measured in
//! studs (8.0 mm). `y` points **up** and is measured in plates (3.2 mm); a
//! brick is three plates tall. Cell `(x, y, z)` is the unit volume
//! `[x, x+1) × [y, y+1) × [z, z+1)`. The ground plane is `y = 0`.
//!
//! LDraw uses LDU with `-Y` up: 1 stud = 20 LDU, 1 plate = 8 LDU. The mapping
//! is `X = 20·x`, `Y = -8·y`, `Z = 20·z`; see [`placement::Placement::ldraw_transform`].

pub mod color;
pub mod grid;
pub mod model;
pub mod parts;
pub mod placement;

pub use color::{Color, ColorId, Palette};
pub use grid::{Fill, VoxelGrid};
pub use model::{Model, ModelMode};
pub use parts::{PartDef, Shape, CATALOG};
pub use placement::{Placement, Rot};

/// LDraw units per stud (horizontal pitch).
pub const LDU_PER_STUD: i32 = 20;
/// LDraw units per plate (vertical).
pub const LDU_PER_PLATE: i32 = 8;
/// Plates per brick.
pub const PLATES_PER_BRICK: i32 = 3;
/// Millimetres per stud.
pub const MM_PER_STUD: f32 = 8.0;
/// Millimetres per plate.
pub const MM_PER_PLATE: f32 = 3.2;

/// A grid cell. `x`/`z` in studs, `y` (up) in plates.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct Cell {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl Cell {
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }
}

/// Answers whether a part was ever produced in a colour. Implemented by
/// `sf-catalog` from Rebrickable data; `None` means "unknown".
pub trait Availability: Send + Sync {
    /// Was `part` (LDraw id, e.g. `"3001"`) produced in `color` (LDraw code)?
    fn is_made(&self, part: &str, color: ColorId) -> Option<bool>;
    /// Number of official inventories containing this part in this colour.
    fn set_count(&self, part: &str, color: ColorId) -> Option<u32> {
        let _ = (part, color);
        None
    }
}

/// An [`Availability`] that knows nothing (every lookup is `None`).
pub struct UnknownAvailability;

impl Availability for UnknownAvailability {
    fn is_made(&self, _part: &str, _color: ColorId) -> Option<bool> {
        None
    }
}
