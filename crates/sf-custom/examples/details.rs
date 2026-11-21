//! Builds every detail template and flower species (and two printed bricks), and renders
//! each to `<out>/<name>.png` (from front-left above, and from the side), plus a contact sheet
//! of every flower, `<out>/flowers.png` (needs ImageMagick's `magick` on the PATH).
//!
//! `cargo run --release -p sf-custom --example details [out_dir]` (default `/tmp/fushion-details`).

#[path = "common/render.rs"]
mod render;

use serde_json::json;
use sf_custom::{CustomShape, StudProfile, build, detail_template};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "/tmp/fushion-details".into()));
    std::fs::create_dir_all(&out)?;
    // (file name, template, params)
    let cases = [
        ("rose", "rose", json!({})),
        ("rose_open_on_stem", "rose", json!({"open": 0.6, "stem_height": 12})),
        ("daisy", "daisy", json!({})),
        ("aster", "aster", json!({})),
        ("gerbera", "gerbera", json!({})),
        ("poppy", "poppy", json!({})),
        ("cornflower", "cornflower", json!({})),
        ("spike_lupin", "spike", json!({"kind": "lupin"})),
        ("spike_snapdragon", "spike", json!({"kind": "snapdragon"})),
        ("spike_lavender", "spike", json!({"kind": "lavender", "florets_per_ring": 6, "rings": 10, "floret_size": 2.2})),
        ("spike_larkspur", "spike", json!({"kind": "larkspur"})),
        ("umbel", "umbel", json!({})),
        ("fern", "fern", json!({})),
        ("stem", "stem", json!({"top": "bud", "angle": 15})),
        ("flower_head", "flower_head", json!({})),
        ("petal", "petal", json!({"tilt": 40})),
        ("leaf", "leaf", json!({"length": 18, "width": 7})),
        ("dome", "dome", json!({})),
        ("cone_beak", "cone", json!({"direction": "+x", "length": 10, "tilt": 10})),
        ("round_plate", "round_plate", json!({})),
    ];
    let mut flowers = Vec::new();
    // FUSHION_ONLY=name1,name2 builds just those.
    let only: Option<Vec<String>> =
        std::env::var("FUSHION_ONLY").ok().map(|v| v.split(',').map(str::to_string).collect());
    for (name, template, p) in cases {
        if only.as_ref().is_some_and(|o| !o.iter().any(|x| x == name)) {
            continue;
        }
        let t = std::time::Instant::now();
        let d = match detail_template(name, name, template, &p) {
            Ok(d) => d,
            Err(e) => {
                println!("{name}: FAILED {e}");
                continue;
            }
        };
        let secs = t.elapsed().as_secs_f64();
        std::fs::write(out.join(format!("{name}.stl")), &d.part.stl)?;
        let tris = render::triangles(&d.part.stl);
        let png = out.join(format!("{name}.png"));
        let views = [
            render::View { eye: [-1.0, -1.4, 1.1], focus: None },
            render::View { eye: [0.0, -1.0, 0.0], focus: None },
        ];
        render::render_views(&tris, &views, 420, &png)?;
        let iso = out.join(format!("{name}_iso.png"));
        render::render_one(&tris, &views[0], 360, &iso)?;
        let (lo, hi) = d.part.bbox_mm;
        println!(
            "{name}: {secs:.1} s, {:.0}×{:.0}×{:.0} mm, {:.0} mm³, {} print / {} display triangles, supports {} ({:.1} mm²), cells {}×{}, {} plates",
            hi[0] - lo[0],
            hi[1] - lo[1],
            hi[2] - lo[2],
            d.part.volume_mm3,
            tris.len(),
            d.design.mesh.indices.len() / 3,
            d.supports,
            d.overhang_mm2,
            d.design.cells.iter().map(|c| c.0).max().unwrap_or(0) + 1,
            d.design.cells.iter().map(|c| c.1).max().unwrap_or(0) + 1,
            d.design.plates
        );
        if !["flower_head", "petal", "leaf", "dome", "cone_beak", "round_plate"].contains(&name) {
            flowers.push((name, iso, d.supports));
        }
    }
    let bricks = if only.is_some() { Vec::new() } else { vec![
        ("printed_L", CustomShape::Printed { cells: vec![(0, 0), (1, 0), (2, 0), (0, 1), (0, 2)], plates: 3, units: vec![vec![(0, 0), (1, 0), (2, 0)], vec![(0, 1), (0, 2)]], label: Some("B017".into()) , trim: None}),
        ("printed_2x4_split", CustomShape::Printed { cells: (0..4).flat_map(|i| (0..2).map(move |j| (i, j))).collect(), plates: 3, units: vec![vec![(0, 0), (1, 0)], vec![(2, 0), (3, 0)], vec![(0, 1), (1, 1), (2, 1)]], label: Some("B123".into()), trim: None }),
    ] };
    for (name, shape) in bricks {
        let part = build(&shape, &StudProfile::fused())?;
        let png = out.join(format!("{name}.png"));
        std::fs::write(out.join(format!("{name}.stl")), &part.stl)?;
        render::render(&part.stl, &png)?;
        let tris = render::triangles(&part.stl);
        let below = out.join(format!("{name}_below.png"));
        render::render_one(&tris, &render::View { eye: [0.0, 0.0, -1.0], focus: None }, 520, &below)?;
        println!("{name}: {:.0} mm³ -> {}", part.volume_mm3, png.display());
    }
    if only.is_some() {
        return Ok(());
    }
    // Contact sheet of the flowers, each labelled with its name (and "supports" if it needs them).
    let mut args: Vec<String> = vec!["montage".into()];
    for (name, iso, supports) in &flowers {
        args.push("-label".into());
        args.push(if *supports { format!("{name} (supports)") } else { (*name).to_string() });
        args.push(iso.display().to_string());
    }
    let sheet = out.join("flowers.png");
    args.extend(
        ["-tile", "5x", "-geometry", "+6+6", "-pointsize", "18", "-background", "#f5f5f5"]
            .map(String::from),
    );
    args.push(sheet.display().to_string());
    match std::process::Command::new("magick").args(&args).status() {
        Ok(s) if s.success() => println!("contact sheet -> {}", sheet.display()),
        other => println!("contact sheet skipped (magick: {other:?})"),
    }
    Ok(())
}
