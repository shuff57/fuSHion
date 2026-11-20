//! `cargo run -p sf-agent --example build_spec -- <spec.json> <session-dir> [image]`
fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let mut s = sf_agent::Session::open(std::path::Path::new(&a[2]))?;
    if let Some(img) = a.get(3) { s.set_image(std::path::Path::new(img))?; }
    let out = s.build_spec(&std::fs::read_to_string(&a[1])?)?;
    println!("{}", out.report_text);
    Ok(())
}
