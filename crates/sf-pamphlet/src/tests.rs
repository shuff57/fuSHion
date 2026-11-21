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
    // Cover, before you build, print list, buy list, 4 steps on 2 pages, finished.
    assert_eq!(info.pages, 7, "{info:?}");
    assert_eq!(info.steps, 4);
    assert_eq!((info.printed_parts, info.printed_designs), (3, 2));
    assert_eq!((info.detail_parts, info.detail_designs), (1, 1));
    assert_eq!(info.catalog_parts, 7);
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
        for want in ["Flower pot (sample)", "Before you build", "What to print", "What to buy", "B001", "D001", "×2", "Finished!", "page 7 of 7"] {
            assert!(s.contains(want), "missing {want:?} in:\n{s}");
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn big_steps_get_a_page_and_size_stays_reasonable() {
    // A step with more kinds than fit a half page goes on its own page.
    let mut m = sample_model();
    m.steps = vec![(0..m.placements.len()).collect()];
    let (_, info) = pamphlet_bytes(&m, &Palette::builtin(), &meshes(), &PamphletOptions { step_image_px: 300, thumb_px: 64, max_kinds_half_page: 3, ..Default::default() }).unwrap();
    assert_eq!(info.pages, 6, "{info:?}");
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
