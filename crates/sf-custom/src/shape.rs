//! What custom parts can be made, and the plane geometry behind them.

use crate::{CustomError, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// A custom part, sized in studs (`w` along X, `d` along Y) and plates (3 plates = 1 brick).
///
/// Coordinates: the stud grid starts at the origin, so stud `(i, j)` is centred at
/// `(pitch·(i + ½), pitch·(j + ½))`; the body stands on `z = 0` and the studs point up (+Z).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CustomShape {
    /// A plain brick or plate: hollow underneath with tubes, studs on every cell.
    Box {
        /// Width in studs (X).
        w: u32,
        /// Depth in studs (Y).
        d: u32,
        /// Height in plates.
        plates: u32,
    },
    /// A 45° slope falling toward −Y: studs on the back row (or rows, where the top is flat),
    /// the roof dropping at 45° to a short vertical lip at the front.
    Slope45 {
        /// Width in studs (X).
        w: u32,
        /// Depth in studs (Y), at least 2.
        d: u32,
        /// Height in plates.
        plates: u32,
    },
    /// A curved slope falling toward −Y: like [`CustomShape::Slope45`], but the roof is a
    /// convex quarter circle, tangent to the flat top at the back and vertical at the front lip.
    Curve {
        /// Width in studs (X).
        w: u32,
        /// Depth in studs (Y), at least 2.
        d: u32,
        /// Height in plates.
        plates: u32,
    },
    /// Any edge-connected set of stud cells, extruded: studs on every cell and bottom tubes
    /// wherever a full 2×2 block of cells meets.
    Footprint {
        /// The cells, as `(i, j)` stud positions (X, Y).
        cells: Vec<(i32, i32)>,
        /// Height in plates.
        plates: u32,
    },
}

impl CustomShape {
    /// Height in plates.
    pub fn plates(&self) -> u32 {
        match self {
            Self::Box { plates, .. }
            | Self::Slope45 { plates, .. }
            | Self::Curve { plates, .. }
            | Self::Footprint { plates, .. } => *plates,
        }
    }

    /// A short human name, e.g. `"slope45 2x3x3"`.
    pub fn label(&self) -> String {
        match self {
            Self::Box { w, d, plates } => format!("box {w}x{d}x{plates}"),
            Self::Slope45 { w, d, plates } => format!("slope45 {w}x{d}x{plates}"),
            Self::Curve { w, d, plates } => format!("curve {w}x{d}x{plates}"),
            Self::Footprint { cells, plates } => {
                format!("footprint {} cells x{plates}", cells.len())
            }
        }
    }
}

/// A point in the plane.
pub(crate) type P2 = [f64; 2];

/// Twice the signed area of a closed loop (positive counter-clockwise).
pub(crate) fn signed_area(loop_: &[P2]) -> f64 {
    let n = loop_.len();
    (0..n)
        .map(|k| {
            let a = loop_[k];
            let b = loop_[(k + 1) % n];
            a[0] * b[1] - b[0] * a[1]
        })
        .sum::<f64>()
        / 2.0
}

/// A footprint made ready to model: its cells, boundary loops (in cell units, material on the
/// left, so outer loops run counter-clockwise and holes clockwise) and full 2×2 corners.
#[derive(Clone, Debug)]
pub(crate) struct Footprint {
    pub cells: BTreeSet<(i32, i32)>,
    /// Boundary loops on the cell grid, collinear points merged.
    pub loops: Vec<Vec<(i32, i32)>>,
}

/// Largest footprint coordinate accepted (a part is at most 64 mm anyway).
const MAX_CELL_COORD: i32 = 64;

impl Footprint {
    /// Checks the cells and traces their outline.
    pub fn new(cells: &[(i32, i32)]) -> Result<Self> {
        let bad = |why: String| Err(CustomError::InvalidShape(why));
        if cells.is_empty() {
            return bad("a footprint needs at least one cell".into());
        }
        if let Some(c) = cells
            .iter()
            .find(|(i, j)| i.abs() > MAX_CELL_COORD || j.abs() > MAX_CELL_COORD)
        {
            return bad(format!(
                "footprint cell {c:?} is out of range (|i|, |j| ≤ {MAX_CELL_COORD})"
            ));
        }
        let set: BTreeSet<(i32, i32)> = cells.iter().copied().collect();
        // Edge-connected, or the part would fall apart into several bodies.
        let mut seen = BTreeSet::new();
        let mut queue = VecDeque::new();
        if let Some(first) = set.iter().next() {
            seen.insert(*first);
            queue.push_back(*first);
        }
        while let Some((i, j)) = queue.pop_front() {
            for n in [(i + 1, j), (i - 1, j), (i, j + 1), (i, j - 1)] {
                if set.contains(&n) && seen.insert(n) {
                    queue.push_back(n);
                }
            }
        }
        if seen.len() != set.len() {
            return bad("footprint cells must be edge-connected (cells touching only at a corner, or apart, would print as separate parts)".into());
        }
        let loops = trace_loops(&set)?;
        Ok(Self { cells: set, loops })
    }

    /// The grid points `(i, j)` where all four cells around them are present: tube positions.
    pub fn full_corners(&self) -> Vec<(i32, i32)> {
        let mut out = Vec::new();
        for &(i, j) in &self.cells {
            // Corner at the top-right of cell (i, j).
            let (ci, cj) = (i + 1, j + 1);
            if self.cells.contains(&(i + 1, j))
                && self.cells.contains(&(i, j + 1))
                && self.cells.contains(&(i + 1, j + 1))
            {
                out.push((ci, cj));
            }
        }
        out
    }

    /// Each boundary loop scaled by `pitch` and moved `inset` into the material.
    pub fn offset_loops(&self, pitch: f64, inset: f64) -> Vec<Vec<P2>> {
        self.loops
            .iter()
            .map(|l| offset_loop(l, pitch, inset))
            .collect()
    }
}

/// Traces the boundary of a set of unit cells into closed loops with the material on the left.
/// Where two loops pinch at one grid point (cells touching only at a corner), the trace turns
/// left, so each loop hugs its own cells and the loops separate when offset into the material.
fn trace_loops(cells: &BTreeSet<(i32, i32)>) -> Result<Vec<Vec<(i32, i32)>>> {
    // Directed boundary edges, keyed by start point.
    let mut out: BTreeMap<(i32, i32), Vec<(i32, i32)>> = BTreeMap::new();
    let mut count = 0usize;
    for &(i, j) in cells {
        let edges = [
            ((i, j - 1), (i, j), (i + 1, j)),
            ((i + 1, j), (i + 1, j), (i + 1, j + 1)),
            ((i, j + 1), (i + 1, j + 1), (i, j + 1)),
            ((i - 1, j), (i, j + 1), (i, j)),
        ];
        for (neighbour, a, b) in edges {
            if !cells.contains(&neighbour) {
                out.entry(a).or_default().push(b);
                count += 1;
            }
        }
    }
    let mut loops = Vec::new();
    let mut used = 0usize;
    while let Some((&start, _)) = out.iter().find(|(_, v)| !v.is_empty()) {
        let mut pts = vec![start];
        let mut cur = start;
        let mut dir: Option<(i32, i32)> = None;
        loop {
            let Some(nexts) = out.get_mut(&cur) else {
                return Err(CustomError::InvalidShape(
                    "footprint outline does not close".into(),
                ));
            };
            if nexts.is_empty() {
                return Err(CustomError::InvalidShape(
                    "footprint outline does not close".into(),
                ));
            }
            // Prefer the left turn (rotate the incoming direction +90°), then straight, then right.
            let pick = match dir {
                Some((dx, dy)) => {
                    let order = [(-dy, dx), (dx, dy), (dy, -dx)];
                    order
                        .iter()
                        .find_map(|want| {
                            nexts
                                .iter()
                                .position(|n| (n.0 - cur.0, n.1 - cur.1) == *want)
                        })
                        .unwrap_or(0)
                }
                None => 0,
            };
            let next = nexts.swap_remove(pick);
            used += 1;
            dir = Some((next.0 - cur.0, next.1 - cur.1));
            cur = next;
            if cur == start {
                break;
            }
            pts.push(cur);
            if used > count {
                return Err(CustomError::InvalidShape(
                    "footprint outline does not close".into(),
                ));
            }
        }
        loops.push(merge_collinear(pts));
    }
    Ok(loops)
}

/// Drops points where a loop goes straight on.
fn merge_collinear(pts: Vec<(i32, i32)>) -> Vec<(i32, i32)> {
    let n = pts.len();
    (0..n)
        .filter(|&k| {
            let a = pts[(k + n - 1) % n];
            let b = pts[k];
            let c = pts[(k + 1) % n];
            (b.0 - a.0) * (c.1 - b.1) - (b.1 - a.1) * (c.0 - b.0) != 0
        })
        .map(|k| pts[k])
        .collect()
}

/// A rectilinear loop (grid units, material on the left, no collinear points) scaled by
/// `pitch` and moved `inset` toward the material: each corner moves by the sum of its two
/// edges' left normals.
fn offset_loop(l: &[(i32, i32)], pitch: f64, inset: f64) -> Vec<P2> {
    let n = l.len();
    (0..n)
        .map(|k| {
            let a = l[(k + n - 1) % n];
            let b = l[k];
            let c = l[(k + 1) % n];
            let unit = |p: (i32, i32), q: (i32, i32)| {
                let (dx, dy) = (f64::from(q.0 - p.0), f64::from(q.1 - p.1));
                let len = dx.hypot(dy).max(f64::MIN_POSITIVE);
                (dx / len, dy / len)
            };
            let (e1x, e1y) = unit(a, b);
            let (e2x, e2y) = unit(b, c);
            let nx = -e1y - e2y;
            let ny = e1x + e2x;
            [
                f64::from(b.0) * pitch + inset * nx,
                f64::from(b.1) * pitch + inset * ny,
            ]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rectangle_traces_to_one_ccw_loop() {
        let cells: Vec<_> = (0..3).flat_map(|i| (0..2).map(move |j| (i, j))).collect();
        let f = Footprint::new(&cells).unwrap();
        assert_eq!(f.loops.len(), 1);
        assert_eq!(f.loops[0].len(), 4);
        let l = &f.offset_loops(8.0, 0.1)[0];
        assert!((signed_area(l) - 23.8 * 15.8).abs() < 1e-9);
        assert_eq!(f.full_corners(), vec![(1, 1), (2, 1)]);
    }

    #[test]
    fn ring_has_a_hole_and_l_shape_has_concave_corner() {
        let ring: Vec<_> = (0..3)
            .flat_map(|i| (0..3).map(move |j| (i, j)))
            .filter(|c| *c != (1, 1))
            .collect();
        let f = Footprint::new(&ring).unwrap();
        assert_eq!(f.loops.len(), 2);
        let areas: Vec<f64> = f
            .offset_loops(8.0, 0.1)
            .iter()
            .map(|l| signed_area(l))
            .collect();
        let total: f64 = areas.iter().sum();
        assert!(
            (total - (23.8 * 23.8 - 8.2 * 8.2)).abs() < 1e-9,
            "{areas:?}"
        );
        let l = Footprint::new(&[(0, 0), (1, 0), (0, 1)]).unwrap();
        assert_eq!(l.loops[0].len(), 6);
        let a = signed_area(&l.offset_loops(8.0, 0.1)[0]);
        // 3 cells minus the strips the inset takes off the outside.
        assert!((a - (15.8 * 7.8 + 7.8 * 8.0)).abs() < 1e-9, "{a}");
    }

    #[test]
    fn diagonal_or_empty_footprints_are_refused() {
        assert!(Footprint::new(&[(0, 0), (1, 1)]).is_err());
        assert!(Footprint::new(&[]).is_err());
        assert!(Footprint::new(&[(0, 0), (1000, 0)]).is_err());
    }
}
