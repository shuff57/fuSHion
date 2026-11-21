//! A finished brick model.

use crate::print::{DetailMesh, DetailSource};
use crate::{Cell, Placement};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelMode {
    #[default]
    Sculpture,
    Relief,
    Scene,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Model {
    pub title: String,
    pub mode: ModelMode,
    pub placements: Vec<Placement>,
    /// Designed to be 3D printed: only printable parts, fused-stud rule kept.
    #[serde(default)]
    pub printable: bool,
    /// Build steps as indices into `placements`, in order. Empty until
    /// computed by the engine.
    #[serde(default)]
    pub steps: Vec<Vec<usize>>,
    /// Display meshes of detail parts, by `CustomKind::Detail::mesh`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub meshes: BTreeMap<String, DetailMesh>,
    /// Print sources of detail parts, by the same key.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub details: BTreeMap<String, DetailSource>,
}

impl Model {
    /// Bounding box `(min, max_exclusive)` over all placement cells.
    pub fn bounds(&self) -> Option<(Cell, Cell)> {
        let mut it = self.placements.iter().flat_map(|p| p.cells());
        let first = it.next()?;
        let (mut lo, mut hi) = (first, Cell::new(first.x + 1, first.y + 1, first.z + 1));
        for c in it {
            lo = Cell::new(lo.x.min(c.x), lo.y.min(c.y), lo.z.min(c.z));
            hi = Cell::new(hi.x.max(c.x + 1), hi.y.max(c.y + 1), hi.z.max(c.z + 1));
        }
        Some((lo, hi))
    }
}
