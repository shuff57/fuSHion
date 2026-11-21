//! Voxel → parts packing.
//!
//! Layers are grouped into brick courses of three plates. In each course,
//! columns that are filled through all three plates with one colour become
//! brick cells; slopes are fitted to edge cells first, then bricks are packed
//! greedily, then leftover cells are packed with plates one layer at a time.
//! The greedy scorer prefers large parts, parts that bridge several parts
//! below, and penalises seams that line up with seams below (running bond).

use sf_core::color::FILLER;
use sf_core::parts::CATALOG;
use sf_core::{Availability, Cell, ColorId, Fill, PartDef, Placement, Rot, Shape};
use sf_spec::{Compiled, SAMPLES, SX, SY, SZ};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Key {
    Color(ColorId),
    Any,
}

fn key(f: Fill) -> Option<Key> {
    match f {
        Fill::Empty => None,
        Fill::Solid(c) => Some(Key::Color(c)),
        Fill::Interior => Some(Key::Any),
    }
}

fn merge(a: Key, b: Key) -> Option<Key> {
    match (a, b) {
        (Key::Any, k) | (k, Key::Any) => Some(k),
        (Key::Color(x), Key::Color(y)) if x == y => Some(a),
        _ => None,
    }
}

#[derive(Clone, Debug)]
pub struct PackOptions {
    pub slopes: bool,
    /// Rounds of local re-packing to fix floating parts and weak joints.
    pub repair_rounds: u32,
}

impl Default for PackOptions {
    fn default() -> Self {
        Self { slopes: true, repair_rounds: 300 }
    }
}

pub struct Packed {
    pub placements: Vec<Placement>,
    /// Parts removed because they formed tiny groups that nothing can hold
    /// (single cells sticking out sideways).
    pub dropped: usize,
    /// Small protruding features that were given an anchor cell inside the
    /// body so they can be held.
    pub anchored: usize,
    /// The grid actually packed, when anchoring changed it.
    pub compiled: Option<Compiled>,
    /// (part, colour) pairs placed although the availability data says they
    /// were never produced.
    pub unavailable: Vec<(String, ColorId)>,
    /// Notes for the report (details skipped or unsupported, say).
    pub warnings: Vec<String>,
    /// Display meshes of trimmed printed bricks.
    pub meshes: std::collections::BTreeMap<String, sf_core::DetailMesh>,
    /// What curving printed bricks did.
    pub curve_stats: Option<crate::print_pack::CurveStats>,
}

struct Packer<'a> {
    c: &'a Compiled,
    avail: &'a dyn Availability,
    dims: [i32; 3],
    owner: Vec<i32>,
    out: Vec<Placement>,
    unavailable: Vec<(String, ColorId)>,
    /// Random score noise used by the repair pass (0 = deterministic).
    jitter: i32,
    rng: u64,
    /// For hidden interior cells: the colour of the nearest visible cell, so
    /// interior parts blend in wherever they end up exposed.
    inner: &'a [ColorId],
    /// Empty cells no part may fill: above a slope's sloped columns and
    /// below an inverted slope's overhang.
    blocked: Vec<bool>,
}

/// Multi-source BFS from visible cells through filled cells.
pub(crate) fn nearest_surface_colors(c: &Compiled) -> Vec<ColorId> {
    let [dx, dy, dz] = c.grid.dims;
    let n = (dx * dy * dz) as usize;
    let idx = |x: i32, y: i32, z: i32| ((y * dz + z) * dx + x) as usize;
    let mut out = vec![FILLER; n];
    let mut seen = vec![false; n];
    let mut q = std::collections::VecDeque::new();
    for (cell, f) in c.grid.filled() {
        if let Fill::Solid(col) = f {
            let i = idx(cell.x, cell.y, cell.z);
            out[i] = col;
            seen[i] = true;
            q.push_back(cell);
        }
    }
    while let Some(cell) = q.pop_front() {
        let col = out[idx(cell.x, cell.y, cell.z)];
        for (ax, ay, az) in [(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)] {
            let nb = Cell::new(cell.x + ax, cell.y + ay, cell.z + az);
            if !c.grid.in_bounds(nb) || !c.grid.get(nb).is_filled() {
                continue;
            }
            let i = idx(nb.x, nb.y, nb.z);
            if !seen[i] {
                seen[i] = true;
                out[i] = col;
                q.push_back(nb);
            }
        }
    }
    out
}

fn bricks() -> Vec<&'static PartDef> {
    CATALOG.iter().filter(|p| p.height == 3 && p.shape == Shape::Box).collect()
}

fn plates() -> Vec<&'static PartDef> {
    CATALOG.iter().filter(|p| p.height == 1 && p.shape == Shape::Box).collect()
}

/// Wedge test for a slope sample: `u` runs from the inner edge (0) to the
/// outer edge (1) of the sloped column, `v` from the bottom (0) to the top (1)
/// of the course.
fn in_slope(u: f32, v: f32) -> bool {
    v <= 1.0 - 0.83 * u
}
fn in_inverted(u: f32, v: f32) -> bool {
    v >= 0.83 * u
}

/// A chosen slope: (sloped column, high column, fall direction, inverted, colour).
type SlopePick = ((i32, i32), (i32, i32), (i32, i32), bool, ColorId);

/// Direction a slope falls → rotation of our rotation-0 part (falls to +x).
fn rot_for(d: (i32, i32)) -> Rot {
    match d {
        (1, 0) => Rot(0),
        (0, -1) => Rot(1),
        (-1, 0) => Rot(2),
        _ => Rot(3),
    }
}

impl<'a> Packer<'a> {
    fn new(c: &'a Compiled, avail: &'a dyn Availability, inner: &'a [ColorId]) -> Self {
        let dims = c.grid.dims;
        let n = (dims[0] * dims[1] * dims[2]) as usize;
        Packer {
            blocked: vec![false; n],
            c,
            avail,
            dims,
            owner: vec![-1; n],
            out: Vec::new(),
            unavailable: Vec::new(),
            jitter: 0,
            rng: 0x9E37_79B9_7F4A_7C15,
            inner,
        }
    }

    fn next_rand(&mut self) -> u64 {
        // xorshift64*
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn idx(&self, c: Cell) -> usize {
        ((c.y * self.dims[2] + c.z) * self.dims[0] + c.x) as usize
    }
    fn in_b(&self, c: Cell) -> bool {
        self.c.grid.in_bounds(c)
    }
    fn owner(&self, c: Cell) -> Option<usize> {
        if !self.in_b(c) {
            return None;
        }
        let o = self.owner[self.idx(c)];
        (o >= 0).then_some(o as usize)
    }
    fn free(&self, c: Cell) -> Option<Key> {
        if !self.in_b(c) || self.owner[self.idx(c)] >= 0 || self.blocked[self.idx(c)] {
            return None;
        }
        key(self.c.grid.get(c))
    }

    /// May the packer use this part in this colour? Never-produced pairs are
    /// out; rare pairs (in 3 sets or fewer) are avoided too, so the parts
    /// list stays cheap and easy to order.
    fn made(&self, part: &str, color: ColorId) -> bool {
        self.avail.is_made(part, color) != Some(false) && self.avail.set_count(part, color).is_none_or(|n| n > RARE_SETS)
    }

    fn place(&mut self, p: Placement) {
        let i = self.out.len() as i32;
        let cells: Vec<Cell> = p.cells().collect();
        for c in cells {
            let k = self.idx(c);
            debug_assert!(self.owner[k] < 0, "overlap at {c:?}");
            self.owner[k] = i;
        }
        // Keep the space over a slope (and under an inverted slope's
        // overhang) clear, or the slope is hidden or blocked.
        if let Some(d) = p.def() {
            let (blocked_y, sloped): (i32, Vec<(u8, u8)>) = match d.shape {
                Shape::Slope { high } => {
                    (p.top(), (0..d.size[1]).flat_map(|lz| (high..d.size[0]).map(move |lx| (lx, lz))).collect())
                }
                Shape::InvertedSlope { high } => {
                    (p.origin.y - 1, (0..d.size[1]).flat_map(|lz| (high..d.size[0]).map(move |lx| (lx, lz))).collect())
                }
                _ => (0, Vec::new()),
            };
            for (lx, lz) in sloped {
                let (x, z) = p.local_to_grid(lx, lz);
                let c = Cell::new(x, blocked_y, z);
                if self.in_b(c) {
                    let k = self.idx(c);
                    self.blocked[k] = true;
                }
            }
        }
        self.out.push(p);
    }

    /// Key for a column being a brick cell of the course starting at `y`.
    /// A column qualifies when at least two of its three plates are filled
    /// (with one colour) and none is taken: a brick then also fills the
    /// missing plate, which costs a plate of accuracy but gives far fewer,
    /// better-connected parts than a stack of plates.
    fn brick_key(&self, x: i32, y: i32, z: i32) -> Option<Key> {
        let mut k: Option<Key> = None;
        let mut filled = 0;
        for dy in 0..3 {
            let c = Cell::new(x, y + dy, z);
            if !self.in_b(c) || self.owner[self.idx(c)] >= 0 || self.blocked[self.idx(c)] {
                return None;
            }
            if let Some(kc) = key(self.c.grid.get(c)) {
                filled += 1;
                k = Some(match k {
                    None => kc,
                    Some(prev) => merge(prev, kc)?,
                });
            }
        }
        if filled >= 2 { k } else { None }
    }

    /// Fraction of samples that agree with a box vs a wedge, for column `e`
    /// over the course at `y`, with outward direction `d`.
    fn wedge_fit(&self, e: (i32, i32), y: i32, d: (i32, i32), inverted: bool) -> (f32, f32) {
        let (mut box_ok, mut wedge_ok, mut n) = (0u32, 0u32, 0u32);
        for dy in 0..3 {
            let m = self.c.sample_mask(Cell::new(e.0, y + dy, e.1));
            for j in 0..SY {
                for k in 0..SZ {
                    for i in 0..SX {
                        let inside = m & sf_spec::sample_bit(i, j, k) != 0;
                        let fx = (i as f32 + 0.5) / SX as f32;
                        let fz = (k as f32 + 0.5) / SZ as f32;
                        let u = match d {
                            (1, 0) => fx,
                            (-1, 0) => 1.0 - fx,
                            (0, 1) => fz,
                            _ => 1.0 - fz,
                        };
                        let v = (dy as f32 + (j as f32 + 0.5) / SY as f32) / 3.0;
                        let w = if inverted { in_inverted(u, v) } else { in_slope(u, v) };
                        box_ok += inside as u32;
                        wedge_ok += (inside == w) as u32;
                        n += 1;
                    }
                }
            }
        }
        (box_ok as f32 / n as f32, wedge_ok as f32 / n as f32)
    }

    fn column_fill(&self, x: i32, y: i32, z: i32) -> f32 {
        (0..3).map(|dy| self.c.sample_mask(Cell::new(x, y + dy, z)).count_ones()).sum::<u32>() as f32 / (3 * SAMPLES) as f32
    }

    fn place_slopes(&mut self, y: i32, mask: &mut [Option<Key>]) {
        let [dx, dy, dz] = self.dims;
        let at = |x: i32, z: i32| (z * dx + x) as usize;
        // (score, e, i, d, inverted, colour)
        let mut cands: Vec<(f32, SlopePick)> = Vec::new();
        for z in 0..dz {
            for x in 0..dx {
                let Some(ke) = mask[at(x, z)] else { continue };
                for d in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let (ix, iz) = (x - d.0, z - d.1);
                    if ix < 0 || iz < 0 || ix >= dx || iz >= dz {
                        continue;
                    }
                    let Some(ki) = mask[at(ix, iz)] else { continue };
                    let Some(k) = merge(ke, ki) else { continue };
                    let color = match k {
                        Key::Color(c) => c,
                        Key::Any => continue,
                    };
                    // The outward neighbour must be open at this course.
                    let (ox, oz) = (x + d.0, z + d.1);
                    if self.c.grid.get(Cell::new(ox, y + 1, oz)).is_filled() {
                        continue;
                    }
                    if self.column_fill(ix, y, iz) < 0.8 {
                        continue;
                    }
                    for inverted in [false, true] {
                        if !inverted
                            && y + 3 < dy
                            && (self.c.grid.get(Cell::new(x, y + 3, z)).is_filled()
                                || self.owner(Cell::new(x, y + 3, z)).is_some())
                        {
                            continue; // something sits on the sloped column
                        }
                        if inverted
                            && (y == 0
                                || self.c.grid.get(Cell::new(x, y - 1, z)).is_filled()
                                || self.owner(Cell::new(x, y - 1, z)).is_some())
                        {
                            continue; // inverted slopes need open space below
                        }
                        let (b, w) = self.wedge_fit((x, z), y, d, inverted);
                        if w >= 0.72 && w >= b + 0.12 {
                            cands.push((w - b, ((x, z), (ix, iz), d, inverted, color)));
                        }
                    }
                }
            }
        }
        cands.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut taken: Vec<SlopePick> = Vec::new();
        for (_, (e, i, d, inv, color)) in cands {
            if mask[at(e.0, e.1)].is_none() || mask[at(i.0, i.1)].is_none() {
                continue;
            }
            mask[at(e.0, e.1)] = None;
            mask[at(i.0, i.1)] = None;
            taken.push((e, i, d, inv, color));
        }
        // Merge side-by-side pairs into 2 × 2 slopes.
        let mut used = vec![false; taken.len()];
        for a in 0..taken.len() {
            if used[a] {
                continue;
            }
            let (e, i, d, inv, color) = taken[a];
            let perp = (d.1.abs(), d.0.abs());
            let wide = if inv { "3660b" } else { "3039" };
            let partner = (0..taken.len()).find(|&b| {
                !used[b] && b != a && {
                    let (e2, _, d2, inv2, c2) = taken[b];
                    d2 == d && inv2 == inv && c2 == color && e2 == (e.0 + perp.0, e.1 + perp.1)
                }
            });
            if let Some(b) = partner.filter(|_| self.made(wide, color)) {
                used[a] = true;
                used[b] = true;
                let (e2, i2, ..) = taken[b];
                let ox = e.0.min(i.0).min(e2.0).min(i2.0);
                let oz = e.1.min(i.1).min(e2.1).min(i2.1);
                self.place(Placement::new(wide, color, Cell::new(ox, y, oz), rot_for(d)));
            }
        }
        for (a, (e, i, d, inv, color)) in taken.into_iter().enumerate() {
            if used[a] {
                continue;
            }
            let narrow = if inv { "3665b" } else { "3040b" };
            let origin = Cell::new(e.0.min(i.0), y, e.1.min(i.1));
            if self.made(narrow, color) {
                self.place(Placement::new(narrow, color, origin, rot_for(d)));
            } else {
                // Give the cells back to the brick packer.
                mask[at(e.0, e.1)] = Some(Key::Color(color));
                mask[at(i.0, i.1)] = Some(Key::Color(color));
            }
        }
    }

    /// Greedy rectangle packing of one layer mask with parts of one height.
    fn pack_mask(&mut self, y: i32, mask: &mut [Option<Key>], parts: &[&'static PartDef], prefer_x: bool) {
        let [dx, _, dz] = self.dims;
        let at = |x: i32, z: i32| (z * dx + x) as usize;
        let below = |s: &Self, x: i32, z: i32| if y == 0 { None } else { s.owner(Cell::new(x, y - 1, z)) };
        for z in 0..dz {
            for x in 0..dx {
                let Some(k0) = mask[at(x, z)] else { continue };
                let mut best: Option<(i32, &'static PartDef, i32, i32, ColorId)> = None;
                for p in parts {
                    let s = [p.size[0] as i32, p.size[1] as i32];
                    let orients: &[(i32, i32)] = if s[0] == s[1] { &[(s[0], s[1])][..] } else { &[(s[0], s[1]), (s[1], s[0])][..] };
                    'o: for &(w, d) in orients {
                        if x + w > dx || z + d > dz {
                            continue;
                        }
                        let mut k = k0;
                        for zz in z..z + d {
                            for xx in x..x + w {
                                let Some(kc) = mask[at(xx, zz)] else { continue 'o };
                                let Some(m) = merge(k, kc) else { continue 'o };
                                k = m;
                            }
                        }
                        let color = match k {
                            Key::Color(c) => c,
                            Key::Any => self.inner[self.idx(Cell::new(x, y, z))],
                        };
                        if !self.made(p.id, color) && !(w == 1 && d == 1) {
                            continue;
                        }
                        // Score.
                        let area = w * d;
                        let mut owners: Vec<usize> = Vec::new();
                        let mut seams = 0;
                        for zz in z..z + d {
                            for xx in x..x + w {
                                if let Some(o) = below(self, xx, zz)
                                    && !owners.contains(&o)
                                {
                                    owners.push(o);
                                }
                            }
                        }
                        let seam = |a: (i32, i32), b: (i32, i32)| -> i32 {
                            match (below(self, a.0, a.1), below(self, b.0, b.1)) {
                                (Some(p), Some(q)) if p != q => 1,
                                _ => 0,
                            }
                        };
                        for zz in z..z + d {
                            seams += seam((x - 1, zz), (x, zz)) + seam((x + w - 1, zz), (x + w, zz));
                        }
                        for xx in x..x + w {
                            seams += seam((xx, z - 1), (xx, z)) + seam((xx, z + d - 1), (xx, z + d));
                        }
                        // Alternate the long axis course by course so layers cross.
                        let orient = if w == d { area } else if (w > d) == prefer_x { 2 * area } else { 0 };
                        let noise = if self.jitter > 0 { (self.next_rand() % (self.jitter as u64 + 1)) as i32 } else { 0 };
                        let score = area * 10 + (owners.len() as i32).min(4) * 8 - seams * 6 + orient + noise;
                        if best.is_none_or(|b| score > b.0) {
                            best = Some((score, p, w, d, color));
                        }
                    }
                }
                let Some((_, p, w, d, color)) = best else { continue };
                for zz in z..z + d {
                    for xx in x..x + w {
                        mask[at(xx, zz)] = None;
                    }
                }
                let rot = if p.size[0] as i32 == w { Rot(0) } else { Rot(1) };
                if !self.made(p.id, color) {
                    self.unavailable.push((p.id.to_string(), color));
                }
                self.place(Placement::new(p.id, color, Cell::new(x, y, z), rot));
            }
        }
    }

    fn plate_layer(&mut self, y: i32, prefer_x: bool) {
        let [dx, _, dz] = self.dims;
        let mut mask: Vec<Option<Key>> =
            (0..dz).flat_map(|z| (0..dx).map(move |x| (x, z))).map(|(x, z)| self.free(Cell::new(x, y, z))).collect();
        let ps = plates();
        self.pack_mask(y, &mut mask, &ps, prefer_x);
    }
}

/// Pack a compiled grid into placements.
fn pack_once(c: &Compiled, avail: &dyn Availability, opts: &PackOptions) -> (Vec<Placement>, Vec<(String, ColorId)>) {
    let dims = c.grid.dims;
    let inner = nearest_surface_colors(c);
    let mut pk = Packer::new(c, avail, &inner);
    // Pick the course phase that yields the most brick cells.
    let phase = (0..3)
        .max_by_key(|&p| {
            let mut count = 0;
            let mut y = p;
            while y + 2 < dims[1] {
                for z in 0..dims[2] {
                    for x in 0..dims[0] {
                        if pk.brick_key(x, y, z).is_some() {
                            count += 1;
                        }
                    }
                }
                y += 3;
            }
            // Prefer phase 0 on ties (bricks on the ground).
            count * 4 - p
        })
        .unwrap_or(0);
    let bricks = bricks();
    let mut y = 0;
    let mut course = 0;
    while y < dims[1] {
        let prefer_x = course % 2 == 0;
        if y >= phase && (y - phase) % 3 == 0 && y + 2 < dims[1] {
            let mut mask: Vec<Option<Key>> = (0..dims[2])
                .flat_map(|z| (0..dims[0]).map(move |x| (x, z)))
                .map(|(x, z)| pk.brick_key(x, y, z))
                .collect();
            if opts.slopes {
                pk.place_slopes(y, &mut mask);
            }
            pk.pack_mask(y, &mut mask, &bricks, prefer_x);
            for dy in 0..3 {
                pk.plate_layer(y + dy, prefer_x ^ (dy % 2 == 1));
            }
            y += 3;
            course += 1;
        } else {
            pk.plate_layer(y, prefer_x);
            y += 1;
        }
    }
    let mut unavailable = std::mem::take(&mut pk.unavailable);
    let out = std::mem::take(&mut pk.out);
    drop(pk);
    let placements = repair(c, avail, &inner, out, opts.repair_rounds, &mut unavailable);
    (placements, unavailable)
}

/// Groups of at most `max` parts that reach neither the ground nor the rest.
pub(crate) fn small_floating(placements: &[Placement], max: usize) -> Vec<Vec<usize>> {
    let a = analyze_parts(placements);
    a.components
        .into_iter()
        .filter(|g| g.len() <= max && !g.iter().any(|&i| placements[i].origin.y == 0))
        .collect()
}

/// Floating groups up to this size get anchor cells; up to `DROP_MAX` are
/// removed if anchoring fails.
pub(crate) const ANCHOR_MAX: usize = 12;
/// Part/colour pairs found in this many sets or fewer count as rare.
pub const RARE_SETS: u32 = 3;
pub(crate) const DROP_MAX: usize = 3;

/// Give each tiny group an anchor: recolour one filled neighbour cell in the
/// same layer (inside the body) to the group's colour, so one part can span
/// the feature and the body and clip in. Returns `None` if nothing changed.
pub(crate) fn anchor(c: &Compiled, placements: &[Placement], groups: &[Vec<usize>]) -> Option<Compiled> {
    let mut out = c.clone();
    let mut changed = false;
    for g in groups {
        // Each cell with the colour of the part that holds it.
        let cells: Vec<(Cell, ColorId)> =
            g.iter().flat_map(|&i| placements[i].cells().map(|cell| (cell, placements[i].color)).collect::<Vec<_>>()).collect();
        let in_group = |cell: Cell| cells.iter().any(|(c2, _)| *c2 == cell);
        // First choice: a hovering overhang with a short gap underneath rests
        // on what is below once the gap (at most one brick) is filled.
        let mut filled_gap = false;
        for &(cell, color) in &cells {
            if in_group(Cell::new(cell.x, cell.y - 1, cell.z)) {
                continue;
            }
            let mut gap = Vec::new();
            let mut y = cell.y - 1;
            while y >= 0 && !c.grid.get(Cell::new(cell.x, y, cell.z)).is_filled() && gap.len() <= 3 {
                gap.push(Cell::new(cell.x, y, cell.z));
                y -= 1;
            }
            let lands = y < 0 || c.grid.get(Cell::new(cell.x, y, cell.z)).is_filled();
            if lands && !gap.is_empty() && gap.len() <= 3 {
                for gc in gap {
                    out.grid.set(gc, Fill::Solid(color));
                }
                filled_gap = true;
            }
        }
        if filled_gap {
            changed = true;
            continue;
        }
        // Second choice: extend the feature one stud into the body, but only
        // into a cell nobody can see (interior, or a surface cell whose only
        // open side is covered by the feature itself).
        let hidden = |nb: Cell| -> bool {
            match c.grid.get(nb) {
                Fill::Interior => true,
                Fill::Solid(_) => [(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)].iter().all(|&(dx, dy, dz)| {
                    let n2 = Cell::new(nb.x + dx, nb.y + dy, nb.z + dz);
                    (n2.y < 0) || c.grid.get(n2).is_filled() || in_group(n2)
                }),
                Fill::Empty => false,
            }
        };
        'cells: for &(cell, color) in &cells {
            for (ax, az) in [(0, 1), (0, -1), (1, 0), (-1, 0)] {
                let nb = Cell::new(cell.x + ax, cell.y, cell.z + az);
                if in_group(nb) || !hidden(nb) {
                    continue;
                }
                if c.grid.get(nb) != Fill::Solid(color) {
                    out.grid.set(nb, Fill::Solid(color));
                    changed = true;
                }
                continue 'cells;
            }
        }
    }
    changed.then_some(out)
}

/// Pack a compiled grid into placements: greedy packing, local repair,
/// anchoring of small protruding features, and finally removal of anything
/// tiny that still can't be held.
pub fn pack(c: &Compiled, avail: &dyn Availability, opts: &PackOptions) -> Packed {
    let mut owned: Option<Compiled> = None;
    let mut anchored = 0;
    for attempt in 0..3 {
        let cc = owned.as_ref().unwrap_or(c);
        let (mut placements, mut unavailable) = pack_once(cc, avail, opts);
        let small = if opts.repair_rounds > 0 { small_floating(&placements, ANCHOR_MAX) } else { Vec::new() };
        if !small.is_empty() && attempt < 2
            && let Some(next) = anchor(cc, &placements, &small) {
                anchored += small.len();
                owned = Some(next);
                continue;
            }
        let drop: Vec<usize> = small.iter().filter(|g| g.len() <= DROP_MAX).flatten().copied().collect();
        let dropped = drop.len();
        let mut i = 0;
        placements.retain(|_| {
            let keep = !drop.contains(&i);
            i += 1;
            keep
        });
        unavailable.sort();
        unavailable.dedup();
        return Packed { placements, dropped, anchored, unavailable, compiled: owned, warnings: Vec::new(), meshes: Default::default(), curve_stats: None };
    }
    unreachable!("the last attempt always returns")
}

fn analyze_parts(placements: &[Placement]) -> crate::physics::Analysis {
    crate::physics::analyze(&sf_core::Model { placements: placements.to_vec(), ..Default::default() })
}

/// Structural badness: (parts not reaching the ground, parts held by single
/// studs). Lower is better.
fn badness(a: &crate::physics::Analysis) -> (usize, usize) {
    (a.floating.len(), a.weak_joints.iter().map(|w| w.parts_held).sum())
}

/// Local re-packing: around a floating part or a weak joint, remove the parts
/// of the surrounding courses in a small window and pack those cells again
/// with random tie-breaking. Keep the change only if the structure got no
/// worse.
pub(crate) fn repair(
    c: &Compiled,
    avail: &dyn Availability,
    inner: &[ColorId],
    mut placements: Vec<Placement>,
    rounds: u32,
    unavailable: &mut Vec<(String, ColorId)>,
) -> Vec<Placement> {
    let mut a = analyze_parts(&placements);
    let mut score = badness(&a);
    let mut seed = 0x1234_5678_9abc_def1u64;
    let mut stale = 0;
    for round in 0..rounds {
        if score == (0, 0) || stale > 80 {
            break;
        }
        let targets: Vec<usize> = if !a.floating.is_empty() {
            a.floating.clone()
        } else {
            a.weak_joints.iter().flat_map(|w| w.parts).collect()
        };
        if targets.is_empty() {
            break;
        }
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let t = placements[targets[(seed >> 33) as usize % targets.len()]].clone();
        // Window: the target's footprint grown by 3 studs; vertically, start
        // with the target's own layers (or the layer above/below it, at
        // random) and grow to whole parts.
        let [fx, fz] = t.footprint();
        let (x0, x1, z0, z1) = (t.origin.x - 3, t.origin.x + fx + 3, t.origin.z - 3, t.origin.z + fz + 3);
        let shift = match (seed >> 20) % 3 {
            0 => 0,
            1 => -1,
            _ => t.height(),
        };
        let in_window = |p: &Placement| {
            let [px, pz] = p.footprint();
            p.origin.x >= x0 && p.origin.x + px <= x1 && p.origin.z >= z0 && p.origin.z + pz <= z1
        };
        let mut span = (t.origin.y + shift, t.origin.y + shift + if shift == 0 { t.height() } else { 1 });
        loop {
            let mut grown = span;
            for p in placements.iter().filter(|p| in_window(p)) {
                if p.origin.y < span.1 && p.top() > span.0 {
                    grown = (grown.0.min(p.origin.y), grown.1.max(p.top()));
                }
            }
            if grown == span || grown.1 - grown.0 > 6 {
                span = grown;
                break;
            }
            span = grown;
        }
        let inside = |p: &Placement| {
            in_window(p) && p.origin.y >= span.0 && p.top() <= span.1 && !p.def().is_some_and(|d| d.is_slope())
        };
        let (removed, kept): (Vec<Placement>, Vec<Placement>) = placements.iter().cloned().partition(|p| inside(p));
        if removed.is_empty() {
            stale += 1;
            continue;
        }
        let mut pk = Packer::new(c, avail, inner);
        pk.jitter = 14;
        pk.rng = seed | 1;
        for p in kept {
            pk.place(p);
        }
        let freed: Vec<Cell> = removed.iter().flat_map(|p| p.cells().collect::<Vec<_>>()).collect();
        let [dx, _, dz] = pk.dims;
        let at = |x: i32, z: i32| (z * dx + x) as usize;
        let bricks = bricks();
        let plates = plates();
        for y in span.0..span.1 {
            let prefer_x = (round as i32 + y) % 2 == 0;
            if y + 3 <= span.1 {
                let mut mask = vec![None; (dx * dz) as usize];
                for cell in freed.iter().filter(|c| c.y == y) {
                    mask[at(cell.x, cell.z)] = pk.brick_key(cell.x, y, cell.z);
                }
                pk.pack_mask(y, &mut mask, &bricks, prefer_x);
            }
            let mut mask = vec![None; (dx * dz) as usize];
            for cell in freed.iter().filter(|c| c.y == y) {
                mask[at(cell.x, cell.z)] = pk.free(*cell);
            }
            pk.pack_mask(y, &mut mask, &plates, prefer_x);
        }
        let na = analyze_parts(&pk.out);
        let new_score = badness(&na);
        if new_score <= score {
            stale = if new_score < score { 0 } else { stale + 1 };
            score = new_score;
            placements = pk.out;
            a = na;
            unavailable.extend(pk.unavailable);
        } else {
            stale += 1;
        }
    }
    placements
}

#[cfg(test)]
mod tests {
    use super::*;
    use sf_core::{Palette, UnknownAvailability};
    use sf_spec::{CompileOptions, Spec, compile};

    fn build(json: &str, res: u32) -> (Compiled, Packed) {
        let s = Spec::from_json(json).unwrap();
        let c = compile(&s, &Palette::builtin(), &CompileOptions { resolution: Some(res), image: None }).unwrap();
        let p = pack(&c, &UnknownAvailability, &PackOptions::default());
        (c, p)
    }

    #[test]
    fn covers_every_cell_exactly_once() {
        let (c, p) = build(
            r#"{"shapes":[{"shape":"ellipsoid","center":[0,5,0],"size":[10,10,8],"color":"Orange"},
            {"shape":"box","center":[0,1,0],"size":[6,2,6],"color":"Blue"}]}"#,
            12,
        );
        let mut seen = std::collections::HashSet::new();
        for pl in &p.placements {
            for cell in pl.cells() {
                assert!(seen.insert(cell), "overlap at {cell:?}");
                assert!(c.grid.get(cell).is_filled() || pl.height() == 3, "plate outside the shape at {cell:?}");
            }
        }
        for (cell, _) in c.grid.filled() {
            assert!(seen.contains(&cell), "cell {cell:?} not covered");
        }
    }

    #[test]
    fn solid_block_uses_big_bricks() {
        // 8 × 8 studs, 20 plates tall: 6 brick courses + 2 plate layers.
        let (_, p) = build(r#"{"shapes":[{"shape":"box","center":[0,0,0],"size":[8,8,8],"color":"Red"}]}"#, 8);
        let bricks = p.placements.iter().filter(|pl| pl.height() == 3).count();
        assert!(p.placements.len() <= 40, "too many parts: {}", p.placements.len());
        assert!(bricks * 2 > p.placements.len(), "mostly bricks expected: {bricks}/{}", p.placements.len());
    }

    #[test]
    fn colours_are_respected() {
        let (c, p) = build(
            r#"{"shapes":[{"shape":"box","center":[0,0,0],"size":[4,2,4],"color":"Red"},
            {"shape":"box","center":[1,0,0],"size":[2,2.2,4.2],"color":"Blue","op":"paint"}]}"#,
            4,
        );
        for pl in &p.placements {
            for cell in pl.cells() {
                if let Fill::Solid(col) = c.grid.get(cell) {
                    assert_eq!(col, pl.color, "colour mismatch at {cell:?}");
                }
            }
        }
    }

    #[test]
    fn dome_gets_slopes() {
        let (_, p) = build(r#"{"shapes":[{"shape":"ellipsoid","center":[0,0,0],"size":[12,10,12],"color":"Green"}]}"#, 12);
        let slopes = p.placements.iter().filter(|pl| pl.def().unwrap().is_slope()).count();
        assert!(slopes > 0, "expected some slopes on a dome");
    }

    /// Random specs: no overlaps, no part contradicting a visible cell's
    /// colour, nothing on a slope's sloped column or under an inverted slope's
    /// overhang, and every visible cell covered unless its part was dropped.
    #[test]
    fn packing_invariants_on_random_specs() {
        let mut seed = 0x5eed_u64;
        let mut rnd = |lo: f32, hi: f32| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            lo + (hi - lo) * ((seed >> 33) as f32 / (1u64 << 31) as f32)
        };
        let colours = ["Red", "White", "Blue", "Yellow", "Black", "Orange", "Green"];
        for case in 0..60 {
            let mut shapes = Vec::new();
            for _ in 0..(2 + case % 5) {
                let kind = ["ellipsoid", "box", "cylinder"][(rnd(0.0, 2.99)) as usize];
                let col = colours[rnd(0.0, 6.99) as usize];
                let op = if rnd(0.0, 1.0) < 0.2 { "paint" } else { "add" };
                shapes.push(format!(
                    r#"{{"shape":"{kind}","center":[{:.2},{:.2},{:.2}],"size":[{:.2},{:.2},{:.2}],"color":"{col}","op":"{op}"}}"#,
                    rnd(-4.0, 4.0), rnd(0.0, 8.0), rnd(-4.0, 4.0), rnd(1.0, 9.0), rnd(1.0, 9.0), rnd(1.0, 9.0)
                ));
            }
            let json = format!(r#"{{"shapes":[{}]}}"#, shapes.join(","));
            let Ok(spec) = Spec::from_json(&json) else { continue };
            let res = 8 + (case % 4) as u32 * 4;
            let Ok(c) = compile(&spec, &Palette::builtin(), &CompileOptions { resolution: Some(res), image: None }) else { continue };
            let p = pack(&c, &UnknownAvailability, &PackOptions::default());
            let grid = p.compiled.as_ref().unwrap_or(&c).grid.clone();
            let mut owner = std::collections::HashMap::new();
            for (i, pl) in p.placements.iter().enumerate() {
                for cell in pl.cells() {
                    assert!(owner.insert(cell, i).is_none(), "case {case}: overlap at {cell:?}\n{json}");
                    if let Fill::Solid(col) = grid.get(cell) {
                        assert_eq!(col, pl.color, "case {case}: colour mismatch at {cell:?}\n{json}");
                    }
                }
            }
            for pl in &p.placements {
                let d = pl.def().unwrap();
                let (y, high) = match d.shape {
                    Shape::Slope { high } => (pl.top(), high),
                    Shape::InvertedSlope { high } => (pl.origin.y - 1, high),
                    _ => continue,
                };
                for lz in 0..d.size[1] {
                    for lx in high..d.size[0] {
                        let (x, z) = pl.local_to_grid(lx, lz);
                        assert!(!owner.contains_key(&Cell::new(x, y, z)), "case {case}: part blocks a slope at ({x},{y},{z})\n{json}");
                    }
                }
            }
            let uncovered = grid.filled().filter(|(cell, f)| matches!(f, Fill::Solid(_)) && !owner.contains_key(cell)).count();
            assert!(uncovered == 0 || p.dropped > 0, "case {case}: {uncovered} visible cells uncovered\n{json}");
        }
    }

}
