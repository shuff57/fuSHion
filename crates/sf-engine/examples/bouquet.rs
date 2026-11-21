//! Builds the 7-stem test bouquets (stub head and leaf designs) in print mode: plain stems
//! (`model.json`, `bouquet.png`) and each flower on its own stem (`mixed-model.json`,
//! `mixed.png`), into `dir`: `cargo run --release -p sf-engine --example bouquet -- [dir]`.
use sf_engine::{BuildOptions, bouquet, build};
use sf_render::{RenderOptions, View, render_sheet};

fn main() -> anyhow::Result<()> {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "/tmp/fushion-bouquet".into());
    std::fs::create_dir_all(&dir)?;
    let lib = sf_ldraw::MeshLibrary::open_default().unwrap_or_else(|_| sf_ldraw::MeshLibrary::procedural());
    let runs =
        [(bouquet::seven_stem_spec(), "model.json", "bouquet.png"), (bouquet::mixed_bouquet_spec(), "mixed-model.json", "mixed.png")];
    for (json, model, png) in runs {
        let spec = sf_spec::Spec::from_json(&json)?;
        let r = build(&spec, &BuildOptions { print: true, details: bouquet::stub_designs(), ..Default::default() }, None)?;
        println!("{}", r.report.to_text());
        std::fs::write(format!("{dir}/{model}"), serde_json::to_string_pretty(&r.model)?)?;
        let views = [View::Front, View::Left, View::Iso, View::Top];
        render_sheet(&r.model, &sf_core::Palette::builtin(), &lib, &views, &RenderOptions::default()).save(format!("{dir}/{png}"))?;
        println!("wrote {dir}/{model} and {dir}/{png}");
    }
    Ok(())
}
