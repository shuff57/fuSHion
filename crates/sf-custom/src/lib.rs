//! Custom 3D-printable bricks for studforge.
//!
//! When no real part fits a spot in a model (a colour never made, a cluster of tiny parts, an
//! odd shape, a retired part), studforge designs one to print. This crate models it with the
//! [SolidCraft](https://github.com/tangency-labs/solidcraft) B-rep engine, in process, by
//! sending it the same JSON command batches its MCP server accepts.
//!
//! * [`StudProfile`]: the connector dimensions (Slant 3D's FDM numbers by default, or an
//!   experimental compliant stud).
//! * [`CustomShape`]: a brick or plate, a 45° slope, a curved slope or any footprint of cells.
//! * [`commands`]: the SolidCraft batch for a shape, with an `expect` check computed by hand.
//! * [`build`]: runs that batch, checks every stage's volume against its closed form, and
//!   exports STL and 3MF.
//! * [`build_from_commands`]: the same safety for a freeform batch (one Claude wrote).
//!
//! All lengths are millimetres; the part stands on `z = 0` with its studs pointing up (+Z).

#![warn(missing_docs)]

mod mesh;
mod plan;
mod profile;
mod run;
mod shape;

pub use mesh::{MeshReport, analyze_stl};
pub use plan::{BBOX_TOL_MM, Plan, Stage, VOLUME_TOL, plan};
pub use profile::{StudProfile, StudStyle};
pub use run::{MAX_COMMANDS, build, build_from_commands, build_plan};
pub use shape::CustomShape;

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
}

/// The SolidCraft command batch for `shape`: `{"commands": [...], "expect": {...}}`, the
/// arguments of SolidCraft's MCP `batch` tool, whose `expect` holds the finished volume, body
/// count and bounding box worked out by hand. See [`plan`] for the per-stage volumes.
pub fn commands(shape: &CustomShape, profile: &StudProfile) -> Result<serde_json::Value> {
    Ok(plan(shape, profile)?.batch())
}
