//! Simple stand-in meshes generated from a [`PartDef`] when no LDraw library
//! is available.
//!
//! Geometry is built in the catalog's rotation-0 frame relative to the
//! top-centre of the body (LDraw axes, -Y up) and then mapped into the LDraw
//! part frame by inverting the part's `ldraw_yaw` / `ldraw_offset`, so
//! `Placement::ldraw_transform()` places procedural and real meshes the same.

use crate::parse::{MAIN_COLOR, PartMesh};
use sf_core::{PartDef, Rot, Shape};

const STUD_R: f32 = 6.0;
const STUD_H: f32 = 4.0;
const STUD_SEGMENTS: usize = 12;
/// Vertical lip at the low end of a slope / high end of an inverted slope.
const LIP: f32 = 4.0;

/// Build a procedural mesh for a catalog part.
pub fn procedural_mesh(def: &PartDef) -> PartMesh {
    let hx = def.size[0] as f32 * 10.0;
    let hz = def.size[1] as f32 * 10.0;
    let h = def.height as f32 * 8.0;
    // Cross-section in the (x, y) plane, convex, extruded along z.
    let profile: Vec<[f32; 2]> = match def.shape {
        Shape::Box | Shape::Tile => vec![[-hx, 0.0], [hx, 0.0], [hx, h], [-hx, h]],
        Shape::Slope { high } => {
            let xb = -hx + high as f32 * 20.0;
            let low = (h - LIP).max(0.0);
            if high == 0 {
                vec![[-hx, 0.0], [hx, low], [hx, h], [-hx, h]]
            } else {
                vec![[-hx, 0.0], [xb, 0.0], [hx, low], [hx, h], [-hx, h]]
            }
        }
        Shape::InvertedSlope { high } => {
            let xb = -hx + high as f32 * 20.0;
            vec![[-hx, 0.0], [hx, 0.0], [hx, LIP.min(h)], [xb, h], [-hx, h]]
        }
    };

    let mut m = PartMesh::default();
    prism(&mut m, &profile, hz);
    for lz in 0..def.size[1] {
        for lx in 0..def.size[0] {
            if def.stud_on_top(lx, lz) {
                let cx = -hx + 10.0 + 20.0 * lx as f32;
                let cz = -hz + 10.0 + 20.0 * lz as f32;
                stud(&mut m, cx, cz);
            }
        }
    }
    to_part_frame(&mut m, def);
    m
}

/// Extrude a convex (x, y) polygon along z ∈ [-hz, hz].
fn prism(m: &mut PartMesh, poly: &[[f32; 2]], hz: f32) {
    let n = poly.len();
    let p = |i: usize, z: f32| [poly[i % n][0], poly[i % n][1], z];
    for i in 0..n {
        let (a0, a1, b0, b1) = (p(i, -hz), p(i + 1, -hz), p(i, hz), p(i + 1, hz));
        m.push_quad(a0, a1, b1, b0, MAIN_COLOR);
        m.edges.push([a0, a1]);
        m.edges.push([b0, b1]);
        m.edges.push([a0, b0]);
    }
    for i in 1..n - 1 {
        m.push_tri(p(0, -hz), p(i + 1, -hz), p(i, -hz), MAIN_COLOR);
        m.push_tri(p(0, hz), p(i, hz), p(i + 1, hz), MAIN_COLOR);
    }
}

fn stud(m: &mut PartMesh, cx: f32, cz: f32) {
    let ring = |k: usize| {
        let a = k as f32 / STUD_SEGMENTS as f32 * std::f32::consts::TAU;
        (cx + STUD_R * a.cos(), cz + STUD_R * a.sin())
    };
    let top = [cx, -STUD_H, cz];
    for k in 0..STUD_SEGMENTS {
        let (x0, z0) = ring(k);
        let (x1, z1) = ring(k + 1);
        m.push_quad([x0, 0.0, z0], [x1, 0.0, z1], [x1, -STUD_H, z1], [x0, -STUD_H, z0], MAIN_COLOR);
        m.push_tri(top, [x0, -STUD_H, z0], [x1, -STUD_H, z1], MAIN_COLOR);
        m.edges.push([[x0, -STUD_H, z0], [x1, -STUD_H, z1]]);
        m.edges.push([[x0, 0.0, z0], [x1, 0.0, z1]]);
    }
}

/// Map from the top-centre rotation-0 frame into the LDraw part frame:
/// `v_part = yawᵀ · (v - offset)`.
fn to_part_frame(m: &mut PartMesh, def: &PartDef) {
    let r = Rot(def.ldraw_yaw).matrix();
    let o = def.ldraw_offset.map(|v| v as f32);
    let map = |v: [f32; 3]| {
        let d = [v[0] - o[0], v[1] - o[1], v[2] - o[2]];
        [
            r[0][0] * d[0] + r[1][0] * d[1] + r[2][0] * d[2],
            r[0][1] * d[0] + r[1][1] * d[1] + r[2][1] * d[2],
            r[0][2] * d[0] + r[1][2] * d[1] + r[2][2] * d[2],
        ]
    };
    let rot = |v: [f32; 3]| {
        [
            r[0][0] * v[0] + r[1][0] * v[1] + r[2][0] * v[2],
            r[0][1] * v[0] + r[1][1] * v[1] + r[2][1] * v[2],
            r[0][2] * v[0] + r[1][2] * v[1] + r[2][2] * v[2],
        ]
    };
    for p in &mut m.positions {
        *p = map(*p);
    }
    for n in &mut m.normals {
        *n = rot(*n);
    }
    for e in &mut m.edges {
        *e = [map(e[0]), map(e[1])];
    }
}
