use super::*;
use crate::sample::sample_model;
use sf_core::Cell;

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Checks a PDF's header, trailer, page count and every xref offset.
fn check_pdf(bytes: &[u8], pages: usize) {
    assert!(bytes.starts_with(b"%PDF-1.4\n"));
    assert!(bytes.ends_with(b"%%EOF\n"));
    let count = format!("/Count {pages} ");
    assert!(find(bytes, count.as_bytes()).is_some(), "no {count}");
    let tail = &bytes[bytes.len().saturating_sub(64)..];
    let t = String::from_utf8_lossy(tail);
    let start: usize = t.split("startxref\n").nth(1).and_then(|s| s.lines().next()).and_then(|s| s.trim().parse().ok()).expect("startxref");
    assert!(bytes[start..].starts_with(b"xref\n"), "startxref points at xref");
    let xref = String::from_utf8_lossy(&bytes[start..]);
    let mut lines = xref.lines().skip(1);
    let n: usize = lines.next().and_then(|l| l.split(' ').nth(1)).and_then(|s| s.parse().ok()).expect("xref size");
    let entries: Vec<&str> = lines.take(n).collect();
    assert_eq!(entries.len(), n);
    for (i, e) in entries.iter().enumerate().skip(1) {
        let off: usize = e[..10].parse().expect("offset");
        let head = format!("{i} 0 obj\n");
        assert!(bytes[off..].starts_with(head.as_bytes()), "object {i} not at {off}");
    }
}

fn meshes() -> MeshLibrary {
    MeshLibrary::procedural()
}

#[test]
fn sample_pamphlet_is_a_valid_pdf() {
    let m = sample_model();
    let (bytes, info) = pamphlet_bytes(&m, &Palette::builtin(), &meshes(), &PamphletOptions { step_image_px: 400, thumb_px: 96, ..Default::default() }).unwrap();
    // Cover, before you build, print list, buy list, steps 1–4 on 2 pages, a sub-build page
    // for each rose (red, then yellow), step 5 adding them, finished.
    assert_eq!(info.pages, 10, "{info:?}");
    assert_eq!(info.steps, 5);
    assert_eq!((info.printed_parts, info.printed_designs), (3, 2));
    assert_eq!((info.detail_parts, info.detail_designs), (1, 1));
    assert_eq!(info.catalog_parts, 7);
    // Two roses in two colour schemes: two sub-builds of one, 6 pieces each; the green hubs
    // share a parts-list row.
    assert_eq!((info.kit_designs, info.kit_uses, info.sub_builds, info.kit_parts), (1, 2, 2, 12));
    assert_eq!(info.step_pages, vec![5, 5, 6, 6, 9]);
    assert_eq!(info.sub_build_pages, vec![7, 8]);
    assert!(info.sub_build_pages.iter().all(|&p| p < info.step_pages[4] && p > info.step_pages[3]));
    check_pdf(&bytes, info.pages);

    // Poppler, when it's installed, must read it too.
    let dir = std::env::temp_dir().join(format!("sf-pamphlet-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("sample.pdf");
    std::fs::write(&path, &bytes).unwrap();
    if let Ok(out) = std::process::Command::new("pdfinfo").arg(&path).output() {
        let s = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "pdfinfo: {}", String::from_utf8_lossy(&out.stderr));
        assert!(s.lines().any(|l| l.starts_with("Pages:") && l.trim_end().ends_with(&format!(" {}", info.pages))), "{s}");
        assert!(out.stderr.is_empty(), "pdfinfo warnings: {}", String::from_utf8_lossy(&out.stderr));
    }
    if let Ok(out) = std::process::Command::new("pdftotext").arg(&path).arg("-").output() {
        let s = String::from_utf8_lossy(&out.stdout);
        for want in [
            "Flower pot (sample)",
            "Before you build",
            "What to print",
            "What to buy",
            "B001",
            "D001",
            "×2",
            "Finished!",
            "page 10 of 10",
            "Build ×1",
            "rose, Red",
            "rose, Yellow",
            "for step 5",
            "rose petal",
            "rose hub",
            "SUPPORTS",
            "Clip the four petals onto the hub.",
            "Built on page 7",
            "Built on page 8",
        ] {
            assert!(s.contains(want), "missing {want:?} in:\n{s}");
        }
        // Page by page: the sub-builds sit on pages 7 and 8, before step 5.
        let pages: Vec<&str> = s.split('\u{c}').collect();
        assert!(pages[6].contains("rose, Red") && pages[7].contains("rose, Yellow"), "{:?}", &pages[6..8]);
        assert!(pages[8].contains("Built on page 7"));
        // Parts list: hubs ×2 (both green), petals ×4 in red and in yellow.
        assert!(pages[2].contains("rose hub"));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn big_steps_get_a_page_and_size_stays_reasonable() {
    // A step with more kinds than fit a half page goes on its own page.
    let mut m = sample_model();
    m.steps = vec![(0..m.placements.len()).collect()];
    let (_, info) = pamphlet_bytes(&m, &Palette::builtin(), &meshes(), &PamphletOptions { step_image_px: 300, thumb_px: 64, max_kinds_half_page: 3, ..Default::default() }).unwrap();
    // Cover, before, print, buy, two sub-builds, one big step page, finished.
    assert_eq!(info.pages, 8, "{info:?}");
    assert_eq!(info.step_pages, vec![7]);
}

#[test]
fn missing_steps_fall_back_to_layers() {
    let mut m = sample_model();
    m.steps.clear();
    let s = steps_of(&m);
    assert_eq!(s.iter().map(Vec::len).sum::<usize>(), m.placements.len());
    for w in s.windows(2) {
        assert!(m.placements[w[0][0]].origin.y <= m.placements[w[1][0]].origin.y);
    }
    // Steps that miss a part are replaced too.
    m.steps = vec![vec![0]];
    assert_eq!(steps_of(&m).iter().map(Vec::len).sum::<usize>(), m.placements.len());
}

/// The corner names match what the picture shows: the named corner is nearest the camera,
/// so it sits lowest in an iso picture.
#[test]
fn corner_names_match_the_view() {
    let corners = [(0, 0, "front left"), (6, 0, "front right"), (6, 6, "back right"), (0, 6, "back left")];
    let mut m = Model::default();
    let mut pal = ScratchPalette::new(&Palette { colors: Vec::new() });
    let codes: Vec<ColorId> = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0]].map(|c| pal.add(c)).to_vec();
    for (k, &(x, z, _)) in corners.iter().enumerate() {
        m.placements.push(Placement::new("3003", codes[k], Cell::new(x, 0, z), Rot(0)));
    }
    let want = |c: [u8; 3], p: &image::Rgba<u8>| (0..3).all(|i| if c[i] == 255 { p[i] > 100 } else { p[i] < 40 });
    for turn in 0..4u8 {
        let img = view::render_turned(&m, &pal.palette, &meshes(), turn, 200);
        let mean_y: Vec<f32> = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0]]
            .iter()
            .map(|&c| {
                let ys: Vec<u32> = img.enumerate_pixels().filter(|(_, _, p)| want(c, p)).map(|(_, y, _)| y).collect();
                assert!(!ys.is_empty(), "turn {turn}: colour {c:?} not visible");
                ys.iter().sum::<u32>() as f32 / ys.len() as f32
            })
            .collect();
        let lowest = (0..4).max_by(|&a, &b| mean_y[a].total_cmp(&mean_y[b])).unwrap();
        assert_eq!(corners[lowest].2, corner(turn), "turn {turn}: {mean_y:?}");
    }
}

#[test]
fn view_choice_prefers_the_side_showing_new_parts() {
    // A wall along the front hides a part placed behind it; the step view should turn.
    let p = |id: &str, x: i32, y: i32, z: i32, r: u8| Placement::new(id, 4, Cell::new(x, y, z), Rot(r));
    let m = Model {
        placements: vec![
            p("3958", 0, 0, 0, 0),
            p("3008", 0, 1, 0, 0),
            p("3008", 0, 4, 0, 0),
            p("3008", 0, 7, 0, 0),
            p("3009", 0, 1, 1, 1),
            p("3009", 0, 4, 1, 1),
            p("3003", 3, 1, 4, 0),
        ],
        steps: vec![vec![0], vec![1, 2, 3, 4, 5], vec![6]],
        ..Default::default()
    };
    let mut pal = ScratchPalette::new(&Palette::builtin());
    let seq = stem::sequence(&m, &m.steps, 1);
    let (sm, new) = stem::scene(&m, &m.steps, &seq, 2, &mut pal, &Palette::builtin(), true);
    let seen0 = view::visible_new(&sm, &new, 0, &meshes(), 160);
    let t = view::best_turn(&sm, &new, &meshes());
    let seen = view::visible_new(&sm, &new, t, &meshes(), 160);
    assert!(seen >= seen0, "{seen} < {seen0}");
    assert_ne!(t, 0, "the front-left view is blocked by the walls (seen {seen0})");
}

/// A brick trimmed to a curved surface reads "curved brick", and gets the SUPPORTS tag when
/// its trim overhangs.
#[test]
fn curved_bricks_are_named_and_tagged() {
    use sf_core::{CustomDef, CustomKind, DetailMesh, Trim};
    let mut m = sample_model();
    let mut mesh = DetailMesh::default();
    crate::sample::box_mesh(&mut mesh, 20.0, 40.0, 0.0, 24.0);
    m.meshes.insert("curve:B009".into(), mesh);
    let def = CustomDef {
        id: "B009".into(),
        kind: CustomKind::Brick,
        cells: vec![(0, 0), (0, 1)],
        colors: Default::default(),
        plates: 3,
        studs: vec![(0, 0), (0, 1)],
        units: Vec::new(),
        sockets: vec![(0, 0), (0, 1)],
        trim: Some(Box::new(Trim { shapes: Vec::new(), protect: Vec::new(), overhang: true })),
        mesh: Some("curve:B009".into()),
    };
    m.placements.push(Placement::printed(def, 322, Cell::new(5, 4, 1), Rot(0)));
    let i = m.placements.len() - 1;
    m.steps[2].push(i);
    let (bytes, info) = pamphlet_bytes(&m, &Palette::builtin(), &meshes(), &PamphletOptions { step_image_px: 300, thumb_px: 64, ..Default::default() }).unwrap();
    assert_eq!((info.curved_parts, info.curved_with_supports), (1, 1), "{info:?}");
    check_pdf(&bytes, info.pages);
    let dir = std::env::temp_dir().join(format!("sf-pamphlet-curved-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("curved.pdf");
    std::fs::write(&path, &bytes).unwrap();
    if let Ok(out) = std::process::Command::new("pdftotext").arg(&path).arg("-").output() {
        let s = String::from_utf8_lossy(&out.stdout);
        let pages: Vec<&str> = s.split('\u{c}').collect();
        assert!(pages[1].contains("Curved bricks are printed bricks"), "{}", pages[1]);
        assert!(pages[1].contains("tagged SUPPORTS"));
        let print = pages[2];
        assert!(print.contains("B009") && print.contains("Curved brick, 2 studs area, 3 plates"), "{print}");
        // Two SUPPORTS tags before: the rose petals in red and yellow; now three.
        assert_eq!(print.matches("SUPPORTS").count(), 3, "{print}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The text of each page, via poppler's pdftotext, whitespace collapsed (None when it isn't
/// installed).
fn page_texts(bytes: &[u8], name: &str) -> Option<Vec<String>> {
    let dir = std::env::temp_dir().join(format!("sf-pamphlet-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join("p.pdf");
    std::fs::write(&path, bytes).ok()?;
    let out = std::process::Command::new("pdftotext").arg(&path).arg("-").output().ok();
    let _ = std::fs::remove_dir_all(&dir);
    let out = out?;
    // Wrapped lines joined back up, so phrases can be matched whole.
    Some(String::from_utf8_lossy(&out.stdout).split('\u{c}').map(|p| p.split_whitespace().collect::<Vec<_>>().join(" ")).collect())
}

/// A bouquet: the stems plug in after the hub, each kind of stem gets a build page just
/// before its first step, identical stems share it, and the print list has them.
#[test]
fn bouquet_stems_get_build_pages_and_steps() {
    let m = crate::sample::bouquet_model();
    let (bytes, info) =
        pamphlet_bytes(&m, &Palette::builtin(), &meshes(), &PamphletOptions { step_image_px: 400, thumb_px: 96, ..Default::default() }).unwrap();
    check_pdf(&bytes, info.pages);
    // Cover, before, print, buy, steps 1–4 (the hub in 4), the rose kit, S01's page, step 5,
    // the S02 ×2 page, steps 6–7, finished.
    assert_eq!(info.pages, 12, "{info:?}");
    assert_eq!(info.steps, 7);
    assert_eq!((info.stems, info.stem_ids, info.stem_builds), (3, 2, 2));
    assert_eq!(info.stem_steps, vec![5, 6, 7]);
    assert_eq!(info.sub_build_pages, vec![7]);
    assert_eq!(info.stem_build_pages, vec![8, 10]);
    assert_eq!(info.step_pages, vec![5, 5, 6, 6, 9, 11, 11]);
    // The rose (a kit) is built once, for S01: 6 pieces. Printed details: the hub, 2 buds, 4 leaves.
    assert_eq!((info.kit_uses, info.kit_parts), (1, 6));
    assert_eq!((info.detail_parts, info.detail_designs), (7, 3));
    let Some(pages) = page_texts(&bytes, "bouquet") else { return };
    let has = |p: usize, s: &str| assert!(pages[p - 1].contains(s), "page {p} lacks {s:?}:\n{}", pages[p - 1]);
    has(2, "Print stems lying flat, as laid out in the file, with the flat side down.");
    has(2, "numbered hole in the hub");
    has(3, "Stems · 3");
    has(3, "S01 rose stem");
    has(3, "140 mm, 2 leaf bars");
    has(3, "S02 stem");
    has(3, "Printed hub, 3 stem holes");
    has(3, "bud");
    has(6, "The hub's holes are numbered");
    has(7, "for stem S01");
    has(8, "S01 rose stem, Green");
    has(8, "Take a rose you built on page 7.");
    has(8, "face pointing up as shown");
    has(8, "onto bar 1 (50 mm up the stem), on the right side");
    has(8, "onto bar 2 (85 mm up the stem), on the left side");
    has(9, "Plug S01 into hole 1.");
    has(9, "Built on page 8");
    has(10, "Build ×2");
    has(10, "for steps 6–7");
    has(11, "Plug S02 into hole 2.");
    has(11, "Plug S02 into hole 3.");
    has(11, "Built on page 10");
}

/// Heads on branch tips: identical heads go on in one step, and each one is a kit to build,
/// so the kit's "Build ×N" counts them all.
#[test]
fn branch_heads_go_on_together_and_count_as_kits() {
    use crate::stem::{Slot, compose, plan};
    let mut m = crate::sample::bouquet_model();
    let sp = &mut m.stems[0];
    sp.def.style = "rose sprig".into();
    sp.def.branches = (0..2)
        .map(|i| sf_core::stem::StemBranch { at: 100.0, angle: if i == 0 { 35.0 } else { -35.0 }, length: 30.0, bend: 0.0, diameter: None, tip_bar: 8.0, tip: None })
        .collect();
    let head = sp.head.clone().unwrap();
    for i in 0..2 {
        let end = sp.def.branch_point(i, 30.0);
        let local = sf_core::KitPose { rotation: crate::stem::rot_z(if i == 0 { 35.0 } else { -35.0 }), position: end };
        let mut h = head.clone();
        h.pose = compose(&sp.pose, &local);
        h.branch = Some(i);
        sp.branch_heads.push(h);
    }
    assert_eq!(plan(&m.stems[0])[0], vec![Slot::Head, Slot::Branch(0), Slot::Branch(1)]);
    let (bytes, info) =
        pamphlet_bytes(&m, &Palette::builtin(), &meshes(), &PamphletOptions { step_image_px: 300, thumb_px: 64, ..Default::default() }).unwrap();
    check_pdf(&bytes, info.pages);
    assert_eq!((info.kit_uses, info.kit_parts), (3, 18), "{info:?}");
    let Some(pages) = page_texts(&bytes, "branches") else { return };
    let all = pages.join("\n");
    assert!(all.contains("Build ×3"), "{all}");
    assert!(all.contains("Take 3 roses you built on page 7."), "{all}");
    assert!(all.contains("Push a rose onto each of the 3 bar ends (the stem's tip and 2 branch ends)"), "{all}");
    assert!(all.contains("S01 rose sprig"), "{all}");
    assert!(all.contains("2 branches"), "{all}");
}
