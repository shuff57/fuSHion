//! Exact plane regions bounded by lines and circular arcs.
//!
//! Areas come from Green's theorem, `A = ½∮(x dy − y dx)`, which is closed-form on lines and
//! arcs. The area two regions share is found the same way: each outline is split where it
//! crosses the other, and the pieces of each that lie inside the other are integrated. That
//! needs no polygon approximation, so the Slant v2 stud's slot, pocket and fin areas are as
//! exact as a circle's.

use crate::shape::P2;
use std::f64::consts::TAU;

/// One piece of an outline: a line, or an arc of the circle about `c` through `a` and `b`
/// (running counter-clockwise from `a` to `b` when `ccw`, else clockwise).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Seg {
    Line { a: P2, b: P2 },
    Arc { a: P2, b: P2, c: P2, ccw: bool },
}

/// A closed outline: each segment starts where the one before it ends.
pub(crate) type Loop = Vec<Seg>;

fn sub(p: P2, q: P2) -> P2 {
    [p[0] - q[0], p[1] - q[1]]
}

fn len(p: P2) -> f64 {
    p[0].hypot(p[1])
}

fn ang(p: P2, c: P2) -> f64 {
    (p[1] - c[1]).atan2(p[0] - c[0])
}

/// `x` brought into `(0, 2π]`.
fn wrap_pos(x: f64) -> f64 {
    let y = x.rem_euclid(TAU);
    if y <= 1e-15 { TAU } else { y }
}

impl Seg {
    /// Radius, start angle and signed sweep of an arc.
    fn arc_params(&self) -> Option<(P2, f64, f64, f64)> {
        let Self::Arc { a, b, c, ccw } = *self else {
            return None;
        };
        let r = len(sub(a, c));
        let (t0, t1) = (ang(a, c), ang(b, c));
        let sweep = if ccw {
            wrap_pos(t1 - t0)
        } else {
            -wrap_pos(t0 - t1)
        };
        Some((c, r, t0, sweep))
    }

    /// The point a fraction `s` of the way along.
    pub fn at(&self, s: f64) -> P2 {
        match *self {
            Self::Line { a, b } => [a[0] + s * (b[0] - a[0]), a[1] + s * (b[1] - a[1])],
            Self::Arc { .. } => {
                let (c, r, t0, sw) = self.arc_params().unwrap_or(([0.0; 2], 0.0, 0.0, 0.0));
                let t = t0 + s * sw;
                [c[0] + r * t.cos(), c[1] + r * t.sin()]
            }
        }
    }

    /// Length of the segment.
    pub fn length(&self) -> f64 {
        match *self {
            Self::Line { a, b } => len(sub(b, a)),
            Self::Arc { .. } => self
                .arc_params()
                .map_or(0.0, |(_, r, _, sweep)| r * sweep.abs()),
        }
    }

    /// The exact extent `(min, max)` of the segment: its ends, and for an arc the points
    /// where it passes due east, north, west or south of its centre.
    pub fn bbox(&self) -> (P2, P2) {
        let (a, b) = match *self {
            Self::Line { a, b } | Self::Arc { a, b, .. } => (a, b),
        };
        let mut lo = [a[0].min(b[0]), a[1].min(b[1])];
        let mut hi = [a[0].max(b[0]), a[1].max(b[1])];
        if let Some((c, r, ..)) = self.arc_params() {
            for k in 0..4 {
                let t = f64::from(k) * TAU / 4.0;
                if self.arc_fraction(t).is_some_and(|s| s > 0.0 && s < 1.0) {
                    let p = [c[0] + r * t.cos(), c[1] + r * t.sin()];
                    for i in 0..2 {
                        lo[i] = lo[i].min(p[i]);
                        hi[i] = hi[i].max(p[i]);
                    }
                }
            }
        }
        (lo, hi)
    }

    /// The same segment run the other way.
    pub fn reversed(&self) -> Self {
        match *self {
            Self::Line { a, b } => Self::Line { a: b, b: a },
            Self::Arc { a, b, c, ccw } => Self::Arc {
                a: b,
                b: a,
                c,
                ccw: !ccw,
            },
        }
    }

    /// The piece between fractions `s0` and `s1`.
    fn piece(&self, s0: f64, s1: f64) -> Self {
        match *self {
            Self::Line { .. } => Self::Line {
                a: self.at(s0),
                b: self.at(s1),
            },
            Self::Arc { c, ccw, .. } => Self::Arc {
                a: self.at(s0),
                b: self.at(s1),
                c,
                ccw,
            },
        }
    }

    /// `½∫(x dy − y dx)` along the segment.
    pub fn green(&self) -> f64 {
        match *self {
            Self::Line { a, b } => 0.5 * (a[0] * b[1] - b[0] * a[1]),
            Self::Arc { .. } => {
                let (c, r, t0, sw) = self.arc_params().unwrap_or(([0.0; 2], 0.0, 0.0, 0.0));
                let t1 = t0 + sw;
                0.5 * (r * c[0] * (t1.sin() - t0.sin()) - r * c[1] * (t1.cos() - t0.cos())
                    + r * r * sw)
            }
        }
    }

    /// The fraction along an arc at which its circle has angle `t`, if `t` is on the arc.
    fn arc_fraction(&self, t: f64) -> Option<f64> {
        let (_, _, t0, sw) = self.arc_params()?;
        let d = if sw >= 0.0 {
            (t - t0).rem_euclid(TAU)
        } else {
            (t0 - t).rem_euclid(TAU)
        };
        // Angles just below 2π are the start point seen from the other side.
        let d = if d > TAU - 1e-12 { 0.0 } else { d };
        let s = d / sw.abs();
        (s <= 1.0 + 1e-12).then_some(s.min(1.0))
    }
}

/// Where two segments cross: fractions along each.
fn crossings(p: &Seg, q: &Seg) -> Vec<(f64, f64)> {
    const E: f64 = 1e-12;
    let inside = |s: f64| (-E..=1.0 + E).contains(&s);
    match (*p, *q) {
        (Seg::Line { a, b }, Seg::Line { a: c, b: d }) => {
            let (r, s) = (sub(b, a), sub(d, c));
            let den = r[0] * s[1] - r[1] * s[0];
            if den.abs() < 1e-15 {
                return Vec::new();
            }
            let w = sub(c, a);
            let t = (w[0] * s[1] - w[1] * s[0]) / den;
            let u = (w[0] * r[1] - w[1] * r[0]) / den;
            if inside(t) && inside(u) {
                vec![(t.clamp(0.0, 1.0), u.clamp(0.0, 1.0))]
            } else {
                Vec::new()
            }
        }
        (Seg::Line { .. }, Seg::Arc { .. }) => line_arc(p, q),
        (Seg::Arc { .. }, Seg::Line { .. }) => {
            line_arc(q, p).into_iter().map(|(a, b)| (b, a)).collect()
        }
        (Seg::Arc { .. }, Seg::Arc { .. }) => {
            let (Some((c0, r0, ..)), Some((c1, r1, ..))) = (p.arc_params(), q.arc_params()) else {
                return Vec::new();
            };
            let d = len(sub(c1, c0));
            if d < 1e-12 || d > r0 + r1 || d < (r0 - r1).abs() {
                return Vec::new();
            }
            let x = (d * d + r0 * r0 - r1 * r1) / (2.0 * d);
            let h = (r0 * r0 - x * x).max(0.0).sqrt();
            let e = [(c1[0] - c0[0]) / d, (c1[1] - c0[1]) / d];
            let m = [c0[0] + x * e[0], c0[1] + x * e[1]];
            let mut out = Vec::new();
            for sgn in [1.0, -1.0] {
                let pt = [m[0] - sgn * h * e[1], m[1] + sgn * h * e[0]];
                if let (Some(s), Some(t)) =
                    (p.arc_fraction(ang(pt, c0)), q.arc_fraction(ang(pt, c1)))
                {
                    out.push((s, t));
                }
                if h < 1e-12 {
                    break;
                }
            }
            out
        }
    }
}

/// Crossings of a line `l` with an arc `c`, as fractions along each.
fn line_arc(l: &Seg, c: &Seg) -> Vec<(f64, f64)> {
    let Seg::Line { a, b } = *l else {
        return Vec::new();
    };
    let Some((cc, r, ..)) = c.arc_params() else {
        return Vec::new();
    };
    let d = sub(b, a);
    let f = sub(a, cc);
    let qa = d[0] * d[0] + d[1] * d[1];
    let qb = 2.0 * (f[0] * d[0] + f[1] * d[1]);
    let qc = f[0] * f[0] + f[1] * f[1] - r * r;
    let disc = qb * qb - 4.0 * qa * qc;
    if disc < 0.0 || qa < 1e-30 {
        return Vec::new();
    }
    let sq = disc.sqrt();
    let mut out = Vec::new();
    for t in [(-qb - sq) / (2.0 * qa), (-qb + sq) / (2.0 * qa)] {
        if (-1e-12..=1.0 + 1e-12).contains(&t) {
            let pt = [a[0] + t * d[0], a[1] + t * d[1]];
            if let Some(s) = c.arc_fraction(ang(pt, cc)) {
                out.push((t.clamp(0.0, 1.0), s));
            }
        }
        if sq < 1e-15 {
            break;
        }
    }
    out
}

/// Signed area of a loop (positive counter-clockwise).
pub(crate) fn area(l: &[Seg]) -> f64 {
    l.iter().map(Seg::green).sum()
}

/// The exact extent `(min, max)` of a loop.
pub(crate) fn bbox(l: &[Seg]) -> (P2, P2) {
    l.iter().map(Seg::bbox).fold(
        ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]),
        |(lo, hi), (a, b)| {
            (
                [lo[0].min(a[0]), lo[1].min(a[1])],
                [hi[0].max(b[0]), hi[1].max(b[1])],
            )
        },
    )
}

/// The distance from `p` to the nearest point of a segment.
#[cfg(test)]
pub(crate) fn distance(s: &Seg, p: P2) -> f64 {
    match *s {
        Seg::Line { a, b } => {
            let d = sub(b, a);
            let t = ((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / (d[0] * d[0] + d[1] * d[1]);
            let t = t.clamp(0.0, 1.0);
            len(sub(p, [a[0] + t * d[0], a[1] + t * d[1]]))
        }
        Seg::Arc { a, b, .. } => {
            let Some((c, r, ..)) = s.arc_params() else {
                return len(sub(p, a));
            };
            if s.arc_fraction(ang(p, c)).is_some() {
                (len(sub(p, c)) - r).abs()
            } else {
                len(sub(p, a)).min(len(sub(p, b)))
            }
        }
    }
}

/// The loop run the other way.
pub(crate) fn reversed(l: &[Seg]) -> Loop {
    l.iter().rev().map(Seg::reversed).collect()
}

/// The loop, counter-clockwise.
pub(crate) fn ccw(l: Loop) -> Loop {
    if area(&l) < 0.0 { reversed(&l) } else { l }
}

/// Whether `p` lies inside a loop: crossings of the ray from `p` toward +x, counted on the
/// exact lines and arcs (half-open in `y` on lines; arcs count a crossing strictly inside).
pub(crate) fn contains(l: &[Seg], p: P2) -> bool {
    let mut inside = false;
    for s in l {
        match *s {
            Seg::Line { a, b } => {
                if (a[1] > p[1]) != (b[1] > p[1]) {
                    let x = a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
                    if x > p[0] {
                        inside = !inside;
                    }
                }
            }
            Seg::Arc { .. } => {
                let Some((c, r, ..)) = s.arc_params() else {
                    continue;
                };
                let dy = p[1] - c[1];
                if dy.abs() >= r {
                    continue;
                }
                let dx = (r * r - dy * dy).sqrt();
                for x in [c[0] - dx, c[0] + dx] {
                    if x > p[0]
                        && let Some(f) = s.arc_fraction(dy.atan2(x - c[0]))
                        && f > 1e-12
                        && f < 1.0 - 1e-12
                    {
                        inside = !inside;
                    }
                }
            }
        }
    }
    inside
}

/// The pieces of `a`'s outline (split where `b` crosses it) whose midpoints lie inside `b`.
fn pieces_inside(a: &[Seg], b: &[Seg]) -> f64 {
    let mut total = 0.0;
    for s in a {
        let mut cuts = vec![0.0, 1.0];
        for t in b {
            cuts.extend(crossings(s, t).into_iter().map(|(u, _)| u));
        }
        cuts.sort_by(f64::total_cmp);
        for w in cuts.windows(2) {
            let (s0, s1) = (w[0], w[1]);
            if s1 - s0 < 1e-12 {
                continue;
            }
            if contains(b, s.at((s0 + s1) / 2.0)) {
                total += s.piece(s0, s1).green();
            }
        }
    }
    total
}

/// Area two counter-clockwise loops share. Their outlines must cross cleanly (no shared or
/// tangent stretches), which every caller arranges by construction.
pub(crate) fn intersection_area(a: &[Seg], b: &[Seg]) -> f64 {
    pieces_inside(a, b) + pieces_inside(b, a)
}

/// A rigid map of the plane, possibly mirrored: `(u, v) ↦ o + u·x + v·y`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Frame {
    pub o: P2,
    pub x: P2,
    pub y: P2,
}

impl Frame {
    pub fn map(&self, p: P2) -> P2 {
        [
            self.o[0] + p[0] * self.x[0] + p[1] * self.y[0],
            self.o[1] + p[0] * self.x[1] + p[1] * self.y[1],
        ]
    }

    fn mirrors(&self) -> bool {
        self.x[0] * self.y[1] - self.x[1] * self.y[0] < 0.0
    }

    /// A loop carried by the map (kept counter-clockwise).
    pub fn map_loop(&self, l: &[Seg]) -> Loop {
        let flip = self.mirrors();
        let out: Loop = l
            .iter()
            .map(|s| match *s {
                Seg::Line { a, b } => Seg::Line {
                    a: self.map(a),
                    b: self.map(b),
                },
                Seg::Arc { a, b, c, ccw } => Seg::Arc {
                    a: self.map(a),
                    b: self.map(b),
                    c: self.map(c),
                    ccw: ccw != flip,
                },
            })
            .collect();
        ccw(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn circle(c: P2, r: f64) -> Loop {
        let a = [c[0] + r, c[1]];
        let b = [c[0] - r, c[1]];
        vec![
            Seg::Arc { a, b, c, ccw: true },
            Seg::Arc {
                a: b,
                b: a,
                c,
                ccw: true,
            },
        ]
    }

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Loop {
        let p = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
        (0..4)
            .map(|k| Seg::Line {
                a: p[k],
                b: p[(k + 1) % 4],
            })
            .collect()
    }

    #[test]
    fn areas_of_circles_rectangles_and_their_overlaps() {
        let c = circle([1.0, 2.0], 3.0);
        assert!((area(&c) - 9.0 * PI).abs() < 1e-12);
        assert!((area(&reversed(&c)) + 9.0 * PI).abs() < 1e-12);
        let r = rect(0.0, 0.0, 2.0, 5.0);
        assert!((area(&r) - 10.0).abs() < 1e-12);
        // Two unit circles a distance 1 apart share a lens of 2π/3 − √3/2.
        let lens = intersection_area(&circle([0.0, 0.0], 1.0), &circle([1.0, 0.0], 1.0));
        assert!(
            (lens - (2.0 * PI / 3.0 - 3f64.sqrt() / 2.0)).abs() < 1e-12,
            "{lens}"
        );
        // A circle cut by a half-plane-ish rectangle: half the disk.
        let half = intersection_area(&circle([0.0, 0.0], 1.0), &rect(0.0, -2.0, 3.0, 2.0));
        assert!((half - PI / 2.0).abs() < 1e-12, "{half}");
        // Nested: all of the small one.
        let all = intersection_area(&circle([0.0, 0.0], 0.5), &rect(-1.0, -1.0, 1.0, 1.0));
        assert!((all - PI / 4.0).abs() < 1e-12, "{all}");
        // Apart: nothing.
        assert!(
            intersection_area(&circle([0.0, 0.0], 1.0), &rect(2.0, 2.0, 3.0, 3.0)).abs() < 1e-15
        );
    }

    #[test]
    fn mirrored_frames_keep_loops_counter_clockwise() {
        let f = Frame {
            o: [5.0, 1.0],
            x: [-1.0, 0.0],
            y: [0.0, 1.0],
        };
        let r = f.map_loop(&rect(0.0, 0.0, 2.0, 1.0));
        assert!((area(&r) - 2.0).abs() < 1e-12);
        assert!(contains(&r, [4.0, 1.5]));
        assert!(!contains(&r, [6.0, 1.5]));
        let c = f.map_loop(&circle([1.0, 0.0], 0.5));
        assert!((area(&c) - PI / 4.0).abs() < 1e-12);
    }
}
