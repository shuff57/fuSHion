//! Writes SolidCraft designs (`.sc`) that show each fused-stud brick variant seated between
//! two fused bricks, for review in the SolidCraft desktop app, with print files and renders:
//!
//! `cargo run --release -p sf-custom --example assemblies -- <out_dir>`
//!
//! Each design holds three bodies: `Below` (translucent), the variant in the middle
//! (`Variant`, orange) and `Above` (translucent), stacked as they would clip together.
//! `Below` and `Above` are the same size as the variant, in the default fused profile
//! ([`StudProfile::fused`]: r 0.4 rounds, centre rib). Every brick is built away from the
//! others and moved into place at the end, so no cut of one brick can touch another.
//!
//! For each variant it also writes the variant on its own as `.stl` and `.3mf` (checked like
//! any custom part), `<name>_top.png` (straight down) and `<name>_below.png` (straight up),
//! and prints the plane fit of the variant on `Below` and under `Above` (no collision allowed).
//! The 2×3 also gets `<name>_triple_closeup.png`: one triple from above, straight down and
//! from the fin side.
//!
//! Environment:
//! * `FUSHION_SET=corners`: fused 2×4 r 0.4 variants (centre rib; end-wall ribs only;
//!   both), plus a default 2×3 (one triple per row) and 1×5 (a pair and a triple);
//! * `FUSHION_SIZE=WxD`: the size of the variants that don't fix their own (default 2x4);
//! * `FUSHION_ONLY=a,b,...`: only the variants whose names start with one of these.

#[path = "common/render.rs"]
mod render;

use render::{View, render_one, render_views, triangles};
use serde_json::{Value, json};
use sf_custom::{CustomShape, StudProfile, TopBrick, fit_below, fit_under, plan};
use solidcraft_engine::Session;
use std::path::PathBuf;

type Error = Box<dyn std::error::Error>;

fn run(s: &mut Session, id: &str, params: Value) -> Result<Value, Error> {
    s.execute(id, &params)
        .map_err(|e| format!("{id} {params}: {e}").into())
}

/// Build one part (body "Part"), rename it, park it at `park`.
fn build_part(
    s: &mut Session,
    profile: &StudProfile,
    name: &str,
    park: [f64; 3],
    size: (u32, u32),
) -> Result<(), Error> {
    let shape = CustomShape::Box {
        w: size.0,
        d: size.1,
        plates: 3,
    };
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
        let names: Vec<String> = m["bodies"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|b| b["name"].to_string())
            .collect();
        eprintln!("{name}: {names:?}");
    }
    run(s, "body.rename", json!({"body": "Part", "name": name}))?;
    run(
        s,
        "solid.move",
        json!({"bodies": [name], "translate": park}),
    )?;
    Ok(())
}

/// A variant: its name, profile, and its own size (else `FUSHION_SIZE`).
struct Variant {
    name: String,
    profile: StudProfile,
    size: Option<(u32, u32)>,
}

fn variant(name: &str, profile: StudProfile, size: Option<(u32, u32)>) -> Variant {
    Variant {
        name: name.into(),
        profile,
        size,
    }
}

/// The plane fit of the variant on `Below` and under `Above` (both aligned), checked.
fn check_fit(v: &Variant, size: (u32, u32), neighbour: &StudProfile) -> Result<(), Error> {
    let shape = CustomShape::Box {
        w: size.0,
        d: size.1,
        plates: 3,
    };
    let aligned = TopBrick {
        w: size.0,
        d: size.1,
        at: (0, 0),
    };
    let on = fit_below(&shape, &v.profile, &aligned, neighbour)?;
    let under = fit_under(&shape, &v.profile, &aligned, neighbour)?;
    for (what, r) in [("on Below", &on), ("under Above", &under)] {
        let rib = r
            .rib_clearance
            .map_or("-".to_string(), |c| format!("{c:+.3}"));
        eprintln!(
            "  {what:<12} fin→wall {:.3}  fin→rib {:.3}  rib/stud {rib}  body/wall {:+.3}  body/tube {:+.4}  crossings {:?}",
            r.fin_wall_preload,
            r.fin_rib_preload,
            r.body_wall_clearance,
            r.body_tube_clearance,
            r.crossings
        );
        let ok = r.crossings.is_empty()
            && r.body_wall_clearance >= 0.0
            && r.body_tube_clearance >= 0.0
            && r.fin_wall_preload <= 0.45
            && r.fin_rib_preload <= 0.45
            && r.rib_clearance
                .is_none_or(|c| c >= -v.profile.wall_rib_interference - 1e-6);
        if !ok {
            return Err(format!("{} {what}: collision {r:?}", v.name).into());
        }
    }
    Ok(())
}

fn main() -> Result<(), Error> {
    let out = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "/tmp/fushion-assemblies".into()),
    );
    std::fs::create_dir_all(&out)?;
    let fused = StudProfile::fused();
    let with = |ribs: bool, centre: bool, r: f64| StudProfile {
        wall_ribs: ribs,
        center_ribs: centre,
        edge_radius: r,
        ..fused
    };
    let mut variants = vec![
        variant("fused_default", fused, None),
        variant("fused_sharp", with(false, true, 0.0), None),
        variant("fused_wall_ribs", with(true, true, 0.4), None),
        variant("fused_no_ribs", with(false, false, 0.4), None),
    ];
    // FUSHION_SET=corners: the r 0.4 rib layouts, and the triples.
    if std::env::var("FUSHION_SET").as_deref() == Ok("corners") {
        variants = vec![
            variant("r0.4_centre_rib", with(false, true, 0.4), None),
            variant("r0.4_wall_ribs", with(true, false, 0.4), None),
            variant("r0.4_centre_and_wall_ribs", with(true, true, 0.4), None),
            variant("fused_default", fused, Some((2, 3))),
            variant("fused_default", fused, Some((1, 5))),
        ];
    }
    let size = std::env::var("FUSHION_SIZE")
        .ok()
        .and_then(|v| {
            v.split_once('x')
                .and_then(|(w, d)| Some((w.parse().ok()?, d.parse().ok()?)))
        })
        .unwrap_or((2, 4));
    let only: Option<Vec<String>> = std::env::var("FUSHION_ONLY")
        .ok()
        .map(|v| v.split(',').map(String::from).collect());
    let height = fused.body_height(3);
    for v in variants {
        if only
            .as_ref()
            .is_some_and(|o| !o.iter().any(|k| v.name.starts_with(k.as_str())))
        {
            continue;
        }
        let size = v.size.unwrap_or(size);
        let name = format!("{}x{}_{}", size.0, size.1, v.name);
        // A print file of the variant on its own, checked like any custom part.
        let shape = CustomShape::Box {
            w: size.0,
            d: size.1,
            plates: 3,
        };
        let part = sf_custom::build(&shape, &v.profile)?;
        std::fs::write(out.join(format!("{name}.stl")), &part.stl)?;
        std::fs::write(out.join(format!("{name}.3mf")), &part.three_mf)?;
        eprintln!("{name}: {:.2} mm³", part.volume_mm3);
        check_fit(&v, size, &fused)?;

        // Renders: straight down (shaded over the top 2.4 mm only, so the studs, slots and
        // pockets stand out from the top face), straight up.
        let tris = triangles(&part.stl);
        let (lo, hi) = part.bbox_mm;
        let top = ([lo[0], lo[1], hi[2] - 2.4], hi);
        for (suffix, eye, focus) in [
            ("top", [0.0, 0.0, 1.0], Some(top)),
            ("below", [0.0, 0.0, -1.0], None),
        ] {
            let png = out.join(format!("{name}_{suffix}.png"));
            render_one(&tris, &View { eye, focus }, 900, &png)?;
            println!("{}", png.display());
        }
        if size == (2, 3) {
            // Row 0's triple: its fins face −X.
            let triple = ([0.0, 0.0, 9.0], [8.0, 24.0, 11.4]);
            let views = [
                View {
                    eye: [0.0, 0.0, 1.0],
                    focus: Some(triple),
                },
                View {
                    eye: [-1.0, -0.5, 1.3],
                    focus: Some(triple),
                },
            ];
            let png = out.join(format!("{name}_triple_closeup.png"));
            render_views(&tris, &views, 700, &png)?;
            println!("{}", png.display());
        }

        let mut s = Session::default();
        // Build each brick far apart, then stack them.
        build_part(&mut s, &fused, "Below", [-100.0, 0.0, 0.0], size)?;
        build_part(&mut s, &v.profile, "Variant", [0.0, 100.0, 0.0], size)?;
        build_part(&mut s, &fused, "Above", [100.0, 0.0, 0.0], size)?;
        run(
            &mut s,
            "solid.move",
            json!({"bodies": ["Variant"], "translate": [0.0, -100.0, 0.0]}),
        )?;
        run(
            &mut s,
            "solid.move",
            json!({"bodies": ["Below"], "translate": [100.0, 0.0, -height]}),
        )?;
        run(
            &mut s,
            "solid.move",
            json!({"bodies": ["Above"], "translate": [-100.0, 0.0, height]}),
        )?;
        run(
            &mut s,
            "appearance.assign",
            json!({"bodies": ["Variant"], "color": "#e5622a"}),
        )?;
        for b in ["Below", "Above"] {
            run(
                &mut s,
                "appearance.assign",
                json!({"bodies": [b], "color": "#5b8fd6", "opacity": 0.35}),
            )?;
        }
        let path = out.join(format!("{name}.sc"));
        run(&mut s, "file.save", json!({"path": path}))?;
        println!("{}", path.display());
    }
    Ok(())
}
