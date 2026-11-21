//! Print-mode packing: every part is designed for its spot and 3D printed.
//!
//! Courses of three plates use the same 2-of-3 rule and phase choice as the catalog
//! packer, and leftover cells become one-plate layers. Within a layer, same-coloured
//! columns merge into connected polyominoes, cut by a lattice of 8 × 4 tiles (4 × 8 on
//! alternate courses) whose offset is chosen so seams don't line up with the layer below;
//! pieces of three cells or fewer are merged into a neighbour when the result still fits.
//! Every part is one 4-connected piece at most [`MAX_SIDE`] studs on a side and
//! [`MAX_CELLS`] cells. Parts with exactly the same columns and colour stacked directly
//! on each other are merged into one taller part (up to [`MAX_PLATES`] plates), which
//! is how thin features stay connected.
//!
//! Studs come last, once everything is placed (details too): each part's studs are fused
//! into units by [`sf_core::print::stud_layout`] so no unit lies under an edge of what sits
//! on it; a column left alone gets no stud. Parts get one id per distinct geometry.

use crate::details::{self, is_detail};
use crate::pack::{ANCHOR_MAX, DROP_MAX, anchor, nearest_surface_colors, small_floating};
use anyhow::Result;
use sf_core::print::stud_layout;
use sf_core::{Cell, ColorId, CustomDef, CustomKind, DetailDesign, Fill, Placement, Rot};
use sf_spec::{Compiled, GridDetail};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Longest side of a printed body part, in studs (64 mm, inside sf-custom's limit).
pub const MAX_SIDE: i32 = 8;
/// Most cells (columns) in one printed body part.
pub const MAX_CELLS: usize = 32;
/// Tallest printed body part, in plates (57.6 mm plus studs, inside sf-custom's limit).
pub const MAX_PLATES: i32 = 18;
/// Pieces this small are merged into a neighbour when the result still fits.
const TINY: usize = 3;

/// The output of [`pack_print`].
pub struct PrintPacked {
    /// Body parts then details, with studs, units and ids assigned.
    pub placements: Vec<Placement>,
    pub dropped: usize,
    pub anchored: usize,
    pub compiled: Option<Compiled>,
    pub warnings: Vec<String>,
}

/// Parts of one layer, as their columns.
type Pieces = Vec<Vec<(i32, i32)>>;

/// A body part while packing.
#[derive(Clone, Debug)]
struct Part {
    cols: Vec<(i32, i32)>,
    y: i32,
    plates: i32,
    color: ColorId,
}

struct Body<'a> {
    c: &'a Compiled,
    dims: [i32; 3],
    inner: Vec<ColorId>,
    owner: Vec<i32>,
    parts: Vec<Part>,
}

impl<'a> Body<'a> {
    fn new(c: &'a Compiled) -> Self {
        let dims = c.grid.dims;
        let n = (dims[0] * dims[1] * dims[2]) as usize;
        Body { c, dims, inner: nearest_surface_colors(c), owner: vec![-1; n], parts: Vec::new() }
    }

    fn idx(&self, x: i32, y: i32, z: i32) -> usize {
        ((y * self.dims[2] + z) * self.dims[0] + x) as usize
    }

    fn in_b(&self, x: i32, y: i32, z: i32) -> bool {
        x >= 0 && y >= 0 && z >= 0 && x < self.dims[0] && y < self.dims[1] && z < self.dims[2]
    }

    fn owner(&self, x: i32, y: i32, z: i32) -> Option<usize> {
        if !self.in_b(x, y, z) {
            return None;
        }
        let o = self.owner[self.idx(x, y, z)];
        (o >= 0).then_some(o as usize)
    }

    /// The colour a free, filled cell takes: its own, or (hidden) the nearest surface's.
    fn cell_color(&self, x: i32, y: i32, z: i32) -> Option<ColorId> {
        if !self.in_b(x, y, z) || self.owner(x, y, z).is_some() {
            return None;
        }
        match self.c.grid.get(Cell::new(x, y, z)) {
            Fill::Empty => None,
            Fill::Solid(col) => Some(col),
            Fill::Interior => Some(self.inner[self.idx(x, y, z)]),
        }
    }

    /// The colour of a brick column of the course at `y`: at least two of its three plates
    /// filled, none taken, and every visible one the same colour.
    fn brick_color(&self, x: i32, y: i32, z: i32) -> Option<ColorId> {
        let (mut filled, mut visible, mut hidden) = (0, None, None);
        for dy in 0..3 {
            if !self.in_b(x, y + dy, z) || self.owner(x, y + dy, z).is_some() {
                return None;
            }
            match self.c.grid.get(Cell::new(x, y + dy, z)) {
                Fill::Empty => {}
                Fill::Solid(col) => {
                    filled += 1;
                    match visible {
                        None => visible = Some(col),
                        Some(v) if v != col => return None,
                        _ => {}
                    }
                }
                Fill::Interior => {
                    filled += 1;
                    hidden.get_or_insert(self.inner[self.idx(x, y + dy, z)]);
                }
            }
        }
        if filled >= 2 { visible.or(hidden) } else { None }
    }

    fn add(&mut self, cols: Vec<(i32, i32)>, y: i32, plates: i32, color: ColorId) {
        let i = self.parts.len() as i32;
        for &(x, z) in &cols {
            for dy in 0..plates {
                let k = self.idx(x, y + dy, z);
                debug_assert!(self.owner[k] < 0, "overlap at ({x}, {}, {z})", y + dy);
                self.owner[k] = i;
            }
        }
        self.parts.push(Part { cols, y, plates, color });
    }

    /// Partitions one layer's columns (with their colours) into parts `plates` high.
    fn layer(&mut self, y: i32, plates: i32, cells: HashMap<(i32, i32), ColorId>, horizontal: bool) {
        if cells.is_empty() {
            return;
        }
        let below = |b: &Self, x: i32, z: i32| if y == 0 { None } else { b.owner(x, y - 1, z) };
        let pieces = partition(&cells, &|x, z| below(self, x, z), horizontal);
        for (cols, color) in pieces {
            self.add(cols, y, plates, color);
        }
    }

    fn pack(mut self) -> Vec<Part> {
        let dims = self.dims;
        // Course phase with the most brick columns (ties: bricks on the ground).
        let phase = (0..3)
            .max_by_key(|&p| {
                let mut count = 0;
                let mut y = p;
                while y + 2 < dims[1] {
                    for z in 0..dims[2] {
                        for x in 0..dims[0] {
                            if self.brick_color(x, y, z).is_some() {
                                count += 1;
                            }
                        }
                    }
                    y += 3;
                }
                count * 4 - p
            })
            .unwrap_or(0);
        let (mut y, mut course) = (0, 0);
        while y < dims[1] {
            let horizontal = course % 2 == 0;
            if y >= phase && (y - phase) % 3 == 0 && y + 2 < dims[1] {
                let mut cells = HashMap::new();
                for z in 0..dims[2] {
                    for x in 0..dims[0] {
                        if let Some(col) = self.brick_color(x, y, z) {
                            cells.insert((x, z), col);
                        }
                    }
                }
                self.layer(y, 3, cells, horizontal);
                for dy in 0..3 {
                    self.plate_layer(y + dy, horizontal ^ (dy % 2 == 1));
                }
                y += 3;
                course += 1;
            } else {
                self.plate_layer(y, horizontal);
                y += 1;
            }
        }
        merge_stacks(self.parts)
    }

    fn plate_layer(&mut self, y: i32, horizontal: bool) {
        let mut cells = HashMap::new();
        for z in 0..self.dims[2] {
            for x in 0..self.dims[0] {
                if let Some(col) = self.cell_color(x, y, z) {
                    cells.insert((x, z), col);
                }
            }
        }
        self.layer(y, 1, cells, horizontal);
    }
}

const NEIGHBOURS: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];

fn bbox(cols: &[(i32, i32)]) -> (i32, i32, i32, i32) {
    let x0 = cols.iter().map(|c| c.0).min().unwrap_or(0);
    let x1 = cols.iter().map(|c| c.0).max().unwrap_or(0);
    let z0 = cols.iter().map(|c| c.1).min().unwrap_or(0);
    let z1 = cols.iter().map(|c| c.1).max().unwrap_or(0);
    (x0, x1, z0, z1)
}

fn fits(cols: &[(i32, i32)]) -> bool {
    let (x0, x1, z0, z1) = bbox(cols);
    x1 - x0 < MAX_SIDE && z1 - z0 < MAX_SIDE && cols.len() <= MAX_CELLS
}

/// Connected components of `cells` under `same(a, b)`, each sorted, in scan order.
fn components(cells: &[(i32, i32)], same: impl Fn((i32, i32), (i32, i32)) -> bool) -> Vec<Vec<(i32, i32)>> {
    let set: HashSet<(i32, i32)> = cells.iter().copied().collect();
    let mut order = cells.to_vec();
    order.sort_by_key(|c| (c.1, c.0));
    let mut seen: HashSet<(i32, i32)> = HashSet::new();
    let mut out = Vec::new();
    for start in order {
        if !seen.insert(start) {
            continue;
        }
        let mut comp = vec![start];
        let mut stack = vec![start];
        while let Some(c) = stack.pop() {
            for (dx, dz) in NEIGHBOURS {
                let n = (c.0 + dx, c.1 + dz);
                if set.contains(&n) && same(c, n) && seen.insert(n) {
                    comp.push(n);
                    stack.push(n);
                }
            }
        }
        comp.sort_by_key(|c| (c.1, c.0));
        out.push(comp);
    }
    out
}

/// Splits a layer into parts: same-coloured regions, each cut into pieces that fit.
fn partition(
    cells: &HashMap<(i32, i32), ColorId>,
    below: &dyn Fn(i32, i32) -> Option<usize>,
    horizontal: bool,
) -> Vec<(Vec<(i32, i32)>, ColorId)> {
    let keys: Vec<(i32, i32)> = cells.keys().copied().collect();
    let regions = components(&keys, |a, b| cells[&a] == cells[&b]);
    let mut out = Vec::new();
    for region in regions {
        let color = cells[&region[0]];
        let pieces = if fits(&region) { vec![region] } else { cut_region(&region, below, horizontal) };
        out.extend(pieces.into_iter().map(|p| (p, color)));
    }
    out
}

/// The best tiling of a region too big for one part.
fn cut_region(region: &[(i32, i32)], below: &dyn Fn(i32, i32) -> Option<usize>, horizontal: bool) -> Vec<Vec<(i32, i32)>> {
    let (tw, td) = if horizontal { (MAX_SIDE, MAX_CELLS as i32 / MAX_SIDE) } else { (MAX_CELLS as i32 / MAX_SIDE, MAX_SIDE) };
    let mut best: Option<(i64, Pieces)> = None;
    for oz in 0..td {
        for ox in 0..tw {
            let tile = |c: (i32, i32)| ((c.0 - ox).div_euclid(tw), (c.1 - oz).div_euclid(td));
            let mut pieces = components(region, |a, b| tile(a) == tile(b));
            merge_tiny(&mut pieces);
            let s = score(&pieces, below);
            if best.as_ref().is_none_or(|b| s < b.0) {
                best = Some((s, pieces));
            }
        }
    }
    best.map(|b| b.1).unwrap_or_default()
}

/// Merges pieces of [`TINY`] cells or fewer into the neighbour they share most border with,
/// when the merged piece still fits.
fn merge_tiny(pieces: &mut Vec<Vec<(i32, i32)>>) {
    loop {
        let mut of: HashMap<(i32, i32), usize> = HashMap::new();
        for (i, p) in pieces.iter().enumerate() {
            for &c in p {
                of.insert(c, i);
            }
        }
        let mut merged = false;
        let mut order: Vec<usize> = (0..pieces.len()).filter(|&i| pieces[i].len() <= TINY).collect();
        order.sort_by_key(|&i| pieces[i].len());
        for t in order {
            let mut border: BTreeMap<usize, usize> = BTreeMap::new();
            for &c in &pieces[t] {
                for (dx, dz) in NEIGHBOURS {
                    if let Some(&j) = of.get(&(c.0 + dx, c.1 + dz))
                        && j != t
                    {
                        *border.entry(j).or_default() += 1;
                    }
                }
            }
            let pick = border
                .iter()
                .filter(|&(&j, _)| {
                    let mut all = pieces[j].clone();
                    all.extend(&pieces[t]);
                    fits(&all)
                })
                .max_by_key(|&(&j, &n)| (n, std::cmp::Reverse(pieces[j].len())))
                .map(|(&j, _)| j);
            if let Some(j) = pick {
                let moved = std::mem::take(&mut pieces[t]);
                pieces[j].extend(moved);
                merged = true;
                break;
            }
        }
        pieces.retain(|p| !p.is_empty());
        if !merged {
            return;
        }
    }
}

/// Lower is better: fewer parts, no seams over seams below, parts bridging parts below,
/// no tiny leftovers.
fn score(pieces: &[Vec<(i32, i32)>], below: &dyn Fn(i32, i32) -> Option<usize>) -> i64 {
    let mut of: HashMap<(i32, i32), usize> = HashMap::new();
    for (i, p) in pieces.iter().enumerate() {
        for &c in p {
            of.insert(c, i);
        }
    }
    let mut aligned = 0i64;
    for (&c, &i) in &of {
        for n in [(c.0 + 1, c.1), (c.0, c.1 + 1)] {
            if let Some(&j) = of.get(&n)
                && j != i
                && let (Some(a), Some(b)) = (below(c.0, c.1), below(n.0, n.1))
                && a != b
            {
                aligned += 1;
            }
        }
    }
    let mut bridges = 0i64;
    for p in pieces {
        let owners: HashSet<usize> = p.iter().filter_map(|&(x, z)| below(x, z)).collect();
        bridges += owners.len().min(4) as i64;
    }
    let tiny = pieces.iter().filter(|p| p.len() <= TINY).count() as i64;
    pieces.len() as i64 * 10 + aligned * 3 - bridges * 2 + tiny * 15
}

/// Merges parts stacked directly on a part with the same columns and colour.
fn merge_stacks(mut parts: Vec<Part>) -> Vec<Part> {
    for p in &mut parts {
        p.cols.sort_unstable();
    }
    let mut at: HashMap<(i32, Vec<(i32, i32)>), usize> = HashMap::new();
    for (i, p) in parts.iter().enumerate() {
        at.insert((p.y, p.cols.clone()), i);
    }
    let mut order: Vec<usize> = (0..parts.len()).collect();
    order.sort_by_key(|&i| parts[i].y);
    let mut gone = vec![false; parts.len()];
    for i in order {
        if gone[i] {
            continue;
        }
        loop {
            let key = (parts[i].y + parts[i].plates, parts[i].cols.clone());
            let Some(&j) = at.get(&key) else { break };
            if gone[j] || parts[j].color != parts[i].color || parts[i].plates + parts[j].plates > MAX_PLATES {
                break;
            }
            parts[i].plates += parts[j].plates;
            gone[j] = true;
        }
    }
    parts.into_iter().zip(gone).filter(|(_, g)| !g).map(|(p, _)| p).collect()
}

/// A body part as a printed placement (studs not yet assigned).
fn to_placement(p: &Part) -> Placement {
    let (x0, _, z0, _) = bbox(&p.cols);
    let mut cells: Vec<(u8, u8)> = p.cols.iter().map(|&(x, z)| ((x - x0) as u8, (z - z0) as u8)).collect();
    cells.sort_unstable();
    let def = CustomDef {
        id: String::new(),
        kind: CustomKind::Brick,
        cells: cells.clone(),
        plates: p.plates as u8,
        studs: Vec::new(),
        units: Vec::new(),
        sockets: cells,
    };
    Placement::printed(def, p.color, Cell::new(x0, p.y, z0), Rot(0))
}

fn is_brick(p: &Placement) -> bool {
    matches!(p.custom.as_deref().map(|c| &c.kind), Some(CustomKind::Brick))
}

/// Lays out every printed brick's studs for what sits on it (see the module docs). Bricks
/// must be placed at `Rot(0)`, so local columns are grid columns less the origin.
pub fn assign_studs(placements: &mut [Placement]) {
    let mut owner: HashMap<Cell, usize> = HashMap::new();
    // Columns over which a stud would hit a detail's body rather than its socket base.
    let mut no_stud: HashSet<Cell> = HashSet::new();
    for (i, p) in placements.iter().enumerate() {
        for c in p.cells() {
            owner.insert(c, i);
        }
        if is_detail(p) {
            let sockets: HashSet<(i32, i32)> = p.bottom_sockets().into_iter().collect();
            for c in p.cells().filter(|c| c.y == p.origin.y) {
                if !sockets.contains(&(c.x, c.z)) {
                    no_stud.insert(c);
                }
            }
        }
    }
    #[allow(clippy::needless_range_loop)] // reads all placements, then writes this one
    for i in 0..placements.len() {
        if !is_brick(&placements[i]) {
            continue;
        }
        let p = &placements[i];
        let top = p.top();
        let (ox, oz) = (p.origin.x, p.origin.z);
        let cols: Vec<(i32, i32)> = {
            let mut v: Vec<(i32, i32)> = p.cells().filter(|c| c.y == p.origin.y).map(|c| (c.x, c.z)).collect();
            v.sort_unstable();
            v.dedup();
            v.into_iter().filter(|&(x, z)| !no_stud.contains(&Cell::new(x, top, z))).collect()
        };
        let above = |x: i32, z: i32| owner.get(&Cell::new(x, top, z)).copied();
        let [fx, fz] = p.footprint();
        let mut best: Option<(usize, Pieces)> = None;
        for along_x in [fx >= fz, fx < fz] {
            let (units, flat) = stud_layout(&cols, along_x, above);
            let covered = flat.iter().filter(|&&(x, z)| above(x, z).is_some()).count();
            let s = covered * 10 + flat.len();
            if best.as_ref().is_none_or(|b| s < b.0) {
                best = Some((s, units));
            }
        }
        let units = best.map(|b| b.1).unwrap_or_default();
        let local: Vec<Vec<(u8, u8)>> =
            units.iter().map(|u| u.iter().map(|&(x, z)| ((x - ox) as u8, (z - oz) as u8)).collect()).collect();
        let def = placements[i].custom.as_mut().expect("a printed brick");
        def.studs = local.iter().flatten().copied().collect();
        def.studs.sort_unstable();
        def.units = local;
    }
}

/// Gives every printed brick an id per distinct geometry (columns, plates, units),
/// "B001", "B002", … bottom-up.
pub fn assign_ids(placements: &mut [Placement]) {
    let mut order: Vec<usize> = (0..placements.len()).filter(|&i| is_brick(&placements[i])).collect();
    order.sort_by_key(|&i| {
        let o = placements[i].origin;
        (o.y, o.z, o.x)
    });
    type Key = (Vec<(u8, u8)>, u8, Vec<Vec<(u8, u8)>>);
    let mut ids: HashMap<Key, String> = HashMap::new();
    for i in order {
        let def = placements[i].custom.as_ref().expect("a printed brick");
        let mut units = def.units.clone();
        units.sort();
        let key = (def.cells.clone(), def.plates, units);
        let n = ids.len() + 1;
        let id = ids.entry(key).or_insert_with(|| format!("B{n:03}")).clone();
        placements[i].custom.as_mut().expect("a printed brick").id = id.clone();
        placements[i].part = id;
    }
}

/// Details with no stud under any socket column (they will fall off), as "D001 (flower head)".
pub fn unsupported_details(placements: &[Placement]) -> Vec<String> {
    let mut studs: HashSet<(i32, i32, i32)> = HashSet::new();
    for p in placements {
        let top = p.top();
        for (x, z) in p.top_studs() {
            studs.insert((x, top, z));
        }
    }
    placements
        .iter()
        .filter(|p| is_detail(p) && p.origin.y > 0)
        .filter(|p| !p.bottom_sockets().iter().any(|&(x, z)| studs.contains(&(x, p.origin.y, z))))
        .map(|p| format!("{} at ({}, {}, {})", p.name(), p.origin.x, p.origin.y, p.origin.z))
        .collect()
}

/// Packs `c` into printed parts, rests the spec's details on it and lays out studs.
pub fn pack_print(
    c: &Compiled,
    uses: &[GridDetail],
    designs: &BTreeMap<String, DetailDesign>,
    repair: bool,
) -> Result<PrintPacked> {
    let mut owned: Option<Compiled> = None;
    let mut anchored = 0;
    for attempt in 0..3 {
        let cc = owned.as_ref().unwrap_or(c);
        let body: Vec<Placement> = Body::new(cc).pack().iter().map(to_placement).collect();
        let placed = details::place_details(uses, designs, &body)?;
        let mut ps = body;
        ps.extend(placed.placements);
        assign_studs(&mut ps);
        let small: Vec<Vec<usize>> = if repair {
            small_floating(&ps, ANCHOR_MAX).into_iter().filter(|g| !g.iter().any(|&i| is_detail(&ps[i]))).collect()
        } else {
            Vec::new()
        };
        if !small.is_empty()
            && attempt < 2
            && let Some(next) = anchor(cc, &ps, &small)
        {
            anchored += small.len();
            owned = Some(next);
            continue;
        }
        let drop: HashSet<usize> = small.iter().filter(|g| g.len() <= DROP_MAX).flatten().copied().collect();
        let dropped = drop.len();
        if dropped > 0 {
            let mut i = 0;
            ps.retain(|_| {
                let keep = !drop.contains(&i);
                i += 1;
                keep
            });
            assign_studs(&mut ps);
        }
        assign_ids(&mut ps);
        let mut warnings = placed.warnings;
        for d in unsupported_details(&ps) {
            warnings.push(format!(
                "detail {d} has no stud under its base and will fall off: give it a base at least two studs long along a row of the part below"
            ));
        }
        return Ok(PrintPacked { placements: ps, dropped, anchored, compiled: owned, warnings });
    }
    unreachable!("the last attempt always returns")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BuildOptions, build};
    use sf_core::{DetailMesh, DetailSource, Palette, print::fused_splits};
    use sf_spec::{CompileOptions, Spec, compile};

    fn compiled(json: &str, res: u32) -> Compiled {
        let s = Spec::from_json(json).unwrap();
        compile(&s, &Palette::builtin(), &CompileOptions { resolution: Some(res), image: None }).unwrap()
    }

    /// Every invariant print packing promises.
    fn check(c: &Compiled, p: &PrintPacked, what: &str) {
        let grid = p.compiled.as_ref().unwrap_or(c).grid.clone();
        let ps = &p.placements;
        let mut owner: HashMap<Cell, usize> = HashMap::new();
        for (i, pl) in ps.iter().enumerate() {
            for cell in pl.cells() {
                assert!(owner.insert(cell, i).is_none(), "{what}: overlap at {cell:?}");
                if let Fill::Solid(col) = grid.get(cell) {
                    assert_eq!(col, pl.color, "{what}: colour mismatch at {cell:?}");
                }
            }
        }
        let uncovered = grid.filled().filter(|(cell, _)| !owner.contains_key(cell)).count();
        assert!(uncovered == 0 || p.dropped > 0, "{what}: {uncovered} cells uncovered");
        assert!(fused_splits(ps).is_empty(), "{what}: a unit lies under an edge: {:?}", fused_splits(ps));
        type Geom = (Vec<(u8, u8)>, u8, Vec<Vec<(u8, u8)>>);
        let mut by_id: HashMap<String, Geom> = HashMap::new();
        for pl in ps.iter().filter(|p| is_brick(p)) {
            let d = pl.custom.as_ref().unwrap();
            let cols: Vec<(i32, i32)> = d.cells.iter().map(|&(x, z)| (x as i32, z as i32)).collect();
            assert_eq!(components(&cols, |_, _| true).len(), 1, "{what}: {} is not one piece", d.id);
            let [fx, fz] = pl.footprint();
            assert!(fx <= MAX_SIDE && fz <= MAX_SIDE && d.cells.len() <= MAX_CELLS, "{what}: {} too big", d.id);
            assert!(pl.height() <= MAX_PLATES);
            for u in &d.units {
                assert!(u.len() == 2 || u.len() == 3, "{what}: unit of {}", u.len());
                let along_x = u.windows(2).all(|w| w[1] == (w[0].0 + 1, w[0].1));
                let along_z = u.windows(2).all(|w| w[1] == (w[0].0, w[0].1 + 1));
                assert!(along_x || along_z, "{what}: unit {u:?} not a row");
                assert!(u.iter().all(|c| d.cells.contains(c)));
            }
            let mut units = d.units.clone();
            units.sort();
            let geom = (d.cells.clone(), d.plates, units);
            if let Some(prev) = by_id.insert(d.id.clone(), geom.clone()) {
                assert_eq!(prev, geom, "{what}: id {} names two shapes", d.id);
            }
        }
    }

    #[test]
    fn print_packing_invariants_on_random_specs() {
        let mut seed = 0x51ab_u64;
        let mut rnd = |lo: f32, hi: f32| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            lo + (hi - lo) * ((seed >> 33) as f32 / (1u64 << 31) as f32)
        };
        let colours = ["Red", "White", "Blue", "Yellow", "Black", "Orange", "Green"];
        for case in 0..40 {
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
            let res = 8 + (case % 4) as u32 * 6;
            let Ok(c) = compile(&spec, &Palette::builtin(), &CompileOptions { resolution: Some(res), image: None }) else { continue };
            let p = pack_print(&c, &[], &BTreeMap::new(), true).unwrap();
            check(&c, &p, &format!("case {case}\n{json}"));
        }
    }

    const SNOWMAN: &str = r#"{"title":"snowman","shapes":[
        {"shape":"ellipsoid","center":[0,4,0],"size":[8,8,8],"color":"White"},
        {"shape":"ellipsoid","center":[0,10,0],"size":[5.5,5.5,5.5],"color":"White"},
        {"shape":"cone","center":[0,10,-3.2],"size":[1,1,2],"axis":"z","rotate":[180,0,0],"color":"Orange"}
    ]}"#;

    #[test]
    fn printing_needs_far_fewer_parts() {
        let spec = Spec::from_json(SNOWMAN).unwrap();
        let catalog = build(&spec, &BuildOptions { resolution: Some(20), ..Default::default() }, None).unwrap();
        let printed = build(&spec, &BuildOptions { resolution: Some(20), print: true, ..Default::default() }, None).unwrap();
        let (a, b) = (catalog.report.parts, printed.report.parts);
        println!("snowman at 20 studs: {a} catalog parts, {b} printed parts\n{}", printed.report.to_text());
        assert!(b * 2 < a, "printed {b} vs catalog {a}");
        assert!(printed.model.printable);
        assert!(printed.report.printed.as_ref().is_some_and(|p| p.bricks == b && p.unique_bricks <= b));
        assert!(printed.analysis.overlaps.is_empty());
        let total: usize = printed.model.steps.iter().map(|s| s.len()).sum();
        assert_eq!(total, b);
    }

    #[test]
    fn a_thin_pole_becomes_one_tall_part() {
        // A 1 × 2 pole on a 4 × 4 slab: one tall part standing on a fused pair. (A 1 × 1 pole
        // can't be held at all: its stud would stand alone.)
        let c = compiled(
            r#"{"shapes":[{"shape":"box","center":[0,0.4,0],"size":[4,0.8,4],"color":"Green"},
            {"shape":"box","center":[0,2,0.5],"size":[2,2.4,1],"color":"Brown"}]}"#,
            4,
        );
        let p = pack_print(&c, &[], &BTreeMap::new(), true).unwrap();
        check(&c, &p, "pole");
        let brown = Palette::builtin().resolve("Brown").unwrap();
        let browns: Vec<&Placement> = p.placements.iter().filter(|q| q.color == brown).collect();
        let colours: Vec<ColorId> = p.placements.iter().map(|q| q.color).collect();
        assert_eq!(browns.len(), 1, "pole in {} parts; colours {colours:?}", browns.len());
        assert!(browns[0].height() >= 3);
        let a = crate::analyze(&sf_core::Model { placements: p.placements.clone(), ..Default::default() });
        assert!(a.floating.is_empty(), "the pole floats");
    }

    fn flower_design() -> DetailDesign {
        // A 2 × 1 base, one plate, sitting on the body; a box mesh stands in for petals.
        DetailDesign {
            name: "flower".into(),
            label: "flower head".into(),
            cells: vec![(0, 0), (1, 0)],
            plates: 1,
            sockets: vec![(0, 0), (1, 0)],
            studs: Vec::new(),
            mesh: DetailMesh { positions: vec![[0.0; 3]; 3], indices: vec![0, 1, 2] },
            source: DetailSource { source: serde_json::json!({"template": "flower_head"}) },
        }
    }

    #[test]
    fn a_detail_sits_on_fused_studs() {
        let json = r#"{"shapes":[{"shape":"box","center":[0,1,0],"size":[6,2,6],"color":"Green"}],
            "details":[{"design":"flower","at":[0.5,2,0.5],"turn":1,"color":"Red"},
                       {"design":"flower","at":[-2,2,-2],"color":"Yellow"}]}"#;
        let spec = Spec::from_json(json).unwrap();
        let mut designs = BTreeMap::new();
        designs.insert("flower".to_string(), flower_design());
        let opts = BuildOptions { resolution: Some(6), print: true, details: designs.clone(), ..Default::default() };
        let r = build(&spec, &opts, None).unwrap();
        println!("{}", r.report.to_text());
        let dets: Vec<&Placement> = r.model.placements.iter().filter(|p| details::is_detail(p)).collect();
        assert_eq!(dets.len(), 2);
        assert!(dets.iter().all(|d| d.part == "D001"));
        assert!(r.model.meshes.contains_key("flower") && r.model.details.contains_key("flower"));
        let s = r.report.printed.as_ref().unwrap();
        assert!(s.unsupported_details.is_empty(), "{:?}", s.unsupported_details);
        assert_eq!(s.details, 2);
        // Each detail rests on the top and every socket column has a fused stud under it.
        let top = r.model.placements.iter().filter(|p| !details::is_detail(p)).map(|p| p.top()).max().unwrap();
        for d in dets {
            assert_eq!(d.origin.y, top);
            for (x, z) in d.bottom_sockets() {
                let under = r.model.placements.iter().find(|p| p.top() == top && p.top_studs().contains(&(x, z)));
                assert!(under.is_some(), "no stud under socket ({x}, {z})");
            }
        }
        assert!(fused_splits(&r.model.placements).is_empty());
        // Unknown designs are refused with the list of known ones.
        let bad = Spec::from_json(&json.replace("\"design\":\"flower\",\"at\":[-2", "\"design\":\"tulip\",\"at\":[-2")).unwrap();
        let e = build(&bad, &opts, None).err().unwrap().to_string();
        assert!(e.contains("tulip") && e.contains("flower"), "{e}");
        // Catalog mode places details too.
        let cat = build(&spec, &BuildOptions { print: false, ..opts }, None).unwrap();
        assert_eq!(cat.model.placements.iter().filter(|p| details::is_detail(p)).count(), 2);
        assert!(cat.report.printed.as_ref().unwrap().unsupported_details.is_empty());
    }
}
