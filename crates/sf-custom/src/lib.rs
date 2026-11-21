//! Custom 3D-printable bricks for fushion.
//!
//! When no real part fits a spot in a model (a colour never made, a cluster of tiny parts, an
//! odd shape, a retired part), fushion designs one to print. This crate models it with the
//! [SolidCraft](https://github.com/tangency-labs/solidcraft) B-rep engine, in process, by
//! sending it the same JSON command batches its MCP server accepts.
//!
//! * [`StudProfile`]: the connector dimensions. fuSHion builds [`StudProfile::fused`]: Slant
//!   3D's "v2" stud reshaped to fit standard bricks, every stud fused into a pair or a triple
//!   with leaf-spring fins, and 0.4 mm rounded vertical edges. [`StudProfile::standard_fdm`]
//!   (Slant 3D's plain FDM brick) stays as the model of a standard brick for fit checks.
//! * [`CustomShape`]: a brick or plate, a 45° slope, a curved slope or any footprint of cells.
//!   [`CustomShape::Printed`] is a body part designed for one spot in a model: any footprint,
//!   fused studs only in the units the packer chose, its id engraved underneath.
//! * [`detail_template`] and [`detail_from_commands`]: shaped detail parts (flower heads,
//!   petals, leaves, domes, cones, round plates, or Claude's own SolidCraft batch) on a socket
//!   base, returned as an `sf_core::DetailDesign` with a display mesh; [`rebuild_detail`]
//!   makes the print again from the design's recorded source.
//! * [`commands`]: the SolidCraft batch for a shape, with an `expect` check computed by hand.
//! * [`build`]: runs that batch, checks every stage's volume against its closed form, and
//!   exports STL and 3MF.
//! * [`build_from_commands`]: the same safety for a freeform batch (one Claude wrote).
//! * [`fit`]: where a brick pushed onto a box part meets its studs, in plane geometry (no
//!   SolidCraft): clearances, and how far fins press; [`fit_under`] the same with the brick
//!   on top made with a profile of its own (wall ribs and all); [`fit_below`] the part pushed
//!   onto a brick below, where its walls and wall ribs meet that brick's studs and fins.
//!
//! All lengths are millimetres; the part stands on `z = 0` with its studs pointing up (+Z).

#![warn(missing_docs)]

mod detail;
mod fit;
mod flower;
mod geom;
mod kit;
mod kits;
mod label;
mod mesh;
mod plan;
mod profile;
mod run;
mod shape;
mod slant;
mod socket;

pub use detail::{
    BASE_HEIGHT, DISPLAY_TRIANGLES, Detail, OVERHANG_MARGIN, SINK, SUPPORT_AREA_MM2, TEMPLATES,
    decimate, detail_from_commands, detail_template, overhang_area, rebuild, rebuild_detail,
};
pub use flower::SPECIES;
pub use kit::{
    BAR_D, BuiltKit, ClipFit, ClipSpec, HOLE_D, MAX_STRAIN, MIN_FEATURE, PEG_D, PEG_LEN, PegFit,
    append_posed, assemble_kit, build_kit_part, clip_fit, coupon, peg_fit,
};
pub use kits::KITS;
pub use socket::{Contact, E_PLA_MPA, SocketFit, SocketSpec, TDims, socket_fit, t_dims};
pub use fit::{FitReport, SlantOutline, TopBrick, fit, fit_below, fit_under, slant_outline};
pub use label::{LABEL_CHARS, LABEL_DEPTH, engravable};
pub use mesh::{MeshReport, analyze_stl};
pub use plan::{BBOX_TOL_MM, Plan, Stage, VOLUME_TOL, fused_units, plan};
pub use profile::{
    FUSED_RIB_OFFSET, MIN_FIN_SECTION, MIN_POCKET_FLOOR, RIB_GAP, RIB_THICKNESS, SLANT_CAP,
    StudProfile, StudStyle, WALL_RIB_LEAD_IN, WALL_RIB_WIDTH,
};
pub use run::{MAX_COMMANDS, build, build_from_commands, build_plan};
pub use shape::CustomShape;
pub use slant::SlantDims;

/// The derived dimensions of `profile`'s Slant v2 stud (an error if it has none, or its
/// numbers don't make one).
pub fn slant_dims(profile: &StudProfile) -> Result<SlantDims> {
    Ok(slant::Lobe::new(profile)?.dims)
}

/// Longest side of any custom part, in mm.
pub const MAX_PART_MM: f64 = 64.0;

/// Why a custom part could not be made.
#[derive(Debug, thiserror::Error)]
pub enum CustomError {
    /// The shape's sizes don't make a part.
    #[error("invalid shape: {0}")]
    InvalidShape(String),
    /// The stud profile's numbers don't make a printable brick.
    #[error("invalid stud profile: {0}")]
    InvalidProfile(String),
    /// The part would be bigger than [`MAX_PART_MM`] on a side.
    #[error("part too large: {0}")]
    TooLarge(String),
    /// A freeform batch was refused before anything ran.
    #[error("batch refused: {0}")]
    Refused(String),
    /// SolidCraft refused or failed a command.
    #[error("step {index} ({command}) of stage {stage} failed: {message}")]
    Engine {
        /// The stage the command belongs to.
        stage: String,
        /// Index of the command in the whole batch.
        index: usize,
        /// The command id.
        command: String,
        /// SolidCraft's message.
        message: String,
    },
    /// A stage built, but the result is not what it should be (volume, body count, box).
    #[error("stage {stage} built the wrong part: {details}")]
    Mismatch {
        /// The stage whose result is wrong.
        stage: String,
        /// What differs, measured against expected.
        details: String,
    },
    /// Exporting or checking the mesh failed.
    #[error("export failed: {0}")]
    Export(String),
}

/// Result with a [`CustomError`].
pub type Result<T> = std::result::Result<T, CustomError>;

/// A finished, checked custom part.
#[derive(Clone, Debug)]
pub struct CustomPart {
    /// Binary STL, in mm.
    pub stl: Vec<u8>,
    /// 3MF package, in mm.
    pub three_mf: Vec<u8>,
    /// Exact (B-rep) volume, mm³.
    pub volume_mm3: f64,
    /// Exact bounding box `(min, max)`, mm.
    pub bbox_mm: ([f64; 3], [f64; 3]),
    /// Each stage's name and the part's measured volume after it, mm³ (empty for freeform
    /// batches).
    pub stage_volumes_mm3: Vec<(String, f64)>,
}

/// The SolidCraft command batch for `shape`: `{"commands": [...], "expect": {...}}`, the
/// arguments of SolidCraft's MCP `batch` tool, whose `expect` holds the finished volume, body
/// count and bounding box worked out by hand. See [`plan`] for the per-stage volumes.
pub fn commands(shape: &CustomShape, profile: &StudProfile) -> Result<serde_json::Value> {
    Ok(plan(shape, profile)?.batch())
}
