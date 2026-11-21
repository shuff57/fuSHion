//! Flower species (after LEGO's Botanical bouquets): each builds into one watertight body on
//! its socket base, with a display mesh within budget, and says whether it needs supports.

use serde_json::{Value, json};
use sf_custom::{DISPLAY_TRIANGLES, Detail, SPECIES, analyze_stl, detail_template};

fn build(name: &str, params: Value) -> Detail {
    let t = std::time::Instant::now();
    let d = detail_template(name, name, name, &params).unwrap_or_else(|e| panic!("{name} {params}: {e}"));
    let g = &d.design;
    assert!(analyze_stl(&d.part.stl).unwrap().is_watertight(), "{name}: not watertight");
    assert!(g.sockets.len() >= 2, "{name}: sockets {:?}", g.sockets);
    for s in &g.sockets {
        assert!(g.cells.contains(s), "{name}: socket {s:?} outside its cells");
    }
    assert!(g.mesh.indices.len() / 3 <= DISPLAY_TRIANGLES, "{name}: display mesh too big");
    assert!(!g.mesh.indices.is_empty());
    eprintln!(
        "{name}: {:.1} s, {:.0} mm³, supports {} ({:.1} mm²), {} display triangles",
        t.elapsed().as_secs_f64(),
        d.part.volume_mm3,
        d.supports,
        d.overhang_mm2,
        g.mesh.indices.len() / 3
    );
    d
}

#[test]
fn every_species_is_listed() {
    let names: Vec<&str> = SPECIES.iter().map(|s| s.0).collect();
    for n in ["rose", "daisy", "aster", "gerbera", "poppy", "cornflower", "spike", "umbel", "fern", "stem"] {
        assert!(names.contains(&n), "{n} missing");
    }
}

#[test]
fn rose_with_petals_at_45_or_more_needs_no_supports() {
    let d = build("rose", json!({}));
    assert!(!d.supports, "{:.1} mm² of overhang", d.overhang_mm2);
    // About LEGO scale: a ~32 mm head.
    let (lo, hi) = d.part.bbox_mm;
    assert!(hi[0] - lo[0] > 24.0 && hi[0] - lo[0] < 34.0, "{:?}", d.part.bbox_mm);
}

#[test]
fn rose_on_a_stem() {
    let d = build("rose", json!({"stem_height": 12, "open": 0.5, "layers": 1, "inner": 3}));
    assert!(!d.supports, "{:.1} mm²", d.overhang_mm2);
}

#[test]
fn flat_daisy_needs_supports() {
    let d = build("daisy", json!({}));
    assert!(d.supports);
}

#[test]
fn aster() {
    build("aster", json!({}));
}

#[test]
fn gerbera() {
    build("gerbera", json!({}));
}

#[test]
fn poppy() {
    let d = build("poppy", json!({}));
    assert!(!d.supports, "cupped petals at 45° or more: {:.1} mm²", d.overhang_mm2);
}

#[test]
fn cornflower() {
    build("cornflower", json!({}));
}

#[test]
fn spikes() {
    for kind in ["lupin", "snapdragon", "larkspur"] {
        build("spike", json!({"kind": kind, "rings": 5, "height": 24}));
    }
    build("spike", json!({"kind": "lavender", "rings": 4, "florets_per_ring": 4, "height": 16}));
}

#[test]
fn umbel_fern_and_stem() {
    build("umbel", json!({}));
    build("fern", json!({}));
    build("fern", json!({"curl": 120, "pairs": 5, "direction": "-y"}));
    let plain = build("stem", json!({}));
    assert!(!plain.supports);
    build("stem", json!({"angle": 20, "top": "knob", "direction": "+y"}));
}

#[test]
fn bad_species_parameters_are_refused() {
    assert!(detail_template("x", "x", "spike", &json!({"kind": "tulip"})).is_err());
    assert!(detail_template("x", "x", "spike", &json!({"height": 56, "stem_height": 40})).is_err());
    assert!(detail_template("x", "x", "rose", &json!({"petals": 9})).is_err());
    assert!(detail_template("x", "x", "stem", &json!({"top": "flag"})).is_err());
}
