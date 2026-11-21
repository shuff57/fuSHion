//! Builds every stem and the hub of an engine bouquet model (default
//! `/tmp/fushion-bouquet/mixed-model.json`), with timings; `--stl DIR` writes the STLs and
//! renders them there.
//!
//! `cargo run --release -p sf-custom --example stem_probe -- [model.json] [--stl DIR]`

#[path = "common/render.rs"]
mod render;

use sf_core::{Hub, Model};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut out = None;
    if let Some(k) = args.iter().position(|a| a == "--stl") {
        out = Some(PathBuf::from(args.remove(k + 1)));
        args.remove(k);
    }
    let path = args
        .first()
        .cloned()
        .unwrap_or_else(|| "/tmp/fushion-bouquet/mixed-model.json".into());
    let m: Model = serde_json::from_str(&std::fs::read_to_string(&path)?)?;
    if let Some(d) = &out {
        std::fs::create_dir_all(d)?;
    }
    let save = |name: &str, stl: &[u8]| -> Result<(), Box<dyn std::error::Error>> {
        if let Some(d) = &out {
            std::fs::write(d.join(format!("{name}.stl")), stl)?;
            render::render(stl, &d.join(format!("{name}.png")))?;
        }
        Ok(())
    };
    for st in &m.stems {
        if std::env::var("STEM_DUMP").is_ok_and(|v| v == st.def.id) {
            let (c, _) = sf_custom::stem::stem_commands(&st.def)?;
            for (i, x) in c.iter().enumerate() {
                println!("[{i}] {x}");
            }
        }
        let t = std::time::Instant::now();
        match sf_custom::build_stem(&st.def) {
            Ok(p) => {
                let (lo, hi) = p.bbox_mm;
                println!(
                    "{} ({}): {:.1} s, {:.0} mm³, {:.0} × {:.0} mm, overhang {:.1} mm²",
                    st.def.id,
                    if st.def.style.is_empty() {
                        "plain"
                    } else {
                        &st.def.style
                    },
                    t.elapsed().as_secs_f64(),
                    p.volume_mm3,
                    hi[0] - lo[0],
                    hi[1] - lo[1],
                    sf_custom::overhang_area(&p.stl, 0.4)
                );
                save(&st.def.id, &p.stl)?;
            }
            Err(e) => println!("{}: FAILED {e}", st.def.id),
        }
    }
    for (key, src) in &m.details {
        if let Some(h) = src.source.get("hub") {
            let hub: Hub = serde_json::from_value(h.clone())?;
            let t = std::time::Instant::now();
            match sf_custom::build_hub(&hub) {
                Ok(p) => {
                    println!(
                        "{key}: {:.1} s, {:.0} mm³, overhang {:.1} mm²",
                        t.elapsed().as_secs_f64(),
                        p.volume_mm3,
                        sf_custom::overhang_area(&p.stl, 1.85)
                    );
                    save(key, &p.stl)?;
                }
                Err(e) => println!("{key}: FAILED {e}"),
            }
        }
    }
    Ok(())
}
