//! Install the LDraw cache (if needed) and print each catalog part's raw
//! LDraw bounds, body bounds (studs excluded) and triangle count.
//!
//! `cargo run -p sf-ldraw --example inspect`

use sf_ldraw::MeshLibrary;

fn main() -> anyhow::Result<()> {
    let t = std::time::Instant::now();
    let dir = sf_ldraw::ensure_installed()?;
    println!("library: {} ({:.2?})", dir.display(), t.elapsed());
    let lib = MeshLibrary::open(&dir)?;
    for d in sf_core::CATALOG {
        let t = std::time::Instant::now();
        let Some(m) = lib.mesh(d.id) else {
            println!("{:6} MISSING", d.id);
            continue;
        };
        let (lo, hi) = m.bounds().unwrap_or_default();
        println!(
            "{:6} tris {:5} edges {:4}  x[{:6.1},{:6.1}] y[{:6.1},{:6.1}] z[{:6.1},{:6.1}]  ({:.1?})  size {:?} h {} {:?}",
            d.id,
            m.triangle_count(),
            m.edges.len(),
            lo[0],
            hi[0],
            lo[1],
            hi[1],
            lo[2],
            hi[2],
            t.elapsed(),
            d.size,
            d.height,
            d.shape
        );
    }
    let missing = lib.missing_files();
    if !missing.is_empty() {
        println!("missing subfiles: {missing:?}");
    }
    Ok(())
}
