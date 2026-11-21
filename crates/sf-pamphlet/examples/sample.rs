//! Write the sample pamphlet: `cargo run --release -p sf-pamphlet --example sample -- [out.pdf]`.

use sf_core::Palette;
use sf_ldraw::MeshLibrary;
use sf_pamphlet::{PamphletOptions, sample::sample_model, write_pamphlet};

fn main() -> anyhow::Result<()> {
    let out = std::env::args().nth(1).unwrap_or_else(|| "/tmp/fork-d-sample.pdf".into());
    let meshes = MeshLibrary::open_default().unwrap_or_else(|_| MeshLibrary::procedural());
    let opts = PamphletOptions {
        print_settings: vec![
            "0.4 mm nozzle, 0.2 mm layers with a 0.2 mm first layer.".into(),
            "PETG if you have it (PLA fins may take a set). Print studs up.".into(),
        ],
        ..Default::default()
    };
    let t = std::time::Instant::now();
    let info = write_pamphlet(&sample_model(), &Palette::builtin(), &meshes, &opts, out.as_ref())?;
    println!("{out}: {info:?} in {:.2?} (LDraw library: {})", t.elapsed(), !meshes.is_procedural());
    Ok(())
}
