//! Times `build_trimmed` stage by stage on real curved bricks from a saved model.
//!
//! `cargo run --release -p sf-custom --example trim_profile -- [model.json] [ids...]`
//! (default model /tmp/fushion-e2e/model.json, default ids B001 B008 B020). With
//! `--stl DIR` each trimmed brick's STL is written to DIR/<id>.stl, with renders from above
//! and below (DIR/<id>.png, DIR/<id>_below.png).

#[path = "common/render.rs"]
mod render;

use sf_custom::{CustomShape, StudProfile, build_trimmed};
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut stl_dir = None;
    if let Some(k) = args.iter().position(|a| a == "--stl") {
        stl_dir = Some(std::path::PathBuf::from(args.remove(k + 1)));
        args.remove(k);
    }
    let model = if args.first().is_some_and(|a| a.ends_with(".json")) {
        args.remove(0)
    } else {
        "/tmp/fushion-e2e/model.json".into()
    };
    let ids: Vec<String> = if args.is_empty() {
        ["B001", "B008", "B020"].map(String::from).to_vec()
    } else {
        args
    };
    let m: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&model)?)?;
    let mut total = 0.0;
    for id in &ids {
        let Some(c) = m["placements"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|p| p["custom"]["id"] == id.as_str())
        else {
            println!("{id}: not in the model");
            continue;
        };
        let def: sf_core::CustomDef = serde_json::from_value(c["custom"].clone())?;
        let Some(trim) = def.trim.as_deref() else {
            println!("{id}: no trim");
            continue;
        };
        let shape = CustomShape::Printed {
            cells: def
                .cells
                .iter()
                .map(|&(a, b)| (i32::from(a), i32::from(b)))
                .collect(),
            plates: u32::from(def.plates),
            units: def
                .units
                .iter()
                .map(|u| {
                    u.iter()
                        .map(|&(a, b)| (i32::from(a), i32::from(b)))
                        .collect()
                })
                .collect(),
            label: Some(def.id.clone()),
            trim: Some(trim.clone()),
            sockets: Some(
                def.sockets
                    .iter()
                    .map(|&(a, b)| (i32::from(a), i32::from(b)))
                    .collect(),
            ),
        };
        let t = Instant::now();
        let r = build_trimmed(&shape, &StudProfile::fused());
        let secs = t.elapsed().as_secs_f64();
        total += secs;
        println!(
            "{id}: {} cells, {} plates, {} shapes, {} protect boxes, {} sockets: {secs:.1} s",
            def.cells.len(),
            def.plates,
            trim.shapes.len(),
            trim.protect.len(),
            def.sockets.len()
        );
        match r {
            Ok(tr) => {
                let stages: Vec<String> = tr
                    .timings
                    .iter()
                    .map(|(n, s)| format!("{n} {s:.2}"))
                    .collect();
                println!("  {}", stages.join(" | "));
                let slow: Vec<String> = tr
                    .slowest
                    .iter()
                    .map(|(s, c, i)| format!("#{i} {c} {s:.2}"))
                    .collect();
                println!("  slowest:\n    {}", slow.join("\n    "));
                println!(
                    "  {:.0} mm³ (sampled {:.0}), overhang {:.1} mm², dropped {} studs, filled {}, {} trim commands",
                    tr.part.volume_mm3,
                    tr.sampled_volume,
                    tr.overhang_mm2,
                    tr.dropped_studs.len(),
                    tr.filled,
                    tr.commands.len()
                );
                if let Some(d) = &stl_dir {
                    std::fs::create_dir_all(d)?;
                    std::fs::write(d.join(format!("{id}.stl")), &tr.part.stl)?;
                    render::render(&tr.part.stl, &d.join(format!("{id}.png")))?;
                    let tris = render::triangles(&tr.part.stl);
                    render::render_one(
                        &tris,
                        &render::View {
                            eye: [0.0, 0.0, -1.0],
                            focus: None,
                        },
                        360,
                        &d.join(format!("{id}_below.png")),
                    )?;
                }
            }
            Err(e) => println!("  FAILED: {e}"),
        }
    }
    println!("total {total:.1} s for {} bricks", ids.len());
    Ok(())
}
