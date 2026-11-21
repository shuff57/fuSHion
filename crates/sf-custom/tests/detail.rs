//! Detail parts: every template builds into one watertight body on its socket base, and
//! Claude-written batches are held to their cells.

use serde_json::json;
use sf_custom::{
    BASE_HEIGHT, TEMPLATES, analyze_stl, detail_from_commands, detail_template, rebuild,
};

fn check(d: &sf_custom::Detail) {
    let m = analyze_stl(&d.part.stl).unwrap();
    assert!(m.is_watertight(), "{}: not watertight", d.design.name);
    let g = &d.design;
    assert!(!g.mesh.indices.is_empty() && g.mesh.indices.len().is_multiple_of(3));
    assert!(g.sockets.len() >= 2);
    for s in &g.sockets {
        assert!(g.cells.contains(s), "{}: socket {s:?} outside cells {:?}", g.name, g.cells);
    }
    assert!(g.cells.iter().any(|c| c.0 == 0) && g.cells.iter().any(|c| c.1 == 0));
    // The mesh sits in the occupied box: x, z within half the cells' extent (+ the tolerance
    // a part may poke past a column), y below the top of `plates` going down to the floor.
    let w = f32::from(g.cells.iter().map(|c| c.0).max().unwrap() + 1) * 10.0;
    let dd = f32::from(g.cells.iter().map(|c| c.1).max().unwrap() + 1) * 10.0;
    let h = f32::from(g.plates) * 8.0;
    for v in &g.mesh.positions {
        assert!(v[0].abs() <= w + 0.3 && v[2].abs() <= dd + 0.3, "{}: {v:?} off the cells", g.name);
        assert!(v[1] >= -0.2 && v[1] <= h + 0.01, "{}: {v:?} off the plates ({h})", g.name);
    }
    // The base's floor is at y = h (LDU, −Y up) and its top 8 LDU (one plate) above it.
    let max_y = g.mesh.positions.iter().map(|v| v[1]).fold(f32::MIN, f32::max);
    assert!((max_y - h).abs() < 0.01, "{}: floor at {max_y}, expected {h}", g.name);
    eprintln!(
        "{}: cells {:?}, sockets {:?}, {} plates, {:.0} mm³, {} triangles",
        g.name,
        g.cells,
        g.sockets,
        g.plates,
        d.part.volume_mm3,
        g.mesh.indices.len() / 3
    );
}

#[test]
fn every_template_builds_with_defaults() {
    for (name, _) in TEMPLATES {
        let t = std::time::Instant::now();
        let d = detail_template(name, name, name, &json!({}))
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        eprintln!("{name}: {:.2} s", t.elapsed().as_secs_f64());
        check(&d);
        // Rebuilding from the recorded source gives the same part.
        let again = rebuild(&d.design.source.source, name, name).unwrap();
        assert!((again.part.volume_mm3 - d.part.volume_mm3).abs() < 1e-6);
    }
}

#[test]
fn templates_take_their_parameters() {
    let cases = [
        ("flower_head", json!({"petals": 5, "diameter": 22, "cup": 35, "petal_width": 7})),
        ("flower_head", json!({"petals": 8, "cup": 0, "centre_height": 0})),
        ("petal", json!({"length": 14, "tilt": 50, "direction": "-y"})),
        ("leaf", json!({"length": 18, "width": 7, "direction": "+y", "base": [1, 2]})),
        ("dome", json!({"diameter": 7, "height": 3.5})),
        ("cone", json!({"direction": "+x", "length": 10, "diameter": 5, "tilt": 10})),
        ("cone", json!({"direction": "up", "top_diameter": 2})),
        ("round_plate", json!({"diameter": 20, "base": [2, 2]})),
    ];
    for (t, p) in cases {
        let d = detail_template(t, t, t, &p).unwrap_or_else(|e| panic!("{t} {p}: {e}"));
        check(&d);
    }
    // A wide flower occupies the columns its petals reach over.
    let wide = detail_template("f", "f", "flower_head", &json!({"diameter": 22})).unwrap();
    assert!(wide.design.cells.len() > 4, "{:?}", wide.design.cells);
    // A sideways beak reaches past its base.
    let beak = detail_template("b", "b", "cone", &json!({"direction": "+x", "length": 12})).unwrap();
    assert!(beak.design.cells.iter().map(|c| c.0).max().unwrap() >= 2);
}

#[test]
fn bad_template_parameters_are_refused() {
    assert!(detail_template("x", "x", "teapot", &json!({})).is_err());
    assert!(detail_template("x", "x", "dome", &json!({"diameter": 4, "height": 3})).is_err());
    assert!(detail_template("x", "x", "petal", &json!({"direction": "up"})).is_err());
    assert!(detail_template("x", "x", "flower_head", &json!({"petals": 40})).is_err());
    assert!(detail_template("x", "x", "dome", &json!({"base": [1, 1]})).is_err());
}

#[test]
fn claude_written_details_are_held_to_their_cells() {
    // A 2 × 1 base with a little box on top: fine.
    let box_on_top = json!({"commands": [
        {"command": "solid.box", "params": {"length": 6, "width": 4, "height": 3, "corner": [5, 2, BASE_HEIGHT - 0.2], "operation": "join"}}
    ]});
    let d = detail_from_commands(&box_on_top, &[(0, 0), (1, 0)], 2, &[(0, 0), (1, 0)], "box", "box")
        .unwrap();
    check(&d);
    // The same box poking far out of its cells is refused.
    let too_far = json!({"commands": [
        {"command": "solid.box", "params": {"length": 20, "width": 4, "height": 3, "corner": [5, 2, BASE_HEIGHT - 0.2], "operation": "join"}}
    ]});
    let e = detail_from_commands(&too_far, &[(0, 0), (1, 0)], 2, &[(0, 0), (1, 0)], "x", "x")
        .unwrap_err();
    assert!(e.to_string().contains("outside its cells"), "{e}");
    // Something floating above the base is refused (two bodies).
    let floating = json!({"commands": [
        {"command": "solid.box", "params": {"length": 4, "width": 4, "height": 2, "corner": [5, 2, 6], "operation": "new_body"}}
    ]});
    assert!(detail_from_commands(&floating, &[(0, 0), (1, 0)], 3, &[(0, 0), (1, 0)], "x", "x").is_err());
    // A lone socket, or file commands, are refused.
    assert!(detail_from_commands(&box_on_top, &[(0, 0), (1, 0)], 2, &[(0, 0)], "x", "x").is_err());
    let files = json!({"commands": [{"command": "file.export", "params": {"path": "/tmp/x.step"}}]});
    assert!(detail_from_commands(&files, &[(0, 0), (1, 0)], 2, &[(0, 0), (1, 0)], "x", "x").is_err());
    // Material put into the base's hollow is cut away again: the base stays a socket.
    let into_hollow = json!({"commands": [
        {"command": "solid.box", "params": {"length": 10, "width": 3, "height": 1.0, "corner": [3, 2.5, 0.2], "operation": "join"}},
        {"command": "solid.cylinder", "params": {"radius": 2, "height": 4, "base": [8, 4, BASE_HEIGHT - 0.4], "operation": "join"}}
    ]});
    let h = detail_from_commands(&into_hollow, &[(0, 0), (1, 0)], 3, &[(0, 0), (1, 0)], "h", "h");
    // The box in the hollow touches nothing else, so it becomes its own body until the hollow
    // is cut; after the cut the part is one body again.
    let h = h.unwrap();
    check(&h);
}
