//! A finished brick model.

use crate::{Cell, Placement};
use serde::{Deserialize, Serialize};

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
    /// Build steps as indices into `placements`, in order. Empty until
    /// computed by the engine.
    #[serde(default)]
    pub steps: Vec<Vec<usize>>,
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
