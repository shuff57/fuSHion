//! Compliant sockets: round studs on top, and springs in the hollow underneath that grip
//! the studs of a brick pushed in from below (a real LEGO stud Ø4.8, or a printed one a
//! little over or under).
//!
//! Everything that flexes bends **within print layers**: each spring is a vertical
//! extrusion, joined to the rest only along vertical edges, and free at its top (a gap
//! under the ceiling), so pushing it bends it in the bed plane and never pulls layers
//! apart.
//!
//! * **T-springs** (every brick but a 1×1): wherever two cells meet along a wall, a web
//!   joined to the wall carries two fingers running along it, one into each cell. Each
//!   finger is a cantilever whose tip presses the stud's flank, where the stud curves away
//!   from the wall; behind it a slot (a shallow groove in the wall) gives it room to bend.
//!   1-wide bricks get them on both long walls, so every stud is squeezed from both sides.
//! * **Wall leaves** (a 1×1, the only brick with no two cells along a wall): two opposite
//!   walls are cut free from the corner on one side and from the top by through-slits, and
//!   carry a pad at the middle that presses the stud.
//! * **Tubes** (between four studs) are rigid stops, Ø6.4: clear of a Ø4.8 stud, they only
//!   touch when a brick is pushed off-centre. (A tube that flexes would have to hang from the
//!   ceiling, which bends it across layers, or be a ring, which is too stiff for its size.)
//!
//! The numbers are in [`SocketSpec`]; [`socket_fit`] works out the preload and the strain at
//! every stud for any stud size and offset.

use crate::geom;
use crate::shape::P2;
use crate::{CustomError, Result, StudProfile};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Young's modulus used for spring forces, MPa (PLA; PETG is about 2100).
pub const E_PLA_MPA: f64 = 3500.0;

/// The springs of a compliant-socket brick, in mm.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SocketSpec {
    /// Finger thickness (across the wall).
    pub finger: f64,
    /// Slot behind a finger (its room to bend), and the slits that free a wall leaf.
    pub slot: f64,
    /// How far a finger's tip (or a leaf's pad) reaches into a nominal stud.
    pub preload: f64,
    /// Width of the web joining a T-spring to its wall.
    pub anchor: f64,
    /// Thickness of a 1×1's wall leaf.
    pub leaf: f64,
    /// Gap kept between a spring's top and the hollow's ceiling.
    pub top_gap: f64,
}

impl SocketSpec {
    /// For a 0.2–0.25 mm nozzle: 0.45 mm fingers, 0.4 mm slots, 0.12 mm preload.
    pub fn fine() -> Self {
        Self { finger: 0.45, slot: 0.4, preload: 0.12, anchor: 0.6, leaf: 0.8, top_gap: 0.6 }
    }
    /// For a 0.4 mm nozzle: 0.6 mm fingers and slots (one line each), 0.10 mm preload (the
    /// thicker finger is stiffer, so less keeps it under [`crate::MAX_STRAIN`]).
    pub fn nozzle04() -> Self {
        Self { finger: 0.6, slot: 0.6, preload: 0.10, anchor: 0.6, leaf: 0.8, top_gap: 0.6 }
    }
}

/// Where the finger's back face stands off the wall's inner face, mm (the wall is grooved
/// behind it to make the slot).
const FINGER_BACK: f64 = 0.3;
/// How far a web reaches into the wall behind its groove, mm.
const WEB_EMBED: f64 = 0.3;
/// Half the width of a wall leaf's pad, mm.
const PAD_HALF: f64 = 0.6;

/// A wall seen from inside a cell: `edge` is a point on the cell edge, `along` the unit
/// direction along it, `inward` the unit normal into the hollow.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WallFrame {
    pub edge: P2,
    pub along: P2,
    pub inward: P2,
}

impl WallFrame {
    /// The point `s` along the wall and `u` in from its inner face (`wall_in` from the edge).
    pub fn at(&self, s: f64, u: f64, wall_in: f64) -> P2 {
        let d = wall_in + u;
        [
            self.edge[0] + s * self.along[0] + d * self.inward[0],
            self.edge[1] + s * self.along[1] + d * self.inward[1],
        ]
    }
}

/// One T-spring: a web at `frame.edge` (the point where two cells meet) and a finger into
/// each of `cells`, the first at negative `s`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TSpring {
    pub frame: WallFrame,
    pub cells: [(i32, i32); 2],
}

/// A 1×1's wall leaf: the wall of `cell` along `frame` (from the cell's corner at `s` = 0),
/// its root at `s` = pitch − (clearance + wall), its free end near `s` = clearance + wall.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Leaf {
    pub frame: WallFrame,
    pub cell: (i32, i32),
}

/// The four sides of a cell: neighbour offset, and the wall frame on that side (`edge` at
/// the side's start corner so `s` runs 0..pitch along it, with the cell to the left).
fn sides(i: i32, j: i32, pitch: f64) -> [((i32, i32), WallFrame); 4] {
    let (x0, y0) = (f64::from(i) * pitch, f64::from(j) * pitch);
    let (x1, y1) = (x0 + pitch, y0 + pitch);
    [
        ((0, -1), WallFrame { edge: [x0, y0], along: [1.0, 0.0], inward: [0.0, 1.0] }),
        ((1, 0), WallFrame { edge: [x1, y0], along: [0.0, 1.0], inward: [-1.0, 0.0] }),
        ((0, 1), WallFrame { edge: [x1, y1], along: [-1.0, 0.0], inward: [0.0, -1.0] }),
        ((-1, 0), WallFrame { edge: [x0, y1], along: [0.0, -1.0], inward: [1.0, 0.0] }),
    ]
}

/// The springs of a footprint: a T-spring wherever two cells meet along a straight run of
/// wall, and wall leaves on two opposite walls of any cell that no T-spring reaches.
pub(crate) fn springs(cells: &BTreeSet<(i32, i32)>, pitch: f64) -> (Vec<TSpring>, Vec<Leaf>) {
    let mut ts = Vec::new();
    for &(i, j) in cells {
        for (k, (nb, f)) in sides(i, j, pitch).into_iter().enumerate() {
            if cells.contains(&(i + nb.0, j + nb.1)) {
                continue;
            }
            // The next cell along this wall (in the `along` direction) shares the wall?
            let step = [f.along[0].round() as i32, f.along[1].round() as i32];
            let next = (i + step[0], j + step[1]);
            if !cells.contains(&next) || cells.contains(&(next.0 + nb.0, next.1 + nb.1)) {
                continue;
            }
            let _ = k;
            let edge = [f.edge[0] + f.along[0] * pitch, f.edge[1] + f.along[1] * pitch];
            ts.push(TSpring { frame: WallFrame { edge, ..f }, cells: [(i, j), next] });
        }
    }
    let reached: BTreeSet<(i32, i32)> = ts.iter().flat_map(|t| t.cells).collect();
    let mut leaves = Vec::new();
    for &(i, j) in cells {
        if reached.contains(&(i, j)) {
            continue;
        }
        let s = sides(i, j, pitch);
        let open = |k: usize| !cells.contains(&(i + s[k].0.0, j + s[k].0.1));
        let pair = if open(0) && open(2) { [0, 2] } else if open(1) && open(3) { [1, 3] } else { continue };
        for k in pair {
            leaves.push(Leaf { frame: s[k].1, cell: (i, j) });
        }
    }
    (ts, leaves)
}

/// A T-spring's dimensions along (`s`, from the web's middle) and across (`u`, in from the
/// wall's inner face) its wall.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct TDims {
    /// The finger's stud-side face.
    pub face: f64,
    /// The finger's back face.
    pub back: f64,
    /// The groove's back (negative: into the wall).
    pub groove: f64,
    /// Where each finger's tip is (from the web's middle).
    pub tip: f64,
    /// Each finger's free length (from the web to the tip).
    pub length: f64,
    /// Height of the springs.
    pub height: f64,
}

/// The T-spring dimensions for `p` (with its socket spec) and a hollow `hollow_h` tall.
pub fn t_dims(p: &StudProfile, spec: &SocketSpec, hollow_h: f64) -> Result<TDims> {
    let half = p.pitch / 2.0 - p.clearance - p.wall;
    let r = p.stud_diameter / 2.0;
    let back = FINGER_BACK;
    let face = back + spec.finger;
    // The tip presses the nominal stud `preload` deep where the stud is `half − face +
    // preload` in from the wall's line through the stud's middle.
    let depth = half - face + spec.preload;
    if depth >= r {
        return Err(CustomError::InvalidProfile("the finger stands too far into the hollow".into()));
    }
    let dx = (r * r - depth * depth).sqrt();
    let tip = p.pitch / 2.0 - dx;
    let length = tip - spec.anchor / 2.0;
    let height = (p.stud_height + spec.top_gap).min(hollow_h - spec.top_gap);
    if length < 1.0 || height < 1.0 {
        return Err(CustomError::InvalidProfile("no room for socket springs".into()));
    }
    Ok(TDims { face, back, groove: back - spec.slot, tip, length, height })
}

/// A cut (`false`) or a solid added (`true`): closed outlines in the plane (mm), extruded
/// from `z0` to `z1`, and the volume the step adds (negative for cuts), mm³.
pub(crate) struct Step {
    pub add: bool,
    pub loops: Vec<Vec<P2>>,
    pub z0: f64,
    pub z1: f64,
    pub volume: f64,
}

/// The socket stages for `cells` (already moved so the smallest i and j are 0): the
/// grooves, the T-springs, and a 1×1's leaf slits, thinning and pads, with their exact
/// volumes. `chamfer` is the bottom chamfer already cut on the outside edges.
pub(crate) fn stages(
    p: &StudProfile,
    spec: &SocketSpec,
    cells: &BTreeSet<(i32, i32)>,
    hollow_h: f64,
) -> Result<Vec<(String, Step)>> {
    let d = t_dims(p, spec, hollow_h)?;
    let wall_in = p.clearance + p.wall;
    let (ts, leaves) = springs(cells, p.pitch);
    let mut out = Vec::new();
    if !ts.is_empty() {
        // Grooves: the wall behind each spring's span, cut back to the slot's back.
        let mut loops = Vec::new();
        let mut vol = 0.0;
        for t in &ts {
            let f = &t.frame;
            if d.groove < 0.0 {
                loops.push(vec![
                    f.at(-d.tip, d.groove, wall_in),
                    f.at(d.tip, d.groove, wall_in),
                    f.at(d.tip, 0.0, wall_in),
                    f.at(-d.tip, 0.0, wall_in),
                ]);
                vol -= 2.0 * d.tip * (-d.groove) * d.height;
            }
        }
        if !loops.is_empty() {
            out.push(("SocketGrooves".to_string(), Step { add: false, loops, z0: -1.0, z1: d.height, volume: vol }));
        }
        // The T-springs: the web (embedded in the wall behind the groove) and two fingers.
        let h = spec.anchor / 2.0;
        let web0 = d.groove.min(0.0) - WEB_EMBED;
        let mut loops = Vec::new();
        let mut vol = 0.0;
        for t in &ts {
            let f = &t.frame;
            let pts = vec![
                f.at(-h, web0, wall_in),
                f.at(h, web0, wall_in),
                f.at(h, d.back, wall_in),
                f.at(d.tip, d.back, wall_in),
                f.at(d.tip, d.face, wall_in),
                f.at(-d.tip, d.face, wall_in),
                f.at(-d.tip, d.back, wall_in),
                f.at(-h, d.back, wall_in),
            ];
            let area = crate::shape::signed_area(&pts).abs();
            // The web's embedded part was solid already.
            vol += (area - spec.anchor * WEB_EMBED) * d.height;
            loops.push(pts);
        }
        out.push(("SocketSprings".to_string(), Step { add: true, loops, z0: 0.0, z1: d.height, volume: vol }));
    }
    if !leaves.is_empty() {
        let s0 = wall_in;
        let s1 = p.pitch - wall_in;
        let sl = spec.slot;
        let hl = (p.stud_height + spec.top_gap).min(hollow_h - sl);
        let c = p.bottom_chamfer;
        let thin = p.wall - spec.leaf;
        // The free end's vertical slit, through the wall, bottom to the leaf's top.
        let mut slit = Vec::new();
        let mut top = Vec::new();
        let mut thinning = Vec::new();
        let mut pads = Vec::new();
        let (mut v_slit, mut v_top, mut v_thin, mut v_pad) = (0.0, 0.0, 0.0, 0.0);
        let pad_face = (p.pitch / 2.0 - wall_in) - (p.stud_diameter / 2.0 - spec.preload);
        for l in &leaves {
            let f = &l.frame;
            // `s` here runs from the cell's corner, `u` from the wall's inner face.
            let at = |s: f64, u: f64| f.at(s, u, wall_in);
            slit.push(vec![at(s0, -p.wall - 1.0), at(s0 + sl, -p.wall - 1.0), at(s0 + sl, 0.3), at(s0, 0.3)]);
            v_slit -= sl * (p.wall * hl - c * c / 2.0);
            top.push(vec![at(s0, -p.wall - 1.0), at(s1, -p.wall - 1.0), at(s1, 0.3), at(s0, 0.3)]);
            v_top -= (s1 - s0) * p.wall * sl;
            thinning.push(vec![at(s0 + sl, -thin), at(s1, -thin), at(s1, 0.0), at(s0 + sl, 0.0)]);
            v_thin -= (s1 - s0 - sl) * thin * hl;
            let mid = p.pitch / 2.0;
            pads.push(vec![at(mid - PAD_HALF, -thin - 0.3), at(mid + PAD_HALF, -thin - 0.3), at(mid + PAD_HALF, pad_face), at(mid - PAD_HALF, pad_face)]);
            v_pad += 2.0 * PAD_HALF * (pad_face + thin) * hl;
        }
        out.push(("LeafSlits".into(), Step { add: false, loops: slit, z0: -1.0, z1: hl, volume: v_slit }));
        out.push(("LeafTops".into(), Step { add: false, loops: top, z0: hl, z1: hl + sl, volume: v_top }));
        out.push(("LeafThinning".into(), Step { add: false, loops: thinning, z0: -1.0, z1: hl, volume: v_thin }));
        out.push(("LeafPads".into(), Step { add: true, loops: pads, z0: 0.0, z1: hl, volume: v_pad }));
    }
    Ok(out)
}

/// One spring pressing one stud.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Contact {
    /// The stud's cell.
    pub cell: (i32, i32),
    /// "finger" or "leaf".
    pub kind: String,
    /// How far the spring is pushed back, mm (0: it doesn't touch).
    pub preload: f64,
    /// Bending strain at its root.
    pub strain: f64,
    /// The force it presses the stud with, N (PLA).
    pub force: f64,
}

/// What a brick with compliant sockets does to studs pushed in from below.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SocketFit {
    /// The studs' diameter, mm.
    pub stud_diameter: f64,
    /// The studs' offset from the cell centres (the brick pushed off-centre), mm.
    pub offset: [f64; 2],
    /// Every spring and how it meets its stud.
    pub contacts: Vec<Contact>,
    /// Least preload on any stud (summing its springs), mm, and the cell.
    pub weakest_stud: ((i32, i32), f64),
    /// Largest strain in any spring.
    pub max_strain: f64,
    /// Least clearance between a stud and anything rigid (walls, tubes, webs), mm
    /// (negative: they clash).
    pub rigid_clearance: f64,
    /// Total grip force over all studs (sum of spring forces, N).
    pub total_force: f64,
}

/// Distance from `q` to the segment `a`–`b`.
fn seg_dist(q: P2, a: P2, b: P2) -> f64 {
    geom::segment_distance(q, a, b)
}

/// How the brick `cells` with profile `p` grips studs of `stud_d` whose centres sit
/// `offset` mm from the cell centres (the brick pushed that far off-centre). Each finger is
/// a cantilever pushed back `δ` (the stud's reach past its face, at its tip): strain
/// `1.5·δ·t/L²`, force `3·E·I·δ/L³` with `I = h·t³/12`. A leaf is the same with its root at
/// the corner and the load at the stud.
pub fn socket_fit(
    cells: &[(i32, i32)],
    p: &StudProfile,
    stud_d: f64,
    offset: [f64; 2],
) -> Result<SocketFit> {
    let spec = p.socket.ok_or_else(|| CustomError::InvalidProfile("not a compliant-socket profile".into()))?;
    let set: BTreeSet<(i32, i32)> = cells.iter().copied().collect();
    let hollow_h = p.plate_height * 3.0 - p.top_thickness;
    let d = t_dims(p, &spec, hollow_h)?;
    let (ts, leaves) = springs(&set, p.pitch);
    let wall_in = p.clearance + p.wall;
    let r = stud_d / 2.0;
    let centre = |c: (i32, i32)| [(f64::from(c.0) + 0.5) * p.pitch + offset[0], (f64::from(c.1) + 0.5) * p.pitch + offset[1]];
    let finger = |delta: f64, l: f64, t: f64, h: f64| -> (f64, f64) {
        let i = h * t.powi(3) / 12.0;
        (1.5 * delta * t / (l * l), 3.0 * E_PLA_MPA * i * delta / l.powi(3))
    };
    let mut contacts = Vec::new();
    for t in &ts {
        for (k, &cell) in t.cells.iter().enumerate() {
            let sgn = if k == 0 { -1.0 } else { 1.0 };
            // Penetration at the tip: the stud's reach toward the wall at the tip's `s`.
            let c = centre(cell);
            let f = &t.frame;
            let rel = [c[0] - f.edge[0], c[1] - f.edge[1]];
            let s_c = rel[0] * f.along[0] + rel[1] * f.along[1];
            let u_c = rel[0] * f.inward[0] + rel[1] * f.inward[1] - wall_in;
            let ds = sgn * d.tip - s_c;
            let delta = if ds.abs() < r { (d.face - (u_c - (r * r - ds * ds).sqrt())).max(0.0) } else { 0.0 };
            let (strain, force) = finger(delta, d.length, spec.finger, d.height);
            contacts.push(Contact { cell, kind: "finger".into(), preload: delta, strain, force });
        }
    }
    let hl = (p.stud_height + spec.top_gap).min(hollow_h - spec.slot);
    for l in &leaves {
        let c = centre(l.cell);
        let f = &l.frame;
        let rel = [c[0] - f.edge[0], c[1] - f.edge[1]];
        let s_c = rel[0] * f.along[0] + rel[1] * f.along[1];
        let u_c = rel[0] * f.inward[0] + rel[1] * f.inward[1] - wall_in;
        let pad_face = (p.pitch / 2.0 - wall_in) - (p.stud_diameter / 2.0 - spec.preload);
        let ds = (s_c - p.pitch / 2.0).abs();
        let delta = if ds <= PAD_HALF { (pad_face - (u_c - r)).max(0.0) } else { 0.0 };
        let root = p.pitch - wall_in;
        let (strain, force) = finger(delta, root - s_c.min(root - 0.5), spec.leaf, hl);
        contacts.push(Contact { cell: l.cell, kind: "leaf".into(), preload: delta, strain, force });
    }
    // Rigid clearance: walls (where no groove), tubes, webs.
    let mut clear = f64::MAX;
    for &cell in cells {
        let c = centre(cell);
        for (nb, f) in sides(cell.0, cell.1, p.pitch) {
            if set.contains(&(cell.0 + nb.0, cell.1 + nb.1)) {
                continue;
            }
            let a = f.at(0.0, 0.0, wall_in);
            let b = f.at(p.pitch, 0.0, wall_in);
            // A leaf's wall is a spring, not rigid.
            if leaves.iter().any(|l| l.cell == cell && l.frame.edge == f.edge) {
                continue;
            }
            // Grooved stretches stand back further; the rest of the wall is at the face.
            clear = clear.min(seg_dist(c, a, b) - r);
        }
        // Webs of springs on this cell's walls (their stud-side end).
        for t in ts.iter().filter(|t| t.cells.contains(&cell)) {
            let f = &t.frame;
            let h = spec.anchor / 2.0;
            let a = f.at(-h, d.face, wall_in);
            let b = f.at(h, d.face, wall_in);
            clear = clear.min(seg_dist(c, a, b) - r);
        }
    }
    let fp = crate::shape::Footprint::new(cells)?;
    for (i, j) in fp.full_corners() {
        let t = [f64::from(i) * p.pitch, f64::from(j) * p.pitch];
        for &cell in cells {
            let c = centre(cell);
            clear = clear.min((c[0] - t[0]).hypot(c[1] - t[1]) - p.tube_outer_diameter / 2.0 - r);
        }
    }
    let mut per: std::collections::BTreeMap<(i32, i32), f64> = cells.iter().map(|&c| (c, 0.0)).collect();
    for c in &contacts {
        *per.entry(c.cell).or_default() += c.preload;
    }
    let weakest = per.iter().map(|(&c, &v)| (c, v)).min_by(|a, b| a.1.total_cmp(&b.1)).unwrap_or(((0, 0), 0.0));
    Ok(SocketFit {
        stud_diameter: stud_d,
        offset,
        max_strain: contacts.iter().map(|c| c.strain).fold(0.0, f64::max),
        total_force: contacts.iter().map(|c| c.force).sum(),
        weakest_stud: weakest,
        rigid_clearance: clear,
        contacts,
    })
}
