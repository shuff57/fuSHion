use sf_engine::*;
fn main() {
    let spec = sf_spec::Spec::from_json(&std::fs::read_to_string("../work/fox1.json").unwrap()).unwrap();
    let r = build(&spec, &Default::default(), None).unwrap();
    println!("res {:?} dims {:?}", r.report.resolution, r.compiled.grid.dims);
    println!("{}", r.report.to_text());
    let ps = &r.model.placements;
    for g in r.analysis.components.iter().skip(1) {
        let desc: Vec<String> = g.iter().map(|&i| format!("{}@({},{},{})h{}", ps[i].part, ps[i].origin.x, ps[i].origin.y, ps[i].origin.z, ps[i].height())).collect();
        println!("group: {}", desc.join(" "));
        // what's around the first part: below/above occupancy
        let p = &ps[g[0]];
        for (x,z) in p.bottom_sockets() {
            let below = r.compiled.grid.get(sf_core::Cell::new(x, p.origin.y-1, z));
            let owner = ps.iter().position(|q| q.cells().any(|c| c.x==x && c.z==z && c.y==p.origin.y-1));
            println!("   below ({x},{z}) grid={:?} owner={:?}", below, owner.map(|o| (&ps[o].part, ps[o].origin, ps[o].top_studs().contains(&(x,z)))));
        }
    }
}
