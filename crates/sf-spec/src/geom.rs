//! Shape geometry: primitives, transforms and inside tests.

use crate::{Op, ShapeSpec, resolve_color};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use sf_core::{ColorId, Palette};

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    X,
    #[default]
    Y,
    Z,
}

/// The plane an `extrude` outline is drawn in.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Plane {
    /// Points are `(x, z)`, extruded along `y` (a footprint).
    #[default]
    Xz,
    /// Points are `(x, y)`, extruded along `z` (a front-facing silhouette).
    Xy,
    /// Points are `(z, y)`, extruded along `x` (a side profile).
    Zy,
}

fn one_f() -> f32 {
    1.0
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum Geom {
    Box {
        center: [f32; 3],
        size: [f32; 3],
    },
    Ellipsoid {
        center: [f32; 3],
        size: [f32; 3],
    },
    Cylinder {
        center: [f32; 3],
        size: [f32; 3],
        #[serde(default)]
        axis: Axis,
    },
    /// A cone or frustum whose tip points toward `+axis`. `top_scale` is the
    /// tip radius as a fraction of the base radius (0 = pointed).
    Cone {
        center: [f32; 3],
        size: [f32; 3],
        #[serde(default)]
        axis: Axis,
        #[serde(default)]
        top_scale: f32,
    },
    /// A rounded rod between two points; good for limbs, tails and handles.
    Capsule {
        from: [f32; 3],
        to: [f32; 3],
        radius: f32,
    },
    /// A polygon outline extruded between `depth[0]` and `depth[1]`.
    Extrude {
        #[serde(default)]
        plane: Plane,
        points: Vec<[f32; 2]>,
        depth: [f32; 2],
    },
    /// Children placed as a unit: scaled, turned about `y`, then moved to `at`.
    Group {
        #[serde(default)]
        at: [f32; 3],
        #[serde(default)]
        rotate_y: f32,
        #[serde(default = "one_f")]
        scale: f32,
        children: Vec<ShapeSpec>,
    },
}

/// Affine transform `p ↦ m·p + t`.
#[derive(Copy, Clone, Debug)]
pub(crate) struct Affine {
    m: [[f32; 3]; 3],
    t: [f32; 3],
}

impl Affine {
    const IDENTITY: Affine = Affine { m: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]], t: [0.; 3] };

    fn translate(t: [f32; 3]) -> Self {
        Affine { t, ..Self::IDENTITY }
    }

    fn linear(m: [[f32; 3]; 3]) -> Self {
        Affine { m, t: [0.; 3] }
    }

    fn euler_deg(r: [f32; 3]) -> Self {
        let (rx, ry, rz) = (r[0].to_radians(), r[1].to_radians(), r[2].to_radians());
        let x = [[1., 0., 0.], [0., rx.cos(), -rx.sin()], [0., rx.sin(), rx.cos()]];
        let y = [[ry.cos(), 0., ry.sin()], [0., 1., 0.], [-ry.sin(), 0., ry.cos()]];
        let z = [[rz.cos(), -rz.sin(), 0.], [rz.sin(), rz.cos(), 0.], [0., 0., 1.]];
        Self::linear(z).then(&Self::linear(y)).then(&Self::linear(x))
    }

    /// `self ∘ other` (apply `other` first).
    fn then(&self, o: &Affine) -> Affine {
        let mut m = [[0.0; 3]; 3];
        for (i, row) in m.iter_mut().enumerate() {
            for (j, v) in row.iter_mut().enumerate() {
                *v = (0..3).map(|k| self.m[i][k] * o.m[k][j]).sum();
            }
        }
        let t = self.apply(o.t);
        Affine { m, t }
    }

    fn apply(&self, p: [f32; 3]) -> [f32; 3] {
        let m = &self.m;
        [
            m[0][0] * p[0] + m[0][1] * p[1] + m[0][2] * p[2] + self.t[0],
            m[1][0] * p[0] + m[1][1] * p[1] + m[1][2] * p[2] + self.t[1],
            m[2][0] * p[0] + m[2][1] * p[1] + m[2][2] * p[2] + self.t[2],
        ]
    }

    fn inverse(&self) -> Option<Affine> {
        let m = &self.m;
        let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
        if det.abs() < 1e-9 {
            return None;
        }
        let d = 1.0 / det;
        let inv = [
            [
                (m[1][1] * m[2][2] - m[1][2] * m[2][1]) * d,
                (m[0][2] * m[2][1] - m[0][1] * m[2][2]) * d,
                (m[0][1] * m[1][2] - m[0][2] * m[1][1]) * d,
            ],
            [
                (m[1][2] * m[2][0] - m[1][0] * m[2][2]) * d,
                (m[0][0] * m[2][2] - m[0][2] * m[2][0]) * d,
                (m[0][2] * m[1][0] - m[0][0] * m[1][2]) * d,
            ],
            [
                (m[1][0] * m[2][1] - m[1][1] * m[2][0]) * d,
                (m[0][1] * m[2][0] - m[0][0] * m[2][1]) * d,
                (m[0][0] * m[1][1] - m[0][1] * m[1][0]) * d,
            ],
        ];
        let a = Affine { m: inv, t: [0.; 3] };
        let t = a.apply(self.t);
        Some(Affine { m: inv, t: [-t[0], -t[1], -t[2]] })
    }
}

#[derive(Clone, Debug)]
enum Kind {
    Box { h: [f32; 3] },
    Ellipsoid { h: [f32; 3] },
    Cylinder { h: [f32; 3], axis: Axis },
    Cone { h: [f32; 3], axis: Axis, top: f32 },
    Capsule { a: [f32; 3], b: [f32; 3], r: f32 },
    Extrude { plane: Plane, pts: Vec<[f32; 2]>, d: [f32; 2] },
}

#[derive(Clone, Debug)]
pub(crate) struct Prim {
    kind: Kind,
    pub op: Op,
    pub color: ColorId,
    pub label: String,
    world: Affine,
    inv: Affine,
}

/// Split `p` into (axial, radial-1, radial-2) for an axis.
fn axial(p: [f32; 3], h: [f32; 3], axis: Axis) -> (f32, f32, f32, f32, f32, f32) {
    match axis {
        Axis::X => (p[0], h[0], p[1], h[1], p[2], h[2]),
        Axis::Y => (p[1], h[1], p[0], h[0], p[2], h[2]),
        Axis::Z => (p[2], h[2], p[0], h[0], p[1], h[1]),
    }
}

fn point_in_polygon(pts: &[[f32; 2]], x: f32, y: f32) -> bool {
    let mut inside = false;
    let n = pts.len();
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = (pts[i][0], pts[i][1]);
        let (xj, yj) = (pts[j][0], pts[j][1]);
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

impl Prim {
    pub fn contains(&self, w: [f32; 3]) -> bool {
        let p = self.inv.apply(w);
        match &self.kind {
            Kind::Box { h } => p[0].abs() <= h[0] && p[1].abs() <= h[1] && p[2].abs() <= h[2],
            Kind::Ellipsoid { h } => (p[0] / h[0]).powi(2) + (p[1] / h[1]).powi(2) + (p[2] / h[2]).powi(2) <= 1.0,
            Kind::Cylinder { h, axis } => {
                let (a, ha, r1, h1, r2, h2) = axial(p, *h, *axis);
                a.abs() <= ha && (r1 / h1).powi(2) + (r2 / h2).powi(2) <= 1.0
            }
            Kind::Cone { h, axis, top } => {
                let (a, ha, r1, h1, r2, h2) = axial(p, *h, *axis);
                if a.abs() > ha {
                    return false;
                }
                let t = (a + ha) / (2.0 * ha);
                let r = 1.0 - t * (1.0 - top);
                (r1 / h1).powi(2) + (r2 / h2).powi(2) <= r * r
            }
            Kind::Capsule { a, b, r } => {
                let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                let ap = [p[0] - a[0], p[1] - a[1], p[2] - a[2]];
                let len2 = ab[0] * ab[0] + ab[1] * ab[1] + ab[2] * ab[2];
                let t = if len2 > 0.0 { ((ap[0] * ab[0] + ap[1] * ab[1] + ap[2] * ab[2]) / len2).clamp(0.0, 1.0) } else { 0.0 };
                let d = [ap[0] - ab[0] * t, ap[1] - ab[1] * t, ap[2] - ab[2] * t];
                d[0] * d[0] + d[1] * d[1] + d[2] * d[2] <= r * r
            }
            Kind::Extrude { plane, pts, d } => {
                let (u, v, w) = match plane {
                    Plane::Xz => (p[0], p[2], p[1]),
                    Plane::Xy => (p[0], p[1], p[2]),
                    Plane::Zy => (p[2], p[1], p[0]),
                };
                w >= d[0].min(d[1]) && w <= d[0].max(d[1]) && point_in_polygon(pts, u, v)
            }
        }
    }

    fn local_aabb(&self) -> ([f32; 3], [f32; 3]) {
        match &self.kind {
            Kind::Box { h } | Kind::Ellipsoid { h } | Kind::Cylinder { h, .. } | Kind::Cone { h, .. } => {
                ([-h[0], -h[1], -h[2]], *h)
            }
            Kind::Capsule { a, b, r } => (
                [a[0].min(b[0]) - r, a[1].min(b[1]) - r, a[2].min(b[2]) - r],
                [a[0].max(b[0]) + r, a[1].max(b[1]) + r, a[2].max(b[2]) + r],
            ),
            Kind::Extrude { plane, pts, d } => {
                let (mut u0, mut v0, mut u1, mut v1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
                for p in pts {
                    u0 = u0.min(p[0]);
                    u1 = u1.max(p[0]);
                    v0 = v0.min(p[1]);
                    v1 = v1.max(p[1]);
                }
                let (w0, w1) = (d[0].min(d[1]), d[0].max(d[1]));
                match plane {
                    Plane::Xz => ([u0, w0, v0], [u1, w1, v1]),
                    Plane::Xy => ([u0, v0, w0], [u1, v1, w1]),
                    Plane::Zy => ([w0, v0, u0], [w1, v1, u1]),
                }
            }
        }
    }

    /// World-space bounding box.
    pub fn aabb(&self) -> ([f32; 3], [f32; 3]) {
        let (lo, hi) = self.local_aabb();
        let mut wlo = [f32::MAX; 3];
        let mut whi = [f32::MIN; 3];
        for i in 0..8 {
            let c = [
                if i & 1 == 0 { lo[0] } else { hi[0] },
                if i & 2 == 0 { lo[1] } else { hi[1] },
                if i & 4 == 0 { lo[2] } else { hi[2] },
            ];
            let w = self.world.apply(c);
            for k in 0..3 {
                wlo[k] = wlo[k].min(w[k]);
                whi[k] = whi[k].max(w[k]);
            }
        }
        (wlo, whi)
    }
}

fn check_size(label: &str, size: [f32; 3]) -> Result<[f32; 3]> {
    if size.iter().any(|v| !v.is_finite() || *v <= 0.0) {
        bail!("shape `{label}` needs a positive `size` on every axis, got {size:?}");
    }
    Ok([size[0] / 2.0, size[1] / 2.0, size[2] / 2.0])
}

/// Flatten groups and mirrors into a list of primitives in spec order.
pub(crate) fn flatten(
    shapes: &[ShapeSpec],
    palette: &Palette,
    default_color: ColorId,
    warnings: &mut Vec<String>,
) -> Result<Vec<Prim>> {
    let mut out = Vec::new();
    walk(shapes, &Affine::IDENTITY, palette, default_color, warnings, &mut out, "")?;
    Ok(out)
}

fn walk(
    shapes: &[ShapeSpec],
    parent: &Affine,
    palette: &Palette,
    inherited: ColorId,
    warnings: &mut Vec<String>,
    out: &mut Vec<Prim>,
    prefix: &str,
) -> Result<()> {
    for (i, s) in shapes.iter().enumerate() {
        let label = match &s.id {
            Some(id) => format!("{prefix}{id}"),
            None => format!("{prefix}#{i}"),
        };
        let color = match &s.color {
            Some(c) => resolve_color(palette, c, warnings),
            None => inherited,
        };
        let mirror = Affine::linear([[-1., 0., 0.], [0., 1., 0.], [0., 0., 1.]]);
        if let Geom::Group { at, rotate_y, scale, children } = &s.geom {
            if *scale <= 0.0 || !scale.is_finite() {
                bail!("group `{label}` needs a positive `scale`");
            }
            let s3 = Affine::linear([[*scale, 0., 0.], [0., *scale, 0.], [0., 0., *scale]]);
            let local = Affine::translate(*at).then(&Affine::euler_deg([0.0, *rotate_y, 0.0])).then(&s3);
            let g = parent.then(&local);
            walk(children, &g, palette, color, warnings, out, &format!("{label}/"))?;
            if s.mirror_x {
                let gm = parent.then(&mirror).then(&local);
                walk(children, &gm, palette, color, warnings, out, &format!("{label}(mirror)/"))?;
            }
            continue;
        }
        let rot = Affine::euler_deg(s.rotate.unwrap_or([0.0; 3]));
        let (kind, local) = match &s.geom {
            Geom::Box { center, size } => (Kind::Box { h: check_size(&label, *size)? }, Affine::translate(*center).then(&rot)),
            Geom::Ellipsoid { center, size } => {
                (Kind::Ellipsoid { h: check_size(&label, *size)? }, Affine::translate(*center).then(&rot))
            }
            Geom::Cylinder { center, size, axis } => {
                (Kind::Cylinder { h: check_size(&label, *size)?, axis: *axis }, Affine::translate(*center).then(&rot))
            }
            Geom::Cone { center, size, axis, top_scale } => (
                Kind::Cone { h: check_size(&label, *size)?, axis: *axis, top: top_scale.clamp(0.0, 1.0) },
                Affine::translate(*center).then(&rot),
            ),
            Geom::Capsule { from, to, radius } => {
                if *radius <= 0.0 {
                    bail!("capsule `{label}` needs a positive `radius`");
                }
                (Kind::Capsule { a: *from, b: *to, r: *radius }, Affine::IDENTITY)
            }
            Geom::Extrude { plane, points, depth } => {
                if points.len() < 3 {
                    bail!("extrude `{label}` needs at least 3 points");
                }
                (Kind::Extrude { plane: *plane, pts: points.clone(), d: *depth }, Affine::IDENTITY)
            }
            Geom::Group { .. } => unreachable!(),
        };
        let mut push = |world: Affine, label: String| -> Result<()> {
            let Some(inv) = world.inverse() else { bail!("shape `{label}` has a degenerate transform") };
            out.push(Prim { kind: kind.clone(), op: s.op, color, label, world, inv });
            Ok(())
        };
        push(parent.then(&local), label.clone())?;
        if s.mirror_x {
            push(parent.then(&mirror).then(&local), format!("{label}(mirror)"))?;
        }
    }
    Ok(())
}

/// Bounding box of all `add` primitives.
pub(crate) fn bounds(prims: &[Prim]) -> Option<([f32; 3], [f32; 3])> {
    let mut it = prims.iter().filter(|p| p.op == Op::Add).map(|p| p.aabb());
    let (mut lo, mut hi) = it.next()?;
    for (l, h) in it {
        for k in 0..3 {
            lo[k] = lo[k].min(l[k]);
            hi[k] = hi[k].max(h[k]);
        }
    }
    Some((lo, hi))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn affine_inverse_roundtrip() {
        let a = Affine::translate([1.0, 2.0, 3.0]).then(&Affine::euler_deg([30.0, 45.0, 10.0]));
        let inv = a.inverse().unwrap();
        let p = [0.3, -1.2, 4.0];
        let q = inv.apply(a.apply(p));
        for k in 0..3 {
            assert!((p[k] - q[k]).abs() < 1e-5);
        }
    }

    #[test]
    fn polygon_test() {
        let sq = [[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]];
        assert!(point_in_polygon(&sq, 1.0, 1.0));
        assert!(!point_in_polygon(&sq, 3.0, 1.0));
    }
}
