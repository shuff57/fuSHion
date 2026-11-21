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

use crate::curves;
use crate::details::{self, is_detail};
use crate::pack::{ANCHOR_MAX, DROP_MAX, anchor, nearest_surface_colors, small_floating};
use anyhow::Result;
use sf_core::print::stud_layout;
use sf_core::{Cell, ColorId, CustomDef, CustomKind, DetailDesign, DetailMesh, Fill, Placement, Rot};
use sf_spec::{Compiled, GridDetail};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Straight bricks: brick columns this close (in studs, 4-neighbour steps) to a course's
/// curved rim are packed as plates (see `Body::widen_rim`).
pub const RIM_BAND: i32 = 3;

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
    /// Display meshes of trimmed bricks, by `CustomDef::mesh` (`"trim:B007"`).
    pub meshes: BTreeMap<String, DetailMesh>,
    /// What curving did (print mode with curves only).
    pub curve_stats: Option<CurveStats>,
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
    /// Filled plates (of 3) a column needs to join a course as a brick column.
    min_fill: usize,
    dims: [i32; 3],
    inner: Vec<ColorId>,
    owner: Vec<i32>,
    parts: Vec<Part>,
}

impl<'a> Body<'a> {
    /// `min_fill`: filled plates (of 3) a column needs to join a course. 2 for plain bricks;
    /// 1 when bricks are trimmed to the true surface, which cuts the empty plates away again.
    /// (With 2, a sphere's rim leaves single-column plates that no fused stud can hold.)
    fn new(c: &'a Compiled, min_fill: usize) -> Self {
        let dims = c.grid.dims;
        let n = (dims[0] * dims[1] * dims[2]) as usize;
        Body { c, min_fill, dims, inner: nearest_surface_colors(c), owner: vec![-1; n], parts: Vec::new() }
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

    /// The colour of a brick column of the course at `y`: at least `min_fill` of its three
    /// plates filled, none taken, and every visible one the same colour.
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
        if filled >= self.min_fill { visible.or(hidden) } else { None }
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
                if self.min_fill == 2 {
                    self.widen_rim(y, &mut cells);
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

    /// Straight bricks: where the outline changes inside a course (a curve), the columns
    /// that don't make a brick leave a rim of single plates, often one column wide and
    /// joined only at corners, which fused studs can't hold. Hand the brick columns next to
    /// that rim to the plate layers too, so the rim is at least two columns wide: plates
    /// stacked with offset seams, stepping the arc a plate at a time.
    fn widen_rim(&self, y: i32, cells: &mut HashMap<(i32, i32), ColorId>) {
        let mut rim: HashSet<(i32, i32)> = HashSet::new();
        for z in 0..self.dims[2] {
            for x in 0..self.dims[0] {
                if !cells.contains_key(&(x, z)) && (0..3).any(|dy| self.cell_color(x, y + dy, z).is_some()) {
                    rim.insert((x, z));
                }
            }
        }
        let r = RIM_BAND;
        cells.retain(|&(x, z), _| {
            !(-r..=r).any(|dx: i32| (-r..=r).any(|dz: i32| dx.abs() + dz.abs() <= r && rim.contains(&(x + dx, z + dz))))
        });
        // Settle: every plate cell needs a plate of its colour beside it in its layer (the
        // band reaches across gaps, so a column it takes can end up alone), and every brick
        // column a brick of its colour beside it. Each round only moves bricks to plates.
        loop {
            let mut take: HashSet<(i32, i32)> = HashSet::new();
            for z in 0..self.dims[2] {
                for x in 0..self.dims[0] {
                    if let Some(col) = cells.get(&(x, z)) {
                        if !NEIGHBOURS.iter().any(|&(dx, dz)| cells.get(&(x + dx, z + dz)) == Some(col)) {
                            take.insert((x, z));
                        }
                        continue;
                    }
                    for dy in 0..3 {
                        let Some(col) = self.cell_color(x, y + dy, z) else { continue };
                        let plate_beside = NEIGHBOURS.iter().any(|&(dx, dz)| {
                            !cells.contains_key(&(x + dx, z + dz)) && self.cell_color(x + dx, y + dy, z + dz) == Some(col)
                        });
                        if !plate_beside
                            && let Some(&(dx, dz)) = NEIGHBOURS.iter().find(|&&(dx, dz)| cells.get(&(x + dx, z + dz)) == Some(&col))
                        {
                            take.insert((x + dx, z + dz));
                        }
                    }
                }
            }
            if take.is_empty() {
                break;
            }
            for c in take {
                cells.remove(&c);
            }
        }
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
            break;
        }
    }
    lend_cells(pieces);
}

/// A single cell no neighbour could take (it would grow past the size limit) borrows an
/// adjacent cell from a neighbour that stays connected and more than tiny without it: a
/// piece of two in a row can be held by a fused pair, a single cell can't.
fn lend_cells(pieces: &mut [Vec<(i32, i32)>]) {
    for t in 0..pieces.len() {
        if pieces[t].len() != 1 {
            continue;
        }
        let c = pieces[t][0];
        'n: for (dx, dz) in NEIGHBOURS {
            let n = (c.0 + dx, c.1 + dz);
            for j in 0..pieces.len() {
                if j == t || !pieces[j].contains(&n) || pieces[j].len() <= TINY + 1 {
                    continue;
                }
                let rest: Vec<(i32, i32)> = pieces[j].iter().copied().filter(|&q| q != n).collect();
                if components(&rest, |_, _| true).len() == 1 {
                    pieces[j] = rest;
                    pieces[t].push(n);
                    pieces[t].sort_by_key(|c| (c.1, c.0));
                    break 'n;
                }
            }
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
        colors: Default::default(),
        trim: None,
        mesh: None,
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
/// must be placed at `Rot(0)`, so local columns are grid columns less the origin. `meshes`
/// are the detail meshes by design name: a detail fills only what its mesh reaches.
///
/// With `exposed_studs` false (curved surfaces), columns with nothing on them get no stud,
/// so the top of the sculpture is smooth.
pub fn assign_studs(placements: &mut [Placement], meshes: &BTreeMap<String, DetailMesh>, exposed_studs: bool) {
    let map = StudMap::new(placements, meshes);
    for i in 0..placements.len() {
        if is_brick(&placements[i]) {
            layout_studs(placements, &map, i, exposed_studs, &|_| true);
        }
    }
}

/// Who fills each cell, and where a stud would hit a detail's body, for stud layout.
struct StudMap {
    owner: HashMap<Cell, usize>,
    /// Columns over which a stud would hit a detail's body rather than its socket base.
    no_stud: HashSet<Cell>,
}

impl StudMap {
    fn new(placements: &[Placement], meshes: &BTreeMap<String, DetailMesh>) -> Self {
        let mut owner: HashMap<Cell, usize> = HashMap::new();
        let mut no_stud: HashSet<Cell> = HashSet::new();
        for (i, p) in placements.iter().enumerate() {
            let cells = details::occupied_cells(p, meshes);
            if is_detail(p) {
                let sockets: HashSet<(i32, i32)> = p.bottom_sockets().into_iter().collect();
                for c in cells.iter().filter(|c| c.y == p.origin.y) {
                    if !sockets.contains(&(c.x, c.z)) {
                        no_stud.insert(*c);
                    }
                }
            }
            for c in cells {
                owner.insert(c, i);
            }
        }
        StudMap { owner, no_stud }
    }
}

/// Lays out brick `i`'s studs over the grid columns `allow` accepts (see [`assign_studs`]).
fn layout_studs(placements: &mut [Placement], map: &StudMap, i: usize, exposed_studs: bool, allow: &dyn Fn((i32, i32)) -> bool) {
    let p = &placements[i];
    let top = p.top();
    let (ox, oz) = (p.origin.x, p.origin.z);
    let cols: Vec<(i32, i32)> = {
        let mut v: Vec<(i32, i32)> = p.cells().filter(|c| c.y == p.origin.y).map(|c| (c.x, c.z)).collect();
        v.sort_unstable();
        v.dedup();
        v.into_iter()
            .filter(|&(x, z)| !map.no_stud.contains(&Cell::new(x, top, z)))
            .filter(|&(x, z)| exposed_studs || map.owner.contains_key(&Cell::new(x, top, z)))
            .filter(|&c| allow(c))
            .collect()
    };
    let above = |x: i32, z: i32| map.owner.get(&Cell::new(x, top, z)).copied();
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
    let local: Vec<Vec<(u8, u8)>> = units.iter().map(|u| u.iter().map(|&(x, z)| ((x - ox) as u8, (z - oz) as u8)).collect()).collect();
    let def = placements[i].custom.as_mut().expect("a printed brick");
    def.studs = local.iter().flatten().copied().collect();
    def.studs.sort_unstable();
    def.units = local;
}

/// What trimming contacts to the shape did (see [`curve_bricks`]).
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct CurveStats {
    /// Display meshes: body triangles, slivers, triangles cutting a crease (see
    /// [`curves::TrimmedMesh`]) and the worst distance off the surface, µm.
    pub mesh_triangles: usize,
    pub slivers: usize,
    pub chamfered: usize,
    pub max_off_um: usize,
    /// Bricks trimmed to the true surface.
    pub curved: usize,
    /// Stud contacts between two trimmed bricks, and how many `S` covers.
    pub contacts: usize,
    pub covered: usize,
    /// Contacts restored although `S` doesn't cover them, so no part is cut loose.
    pub restored: usize,
    /// Parts that needed contacts restored.
    pub parts_restored: usize,
    /// After restoring: parts that had a connection before contacts were trimmed and have
    /// none now, and parts floating now that weren't before. Both 0 unless no contact could
    /// be restored.
    pub cut_loose: usize,
    pub newly_floating: usize,
    /// Studs on body bricks before and after trimming contacts.
    pub studs_before: usize,
    pub studs_after: usize,
    /// Protect boxes before (every contact kept) and after.
    pub boxes_before: usize,
    pub boxes_after: usize,
    /// Of those, boxes reaching outside `S` grown by the contact margin (ledges).
    pub ledges_before: usize,
    pub ledges_after: usize,
    /// Ground columns of curved bricks, how many keep a flat pad (`S` doesn't reach the
    /// floor there), and how many stand on the floor at all.
    pub ground_columns: usize,
    pub pads: usize,
    pub ground_touching: usize,
    /// The base following the outline would have unbalanced the model, so it kept the
    /// plain bottoms.
    pub ground_fallback: bool,
}

/// Trims every exposed printed brick to the true surface, contacts included: a contact
/// between two trimmed bricks keeps its stud, socket and protect boxes only where `S`
/// covers them (see [`curves::contacts`]). Then, so no part is cut loose, a part left with
/// no connection (or newly floating) gets its best-covered contacts back until it is held
/// as well as before. Studs and sockets follow the kept contacts.
fn curve_bricks(ps: &mut [Placement], meshes: &BTreeMap<String, DetailMesh>, surface: &[sf_spec::SurfaceShape]) -> CurveStats {
    let n = ps.len();
    let occ = curves::Occupancy::new(ps, meshes);
    let trimmed: Vec<bool> = ps.iter().enumerate().map(|(i, p)| is_brick(p) && curves::is_exposed(i, p, &occ)).collect();
    let solid = curves::Solid::new(surface);
    let contacts = curves::contacts(ps, &occ, &trimmed, &solid);
    let by_lower: HashMap<(usize, (i32, i32)), usize> = contacts.iter().enumerate().map(|(k, c)| ((c.lower, c.col), k)).collect();
    let by_upper: HashMap<(usize, (i32, i32)), usize> = contacts.iter().enumerate().map(|(k, c)| ((c.upper, c.col), k)).collect();
    let studs = |ps: &[Placement]| ps.iter().filter(|p| is_brick(p)).map(|p| p.top_studs().len()).sum::<usize>();
    let mut stats = CurveStats {
        curved: 0,
        contacts: contacts.iter().filter(|c| !c.fixed).count(),
        covered: contacts.iter().filter(|c| !c.fixed && c.covered).count(),
        studs_before: studs(ps),
        ..Default::default()
    };
    let analysis =
        |ps: &[Placement]| crate::analyze(&sf_core::Model { placements: ps.to_vec(), meshes: meshes.clone(), ..Default::default() });
    let linked = |a: &crate::Analysis, ps: &[Placement]| -> Vec<bool> {
        let mut l: Vec<bool> = ps.iter().map(|p| p.origin.y == 0).collect();
        for &(a, b, _) in &a.connections {
            l[a] = true;
            l[b] = true;
        }
        l
    };
    let base = analysis(ps);
    let base_linked = linked(&base, ps);
    let base_floating: HashSet<usize> = base.floating.iter().copied().collect();
    let map = StudMap::new(ps, meshes);
    // How each ground column of a curved brick meets the floor.
    let ground_cols: Vec<(usize, (i32, i32))> = (0..n)
        .filter(|&i| trimmed[i] && ps[i].origin.y == 0)
        .flat_map(|i| {
            ps[i].custom.as_ref().map(|d| d.cells.iter().map(|&(x, z)| (i, (x as i32, z as i32))).collect::<Vec<_>>()).unwrap_or_default()
        })
        .collect();
    let ps_ref: &[Placement] = ps;
    let mut feet: HashMap<(usize, (i32, i32)), curves::GroundFoot> =
        ground_cols.iter().copied().zip(curves::par_map(&ground_cols, |&(i, c)| curves::ground_foot(&solid, &ps_ref[i], c))).collect();
    stats.ground_columns = feet.len();
    let mut keep: Vec<bool> = contacts.iter().map(|c| c.fixed || c.covered).collect();
    let lowers: Vec<usize> = (0..n).filter(|&i| is_brick(&ps[i]) && contacts.iter().any(|c| c.lower == i && !c.fixed)).collect();
    let mut restored_parts: HashSet<usize> = HashSet::new();
    for _round in 0..(4 * contacts.len() + 1) {
        // Studs over kept contacts only; sockets of trimmed bricks over studs (or the ground).
        for &i in &lowers {
            let allow = |c: (i32, i32)| by_lower.get(&(i, c)).is_none_or(|&k| keep[k]);
            layout_studs(ps, &map, i, false, &allow);
        }
        let stud_at: HashSet<(i32, i32, i32)> =
            ps.iter().flat_map(|p| p.top_studs().into_iter().map(move |(x, z)| (x, p.top(), z))).collect();
        for j in (0..n).filter(|&j| trimmed[j]) {
            let o = ps[j].origin;
            let def = ps[j].custom.as_mut().expect("a printed brick");
            let socket = |&(x, z): &(u8, u8)| {
                if o.y == 0 {
                    feet.get(&(j, (x as i32, z as i32))).is_none_or(|f| f.touches)
                } else {
                    stud_at.contains(&(o.x + x as i32, o.y, o.z + z as i32))
                }
            };
            def.sockets = def.cells.iter().filter(|c| socket(c)).copied().collect();
        }
        let a = analysis(ps);
        let now = linked(&a, ps);
        let floating: HashSet<usize> = a.floating.iter().copied().collect();
        let loose: Vec<usize> = (0..n).filter(|&k| trimmed[k] && base_linked[k] && !now[k]).collect();
        let fresh: Vec<usize> = (0..n).filter(|k| floating.contains(k) && !base_floating.contains(k)).collect();
        stats.cut_loose = loose.len();
        stats.newly_floating = fresh.len();
        let victims = if loose.is_empty() { fresh } else { loose };
        let mut progress = false;
        for &v in &victims {
            // The victim's best dropped contact, preferring one to a part that is held.
            let best = (0..contacts.len()).filter(|&k| !keep[k] && (contacts[k].lower == v || contacts[k].upper == v)).max_by(|&a, &b| {
                let key = |k: usize| {
                    let other = if contacts[k].lower == v { contacts[k].upper } else { contacts[k].lower };
                    (!floating.contains(&other), contacts[k].score)
                };
                let (ka, kb) = (key(a), key(b));
                ka.0.cmp(&kb.0).then(ka.1.total_cmp(&kb.1))
            });
            if let Some(k) = best {
                keep[k] = true;
                stats.restored += 1;
                restored_parts.insert(v);
                progress = true;
            }
        }
        if !progress && !victims.is_empty() {
            // Its own contacts are all back, yet it isn't held: dropping other columns changed
            // how a neighbour's studs fuse. Give the bricks it touches every contact back, so
            // their studs are laid out as before trimming.
            let touching: HashSet<usize> = contacts
                .iter()
                .filter(|c| victims.contains(&c.lower) || victims.contains(&c.upper))
                .flat_map(|c| [c.lower, c.upper])
                .collect();
            for (k, c) in contacts.iter().enumerate() {
                if !keep[k] && touching.contains(&c.lower) {
                    keep[k] = true;
                    stats.restored += 1;
                    restored_parts.insert(c.upper);
                    progress = true;
                }
            }
        }
        if !progress {
            break;
        }
    }
    // The base must still balance: if following the outline tips it, keep plain bottoms.
    if base.balance.ok && !analysis(ps).balance.ok {
        stats.ground_fallback = true;
        for (&(i, c), f) in feet.iter_mut() {
            *f = curves::GroundFoot { boxes: curves::plain_bottom(&ps[i], c), touches: true, pad: false };
        }
        for &(i, _) in &ground_cols {
            let def = ps[i].custom.as_mut().expect("a printed brick");
            def.sockets = def.cells.clone();
        }
    }
    stats.pads = feet.values().filter(|f| f.pad).count();
    stats.ground_touching = feet.values().filter(|f| f.touches).count();
    stats.parts_restored = restored_parts.len();
    stats.studs_after = studs(ps);
    // Trims: protect only contacts that stayed (fixed ones, or with a stud and socket).
    let occ = curves::Occupancy::new(ps, meshes);
    let is_trimmed = |j: usize| trimmed[j];
    let ids: Vec<usize> = (0..n).filter(|&i| trimmed[i]).collect();
    let ps_ref: &[Placement] = ps;
    let results = curves::par_map(&ids, |&i| {
        let ps = ps_ref;
        let mut stats = CurveStats::default();
        let p = &ps[i];
        let o = p.origin;
        let studs: HashSet<(u8, u8)> = p.custom.as_ref().map(|d| d.studs.iter().copied().collect()).unwrap_or_default();
        let sockets: HashSet<(u8, u8)> = p.custom.as_ref().map(|d| d.sockets.iter().copied().collect()).unwrap_or_default();
        let local = |c: (i32, i32)| (c.0 as u8, c.1 as u8);
        let top =
            |c: (i32, i32)| by_lower.get(&(i, (o.x + c.0, o.z + c.1))).is_none_or(|&k| contacts[k].fixed || studs.contains(&local(c)));
        let bottom =
            |c: (i32, i32)| by_upper.get(&(i, (o.x + c.0, o.z + c.1))).is_none_or(|&k| contacts[k].fixed || sockets.contains(&local(c)));
        let all = |_: (i32, i32)| true;
        let plain = |c: (i32, i32)| curves::plain_bottom(p, c);
        let ground = |c: (i32, i32)| feet.get(&(i, c)).map(|f| f.boxes.clone()).unwrap_or_default();
        let before =
            curves::brick_trim(i, p, &occ, surface, &is_trimmed, &curves::KeptContacts { top: &all, bottom: &all, ground: &plain });
        let after =
            curves::brick_trim(i, p, &occ, surface, &is_trimmed, &curves::KeptContacts { top: &top, bottom: &bottom, ground: &ground });
        // Kept boxes are a subset of the full set: test each box once.
        let outside: HashMap<String, bool> = before
            .iter()
            .flat_map(|t| t.protect.iter())
            .map(|b| (format!("{b:?}"), solid.reaches_outside(p, b, curves::CONTACT_MARGIN)))
            .collect();
        let ledges = |t: &sf_core::Trim| t.protect.iter().filter(|b| outside.get(&format!("{b:?}")).copied().unwrap_or(false)).count();
        if let Some(t) = &before {
            stats.boxes_before += t.protect.len();
            stats.ledges_before += ledges(t);
        }
        if let Some(t) = &after {
            stats.boxes_after += t.protect.len();
            stats.ledges_after += ledges(t);
            stats.curved += 1;
        }
        (after, stats)
    });
    for (&i, (t, s)) in ids.iter().zip(results) {
        stats.boxes_before += s.boxes_before;
        stats.boxes_after += s.boxes_after;
        stats.ledges_before += s.ledges_before;
        stats.ledges_after += s.ledges_after;
        stats.curved += s.curved;
        if let (Some(def), Some(t)) = (ps[i].custom.as_mut(), t) {
            def.trim = Some(Box::new(t));
        }
    }
    stats
}

/// Gives every printed brick an id per distinct geometry (columns, plates, units),
/// "B001", "B002", … bottom-up.
pub fn assign_ids(placements: &mut [Placement]) {
    let mut order: Vec<usize> = (0..placements.len()).filter(|&i| is_brick(&placements[i])).collect();
    order.sort_by_key(|&i| {
        let o = placements[i].origin;
        (o.y, o.z, o.x)
    });
    type Key = (Vec<(u8, u8)>, u8, Vec<Vec<(u8, u8)>>, Vec<(u8, u8)>, String);
    let mut ids: HashMap<Key, String> = HashMap::new();
    for i in order {
        let def = placements[i].custom.as_ref().expect("a printed brick");
        let mut units = def.units.clone();
        units.sort();
        let trim = def.trim.as_ref().map(|t| serde_json::to_string(t).unwrap_or_default()).unwrap_or_default();
        let mut sockets = def.sockets.clone();
        sockets.sort_unstable();
        let key = (def.cells.clone(), def.plates, units, sockets, trim);
        let n = ids.len() + 1;
        let id = ids.entry(key).or_insert_with(|| format!("B{n:03}")).clone();
        placements[i].custom.as_mut().expect("a printed brick").id = id.clone();
        placements[i].part = id;
    }
}

/// Builds one display mesh per distinct trimmed brick (in parallel), sets each trimmed
/// brick's `mesh` key and overhang flag, and returns the meshes by key.
fn mesh_trims(ps: &mut [Placement], quality: &mut CurveStats) -> BTreeMap<String, DetailMesh> {
    let mut jobs: BTreeMap<String, (CustomDef, sf_core::Trim)> = BTreeMap::new();
    for p in ps.iter() {
        if let Some(def) = p.custom.as_deref()
            && let Some(t) = &def.trim
        {
            jobs.entry(def.id.clone()).or_insert_with(|| (def.clone(), (**t).clone()));
        }
    }
    let jobs: Vec<(String, CustomDef, sf_core::Trim)> = jobs.into_iter().map(|(k, (d, t))| (k, d, t)).collect();
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(jobs.len().max(1));
    let mut results: Vec<(String, curves::TrimmedMesh)> = Vec::with_capacity(jobs.len());
    std::thread::scope(|s| {
        let chunks: Vec<&[(String, CustomDef, sf_core::Trim)]> = jobs.chunks(jobs.len().div_ceil(threads).max(1)).collect();
        let handles: Vec<_> = chunks
            .into_iter()
            .map(|chunk| s.spawn(move || chunk.iter().map(|(k, d, t)| (k.clone(), curves::trimmed_mesh(d, t))).collect::<Vec<_>>()))
            .collect();
        for h in handles {
            results.extend(h.join().expect("mesh thread panicked"));
        }
    });
    let mut meshes = BTreeMap::new();
    let mut overhang: HashMap<String, bool> = HashMap::new();
    for (id, m) in results {
        // Quality over every placed copy.
        let copies = ps.iter().filter(|p| p.custom.as_ref().is_some_and(|d| d.id == id)).count();
        quality.mesh_triangles += m.triangles * copies;
        quality.slivers += m.slivers * copies;
        quality.chamfered += m.chamfered * copies;
        quality.max_off_um = quality.max_off_um.max((m.max_off_mm * 1000.0).round() as usize);
        overhang.insert(id.clone(), m.overhang);
        meshes.insert(format!("trim:{id}"), m.mesh);
    }
    for p in ps.iter_mut() {
        if let Some(def) = p.custom.as_mut()
            && let Some(t) = def.trim.as_mut()
        {
            t.overhang = overhang.get(&def.id).copied().unwrap_or(false);
            def.mesh = Some(format!("trim:{}", def.id));
        }
    }
    meshes
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

/// With curves, an empty cell joins the model when at least this many of its 18 samples
/// are inside the shape (22 %): the trim cuts it back to the true outline, so a curve that
/// bulges past a cell's centre isn't left flat at the cell's edge.
pub const GROW_SAMPLES: u32 = 4;

/// The grid grown for trimmed bricks (see [`GROW_SAMPLES`]): every empty cell that the
/// shape covers enough of and that touches a filled cell in its layer joins, in that
/// neighbour's colour. `None` when nothing joins. Cells outside the grid (cropped away)
/// can't join.
fn grow_to_shape(c: &Compiled, min_samples: u32) -> Option<Compiled> {
    let [dx, dy, dz] = c.grid.dims;
    let mut grid = c.grid.clone();
    let mut added = 0;
    for y in 0..dy {
        for z in 0..dz {
            for x in 0..dx {
                let cell = Cell::new(x, y, z);
                if c.grid.get(cell) != Fill::Empty || c.sample_mask(cell).count_ones() < min_samples {
                    continue;
                }
                // Same layer only: a bulge past a cell's edge. (Up and down, the course rule
                // already takes a column with one plate filled; a plate grown above a shape's
                // top would make a loose sliver.)
                let colour = [(1, 0, 0), (-1, 0, 0), (0, 0, 1), (0, 0, -1)].iter().find_map(|&(ox, oy, oz)| {
                    let n = Cell::new(x + ox, y + oy, z + oz);
                    match c.grid.in_bounds(n).then(|| c.grid.get(n)) {
                        Some(Fill::Solid(col)) => Some(col),
                        _ => None,
                    }
                });
                if let Some(col) = colour {
                    grid.set(cell, Fill::Solid(col));
                    added += 1;
                }
            }
        }
    }
    if added == 0 {
        return None;
    }
    grid.mark_interior();
    Some(Compiled { grid, ..c.clone() })
}

/// Un-grows cells that pack into pieces no stud can hold: after packing `grown`, every grown
/// cell in a one-column part, or next to one in its layer, goes back to empty (the trim
/// leaves that bit of curve flat instead). Repeats until the packing settles (3 rounds).
fn keep_holdable(c: &Compiled, mut grown: Compiled) -> Compiled {
    let is_grown = |g: &Compiled, cell: Cell| c.grid.get(cell) == Fill::Empty && g.grid.get(cell) != Fill::Empty;
    for _ in 0..3 {
        let parts = Body::new(&grown, 1).pack();
        let mut undo: Vec<Cell> = Vec::new();
        for part in parts.iter().filter(|p| p.cols.len() == 1) {
            let (x, z) = part.cols[0];
            for y in part.y..part.y + part.plates {
                for (dx, dz) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let cell = Cell::new(x + dx, y, z + dz);
                    if grown.grid.in_bounds(cell) && is_grown(&grown, cell) {
                        undo.push(cell);
                    }
                }
            }
        }
        if undo.is_empty() {
            break;
        }
        for cell in undo {
            grown.grid.set(cell, Fill::Empty);
        }
    }
    grown
}

/// Re-shapes small loose groups (at most [`DROP_MAX`] parts) so they join a held part
/// beside them instead of being removed. For each part of a group, the first held part `q`
/// beside it in its layer whose height range covers the part's: the part's column is filled
/// out to `q`'s range (only where those cells are empty) in `q`'s colour, so it joins `q`.
/// With nothing beside it, a one-column piece grows a second column instead.
///
/// Returns the new grid and how many groups changed, or `None`.
fn rescue(c: &Compiled, ps: &[Placement], groups: &[Vec<usize>]) -> Option<(Compiled, usize)> {
    let mut owner: HashMap<Cell, usize> = HashMap::new();
    for (i, p) in ps.iter().enumerate() {
        for cell in p.cells() {
            owner.insert(cell, i);
        }
    }
    let loose: HashSet<usize> = groups.iter().flatten().copied().collect();
    let mut out = c.clone();
    let mut changed = 0;
    for g in groups.iter().filter(|g| g.len() <= DROP_MAX) {
        let mut done = false;
        'parts: for &i in g {
            let p = &ps[i];
            if !is_brick(p) {
                continue;
            }
            for cell in p.cells() {
                for (dx, dz) in NEIGHBOURS {
                    let Some(&j) = owner.get(&Cell::new(cell.x + dx, cell.y, cell.z + dz)) else { continue };
                    let q = &ps[j];
                    if loose.contains(&j) || !is_brick(q) {
                        continue;
                    }
                    let (lo, hi) = (q.origin.y, q.top());
                    if lo > p.origin.y || hi < p.top() {
                        continue;
                    }
                    // The rest of this column within q's range must be free to fill.
                    let fill: Vec<Cell> =
                        (lo..hi).map(|y| Cell::new(cell.x, y, cell.z)).filter(|c2| c2.y < p.origin.y || c2.y >= p.top()).collect();
                    if !fill.iter().all(|&c2| out.grid.in_bounds(c2) && !out.grid.get(c2).is_filled() && !owner.contains_key(&c2)) {
                        continue;
                    }
                    for c2 in fill.into_iter().chain(p.cells()) {
                        out.grid.set(c2, Fill::Solid(q.color));
                    }
                    done = true;
                    break 'parts;
                }
            }
        }
        if !done {
            // Nothing beside it to join: a one-column piece grows a second column over the
            // same part below (or under the same part above), where the shape covers most, so
            // a fused pair can hold it.
            for &i in g {
                let p = &ps[i];
                if !is_brick(p) || p.custom.as_ref().is_none_or(|d| d.cells.len() != 1) {
                    continue;
                }
                let (x, z) = (p.origin.x, p.origin.z);
                let support = |y: i32, x: i32, z: i32| owner.get(&Cell::new(x, y, z)).copied().filter(|j| !loose.contains(j));
                let pick = [(p.origin.y - 1, p.origin.y > 0), (p.top(), true)].into_iter().filter(|&(_, ok)| ok).find_map(|(sy, _)| {
                    let l = support(sy, x, z)?;
                    NEIGHBOURS
                        .iter()
                        .map(|&(dx, dz)| (x + dx, z + dz))
                        .filter(|&(nx, nz)| support(sy, nx, nz) == Some(l))
                        .filter(|&(nx, nz)| {
                            (p.origin.y..p.top()).all(|y| {
                                let c2 = Cell::new(nx, y, nz);
                                out.grid.in_bounds(c2) && !out.grid.get(c2).is_filled() && !owner.contains_key(&c2)
                            })
                        })
                        .max_by_key(|&(nx, nz)| {
                            (p.origin.y..p.top()).map(|y| c.sample_mask(Cell::new(nx, y, nz)).count_ones()).sum::<u32>()
                        })
                });
                if let Some((nx, nz)) = pick {
                    for y in p.origin.y..p.top() {
                        out.grid.set(Cell::new(nx, y, nz), Fill::Solid(p.color));
                    }
                    done = true;
                    break;
                }
            }
        }
        changed += done as usize;
    }
    (changed > 0).then_some((out, changed))
}

/// Packs `c` into printed parts, rests the spec's details on it and lays out studs.
pub fn pack_print(
    c: &Compiled,
    uses: &[GridDetail],
    designs: &BTreeMap<String, DetailDesign>,
    repair: bool,
    curves: bool,
) -> Result<PrintPacked> {
    let mut anchored = 0;
    let meshes = details::design_meshes(designs);
    let curves = curves && !c.surface.is_empty();
    let mut owned: Option<Compiled> = if curves { grow_to_shape(c, GROW_SAMPLES).map(|g| keep_holdable(c, g)) } else { None };
    let mut rescued = 0;
    for attempt in 0..5 {
        let cc = owned.as_ref().unwrap_or(c);
        let body: Vec<Placement> = Body::new(cc, if curves { 1 } else { 2 }).pack().iter().map(to_placement).collect();
        let placed = details::place_details(uses, designs, &body)?;
        let mut ps = body;
        ps.extend(placed.placements);
        // A bouquet's hub sits on the vase top like a detail, before studs are laid out.
        let mut hub_warning = None;
        if let Some(b) = c.bouquet.as_ref() {
            match crate::bouquet::plan_hub(b, designs, &ps, &crate::bouquet::hub_id(&ps), !repair) {
                Ok(plan) => ps.push(plan.placement),
                Err(e) => hub_warning = Some(e),
            }
        }
        assign_studs(&mut ps, &meshes, !curves);
        let small: Vec<Vec<usize>> = if repair {
            small_floating(&ps, ANCHOR_MAX).into_iter().filter(|g| !g.iter().any(|&i| is_detail(&ps[i]))).collect()
        } else {
            Vec::new()
        };
        if !small.is_empty() && attempt < 4 {
            // First anchor (a short gap filled, or a hidden cell recoloured); then re-shape what
            // is still loose before anything is removed.
            if attempt < 2
                && let Some(next) = anchor(cc, &ps, &small)
            {
                anchored += small.len();
                owned = Some(next);
                continue;
            }
            if let Some((next, n)) = rescue(cc, &ps, &small) {
                rescued += n;
                owned = Some(next);
                continue;
            }
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
            assign_studs(&mut ps, &meshes, !curves);
        }
        let curve_stats = curves.then(|| curve_bricks(&mut ps, &meshes, &cc.surface));
        assign_ids(&mut ps);
        let mut curve_stats = curve_stats;
        let trim_meshes = match curve_stats.as_mut() {
            Some(q) => mesh_trims(&mut ps, q),
            None => BTreeMap::new(),
        };
        let mut warnings = placed.warnings;
        warnings.extend(hub_warning);
        if rescued > 0 {
            warnings.push(format!(
                "re-shaped {rescued} small piece(s) no stud could hold: each joined the part beside it (filled out to its height, in its colour) or grew a second stud's width"
            ));
        }
        for d in unsupported_details(&ps) {
            warnings.push(format!(
                "detail {d} has no stud under its base and will fall off: give it a base at least two studs long along a row of the part below"
            ));
        }
        return Ok(PrintPacked { placements: ps, meshes: trim_meshes, dropped, anchored, compiled: owned, warnings, curve_stats });
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
                    rnd(-4.0, 4.0),
                    rnd(0.0, 8.0),
                    rnd(-4.0, 4.0),
                    rnd(1.0, 9.0),
                    rnd(1.0, 9.0),
                    rnd(1.0, 9.0)
                ));
            }
            let json = format!(r#"{{"shapes":[{}]}}"#, shapes.join(","));
            let Ok(spec) = Spec::from_json(&json) else { continue };
            let res = 8 + (case % 4) as u32 * 6;
            let Ok(c) = compile(&spec, &Palette::builtin(), &CompileOptions { resolution: Some(res), image: None }) else { continue };
            let p = pack_print(&c, &[], &BTreeMap::new(), true, false).unwrap();
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
        let p = pack_print(&c, &[], &BTreeMap::new(), true, false).unwrap();
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
            kit: None,
            mount: None,
            stem: None,
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

    /// Axis-aligned boxes as one mesh (local LDU frame), each `[x0, x1, y0, y1, z0, z1]`.
    fn boxes(bs: &[[f32; 6]]) -> DetailMesh {
        let mut m = DetailMesh::default();
        for b in bs {
            let base = m.positions.len() as u32;
            for k in 0..8 {
                m.positions.push([b[k & 1], b[2 + ((k >> 1) & 1)], b[4 + ((k >> 2) & 1)]]);
            }
            for f in [[0, 1, 3, 2], [4, 6, 7, 5], [0, 4, 5, 1], [2, 3, 7, 6], [0, 2, 6, 4], [1, 5, 7, 3]] {
                m.indices.extend([base + f[0], base + f[1], base + f[2], base + f[0], base + f[2], base + f[3]]);
            }
        }
        m
    }

    /// A leaf: a 2 × 1 socket base at local row 0, a blade rising over rows 1 and 2
    /// (6 plates tall box: rows 1 and 2 reach down only to plates 1 and 3).
    fn leaf_design() -> DetailDesign {
        DetailDesign {
            name: "leaf".into(),
            label: "leaf".into(),
            cells: vec![(0, 0), (1, 0), (0, 1), (1, 1), (0, 2), (1, 2)],
            plates: 6,
            sockets: vec![(0, 0), (1, 0)],
            studs: Vec::new(),
            mesh: boxes(&[
                [-20.0, 20.0, 40.0, 48.0, -30.0, -10.0],
                [-19.0, 19.0, 16.0, 40.0, -9.0, 9.0],
                [-19.0, 19.0, 0.0, 24.0, 11.0, 29.0],
            ]),
            kit: None,
            mount: None,
            stem: None,
            source: DetailSource { source: serde_json::json!({"template": "leaf"}) },
        }
    }

    #[test]
    fn a_leaf_rests_on_the_soil_inside_a_pot_rim() {
        // Soil 6 × 6 studs, 3 plates; a rim one brick higher along the back row (grid z 5).
        // The leaf's base sits on soil row 3, its blade rises over rows 4 and 5: the part
        // over the rim starts 3 plates up, clear of it.
        let json = r#"{"shapes":[
            {"shape":"box","center":[0,0.6,0],"size":[6,1.2,6],"color":"Dark Brown"},
            {"shape":"box","center":[0,1.8,2.5],"size":[6,1.2,1],"color":"Reddish Brown"}],
            "details":[{"design":"leaf","at":[0,1.2,0.5],"color":"Green"}]}"#;
        let spec = Spec::from_json(json).unwrap();
        let mut designs = BTreeMap::new();
        designs.insert("leaf".to_string(), leaf_design());
        let opts = BuildOptions { resolution: Some(6), print: true, details: designs, ..Default::default() };
        let r = build(&spec, &opts, None).unwrap();
        let text = r.report.to_text();
        println!("{text}");
        let leaf = r.model.placements.iter().find(|p| details::is_detail(p)).expect("the leaf was skipped");
        assert_eq!(leaf.origin.y, 3, "the leaf should rest on the soil, not float: {:?}", leaf.origin);
        assert!(!text.contains("overlap") && !text.contains("no stud"), "{text}");
        assert!(r.analysis.overlaps.is_empty(), "{:?}", r.analysis.overlaps);
        assert!(r.report.printed.as_ref().unwrap().unsupported_details.is_empty());
        // Its blade really clears the rim: no occupied cell of the leaf is a body cell.
        let body: HashSet<Cell> = r.model.placements.iter().filter(|p| !details::is_detail(p)).flat_map(|p| p.cells()).collect();
        let occ = details::occupied_cells(leaf, &r.model.meshes);
        assert!(occ.iter().all(|c| !body.contains(c)));
        assert!(occ.iter().any(|c| c.z == 5 && c.y >= 6), "the blade over the rim is missing: {occ:?}");
        // The coarse box would have hit the rim.
        assert!(leaf.cells().any(|c| body.contains(&c)));
    }

    #[test]
    fn detail_colour_roles_and_kit_parts() {
        use sf_core::{Kit, KitPart, KitPose};
        let pose = KitPose { rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], position: [0.0; 3] };
        let part = |key: &str, role: &str, n: usize| KitPart {
            key: key.into(),
            label: key.replace('_', " "),
            role: role.into(),
            mesh: DetailMesh::default(),
            source: DetailSource { source: serde_json::json!({}) },
            supports: false,
            copies: vec![pose; n],
        };
        let mut design = flower_design();
        design.kit = Some(Kit {
            parts: vec![part("petal", "main", 5), part("centre", "centre", 1)],
            steps: Vec::new(),
            roles: vec![("main".into(), "Red".into()), ("centre".into(), "Yellow".into())],
        });
        let mut designs = BTreeMap::new();
        designs.insert("flower".to_string(), design);
        let json = r#"{"shapes":[{"shape":"box","center":[0,1,0],"size":[6,2,6],"color":"Green"}],
            "details":[{"design":"flower","at":[0.5,2,0.5],"color":"Red","colors":{"centre":"Orange"}},
                       {"design":"flower","at":[-2,2,-2],"color":"Yellow"}]}"#;
        let opts = BuildOptions { resolution: Some(6), print: true, details: designs, ..Default::default() };
        let r = build(&Spec::from_json(json).unwrap(), &opts, None).unwrap();
        let dets: Vec<&Placement> = r.model.placements.iter().filter(|p| details::is_detail(p)).collect();
        assert_eq!(dets.len(), 2);
        let orange = Palette::builtin().resolve("Orange").unwrap();
        assert!(dets.iter().any(|d| d.custom.as_ref().unwrap().colors.get("centre") == Some(&orange)));
        assert!(dets.iter().any(|d| d.custom.as_ref().unwrap().colors.is_empty()));
        // Labels never name a colour: both uses read "flower head".
        assert!(dets.iter().all(|d| d.name().contains("flower head") && !d.name().contains("Red") && !d.name().contains("Yellow")));
        assert!(r.model.kits.contains_key("flower"));
        let s = r.report.printed.as_ref().unwrap();
        assert_eq!(s.kit_parts, 12);
        assert_eq!(s.total, s.bricks + 12);
        assert!(r.report.to_text().contains("built from 12 kit parts"), "{}", r.report.to_text());
        // An unknown role is refused with the design's roles.
        let bad = json.replace("\"centre\":\"Orange\"", "\"stem\":\"Green\"");
        let e = build(&Spec::from_json(&bad).unwrap(), &opts, None).err().unwrap().to_string();
        assert!(e.contains("stem") && e.contains("centre"), "{e}");
    }

    #[test]
    fn straight_bricks_hold_every_part_of_round_shapes() {
        let pot = r#"{"shapes":[
            {"shape":"cylinder","center":[0,3,0],"size":[8,6,8],"color":"Reddish Brown"},
            {"shape":"cylinder","center":[0,6.3,0],"size":[9,1,9],"color":"Reddish Brown"},
            {"shape":"cylinder","center":[0,6.6,0],"size":[7.5,0.6,7.5],"color":"Dark Brown"}]}"#;
        for (what, json, res) in [("snowman", SNOWMAN, 21), ("pot", pot, 12)] {
            let r =
                build(&Spec::from_json(json).unwrap(), &BuildOptions { resolution: Some(res), print: true, ..Default::default() }, None)
                    .unwrap();
            let text = r.report.to_text();
            println!("{what}: {} parts\n{text}", r.report.parts);
            assert!(!text.contains("removed"), "{what}: parts removed\n{text}");
            assert!(r.analysis.floating.is_empty(), "{what}");
            // At most the odd sideways feature (the snowman's nose) is anchored.
            let anchored = r
                .report
                .warnings
                .iter()
                .find(|w| w.contains("small feature"))
                .map_or(0, |w| w.split(' ').next().unwrap().parse::<usize>().unwrap());
            assert!(anchored <= 2, "{what}: {anchored} anchored");
            assert!(fused_splits(&r.model.placements).is_empty(), "{what}");
            assert!(r.model.placements.iter().all(|p| p.custom.as_ref().is_some_and(|d| d.trim.is_none())), "{what}: straight bricks only");
        }
    }
}
