//! Coloured voxel grids: the output of the spec compiler and input to the packer.

use crate::{Cell, ColorId};
use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Fill {
    #[default]
    Empty,
    /// A visible cell that must be this colour.
    Solid(ColorId),
    /// A hidden interior cell; any colour will do.
    Interior,
}

impl Fill {
    pub fn is_filled(self) -> bool {
        !matches!(self, Fill::Empty)
    }
    pub fn color(self) -> Option<ColorId> {
        match self {
            Fill::Solid(c) => Some(c),
            _ => None,
        }
    }
}

/// A dense grid of cells: `dims = [x, y, z]` with `y` in plates.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VoxelGrid {
    pub dims: [i32; 3],
    cells: Vec<Fill>,
}

impl VoxelGrid {
    pub fn new(dims: [i32; 3]) -> Self {
        let n = (dims[0].max(0) * dims[1].max(0) * dims[2].max(0)) as usize;
        Self { dims, cells: vec![Fill::Empty; n] }
    }

    pub fn in_bounds(&self, c: Cell) -> bool {
        c.x >= 0 && c.y >= 0 && c.z >= 0 && c.x < self.dims[0] && c.y < self.dims[1] && c.z < self.dims[2]
    }

    fn idx(&self, c: Cell) -> usize {
        ((c.y * self.dims[2] + c.z) * self.dims[0] + c.x) as usize
    }

    pub fn get(&self, c: Cell) -> Fill {
        if self.in_bounds(c) { self.cells[self.idx(c)] } else { Fill::Empty }
    }

    pub fn set(&mut self, c: Cell, f: Fill) {
        if self.in_bounds(c) {
            let i = self.idx(c);
            self.cells[i] = f;
        }
    }

    /// Iterate over filled cells.
    pub fn filled(&self) -> impl Iterator<Item = (Cell, Fill)> + '_ {
        let [dx, dy, dz] = self.dims;
        (0..dy).flat_map(move |y| {
            (0..dz).flat_map(move |z| (0..dx).map(move |x| Cell::new(x, y, z)))
        })
        .map(|c| (c, self.get(c)))
        .filter(|(_, f)| f.is_filled())
    }

    pub fn count_filled(&self) -> usize {
        self.cells.iter().filter(|f| f.is_filled()).count()
    }

    /// Mark filled cells with no empty 6-neighbour (or out-of-bounds exposure
    /// except the ground) as [`Fill::Interior`]. Cells on the bottom layer stay
    /// visible.
    pub fn mark_interior(&mut self) {
        let mut out = self.cells.clone();
        for (c, f) in self.filled() {
            if c.y == 0 {
                continue;
            }
            let hidden = [(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)]
                .iter()
                .all(|&(dx, dy, dz)| self.get(Cell::new(c.x + dx, c.y + dy, c.z + dz)).is_filled());
            if hidden && matches!(f, Fill::Solid(_)) {
                out[self.idx(c)] = Fill::Interior;
            }
        }
        self.cells = out;
    }
}
