//! Detail parts from the spec's `details`: each rests on the body under its socket base.
//!
//! A detail's `cells × plates` is a coarse box: a tilted leaf's blade sweeps up from its
//! base and may hang over a pot's rim. So the cells a detail really fills are worked out
//! per column from its mesh ([`column_floors`]): in each column, from the lowest plate the
//! mesh reaches up to the top of the box. Socket columns always fill from the bottom.

use anyhow::{Result, bail};
use sf_core::{Cell, ColorId, CustomKind, DetailDesign, DetailMesh, Model, Placement, Rot};
use sf_spec::GridDetail;
use std::collections::{BTreeMap, HashMap, HashSet};

/// Sample spacing on a mesh's triangles, in LDU (an eighth of a plate is 1 LDU).
const SAMPLE_LDU: f32 = 2.0;

/// For each local column `(lx, lz)` of a design that its mesh reaches, the lowest plate
/// (counted from the part's bottom) the mesh reaches in it. Socket columns are 0. Columns
/// the mesh never enters are left out: the part doesn't fill them.
///
/// The mesh frame is the one `Placement::ldraw_transform` uses: LDU, `-Y` up, origin at
/// the top centre of the local bounding box (`size × plates`).
pub fn column_floors(cells: &[(u8, u8)], plates: u8, sockets: &[(u8, u8)], mesh: &DetailMesh) -> HashMap<(u8, u8), u8> {
    let sx = cells.iter().map(|c| c.0 + 1).max().unwrap_or(1) as f32;
    let sz = cells.iter().map(|c| c.1 + 1).max().unwrap_or(1) as f32;
    let h = plates as f32 * 8.0;
    let known: HashSet<(u8, u8)> = cells.iter().copied().collect();
    // Lowest point (largest LDU y) per column.
    let mut low: HashMap<(u8, u8), f32> = HashMap::new();
    let mut put = |p: [f32; 3]| {
        let lx = ((p[0] + sx * 10.0) / 20.0).floor();
        let lz = ((p[2] + sz * 10.0) / 20.0).floor();
        if lx < 0.0 || lz < 0.0 || lx >= sx || lz >= sz {
            return;
        }
        let col = (lx as u8, lz as u8);
        if known.contains(&col) {
            let e = low.entry(col).or_insert(f32::MIN);
            *e = e.max(p[1]);
        }
    };
    let pos = &mesh.positions;
    for t in mesh.indices.as_chunks::<3>().0 {
        let (Some(&a), Some(&b), Some(&c)) = (pos.get(t[0] as usize), pos.get(t[1] as usize), pos.get(t[2] as usize)) else {
            continue;
        };
        let len = |p: [f32; 3], q: [f32; 3]| ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt();
        let n = ((len(a, b).max(len(b, c)).max(len(c, a)) / SAMPLE_LDU).ceil() as usize).clamp(1, 128);
        for i in 0..=n {
            for j in 0..=(n - i) {
                let (u, v) = (i as f32 / n as f32, j as f32 / n as f32);
                put([
                    a[0] + (b[0] - a[0]) * u + (c[0] - a[0]) * v,
                    a[1] + (b[1] - a[1]) * u + (c[1] - a[1]) * v,
                    a[2] + (b[2] - a[2]) * u + (c[2] - a[2]) * v,
                ]);
            }
        }
    }
    let mut out: HashMap<(u8, u8), u8> = HashMap::new();
    for (col, y) in low {
        // Height above the bottom of the lowest point; a hair of slack so a face lying
        // exactly on a plate boundary doesn't claim the plate below.
        let above = (h - y).max(0.0);
        let floor = ((above + 0.01) / 8.0).floor() as i32;
        if floor < plates as i32 {
            out.insert(col, floor.max(0) as u8);
        }
    }
    for &s in sockets {
        out.insert(s, 0);
    }
    out
}

/// The cells a placement really fills: a detail with a mesh in `meshes`, per column from
/// its mesh (see [`column_floors`]); anything else, all of [`Placement::cells`].
pub fn occupied_cells(p: &Placement, meshes: &BTreeMap<String, DetailMesh>) -> Vec<Cell> {
    let Some(def) = p.custom.as_deref() else { return p.cells().collect() };
    let CustomKind::Detail { mesh, .. } = &def.kind else { return p.cells().collect() };
    let Some(m) = meshes.get(mesh) else { return p.cells().collect() };
    let floors = column_floors(&def.cells, def.plates, &def.sockets, m);
    let mut out = Vec::new();
    for (&(lx, lz), &f) in &floors {
        let (x, z) = p.local_to_grid(lx, lz);
        for k in f as i32..def.plates as i32 {
            out.push(Cell::new(x, p.origin.y + k, z));
        }
    }
    out.sort();
    out
}

/// The meshes of `designs`, by name.
pub fn design_meshes(designs: &BTreeMap<String, DetailDesign>) -> BTreeMap<String, DetailMesh> {
    designs.iter().map(|(k, d)| (k.clone(), d.mesh.clone())).collect()
}

/// How far above the requested height (in plates) a surface may be and still count as
/// "at or below" it, so a detail asked for just under the top of a course still lands on it.
const ABOVE_TOLERANCE: f32 = 1.5;

/// Details placed on a body, and what went wrong with the rest.
pub struct PlacedDetails {
    pub placements: Vec<Placement>,
    pub warnings: Vec<String>,
}

/// Fails with a message listing the available designs when a use names an unknown one,
/// or the design's colour roles when a use sets a role the design doesn't have.
pub fn check_designs(uses: &[GridDetail], designs: &BTreeMap<String, DetailDesign>) -> Result<()> {
    for u in uses {
        let Some(d) = designs.get(&u.design) else {
            let have: Vec<&str> = designs.keys().map(String::as_str).collect();
            bail!(
                "detail design `{}` doesn't exist; available: {}",
                u.design,
                if have.is_empty() { "none yet (make one first)".to_string() } else { have.join(", ") }
            );
        };
        let roles: Vec<&str> = d.kit.as_ref().map(|k| k.roles.iter().map(|(r, _)| r.as_str()).collect()).unwrap_or_default();
        for role in u.colors.keys() {
            if !roles.contains(&role.as_str()) {
                bail!(
                    "detail design `{}` has no colour role `{role}`; its roles are: main (the detail's `color`){}",
                    u.design,
                    roles.iter().filter(|r| **r != "main").map(|r| format!(", {r}")).collect::<String>()
                );
            }
        }
    }
    Ok(())
}

/// The id of each used design ("D001", … in order of first use).
fn ids(uses: &[GridDetail]) -> HashMap<&str, String> {
    let mut out: HashMap<&str, String> = HashMap::new();
    for u in uses {
        let n = out.len() + 1;
        out.entry(u.design.as_str()).or_insert_with(|| format!("D{n:03}"));
    }
    out
}

/// Rests each detail on the highest surface of `body` under its socket columns at or
/// below its requested height (or on the ground where nothing is under it). Details that
/// would overlap the body or an earlier detail are skipped with a warning.
pub fn place_details(
    uses: &[GridDetail],
    designs: &BTreeMap<String, DetailDesign>,
    body: &[Placement],
) -> Result<PlacedDetails> {
    check_designs(uses, designs)?;
    let ids = ids(uses);
    let meshes = design_meshes(designs);
    // What fills each cell, for the overlap message.
    let mut taken: HashMap<Cell, String> = HashMap::new();
    for p in body {
        for c in occupied_cells(p, &meshes) {
            taken.insert(c, format!("body part {}", p.part));
        }
    }
    // Occupied heights per column.
    let mut columns: HashMap<(i32, i32), Vec<i32>> = HashMap::new();
    for c in taken.keys() {
        columns.entry((c.x, c.z)).or_default().push(c.y);
    }
    let mut placements = Vec::new();
    let mut warnings = Vec::new();
    for u in uses {
        let design = &designs[&u.design];
        let id = &ids[u.design.as_str()];
        let rot = Rot(u.turn % 4);
        // Socket columns relative to an origin at (0, 0, 0).
        let probe = Placement::printed(design.instance(id), u.color, Cell::new(0, 0, 0), rot);
        let sockets = probe.bottom_sockets();
        if sockets.is_empty() {
            warnings.push(format!("detail `{}` has no socket base; skipped", u.design));
            continue;
        }
        let n = sockets.len() as f32;
        let cx = sockets.iter().map(|s| s.0 as f32 + 0.5).sum::<f32>() / n;
        let cz = sockets.iter().map(|s| s.1 as f32 + 0.5).sum::<f32>() / n;
        let (ox, oz) = ((u.at[0] - cx).round() as i32, (u.at[2] - cz).round() as i32);
        // The surface under each socket column at or below the requested height.
        let mut y = 0;
        let mut on_body = false;
        for &(sx, sz) in &sockets {
            let col = (ox + sx, oz + sz);
            let Some(ys) = columns.get(&col) else { continue };
            let set: HashSet<i32> = ys.iter().copied().collect();
            let mut surfaces: Vec<i32> = ys.iter().map(|y| y + 1).filter(|top| !set.contains(top)).collect();
            surfaces.sort_unstable();
            let pick = surfaces
                .iter()
                .rev()
                .find(|&&top| top as f32 <= u.at[1] + ABOVE_TOLERANCE)
                .or_else(|| surfaces.first())
                .copied();
            if let Some(top) = pick {
                y = y.max(top);
                on_body = true;
            }
        }
        let mut def = design.instance(id);
        def.colors = u.colors.clone();
        let p = Placement::printed(def, u.color, Cell::new(ox, y, oz), rot);
        let cells = occupied_cells(&p, &meshes);
        if let Some((c, what)) = cells.iter().find_map(|c| taken.get(c).map(|w| (c, w))) {
            warnings.push(format!(
                "detail `{}` at ({:.1}, {:.1}, {:.1}) would overlap {what} at cell ({}, {}, {}); skipped. Move it apart{}",
                u.design,
                u.at[0],
                u.at[1],
                u.at[2],
                c.x,
                c.y,
                c.z,
                if what.starts_with("body") { " or make the body there lower" } else { "" }
            ));
            continue;
        }
        if !on_body && y == 0 {
            warnings.push(format!(
                "detail `{}` at ({:.1}, {:.1}, {:.1}) has nothing under it; it stands on the ground",
                u.design, u.at[0], u.at[1], u.at[2]
            ));
        }
        for c in cells {
            taken.insert(c, format!("detail `{}` ({id})", u.design));
            columns.entry((c.x, c.z)).or_default().push(c.y);
        }
        placements.push(p);
    }
    Ok(PlacedDetails { placements, warnings })
}

/// Copies the meshes, print sources and kits of every design `model` uses into it.
pub fn attach_designs(model: &mut Model, designs: &BTreeMap<String, DetailDesign>) {
    for p in &model.placements {
        if let Some(CustomKind::Detail { mesh, .. }) = p.custom.as_ref().map(|c| &c.kind)
            && let Some(d) = designs.get(mesh)
        {
            model.meshes.entry(mesh.clone()).or_insert_with(|| d.mesh.clone());
            model.details.entry(mesh.clone()).or_insert_with(|| d.source.clone());
            if let Some(kit) = &d.kit {
                model.kits.entry(mesh.clone()).or_insert_with(|| kit.clone());
            }
        }
    }
}

/// Is this placement a detail part?
pub fn is_detail(p: &Placement) -> bool {
    matches!(p.custom.as_deref().map(|c| &c.kind), Some(sf_core::CustomKind::Detail { .. }))
}

/// Colour helper for tests and callers: a use at grid position `at`.
pub fn grid_use(design: &str, at: [f32; 3], turn: u8, color: ColorId) -> GridDetail {
    GridDetail { design: design.to_string(), at, turn, color, colors: BTreeMap::new() }
}
