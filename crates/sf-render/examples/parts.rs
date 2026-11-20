//! Contact sheet of catalog parts: each part rendered alone in the given
//! views, with LDraw meshes (left) and procedural meshes (right).
//!
//! `cargo run -p sf-render --example parts -- [part ids...]` writes
//! `/tmp/studforge-parts.png`.

use sf_core::{CATALOG, Cell, Palette, Placement, Rot};
use sf_ldraw::MeshLibrary;
use sf_render::{RenderOptions, View, render_part};

fn main() -> anyhow::Result<()> {
    let ids: Vec<String> = std::env::args().skip(1).collect();
    let parts: Vec<&str> =
        CATALOG.iter().map(|d| d.id).filter(|id| ids.is_empty() || ids.iter().any(|i| i == id)).collect();
    let lib = MeshLibrary::open(&sf_ldraw::ensure_installed()?)?;
    let proc_lib = MeshLibrary::procedural();
    let pal = Palette::builtin();
    let opts = RenderOptions { view_size: 160, ..Default::default() };
    let views = [View::Front, View::Iso, View::Top];
    let cols = views.len() as u32 * 2;
    let mut sheet = image::RgbaImage::from_pixel(cols * 160, parts.len() as u32 * 160, image::Rgba([0, 0, 0, 255]));
    for (row, id) in parts.iter().enumerate() {
        let p = Placement::new(id, 4, Cell::new(0, 0, 0), Rot(0));
        for (i, lib) in [&lib, &proc_lib].into_iter().enumerate() {
            for (j, v) in views.iter().enumerate() {
                let img = render_part(&p, &pal, lib, *v, &opts);
                let x = (i * views.len() + j) as i64 * 160;
                image::imageops::replace(&mut sheet, &img, x, row as i64 * 160);
            }
        }
    }
    sheet.save("/tmp/studforge-parts.png")?;
    println!("wrote /tmp/studforge-parts.png ({} parts; columns: LDraw front/iso/top, procedural front/iso/top)", parts.len());
    Ok(())
}
