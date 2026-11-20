use sf_engine::*;
fn main() {
    let spec = sf_spec::Spec::from_json(&std::fs::read_to_string("../work/fox1.json").unwrap()).unwrap();
    let c = sf_spec::compile(&spec, &sf_core::Palette::builtin(), &sf_spec::CompileOptions{resolution: Some(18), image: None}).unwrap();
    for rounds in [0u32, 300, 1000, 3000] {
        let t = std::time::Instant::now();
        let p = pack::pack(&c, &sf_core::UnknownAvailability, &pack::PackOptions{slopes: true, repair_rounds: rounds});
        let m = sf_core::Model{placements: p.placements.clone(), ..Default::default()};
        let a = analyze(&m);
        println!("rounds {rounds}: parts {} floating {} weak {} (held {}) dropped {} anchored {} in {:?}", m.placements.len(), a.floating.len(), a.weak_joints.len(), a.weak_joints.iter().map(|w| w.parts_held).sum::<usize>(), p.dropped, p.anchored, t.elapsed());
    }
}
