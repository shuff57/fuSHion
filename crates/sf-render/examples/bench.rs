//! Timing: a ~1,500-part model rendered as a 6-view sheet at 384 px.
//!
//! `cargo run -p sf-render --example bench [--release]`

use sf_core::{Cell, Model, Palette, Placement, Rot};
use sf_ldraw::MeshLibrary;
use sf_render::{RenderOptions, View, render_sheet, render_view};
use std::time::Instant;

fn main() -> anyhow::Result<()> {
    let lib = match sf_ldraw::ensure_installed().and_then(|d| MeshLibrary::open(&d)) {
        Ok(l) => l,
        Err(_) => MeshLibrary::procedural(),
    };
    let pal = Palette::builtin();
    let colors: Vec<u16> = pal.colors.iter().map(|c| c.code).collect();
    // A 24x24-stud tower: layers of 2x4 / 1x2 / 2x2 bricks and some slopes.
    let mut placements = Vec::new();
    let mut i = 0usize;
    'outer: for layer in 0..40 {
        let y = layer * 3;
        let rot = Rot((layer % 2) as u8);
        for a in (0..24).step_by(4) {
            for b in (0..24).step_by(2) {
                let (x, z) = if rot.0 == 0 { (a, b) } else { (b, a) };
                let part = match i % 7 {
                    0 => "3003",
                    1 => "3039",
                    _ => "3001",
                };
                let c = colors[i % colors.len()];
                if part == "3003" || part == "3039" {
                    placements.push(Placement::new(part, c, Cell::new(x, y, z), Rot(0)));
                    let (x2, z2) = if rot.0 == 0 { (x + 2, z) } else { (x, z + 2) };
                    placements.push(Placement::new("3003", c, Cell::new(x2, y, z2), Rot(0)));
                } else {
                    placements.push(Placement::new(part, c, Cell::new(x, y, z), rot));
                }
                i += 1;
                if placements.len() >= 1500 {
                    break 'outer;
                }
            }
        }
    }
    let model = Model { placements, ..Default::default() };
    println!("{} placements, library: {}", model.placements.len(), if lib.is_procedural() { "procedural" } else { "LDraw" });
    let opts = RenderOptions::default();
    // Warm the mesh cache.
    let _ = render_view(&model, &pal, &lib, View::Front, &RenderOptions { view_size: 16, ..opts.clone() });
    let t = Instant::now();
    let _ = render_view(&model, &pal, &lib, View::Iso, &opts);
    println!("one view: {:.1?}", t.elapsed());
    let t = Instant::now();
    let sheet = render_sheet(&model, &pal, &lib, &View::ALL, &opts);
    println!("6-view sheet (threaded): {:.1?}", t.elapsed());
    let t = Instant::now();
    for v in View::ALL {
        let _ = render_view(&model, &pal, &lib, v, &opts);
    }
    println!("6 views sequential: {:.1?}", t.elapsed());
    sheet.save("/tmp/studforge-bench.png")?;
    Ok(())
}
