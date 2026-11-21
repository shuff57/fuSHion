//! Builds every kit template and renders each to `<out>/<template>.png`: the assembled kit,
//! an exploded view (parts spread apart) and every part laid out as it prints; a contact
//! sheet `<out>/kits.png`; and the clip test coupon as STEP, STL and 3MF in `<out>/coupon/`
//! (needs ImageMagick's `magick` for the sheets).
//!
//! `cargo run --release -p sf-custom --example kits [out_dir]` (default `/tmp/fushion-kits`);
//! `FUSHION_ONLY=rose,daisy` builds just those.

#[path = "common/render.rs"]
mod render;

use render::V3;
use serde_json::json;
use sf_core::{DetailMesh, KitPose};
use sf_custom::{KITS, build_kit_part, clip_fit, coupon, detail_template, ClipSpec};
use solidcraft_engine::Session;
use std::path::{Path, PathBuf};
use std::process::Command;

fn tris_of(mesh: &DetailMesh, pose: Option<&KitPose>, shift: [f32; 3]) -> Vec<[V3; 3]> {
    let place = |v: [f32; 3]| -> [f32; 3] {
        match pose {
            Some(p) => {
                let r = &p.rotation;
                [
                    r[0][0] * v[0] + r[0][1] * v[1] + r[0][2] * v[2] + p.position[0],
                    r[1][0] * v[0] + r[1][1] * v[1] + r[1][2] * v[2] + p.position[1],
                    r[2][0] * v[0] + r[2][1] * v[1] + r[2][2] * v[2] + p.position[2],
                ]
            }
            None => v,
        }
    };
    // LDU (−Y up) → the renderer's z-up frame.
    let to = |v: [f32; 3]| -> V3 {
        let w = place(v);
        [f64::from(w[0] + shift[0]), f64::from(w[2] + shift[2]), -f64::from(w[1] + shift[1])]
    };
    mesh.indices
        .as_chunks::<3>()
        .0
        .iter()
        .map(|t| [to(mesh.positions[t[0] as usize]), to(mesh.positions[t[1] as usize]), to(mesh.positions[t[2] as usize])])
        .collect()
}

fn magick(args: &[String]) -> bool {
    Command::new("magick").args(args).status().is_ok_and(|s| s.success())
}

fn write_step(cmds: &[serde_json::Value], path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut s = Session::default();
    for c in cmds {
        let id = c["command"].as_str().ok_or("command without id")?;
        s.execute(id, &c["params"]).map_err(|e| format!("{id}: {e}"))?;
    }
    s.execute("file.export", &json!({"path": path.to_string_lossy()})).map_err(|e| format!("STEP: {e}"))?;
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "/tmp/fushion-kits".into()));
    std::fs::create_dir_all(out.join("coupon"))?;
    let only: Option<Vec<String>> = std::env::var("FUSHION_ONLY").ok().map(|v| v.split(',').map(str::to_string).collect());

    // The coupon.
    if only.is_none() {
        for i in [0.0, 0.05, 0.10, 0.15] {
            let f = clip_fit(&ClipSpec { interference: i, ..ClipSpec::default() });
            println!(
                "clip i={i:.2}: snap strain {:.2} %, grip strain {:.2} %, jaw {:.2} mm, retention {:.2} mm, ok {}",
                f.snap_strain * 100.0, f.grip_strain * 100.0, f.jaw_length, f.retention, f.ok()
            );
        }
        for (name, cmds) in coupon() {
            let part = build_kit_part(&json!({"kit_part": {"base": null, "commands": cmds}}))?;
            let dir = out.join("coupon");
            std::fs::write(dir.join(format!("{name}.stl")), &part.stl)?;
            std::fs::write(dir.join(format!("{name}.3mf")), &part.three_mf)?;
            write_step(&cmds, &dir.join(format!("{name}.step")))?;
            println!("coupon {name}: {:.0} mm³ -> {}", part.volume_mm3, dir.join(format!("{name}.step")).display());
        }
    }

    let mut sheets = Vec::new();
    for (template, _) in KITS {
        if only.as_ref().is_some_and(|o| !o.iter().any(|x| x == template)) {
            continue;
        }
        let t = std::time::Instant::now();
        let d = match detail_template(template, template, template, &json!({})) {
            Ok(d) => d,
            Err(e) => {
                println!("{template}: FAILED {e}");
                continue;
            }
        };
        let secs = t.elapsed().as_secs_f64();
        let kit = d.design.kit.as_ref().expect("kit");
        let dir = out.join(template);
        std::fs::create_dir_all(&dir)?;
        // Assembled, and exploded (each copy pushed out from the middle).
        let views = [render::View { eye: [-1.0, -1.4, 1.1], focus: None }];
        let asm = tris_of(&d.design.mesh, None, [0.0; 3]);
        render::render_one(&asm, &views[0], 420, &dir.join("assembled.png"))?;
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for v in &d.design.mesh.positions {
            for i in 0..3 {
                lo[i] = lo[i].min(v[i]);
                hi[i] = hi[i].max(v[i]);
            }
        }
        let mid = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, (lo[2] + hi[2]) / 2.0];
        let mut exploded = Vec::new();
        for p in &kit.parts {
            for pose in &p.copies {
                let c = pose.position;
                let push = [(c[0] - mid[0]) * 0.9, (c[1] - mid[1]) * 0.9 - 12.0 * (p.key != "hub" && p.key != "mount" && p.key != "stem" && p.key != "eye") as i32 as f32, (c[2] - mid[2]) * 0.9];
                exploded.extend(tris_of(&p.mesh, Some(pose), push));
            }
        }
        render::render_one(&exploded, &views[0], 420, &dir.join("exploded.png"))?;
        // Each part as it prints, and its files.
        let mut part_pngs = Vec::new();
        for (p, pr) in kit.parts.iter().zip(&d.kit_prints) {
            std::fs::write(dir.join(format!("{}.stl", p.key)), &pr.stl)?;
            let tris = render::triangles(&pr.stl);
            let png = dir.join(format!("part_{}.png", p.key));
            render::render_one(&tris, &views[0], 220, &png)?;
            part_pngs.push((p, png));
            println!(
                "  {template}/{}: {} × {}, {} ({:.1}×{:.1}×{:.1} mm), supports {}",
                p.key,
                p.copies.len(),
                p.label,
                p.role,
                pr.bbox_mm.1[0] - pr.bbox_mm.0[0],
                pr.bbox_mm.1[1] - pr.bbox_mm.0[1],
                pr.bbox_mm.1[2] - pr.bbox_mm.0[2],
                p.supports
            );
        }
        println!(
            "{template}: {secs:.1} s, {} parts ({} pieces), {} steps, {} display triangles, cells {}×{}, {} plates, any supports {}",
            kit.parts.len(),
            kit.parts.iter().map(|p| p.copies.len()).sum::<usize>(),
            kit.steps.len(),
            d.design.mesh.indices.len() / 3,
            d.design.cells.iter().map(|c| c.0).max().unwrap_or(0) + 1,
            d.design.cells.iter().map(|c| c.1).max().unwrap_or(0) + 1,
            d.design.plates,
            d.supports
        );
        // The template's sheet: assembled | exploded, then the parts.
        let mut row = vec!["montage".to_string()];
        for (p, png) in &part_pngs {
            row.push("-label".into());
            row.push(format!("{} ×{}{}", p.label, p.copies.len(), if p.supports { " (supports)" } else { "" }));
            row.push(png.display().to_string());
        }
        let parts_png = dir.join("parts.png");
        row.extend(["-tile", "x1", "-geometry", "+4+4", "-pointsize", "14", "-background", "#f5f5f5"].map(String::from));
        row.push(parts_png.display().to_string());
        let sheet = out.join(format!("{template}.png"));
        if magick(&row)
            && magick(&[
                dir.join("assembled.png").display().to_string(),
                dir.join("exploded.png").display().to_string(),
                "+append".into(),
                parts_png.display().to_string(),
                "-gravity".into(),
                "center".into(),
                "-background".into(),
                "#f5f5f5".into(),
                "-append".into(),
                sheet.display().to_string(),
            ])
        {
            sheets.push((template.to_string(), sheet));
        }
    }
    if only.is_none() && !sheets.is_empty() {
        let mut args = vec!["montage".to_string()];
        for (n, s) in &sheets {
            args.push("-label".into());
            args.push(n.clone());
            args.push(s.display().to_string());
        }
        args.extend(["-tile", "5x", "-geometry", "+8+8", "-pointsize", "20", "-background", "#ffffff"].map(String::from));
        let all = out.join("kits.png");
        args.push(all.display().to_string());
        if magick(&args) {
            println!("contact sheet -> {}", all.display());
        }
    }
    Ok(())
}
