//! Builds a spec in print mode with straight bricks and with curved surfaces, prints
//! timings, part counts and repairs, and renders both: `cargo run --release -p sf-engine
//! --example curved -- spec.json [out.png] [resolution|-] [budget]`. The curved render goes
//! to `out.png`, the straight one to `out-straight.png`.
use sf_engine::{BuildOptions, build};
use sf_render::{RenderOptions, View, render_sheet};
use std::time::Instant;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: curved spec.json [out.png] [resolution]");
    let out = args.next().unwrap_or_else(|| "/tmp/fork-a-curved.png".into());
    let res: Option<u32> = args.next().filter(|r| r != "-").map(|r| r.parse()).transpose()?;
    let budget: Option<u32> = args.next().map(|b| b.parse()).transpose()?;
    let spec = sf_spec::Spec::from_json(&std::fs::read_to_string(&path)?)?;
    for curves in [false, true] {
        let opts = BuildOptions { print: true, curves, resolution: res, budget, ..Default::default() };
        let t = Instant::now();
        let r = build(&spec, &opts, None)?;
        let took = t.elapsed();
        let p = r.report.printed.clone().unwrap_or_default();
        let tris: usize = r.model.meshes.values().map(|m| m.indices.len() / 3).sum();
        println!(
            "curves {curves}: {} bricks ({} files), {} curved, {} need supports, {} mesh triangles, built in {took:.2?}",
            p.bricks, p.unique_bricks, p.curved, p.need_supports, tris
        );
        if let Some(c) = p.contacts.as_ref() {
            println!("  {c:?}");
        }
        let open =
            r.model.meshes.iter().filter(|(k, _)| k.starts_with("trim:")).map(|(_, m)| sf_engine::curves::open_edges(m)).sum::<usize>();
        let dropped = r.analysis.floating.len();
        let studs: usize = r.model.placements.iter().map(|p| p.top_studs().len()).sum();
        println!(
            "  {} parts at {:?} studs, {studs} studs, open mesh edges {open}, floating parts {dropped}, balanced {}",
            r.report.parts, r.report.resolution, r.analysis.balance.ok
        );
        for w in r.report.warnings.iter().filter(|w| w.contains("small feature") || w.contains("removed") || w.contains("re-shaped")) {
            println!("  {w}");
        }
        {
            if let Some(dir) = std::path::Path::new(&out).parent() {
                std::fs::create_dir_all(dir)?;
            }
            let out = if curves { out.clone() } else { out.replace(".png", "-straight.png") };
            let lib = sf_ldraw::MeshLibrary::open_default().unwrap_or_else(|_| sf_ldraw::MeshLibrary::procedural());
            let t = Instant::now();
            let img = render_sheet(
                &r.model,
                &sf_core::Palette::builtin(),
                &lib,
                &[View::Front, View::Left, View::Iso, View::Top],
                &RenderOptions::default(),
            );
            img.save(&out)?;
            println!("rendered {out} in {:.2?}", t.elapsed());
        }
    }
    Ok(())
}
