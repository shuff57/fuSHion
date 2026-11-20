//! Exporters: LDraw `.ldr` (write and read back), a parts-list CSV, and a
//! BrickLink wanted-list XML.

use anyhow::{Result, bail};
use sf_catalog::{CatalogIndex, builtin_bricklink_part, normalize_ldraw_id};
use sf_core::{Availability, Cell, ColorId, Model, Palette, Placement, Rot, parts};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

// ---------------------------------------------------------------- LDraw out

/// Format an LDraw number compactly: integers without `.0`, otherwise at
/// most three decimals with trailing zeros removed.
fn fmt_num(v: f32) -> String {
    let r = (f64::from(v) * 1000.0).round() / 1000.0;
    if r == 0.0 || !r.is_finite() {
        return "0".into();
    }
    if r.fract() == 0.0 {
        return format!("{}", r as i64);
    }
    let s = format!("{r:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// A file-name-safe slug: lower-case ASCII alphanumerics joined by `-`.
pub fn slug(title: &str) -> String {
    let mut out = String::new();
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-');
    if out.is_empty() { "model".to_string() } else { out.chars().take(60).collect() }
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn type1_line(p: &Placement) -> String {
    let (pos, m) = p.ldraw_transform();
    let mut s = format!("1 {}", p.color);
    for v in pos.iter().chain(m.iter().flatten()) {
        s.push(' ');
        s.push_str(&fmt_num(*v));
    }
    let _ = write!(s, " {}.dat", p.part);
    s
}

/// Write an LDraw `.ldr` model. Each entry of `model.steps` becomes one step
/// ending in `0 STEP`; placements not covered by any step (or every placement
/// when `steps` is empty) go into a final step. Out-of-range step indices are
/// skipped.
pub fn write_ldr(model: &Model, palette: &Palette) -> String {
    let title = one_line(&model.title);
    let title = if title.is_empty() { "Untitled".to_string() } else { title };
    let mut out = String::new();
    let _ = writeln!(out, "0 {title}");
    let _ = writeln!(out, "0 Name: {}.ldr", slug(&title));
    let _ = writeln!(out, "0 Author: fushion");
    let _ = writeln!(out, "0 // Part geometry comes from the LDraw parts library (www.ldraw.org), licensed CC BY 4.0.");
    let used: BTreeSet<ColorId> = model.placements.iter().map(|p| p.color).collect();
    if !used.is_empty() {
        let names: Vec<String> = used.iter().map(|&c| format!("{} ({c})", palette.name(c))).collect();
        let _ = writeln!(out, "0 // Colours: {}", names.join(", "));
    }

    let n = model.placements.len();
    let mut written = vec![false; n];
    let mut steps: Vec<Vec<usize>> = Vec::new();
    for step in &model.steps {
        let s: Vec<usize> = step
            .iter()
            .copied()
            .filter(|&i| i < n && !std::mem::replace(&mut written[i], true))
            .collect();
        if !s.is_empty() {
            steps.push(s);
        }
    }
    let rest: Vec<usize> = (0..n).filter(|&i| !written[i]).collect();
    if !rest.is_empty() {
        steps.push(rest);
    }
    for step in steps {
        for i in step {
            out.push_str(&type1_line(&model.placements[i]));
            out.push('\n');
        }
        out.push_str("0 STEP\n");
    }
    out
}

// ----------------------------------------------------------------- LDraw in

fn close(a: [[f64; 3]; 3], b: [[f32; 3]; 3]) -> bool {
    a.iter().flatten().zip(b.iter().flatten()).all(|(x, y)| (x - f64::from(*y)).abs() < 1e-3)
}

/// Recover a placement from a type-1 line's part, colour, position and
/// matrix, snapping the position to the grid.
fn placement_from(part: &str, color: ColorId, pos: [f64; 3], m: [[f64; 3]; 3]) -> Result<Placement> {
    let Some(def) = parts::part(part) else { bail!("{part}.dat is not a catalog part") };
    // M = R · Yaw, so R = M · Yawᵀ.
    let yaw = Rot(def.ldraw_yaw).matrix();
    let mut r = [[0.0f64; 3]; 3];
    for (i, row) in r.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = (0..3).map(|k| m[i][k] * f64::from(yaw[j][k])).sum();
        }
    }
    let Some(rot) = (0..4u8).map(Rot).find(|rt| close(r, rt.matrix())) else {
        bail!("rotation is not a quarter turn about the vertical axis");
    };
    let rm = rot.matrix();
    let off = def.ldraw_offset.map(f64::from);
    let ro: Vec<f64> = (0..3).map(|i| (0..3).map(|k| f64::from(rm[i][k]) * off[k]).sum()).collect();
    let tmp = Placement::new(part, color, Cell::new(0, 0, 0), rot);
    let [fx, fz] = tmp.footprint();
    let stud = f64::from(sf_core::LDU_PER_STUD);
    let plate = f64::from(sf_core::LDU_PER_PLATE);
    let to_i = |v: f64| -> Result<i32> {
        let r = v.round();
        if !r.is_finite() || r.abs() > 1e6 {
            bail!("position out of range");
        }
        Ok(r as i32)
    };
    // centre = (2·origin + footprint)·stud/2
    let ox = to_i(((pos[0] - ro[0]) / stud * 2.0 - f64::from(fx)) / 2.0)?;
    let oz = to_i(((pos[2] - ro[2]) / stud * 2.0 - f64::from(fz)) / 2.0)?;
    let top = to_i(-(pos[1] - ro[1]) / plate)?;
    Ok(Placement::new(part, color, Cell::new(ox, top - tmp.height(), oz), rot))
}

/// Parse an `.ldr` written by [`write_ldr`] (or any single-file model built
/// from catalog parts). `0 STEP`/`0 ROTSTEP` split steps; the first plain
/// comment line becomes the title. Errors name the offending line.
pub fn read_ldr(text: &str) -> Result<Model> {
    let mut model = Model::default();
    let mut step: Vec<usize> = Vec::new();
    let mut have_title = false;
    for (ln, raw) in text.lines().enumerate() {
        let line = raw.trim().trim_start_matches('\u{feff}');
        let mut tok = line.split_whitespace();
        match tok.next() {
            Some("0") => {
                let rest = line[1..].trim();
                let head = rest.split_whitespace().next().unwrap_or("");
                if head == "STEP" || head == "ROTSTEP" {
                    if !step.is_empty() {
                        model.steps.push(std::mem::take(&mut step));
                    }
                } else if !have_title
                    && !rest.is_empty()
                    && !head.ends_with(':')
                    && !head.starts_with('!')
                    && !head.starts_with("//")
                    && head != "BFC"
                {
                    model.title = rest.to_string();
                    have_title = true;
                }
            }
            Some("1") => {
                let f: Vec<&str> = tok.collect();
                if f.len() < 14 {
                    bail!("line {}: type-1 line needs 14 fields", ln + 1);
                }
                let color: ColorId = f[0]
                    .parse()
                    .map_err(|_| anyhow::anyhow!("line {}: unsupported colour {:?}", ln + 1, f[0]))?;
                let mut nums = [0.0f64; 12];
                for (k, n) in nums.iter_mut().enumerate() {
                    *n = f[k + 1]
                        .parse::<f64>()
                        .ok()
                        .filter(|v| v.is_finite())
                        .ok_or_else(|| anyhow::anyhow!("line {}: bad number {:?}", ln + 1, f[k + 1]))?;
                }
                let part = normalize_ldraw_id(&f[13..].join(" "));
                let m = [[nums[3], nums[4], nums[5]], [nums[6], nums[7], nums[8]], [nums[9], nums[10], nums[11]]];
                let p = placement_from(&part, color, [nums[0], nums[1], nums[2]], m)
                    .map_err(|e| anyhow::anyhow!("line {}: {e}", ln + 1))?;
                step.push(model.placements.len());
                model.placements.push(p);
            }
            _ => {}
        }
    }
    if !step.is_empty() {
        model.steps.push(step);
    }
    Ok(model)
}

// ---------------------------------------------------------------- part lists

/// `(ldraw part, colour) → quantity`, sorted by quantity desc, then part, colour.
fn tally(model: &Model) -> Vec<((String, ColorId), u32)> {
    let mut m: BTreeMap<(String, ColorId), u32> = BTreeMap::new();
    for p in &model.placements {
        *m.entry((p.part.clone(), p.color)).or_default() += 1;
    }
    let mut v: Vec<_> = m.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    v
}

fn bl_part<'a>(catalog: Option<&'a CatalogIndex>, part: &str) -> Option<&'a str> {
    catalog.and_then(|c| c.bricklink_part(part)).or_else(|| builtin_bricklink_part(part))
}

fn bl_color(catalog: Option<&CatalogIndex>, palette: &Palette, code: ColorId) -> Option<u32> {
    catalog.and_then(|c| c.bricklink_color(code)).or_else(|| palette.get(code).and_then(|c| c.bricklink))
}

/// Parts list as CSV with columns `ldraw_part, part_name, bricklink_part,
/// color, ldraw_color, bricklink_color, quantity, sets_in_this_color, note`,
/// one row per (part, colour), largest quantity first. Availability columns
/// are blank when `catalog` is `None` or does not know the pair.
pub fn write_parts_csv(model: &Model, palette: &Palette, catalog: Option<&CatalogIndex>) -> String {
    let mut w = csv::Writer::from_writer(Vec::new());
    let _ = w.write_record([
        "ldraw_part",
        "part_name",
        "bricklink_part",
        "color",
        "ldraw_color",
        "bricklink_color",
        "quantity",
        "sets_in_this_color",
        "note",
    ]);
    for ((part, code), qty) in tally(model) {
        let def = parts::part(&part);
        let sets = catalog.and_then(|c| c.set_count(&part, code));
        let note = if def.is_none() {
            "not a catalog part".to_string()
        } else if let Some(c) = catalog {
            match c.is_made(&part, code) {
                Some(false) => "never produced in this colour".to_string(),
                Some(true) if c.is_rare(&part, code) => {
                    let year = c.last_year(&part, code).map(|y| format!(", last {y}")).unwrap_or_default();
                    format!("rare: {} sets{year}", sets.unwrap_or(0))
                }
                _ => String::new(),
            }
        } else {
            String::new()
        };
        let _ = w.write_record([
            part.as_str(),
            def.map_or("", |d| d.name),
            bl_part(catalog, &part).unwrap_or(""),
            &palette.name(code),
            &code.to_string(),
            &bl_color(catalog, palette, code).map(|c| c.to_string()).unwrap_or_default(),
            &qty.to_string(),
            &sets.map(|s| s.to_string()).unwrap_or_default(),
            &note,
        ]);
    }
    w.into_inner().ok().and_then(|b| String::from_utf8(b).ok()).unwrap_or_default()
}

fn xml_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            c => o.push(c),
        }
    }
    o
}

/// BrickLink wanted-list XML (no `<?xml?>` declaration), one `<ITEM>` per
/// BrickLink (part, colour). Part ids come from `catalog`, else the builtin
/// verified table; colour ids from `catalog`, else `palette`. Fails, listing
/// every unmapped part and colour, rather than guessing.
pub fn write_bricklink_xml(model: &Model, catalog: Option<&CatalogIndex>, palette: &Palette) -> Result<String> {
    let mut missing_parts = BTreeSet::new();
    let mut missing_colors = BTreeSet::new();
    let mut items: Vec<((String, u32), u32)> = Vec::new();
    for ((part, code), qty) in tally(model) {
        let p = bl_part(catalog, &part);
        let c = bl_color(catalog, palette, code);
        if p.is_none() {
            missing_parts.insert(part.clone());
        }
        if c.is_none() {
            missing_colors.insert(code);
        }
        if let (Some(p), Some(c)) = (p, c) {
            match items.iter_mut().find(|(k, _)| k.0 == p && k.1 == c) {
                Some(e) => e.1 += qty,
                None => items.push(((p.to_string(), c), qty)),
            }
        }
    }
    if !missing_parts.is_empty() || !missing_colors.is_empty() {
        let mut msg = String::from("no BrickLink mapping for");
        if !missing_parts.is_empty() {
            let _ = write!(msg, " parts: {}", missing_parts.into_iter().collect::<Vec<_>>().join(", "));
        }
        if !missing_colors.is_empty() {
            let list: Vec<String> = missing_colors.iter().map(|&c| format!("{} ({c})", palette.name(c))).collect();
            let _ = write!(msg, "{}LDraw colours: {}", if msg.ends_with("for") { " " } else { "; " }, list.join(", "));
        }
        bail!(msg);
    }
    items.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let mut out = String::from("<INVENTORY>\n");
    for ((p, c), q) in items {
        let _ = writeln!(
            out,
            "<ITEM><ITEMTYPE>P</ITEMTYPE><ITEMID>{}</ITEMID><COLOR>{c}</COLOR><MINQTY>{q}</MINQTY></ITEM>",
            xml_escape(&p)
        );
    }
    out.push_str("</INVENTORY>\n");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sf_catalog::{ColorEntry, ColorStat, PartEntry};
    use sf_core::CATALOG;

    fn model() -> Model {
        Model {
            title: "My  Cat\nStatue!".into(),
            placements: vec![
                Placement::new("3001", 4, Cell::new(0, 0, 0), Rot(0)),
                Placement::new("3001", 4, Cell::new(0, 3, 0), Rot(1)),
                Placement::new("3024", 15, Cell::new(5, 0, 5), Rot(0)),
                Placement::new("3070b", 25, Cell::new(5, 1, 5), Rot(2)),
            ],
            ..Default::default()
        }
    }

    fn catalog() -> CatalogIndex {
        let entry = |l: &str, bl: &str, cols: &[(ColorId, u32, u16)]| PartEntry {
            ldraw: l.into(),
            rebrickable: Some(l.into()),
            bricklink: Some(bl.into()),
            aliases: vec![],
            colors: cols.iter().map(|&(c, n, y)| (c, ColorStat(n, y))).collect(),
        };
        let col = |l: ColorId, name: &str, bl| ColorEntry {
            ldraw: l,
            rebrickable: l as i32,
            name: name.into(),
            rgb: "000000".into(),
            trans: false,
            bricklink: Some(bl),
        };
        CatalogIndex::new(
            "2026-11-20".into(),
            100,
            vec![entry("3001", "3001", &[(4, 900, 2026)]), entry("3024", "3024", &[(15, 2, 2001)]), entry("3070b", "3070", &[])],
            vec![col(4, "Red", 5), col(15, "White", 1), col(25, "Orange", 4)],
        )
    }

    #[test]
    fn numbers_are_compact() {
        assert_eq!(fmt_num(40.0), "40");
        assert_eq!(fmt_num(-24.0), "-24");
        assert_eq!(fmt_num(-0.0), "0");
        assert_eq!(fmt_num(0.5), "0.5");
        assert_eq!(fmt_num(1.0 / 3.0), "0.333");
        assert_eq!(fmt_num(f32::NAN), "0");
    }

    #[test]
    fn ldr_header_and_lines() {
        let s = write_ldr(&model(), &Palette::builtin());
        let lines: Vec<&str> = s.lines().collect();
        assert_eq!(lines[0], "0 My Cat Statue!");
        assert_eq!(lines[1], "0 Name: my-cat-statue.ldr");
        assert_eq!(lines[2], "0 Author: fushion");
        assert!(lines[3].contains("CC BY 4.0"));
        assert!(s.contains("1 4 40 -24 20 1 0 0 0 1 0 0 0 1 3001.dat\n"), "{s}");
        assert_eq!(s.matches("0 STEP").count(), 1);
        assert!(s.ends_with("0 STEP\n"));
    }

    #[test]
    fn ldr_steps_and_bad_indices() {
        let mut m = model();
        m.steps = vec![vec![2, 99], vec![], vec![0, 0]];
        let s = write_ldr(&m, &Palette::builtin());
        assert_eq!(s.matches("0 STEP").count(), 3, "two given steps plus leftovers");
        let back = read_ldr(&s).unwrap();
        assert_eq!(back.steps, vec![vec![0], vec![1], vec![2, 3]]);
        assert_eq!(back.placements[0], m.placements[2]);
        assert_eq!(back.title, "My Cat Statue!");
    }

    #[test]
    fn empty_model() {
        let s = write_ldr(&Model::default(), &Palette::builtin());
        assert!(s.starts_with("0 Untitled\n0 Name: untitled.ldr\n"));
        let back = read_ldr(&s).unwrap();
        assert!(back.placements.is_empty() && back.steps.is_empty());
    }

    /// xorshift64*: deterministic, dependency-free.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }
        fn range(&mut self, lo: i32, hi: i32) -> i32 {
            lo + (self.next() % (hi - lo) as u64) as i32
        }
    }

    #[test]
    fn ldr_round_trip_random_models() {
        let pal = Palette::builtin();
        let mut rng = Rng(0x5eed_1234_abcd_0001);
        for _ in 0..200 {
            let n = rng.range(0, 40) as usize;
            let placements: Vec<Placement> = (0..n)
                .map(|_| {
                    let def = &CATALOG[rng.range(0, CATALOG.len() as i32) as usize];
                    let col = pal.colors[rng.range(0, pal.colors.len() as i32) as usize].code;
                    let o = Cell::new(rng.range(-30, 30), rng.range(-5, 60), rng.range(-30, 30));
                    Placement::new(def.id, col, o, Rot(rng.range(0, 4) as u8))
                })
                .collect();
            let m = Model { title: "rt".into(), placements, ..Default::default() };
            let back = read_ldr(&write_ldr(&m, &pal)).unwrap();
            assert_eq!(back.placements, m.placements);
        }
    }

    #[test]
    fn read_rejects_bad_input() {
        let tilt = "1 4 0 0 0 1 0 0 0 0.7071 -0.7071 0 0.7071 0.7071 3001.dat";
        assert!(read_ldr(tilt).unwrap_err().to_string().contains("quarter turn"));
        let mirror = "1 4 0 0 0 -1 0 0 0 1 0 0 0 1 3001.dat";
        assert!(read_ldr(mirror).is_err());
        assert!(read_ldr("1 4 0 0 0 1 0 0 0 1 0 0 0 1 973.dat").unwrap_err().to_string().contains("line 1"));
        assert!(read_ldr("1 4 0 0").is_err());
        assert!(read_ldr("1 0x2FF0000 0 0 0 1 0 0 0 1 0 0 0 1 3001.dat").is_err());
        assert!(read_ldr("1 4 0 x 0 1 0 0 0 1 0 0 0 1 3001.dat").is_err());
        assert!(read_ldr("1 4 0 1e30 0 1 0 0 0 1 0 0 0 1 3001.dat").is_err());
        // Case, path prefixes and slightly-off positions are tolerated.
        let m = read_ldr("0 Hi\n1 4 40.2 -23.9 20 1 0 0 0 1 0 0 0 1 parts\\3001.DAT\n").unwrap();
        assert_eq!(m.placements, vec![Placement::new("3001", 4, Cell::new(0, 0, 0), Rot(0))]);
    }

    #[test]
    fn parts_csv_groups_and_notes() {
        let mut m = model();
        m.placements.push(Placement::new("9999", 4, Cell::new(9, 0, 9), Rot(0)));
        let pal = Palette::builtin();
        let s = write_parts_csv(&m, &pal, Some(&catalog()));
        let lines: Vec<&str> = s.lines().collect();
        assert_eq!(
            lines[0],
            "ldraw_part,part_name,bricklink_part,color,ldraw_color,bricklink_color,quantity,sets_in_this_color,note"
        );
        assert_eq!(lines[1], "3001,Brick 2 x 4,3001,Red,4,5,2,900,");
        assert!(lines.contains(&"3024,Plate 1 x 1,3024,White,15,1,1,2,\"rare: 2 sets, last 2001\""), "{s}");
        assert!(lines.contains(&"3070b,Tile 1 x 1,3070,Orange,25,4,1,0,never produced in this colour"), "{s}");
        assert!(lines.contains(&"9999,,,Red,4,5,1,,not a catalog part"), "{s}");
        // Without a catalog the availability columns are blank.
        let s = write_parts_csv(&model(), &pal, None);
        assert!(s.contains("3070b,Tile 1 x 1,3070,Orange,25,4,1,,\n"), "{s}");
    }

    #[test]
    fn bricklink_xml() {
        let pal = Palette::builtin();
        let x = write_bricklink_xml(&model(), Some(&catalog()), &pal).unwrap();
        assert!(!x.contains("<?xml"));
        assert!(x.starts_with("<INVENTORY>\n<ITEM><ITEMTYPE>P</ITEMTYPE><ITEMID>3001</ITEMID><COLOR>5</COLOR><MINQTY>2</MINQTY></ITEM>\n"), "{x}");
        assert!(x.contains("<ITEMID>3070</ITEMID><COLOR>4</COLOR>"), "BrickLink ids, not LDraw: {x}");
        assert!(x.ends_with("</INVENTORY>\n"));
        // Without a catalog the builtin tables are used.
        assert_eq!(write_bricklink_xml(&model(), None, &pal).unwrap(), x);
    }

    #[test]
    fn bricklink_xml_refuses_to_guess() {
        let mut m = model();
        m.placements.push(Placement::new("9999", 4, Cell::new(9, 0, 9), Rot(0)));
        m.placements.push(Placement::new("3001", 1234, Cell::new(9, 0, 9), Rot(0)));
        let e = write_bricklink_xml(&m, None, &Palette::builtin()).unwrap_err().to_string();
        assert!(e.contains("parts: 9999") && e.contains("(1234)"), "{e}");
    }

    #[test]
    fn xml_escapes() {
        assert_eq!(xml_escape("a<b>&\"'"), "a&lt;b&gt;&amp;&quot;&apos;");
    }
}
