//! End to end in print mode: design two details, build a flower pot, export print files
//! and the instruction pamphlet into /tmp/fushion-e2e.
use sf_agent::Session;
use std::path::Path;
fn main() -> anyhow::Result<()> {
    let dir = Path::new("/tmp/fushion-e2e");
    let _ = std::fs::remove_dir_all(dir);
    let mut s = Session::open(dir)?;
    s.set_print(true)?;
    s.set_budget(Some(150))?;
    let t = std::time::Instant::now();
    let (d, png) = s.design_detail(&serde_json::json!({"template": "rose", "params": {}}), "red_flower", "red flower", "Red")?;
    std::fs::write("/tmp/fushion-e2e-detail.png", &png)?;
    let (_, png2) = s.design_detail(&serde_json::json!({"template": "leaf", "params": {"length": 16, "tilt": 30}}), "leaf", "leaf", "Green")?;
    std::fs::write("/tmp/fushion-e2e-leaf.png", &png2)?;
    println!("designs in {:.1}s: {:?} cells {:?}", t.elapsed().as_secs_f32(), d.name, d.cells);
    let spec = r#"{"title":"Flower pot","mode":"sculpture","resolution":12,"shapes":[
      {"shape":"cylinder","center":[0,3,0],"size":[8,6,8],"color":"Reddish Brown"},
      {"shape":"cylinder","center":[0,6.3,0],"size":[9,1,9],"color":"Reddish Brown"},
      {"shape":"cylinder","center":[0,6.6,0],"size":[7.5,0.6,7.5],"color":"Dark Brown"}],
      "details":[
        {"design":"red_flower","at":[-2,7,-2],"color":"Red"},
        {"design":"red_flower","at":[2.5,7,1.5],"color":"Yellow","colors":{"stem":"Dark Green"}},
        {"design":"leaf","at":[-2.5,7,2.5],"color":"Green","turn":2}]}"#;
    let out = s.build_spec(spec)?;
    std::fs::write("/tmp/fushion-e2e-sheet.png", &out.sheet_png)?;
    println!("{}", out.report_text);
    let t = std::time::Instant::now();
    let files = s.export(&dir.join("export"))?;
    println!("export in {:.1}s, {} files", t.elapsed().as_secs_f32(), files.len());
    for f in files.iter().filter(|f| !f.to_string_lossy().ends_with(".3mf")).take(40) { println!("  {}", f.display()); }
    Ok(())
}
