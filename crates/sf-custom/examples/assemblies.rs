//! Writes SolidCraft designs (`.sc`) that show each brick variant seated
//! between two standard bricks, for review in the SolidCraft desktop app:
//!
//! `cargo run --release -p sf-custom --example assemblies -- <out_dir>`
//!
//! Each design holds three bodies: `Below` (standard 2×4, translucent), the
//! variant in the middle (`Variant`, orange) and `Above` (standard 2×4,
//! translucent), stacked as they would clip together. Every brick is built
//! away from the others and moved into place at the end, so no cut of one
//! brick can touch another.

use serde_json::{Value, json};
use sf_custom::{CustomShape, StudProfile, plan};
use solidcraft_engine::Session;
use std::path::PathBuf;

fn run(s: &mut Session, id: &str, params: Value) -> Result<Value, Box<dyn std::error::Error>> {
    s.execute(id, &params).map_err(|e| format!("{id} {params}: {e}").into())
}

/// Build one part (body "Part"), rename it, park it at `park`.
fn build_part(s: &mut Session, profile: &StudProfile, name: &str, park: [f64; 3]) -> Result<(), Box<dyn std::error::Error>> {
    let shape = CustomShape::Box { w: 2, d: 4, plates: 3 };
    let p = plan(&shape, profile)?;
    // Feature names must be unique in a design: prefix this brick's names
    // (and every reference to them) with the brick's own name.
    let mut names: Vec<String> = p
        .stages
        .iter()
        .flat_map(|st| st.commands.iter())
        .filter_map(|c| c["params"]["name"].as_str().map(String::from))
        .collect();
    names.sort_by_key(|n| std::cmp::Reverse(n.len()));
    names.dedup();
    for stage in &p.stages {
        for c in &stage.commands {
            let id = c["command"].as_str().ok_or("command without id")?;
            let mut text = c["params"].to_string();
            for n in &names {
                text = text.replace(&format!("\"{n}\""), &format!("\"{name}-{n}\""));
            }
            run(s, id, serde_json::from_str(&text)?)?;
        }
    }
    if std::env::var("DEBUG_BODIES").is_ok() {
        let m = run(s, "inspect.measure", json!({}))?;
        let names: Vec<String> = m["bodies"].as_array().into_iter().flatten().map(|b| b["name"].to_string()).collect();
        eprintln!("{name}: {names:?}");
    }
    run(s, "body.rename", json!({"body": "Part", "name": name}))?;
    run(s, "solid.move", json!({"bodies": [name], "translate": park}))?;
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "/tmp/fushion-assemblies".into()));
    std::fs::create_dir_all(&out)?;
    let std_fdm = StudProfile::standard_fdm();
    let with = |base: StudProfile, ribs: bool, r: f64| StudProfile { wall_ribs: ribs, edge_radius: r, ..base };
    let variants: Vec<(&str, StudProfile)> = vec![
        ("a_centre_rib_only", std_fdm),
        ("b_wall_ribs", with(std_fdm, true, 0.0)),
        ("c_rounded_edges_r0.5", with(std_fdm, false, 0.5)),
        ("d_ribs_and_rounded", with(std_fdm, true, 0.5)),
        ("v2_slant_fit_safe", StudProfile::slant_v2_experimental()),
        ("v2_slant_ribs_and_rounded", with(StudProfile::slant_v2_experimental(), true, 0.5)),
    ];
    let height = std_fdm.body_height(3);
    for (name, profile) in variants {
        let mut s = Session::default();
        // Build each brick far apart, then stack them.
        build_part(&mut s, &std_fdm, "Below", [-100.0, 0.0, 0.0])?;
        build_part(&mut s, &profile, "Variant", [0.0, 100.0, 0.0])?;
        build_part(&mut s, &std_fdm, "Above", [100.0, 0.0, 0.0])?;
        run(&mut s, "solid.move", json!({"bodies": ["Variant"], "translate": [0.0, -100.0, 0.0]}))?;
        run(&mut s, "solid.move", json!({"bodies": ["Below"], "translate": [100.0, 0.0, -height]}))?;
        run(&mut s, "solid.move", json!({"bodies": ["Above"], "translate": [-100.0, 0.0, height]}))?;
        run(&mut s, "appearance.assign", json!({"bodies": ["Variant"], "color": "#e5622a"}))?;
        for b in ["Below", "Above"] {
            run(&mut s, "appearance.assign", json!({"bodies": [b], "color": "#5b8fd6", "opacity": 0.35}))?;
        }
        let path = out.join(format!("{name}.sc"));
        run(&mut s, "file.save", json!({"path": path}))?;
        println!("{}", path.display());
    }
    Ok(())
}
