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
//! - optional bar sections ([`BAR_D`]) where leaves clip on;
//! - optional side branches in the same plane (so the whole stem still prints flat as one
//!   piece), each ending in a bar for another head, a bud, a flat blade, or nothing.
//!
//! # Each flower its own stem
//! Like LEGO's bouquet, each kind of flower has its own stem: a rose on a plain stem with
//! leaves, daisies as a forked sprig of several heads, lavender as a brown stem branching into
//! many thin sprigs, a snapdragon stalk with buds along the top, grass as a fan of blades. A
//! head design suggests its stem with [`StemStyle`] (`DetailDesign::stem`); the engine turns
//! that into a concrete [`StemDef`] sized for each use.
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
/// Length of a stem's tip bar, mm: as deep as a head's bar hole, so the head seats on the
/// rod's shoulder.
pub const TIP_BAR: f32 = 4.5;
/// A hub's thickness, mm: the peg hole (10.5 mm) plus the floor over the hub's sockets at
/// the steepest lean the engine allows.
pub const HUB_HEIGHT: f32 = 13.2;
/// Closest the axes of two hub holes may come anywhere along the hole depth, mm.
pub const HUB_HOLE_GAP: f32 = 5.8;
/// Closest a hub hole's axis may come to the edge of the hub's footprint, at the top and
/// at the bottom of the hole, mm.
pub const HUB_HOLE_EDGE: f32 = 3.6;
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
    /// Rod diameter, mm (default [`STEM_D`]; never below [`MIN_STEM_D`]).
    #[serde(default = "default_stem_d")]
    pub diameter: f32,
    /// What the main stem ends in when it carries no head (`tip_bar` 0): a bud, a blade.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tip: Option<StemTip>,
    /// Side branches, all in the stem's plane.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub branches: Vec<StemBranch>,
    /// The style's name, for parts lists and pages ("rose stem", "daisy sprig").
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub style: String,
}

fn default_stem_d() -> f32 {
    STEM_D
}

/// Thinnest rod a stem or branch may have, mm.
pub const MIN_STEM_D: f32 = 3.5;

/// A side branch of a stem, in the stem's plane. It leaves the main stem at `at` mm from
/// the base, turned `angle` degrees from the main stem's direction there (positive toward
/// local `+X`), and runs `length` mm as an arc bending `bend` degrees (positive toward `+X`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StemBranch {
    pub at: f32,
    pub angle: f32,
    pub length: f32,
    #[serde(default)]
    pub bend: f32,
    /// Rod diameter, mm (default: the main stem's, less 1 mm, at least [`MIN_STEM_D`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diameter: Option<f32>,
    /// Length of a bar at its end for a head, mm (0: no head; see `tip`).
    #[serde(default)]
    pub tip_bar: f32,
    /// What it ends in when it carries no head.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tip: Option<StemTip>,
}

/// A stem or branch end that is part of the printed stem itself (it prints flat with it).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StemTip {
    /// A closed bud: a pointed oval `length` × `width` mm, flat underneath.
    Bud { length: f32, width: f32 },
    /// A flat blade (grass, a pine needle bunch, a sepal): tapering to a point, `length` ×
    /// `width` mm, as thick as the stem's flat allows (about 1.6 mm).
    Blade { length: f32, width: f32 },
}

/// A head design's suggested stem: the engine scales it to each use. Lengths and positions
/// are fractions of the main stem's length so one style fits any stem.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StemStyle {
    /// "rose stem", "daisy sprig", "lavender sprig", "snapdragon stalk", "grass".
    pub name: String,
    /// Rod diameter, mm.
    #[serde(default = "default_stem_d")]
    pub diameter: f32,
    /// Default stem colour name ("Dark Green"; "Reddish Brown" for lavender).
    pub color: String,
    /// Whether the main stem's tip carries the head (false: it ends in `tip` or bare, and
    /// only branches carry heads).
    #[serde(default = "yes")]
    pub head_on_tip: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tip: Option<StemTip>,
    pub branches: Vec<StyleBranch>,
    /// Suggested leaf positions along the main stem (fractions of its length from the base).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub leaves: Vec<f32>,
}

fn yes() -> bool {
    true
}

/// A branch of a [`StemStyle`]: `at` and `length` are fractions of the main stem's length.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct StyleBranch {
    pub at: f32,
    pub angle: f32,
    pub length: f32,
    #[serde(default)]
    pub bend: f32,
    /// Carries a head (the same design as the stem's head unless the spec says otherwise).
    #[serde(default)]
    pub head: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tip: Option<StemTip>,
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

    /// Branch `i`'s centreline point at distance `u` mm from where it leaves the main stem
    /// (0 ≤ u ≤ its length), in the stem's own frame (LDU).
    pub fn branch_point(&self, i: usize, u: f32) -> [f32; 3] {
        let b = &self.branches[i];
        let p0 = self.point(b.at);
        let (x0, up0) = (p0[0] / LDU_PER_MM, -p0[1] / LDU_PER_MM);
        let a0 = self.bend.to_radians() * (b.at / self.length.max(1e-3)) + b.angle.to_radians();
        let k = b.bend.to_radians() / b.length.max(1e-3);
        let (x, up) = if k.abs() < 1e-6 {
            (x0 + u * a0.sin(), up0 + u * a0.cos())
        } else {
            let a = a0 + k * u;
            (x0 + (a0.cos() - a.cos()) / k, up0 + (a.sin() - a0.sin()) / k)
        };
        [x * LDU_PER_MM, -up * LDU_PER_MM, 0.0]
    }

    /// Branch `i`'s unit tangent at distance `u` from where it leaves the main stem.
    pub fn branch_tangent(&self, i: usize, u: f32) -> [f32; 3] {
        let b = &self.branches[i];
        let a = self.bend.to_radians() * (b.at / self.length.max(1e-3))
            + b.angle.to_radians()
            + b.bend.to_radians() * (u / b.length.max(1e-3));
        [a.sin(), -a.cos(), 0.0]
    }

    /// Branch `i`'s rod diameter, mm.
    pub fn branch_diameter(&self, i: usize) -> f32 {
        self.branches[i].diameter.unwrap_or(self.diameter - 1.0).max(MIN_STEM_D)
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
    /// For a head on a branch's tip bar: the index into [`StemDef::branches`] (`None`: the
    /// main stem's tip).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<usize>,
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
    /// Heads on branch tips (each with `branch` set).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub branch_heads: Vec<StemAttach>,
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
