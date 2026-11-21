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
    let (sm, new) = view::step_model(&m, &m.steps, 2, &mut pal, true);
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
