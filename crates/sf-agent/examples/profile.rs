use std::time::Instant;
fn main() {
    let t = Instant::now();
    let cat = sf_catalog::CatalogIndex::load_default();
    println!("catalog load {:?} ok={}", t.elapsed(), cat.as_ref().map(|c| c.is_some()).unwrap_or(false));
    let t = Instant::now();
    let dir = sf_ldraw::ensure_installed().unwrap();
    println!("ensure_installed {:?}", t.elapsed());
    let t = Instant::now();
    let m = sf_ldraw::MeshLibrary::open(&dir).unwrap();
    println!("meshlib open {:?}", t.elapsed());
    let t = Instant::now();
    for p in sf_core::CATALOG { let _ = m.mesh(p.id); }
    println!("load all meshes {:?}", t.elapsed());
    let spec = sf_spec::Spec::from_json(&std::fs::read_to_string("../work/snowman.json").unwrap()).unwrap();
    let t = Instant::now();
    let r = sf_engine::build(&spec, &Default::default(), None).unwrap();
    println!("engine build {:?} parts {}", t.elapsed(), r.model.placements.len());
    let t = Instant::now();
    let img = sf_render::render_sheet(&r.model, &sf_core::Palette::builtin(), &m, &sf_agent::session::SHEET_VIEWS, &sf_render::RenderOptions{view_size:384, edges:true, background:[236,238,241]});
    println!("render {:?} {:?}", t.elapsed(), img.dimensions());
}
