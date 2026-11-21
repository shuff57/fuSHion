//! The stem fit coupon: a short bent test stem (a leaf bar and a tip bar), a mini hub with
//! peg holes leaning 0°, 15° and 30°, a leaf with a clip, and a short branched sprig (a bud
//! and a blade). Writes each to `<out>/<NN_name>/` (default `~/Desktop/3D/Lego-Bricks/stems`)
//! with STL, 3MF and a PRINT-LOG.txt (kept if already there), and renders to
//! `/tmp/fushion-stems/`.
//!
//! `cargo run --release -p sf-custom --example stem_fit [out_dir]`

#[path = "common/render.rs"]
mod render;

use serde_json::json;
use sf_core::{Hub, HubHole, StemBar, StemBranch, StemDef, StemTip};
use sf_custom::{BAR_MOUNT_DEPTH, CustomPart, build_hub, build_stem, detail_template, stem_fit};
use std::path::{Path, PathBuf};

fn stem(id: &str, length: f32, bend: f32, diameter: f32) -> StemDef {
    StemDef {
        id: id.into(),
        length,
        bend,
        tip_bar: BAR_MOUNT_DEPTH as f32,
        bars: vec![],
        diameter,
        tip: None,
        branches: vec![],
        style: String::new(),
    }
}

fn write(
    dir: &Path,
    stem_name: &str,
    part: &CustomPart,
    log: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(format!("{stem_name}.stl")), &part.stl)?;
    std::fs::write(dir.join(format!("{stem_name}.3mf")), &part.three_mf)?;
    let log_path = dir.join("PRINT-LOG.txt");
    if !log_path.exists() {
        std::fs::write(log_path, log)?;
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let out = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| format!("{home}/Desktop/3D/Lego-Bricks/stems")),
    );
    let renders = PathBuf::from("/tmp/fushion-stems");
    std::fs::create_dir_all(&renders)?;
    let f = stem_fit();
    println!("fit: {}", serde_json::to_string_pretty(&f)?);
    let mut pngs = Vec::new();
    let mut parts: Vec<(&str, String, CustomPart, String)> = Vec::new();

    // 1. A short bent stem: a leaf bar and a tip bar.
    let mut s1 = stem("S01", 60.0, 20.0, 5.0);
    s1.bars = vec![StemBar {
        at: 30.0,
        length: 5.0,
    }];
    let t = std::time::Instant::now();
    let p1 = build_stem(&s1)?;
    let secs1 = t.elapsed().as_secs_f64();
    parts.push((
        "01_test_stem",
        "fushion_S01".into(),
        p1,
        format!(
            "TEST STEM S01 — 60 mm, bent 20°, Ø5 rod, one leaf bar, a {tip} mm tip bar (built in {secs1:.1} s)
Print: lying flat as it comes (the flat underside on the bed), no supports, 0.2 mm layers,
PLA or PETG, 100 % infill (it's thin: perimeters do the work).
The id \"01\" is engraved under the peg (the S isn't in the font).

Check, and write down what you find:
1. PEG FIT: push the peg into the mini hub's 0° hole (02_mini_hub). Firm push, stops on the
   collar, doesn't turn (the flat is the key)? Too tight / too loose / wobbles?
   (Design: Ø4.8 peg in a Ø5.0 D-hole, three crush ribs {pr:.2} mm into the peg, key flat
   {kc:.2} mm clear, so it turns at most {play:.1}° before the key stops it.)
2. CLIP ON THE BAR: snap the leaf's clip (03_leaf) onto the bar section in the middle.
   Snaps on, holds, turns round the bar with a little effort? Room at the rod's shoulders?
   (Bar Ø3.18 with a {fd:.2} mm flat where it meets the bed; clip grip {ci:.2} mm round it,
   {cif:.2} mm across the flat; {gap:.1} mm to spare each side of the clip.)
3. HEAD ON THE TIP BAR: push any flower head (printed after this change) onto the tip bar.
   Goes in firmly, the rod's end stops at the hole's mouth, the head doesn't spin freely?
   (Bar hole Ø3.48 with three crush ribs {hr:.2} mm into the bar, {depth} mm deep.)
4. STIFFNESS: hold the peg and flick the tip. Too floppy for a 200 mm stem? Snaps where?
   Would Ø4 do (stiff enough) or should long stems be Ø5?

Result:
",
            tip = BAR_MOUNT_DEPTH,
            pr = f.peg_rib_interference,
            kc = f.peg_key_clearance,
            play = f.peg_key_play_deg,
            fd = f.bar_flat_depth,
            ci = f.clip_interference,
            cif = f.clip_interference_flat,
            gap = f.clip_shoulder_gap,
            hr = f.head_rib_interference,
            depth = BAR_MOUNT_DEPTH,
        ),
    ));

    // 2. A mini hub: three holes leaning 0°, 15° and 30° (toward +y), keys toward +x.
    let height = 15.0f32;
    let hole = |x: f32, y: f32, lean: f32| {
        let a = lean.to_radians();
        HubHole {
            top: [x, y, height],
            dir: [0.0, a.sin(), a.cos()],
            key: [1.0, 0.0, 0.0],
        }
    };
    let hub = Hub {
        cells: (0..4u8)
            .flat_map(|i| (0..2u8).map(move |j| (i, j)))
            .collect(),
        height,
        holes: vec![
            hole(6.0, 8.0, 0.0),
            hole(15.5, 9.4, 15.0),
            hole(25.0, 10.6, 30.0),
        ],
    };
    let t = std::time::Instant::now();
    let p2 = build_hub(&hub)?;
    let secs2 = t.elapsed().as_secs_f64();
    parts.push((
        "02_mini_hub",
        "fushion_hub".into(),
        p2,
        format!(
            "MINI HUB — 4×2 studs, 15 mm thick, keyed peg holes leaning 0°, 15° and 30° (built in {secs2:.1} s)
Print: top up, on its socket base, no supports (the holes lean less than 45°).

Check:
1. Does it press onto a 4×2 of fused studs and hold like a brick?
2. PEG FIT in each hole (0°, 15°, 30°): same firmness? The leaning ones print a little oval:
   is 30° still a firm fit, or loose / too tight?
3. KEY: with the peg in, does the stem stand with its flat (and its bend) facing the
   same way every time?
4. The 30° hole: any sagging or stringing inside it?

Result:
"
        ),
    ));

    // 3. A leaf with a clip (the kit leaf's clipped part, printed alone for a stem).
    let t = std::time::Instant::now();
    let leaf = detail_template(
        "leaf",
        "test leaf",
        "leaf",
        &json!({"length": 24, "width": 9}),
    )?;
    let secs3 = t.elapsed().as_secs_f64();
    let kit = leaf.design.kit.as_ref().ok_or("the leaf is a kit")?;
    let li = kit
        .parts
        .iter()
        .position(|p| p.key == "leaf")
        .ok_or("no leaf part")?;
    parts.push((
        "03_leaf",
        "fushion_leaf".into(),
        leaf.kit_prints[li].clone(),
        format!(
            "LEAF WITH A CLIP — 24 × 9 mm (built in {secs3:.1} s)
Print: flat as it comes, no supports (the clip's C is in the bed plane, so its jaws bend
within layers).

Check:
1. Snaps onto the test stem's bar section and holds? Turns round the bar to any angle and
   stays there?
2. Jaws crack after 5 on/off cycles? (Snap strain {strain:.1} %, limit 3 %.)

Result:
",
            strain = f.clip_snap_strain * 100.0
        ),
    ));

    // 4. A branched sprig: a bud and a blade.
    let mut s2 = stem("S02", 70.0, 0.0, 4.0);
    s2.branches = vec![
        StemBranch {
            at: 32.0,
            angle: 35.0,
            length: 22.0,
            bend: -10.0,
            diameter: None,
            tip_bar: 0.0,
            tip: Some(StemTip::Bud {
                length: 9.0,
                width: 5.0,
            }),
        },
        StemBranch {
            at: 46.0,
            angle: -35.0,
            length: 20.0,
            bend: 10.0,
            diameter: None,
            tip_bar: 0.0,
            tip: Some(StemTip::Blade {
                length: 18.0,
                width: 5.0,
            }),
        },
    ];
    let t = std::time::Instant::now();
    let p4 = build_stem(&s2)?;
    let secs4 = t.elapsed().as_secs_f64();
    parts.push((
        "04_branched_sprig",
        "fushion_S02".into(),
        p4,
        format!(
            "BRANCHED SPRIG S02 — 70 mm Ø4 stem, a tip bar, a bud branch and a blade branch (built in {secs4:.1} s)
Print: flat as it comes, no supports.

Check:
1. FORKS: bend each branch back and forth (gently, then harder). Does it crack at the fork,
   or further out? (Each fork has a node 1.25× the stem and a gusset in the crotch.)
2. BUD: is the pointed oval clean on top, flat underneath, its point printed (not stringy)?
3. BLADE: 1.6 mm thick: stiff enough, or floppy? Tip printed to a point?
4. Peg and tip bar as for S01.

Result:
"
        ),
    ));

    for (folder, name, part, log) in &parts {
        let dir = out.join(folder);
        write(&dir, name, part, log)?;
        let png = renders.join(format!("{folder}.png"));
        render::render(&part.stl, &png)?;
        let tris = render::triangles(&part.stl);
        render::render_one(
            &tris,
            &render::View {
                eye: [0.0, 0.0, -1.0],
                focus: None,
            },
            360,
            &renders.join(format!("{folder}_below.png")),
        )?;
        println!(
            "{folder}: {:.0} mm³, box {:?} -> {}",
            part.volume_mm3,
            part.bbox_mm,
            dir.display()
        );
        pngs.push((folder.to_string(), png));
    }
    let mut args = vec!["montage".to_string()];
    for (f, png) in &pngs {
        args.extend(["-label".into(), f.clone(), png.display().to_string()]);
    }
    args.extend([
        "-tile".into(),
        "1x".into(),
        "-geometry".into(),
        "+4+4".into(),
    ]);
    args.push(renders.join("stems.png").display().to_string());
    if std::process::Command::new("magick")
        .args(&args)
        .status()
        .is_ok_and(|s| s.success())
    {
        println!("contact sheet -> {}", renders.join("stems.png").display());
    }
    Ok(())
}
