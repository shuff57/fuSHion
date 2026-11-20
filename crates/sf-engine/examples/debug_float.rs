use sf_engine::*;
use sf_spec::Spec;
fn main() {
    let spec = Spec::from_json(r#"{"title":"snowman","budget":150,"shapes":[
        {"shape":"ellipsoid","center":[0,4,0],"size":[8,8,8],"color":"White"},
        {"shape":"ellipsoid","center":[0,10,0],"size":[5.5,5.5,5.5],"color":"White"}]}"#).unwrap();
    let r = build(&spec, &BuildOptions::default(), None).unwrap();
    let ps = &r.model.placements;
    println!("res {:?} dims {:?}", r.report.resolution, r.compiled.grid.dims);
    for &i in r.analysis.floating.iter() {
        let p = &ps[i];
        let conns: Vec<_> = r.analysis.connections.iter().filter(|c| c.1 == i || c.0 == i).collect();
        println!("float #{i} {} at {:?} rot {:?} fp {:?} h {} conns {:?}", p.part, p.origin, p.rot, p.footprint(), p.height(), conns);
        for (x,z) in p.bottom_sockets() {
            for q in ps.iter() { if q.cells().any(|c| c.x==x && c.z==z && c.y==p.origin.y-1) { print!(" below({x},{z})={}@{:?}", q.part, q.origin); } }
        }
        for (x,z) in p.top_studs() {
            for q in ps.iter() { if q.cells().any(|c| c.x==x && c.z==z && c.y==p.top()) { print!(" above({x},{z})={}@{:?}", q.part, q.origin); } }
        }
        println!();
    }
}
