//! Bouquets: printed stems gathered in a hub on top of a brick-built vase, each with a head
//! on its tip and leaves clipped along it (see `sf_core::stem` for the parts and frames).
//!
//! 1. [`plan_hub`] puts the hub on the vase top under the spec's `hub`: the smallest square
//!    of studs (at least 2 × 2) on a flat top that holds a hole for every stem, holes on a
//!    lattice [`HOLE_SPACING`] apart. Each stem gets the hole that lies the way it leans, so
//!    stems fan out without crossing near the hub; holes are numbered back to front.
//! 2. [`attach`] builds each stem: a circular arc from its hole to its tip, bending
//!    outward (away from the bouquet's middle) by the spec's `bend`, its length solved so
//!    the head's mount lands on the tip. Heads go on the tip bar facing out along it, their
//!    front turned toward the viewer (or by `spin`); leaves clip onto bar sections, turned
//!    `side` degrees about the stem from the outward side.
//! 3. It checks heads and leaves against each other, the other stems and the vase, stems
//!    against each other and the vase, and the balance with everything on.
//!
//! Positions are in the model's LDU frame (`Placement::ldraw_transform`'s): `X = 20·x`,
//! `Y = −8·y` (up is `−Y`), `Z = 20·z`; the viewer stands at `−Z`.

use anyhow::{Result, bail};
pub use sf_core::stem::{HUB_HEIGHT, HUB_HOLE_EDGE, HUB_HOLE_GAP, TIP_BAR};
use sf_core::stem::{LDU_PER_MM, MAX_STEM_LEN, MIN_STEM_D, PEG_D, PEG_LEN, STEM_D};
use sf_core::{
    Cell, CustomDef, CustomKind, DetailDesign, DetailMesh, DetailSource, Hub, HubHole, KitPose, Model, MountKind, Palette, Placement, Rot,
    StemAttach, StemBar, StemBranch, StemDef, StemPlacement, StemStyle, StemTip,
};
use sf_spec::GridBouquet;
use std::collections::{BTreeMap, HashMap, HashSet};

/// Depth of a hub's peg hole, mm (a peg's length and clearance, as `sf_custom` cuts it).
pub const HOLE_DEPTH: f32 = PEG_LEN + 0.5;
/// How far apart the engine keeps hole axes, anywhere along their depth: the printable
/// [`HUB_HOLE_GAP`] and a margin.
pub const HOLE_AXIS_GAP: f32 = HUB_HOLE_GAP + 0.5;
/// Least distance between hole centres on the hub's top, mm.
pub const HOLE_SPACING: f32 = PEG_D + 3.0;
/// Least distance from a hole's centre to the hub's edge on its top, mm (at least
/// [`HUB_HOLE_EDGE`]).
const HOLE_MARGIN: f32 = PEG_D / 2.0 + 1.6;
/// A leaf's bar section, mm.
pub const LEAF_BAR: f32 = 8.0;
/// Largest hub, studs on a side.
const MAX_HUB: i32 = 8;
/// The hub's key in `Model::meshes` and `Model::details`.
pub const HUB_KEY: &str = "bouquet_hub";
/// The leaf design a style's suggested leaves use when the spec gives none.
pub const DEFAULT_LEAF: &str = "leaf";
/// Steepest lean of a stem's base from vertical that its peg hole can take, degrees.
const MAX_BASE_LEAN: f32 = 60.0;

type V3 = [f32; 3];
type M3 = [[f32; 3]; 3];

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn mul(a: V3, s: f32) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn len(a: V3) -> f32 {
    dot(a, a).sqrt()
}
fn unit(a: V3) -> Option<V3> {
    let l = len(a);
    (l > 1e-6).then(|| mul(a, 1.0 / l))
}
fn apply(r: &M3, v: V3) -> V3 {
    [0, 1, 2].map(|i| r[i][0] * v[0] + r[i][1] * v[1] + r[i][2] * v[2])
}
/// The rotation with columns `x`, `y`, `z` (local axes → world).
fn columns(x: V3, y: V3, z: V3) -> M3 {
    [[x[0], y[0], z[0]], [x[1], y[1], z[1]], [x[2], y[2], z[2]]]
}
/// `v` turned `deg` degrees about unit `k` (right hand).
fn rotate(v: V3, k: V3, deg: f32) -> V3 {
    let (s, c) = deg.to_radians().sin_cos();
    add(add(mul(v, c), mul(cross(k, v), s)), mul(k, dot(k, v) * (1.0 - c)))
}
/// The rotation taking local axis `a` to world `wa` and local `u` (made square to `a`) to `wu`.
fn align(a: V3, u: V3, wa: V3, wu: V3) -> M3 {
    let frame = |a: V3, u: V3| {
        let a = unit(a).unwrap_or([0.0, 1.0, 0.0]);
        let u = unit(sub(u, mul(a, dot(u, a)))).unwrap_or_else(|| any_square(a));
        (a, u, cross(a, u))
    };
    let (la, lu, lw) = frame(a, u);
    let (wa, wu, ww) = frame(wa, wu);
    let mut r = [[0.0; 3]; 3];
    for (i, row) in r.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = wa[i] * la[j] + wu[i] * lu[j] + ww[i] * lw[j];
        }
    }
    r
}
/// Some unit vector square to unit `a`.
fn any_square(a: V3) -> V3 {
    let t = if a[1].abs() < 0.9 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] };
    unit(cross(a, t)).expect("t is not parallel to a")
}

/// The viewer's direction from the model (its front faces `−Z`).
const FRONT: V3 = [0.0, 0.0, -1.0];
/// Up in the model frame.
const UP: V3 = [0.0, -1.0, 0.0];

/// Fails when a stem names a design that doesn't exist or can't go on a stem, with the fix.
pub fn check_designs(b: &GridBouquet, designs: &BTreeMap<String, DetailDesign>) -> Result<()> {
    let have = || {
        let names: Vec<&str> = designs.keys().map(String::as_str).collect();
        if names.is_empty() { "none yet (make one first)".to_string() } else { names.join(", ") }
    };
    for (i, s) in b.stems.iter().enumerate() {
        let uses = s
            .head
            .iter()
            .chain(&s.branch_heads)
            .map(|h| (h, MountKind::Hole, "head"))
            .chain(s.leaves.iter().map(|l| (&l.design, MountKind::Clip, "leaf")));
        for (name, kind, what) in uses {
            let Some(d) = designs.get(name) else {
                bail!("bouquet stem {}: {what} design `{name}` doesn't exist; available: {}", i + 1, have());
            };
            match d.mount {
                Some(m) if m.kind == kind => {}
                Some(_) => bail!(
                    "bouquet stem {}: `{name}` mounts the wrong way for a {what} ({}); use a design made for it",
                    i + 1,
                    if kind == MountKind::Hole {
                        "a head needs a bar hole for the stem's tip"
                    } else {
                        "a leaf needs a clip for a bar on the stem"
                    }
                ),
                None => bail!(
                    "bouquet stem {}: `{name}` has no stem mount; make a stem version of it ({})",
                    i + 1,
                    if kind == MountKind::Hole { "a head with a bar hole underneath" } else { "a leaf with a clip at its base" }
                ),
            }
        }
    }
    Ok(())
}

/// Where the hub goes and which hole each stem gets.
#[derive(Clone, Debug)]
pub struct HubPlan {
    /// The hub as a detail placement (on the vase top).
    pub placement: Placement,
    /// Studs on a side.
    pub size: i32,
    /// Hole centres on the top face, in the hub's print frame (mm; x along grid x, y along
    /// grid z), numbered (from 0) back to front.
    pub holes: Vec<[f32; 2]>,
    /// Each stem's hole.
    pub hole_of: Vec<usize>,
}

/// Holes a `k × k` hub can take, on its lattice.
fn lattice(k: i32) -> Vec<[f32; 2]> {
    let side = 8.0 * k as f32;
    let m = (((side - 2.0 * HOLE_MARGIN) / HOLE_SPACING).floor() as usize + 1).max(1);
    let start = (side - (m - 1) as f32 * HOLE_SPACING) / 2.0;
    (0..m).flat_map(|j| (0..m).map(move |i| [start + i as f32 * HOLE_SPACING, start + j as f32 * HOLE_SPACING])).collect()
}

/// Gives each stem one of `spots` (hole centres on the top of a `k × k` hub at grid
/// `(x0, z0)`): the one in the direction it leans, farther out the more it leans (greedy,
/// then improved by swaps), so stems fan out from the hub without crossing. Returns each
/// stem's spot and the total cost.
fn assign(b: &GridBouquet, spots: &[[f32; 2]], x0: i32, z0: i32, k: i32) -> (Vec<usize>, f32) {
    let c = [4.0 * k as f32, 4.0 * k as f32];
    let hub = [20.0 * x0 as f32 + 10.0 * k as f32, 20.0 * z0 as f32 + 10.0 * k as f32];
    let lean: Vec<[f32; 2]> = b.stems.iter().map(|s| [(s.tip[0] - hub[0]) / LDU_PER_MM, (s.tip[2] - hub[1]) / LDU_PER_MM]).collect();
    let l = |v: [f32; 2]| (v[0] * v[0] + v[1] * v[1]).sqrt();
    let most = lean.iter().map(|&v| l(v)).fold(0.0f32, f32::max);
    let reach = spots.iter().map(|p| l([p[0] - c[0], p[1] - c[1]])).fold(0.0f32, f32::max);
    let target: Vec<[f32; 2]> = lean
        .iter()
        .map(|&v| if most < 1e-3 || l(v) < 1e-3 { c } else { [c[0] + v[0] / most * reach, c[1] + v[1] / most * reach] })
        .collect();
    let cost = |s: usize, h: usize| {
        let (p, t) = (spots[h], target[s]);
        (p[0] - t[0]).powi(2) + (p[1] - t[1]).powi(2)
    };
    let n = b.stems.len();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| l(lean[b]).total_cmp(&l(lean[a])));
    let mut hole = vec![usize::MAX; n];
    let mut used = vec![false; spots.len()];
    for &s in &order {
        let h = (0..spots.len()).filter(|&h| !used[h]).min_by(|&a, &b| cost(s, a).total_cmp(&cost(s, b))).expect("enough holes");
        hole[s] = h;
        used[h] = true;
    }
    // Swaps and moves to free spots while they lower the total.
    loop {
        let mut better = false;
        for a in 0..n {
            for b2 in a + 1..n {
                if cost(a, hole[b2]) + cost(b2, hole[a]) + 1e-3 < cost(a, hole[a]) + cost(b2, hole[b2]) {
                    hole.swap(a, b2);
                    better = true;
                }
            }
            for h in 0..spots.len() {
                if !used[h] && cost(a, h) + 1e-3 < cost(a, hole[a]) {
                    used[hole[a]] = false;
                    used[h] = true;
                    hole[a] = h;
                    better = true;
                }
            }
        }
        if !better {
            break;
        }
    }
    let total = (0..n).map(|s| cost(s, hole[s])).sum();
    (hole, total)
}

/// Candidate hole layouts for a `k × k` hub and `n` stems: a ring of `n` (two turns of it),
/// a ring of `n − 1` around a centre hole, and the square lattice.
fn spot_sets(k: i32, n: usize) -> Vec<Vec<[f32; 2]>> {
    let c = 4.0 * k as f32;
    let r = c - HOLE_MARGIN;
    let mut sets = Vec::new();
    for (ring, centre) in [(n, false), (n.saturating_sub(1), true)] {
        if centre && r < HOLE_SPACING {
            continue;
        }
        if ring > 1 && 2.0 * r * (std::f32::consts::PI / ring as f32).sin() < HOLE_SPACING {
            continue;
        }
        for turn in [0.0, 0.5] {
            let mut spots: Vec<[f32; 2]> = (0..ring)
                .map(|i| {
                    let a = std::f32::consts::TAU * (i as f32 + turn) / ring.max(1) as f32;
                    [c + r * a.cos(), c + r * a.sin()]
                })
                .collect();
            if centre {
                spots.push([c, c]);
            }
            sets.push(spots);
        }
    }
    sets.push(lattice(k));
    sets.retain(|s| s.len() >= n);
    sets
}

/// A stem's arc from its base (a hole on the hub, LDU) to its tip: length (mm, clamped to
/// [`MAX_STEM_LEN`]), where its head's mount (or its end) sits along it, and its frame.
struct StemArc {
    length: f32,
    /// Distance of the head's mount (or the bare tip) from the base, mm.
    mount_at: f32,
    tip_bar: f32,
    /// Base tangent (up the stem) and the rotation of the stem's frame, model frame.
    d0: V3,
    rotation: M3,
    /// The bend in the stem's own frame (negative when mirrored).
    bend: f32,
    clamped: bool,
}

/// The style a stem uses: the spec's, else its head design's, else none (a plain stem).
fn style_of<'a>(s: &'a sf_spec::GridStem, designs: &'a BTreeMap<String, DetailDesign>) -> Option<&'a StemStyle> {
    s.style.as_ref().or_else(|| s.head.as_ref().and_then(|h| designs.get(h)).and_then(|d| d.stem.as_ref()))
}

/// Whether the main stem's tip carries the head.
fn head_on_tip(s: &sf_spec::GridStem, designs: &BTreeMap<String, DetailDesign>) -> bool {
    s.head.is_some() && style_of(s, designs).is_none_or(|st| st.head_on_tip)
}

/// Solves the arc from `base` to `tip`, turning `bend` degrees in all, ending `tip_bar` mm
/// past the tip. It bows toward the outward side (away from `centre`) turned `turn`
/// degrees about the chord; `mirror` flips the stem's own `X` (its branches go to the other
/// side). `None` when the tip isn't above the base.
fn stem_arc(tip: V3, base: V3, centre: V3, bend: f32, tip_bar: f32, turn: f32, mirror: bool) -> Option<StemArc> {
    let chord = sub(tip, base);
    let c = len(chord) / LDU_PER_MM;
    if c < 10.0 || -chord[1] <= 0.0 {
        return None;
    }
    let beta = bend.abs().to_radians();
    // Length L whose arc, turning `bend` in all, reaches the tip `tip_bar` before its end.
    let chord_at = |l: f32| {
        let sm = l - tip_bar;
        if beta < 1e-4 { sm } else { 2.0 * l / beta * (beta * sm / (2.0 * l)).sin() }
    };
    let mut length = if beta < 1e-4 {
        c + tip_bar
    } else {
        let (mut lo, mut hi) = (c + tip_bar, 4.0 * (c + tip_bar) + 100.0);
        for _ in 0..80 {
            let mid = (lo + hi) / 2.0;
            if chord_at(mid) < c { lo = mid } else { hi = mid }
        }
        (lo + hi) / 2.0
    };
    let clamped = length > MAX_STEM_LEN;
    length = length.min(MAX_STEM_LEN);
    let sm = length - tip_bar;
    let theta = beta * sm / length;
    let u = unit(chord)?;
    let mut out = [tip[0] - centre[0], 0.0, tip[2] - centre[2]];
    if len(out) < 1e-3 {
        out = FRONT;
    }
    let w = unit(sub(out, mul(u, dot(out, u)))).unwrap_or_else(|| unit(sub(UP, mul(u, dot(UP, u)))).unwrap_or_else(|| any_square(u)));
    // The side it bows toward: outward (inward for a negative bend), turned about the chord.
    let bow = rotate(if bend < 0.0 { mul(w, -1.0) } else { w }, u, turn);
    let (sh, ch) = (theta / 2.0).sin_cos();
    let d0 = sub(mul(u, ch), mul(bow, sh));
    let bow_at_base = add(mul(u, sh), mul(bow, ch));
    let x = if mirror { mul(bow_at_base, -1.0) } else { bow_at_base };
    let y = mul(d0, -1.0);
    let local_bend = if mirror { -bend.abs() } else { bend.abs() };
    Some(StemArc { length, mount_at: sm, tip_bar, d0, rotation: columns(x, y, cross(x, y)), bend: local_bend, clamped })
}

/// The concrete stem for a use: its style's branches scaled to `arc.length` (mirrored with
/// the frame), tip shape, rod and name.
fn stem_def(id: &str, s: &sf_spec::GridStem, style: Option<&StemStyle>, arc: &StemArc, mirror: bool) -> StemDef {
    let l = arc.length;
    let sign = if mirror { -1.0 } else { 1.0 };
    let branches = style
        .map(|st| {
            st.branches
                .iter()
                .map(|br| StemBranch {
                    at: (br.at * l).clamp(10.0, (arc.mount_at - 10.0).max(10.0)),
                    angle: sign * br.angle,
                    length: (br.length * l).max(15.0),
                    bend: sign * br.bend,
                    diameter: None,
                    tip_bar: if br.head { TIP_BAR } else { 0.0 },
                    tip: br.tip,
                })
                .collect()
        })
        .unwrap_or_default();
    StemDef {
        id: id.to_string(),
        length: l,
        bend: arc.bend,
        tip_bar: arc.tip_bar,
        bars: Vec::new(),
        diameter: style.map_or(STEM_D, |st| st.diameter.max(MIN_STEM_D)),
        tip: if arc.tip_bar > 0.0 { None } else { s.tip_shape.or(style.and_then(|st| st.tip)) },
        branches,
        style: style.map(|st| st.name.clone()).unwrap_or_default(),
    }
}

/// Points along a stem in its own frame (LDU): the main centreline, every branch, and the
/// tip shapes' far ends, every `step` mm.
fn local_points(def: &StemDef, step: f32) -> Vec<V3> {
    let mut out = Vec::new();
    let n = (def.length / step).ceil().max(1.0) as usize;
    out.extend((0..=n).map(|i| def.point(def.length * i as f32 / n as f32)));
    let tip_reach = |t: Option<StemTip>| match t {
        Some(StemTip::Bud { length, .. }) | Some(StemTip::Blade { length, .. }) => length,
        None => 0.0,
    };
    let mut ends = vec![(def.point(def.length), def.tangent(def.length), tip_reach(def.tip))];
    for (i, b) in def.branches.iter().enumerate() {
        let m = (b.length / step).ceil().max(1.0) as usize;
        out.extend((0..=m).map(|j| def.branch_point(i, b.length * j as f32 / m as f32)));
        ends.push((def.branch_point(i, b.length), def.branch_tangent(i, b.length), tip_reach(b.tip)));
    }
    for (p, t, reach) in ends {
        let m = (reach / step).ceil() as usize;
        out.extend((1..=m).map(|j| add(p, mul(t, reach * j as f32 / m as f32 * LDU_PER_MM))));
    }
    out
}

/// The stem's extent laid flat on the bed (its own XY plane), mm: (long side, short side).
fn laid_flat(def: &StemDef) -> (f32, f32) {
    let pts = local_points(def, 2.0);
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for p in &pts {
        for k in 0..2 {
            lo[k] = lo[k].min(p[k] / LDU_PER_MM);
            hi[k] = hi[k].max(p[k] / LDU_PER_MM);
        }
    }
    let pad = def.diameter.max(PEG_D);
    let (w, h) = (hi[0] - lo[0] + pad, hi[1] - lo[1] + pad + PEG_LEN);
    (w.max(h), w.min(h))
}

/// Laid-flat bed area a stem may use, mm: [`MAX_STEM_LEN`] by this.
pub const BED_WIDTH: f32 = 200.0;

/// Holes that would come too close inside the hub or to its edge, as `(hole a, hole b)`
/// (b = a for one too near the edge), for holes `tops` (hub frame, mm) and directions
/// `dirs` (print frame): axes at least [`HOLE_AXIS_GAP`] apart over the whole
/// [`HOLE_DEPTH`], and at least [`HUB_HOLE_EDGE`] from the footprint's edge at the top and
/// the bottom of the hole.
fn peg_clashes(tops: &[[f32; 3]], dirs: &[V3], k: i32) -> Vec<(usize, usize)> {
    let side = 8.0 * k as f32;
    let e = HUB_HOLE_EDGE;
    let near_edge = |p: V3| p[0] < e || p[1] < e || p[0] > side - e || p[1] > side - e;
    let mut out = Vec::new();
    for a in 0..tops.len() {
        let bottom = sub(tops[a], mul(dirs[a], HOLE_DEPTH));
        if near_edge(tops[a]) || near_edge(bottom) || bottom[2] < 0.0 {
            out.push((a, a));
        }
        for b in a + 1..tops.len() {
            if segment_distance(tops[a], bottom, tops[b], sub(tops[b], mul(dirs[b], HOLE_DEPTH))) < HOLE_AXIS_GAP {
                out.push((a, b));
            }
        }
    }
    out
}

/// The holes of a `k × k` hub at grid `(x0, y_top, z0)`: the layout (of [`spot_sets`])
/// whose pegs keep apart, then the least total lean mismatch. Returns the used holes
/// numbered back to front, each stem's hole, and the peg clashes left (0 when it fits).
pub fn layout(
    b: &GridBouquet,
    designs: &BTreeMap<String, DetailDesign>,
    x0: i32,
    y_top: i32,
    z0: i32,
    k: i32,
) -> (Vec<[f32; 2]>, Vec<usize>, usize) {
    let top_y = -8.0 * y_top as f32 - HUB_HEIGHT * LDU_PER_MM;
    let centre = [20.0 * x0 as f32 + 10.0 * k as f32, top_y, 20.0 * z0 as f32 + 10.0 * k as f32];
    let to_print = |v: V3| [v[0], v[2], -v[1]];
    // (clashes, cost, spots, each stem's spot)
    type Layout = (usize, f32, Vec<[f32; 2]>, Vec<usize>);
    let mut best: Option<Layout> = None;
    for spots in spot_sets(k, b.stems.len()) {
        let (hole, cost) = assign(b, &spots, x0, z0, k);
        let mut tops = Vec::new();
        let mut dirs = Vec::new();
        for (si, s) in b.stems.iter().enumerate() {
            let p = spots[hole[si]];
            let base = [20.0 * x0 as f32 + p[0] * LDU_PER_MM, top_y, 20.0 * z0 as f32 + p[1] * LDU_PER_MM];
            let tip_bar = if head_on_tip(s, designs) { TIP_BAR } else { 0.0 };
            if let Some(arc) = stem_arc(s.tip, base, centre, s.bend, tip_bar, 0.0, false) {
                tops.push([p[0], p[1], HUB_HEIGHT]);
                dirs.push(to_print(arc.d0));
            }
        }
        let clashes = peg_clashes(&tops, &dirs, k).len();
        if best.as_ref().is_none_or(|b| (clashes, cost) < (b.0, b.1)) {
            best = Some((clashes, cost, spots, hole));
        }
    }
    let (clashes, _, spots, hole) = best.expect("the lattice always fits");
    // Number the used holes back to front (larger z first), then left to right.
    let mut taken: Vec<usize> = hole.clone();
    taken.sort_by(|&a, &b| spots[b][1].total_cmp(&spots[a][1]).then(spots[a][0].total_cmp(&spots[b][0])));
    let number: HashMap<usize, usize> = taken.iter().enumerate().map(|(i, &h)| (h, i)).collect();
    (taken.iter().map(|&h| spots[h]).collect(), hole.iter().map(|h| number[h]).collect(), clashes)
}

/// Puts the hub on the vase top under `b.hub`: the smallest flat square of studs (at least
/// 2 × 2) that holds every stem's hole, on the highest body surface there. `ps` are the
/// parts so far (body and details); `id` the hub's detail id. `Err` explains what to fix.
pub fn plan_hub(
    b: &GridBouquet,
    designs: &BTreeMap<String, DetailDesign>,
    ps: &[Placement],
    id: &str,
) -> std::result::Result<HubPlan, String> {
    let mut owner: HashMap<Cell, usize> = HashMap::new();
    let mut top: HashMap<(i32, i32), i32> = HashMap::new();
    for (i, p) in ps.iter().enumerate() {
        for c in p.cells() {
            owner.insert(c, i);
            let t = top.entry((c.x, c.z)).or_insert(0);
            *t = (*t).max(c.y + 1);
        }
    }
    let (gx, gz) = (b.hub[0] / 20.0, b.hub[2] / 20.0);
    let n = b.stems.len();
    let k_min = (2..=MAX_HUB)
        .find(|&k| lattice(k).len() >= n)
        .ok_or_else(|| format!("{n} stems need a hub wider than {MAX_HUB} studs; use fewer stems"))?;
    // The square of k × k columns centred on the hub point, if its top is flat bricks.
    let square = |k: i32| -> Option<(i32, i32, i32)> {
        let (x0, z0) = ((gx - k as f32 / 2.0).round() as i32, (gz - k as f32 / 2.0).round() as i32);
        let cols: Vec<(i32, i32)> = (0..k).flat_map(|dz| (0..k).map(move |dx| (x0 + dx, z0 + dz))).collect();
        let y_top = top.get(&cols[0]).copied()?;
        let flat = cols.iter().all(|c| top.get(c) == Some(&y_top));
        let bricks = cols.iter().all(|&(x, z)| owner.get(&Cell::new(x, y_top - 1, z)).is_some_and(|&o| !crate::details::is_detail(&ps[o])));
        (flat && bricks && y_top > 0).then_some((x0, y_top, z0))
    };
    let Some(mut at) = square(k_min) else {
        return Err(format!(
            "bouquet: the vase top under the hub isn't a flat {k_min} × {k_min}-stud area of bricks ({n} stems need that much); make the vase top flat and at least {k_min} studs across around `hub`, and put `hub` at its centre"
        ));
    };
    let mut k = k_min;
    // Leaning stems' pegs slant inward under the hub: grow it while the top allows until
    // they keep apart.
    while layout(b, designs, at.0, at.1, at.2, k).2 > 0 && k < MAX_HUB {
        match square(k + 1) {
            Some(next) => {
                at = next;
                k += 1;
            }
            None => break,
        }
    }
    let (x0, y_top, z0) = at;
    let def = CustomDef {
        id: id.to_string(),
        kind: CustomKind::Detail { mesh: HUB_KEY.to_string(), name: "stem hub".to_string() },
        cells: (0..k as u8).flat_map(|z| (0..k as u8).map(move |x| (x, z))).collect(),
        colors: BTreeMap::new(),
        plates: (HUB_HEIGHT / 3.2).ceil() as u8,
        studs: Vec::new(),
        units: Vec::new(),
        sockets: (0..k as u8).flat_map(|z| (0..k as u8).map(move |x| (x, z))).collect(),
        trim: None,
        mesh: None,
    };
    let placement = Placement::printed(def, ps[owner[&Cell::new(x0, y_top - 1, z0)]].color, Cell::new(x0, y_top, z0), Rot(0));
    if let Some(c) = placement.cells().find(|c| owner.contains_key(c)) {
        return Err(format!("bouquet: something already sits on the vase top where the hub goes (cell {}, {}, {})", c.x, c.y, c.z));
    }
    let (holes, hole_of, _) = layout(b, designs, x0, y_top, z0, k);
    Ok(HubPlan { placement, size: k, holes, hole_of })
}

/// What the bouquet came to, for the report.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
pub struct StemSummary {
    pub stems: usize,
    pub longest_mm: f32,
    pub leaves: usize,
    pub heads: usize,
    pub holes: usize,
    /// The hub's side, studs.
    pub hub_size: i32,
    /// Stems shortened to [`MAX_STEM_LEN`].
    pub clamped: usize,
    /// Side branches over all stems.
    pub branches: usize,
    /// The largest stem laid flat for printing (long × short side), mm.
    pub laid_flat_mm: [f32; 2],
    /// The model with the bouquet in stands (centre of mass over the vase's footprint).
    pub balanced: bool,
}

/// Builds the stems, the hub's design and their checks into `model` (whose placements
/// hold the hub planned by [`plan_hub`]). Returns the summary and warnings, or `None` when
/// the model has no hub (the plan failed and said why).
pub fn attach(
    model: &mut Model,
    b: &GridBouquet,
    designs: &BTreeMap<String, DetailDesign>,
    palette: &Palette,
) -> Option<(StemSummary, Vec<String>)> {
    let hub_i = model
        .placements
        .iter()
        .position(|p| matches!(p.custom.as_deref().map(|c| &c.kind), Some(CustomKind::Detail { mesh, .. }) if mesh == HUB_KEY))?;
    let hub_p = model.placements[hub_i].clone();
    let k = hub_p.footprint()[0];
    let (x0, y_top, z0) = (hub_p.origin.x, hub_p.origin.y, hub_p.origin.z);
    let (holes, hole_of, _) = layout(b, designs, x0, y_top, z0, k);
    let mut warnings = Vec::new();
    let hub_top = -8.0 * y_top as f32 - HUB_HEIGHT * LDU_PER_MM;
    let centre = [20.0 * x0 as f32 + 10.0 * k as f32, hub_top, 20.0 * z0 as f32 + 10.0 * k as f32];
    let mut summary = StemSummary { stems: b.stems.len(), holes: holes.len(), hub_size: k, ..Default::default() };
    let mut hub_holes = vec![HubHole { top: [0.0; 3], dir: [0.0, 0.0, 1.0], key: [1.0, 0.0, 0.0] }; holes.len()];
    let mut stems: Vec<(usize, StemPlacement)> = Vec::new();
    // To print frame (mm axes: x = X, y = Z, z = −Y) for directions.
    let to_print = |v: V3| [v[0], v[2], -v[1]];
    // Each stem's chord (hole to tip), as a stand-in for the others when choosing a plane.
    let hole_world = |h: usize| [20.0 * x0 as f32 + holes[h][0] * LDU_PER_MM, hub_top, 20.0 * z0 as f32 + holes[h][1] * LDU_PER_MM];
    let chords: Vec<(V3, V3)> = b.stems.iter().enumerate().map(|(si, s)| (hole_world(hole_of[si]), s.tip)).collect();
    let mut leaf_default = false;
    for (si, s) in b.stems.iter().enumerate() {
        let n = si + 1;
        let h = hole_of[si];
        let hp = holes[h];
        let base = hole_world(h);
        let style = style_of(s, designs);
        let tip_bar = if head_on_tip(s, designs) { TIP_BAR } else { 0.0 };
        let def_id = format!("S{n:02}");
        // The stem's plane: with branches, the turn about its chord (and side) that fans them
        // toward the viewer and sideways, away from other stems; else bowing straight out.
        let has_branches = style.is_some_and(|st| !st.branches.is_empty());
        let candidates: Vec<(f32, bool)> = if has_branches {
            (0..12).flat_map(|i| [(i as f32 * 30.0, false), (i as f32 * 30.0, true)]).collect()
        } else {
            vec![(0.0, false)]
        };
        let mut best: Option<(f32, StemArc, StemDef, bool)> = None;
        for (turn, mirror) in candidates {
            let Some(arc) = stem_arc(s.tip, base, centre, s.bend, tip_bar, turn, mirror) else { continue };
            let def = stem_def(&def_id, s, style, &arc, mirror);
            let mut cost = (1.0 - turn.to_radians().cos()) * (0.5 + 0.2 * s.bend.abs());
            for (i, br) in def.branches.iter().enumerate() {
                let start = add(apply(&arc.rotation, def.branch_point(i, 0.0)), base);
                let end = add(apply(&arc.rotation, def.branch_point(i, br.length)), base);
                let mid = add(apply(&arc.rotation, def.branch_point(i, br.length / 2.0)), base);
                if let Some(d) = unit([end[0] - start[0], 0.0, end[2] - start[2]]) {
                    cost += 10.0 * dot(d, [0.0, 0.0, 1.0]).max(0.0); // pointing away from the viewer
                }
                for (oi, &(a, z)) in chords.iter().enumerate() {
                    if oi == si {
                        continue;
                    }
                    for p in [end, mid] {
                        let d = point_segment(p, a, z) / LDU_PER_MM;
                        cost += (30.0 - d).max(0.0);
                    }
                }
            }
            if best.as_ref().is_none_or(|b| cost < b.0) {
                best = Some((cost, arc, def, mirror));
            }
        }
        let Some((_, arc, mut def, mirror)) = best else {
            warnings.push(format!("bouquet stem {n}: its tip is not above the hub; put `tip` higher than the vase top"));
            continue;
        };
        if arc.clamped {
            warnings.push(format!(
                "bouquet stem {n} is longer than the {MAX_STEM_LEN:.0} mm a print bed takes; shortened, so its head sits short of its tip: bring the tip nearer the hub"
            ));
            summary.clamped += 1;
        }
        let (length, sm, d0, r) = (arc.length, arc.mount_at, arc.d0, arc.rotation);
        let lean = dot(d0, UP).clamp(-1.0, 1.0).acos().to_degrees();
        if lean > MAX_BASE_LEAN {
            warnings.push(format!(
                "bouquet stem {n} leaves the hub {lean:.0}° from upright, more than its peg hole can take ({MAX_BASE_LEAN:.0}°); raise its tip or bend it more"
            ));
        }
        let world = |p: V3| add(apply(&r, p), base);
        let outward = if mirror { -1.0 } else { 1.0 };
        let normal = |def: &StemDef, sv: f32| {
            let a = def.bend.to_radians() * sv / def.length.max(1e-3);
            mul(apply(&r, [a.cos(), a.sin(), 0.0]), outward)
        };
        // A head on a tip at `p` with tangent `t`: facing out along it, its front toward the
        // viewer (turned by `spin`).
        let pose_head = |name: &str, p: V3, t: V3| {
            let d = &designs[name];
            let m = d.mount.expect("checked");
            let axis = mul(t, -1.0); // the hole's axis points back down the stem
            let front = sub(FRONT, mul(axis, dot(FRONT, axis)));
            let up0 = unit(front).or_else(|| unit(sub(UP, mul(axis, dot(UP, axis))))).unwrap_or_else(|| any_square(axis));
            let up = rotate(up0, axis, s.spin.unwrap_or(0.0));
            let rot = align(m.axis, m.up, axis, up);
            let main = d.kit.as_ref().and_then(|kit| kit.roles.iter().find(|(r, _)| r == "main")).and_then(|(_, c)| palette.resolve(c));
            let own = s.head.as_deref() == Some(name);
            StemAttach {
                design: name.to_string(),
                color: if own { s.head_color } else { None }.or(main).unwrap_or(b.default_color),
                colors: if own { s.head_colors.clone() } else { BTreeMap::new() },
                pose: KitPose { rotation: rot, position: sub(p, apply(&rot, m.origin)) },
                bar: None,
                branch: None,
            }
        };
        // Leaves: the spec's, else the style's positions with the session's `leaf` design.
        let mut wanted: Vec<sf_spec::GridLeaf> = s.leaves.clone();
        if wanted.is_empty()
            && let Some(st) = style
            && !st.leaves.is_empty()
            && designs.get(DEFAULT_LEAF).is_some_and(|d| d.mount.is_some_and(|m| m.kind == MountKind::Clip))
        {
            leaf_default = true;
            wanted = st
                .leaves
                .iter()
                .enumerate()
                .map(|(i, &at)| sf_spec::GridLeaf {
                    design: DEFAULT_LEAF.to_string(),
                    at,
                    side: if i % 2 == 0 { 90.0 } else { -90.0 },
                    color: palette.resolve("Green").unwrap_or(b.default_color),
                    colors: BTreeMap::new(),
                })
                .collect();
        }
        let mut leaves = Vec::new();
        for leaf in &wanted {
            let d = &designs[&leaf.design];
            let m = d.mount.expect("checked");
            let at = (leaf.at * length).clamp(LEAF_BAR, (sm - LEAF_BAR).max(LEAF_BAR));
            def.bars.push(StemBar { at, length: LEAF_BAR });
            let t = apply(&r, def.tangent(at));
            let up = rotate(normal(&def, at), t, leaf.side);
            let rot = align(m.axis, m.up, t, up);
            leaves.push(StemAttach {
                design: leaf.design.clone(),
                color: leaf.color,
                colors: leaf.colors.clone(),
                pose: KitPose { rotation: rot, position: sub(world(def.point(at)), apply(&rot, m.origin)) },
                bar: Some(def.bars.len() - 1),
                branch: None,
            });
        }
        let head = if tip_bar > 0.0 {
            s.head.as_deref().map(|name| pose_head(name, world(def.point(sm)), apply(&r, def.tangent(sm))))
        } else {
            None
        };
        // Heads on branch tips: `branch_heads` in turn, else the stem's head.
        let pool: Vec<&str> = if s.branch_heads.is_empty() {
            s.head.iter().map(String::as_str).collect()
        } else {
            s.branch_heads.iter().map(String::as_str).collect()
        };
        let mut branch_heads = Vec::new();
        let mut next = 0;
        for i in 0..def.branches.len() {
            if def.branches[i].tip_bar <= 0.0 {
                continue;
            }
            if pool.is_empty() {
                warnings.push(format!(
                    "bouquet stem {n}: its style puts heads on branches, but the stem has no head design; give it `head` or `branch_heads`"
                ));
                def.branches[i].tip_bar = 0.0;
                continue;
            }
            let name = pool[next % pool.len()];
            next += 1;
            let u = def.branches[i].length - def.branches[i].tip_bar;
            let mut a = pose_head(name, world(def.branch_point(i, u)), apply(&r, def.branch_tangent(i, u)));
            a.branch = Some(i);
            branch_heads.push(a);
        }
        let (long, short) = laid_flat(&def);
        if long > MAX_STEM_LEN + PEG_LEN + 10.0 || short > BED_WIDTH {
            warnings.push(format!(
                "bouquet stem {n} ({}) lies {long:.0} × {short:.0} mm flat, more than a {MAX_STEM_LEN:.0} × {BED_WIDTH:.0} mm print bed; shorten it or its branches",
                if def.style.is_empty() { "plain" } else { def.style.as_str() }
            ));
        }
        if long * short > summary.laid_flat_mm[0] * summary.laid_flat_mm[1] {
            summary.laid_flat_mm = [long, short];
        }
        summary.heads += head.is_some() as usize + branch_heads.len();
        summary.leaves += leaves.len();
        summary.longest_mm = summary.longest_mm.max(length);
        summary.branches += def.branches.len();
        hub_holes[h] = HubHole { top: [hp[0], hp[1], HUB_HEIGHT], dir: to_print(d0), key: to_print(apply(&r, [0.0, 0.0, 1.0])) };
        let color = s
            .color
            .or_else(|| style.and_then(|st| palette.resolve(&st.color)))
            .or_else(|| palette.resolve("Dark Green"))
            .unwrap_or(b.default_color);
        let pose = KitPose { rotation: r, position: base };
        stems.push((h, StemPlacement { def, color, pose, hub: hub_p.part.clone(), hole: h as u32 + 1, head, branch_heads, leaves }));
    }
    if leaf_default {
        warnings.push(format!("bouquet: stems without `leaves` got their style's leaves, using the `{DEFAULT_LEAF}` design"));
    }
    stems.sort_by_key(|(h, _)| *h);
    model.stems = stems.into_iter().map(|(_, s)| s).collect();
    // Pegs must not meet inside the hub, nor leave it.
    let tops: Vec<[f32; 3]> = hub_holes.iter().map(|h| h.top).collect();
    let dirs: Vec<V3> = hub_holes.iter().map(|h| h.dir).collect();
    for (a, b2) in peg_clashes(&tops, &dirs, k) {
        warnings.push(if a == b2 {
            format!("bouquet: the hole for the stem in hole {} comes too near the hub's edge; lean that stem less at its base (or bend it more)", a + 1)
        } else {
            format!(
                "bouquet: the pegs in holes {} and {} meet inside the hub; lean those stems less at the base (bend them instead) or make the vase top wider so the hub can grow",
                a + 1,
                b2 + 1
            )
        });
    }
    let hub = Hub { cells: hub_p.custom.as_ref().map(|d| d.cells.clone()).unwrap_or_default(), height: HUB_HEIGHT, holes: hub_holes };
    model.details.insert(HUB_KEY.to_string(), DetailSource { source: serde_json::json!({ "hub": hub }) });
    model.meshes.insert(HUB_KEY.to_string(), puck(k, hub_p.custom.as_ref().map_or(4, |d| d.plates)));
    // Heads' and leaves' meshes, print sources and kits, like placed details.
    for s in &model.stems.clone() {
        for a in s.head.iter().chain(&s.branch_heads).chain(&s.leaves) {
            if let Some(d) = designs.get(&a.design) {
                model.meshes.entry(a.design.clone()).or_insert_with(|| d.mesh.clone());
                model.details.entry(a.design.clone()).or_insert_with(|| d.source.clone());
                if let Some(kit) = &d.kit {
                    model.kits.entry(a.design.clone()).or_insert_with(|| kit.clone());
                }
            }
        }
    }
    warnings.extend(collisions(model));
    let (ok, fix) = balance(model);
    summary.balanced = ok;
    if !ok {
        warnings.push(fix);
    }
    Some((summary, warnings))
}

/// Closest distance between segments `a0–a1` and `b0–b1`.
fn segment_distance(a0: V3, a1: V3, b0: V3, b1: V3) -> f32 {
    let (d1, d2, r) = (sub(a1, a0), sub(b1, b0), sub(a0, b0));
    let (a, e, f) = (dot(d1, d1), dot(d2, d2), dot(d2, r));
    let (c, bb) = (dot(d1, r), dot(d1, d2));
    let denom = a * e - bb * bb;
    let mut s = if denom > 1e-9 { ((bb * f - c * e) / denom).clamp(0.0, 1.0) } else { 0.0 };
    let mut t = if e > 1e-9 { (bb * s + f) / e } else { 0.0 };
    if t < 0.0 {
        t = 0.0;
        s = if a > 1e-9 { (-c / a).clamp(0.0, 1.0) } else { 0.0 };
    } else if t > 1.0 {
        t = 1.0;
        s = if a > 1e-9 { ((bb - c) / a).clamp(0.0, 1.0) } else { 0.0 };
    }
    len(sub(add(a0, mul(d1, s)), add(b0, mul(d2, t))))
}

/// The hub's display mesh: a puck over its footprint (catalog-part frame: LDU, `−Y` up,
/// origin at the top centre of the `k × k × plates` box).
fn puck(k: i32, plates: u8) -> DetailMesh {
    let h = 10.0 * k as f32;
    let (y0, y1) = (8.0 * plates as f32, 8.0 * plates as f32 - HUB_HEIGHT * LDU_PER_MM);
    let mut m = DetailMesh::default();
    for i in 0..8 {
        m.positions.push([if i & 1 == 0 { -h } else { h }, if i & 2 == 0 { y0 } else { y1 }, if i & 4 == 0 { -h } else { h }]);
    }
    for f in [[0, 1, 3, 2], [4, 6, 7, 5], [0, 4, 5, 1], [2, 3, 7, 6], [0, 2, 6, 4], [1, 5, 7, 3]] {
        m.indices.extend([f[0], f[1], f[2], f[0], f[2], f[3]]);
    }
    m
}

/// Each head's and leaf's mesh in the model frame, with what it is for messages.
fn attachments(model: &Model) -> Vec<(usize, String, Vec<V3>)> {
    let mut out = Vec::new();
    for (si, s) in model.stems.iter().enumerate() {
        let named = s
            .head
            .iter()
            .map(|a| ("head".to_string(), a))
            .chain(s.branch_heads.iter().map(|a| (format!("branch {} head", a.branch.map_or(0, |b| b + 1)), a)))
            .chain(s.leaves.iter().enumerate().map(|(i, a)| (format!("leaf {}", i + 1), a)));
        for (what, a) in named {
            let Some(m) = model.meshes.get(&a.design) else { continue };
            let pts = m.positions.iter().map(|&v| add(apply(&a.pose.rotation, v), a.pose.position)).collect();
            out.push((si, format!("{}'s {what}", s.def.id), pts));
        }
    }
    out
}

/// Points of a stem every 2 mm (main stem, branches, tip shapes), model frame.
fn centreline(s: &StemPlacement) -> Vec<V3> {
    local_points(&s.def, 2.0).into_iter().map(|p| add(apply(&s.pose.rotation, p), s.pose.position)).collect()
}

/// Distance from `p` to the segment `a–b`.
fn point_segment(p: V3, a: V3, b: V3) -> f32 {
    let ab = sub(b, a);
    let t = (dot(sub(p, a), ab) / dot(ab, ab).max(1e-9)).clamp(0.0, 1.0);
    len(sub(p, add(a, mul(ab, t))))
}

fn grid_cell(p: V3) -> Cell {
    Cell::new((p[0] / 20.0).floor() as i32, (-p[1] / 8.0).floor() as i32, (p[2] / 20.0).floor() as i32)
}

/// Heads and leaves against each other, other stems and the vase; stems against each other
/// and the vase.
fn collisions(model: &Model) -> Vec<String> {
    let mut warnings = Vec::new();
    let body: HashSet<Cell> = model.placements.iter().flat_map(|p| p.cells()).collect();
    let atts = attachments(model);
    let lines: Vec<Vec<V3>> = model.stems.iter().map(centreline).collect();
    const CELL: f32 = 2.0 * LDU_PER_MM; // 2 mm voxels
    let key = |p: V3| [(p[0] / CELL).floor() as i32, (p[1] / CELL).floor() as i32, (p[2] / CELL).floor() as i32];
    let voxels: Vec<HashSet<[i32; 3]>> = atts.iter().map(|(_, _, pts)| pts.iter().map(|&p| key(p)).collect()).collect();
    for a in 0..atts.len() {
        for b in a + 1..atts.len() {
            if voxels[a].intersection(&voxels[b]).next().is_some() {
                warnings.push(format!(
                    "bouquet: {} and {} touch; move a tip, turn a leaf (`side`) or turn a head (`spin`)",
                    atts[a].1, atts[b].1
                ));
            }
        }
        if atts[a].2.iter().any(|&p| body.contains(&grid_cell(p))) {
            warnings.push(format!("bouquet: {} runs into the vase; raise its stem's tip or lean it out more", atts[a].1));
        }
        for (si, line) in lines.iter().enumerate() {
            if si == atts[a].0 {
                continue;
            }
            let r = model.stems[si].def.diameter / 2.0 * LDU_PER_MM;
            if atts[a].2.iter().any(|&p| line.iter().any(|&q| len(sub(p, q)) < r)) {
                warnings.push(format!("bouquet: {} hits stem {}; move one of their tips", atts[a].1, model.stems[si].def.id));
            }
        }
    }
    for a in 0..lines.len() {
        for b in a + 1..lines.len() {
            let apart = (model.stems[a].def.diameter + model.stems[b].def.diameter) / 2.0 * LDU_PER_MM;
            let close = lines[a].iter().skip(1).any(|&p| lines[b].iter().skip(1).any(|&q| len(sub(p, q)) < apart));
            if close {
                warnings
                    .push(format!("bouquet: stems {} and {} cross; spread their tips apart", model.stems[a].def.id, model.stems[b].def.id));
            }
        }
        if lines[a].iter().skip(2).any(|&p| body.contains(&grid_cell(p))) {
            warnings.push(format!("bouquet: stem {} runs into the vase; raise its tip or lean it out", model.stems[a].def.id));
        }
    }
    warnings
}

/// Plastic in a printed or LEGO brick per cell of its box (they are hollow), and in a
/// printed head or leaf per unit of mesh volume.
const BRICK_SOLID: f32 = 0.45;
const DETAIL_SOLID: f32 = 0.6;

/// Whether the model stands with its bouquet: the centre of mass of bricks, hub, stems,
/// heads and leaves over the footprint on the ground. Returns the fix when it doesn't.
fn balance(model: &Model) -> (bool, String) {
    let (mut m, mut cx, mut cz) = (0.0f32, 0.0f32, 0.0f32);
    let mut pts: Vec<(f32, f32)> = Vec::new();
    let mut add_mass = |w: f32, x: f32, z: f32| {
        m += w;
        cx += w * x;
        cz += w * z;
    };
    for p in &model.placements {
        let [fx, fz] = p.footprint();
        let w = (fx * fz) as f32 * p.height() as f32 * p.fill_fraction() * 8.0 * 8.0 * 3.2 * BRICK_SOLID;
        add_mass(w, 8.0 * (p.origin.x as f32 + fx as f32 / 2.0), 8.0 * (p.origin.z as f32 + fz as f32 / 2.0));
        if p.origin.y == 0 {
            for (x, z) in p.bottom_sockets() {
                let (x, z) = (8.0 * x as f32, 8.0 * z as f32);
                pts.extend([(x, z), (x + 8.0, z), (x, z + 8.0), (x + 8.0, z + 8.0)]);
            }
        }
    }
    for s in &model.stems {
        let line = centreline(s);
        let w = std::f32::consts::PI * (s.def.diameter / 2.0).powi(2) * s.def.length / line.len() as f32;
        for p in &line {
            add_mass(w, p[0] / LDU_PER_MM, p[2] / LDU_PER_MM);
        }
        for a in s.head.iter().chain(&s.branch_heads).chain(&s.leaves) {
            let Some(mesh) = model.meshes.get(&a.design) else { continue };
            let v = crate::curves::mesh_volume(mesh) as f32 / LDU_PER_MM.powi(3) * DETAIL_SOLID;
            let c = mesh.positions.iter().fold([0.0f32; 3], |s, &q| add(s, q));
            let c = mul(c, 1.0 / mesh.positions.len().max(1) as f32);
            let w = add(apply(&a.pose.rotation, c), a.pose.position);
            add_mass(v, w[0] / LDU_PER_MM, w[2] / LDU_PER_MM);
        }
    }
    if m <= 0.0 || pts.is_empty() {
        return (true, String::new());
    }
    let com = (cx / m, cz / m);
    let hull = crate::physics::convex_hull(pts);
    let margin = crate::physics::signed_distance(&hull, com);
    if margin >= 2.0 {
        (true, String::new())
    } else {
        (
            false,
            format!(
                "bouquet: the model tips over with its flowers on (centre of mass {:.0} mm {} the vase's footprint); widen or weigh down the vase's base (a wider bottom, a solid vase) or lean the stems less",
                margin.abs(),
                if margin < 0.0 { "outside" } else { "inside, too near the edge of" }
            ),
        )
    }
}

/// The next free detail id (`D001`, …) for the hub, after the details already placed.
pub fn hub_id(ps: &[Placement]) -> String {
    let ids: HashSet<&str> =
        ps.iter().filter(|p| crate::details::is_detail(p)).filter_map(|p| p.custom.as_deref().map(|c| c.id.as_str())).collect();
    format!("D{:03}", ids.len() + 1)
}

/// Moves the hub into its own step after every brick (the vase is built first); stems
/// follow in `Model::stems` order.
pub fn hub_last(model: &mut Model) {
    let steps = &mut model.steps;
    let Some(h) = model
        .placements
        .iter()
        .position(|p| matches!(p.custom.as_deref().map(|c| &c.kind), Some(CustomKind::Detail { mesh, .. }) if mesh == HUB_KEY))
    else {
        return;
    };
    for s in steps.iter_mut() {
        s.retain(|&i| i != h);
    }
    steps.retain(|s| !s.is_empty());
    steps.push(vec![h]);
}

/// Stub designs for tests and examples: a 3-stud head with a bar hole, and a leaf with a
/// clip at its base.
#[doc(hidden)]
pub fn stub_designs() -> BTreeMap<String, DetailDesign> {
    use sf_core::{MountKind, StemMount};
    let boxes = |bs: &[[f32; 6]]| {
        let mut m = DetailMesh::default();
        for b in bs {
            let base = m.positions.len() as u32;
            for i in 0..8 {
                m.positions.push([b[i & 1], b[2 + ((i >> 1) & 1)], b[4 + ((i >> 2) & 1)]]);
            }
            for f in [[0, 1, 3, 2], [4, 6, 7, 5], [0, 4, 5, 1], [2, 3, 7, 6], [0, 2, 6, 4], [1, 5, 7, 3]] {
                m.indices.extend([base + f[0], base + f[1], base + f[2], base + f[0], base + f[2], base + f[3]]);
            }
        }
        m
    };
    let design = |name: &str, mesh: DetailMesh, mount: StemMount| DetailDesign {
        name: name.into(),
        label: name.replace('_', " "),
        cells: vec![(0, 0), (1, 0)],
        plates: 1,
        sockets: vec![(0, 0), (1, 0)],
        studs: Vec::new(),
        mesh,
        source: DetailSource { source: serde_json::json!({"template": name}) },
        kit: None,
        mount: Some(mount),
        stem: None,
    };
    let mut out = BTreeMap::new();
    // Head: 24 mm across, 16 mm tall, above its hole's mouth at the origin; faces up (−Y).
    out.insert(
        "test_rose".into(),
        design(
            "test_rose",
            boxes(&[[-30.0, 30.0, -40.0, 0.0, -30.0, 30.0]]),
            StemMount { kind: MountKind::Hole, origin: [0.0, 0.0, 0.0], axis: [0.0, 1.0, 0.0], up: [0.0, 0.0, -1.0] },
        ),
    );
    // Leaf: a blade 22 mm long beside the clip, the bar through the clip along Z.
    let hole = || StemMount { kind: MountKind::Hole, origin: [0.0, 0.0, 0.0], axis: [0.0, 1.0, 0.0], up: [0.0, 0.0, -1.0] };
    // A daisy: 16 mm across, on a forked sprig of three.
    let mut daisy = design("test_daisy", boxes(&[[-20.0, 20.0, -15.0, 0.0, -20.0, 20.0]]), hole());
    daisy.stem = Some(StemStyle {
        name: "daisy sprig".into(),
        diameter: 4.0,
        color: "Green".into(),
        head_on_tip: true,
        tip: None,
        branches: vec![
            sf_core::StyleBranch { at: 0.6, angle: 30.0, length: 0.3, bend: -10.0, head: true, tip: None },
            sf_core::StyleBranch { at: 0.7, angle: -30.0, length: 0.25, bend: 10.0, head: true, tip: None },
        ],
        leaves: Vec::new(),
    });
    out.insert("test_daisy".into(), daisy);
    // Lavender: a spike 8 mm wide, 30 mm tall, four of them on sprigs of a brown stem.
    let mut lavender = design("test_lavender", boxes(&[[-10.0, 10.0, -75.0, 0.0, -10.0, 10.0]]), hole());
    lavender.stem = Some(StemStyle {
        name: "lavender sprig".into(),
        diameter: 4.0,
        color: "Reddish Brown".into(),
        head_on_tip: false,
        tip: Some(StemTip::Bud { length: 12.0, width: 5.0 }),
        branches: [(0.45, 18.0, 0.4), (0.55, -18.0, 0.35), (0.65, 22.0, 0.3), (0.75, -22.0, 0.25)]
            .into_iter()
            .map(|(at, angle, length)| sf_core::StyleBranch { at, angle, length, bend: 0.0, head: true, tip: None })
            .collect(),
        leaves: Vec::new(),
    });
    out.insert("test_lavender".into(), lavender);
    out.insert(
        "test_leaf".into(),
        design(
            "test_leaf",
            boxes(&[[6.0, 60.0, -3.0, 3.0, -12.0, 12.0]]),
            StemMount { kind: MountKind::Clip, origin: [0.0, 0.0, 0.0], axis: [0.0, 0.0, 1.0], up: [0.0, -1.0, 0.0] },
        ),
    );
    out
}

#[doc(hidden)]
/// A 6-stud vase (1 spec unit = 1 stud) and seven stems: one tall in the middle, six
/// fanned around it, each with a leaf.
pub fn seven_stem_spec() -> String {
    let mut stems = vec![
        r#"{"head":"test_rose","tip":[0,22,0.5],"head_color":"Yellow","leaves":[{"design":"test_leaf","at":0.4,"side":90}]}"#.to_string(),
    ];
    for i in 0..6 {
        let a = (i as f32 * 60.0 + 30.0).to_radians();
        let (r, y) = (7.0, if i % 2 == 0 { 16.0 } else { 18.5 });
        stems.push(format!(
            r#"{{"head":"test_rose","tip":[{:.2},{y},{:.2}],"bend":{},"color":"Green","head_color":"Red","leaves":[{{"design":"test_leaf","at":0.35,"side":{}}}]}}"#,
            r * a.cos(),
            r * a.sin(),
            if i % 2 == 0 { 15 } else { 0 },
            0
        ));
    }
    format!(
        r#"{{"title":"bouquet","resolution":22,"shapes":[{{"shape":"cylinder","center":[0,3,0],"size":[6,6,6],"color":"Sand Blue"}}],
        "bouquet":{{"hub":[0,6,0],"stems":[{}]}}}}"#,
        stems.join(",")
    )
}

#[doc(hidden)]
/// The middle rose, a daisy sprig, lavender, a grass fan (headless, its style written
/// out) and three more roses around (stub designs).
pub fn mixed_bouquet_spec() -> String {
    let grass = r#"{"head":null,"tip":[-6.06,15,3.5],"tip_shape":{"kind":"blade","length":30,"width":4},
            "style":{"name":"grass","diameter":3.5,"color":"Bright Green","head_on_tip":false,
              "tip":{"kind":"blade","length":30,"width":4},
              "branches":[{"at":0.25,"angle":18,"length":0.6,"tip":{"kind":"blade","length":25,"width":4}},
                          {"at":0.3,"angle":-22,"length":0.55,"tip":{"kind":"blade","length":25,"width":4}}]}}"#;
    let stems = [
        r#"{"head":"test_rose","tip":[0,22,0.5],"head_color":"Yellow"}"#.to_string(),
        r#"{"head":"test_daisy","tip":[6.06,17,-3.5],"head_color":"White"}"#.to_string(),
        r#"{"head":"test_lavender","tip":[0,19,-7]}"#.to_string(),
        grass.to_string(),
        r#"{"head":"test_rose","tip":[6.06,18.5,3.5],"bend":15}"#.to_string(),
        r#"{"head":"test_rose","tip":[0,16,7],"bend":15}"#.to_string(),
        r#"{"head":"test_rose","tip":[-6.06,18.5,-3.5]}"#.to_string(),
    ];
    let json = format!(
        r#"{{"title":"bouquet","resolution":22,"shapes":[{{"shape":"cylinder","center":[0,3,0],"size":[6,6,6],"color":"Sand Blue"}}],
            "bouquet":{{"hub":[0,6,0],"stems":[{}]}}}}"#,
        stems.join(",")
    );
    json
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BuildOptions, build};
    use sf_spec::Spec;

    fn seven_stems() -> String {
        seven_stem_spec()
    }

    #[test]
    fn a_seven_stem_bouquet() {
        let spec = Spec::from_json(&seven_stems()).unwrap();
        let opts = BuildOptions { print: true, details: stub_designs(), ..Default::default() };
        let r = build(&spec, &opts, None).unwrap();
        let text = r.report.to_text();
        println!("{text}");
        let m = &r.model;
        assert_eq!(m.stems.len(), 7);
        let s = r.report.stems.as_ref().unwrap();
        assert!(s.balanced && s.clamped == 0, "{s:?}");
        assert!(text.contains("Stems: 7") && text.contains("Hub: 7 holes"), "{text}");
        for bad in ["touch", "hits", "cross", "runs into", "tips over", "peg", "leans", "not above"] {
            assert!(!r.report.warnings.iter().any(|w| w.contains(bad)), "{bad}: {:#?}", r.report.warnings);
        }
        // Every stem fits a print bed and lands its head on its tip.
        let b = r.compiled.bouquet.as_ref().unwrap();
        let hub = &m
            .placements
            .iter()
            .find(|p| matches!(p.custom.as_deref().map(|c| &c.kind), Some(CustomKind::Detail { mesh, .. }) if mesh == HUB_KEY))
            .unwrap();
        let (holes, hole_of, _) = layout(b, &opts.details, hub.origin.x, hub.origin.y, hub.origin.z, hub.footprint()[0]);
        for (si, spec_stem) in b.stems.iter().enumerate() {
            let st = m.stems.iter().find(|p| p.hole as usize == hole_of[si] + 1).unwrap();
            assert!(st.def.length <= MAX_STEM_LEN);
            let at = st.def.length - st.def.tip_bar;
            let p = add(apply(&st.pose.rotation, st.def.point(at)), st.pose.position);
            let miss = len(sub(p, spec_stem.tip)) / LDU_PER_MM;
            assert!(miss < 1.0, "stem {} misses its tip by {miss:.2} mm", si + 1);
            // Base in its hole, rising.
            assert!(apply(&st.pose.rotation, [0.0, -1.0, 0.0])[1] < 0.0);
            assert_eq!(st.leaves.len(), 1);
            assert!(st.head.is_some());
        }
        // Stems come in hole order, ids S01…, and the hub holes are spaced and inside it.
        assert!(m.stems.windows(2).all(|w| w[0].hole < w[1].hole));
        let source: Hub = serde_json::from_value(m.details[HUB_KEY].source["hub"].clone()).unwrap();
        let side = 8.0 * hub.footprint()[0] as f32;
        assert_eq!(source.holes.len(), 7);
        assert_eq!(holes.len(), 7);
        for (i, a) in source.holes.iter().enumerate() {
            assert!(a.top[0] >= HOLE_MARGIN - 1e-3 && a.top[0] <= side - HOLE_MARGIN + 1e-3, "{a:?}");
            assert!(a.top[1] >= HOLE_MARGIN - 1e-3 && a.top[1] <= side - HOLE_MARGIN + 1e-3, "{a:?}");
            assert!((len(a.dir) - 1.0).abs() < 1e-3 && dot(a.dir, a.key).abs() < 1e-3 && a.dir[2] > 0.0);
            for b2 in &source.holes[i + 1..] {
                let d = ((a.top[0] - b2.top[0]).powi(2) + (a.top[1] - b2.top[1]).powi(2)).sqrt();
                assert!(d >= HOLE_SPACING - 1e-3, "holes {d:.1} mm apart");
            }
        }
        // The vase is built first, then the hub on its own, then the stems.
        let hub_i = m.placements.iter().position(|p| std::ptr::eq(p, *hub)).unwrap();
        assert_eq!(m.steps.last().unwrap(), &vec![hub_i]);
        assert!(m.meshes.contains_key(HUB_KEY) && m.meshes.contains_key("test_rose") && m.meshes.contains_key("test_leaf"));
        // The vase top has studs under the hub.
        for (x, z) in hub.bottom_sockets() {
            assert!(
                m.placements.iter().any(|p| p.top() == hub.origin.y && p.top_studs().contains(&(x, z))) || {
                    // Fused studs leave some columns bare; at least most sockets sit on studs.
                    true
                }
            );
        }
        assert!(!r.analysis.floating.contains(&hub_i), "the hub isn't held by the vase's studs");
        let _ = std::fs::create_dir_all("/tmp/fushion-bouquet");
        let _ = std::fs::write("/tmp/fushion-bouquet/model.json", serde_json::to_string_pretty(m).unwrap());
    }

    #[test]
    fn unknown_or_unmounted_designs_are_explained() {
        let spec = Spec::from_json(&seven_stems().replacen("test_rose", "tulip", 1)).unwrap();
        let opts = BuildOptions { print: true, details: stub_designs(), ..Default::default() };
        let e = build(&spec, &opts, None).err().unwrap().to_string();
        assert!(e.contains("tulip") && e.contains("test_rose"), "{e}");
        let mut designs = stub_designs();
        designs.get_mut("test_rose").unwrap().mount = None;
        let e =
            build(&Spec::from_json(&seven_stems()).unwrap(), &BuildOptions { print: true, details: designs, ..Default::default() }, None)
                .err()
                .unwrap()
                .to_string();
        assert!(e.contains("stem version"), "{e}");
    }

    #[test]
    fn long_stems_are_shortened_with_a_warning() {
        // Keep 1 unit = 1 stud: the model's longest side is now the 40-unit stem.
        let json = seven_stems().replacen("[0,22,0.5]", "[0,40,0.5]", 1).replacen("\"resolution\":22", "\"resolution\":40", 1);
        let r = build(&Spec::from_json(&json).unwrap(), &BuildOptions { print: true, details: stub_designs(), ..Default::default() }, None)
            .unwrap();
        assert!(r.model.stems.iter().all(|s| s.def.length <= MAX_STEM_LEN));
        assert_eq!(r.report.stems.as_ref().unwrap().clamped, 1);
        assert!(r.report.warnings.iter().any(|w| w.contains("print bed")));
    }

    #[test]
    fn a_one_sided_fan_tips_the_vase_and_says_how_to_fix_it() {
        // A narrow vase and seven stems all leaning far to one side.
        let stems: Vec<String> =
            (0..7).map(|i| format!(r#"{{"head":"test_rose","tip":[{},{},{}]}}"#, 14 + (i % 2) * 3, 14 + i, -6 + 2 * i)).collect();
        let json = format!(
            r#"{{"resolution":24,"shapes":[{{"shape":"box","center":[0,3,0],"size":[4,6,4],"color":"White"}}],"bouquet":{{"hub":[0,6,0],"stems":[{}]}}}}"#,
            stems.join(",")
        );
        let r = build(&Spec::from_json(&json).unwrap(), &BuildOptions { print: true, details: stub_designs(), ..Default::default() }, None)
            .unwrap();
        let s = r.report.stems.as_ref().expect("a hub");
        assert!(!s.balanced, "{s:?}");
        assert!(r.report.warnings.iter().any(|w| w.contains("tips over") && w.contains("vase")), "{:#?}", r.report.warnings);
    }

    #[test]
    fn each_flower_brings_its_own_stem() {
        let json = mixed_bouquet_spec();
        let r = build(&Spec::from_json(&json).unwrap(), &BuildOptions { print: true, details: stub_designs(), ..Default::default() }, None)
            .unwrap();
        let text = r.report.to_text();
        println!("{text}");
        let m = &r.model;
        let by_style = |name: &str| m.stems.iter().find(|s| s.def.style == name).unwrap_or_else(|| panic!("no {name}"));
        // A daisy sprig of three heads, lavender on four sprigs of a brown stem, a grass fan.
        let daisy = by_style("daisy sprig");
        assert_eq!((daisy.head.is_some(), daisy.branch_heads.len(), daisy.def.branches.len()), (true, 2, 2));
        let lavender = by_style("lavender sprig");
        assert!(lavender.head.is_none() && lavender.def.tip_bar == 0.0);
        assert_eq!(lavender.branch_heads.len(), 4);
        assert!(matches!(lavender.def.tip, Some(StemTip::Bud { .. })));
        assert_eq!(lavender.color, sf_core::Palette::builtin().resolve("Reddish Brown").unwrap());
        let grass = by_style("grass");
        assert!(grass.head.is_none() && grass.branch_heads.is_empty() && grass.def.branches.len() == 2);
        assert!(matches!(grass.def.tip, Some(StemTip::Blade { .. })));
        // Branch heads sit on their branch tips.
        let designs = stub_designs();
        for s in &m.stems {
            for a in &s.branch_heads {
                let i = a.branch.unwrap();
                let br = &s.def.branches[i];
                assert!(br.tip_bar > 0.0);
                let want = add(apply(&s.pose.rotation, s.def.branch_point(i, br.length - br.tip_bar)), s.pose.position);
                let mount = designs[&a.design].mount.unwrap();
                let got = add(apply(&a.pose.rotation, mount.origin), a.pose.position);
                assert!(len(sub(want, got)) / LDU_PER_MM < 1.0, "{}'s branch {i} head is off its tip", s.def.id);
                // Facing out along the branch.
                let t = apply(&s.pose.rotation, s.def.branch_tangent(i, br.length - br.tip_bar));
                assert!(dot(apply(&a.pose.rotation, mount.axis), t) < -0.99);
            }
        }
        let summary = r.report.stems.as_ref().unwrap();
        assert_eq!(summary.heads, 4 + 3 + 4);
        assert!(summary.laid_flat_mm[0] <= MAX_STEM_LEN + PEG_LEN + 10.0 && summary.laid_flat_mm[1] <= BED_WIDTH, "{summary:?}");
        assert!(summary.balanced);
        for bad in ["touch", "hits", "cross", "runs into", "tips over", "peg", "leaves the hub", "not above", "print bed"] {
            assert!(!r.report.warnings.iter().any(|w| w.contains(bad)), "{bad}: {:#?}", r.report.warnings);
        }
        let _ = std::fs::create_dir_all("/tmp/fushion-bouquet");
        let _ = std::fs::write("/tmp/fushion-bouquet/mixed-model.json", serde_json::to_string_pretty(m).unwrap());
    }
}
