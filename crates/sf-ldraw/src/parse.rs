//! LDraw file parsing and flattening into a [`PartMesh`].

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// LDraw colour code meaning "inherit the main (placement) colour".
pub const MAIN_COLOR: u16 = 16;
/// LDraw colour code meaning "edge colour of the current colour".
pub const EDGE_COLOR: u16 = 24;

/// A flattened part in LDraw coordinates (LDU, -Y up).
///
/// Triangles are stored unshared: every triangle owns three consecutive
/// vertices, so `normals` are flat face normals and `indices` is simply
/// `0..positions.len()`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PartMesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
    /// Per-vertex LDraw colour code; [`MAIN_COLOR`] (16) means "use the
    /// placement's colour".
    pub colors: Vec<u16>,
    /// Type-2 edge lines.
    pub edges: Vec<[[f32; 3]; 2]>,
}

impl PartMesh {
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Axis-aligned bounds `(min, max)` of all triangle vertices.
    pub fn bounds(&self) -> Option<([f32; 3], [f32; 3])> {
        let first = *self.positions.first()?;
        let (mut lo, mut hi) = (first, first);
        for p in &self.positions {
            for i in 0..3 {
                lo[i] = lo[i].min(p[i]);
                hi[i] = hi[i].max(p[i]);
            }
        }
        Some((lo, hi))
    }

    /// Append a triangle with a flat normal. Degenerate triangles are dropped.
    pub fn push_tri(&mut self, a: [f32; 3], b: [f32; 3], c: [f32; 3], color: u16) {
        let n = cross(sub(b, a), sub(c, a));
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if len < 1e-6 || !len.is_finite() {
            return;
        }
        let n = [n[0] / len, n[1] / len, n[2] / len];
        let base = self.positions.len() as u32;
        self.positions.extend_from_slice(&[a, b, c]);
        self.normals.extend_from_slice(&[n, n, n]);
        self.colors.extend_from_slice(&[color, color, color]);
        self.indices.extend_from_slice(&[base, base + 1, base + 2]);
    }

    /// Append a quad `a b c d` as triangles `a b c` and `a c d`.
    pub fn push_quad(&mut self, a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3], color: u16) {
        self.push_tri(a, b, c, color);
        self.push_tri(a, c, d, color);
    }
}

/// A 4×3 affine transform: `p' = pos + m · p` (row-major `m`).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Xform {
    pub pos: [f32; 3],
    pub m: [[f32; 3]; 3],
}

impl Xform {
    pub const IDENTITY: Xform = Xform { pos: [0.0; 3], m: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]] };

    #[inline]
    pub fn apply(&self, p: [f32; 3]) -> [f32; 3] {
        let m = &self.m;
        [
            self.pos[0] + m[0][0] * p[0] + m[0][1] * p[1] + m[0][2] * p[2],
            self.pos[1] + m[1][0] * p[0] + m[1][1] * p[1] + m[1][2] * p[2],
            self.pos[2] + m[2][0] * p[0] + m[2][1] * p[1] + m[2][2] * p[2],
        ]
    }

    /// `self ∘ child`: first apply `child`, then `self`.
    pub fn then_child(&self, child: &Xform) -> Xform {
        let mut m = [[0.0f32; 3]; 3];
        for (i, row) in m.iter_mut().enumerate() {
            for (j, v) in row.iter_mut().enumerate() {
                *v = (0..3).map(|k| self.m[i][k] * child.m[k][j]).sum();
            }
        }
        Xform { pos: self.apply(child.pos), m }
    }

    pub fn det(&self) -> f32 {
        let m = &self.m;
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    }
}

/// One drawable command from an LDraw file.
#[derive(Clone, Debug, PartialEq)]
pub enum Cmd {
    Sub { color: u16, xf: Xform, file: String },
    Line { a: [f32; 3], b: [f32; 3] },
    Tri { color: u16, p: [[f32; 3]; 3] },
    Quad { color: u16, p: [[f32; 3]; 4] },
}

/// Normalise an LDraw file reference: backslashes to `/`, lowercase, trimmed.
pub fn normalize_name(s: &str) -> String {
    s.trim().replace('\\', "/").to_ascii_lowercase()
}

fn parse_color(s: &str) -> u16 {
    // Direct colours (0x2RRGGBB) and anything out of range fall back to "inherit".
    s.parse::<u32>().ok().and_then(|v| u16::try_from(v).ok()).unwrap_or(MAIN_COLOR)
}

fn floats<const N: usize>(toks: &[&str]) -> Option<[f32; N]> {
    if toks.len() < N {
        return None;
    }
    let mut out = [0.0f32; N];
    for (o, t) in out.iter_mut().zip(toks) {
        *o = t.parse().ok()?;
        if !o.is_finite() {
            return None;
        }
    }
    Some(out)
}

/// Split off the first `n` whitespace-separated tokens; return them and the
/// trimmed remainder (used for type-1 filenames, which may contain spaces).
fn split_tokens(line: &str, n: usize) -> Option<(Vec<&str>, &str)> {
    let mut toks = Vec::with_capacity(n);
    let mut rest = line.trim_start();
    for _ in 0..n {
        let end = rest.find(char::is_whitespace)?;
        toks.push(&rest[..end]);
        rest = rest[end..].trim_start();
    }
    Some((toks, rest.trim_end()))
}

/// Parse the drawable commands of an LDraw file. Comments, meta commands,
/// conditional lines (type 5) and malformed lines are skipped.
pub fn parse_file(text: &str) -> Vec<Cmd> {
    let mut out = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        let Some(kind) = line.split_whitespace().next() else { continue };
        let toks: Vec<&str> = line.split_whitespace().collect();
        match kind {
            "1" => {
                let Some((t, file)) = split_tokens(line, 14) else { continue };
                if file.is_empty() {
                    continue;
                }
                let Some(v) = floats::<12>(&t[2..]) else { continue };
                let xf = Xform { pos: [v[0], v[1], v[2]], m: [[v[3], v[4], v[5]], [v[6], v[7], v[8]], [v[9], v[10], v[11]]] };
                out.push(Cmd::Sub { color: parse_color(t[1]), xf, file: normalize_name(file) });
            }
            "2" => {
                if let Some(v) = floats::<6>(toks.get(2..).unwrap_or(&[])) {
                    out.push(Cmd::Line { a: [v[0], v[1], v[2]], b: [v[3], v[4], v[5]] });
                }
            }
            "3" => {
                if let Some(v) = floats::<9>(toks.get(2..).unwrap_or(&[])) {
                    let p = [[v[0], v[1], v[2]], [v[3], v[4], v[5]], [v[6], v[7], v[8]]];
                    out.push(Cmd::Tri { color: parse_color(toks[1]), p });
                }
            }
            "4" => {
                if let Some(v) = floats::<12>(toks.get(2..).unwrap_or(&[])) {
                    let p = [[v[0], v[1], v[2]], [v[3], v[4], v[5]], [v[6], v[7], v[8]], [v[9], v[10], v[11]]];
                    out.push(Cmd::Quad { color: parse_color(toks[1]), p });
                }
            }
            _ => {}
        }
    }
    out
}

/// The type-1 references of a parsed file (normalised names).
pub fn references(cmds: &[Cmd]) -> impl Iterator<Item = &str> {
    cmds.iter().filter_map(|c| match c {
        Cmd::Sub { file, .. } => Some(file.as_str()),
        _ => None,
    })
}

/// Resolve a colour: 16 and 24 inherit from the parent's colour.
#[inline]
fn resolve(color: u16, parent: u16) -> u16 {
    if color == MAIN_COLOR || color == EDGE_COLOR { parent } else { color }
}

/// Loads file text by normalised reference name, e.g. `"s/3001s01.dat"` or
/// `"stud.dat"`, applying the library search path.
pub trait FileSource {
    fn load(&self, name: &str) -> Option<String>;
}

impl<F: Fn(&str) -> Option<String>> FileSource for F {
    fn load(&self, name: &str) -> Option<String> {
        self(name)
    }
}

/// Flattens files into meshes, caching parsed files across calls.
#[derive(Default)]
pub struct Flattener {
    parsed: HashMap<String, Option<Arc<Vec<Cmd>>>>,
    /// References that could not be found.
    pub missing: HashSet<String>,
}

const MAX_DEPTH: usize = 32;

impl Flattener {
    pub fn new() -> Self {
        Self::default()
    }

    fn get(&mut self, src: &dyn FileSource, name: &str) -> Option<Arc<Vec<Cmd>>> {
        if let Some(c) = self.parsed.get(name) {
            return c.clone();
        }
        let cmds = src.load(name).map(|t| Arc::new(parse_file(&t)));
        if cmds.is_none() {
            self.missing.insert(name.to_string());
        }
        self.parsed.insert(name.to_string(), cmds.clone());
        cmds
    }

    /// Flatten `name` (a normalised reference) into a mesh. Returns `None`
    /// if the top-level file can't be loaded; missing subfiles are skipped
    /// and recorded in [`Flattener::missing`].
    pub fn flatten(&mut self, src: &dyn FileSource, name: &str) -> Option<PartMesh> {
        let cmds = self.get(src, name)?;
        let mut mesh = PartMesh::default();
        self.walk(src, &cmds, &Xform::IDENTITY, MAIN_COLOR, 0, &mut mesh);
        Some(mesh)
    }

    /// Flatten already-parsed commands (useful for tests and inline models).
    pub fn flatten_cmds(&mut self, src: &dyn FileSource, cmds: &[Cmd]) -> PartMesh {
        let mut mesh = PartMesh::default();
        self.walk(src, cmds, &Xform::IDENTITY, MAIN_COLOR, 0, &mut mesh);
        mesh
    }

    fn walk(&mut self, src: &dyn FileSource, cmds: &[Cmd], xf: &Xform, color: u16, depth: usize, mesh: &mut PartMesh) {
        if depth > MAX_DEPTH {
            return;
        }
        // A mirroring transform flips winding; swap two vertices to keep
        // triangles consistently oriented.
        let flip = xf.det() < 0.0;
        for cmd in cmds {
            match cmd {
                Cmd::Sub { color: c, xf: child, file } => {
                    if let Some(sub) = self.get(src, file) {
                        let x = xf.then_child(child);
                        self.walk(src, &sub, &x, resolve(*c, color), depth + 1, mesh);
                    }
                }
                Cmd::Line { a, b } => mesh.edges.push([xf.apply(*a), xf.apply(*b)]),
                Cmd::Tri { color: c, p } => {
                    let [a, b, cc] = p.map(|v| xf.apply(v));
                    let col = resolve(*c, color);
                    if flip { mesh.push_tri(a, cc, b, col) } else { mesh.push_tri(a, b, cc, col) }
                }
                Cmd::Quad { color: c, p } => {
                    let [a, b, cc, d] = p.map(|v| xf.apply(v));
                    let col = resolve(*c, color);
                    if flip { mesh.push_quad(a, d, cc, b, col) } else { mesh.push_quad(a, b, cc, d, col) }
                }
            }
        }
    }
}

#[inline]
pub(crate) fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

#[inline]
pub(crate) fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lib(files: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |n: &str| files.iter().find(|(k, _)| *k == n).map(|(_, v)| v.to_string())
    }

    #[test]
    fn quad_splits_into_two_triangles() {
        let cmds = parse_file("4 16 0 0 0 10 0 0 10 0 10 0 0 10\n");
        let m = Flattener::new().flatten_cmds(&|_: &str| None, &cmds);
        assert_eq!(m.triangle_count(), 2);
        assert_eq!(m.positions.len(), 6);
        assert_eq!(m.positions[0], [0., 0., 0.]);
        assert_eq!(m.positions[1], [10., 0., 0.]);
        assert_eq!(m.positions[2], [10., 0., 10.]);
        assert_eq!(m.positions[5], [0., 0., 10.]);
        // Both triangles share the same flat normal (±Y).
        assert!(m.normals.iter().all(|n| n[1].abs() > 0.999));
        assert_eq!(m.normals[0], m.normals[3]);
    }

    #[test]
    fn subfile_transforms_compose() {
        // b.dat: one triangle at the origin. a.dat: b scaled ×2 and moved +10 x.
        // top: a rotated a quarter turn about y and moved +100 z.
        let files: &'static [(&str, &str)] = &[
            ("b.dat", "3 16 1 0 0 0 1 0 0 0 1\n"),
            ("a.dat", "1 16 10 0 0 2 0 0 0 2 0 0 0 2 B.DAT\n"),
        ];
        let src = lib(files);
        let top = parse_file("1 16 0 0 100 0 0 1 0 1 0 -1 0 0 a.dat\n");
        let m = Flattener::new().flatten_cmds(&src, &top);
        assert_eq!(m.triangle_count(), 1);
        // (1,0,0) -> a: (12,0,0) -> top: x' = z = 0, z' = 100 - x = 88.
        assert_eq!(m.positions[0], [0., 0., 88.]);
        // (0,1,0) -> a: (10,2,0) -> top: (0, 2, 90)
        assert_eq!(m.positions[1], [0., 2., 90.]);
        // (0,0,1) -> a: (10,0,2) -> top: (2, 0, 90)
        assert_eq!(m.positions[2], [2., 0., 90.]);
    }

    #[test]
    fn colour_inheritance() {
        let files: &'static [(&str, &str)] = &[
            ("inner.dat", "3 16 0 0 0 1 0 0 0 0 1\n3 15 0 0 0 1 0 0 0 0 1\n2 24 0 0 0 1 1 1\n"),
            ("outer.dat", "1 16 0 0 0 1 0 0 0 1 0 0 0 1 inner.dat\n1 4 0 0 0 1 0 0 0 1 0 0 0 1 inner.dat\n"),
        ];
        let src = lib(files);
        let m = Flattener::new().flatten(&src, "outer.dat").unwrap();
        let tri_colors: Vec<u16> = m.colors.chunks(3).map(|c| c[0]).collect();
        // 16 under 16 stays 16 (main colour); explicit 15 stays; 16 under 4 becomes 4.
        assert_eq!(tri_colors, vec![16, 15, 4, 15]);
        assert_eq!(m.edges.len(), 2);
    }

    #[test]
    fn mirrored_subfile_keeps_winding() {
        let files: &'static [(&str, &str)] = &[("t.dat", "3 16 0 0 0 1 0 0 0 0 1\n")];
        let src = lib(files);
        let plain = Flattener::new().flatten_cmds(&src, &parse_file("1 16 0 0 0 1 0 0 0 1 0 0 0 1 t.dat"));
        let mirror = Flattener::new().flatten_cmds(&src, &parse_file("1 16 0 0 0 -1 0 0 0 1 0 0 0 1 t.dat"));
        // Mirroring x flips the geometric normal's x component only if the
        // winding is kept consistent; here the triangle's normal is along y
        // (x,z plane is the triangle) — the y component must keep its sign.
        assert!((plain.normals[0][1] - mirror.normals[0][1]).abs() < 1e-6);
    }

    #[test]
    fn missing_subfiles_are_skipped() {
        let mut f = Flattener::new();
        let m = f.flatten_cmds(&|_: &str| None, &parse_file("1 16 0 0 0 1 0 0 0 1 0 0 0 1 nope.dat\n3 16 0 0 0 1 0 0 0 0 1"));
        assert_eq!(m.triangle_count(), 1);
        assert!(f.missing.contains("nope.dat"));
    }

    #[test]
    fn filenames_with_spaces_and_bad_lines() {
        let cmds = parse_file("1 16 0 0 0 1 0 0 0 1 0 0 0 1 My Part.dat\n3 16 a b c\n5 24 0 0 0 1 1 1 0 0 0 0 0 0\n0 comment");
        assert_eq!(cmds.len(), 1);
        assert!(matches!(&cmds[0], Cmd::Sub { file, .. } if file == "my part.dat"));
    }
}
