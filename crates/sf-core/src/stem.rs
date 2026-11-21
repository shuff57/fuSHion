//! Stems: long thin parts that each print as **one piece**, lying flat on the bed, running
//! from a hub in the vase up to a flower head, with leaves clipped on along the way. This is
//! how LEGO's Botanical bouquets hold their flowers: thin stems, gathered in a vase, with heads
//! and leaves clipped on at angles.
//!
//! # A stem
//! A stem's centreline is a circular arc in one plane (straight when `bend` is 0), so it
//! prints flat. Its cross-section is a round [`STEM_D`] rod with a flat underside
//! ([`STEM_FLAT`] below the axis, on the local `+Z` side) to lie on the bed. Along it:
//! - at the base, a keyed peg ([`PEG_D`], flat on one side) that plugs into a hole in the
//!   hub; its flat is on the same side as the stem's (it prints flat too) and sets which way
//!   the stem's bend and leaves face;
//! - at the tip, a LEGO bar ([`BAR_D`], `tip_bar` long) that goes into the head's bar hole;
//! - optional bar sections ([`BAR_D`]) where leaves clip on.
//!
//! # Frames
//! A stem's own frame is in LDU with LDraw axes (`-Y` up), like a part's: origin at the base
//! shoulder (where the peg meets the stem, flush with the hub's top), the stem rising along
//! `-Y`, bending toward `+X` (in the local XY plane). The peg goes down along `+Y` from the
//! origin. [`StemDef::point`] gives the centreline. [`StemPlacement::pose`] places it in the
//! model (LDU, the same frame as `Placement::ldraw_transform`). The engine lays the stem flat
//! for printing: the local XY plane becomes the bed, `+Z` down onto it.

use crate::print::KitPose;
use crate::ColorId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Stem rod diameter, mm (stiff enough to print a 250 mm stem flat in PLA or PETG).
pub const STEM_D: f32 = 5.0;
/// Depth of the flat underside below the stem's axis, mm (the flat is about 4 mm wide).
pub const STEM_FLAT: f32 = 1.5;
/// LEGO bar diameter, mm.
pub const BAR_D: f32 = 3.18;
/// The base peg's diameter, mm (stud size; it has a flat on one side as a key).
pub const PEG_D: f32 = 4.8;
/// The base peg's length into the hub, mm.
pub const PEG_LEN: f32 = 10.0;
/// Longest stem that prints on a common bed (250–256 mm), centreline, mm.
pub const MAX_STEM_LEN: f32 = 240.0;
/// Millimetres to LDU.
pub const LDU_PER_MM: f32 = 2.5;

/// One printed stem.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StemDef {
    /// Its print id, e.g. `"S01"` (engraved under the peg's shoulder, like brick numbers).
    pub id: String,
    /// Centreline length from the base shoulder to the tip of the tip bar, mm.
    pub length: f32,
    /// Total bend of the centreline, degrees, toward local `+X` (0 = straight).
    #[serde(default)]
    pub bend: f32,
    /// Length of the LEGO bar at the tip, mm (0: no head, the stem just ends).
    #[serde(default)]
    pub tip_bar: f32,
    /// Bar sections for leaves to clip on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bars: Vec<StemBar>,
}

/// A [`BAR_D`] bar section along a stem.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct StemBar {
    /// Distance of its middle from the base shoulder along the centreline, mm.
    pub at: f32,
    /// Its length, mm.
    pub length: f32,
}

impl StemDef {
    /// The centreline point at distance `s` mm from the base shoulder (0 ≤ s ≤ length), in
    /// the stem's own frame (LDU, `-Y` up, bending toward `+X`).
    pub fn point(&self, s: f32) -> [f32; 3] {
        let k = self.bend.to_radians() / self.length.max(1e-3);
        let (x, up) = if k.abs() < 1e-6 {
            (0.0, s)
        } else {
            let r = 1.0 / k;
            (r * (1.0 - (k * s).cos()), r * (k * s).sin())
        };
        [x * LDU_PER_MM, -up * LDU_PER_MM, 0.0]
    }

    /// The unit tangent at distance `s`, pointing toward the tip, in the stem's own frame.
    pub fn tangent(&self, s: f32) -> [f32; 3] {
        let a = self.bend.to_radians() * (s / self.length.max(1e-3));
        [a.sin(), -a.cos(), 0.0]
    }
}

/// Where a design attaches to a stem, in the design's mesh frame (LDU, `-Y` up).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct StemMount {
    pub kind: MountKind,
    /// For a [`MountKind::Hole`]: the hole's mouth. For a [`MountKind::Clip`]: the centre of
    /// the bar inside the clip.
    pub origin: [f32; 3],
    /// Unit vector. A hole: pointing out of the hole, toward the stem (a flower facing up
    /// has `[0, 1, 0]`, down). A clip: along the bar through the clip.
    pub axis: [f32; 3],
    /// Unit vector square to `axis`: the design's "front" (a flower's face direction for a
    /// clip; for a hole, which way the head's front turns). The engine turns the design
    /// about the stem so `up` points where the spec asks.
    pub up: [f32; 3],
}

/// How a design attaches to a stem.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MountKind {
    /// A [`BAR_D`] hole the stem's tip bar pushes into (flower heads).
    Hole,
    /// A clip that snaps onto one of the stem's bar sections (leaves).
    Clip,
}

/// A design attached to a stem: a head on its tip, or a leaf on a bar section.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StemAttach {
    /// The design's name (a key of `Model::meshes`, and of `Model::kits` for a kit).
    pub design: String,
    pub color: ColorId,
    /// Colours of the design's other roles.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub colors: BTreeMap<String, ColorId>,
    /// The design's mesh frame placed in the model (LDU).
    pub pose: KitPose,
    /// For a leaf: the index into [`StemDef::bars`] it clips onto.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bar: Option<usize>,
}

/// A stem in the model, with what is attached to it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StemPlacement {
    pub def: StemDef,
    pub color: ColorId,
    /// The stem's own frame placed in the model (LDU).
    pub pose: KitPose,
    /// The hub (a detail placement's id, e.g. `"D001"`) and the 1-based hole number it
    /// plugs into.
    pub hub: String,
    pub hole: u32,
    pub head: Option<StemAttach>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub leaves: Vec<StemAttach>,
}

/// A hub's design: a printed puck that sits on studs (the top of a brick-built vase) and
/// holds the stems' pegs in angled, keyed holes. It goes into `Model::details` as the
/// source `{"hub": Hub}` of a detail placement, so it prints and renders like any detail.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hub {
    /// Footprint in studs (the puck covers these columns and has a socket under each).
    pub cells: Vec<(u8, u8)>,
    /// Thickness, mm (deep enough for [`PEG_LEN`] at the steepest lean).
    pub height: f32,
    pub holes: Vec<HubHole>,
}

/// One keyed peg hole in a hub, in the hub's print frame: mm, z up, origin at the corner
/// of its footprint's bounding box on the bottom (as for detail sources).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct HubHole {
    /// Where the stem's axis leaves the hub's top face.
    pub top: [f32; 3],
    /// Unit vector along the stem, pointing up and out of the hole.
    pub dir: [f32; 3],
    /// Unit vector square to `dir`: the side the peg's flat faces (the stem's local `+Z`).
    pub key: [f32; 3],
}
