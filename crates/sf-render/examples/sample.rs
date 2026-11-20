//! Render a small hand-made model as a 6-view sheet.
//!
//! `cargo run -p sf-render --example sample [--release]` writes
//! `/tmp/studforge-sample.png` (LDraw meshes, falling back to procedural ones
//! when the library isn't installed) and `/tmp/studforge-sample-procedural.png`
//! (iso view with procedural meshes, for comparison).

use sf_core::{Cell, Model, Palette, Placement, Rot};
use sf_ldraw::MeshLibrary;
use sf_render::{RenderOptions, View, render_sheet, render_view};
use std::time::Instant;

fn sample_model() -> Model {
    let p = |id: &str, color: u16, x: i32, y: i32, z: i32, r: u8| Placement::new(id, color, Cell::new(x, y, z), Rot(r));
    Model {
        title: "sample".into(),
        placements: vec![
            // Base layer: a white 2x4 plate along x and a light grey 2x4 rotated.
            p("3020", 15, 0, 0, 0, 0),
            p("3020", 71, 4, 0, 0, 1),
            // Bricks on the plate.
            p("3001", 4, 0, 1, 0, 0),  // red 2x4
            p("3004", 1, 4, 1, 0, 0),  // blue 1x2 along x
            p("3004", 14, 4, 1, 2, 1), // yellow 1x2 rotated (along z)
            p("3005", 2, 5, 1, 3, 0),  // green 1x1
            // Second layer: a 3040b slope falling toward +x on the red brick,
            // and one rotated to fall toward -z.
            p("3040b", 25, 0, 4, 0, 0),
            p("3040b", 25, 2, 4, 0, 1),
            // An inverted slope (3665) hanging out over -x side at rot 2.
            p("3665b", 73, 4, 4, 0, 2),
            // A plate and a tile on top.
            p("3023", 0, 0, 4, 1, 0),
            p("3070b", 15, 3, 4, 1, 0),
            p("54200", 4, 5, 4, 3, 2),
        ],
        ..Default::default()
    }
}

fn main() -> anyhow::Result<()> {
    let palette = Palette::builtin();
    let lib = match sf_ldraw::ensure_installed().and_then(|d| MeshLibrary::open(&d)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("LDraw library unavailable ({e:#}); using procedural meshes");
            MeshLibrary::procedural()
        }
    };
    let model = sample_model();
    let opts = RenderOptions::default();
    let t = Instant::now();
    let sheet = render_sheet(&model, &palette, &lib, &View::ALL, &opts);
    println!("sheet {}x{} in {:.1?}", sheet.width(), sheet.height(), t.elapsed());
    sheet.save("/tmp/studforge-sample.png")?;

    let proc_lib = MeshLibrary::procedural();
    let a = render_view(&model, &palette, &lib, View::Iso, &opts);
    let b = render_view(&model, &palette, &proc_lib, View::Iso, &opts);
    let mut pair = image::RgbaImage::new(a.width() * 2, a.height());
    image::imageops::replace(&mut pair, &a, 0, 0);
    image::imageops::replace(&mut pair, &b, a.width() as i64, 0);
    pair.save("/tmp/studforge-sample-procedural.png")?;
    println!("wrote /tmp/studforge-sample.png and /tmp/studforge-sample-procedural.png");
    Ok(())
}
