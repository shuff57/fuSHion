//! Compliant-socket test prints: round studs on top, springs underneath (see
//! `sf_custom::StudProfile::compliant_socket`). Writes, under `<out>` (default
//! `~/Desktop/3D/Lego-Bricks/sockets`):
//!
//! * `nozzle-0.2mm/` and `nozzle-0.4mm/`, each with `01_2x4x3`, `02_2x2x3`, `03_1x4x3`,
//!   `04_1x1x3` and `05_2x4x1` (a plate): `.step`, `.stl`, `.3mf` and a `PRINT-LOG.txt`
//!   (kept if it is already there);
//! * `00_stud-calibration/`: a plate with five pairs of round studs, Ø4.70 to Ø4.90 (1 to 5
//!   dots beside each pair), to press a real 1×2 LEGO brick onto;
//!
//! and renders to `/tmp/fushion-sockets/` (top and below views of each brick, a close-up of
//! a spring, a contact sheet `sockets.png`; the sheet needs ImageMagick's `magick`).
//!
//! `cargo run --release -p sf-custom --example sockets [out_dir]`

#[path = "common/render.rs"]
mod render;

use serde_json::{Value, json};
use sf_custom::{CustomShape, StudProfile, build, build_kit_part, plan, socket_fit};
use solidcraft_engine::Session;
use std::path::{Path, PathBuf};

fn step_from(cmds: &[Value], path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut s = Session::default();
    for c in cmds {
        let id = c["command"].as_str().ok_or("command without id")?;
        s.execute(id, &c["params"]).map_err(|e| format!("{id}: {e}"))?;
    }
    s.execute("file.export", &json!({"path": path.to_string_lossy()})).map_err(|e| format!("STEP: {e}"))?;
    Ok(())
}

fn log(name: &str, set: &str, files: &str, socket: bool) -> String {
    let questions = if socket {
        "Springs and slots under the brick open? (y/n):\n\
         Grip on real LEGO studs (loose / good / tight):\n\
         Grip on fused-stud printed bricks (the nozzle-0.x folders) (loose / good / tight):\n\
         Grip on these printed bricks (one on another) (loose / good / tight):\n\
         Real LEGO pressed on top fits? (loose / good / tight):\n\
         Springs still grip after 10 presses? (y/n):\n"
    } else {
        "Which stud size holds a real 1×2 LEGO brick best (1-5 dots = Ø4.70 / 4.75 / 4.80 / 4.85 / 4.90)?:\n\
         Too loose from (dots):\n\
         Too tight from (dots):\n"
    };
    format!(
        "{name}  ({set})\nFiles: {files}\n\nOne block per print. Copy it for each new print.\n\n\
         Date:\nPrinter / nozzle:\nMaterial / colour:\nLayer height / line width:\nSlicer settings changed:\nPrint time:\n\
         {questions}Notes:\n"
    )
}

fn write_part(dir: &Path, stem: &str, stl: &[u8], three_mf: &[u8], cmds: &[Value]) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(format!("{stem}.stl")), stl)?;
    std::fs::write(dir.join(format!("{stem}.3mf")), three_mf)?;
    step_from(cmds, &dir.join(format!("{stem}.step")))?;
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let out = PathBuf::from(
        std::env::args().nth(1).unwrap_or_else(|| format!("{home}/Desktop/3D/Lego-Bricks/sockets")),
    );
    let renders = PathBuf::from("/tmp/fushion-sockets");
    std::fs::create_dir_all(&renders)?;
    let bricks = [
        ("01_2x4x3", 2, 4, 3),
        ("02_2x2x3", 2, 2, 3),
        ("03_1x4x3", 1, 4, 3),
        ("04_1x1x3", 1, 1, 3),
        ("05_2x4x1", 2, 4, 1),
    ];
    let mut sheet = Vec::new();
    for (set, p, suffix) in [
        ("nozzle-0.2mm", StudProfile::compliant_socket(), ""),
        ("nozzle-0.4mm", StudProfile::compliant_socket_nozzle04(), "_nozzle04"),
    ] {
        let spec = p.socket.expect("socket profile");
        for (folder, w, d, plates) in bricks {
            let shape = CustomShape::Box { w, d, plates };
            let part = build(&shape, &p)?;
            let cmds: Vec<Value> = plan(&shape, &p)?.stages.iter().flat_map(|s| s.commands.clone()).collect();
            let stem = format!("fushion_socket_{w}x{d}x{plates}{suffix}");
            let dir = out.join(set).join(folder);
            write_part(&dir, &stem, &part.stl, &part.three_mf, &cmds)?;
            let log_path = dir.join("PRINT-LOG.txt");
            if !log_path.exists() {
                std::fs::write(&log_path, log(&stem, &format!("compliant sockets, {set} set"), &format!("{stem}.step / .stl / .3mf"), true))?;
            }
            let cells: Vec<(i32, i32)> = (0..w as i32).flat_map(|i| (0..d as i32).map(move |j| (i, j))).collect();
            let f = socket_fit(&cells, &p, 4.8, [0.0, 0.0])?;
            println!(
                "{set}/{folder}: {:.0} mm³, {} springs, preload {:.2} mm, {:.1} N in all, max strain {:.2} % -> {}",
                part.volume_mm3,
                f.contacts.len(),
                spec.preload,
                f.total_force,
                f.max_strain * 100.0,
                dir.display()
            );
            // Renders: from above-front, from straight below.
            let tris = render::triangles(&part.stl);
            let top = renders.join(format!("{stem}_top.png"));
            let below = renders.join(format!("{stem}_below.png"));
            render::render_one(&tris, &render::View { eye: [-1.0, -1.4, 1.1], focus: None }, 360, &top)?;
            render::render_one(&tris, &render::View { eye: [0.0, 0.0, -1.0], focus: None }, 360, &below)?;
            sheet.push((format!("{set} {folder}"), top, below));
            if folder == "01_2x4x3" {
                // Close-up of one T-spring (the long wall where cells 0 and 1 meet), from below
                // and a little inside.
                let close = renders.join(format!("{stem}_spring.png"));
                let focus = ([0.2, 4.8, 0.0], [5.0, 11.2, 3.0]);
                render::render_one(&tris, &render::View { eye: [0.5, 0.0, -1.0], focus: Some(focus) }, 420, &close)?;
                sheet.push((format!("{set} spring close-up"), close.clone(), close));
            }
        }
    }
    // The calibration plate: five pairs of studs, 1 to 5 dots beside each.
    let mut c = vec![json!({"command": "solid.box", "params": {"length": 26, "width": 50, "height": 2.4, "corner": [0, 0, 0], "operation": "new", "body_name": "Part", "name": "Plate"}})];
    let sizes = [4.70, 4.75, 4.80, 4.85, 4.90];
    for (k, d) in sizes.iter().enumerate() {
        let y = 5.0 + 10.0 * k as f64;
        for x in [5.0, 13.0] {
            c.push(json!({"command": "solid.cylinder", "params": {"radius": d / 2.0, "height": 2.1, "base": [x, y, 2.1], "operation": "join", "targets": ["Part"], "name": format!("Stud{k}{x}")}}));
        }
        for m in 0..=k {
            c.push(json!({"command": "solid.cylinder", "params": {"radius": 0.45, "height": 1.0, "base": [18.0 + 1.6 * m as f64, y, 2.0], "operation": "cut", "targets": ["Part"], "name": format!("Dot{k}{m}")}}));
        }
    }
    let cal = build_kit_part(&json!({"kit_part": {"base": null, "commands": c}}))?;
    let dir = out.join("00_stud-calibration");
    write_part(&dir, "fushion_stud_calibration", &cal.stl, &cal.three_mf, &c)?;
    let log_path = dir.join("PRINT-LOG.txt");
    if !log_path.exists() {
        std::fs::write(&log_path, log("fushion_stud_calibration", "stud calibration plate", "fushion_stud_calibration.step / .stl / .3mf", false))?;
    }
    let tris = render::triangles(&cal.stl);
    let cal_png = renders.join("stud_calibration.png");
    render::render_one(&tris, &render::View { eye: [-1.0, -1.4, 1.1], focus: None }, 360, &cal_png)?;
    sheet.push(("00 stud calibration".into(), cal_png.clone(), cal_png));
    println!("calibration plate: {:.0} mm³ -> {}", cal.volume_mm3, dir.display());
    // Contact sheet: each brick's top and below views side by side.
    let mut args = vec!["montage".to_string()];
    for (label, a, b) in &sheet {
        let pair = renders.join(format!("pair_{}.png", label.replace([' ', '/'], "_")));
        let ok = std::process::Command::new("magick")
            .args([a.display().to_string(), b.display().to_string(), "+append".into(), pair.display().to_string()])
            .status()
            .is_ok_and(|s| s.success());
        if ok {
            args.extend(["-label".into(), label.clone(), pair.display().to_string()]);
        }
    }
    args.extend(["-tile", "3x", "-geometry", "+6+6", "-pointsize", "16", "-background", "#f5f5f5"].map(String::from));
    let sheet_png = renders.join("sockets.png");
    args.push(sheet_png.display().to_string());
    if std::process::Command::new("magick").args(&args).status().is_ok_and(|s| s.success()) {
        println!("contact sheet -> {}", sheet_png.display());
    }
    Ok(())
}
