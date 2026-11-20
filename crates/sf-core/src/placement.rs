//! A part placed on the grid, and its mapping to LDraw.

use crate::{Cell, ColorId, LDU_PER_PLATE, LDU_PER_STUD, PartDef, parts};
use serde::{Deserialize, Serialize};

/// Quarter turns about the vertical axis (0..4). Turn 1 maps `+x → -z` and
/// `+z → +x` (LDraw matrix `[[0,0,1],[0,1,0],[-1,0,0]]`).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rot(pub u8);

impl Rot {
    pub fn turns(self) -> u8 {
        self.0 % 4
    }

    /// Rotate a doubled-coordinate vector `(u, v)` in the xz plane.
    pub fn apply(self, u: i32, v: i32) -> (i32, i32) {
        match self.turns() {
            0 => (u, v),
            1 => (v, -u),
            2 => (-u, -v),
            _ => (-v, u),
        }
    }

    /// 3×3 rotation in LDraw axes (row-major).
    pub fn matrix(self) -> [[f32; 3]; 3] {
        match self.turns() {
            0 => [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
            1 => [[0., 0., 1.], [0., 1., 0.], [-1., 0., 0.]],
            2 => [[-1., 0., 0.], [0., 1., 0.], [0., 0., -1.]],
            _ => [[0., 0., -1.], [0., 1., 0.], [1., 0., 0.]],
        }
    }

    pub fn then(self, other: Rot) -> Rot {
        Rot((self.turns() + other.turns()) % 4)
    }
}

/// A part at a grid position. `origin` is the minimum corner of the part's
/// bounding box after rotation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    pub part: String,
    pub color: ColorId,
    pub origin: Cell,
    #[serde(default)]
    pub rot: Rot,
}

fn matmul(a: [[f32; 3]; 3], b: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let mut m = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            m[i][j] = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    m
}

impl Placement {
    pub fn new(part: &str, color: ColorId, origin: Cell, rot: Rot) -> Self {
        Self { part: part.to_string(), color, origin, rot }
    }

    /// The catalog definition. Panics never; unknown ids return `None`.
    pub fn def(&self) -> Option<&'static PartDef> {
        parts::part(&self.part)
    }

    /// Footprint `[x, z]` on the grid after rotation.
    pub fn footprint(&self) -> [i32; 2] {
        let d = self.def().map(|d| d.size).unwrap_or([1, 1]);
        if self.rot.turns() % 2 == 1 { [d[1] as i32, d[0] as i32] } else { [d[0] as i32, d[1] as i32] }
    }

    pub fn height(&self) -> i32 {
        self.def().map(|d| d.height as i32).unwrap_or(1)
    }

    /// Map a local column `(lx, lz)` of the unrotated part to grid `(x, z)`.
    pub fn local_to_grid(&self, lx: u8, lz: u8) -> (i32, i32) {
        let d = self.def().map(|d| d.size).unwrap_or([1, 1]);
        let (sx, sz) = (d[0] as i32, d[1] as i32);
        let [fx, fz] = self.footprint();
        let (u, v) = self.rot.apply(2 * lx as i32 + 1 - sx, 2 * lz as i32 + 1 - sz);
        (self.origin.x + (u + fx - 1) / 2, self.origin.z + (v + fz - 1) / 2)
    }

    /// Every grid cell inside the bounding box.
    pub fn cells(&self) -> impl Iterator<Item = Cell> + '_ {
        let [fx, fz] = self.footprint();
        let h = self.height();
        let o = self.origin;
        (0..h).flat_map(move |dy| (0..fz).flat_map(move |dz| (0..fx).map(move |dx| Cell::new(o.x + dx, o.y + dy, o.z + dz))))
    }

    /// Grid `(x, z)` columns with a stud on top (studs sit at `y = top()`).
    pub fn top_studs(&self) -> Vec<(i32, i32)> {
        self.columns(|d, lx, lz| d.stud_on_top(lx, lz))
    }

    /// Grid `(x, z)` columns that accept studs from below (at `y = origin.y`).
    pub fn bottom_sockets(&self) -> Vec<(i32, i32)> {
        self.columns(|d, lx, lz| d.connects_below(lx, lz))
    }

    fn columns(&self, f: impl Fn(&PartDef, u8, u8) -> bool) -> Vec<(i32, i32)> {
        let Some(d) = self.def() else {
            let [fx, fz] = self.footprint();
            return (0..fz).flat_map(|z| (0..fx).map(move |x| (self.origin.x + x, self.origin.z + z))).collect();
        };
        let mut out = Vec::new();
        for lz in 0..d.size[1] {
            for lx in 0..d.size[0] {
                if f(d, lx, lz) {
                    out.push(self.local_to_grid(lx, lz));
                }
            }
        }
        out
    }

    /// The plate level just above the part.
    pub fn top(&self) -> i32 {
        self.origin.y + self.height()
    }

    /// LDraw position (LDU) and row-major rotation matrix for this placement.
    pub fn ldraw_transform(&self) -> ([f32; 3], [[f32; 3]; 3]) {
        let [fx, fz] = self.footprint();
        let cx = (2 * self.origin.x + fx) as f32 * LDU_PER_STUD as f32 / 2.0;
        let cz = (2 * self.origin.z + fz) as f32 * LDU_PER_STUD as f32 / 2.0;
        let ty = -(self.top() as f32) * LDU_PER_PLATE as f32;
        let (yaw, off) = self.def().map(|d| (Rot(d.ldraw_yaw), d.ldraw_offset)).unwrap_or((Rot(0), [0, 0, 0]));
        let r = self.rot.matrix();
        let o = [off[0] as f32, off[1] as f32, off[2] as f32];
        let ro = [
            r[0][0] * o[0] + r[0][1] * o[1] + r[0][2] * o[2],
            r[1][0] * o[0] + r[1][1] * o[1] + r[1][2] * o[2],
            r[2][0] * o[0] + r[2][1] * o[1] + r[2][2] * o[2],
        ];
        ([cx + ro[0], ty + ro[1], cz + ro[2]], matmul(r, yaw.matrix()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn footprint_and_cells_rotate() {
        let p = Placement::new("3001", 4, Cell::new(2, 0, 3), Rot(1));
        assert_eq!(p.footprint(), [2, 4]);
        assert_eq!(p.cells().count(), 24);
        let studs = p.top_studs();
        assert_eq!(studs.len(), 8);
        assert!(studs.iter().all(|&(x, z)| (2..4).contains(&x) && (3..7).contains(&z)));
    }

    #[test]
    fn slope_studs_follow_rotation() {
        // Rot 0: slope falls toward +x, so the stud is on the low-x column.
        let p = Placement::new("3040b", 4, Cell::new(0, 0, 0), Rot(0));
        assert_eq!(p.top_studs(), vec![(0, 0)]);
        // Rot 2: falls toward -x, so the stud is on the high-x column.
        let p = Placement::new("3040b", 4, Cell::new(0, 0, 0), Rot(2));
        assert_eq!(p.top_studs(), vec![(1, 0)]);
        // Rot 1: +x → -z, so it falls toward -z and the stud is at z = 1.
        let p = Placement::new("3040b", 4, Cell::new(0, 0, 0), Rot(1));
        assert_eq!(p.top_studs(), vec![(0, 1)]);
    }

    #[test]
    fn ldraw_transform_of_plain_brick() {
        let p = Placement::new("3001", 4, Cell::new(0, 0, 0), Rot(0));
        let (pos, _) = p.ldraw_transform();
        assert_eq!(pos, [40.0, -24.0, 20.0]);
    }
}
