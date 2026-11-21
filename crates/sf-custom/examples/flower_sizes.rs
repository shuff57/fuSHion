//! Rose, aster and daisy kits at their default size and at the largest (64 mm), assembled
//! and rendered side by side to `/tmp/fushion-flowers/` (also a contact sheet `sizes.png`),
//! with build times and every part's size.
//!
//! `cargo run --release -p sf-custom --example flower_sizes`

#[path = "common/render.rs"]
mod render;

use serde_json::json;
use sf_custom::{MAX_PART_MM, detail_template};
use std::path::PathBuf;

/// The assembled display mesh (LDU, −Y up) as a binary STL in mm, z up.
fn mesh_stl(m: &sf_core::DetailMesh) -> Vec<u8> {
    let mut out = vec![0u8; 80];
    let n = m.indices.len() / 3;
    out.extend((n as u32).to_le_bytes());
    let v = |i: u32| {
        let p = m.positions[i as usize];
        [p[0] / 2.5, p[2] / 2.5, -p[1] / 2.5]
    };
    for t in m.indices.chunks(3) {
        out.extend([0f32; 3].iter().flat_map(|f| f.to_le_bytes()));
        for &i in t {
            out.extend(v(i).iter().flat_map(|f| f.to_le_bytes()));
        }
        out.extend([0u8; 2]);
    }
    out
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = PathBuf::from("/tmp/fushion-flowers");
    std::fs::create_dir_all(&dir)?;
    let mut tiles = Vec::new();
    for (template, sizes) in [
        ("rose", [36.0, 64.0]),
        ("aster", [34.0, 64.0]),
        ("daisy", [40.0, 64.0]),
    ] {
        for size in sizes {
            let t = std::time::Instant::now();
            let d = detail_template(template, template, template, &json!({"size": size}))?;
            let secs = t.elapsed().as_secs_f64();
            let kit = d.design.kit.as_ref().ok_or("a kit")?;
            let biggest = d
                .kit_prints
                .iter()
                .map(|p| {
                    (0..3)
                        .map(|k| p.bbox_mm.1[k] - p.bbox_mm.0[k])
                        .fold(0.0, f64::max)
                })
                .fold(0.0, f64::max);
            let (lo, hi) = d.design.mesh.positions.iter().fold(
                ([f32::MAX; 3], [f32::MIN; 3]),
                |(mut a, mut b), p| {
                    for k in 0..3 {
                        a[k] = a[k].min(p[k]);
                        b[k] = b[k].max(p[k]);
                    }
                    (a, b)
                },
            );
            let across = ((hi[0] - lo[0]).max(hi[2] - lo[2]) / 2.5) as f64;
            let roles: Vec<String> = kit.roles.iter().map(|(r, c)| format!("{r}={c}")).collect();
            println!(
                "{template} size {size}: {secs:.1} s, {across:.1} mm across ({:.1} studs), {} cells, {} plates, largest part {biggest:.1} mm (limit {MAX_PART_MM}), roles {}",
                across / 8.0,
                d.design.cells.len(),
                d.design.plates,
                roles.join(" ")
            );
            let png = dir.join(format!("{template}_{size}.png"));
            render::render(&mesh_stl(&d.design.mesh), &png)?;
            tiles.push((format!("{template} {size} mm"), png));
        }
    }
    let mut args = vec!["montage".to_string()];
    for (label, png) in &tiles {
        args.extend(["-label".into(), label.clone(), png.display().to_string()]);
    }
    args.extend([
        "-tile".into(),
        "2x".into(),
        "-geometry".into(),
        "+4+4".into(),
        dir.join("sizes.png").display().to_string(),
    ]);
    if std::process::Command::new("magick")
        .args(&args)
        .status()
        .is_ok_and(|s| s.success())
    {
        println!("contact sheet -> {}", dir.join("sizes.png").display());
    }
    Ok(())
}
