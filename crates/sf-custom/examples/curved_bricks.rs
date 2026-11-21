//! Curved printed bricks (`CustomShape::Printed` with a trim): a 2×4 whose far row rolls
//! over in a quarter-round roof, a dome-topped 2×2, and a 2×4 with an overhanging belly.
//! Writes them to `<out>` (default `~/Desktop/3D/Lego-Bricks/curved`) in numbered folders
//! with STEP, STL, 3MF and a PRINT-LOG.txt (kept if already there), and renders to
//! `/tmp/fushion-curved/`.
//!
//! `cargo run --release -p sf-custom --example curved_bricks [out_dir]`

#[path = "common/render.rs"]
mod render;

use serde_json::json;
use sf_core::{Trim, TrimBox, TrimKind, TrimOp, TrimShape};
use sf_custom::{CustomShape, StudProfile, build_trimmed, fused_units};
use solidcraft_engine::Session;
use std::path::PathBuf;

const ID: [[f64; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
/// Local z along the brick's y.
const ALONG_Y: [[f64; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0]];

fn add(kind: TrimKind, rotation: [[f64; 3]; 3], translation: [f64; 3]) -> TrimShape {
    TrimShape { op: TrimOp::Add, kind, rotation, translation }
}

/// The three test bricks: (folder, description, w, d, trim).
pub fn bricks() -> Vec<(&'static str, &'static str, u32, u32, Trim)> {
    vec![
        (
            "01_curved_2x4x3",
            "2x4 whose far row rolls over in a quarter-round roof (r 9.6 mm)",
            2,
            4,
            Trim {
                shapes: vec![
                    add(TrimKind::Box { size: [7.4, 40.0, 14.0] }, ID, [2.7, 16.0, 5.0]),
                    add(TrimKind::Cylinder { radii: [9.6, 9.6], height: 40.0 }, ALONG_Y, [6.4, 16.0, 0.0]),
                ],
                protect: vec![],
                overhang: false,
            },
        ),
        (
            "02_dome_2x2x3",
            "2x2 with a domed top (no studs)",
            2,
            2,
            Trim {
                shapes: vec![
                    add(TrimKind::Box { size: [20.0, 20.0, 5.6] }, ID, [8.0, 8.0, 2.0]),
                    add(TrimKind::Ellipsoid { radii: [11.3, 11.3, 4.8] }, ID, [8.0, 8.0, 4.8]),
                ],
                protect: vec![],
                overhang: false,
            },
        ),
        (
            "03_belly_2x4x3",
            "2x4 with an overhanging rounded belly under a flat studded top",
            2,
            4,
            Trim {
                shapes: vec![add(TrimKind::Ellipsoid { radii: [8.5, 17.0, 9.6] }, ID, [8.0, 16.0, 9.6])],
                protect: vec![TrimBox { min: [-1.0, -1.0, 6.4], max: [17.0, 33.0, 12.0] }],
                overhang: true,
            },
        ),
    ]
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let out = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| format!("{home}/Desktop/3D/Lego-Bricks/curved")));
    let renders = PathBuf::from("/tmp/fushion-curved");
    std::fs::create_dir_all(&renders)?;
    let p = StudProfile::fused();
    let mut pngs = Vec::new();
    for (folder, what, w, d, trim) in bricks() {
        let units = fused_units(&CustomShape::Box { w, d, plates: 3 }, &p)?;
        let cells: Vec<(i32, i32)> = (0..w as i32).flat_map(|i| (0..d as i32).map(move |j| (i, j))).collect();
        let shape = CustomShape::Printed { cells, plates: 3, units, label: None, trim: Some(trim) };
        let t = std::time::Instant::now();
        let r = build_trimmed(&shape, &p)?;
        let secs = t.elapsed().as_secs_f64();
        let dir = out.join(folder);
        std::fs::create_dir_all(&dir)?;
        let stem = format!("fushion_{folder}");
        std::fs::write(dir.join(format!("{stem}.stl")), &r.part.stl)?;
        std::fs::write(dir.join(format!("{stem}.3mf")), &r.part.three_mf)?;
        let mut s = Session::default();
        for c in &r.commands {
            let id = c["command"].as_str().ok_or("command without id")?;
            s.execute(id, &c["params"]).map_err(|e| format!("{id}: {e}"))?;
        }
        s.execute("file.export", &json!({"path": dir.join(format!("{stem}.step")).to_string_lossy()})).map_err(|e| format!("STEP: {e}"))?;
        let log = dir.join("PRINT-LOG.txt");
        if !log.exists() {
            std::fs::write(
                &log,
                format!(
                    "{stem}  ({what})\nFiles: {stem}.step / .stl / .3mf\nfuSHion says supports: {}\n\n\
                     One block per print. Copy it for each new print.\n\n\
                     Date:\nPrinter / nozzle:\nMaterial / colour:\nLayer height / line width:\nSlicer settings changed:\nPrint time:\n\
                     Supports needed? (none / some / lots):\n\
                     Curved surface quality (smooth / stepped / rough):\n\
                     Any holes or thin spots in the curved walls? (y/n, where):\n\
                     Clutch on another fused brick (loose / good / tight):\n\
                     Notes:\n",
                    if r.overhang_mm2 > sf_custom::SUPPORT_AREA_MM2 { "yes" } else { "no" }
                ),
            )?;
        }
        println!(
            "{folder}: {secs:.1} s, {:.0} mm³ (sampled {:.0}, {:+.1} %), untrimmed {:.0} mm³, studs dropped {:?}, hollow filled {}, overhang {:.1} mm² -> {}",
            r.part.volume_mm3,
            r.sampled_volume,
            (r.part.volume_mm3 / r.sampled_volume - 1.0) * 100.0,
            r.untrimmed.volume_mm3,
            r.dropped_studs,
            r.filled,
            r.overhang_mm2,
            dir.display()
        );
        let tris = render::triangles(&r.part.stl);
        let top = renders.join(format!("{folder}.png"));
        render::render(&r.part.stl, &top)?;
        let below = renders.join(format!("{folder}_below.png"));
        render::render_one(&tris, &render::View { eye: [0.0, 0.0, -1.0], focus: None }, 360, &below)?;
        pngs.push((folder, top));
    }
    let mut args = vec!["montage".to_string()];
    for (f, png) in &pngs {
        args.extend(["-label".into(), f.to_string(), png.display().to_string()]);
    }
    args.extend(["-tile", "1x", "-geometry", "+4+4", "-pointsize", "16", "-background", "#f5f5f5"].map(String::from));
    args.push(renders.join("curved.png").display().to_string());
    if std::process::Command::new("magick").args(&args).status().is_ok_and(|s| s.success()) {
        println!("contact sheet -> {}", renders.join("curved.png").display());
    }
    Ok(())
}
