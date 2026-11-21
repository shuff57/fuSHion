//! Builds a spec in print mode with and without curved surfaces, prints timings and part
//! counts, and renders the curved model: `cargo run --release -p sf-engine --example curved
//! -- spec.json [out.png] [resolution]`.
use sf_engine::{BuildOptions, build};
use sf_render::{RenderOptions, View, render_sheet};
use std::time::Instant;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: curved spec.json [out.png] [resolution]");
    let out = args.next().unwrap_or_else(|| "/tmp/fork-a-curved.png".into());
    let res: Option<u32> = args.next().map(|r| r.parse()).transpose()?;
    let spec = sf_spec::Spec::from_json(&std::fs::read_to_string(&path)?)?;
    for curves in [false, true] {
        let opts = BuildOptions { print: true, curves, resolution: res, ..Default::default() };
        let t = Instant::now();
        let r = build(&spec, &opts, None)?;
        let took = t.elapsed();
        let p = r.report.printed.clone().unwrap_or_default();
        let tris: usize = r.model.meshes.values().map(|m| m.indices.len() / 3).sum();
        println!(
            "curves {curves}: {} bricks ({} files), {} curved, {} need supports, {} mesh triangles, built in {took:.2?}",
            p.bricks, p.unique_bricks, p.curved, p.need_supports, tris
        );
        if curves {
            let lib = sf_ldraw::MeshLibrary::open_default().unwrap_or_else(|_| sf_ldraw::MeshLibrary::procedural());
            let t = Instant::now();
            let img = render_sheet(&r.model, &sf_core::Palette::builtin(), &lib, &[View::Front, View::Left, View::Iso], &RenderOptions::default());
            img.save(&out)?;
            println!("rendered {out} in {:.2?}", t.elapsed());
        }
    }
    Ok(())
}
