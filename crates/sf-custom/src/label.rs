//! Part numbers engraved under a printed part's top, so a builder can match each print to the
//! instructions.
//!
//! The text is drawn in a 3×5 pixel font and engraved [`LABEL_DEPTH`] into the hollow's
//! ceiling, mirrored so it reads correctly when the part is turned over and looked at from
//! below. A pixel font keeps the engraving exact: its area is the pixel count times the pixel
//! area, so the stage's volume check stays a closed form. No glyph has two pixels that touch
//! only at a corner, so every outline is a clean manifold loop.
//!
//! Where it goes: the largest pixel size (from [`PIXEL_SIZES`]) and the spot nearest the
//! footprint's middle where the whole label, grown by [`LABEL_MARGIN`], lies on the bare
//! ceiling: inside the hollow, clear of tubes, ribs and wall ribs, and not over a stud's
//! pocket or slot (which already thin the top from above). If no spot fits, the part has no
//! label.

use crate::geom::{self, Loop};
use crate::shape::{P2, trace_loops};
use std::collections::BTreeSet;

/// How deep the label is engraved into the ceiling, in mm. Under a pocket (0.6 mm into a
/// 1.4 mm top) that would leave 0.4 mm, so labels keep off pockets and slots.
pub const LABEL_DEPTH: f64 = 0.4;
/// Pixel sizes tried, largest first, in mm. 0.4 mm is about one 0.4 mm nozzle line.
pub const PIXEL_SIZES: [f64; 4] = [0.6, 0.5, 0.45, 0.4];
/// Clear room kept round the label, in mm.
pub const LABEL_MARGIN: f64 = 0.3;
/// Characters the font draws. Others (such as the `B` of `"B017"`) are left out of the
/// engraving: the font has no clean 3×5 form for them.
pub const LABEL_CHARS: &str = "0123456789ACEFHLPU-";

/// The 3×5 glyph for `c`, rows top to bottom, `#` for a pixel.
fn glyph(c: char) -> Option<[&'static str; 5]> {
    Some(match c {
        '0' => ["###", "#.#", "#.#", "#.#", "###"],
        '1' => [".#.", "##.", ".#.", ".#.", "###"],
        '2' => ["###", "..#", "###", "#..", "###"],
        '3' => ["###", "..#", "###", "..#", "###"],
        '4' => ["#.#", "#.#", "###", "..#", "..#"],
        '5' => ["###", "#..", "###", "..#", "###"],
        '6' => ["###", "#..", "###", "#.#", "###"],
        '7' => ["###", "..#", "..#", "..#", "..#"],
        '8' => ["###", "#.#", "###", "#.#", "###"],
        '9' => ["###", "#.#", "###", "..#", "###"],
        'A' => ["###", "#.#", "###", "#.#", "#.#"],
        'C' => ["###", "#..", "#..", "#..", "###"],
        'E' => ["###", "#..", "###", "#..", "###"],
        'F' => ["###", "#..", "###", "#..", "#.."],
        'H' => ["#.#", "#.#", "###", "#.#", "#.#"],
        'L' => ["#..", "#..", "#..", "#..", "###"],
        'P' => ["###", "#.#", "###", "#..", "#.."],
        'U' => ["#.#", "#.#", "#.#", "#.#", "###"],
        '-' => ["...", "...", "###", "...", "..."],
        _ => return None,
    })
}

/// The characters of `label` the font can draw (others dropped), upper-cased.
pub fn engravable(label: &str) -> String {
    label
        .chars()
        .map(|c| c.to_ascii_uppercase())
        .filter(|&c| glyph(c).is_some())
        .collect()
}

/// The text's pixels as `(column, row)` with row 0 at the **bottom**, mirrored left to right
/// (the first character at the right), so it reads correctly from below. Characters are 3
/// pixels wide with one pixel between them.
fn pixels(text: &str) -> (BTreeSet<(i32, i32)>, i32) {
    let n = text.chars().count() as i32;
    let width = (4 * n - 1).max(0);
    let mut out = BTreeSet::new();
    for (k, c) in text.chars().enumerate() {
        let Some(g) = glyph(c) else { continue };
        let x0 = 4 * k as i32;
        for (r, row) in g.iter().enumerate() {
            for (col, ch) in row.chars().enumerate() {
                if ch == '#' {
                    let x = x0 + col as i32;
                    out.insert((width - 1 - x, 4 - r as i32));
                }
            }
        }
    }
    (out, width)
}

/// Where a label can't go, in the part's plane (mm).
pub(crate) struct Keepout<'a> {
    /// The hollow's outline loops (material outside: an outer loop and any holes), mm.
    pub hollow: &'a [Vec<P2>],
    /// Tube centres and outer radius.
    pub tubes: &'a [P2],
    pub tube_radius: f64,
    /// Rectangles `(lo, hi)` standing on the ceiling: centre ribs and wall ribs.
    pub rects: &'a [(P2, P2)],
    /// Outlines cut into the top from above (pockets and slots).
    pub cuts: &'a [Loop],
}

/// An engraving: closed outlines in the part's plane (mm) and their total area (mm²).
#[derive(Clone, Debug)]
pub(crate) struct Engraving {
    pub loops: Vec<Vec<P2>>,
    pub area: f64,
    /// The characters engraved.
    pub text: String,
}

/// Even-odd point-in-polygon over a set of loops.
fn inside(loops: &[Vec<P2>], q: P2) -> bool {
    let mut c = false;
    for l in loops {
        let n = l.len();
        for k in 0..n {
            let (a, b) = (l[k], l[(k + 1) % n]);
            if (a[1] > q[1]) != (b[1] > q[1]) {
                let x = a[0] + (q[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
                if x > q[0] {
                    c = !c;
                }
            }
        }
    }
    c
}

/// Places and draws `label` (see the module docs), or `None` when nothing of it can be drawn
/// or no spot fits.
pub(crate) fn engrave(label: &str, k: &Keepout<'_>) -> Option<Engraving> {
    let text = engravable(label);
    if text.is_empty() {
        return None;
    }
    let (px, width) = pixels(&text);
    // A raster of free ceiling at RES mm.
    const RES: f64 = 0.1;
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for q in k.hollow.iter().flatten() {
        for a in 0..2 {
            lo[a] = lo[a].min(q[a]);
            hi[a] = hi[a].max(q[a]);
        }
    }
    if lo[0] >= hi[0] {
        return None;
    }
    let nx = ((hi[0] - lo[0]) / RES).ceil() as usize;
    let ny = ((hi[1] - lo[1]) / RES).ceil() as usize;
    let at = |i: usize, j: usize| [lo[0] + (i as f64 + 0.5) * RES, lo[1] + (j as f64 + 0.5) * RES];
    // Prefix sums of blocked cells.
    let mut sum = vec![0u32; (nx + 1) * (ny + 1)];
    let idx = |i: usize, j: usize| j * (nx + 1) + i;
    for j in 0..ny {
        for i in 0..nx {
            let q = at(i, j);
            let blocked = !inside(k.hollow, q)
                || k.tubes.iter().any(|t| (q[0] - t[0]).hypot(q[1] - t[1]) < k.tube_radius)
                || k.rects.iter().any(|(a, b)| {
                    q[0] >= a[0].min(b[0])
                        && q[0] <= a[0].max(b[0])
                        && q[1] >= a[1].min(b[1])
                        && q[1] <= a[1].max(b[1])
                })
                || k.cuts.iter().any(|l| geom::contains(l, q));
            sum[idx(i + 1, j + 1)] =
                u32::from(blocked) + sum[idx(i, j + 1)] + sum[idx(i + 1, j)] - sum[idx(i, j)];
        }
    }
    // Is the box [x0, x1] × [y0, y1] (mm) free?
    let free = |x0: f64, y0: f64, x1: f64, y1: f64| -> bool {
        let fi = |x: f64| ((x - lo[0]) / RES).floor();
        let fj = |y: f64| ((y - lo[1]) / RES).floor();
        let (i0, j0, i1, j1) = (fi(x0), fj(y0), fi(x1) + 1.0, fj(y1) + 1.0);
        if i0 < 0.0 || j0 < 0.0 || i1 > nx as f64 || j1 > ny as f64 {
            return false;
        }
        let (i0, j0, i1, j1) = (i0 as usize, j0 as usize, i1 as usize, j1 as usize);
        sum[idx(i1, j1)] + sum[idx(i0, j0)] - sum[idx(i0, j1)] - sum[idx(i1, j0)] == 0
    };
    // Candidate centres, nearest the hollow's middle first.
    let mid = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0];
    let step = 0.25;
    let mut centres = Vec::new();
    let (cx, cy) = (((hi[0] - lo[0]) / step) as i32, ((hi[1] - lo[1]) / step) as i32);
    for i in 0..=cx {
        for j in 0..=cy {
            centres.push([lo[0] + f64::from(i) * step, lo[1] + f64::from(j) * step]);
        }
    }
    centres.sort_by(|a, b| {
        let d = |q: &P2| (q[0] - mid[0]).hypot(q[1] - mid[1]);
        d(a).total_cmp(&d(b))
    });
    let m = LABEL_MARGIN;
    for p in PIXEL_SIZES {
        let (w, h) = (f64::from(width) * p, 5.0 * p);
        for c in &centres {
            // Reading along X, then along Y.
            for along_x in [true, false] {
                let (hw, hh) = if along_x { (w / 2.0, h / 2.0) } else { (h / 2.0, w / 2.0) };
                if !free(c[0] - hw - m, c[1] - hh - m, c[0] + hw + m, c[1] + hh + m) {
                    continue;
                }
                // Pixel (col, row) → plane: along X, columns run +X and rows +Y; along Y
                // (a quarter turn counter-clockwise) columns run +Y and rows −X.
                let map = |u: f64, v: f64| -> P2 {
                    let (u, v) = (u * p - w / 2.0, v * p - h / 2.0);
                    if along_x { [c[0] + u, c[1] + v] } else { [c[0] - v, c[1] + u] }
                };
                let loops: Vec<Vec<P2>> = trace_loops(&px)
                    .ok()?
                    .into_iter()
                    .map(|l| l.into_iter().map(|(a, b)| map(f64::from(a), f64::from(b))).collect())
                    .collect();
                return Some(Engraving {
                    loops,
                    area: px.len() as f64 * p * p,
                    text,
                });
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_glyph_has_corner_only_contacts() {
        for c in LABEL_CHARS.chars() {
            let g = glyph(c).unwrap();
            let on = |x: i32, y: i32| {
                (0..3).contains(&x)
                    && (0..5).contains(&y)
                    && g[y as usize].as_bytes()[x as usize] == b'#'
            };
            for y in -1..5 {
                for x in -1..3 {
                    let (a, b, cc, d) = (on(x, y), on(x + 1, y), on(x, y + 1), on(x + 1, y + 1));
                    assert!(!(a && d && !b && !cc) && !(b && cc && !a && !d), "{c} at {x},{y}");
                }
            }
        }
    }

    #[test]
    fn text_is_mirrored_and_unknown_letters_dropped() {
        assert_eq!(engravable("B017"), "017");
        let (px, w) = pixels("1-");
        assert_eq!(w, 7);
        // '1' is first, so after mirroring it is on the right: its foot row spans x 4..=6.
        assert!(px.contains(&(4, 0)) && px.contains(&(6, 0)));
        // The dash's middle row is at the left.
        assert!(px.contains(&(0, 2)) && px.contains(&(2, 2)));
    }

    #[test]
    fn a_label_avoids_a_tube_and_fits_a_cell() {
        let hollow = vec![vec![[1.3, 1.3], [14.7, 1.3], [14.7, 30.7], [1.3, 30.7]]];
        let tubes = [[8.0, 8.0], [8.0, 16.0], [8.0, 24.0]];
        let k = Keepout {
            hollow: &hollow,
            tubes: &tubes,
            tube_radius: 3.255,
            rects: &[],
            cuts: &[],
        };
        let e = engrave("B017", &k).expect("fits beside the tubes");
        assert_eq!(e.text, "017");
        for q in e.loops.iter().flatten() {
            for t in &tubes {
                assert!((q[0] - t[0]).hypot(q[1] - t[1]) >= 3.255 + LABEL_MARGIN - 1e-9);
            }
        }
        let shoelace: f64 = e.loops.iter().map(|l| crate::shape::signed_area(l)).sum();
        assert!((shoelace.abs() - e.area).abs() < 1e-9, "{shoelace} vs {}", e.area);
    }
}
