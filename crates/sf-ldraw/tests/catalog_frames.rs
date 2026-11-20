//! Checks that every catalog part's `ldraw_yaw` / `ldraw_offset` / `size`
//! put the real LDraw geometry exactly on the grid footprint the catalog
//! claims, for all four rotations, and that slopes face the right way.
//!
//! Skipped (with a note) when the LDraw library isn't available.

use sf_core::{CATALOG, Cell, PartDef, Placement, Rot, Shape};
use sf_ldraw::{MeshLibrary, PartMesh, procedural_mesh};

const EPS: f32 = 0.5;
const STUD_H: f32 = 4.0;
const STUD_R: f32 = 6.0;

fn library() -> Option<MeshLibrary> {
    match sf_ldraw::ensure_installed().and_then(|d| MeshLibrary::open(&d)) {
        Ok(lib) => Some(lib),
        Err(e) => {
            eprintln!("NOTE: skipping LDraw frame check, library not available: {e:#}");
            None
        }
    }
}

/// World (LDraw) vertices of `mesh` placed at the origin with rotation `k`,
/// paired with their coordinates in the part's rotation-0 frame relative to
/// the footprint centre (`local`, LDraw axes).
fn place(def: &PartDef, mesh: &PartMesh, k: u8) -> (Placement, Vec<[f32; 3]>, Vec<[f32; 3]>) {
    let p = Placement::new(def.id, 4, Cell::new(0, 0, 0), Rot(k));
    let (pos, m) = p.ldraw_transform();
    let [fx, fz] = p.footprint();
    let (cx, cz) = (fx as f32 * 10.0, fz as f32 * 10.0);
    let r = Rot(k).matrix();
    let mut world = Vec::new();
    let mut local = Vec::new();
    for v in &mesh.positions {
        let w = [
            pos[0] + m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
            pos[1] + m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
            pos[2] + m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
        ];
        let d = [w[0] - cx, w[1], w[2] - cz];
        // Inverse of the placement rotation (transpose).
        local.push([
            r[0][0] * d[0] + r[1][0] * d[1] + r[2][0] * d[2],
            d[1],
            r[0][2] * d[0] + r[1][2] * d[1] + r[2][2] * d[2],
        ]);
        world.push(w);
    }
    (p, world, local)
}

/// All problems found for one part / rotation.
fn check(def: &PartDef, mesh: &PartMesh, k: u8) -> Vec<String> {
    let mut errs = Vec::new();
    let (p, world, local) = place(def, mesh, k);
    let [fx, fz] = p.footprint();
    let (fxl, fzl) = (fx as f32 * 20.0, fz as f32 * 20.0);
    let top = -(def.height as f32) * 8.0;
    let studs: Vec<(f32, f32)> =
        p.top_studs().iter().map(|&(x, z)| (x as f32 * 20.0 + 10.0, z as f32 * 20.0 + 10.0)).collect();

    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for w in &world {
        for i in 0..3 {
            lo[i] = lo[i].min(w[i]);
            hi[i] = hi[i].max(w[i]);
        }
    }
    if lo[0] < -EPS || hi[0] > fxl + EPS || lo[2] < -EPS || hi[2] > fzl + EPS {
        errs.push(format!("footprint x[{:.1},{:.1}] z[{:.1},{:.1}], want x[0,{fxl}] z[0,{fzl}]", lo[0], hi[0], lo[2], hi[2]));
    }
    if (lo[0].abs() > EPS || (hi[0] - fxl).abs() > EPS || lo[2].abs() > EPS || (hi[2] - fzl).abs() > EPS) && errs.is_empty() {
        errs.push(format!("footprint not filled: x[{:.1},{:.1}] z[{:.1},{:.1}]", lo[0], hi[0], lo[2], hi[2]));
    }
    if hi[1].abs() > EPS {
        errs.push(format!("bottom at y={:.1}, want 0", hi[1]));
    }
    if lo[1] < top - STUD_H - EPS {
        errs.push(format!("geometry above the studs: min y={:.1}, top {top}", lo[1]));
    }
    // Body top must reach y = top (tolerate LDraw's rounded cheese-slope tops).
    let body_top = world.iter().map(|w| w[1]).filter(|&y| y >= top - EPS).fold(f32::MAX, f32::min);
    if (body_top - top).abs() > 0.5 && !(def.is_slope() && (body_top - top).abs() < 0.5 + 0.5) {
        errs.push(format!("body top at y={body_top:.1}, want {top}"));
    }
    // Anything above the body top must be a stud on a column we claim has one.
    for w in world.iter().filter(|w| w[1] < top - EPS) {
        let ok = studs.iter().any(|&(sx, sz)| ((w[0] - sx).powi(2) + (w[2] - sz).powi(2)).sqrt() <= STUD_R + EPS);
        if !ok {
            errs.push(format!("stud geometry at ({:.1},{:.1},{:.1}) not over a stud column {:?}", w[0], w[1], w[2], p.top_studs()));
            break;
        }
    }

    let hx = def.size[0] as f32 * 10.0;
    match def.shape {
        Shape::Slope { high } => {
            let xb = -hx + high as f32 * 20.0;
            // The full-height top only exists over the high columns.
            if high > 0 {
                if let Some(l) = local.iter().find(|l| (l[1] - top).abs() < EPS && l[0] > xb + EPS) {
                    errs.push(format!("slope top surface at local x={:.1} beyond the high columns (x<{xb})", l[0]));
                }
            } else {
                // Cheese slope: the highest point is on the low-x side.
                let peak = local.iter().min_by(|a, b| a[1].total_cmp(&b[1])).unwrap();
                if peak[0] > 0.0 {
                    errs.push(format!("cheese slope peaks at local x={:.1}, want < 0", peak[0]));
                }
            }
            // Falls toward +x: at the +x end everything is well below the top.
            let far_top = local.iter().filter(|l| l[0] > hx - EPS).map(|l| l[1]).fold(f32::MAX, f32::min);
            if far_top < top + 4.0 {
                errs.push(format!("slope +x end reaches y={far_top:.1}; it should fall toward +x"));
            }
        }
        Shape::InvertedSlope { high } => {
            let xb = -hx + high as f32 * 20.0;
            if let Some(l) = local.iter().find(|l| l[1].abs() < EPS && l[0] > xb + EPS) {
                errs.push(format!("inverted slope bottom at local x={:.1} beyond the high columns (x<{xb})", l[0]));
            }
            // Flat top over the whole length.
            let far_top = local.iter().filter(|l| l[0] > hx - EPS).map(|l| l[1]).fold(f32::MAX, f32::min);
            if (far_top - top).abs() > EPS {
                errs.push(format!("inverted slope +x end top at y={far_top:.1}, want {top}"));
            }
        }
        _ => {}
    }
    errs
}

#[test]
fn catalog_frames_match_ldraw() {
    let Some(lib) = library() else { return };
    let mut failures = Vec::new();
    for def in CATALOG {
        let Some(mesh) = lib.mesh(def.id) else {
            failures.push(format!("{}: no mesh", def.id));
            continue;
        };
        for k in 0..4 {
            for e in check(def, &mesh, k) {
                failures.push(format!("{} ({}) rot {k}: {e}", def.id, def.name));
            }
        }
    }
    assert!(lib.missing_files().is_empty(), "missing LDraw files: {:?}", lib.missing_files());
    assert!(failures.is_empty(), "{} frame problems:\n{}", failures.len(), failures.join("\n"));
}

/// The procedural stand-ins obey the same frame contract (no library needed).
#[test]
fn procedural_frames_match_catalog() {
    let mut failures = Vec::new();
    for def in CATALOG {
        let mesh = procedural_mesh(def);
        for k in 0..4 {
            for e in check(def, &mesh, k) {
                failures.push(format!("{} rot {k}: {e}", def.id));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
