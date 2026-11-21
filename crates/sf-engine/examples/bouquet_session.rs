//! Rebuilds a saved session's spec in print mode with the session's detail designs, prints
//! the report and renders front and iso views:
//! `cargo run --release -p sf-engine --example bouquet_session -- <session dir> <out.png> [auto]`.
//! With `auto`, every stem's `bend` is dropped so the engine chooses it.
use sf_engine::{BuildOptions, build};
use sf_render::{RenderOptions, View, render_sheet};
use std::collections::BTreeMap;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let dir = std::path::PathBuf::from(args.next().expect("usage: bouquet_session <session dir> <out.png> [auto]"));
    let out = args.next().unwrap_or_else(|| "/tmp/bouquet-session.png".into());
    let auto = args.next().as_deref() == Some("auto");
    let mut json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("spec.json"))?)?;
    if auto && let Some(stems) = json.pointer_mut("/bouquet/stems").and_then(|v| v.as_array_mut()) {
        for s in stems {
            s.as_object_mut().map(|o| o.remove("bend"));
        }
    }
    let spec: sf_spec::Spec = serde_json::from_value(json)?;
    let mut details = BTreeMap::new();
    for e in std::fs::read_dir(dir.join("details"))? {
        let path = e?.path();
        if path.extension().is_some_and(|x| x == "json")
            && let Ok(d) = serde_json::from_str::<sf_core::DetailDesign>(&std::fs::read_to_string(&path)?)
        {
            details.insert(d.name.clone(), d);
        }
    }
    let budget = std::fs::read_to_string(dir.join("budget.txt")).ok().and_then(|b| b.trim().parse().ok());
    let t = std::time::Instant::now();
    let r = build(&spec, &BuildOptions { print: true, details, budget, ..Default::default() }, None)?;
    println!("built in {:.2?}\n{}", t.elapsed(), r.report.to_text());
    let lib = sf_ldraw::MeshLibrary::open_default().unwrap_or_else(|_| sf_ldraw::MeshLibrary::procedural());
    render_sheet(
        &r.model,
        &sf_core::Palette::builtin(),
        &lib,
        &[View::Front, View::Iso],
        &RenderOptions { view_size: 640, ..Default::default() },
    )
    .save(&out)?;
    std::fs::write(out.replace(".png", ".json"), serde_json::to_string(&r.model)?)?;
    println!("wrote {out}");
    Ok(())
}
