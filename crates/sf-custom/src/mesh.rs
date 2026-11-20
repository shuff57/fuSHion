//! Checks on an exported binary STL: watertightness, volume and extent.

use crate::{CustomError, Result};
use std::collections::HashMap;

/// What [`analyze_stl`] found in a binary STL.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshReport {
    /// Number of triangles.
    pub triangles: usize,
    /// Enclosed volume (signed tetrahedron sum), mm³.
    pub volume_mm3: f64,
    /// Bounding box `(min, max)` of the vertices, mm.
    pub bbox_mm: ([f64; 3], [f64; 3]),
    /// Edges used by only one triangle (holes in the surface).
    pub open_edges: usize,
    /// Edges used by more than two triangles.
    pub non_manifold_edges: usize,
    /// Edges whose two triangles run the same way along them (inconsistent winding).
    pub misoriented_edges: usize,
}

impl MeshReport {
    /// True when every edge is shared by exactly two triangles running opposite ways along it.
    pub fn is_watertight(&self) -> bool {
        self.open_edges == 0 && self.non_manifold_edges == 0 && self.misoriented_edges == 0
    }
}

/// Reads a binary STL and checks its surface. Vertices are matched by exact coordinates, as
/// one exporter writes a shared vertex the same way in every triangle.
pub fn analyze_stl(bytes: &[u8]) -> Result<MeshReport> {
    let bad = |why: &str| Err(CustomError::Export(format!("STL: {why}")));
    if bytes.len() < 84 {
        return bad("shorter than its header");
    }
    let n = u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]) as usize;
    if bytes.len() != 84 + n * 50 {
        return bad("length does not match its triangle count");
    }
    let f = |o: usize| f32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
    let mut ids: HashMap<[u32; 3], u32> = HashMap::new();
    let mut points: Vec<[f64; 3]> = Vec::new();
    // Directed edge (a, b) -> how many triangles run a→b.
    let mut edges: HashMap<(u32, u32), u32> = HashMap::new();
    let mut volume = 0.0;
    let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
    for t in 0..n {
        let base = 84 + t * 50 + 12;
        let mut tri = [0u32; 3];
        let mut pos = [[0.0f64; 3]; 3];
        for (k, slot) in tri.iter_mut().enumerate() {
            let o = base + k * 12;
            let v = [f(o), f(o + 4), f(o + 8)];
            if v.iter().any(|c| !c.is_finite()) {
                return bad("a vertex is not a finite number");
            }
            let key = v.map(f32::to_bits);
            let next = u32::try_from(points.len()).unwrap_or(u32::MAX);
            let id = *ids.entry(key).or_insert_with(|| {
                points.push(v.map(f64::from));
                next
            });
            *slot = id;
            pos[k] = v.map(f64::from);
            for a in 0..3 {
                lo[a] = lo[a].min(pos[k][a]);
                hi[a] = hi[a].max(pos[k][a]);
            }
        }
        let [a, b, c] = pos;
        volume += (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
            + a[2] * (b[0] * c[1] - b[1] * c[0]))
            / 6.0;
        if tri[0] == tri[1] || tri[1] == tri[2] || tri[0] == tri[2] {
            continue; // a sliver collapsed to a line adds no edges
        }
        for (p, q) in [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])] {
            *edges.entry((p, q)).or_default() += 1;
        }
    }
    let (mut open, mut non_manifold, mut misoriented) = (0, 0, 0);
    for (&(a, b), &fwd) in &edges {
        let back = edges.get(&(b, a)).copied().unwrap_or(0);
        // Look at each undirected edge once.
        if (a, b) > (b, a) && back > 0 {
            continue;
        }
        match (fwd, back) {
            (1, 1) => {}
            (1, 0) => open += 1,
            (2, 0) => misoriented += 1,
            _ => non_manifold += 1,
        }
    }
    if n == 0 {
        lo = [0.0; 3];
        hi = [0.0; 3];
    }
    Ok(MeshReport {
        triangles: n,
        volume_mm3: volume,
        bbox_mm: (lo, hi),
        open_edges: open,
        non_manifold_edges: non_manifold,
        misoriented_edges: misoriented,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stl(tris: &[[[f32; 3]; 3]]) -> Vec<u8> {
        let mut out = vec![0u8; 80];
        out.extend_from_slice(&(tris.len() as u32).to_le_bytes());
        for t in tris {
            out.extend_from_slice(&[0u8; 12]);
            for v in t {
                for c in v {
                    out.extend_from_slice(&c.to_le_bytes());
                }
            }
            out.extend_from_slice(&[0u8; 2]);
        }
        out
    }

    #[test]
    fn a_tetrahedron_is_watertight_and_a_missing_face_is_not() {
        let (o, x, y, z) = (
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
        );
        let tris = [[o, y, x], [o, x, z], [o, z, y], [x, y, z]];
        let r = analyze_stl(&stl(&tris)).unwrap();
        assert!(r.is_watertight(), "{r:?}");
        assert!((r.volume_mm3 - 1.0 / 6.0).abs() < 1e-9);
        let r = analyze_stl(&stl(&tris[..3])).unwrap();
        assert_eq!(r.open_edges, 3);
        assert!(analyze_stl(&[0u8; 10]).is_err());
    }
}
