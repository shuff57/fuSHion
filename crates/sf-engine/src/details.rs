//! Detail parts from the spec's `details`: each rests on the body under its socket base.

use anyhow::{Result, bail};
use sf_core::{Cell, ColorId, DetailDesign, Model, Placement, Rot};
use sf_spec::GridDetail;
use std::collections::{BTreeMap, HashMap, HashSet};

/// How far above the requested height (in plates) a surface may be and still count as
/// "at or below" it, so a detail asked for just under the top of a course still lands on it.
const ABOVE_TOLERANCE: f32 = 1.5;

/// Details placed on a body, and what went wrong with the rest.
pub struct PlacedDetails {
    pub placements: Vec<Placement>,
    pub warnings: Vec<String>,
}

/// Fails with a message listing the available designs when a use names an unknown one.
pub fn check_designs(uses: &[GridDetail], designs: &BTreeMap<String, DetailDesign>) -> Result<()> {
    for u in uses {
        if !designs.contains_key(&u.design) {
            let have: Vec<&str> = designs.keys().map(String::as_str).collect();
            bail!(
                "detail design `{}` doesn't exist; available: {}",
                u.design,
                if have.is_empty() { "none yet (make one first)".to_string() } else { have.join(", ") }
            );
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
    let mut taken: HashSet<Cell> = body.iter().flat_map(|p| p.cells().collect::<Vec<_>>()).collect();
    // Occupied heights per column.
    let mut columns: HashMap<(i32, i32), Vec<i32>> = HashMap::new();
    for c in &taken {
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
        let p = Placement::printed(design.instance(id), u.color, Cell::new(ox, y, oz), rot);
        let cells: Vec<Cell> = p.cells().collect();
        if let Some(c) = cells.iter().find(|c| taken.contains(c)) {
            warnings.push(format!(
                "detail `{}` at ({:.1}, {:.1}, {:.1}) would overlap the model at cell ({}, {}, {}); skipped. Move it or make the body there lower",
                u.design, u.at[0], u.at[1], u.at[2], c.x, c.y, c.z
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
            taken.insert(c);
            columns.entry((c.x, c.z)).or_default().push(c.y);
        }
        placements.push(p);
    }
    Ok(PlacedDetails { placements, warnings })
}

/// Copies the meshes and print sources of every design `model` uses into it.
pub fn attach_designs(model: &mut Model, designs: &BTreeMap<String, DetailDesign>) {
    for p in &model.placements {
        if let Some(sf_core::CustomKind::Detail { mesh, .. }) = p.custom.as_ref().map(|c| &c.kind)
            && let Some(d) = designs.get(mesh)
        {
            model.meshes.entry(mesh.clone()).or_insert_with(|| d.mesh.clone());
            model.details.entry(mesh.clone()).or_insert_with(|| d.source.clone());
        }
    }
}

/// Is this placement a detail part?
pub fn is_detail(p: &Placement) -> bool {
    matches!(p.custom.as_deref().map(|c| &c.kind), Some(sf_core::CustomKind::Detail { .. }))
}

/// Colour helper for tests and callers: a use at grid position `at`.
pub fn grid_use(design: &str, at: [f32; 3], turn: u8, color: ColorId) -> GridDetail {
    GridDetail { design: design.to_string(), at, turn, color }
}
