//! Custom printable parts: which parts of a model need one, and generating
//! them with `sf-custom` (SolidCraft). Built only with the `custom` feature;
//! without it every generator returns an error saying so.

use anyhow::Result;
use serde_json::Value;
use sf_core::{Availability, ColorId, Model, Palette};
use std::path::{Path, PathBuf};

/// Whether this build can generate custom parts.
pub const ENABLED: bool = cfg!(feature = "custom");

/// Part/colour pairs found in this many sets or fewer are worth printing.
const RARE_SETS: u32 = 3;

/// A part/colour pair in the model that would be better printed.
#[derive(Clone, Debug)]
pub struct CustomNeed {
    pub part: String,
    pub color: ColorId,
    pub count: usize,
    pub reason: String,
}

/// Parts of `model` whose colour was never produced or is rare.
pub fn custom_needs(model: &Model, availability: &dyn Availability) -> Vec<CustomNeed> {
    let mut counts: std::collections::BTreeMap<(String, ColorId), usize> = Default::default();
    for p in &model.placements {
        *counts.entry((p.part.clone(), p.color)).or_default() += 1;
    }
    counts
        .into_iter()
        .filter_map(|((part, color), count)| {
            let reason = match (availability.is_made(&part, color), availability.set_count(&part, color)) {
                (Some(false), _) => "colour never produced".to_string(),
                (_, Some(n)) if n <= RARE_SETS => format!("rare (in {n} sets)"),
                _ => return None,
            };
            Some(CustomNeed { part, color, count, reason })
        })
        .collect()
}

/// A generated part written to disk.
#[derive(Clone, Debug)]
pub struct MadePart {
    pub stl: PathBuf,
    pub three_mf: PathBuf,
    pub volume_mm3: f64,
}

fn slug(s: &str) -> String {
    crate::session::slug(s)
}

/// Grams of PLA per mm³ (1.24 g/cm³, solid).
pub const PLA_G_PER_MM3: f64 = 1.24e-3;

/// One line of a print set's list: a printed part in one colour, or a catalog part to buy.
#[derive(Clone, Debug, serde::Serialize)]
pub struct PrintRow {
    /// The printed part's id (`"B017"`, `"D003"`), or the LDraw id of a part to buy.
    pub id: String,
    /// `"brick"`, `"plate"`, `"detail"` or `"buy"`.
    pub kind: String,
    pub description: String,
    pub colour: String,
    /// How many to print (or buy) in this colour.
    pub count: usize,
    pub plates: u32,
    /// Footprint `[x, z]` in studs (the bounding box of its cells).
    pub size_studs: [u8; 2],
    /// One part's volume, mm³ (0 for parts to buy).
    pub volume_mm3: f64,
    /// One part's weight in solid PLA, g.
    pub grams: f64,
    /// Whether it needs supports printed standing on its base (details with overhangs
    /// flatter than 45°; bricks never do: their ceilings bridge like any brick's).
    pub supports: bool,
    /// The STL, relative to the print set's folder (`None` for parts to buy).
    pub file: Option<String>,
}

/// What [`export_print_set`] wrote.
#[derive(Clone, Debug)]
pub struct PrintSet {
    /// Every file written (STLs, 3MFs and the list).
    pub files: Vec<PathBuf>,
    pub rows: Vec<PrintRow>,
    /// `print-list.csv`.
    pub list: PathBuf,
    /// Parts that could not be made, and why.
    pub notes: Vec<String>,
}

#[cfg(feature = "custom")]
mod imp {
    use super::*;
    use anyhow::{Context, bail};
    use sf_core::Shape;
    use sf_custom::{CustomShape, StudProfile};

    /// Every custom part is made with the fused-stud profile (pairs and triples with
    /// leaf-spring fins, r 0.4 mm vertical edges); plain round studs are not offered.
    pub(super) fn profile() -> StudProfile {
        StudProfile::fused()
    }

    /// The printable equivalent of a catalog part, if there is one.
    pub(super) fn shape_for(part: &str) -> Option<CustomShape> {
        let d = sf_core::parts::part(part)?;
        let plates = d.height as u32;
        match d.shape {
            Shape::Box | Shape::Tile => Some(CustomShape::Box { w: d.size[0] as u32, d: d.size[1] as u32, plates }),
            // Our slopes fall along x; sf-custom's fall along -Y.
            Shape::Slope { high } if high >= 1 && d.size[0] >= 2 => {
                Some(CustomShape::Slope45 { w: d.size[1] as u32, d: d.size[0] as u32, plates })
            }
            _ => None,
        }
    }

    pub(super) fn write(part: &sf_custom::CustomPart, out: &Path, name: &str) -> Result<MadePart> {
        std::fs::create_dir_all(out)?;
        let stl = out.join(format!("{name}.stl"));
        let three_mf = out.join(format!("{name}.3mf"));
        std::fs::write(&stl, &part.stl).with_context(|| format!("writing {}", stl.display()))?;
        std::fs::write(&three_mf, &part.three_mf)?;
        Ok(MadePart { stl, three_mf, volume_mm3: part.volume_mm3 })
    }

    pub(super) fn make(shape: &Value, out: &Path, name: &str) -> Result<MadePart> {
        let shape: CustomShape = serde_json::from_value(shape.clone())
            .context("`shape` must be {\"kind\": \"box\"|\"slope45\"|\"curve\"|\"footprint\", ...}")?;
        let part = sf_custom::build(&shape, &profile()).map_err(|e| anyhow::anyhow!("{e}"))?;
        write(&part, out, name)
    }

    pub(super) fn from_commands(batch: &Value, out: &Path, name: &str) -> Result<MadePart> {
        let part = sf_custom::build_from_commands(batch).map_err(|e| anyhow::anyhow!("{e}"))?;
        write(&part, out, name)
    }

    /// The print of one printed part: a brick from its cells and units (labelled with its
    /// id), a detail rebuilt from its recorded source.
    pub(super) fn print_part(def: &sf_core::CustomDef, model: &Model) -> Result<sf_custom::CustomPart> {
        match &def.kind {
            sf_core::CustomKind::Brick => {
                let shape = CustomShape::Printed {
                    cells: def.cells.iter().map(|&(a, b)| (i32::from(a), i32::from(b))).collect(),
                    plates: u32::from(def.plates),
                    units: def
                        .units
                        .iter()
                        .map(|u| u.iter().map(|&(a, b)| (i32::from(a), i32::from(b))).collect())
                        .collect(),
                    label: Some(def.id.clone()),
                };
                sf_custom::build(&shape, &profile()).map_err(|e| anyhow::anyhow!("{}: {e}", def.id))
            }
            sf_core::CustomKind::Detail { mesh, .. } => {
                let Some(src) = model.details.get(mesh) else {
                    bail!("{}: the model has no design `{mesh}`", def.id)
                };
                sf_custom::rebuild_detail(&src.source).map_err(|e| anyhow::anyhow!("{}: {e}", def.id))
            }
        }
    }

    /// A detail's socket ceiling (1.8 mm) plus a hair: overhangs are measured above it.
    const DETAIL_CEILING_MM: f64 = 1.85;

    pub(super) fn print_set(
        model: &Model,
        palette: &Palette,
        out: &Path,
        progress: &(dyn Fn(usize, usize) + Sync),
    ) -> Result<PrintSet> {
        use std::collections::BTreeMap;
        use std::sync::Mutex;
        use std::sync::atomic::{AtomicUsize, Ordering};
        // Unique printed parts by id (each id is one geometry), and their colours.
        let mut defs: BTreeMap<String, sf_core::CustomDef> = BTreeMap::new();
        let mut uses: BTreeMap<(String, ColorId), usize> = BTreeMap::new();
        let mut buy: BTreeMap<(String, ColorId), usize> = BTreeMap::new();
        // Kit parts by (design, part index, colour): how many to print, and which detail ids
        // use them.
        let mut kit_uses: BTreeMap<(String, usize, ColorId), usize> = BTreeMap::new();
        let mut kit_ids: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for p in &model.placements {
            match &p.custom {
                Some(c) if matches!(&c.kind, sf_core::CustomKind::Detail { mesh, .. } if model.kits.contains_key(mesh)) => {
                    let sf_core::CustomKind::Detail { mesh, .. } = &c.kind else { unreachable!() };
                    let kit = &model.kits[mesh];
                    for (i, part) in kit.parts.iter().enumerate() {
                        let colour = kit.role_color(&part.role, p.color, &c.colors, |n| palette.resolve(n));
                        *kit_uses.entry((mesh.clone(), i, colour)).or_default() += part.copies.len();
                    }
                    let ids = kit_ids.entry(mesh.clone()).or_default();
                    if !ids.contains(&c.id) {
                        ids.push(c.id.clone());
                    }
                }
                Some(c) => {
                    if let Some(prev) = defs.get(&c.id) {
                        let same = prev.kind == c.kind
                            && prev.cells == c.cells
                            && prev.plates == c.plates
                            && prev.units == c.units
                            && prev.studs == c.studs;
                        if !same {
                            bail!("two different parts share the id {}", c.id);
                        }
                    } else {
                        defs.insert(c.id.clone(), (**c).clone());
                    }
                    *uses.entry((c.id.clone(), p.color)).or_default() += 1;
                }
                None => *buy.entry((p.part.clone(), p.color)).or_default() += 1,
            }
        }
        std::fs::create_dir_all(out)?;
        // Jobs: each printed part's id, then each kit part (keyed "kit:<design>:<index>").
        enum Job<'a> {
            Def(&'a sf_core::CustomDef),
            Kit(&'a str, usize),
        }
        let kit_parts: std::collections::BTreeSet<(String, usize)> =
            kit_uses.keys().map(|(m, i, _)| (m.clone(), *i)).collect();
        let jobs: Vec<Job> = defs
            .values()
            .filter(|d| uses.keys().any(|(id, _)| id == &d.id))
            .map(Job::Def)
            .chain(kit_parts.iter().map(|(m, i)| Job::Kit(m.as_str(), *i)))
            .collect();
        let total = jobs.len();
        let results: Mutex<BTreeMap<String, Result<sf_custom::CustomPart>>> = Mutex::new(BTreeMap::new());
        let next = AtomicUsize::new(0);
        let done = AtomicUsize::new(0);
        progress(0, total);
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 8).min(total.max(1));
        std::thread::scope(|scope| {
            for _ in 0..threads {
                scope.spawn(|| loop {
                    let k = next.fetch_add(1, Ordering::SeqCst);
                    let Some(job) = jobs.get(k) else { break };
                    let (key, r) = match job {
                        Job::Def(def) => (def.id.clone(), print_part(def, model)),
                        Job::Kit(mesh, i) => {
                            let src = &model.kits[*mesh].parts[*i].source.source;
                            (
                                format!("kit:{mesh}:{i}"),
                                sf_custom::build_kit_part(src).map_err(|e| anyhow::anyhow!("{mesh} {}: {e}", model.kits[*mesh].parts[*i].key)),
                            )
                        }
                    };
                    results.lock().unwrap_or_else(|e| e.into_inner()).insert(key, r);
                    progress(done.fetch_add(1, Ordering::SeqCst) + 1, total);
                });
            }
        });
        let results = results.into_inner().unwrap_or_else(|e| e.into_inner());
        let mut files = Vec::new();
        let mut rows = Vec::new();
        let mut notes = Vec::new();
        for ((id, color), count) in &uses {
            let def = &defs[id];
            let colour = palette.name(*color);
            let (kind, desc) = match &def.kind {
                sf_core::CustomKind::Brick if def.plates.is_multiple_of(3) => ("brick", def.describe()),
                sf_core::CustomKind::Brick => ("plate", def.describe()),
                sf_core::CustomKind::Detail { .. } => ("detail", def.describe()),
            };
            let supports = matches!(def.kind, sf_core::CustomKind::Detail { .. });
            let part = match results.get(id) {
                Some(Ok(p)) => p,
                Some(Err(e)) => {
                    notes.push(format!("{id} in {colour}: {e:#}"));
                    continue;
                }
                None => continue,
            };
            let dir = out.join(slug(&colour));
            let made = write(part, &dir, id)?;
            files.push(made.stl.clone());
            files.push(made.three_mf);
            rows.push(PrintRow {
                id: id.clone(),
                kind: kind.into(),
                description: desc,
                colour,
                count: *count,
                plates: u32::from(def.plates),
                size_studs: def.size(),
                volume_mm3: part.volume_mm3,
                grams: part.volume_mm3 * PLA_G_PER_MM3,
                supports: supports
                    && sf_custom::overhang_area(&part.stl, DETAIL_CEILING_MM) > sf_custom::SUPPORT_AREA_MM2,
                file: made.stl.strip_prefix(out).ok().map(|p| p.to_string_lossy().into_owned()),
            });
        }
        for ((mesh, i, color), count) in &kit_uses {
            let part = &model.kits[mesh].parts[*i];
            let colour = palette.name(*color);
            let made_part = match results.get(&format!("kit:{mesh}:{i}")) {
                Some(Ok(p)) => p,
                Some(Err(e)) => {
                    notes.push(format!("{mesh} {} in {colour}: {e:#}", part.key));
                    continue;
                }
                None => continue,
            };
            let name = format!("{}_{}", slug(mesh), slug(&part.key));
            let dir = out.join(slug(&colour));
            let made = write(made_part, &dir, &name)?;
            files.push(made.stl.clone());
            files.push(made.three_mf);
            let (lo, hi) = made_part.bbox_mm;
            let studs = |mm: f64| (mm / 8.0).ceil().clamp(0.0, 255.0) as u8;
            rows.push(PrintRow {
                id: kit_ids.get(mesh).map(|v| v.join(" ")).unwrap_or_default(),
                kind: "kit part".into(),
                description: format!("{} ({mesh})", part.label),
                colour,
                count: *count,
                plates: ((hi[2] - lo[2]) / 3.2).ceil() as u32,
                size_studs: [studs(hi[0] - lo[0]), studs(hi[1] - lo[1])],
                volume_mm3: made_part.volume_mm3,
                grams: made_part.volume_mm3 * PLA_G_PER_MM3,
                supports: part.supports,
                file: made.stl.strip_prefix(out).ok().map(|p| p.to_string_lossy().into_owned()),
            });
        }
        for ((part, color), count) in &buy {
            let def = sf_core::parts::part(part);
            rows.push(PrintRow {
                id: part.clone(),
                kind: "buy".into(),
                description: def.map_or_else(|| part.clone(), |d| d.name.to_string()),
                colour: palette.name(*color),
                count: *count,
                plates: def.map_or(0, |d| u32::from(d.height)),
                size_studs: def.map_or([0, 0], |d| d.size),
                volume_mm3: 0.0,
                grams: 0.0,
                supports: false,
                file: None,
            });
        }
        let csv_field = |s: &str| {
            if s.contains([',', '"', '\n']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() }
        };
        let mut csv = String::from("id,kind,description,colour,count,plates,size_studs,volume_mm3,grams_pla,supports,file\n");
        for r in &rows {
            csv.push_str(&format!(
                "{},{},{},{},{},{},{}x{},{:.1},{:.2},{},{}\n",
                csv_field(&r.id),
                r.kind,
                csv_field(&r.description),
                csv_field(&r.colour),
                r.count,
                r.plates,
                r.size_studs[0],
                r.size_studs[1],
                r.volume_mm3,
                r.grams,
                if r.supports { "yes" } else { "no" },
                csv_field(r.file.as_deref().unwrap_or(""))
            ));
        }
        let list = out.join("print-list.csv");
        std::fs::write(&list, csv)?;
        files.push(list.clone());
        Ok(PrintSet { files, rows, list, notes })
    }

    pub(super) fn for_needs(needs: &[CustomNeed], palette: &Palette, out: &Path) -> Result<(Vec<PathBuf>, Vec<String>)> {
        let mut files = Vec::new();
        let mut notes = Vec::new();
        let mut csv = String::from("ldraw_part,color,quantity,reason,stl,volume_mm3\n");
        for n in needs {
            let colour = palette.name(n.color);
            let Some(shape) = shape_for(&n.part) else {
                notes.push(format!("{} in {colour}: no printable equivalent yet; order it or change the colour", n.part));
                continue;
            };
            let name = format!("{}-{}", n.part, slug(&colour));
            // A part whose studs can't all be fused (a 1×1, say) is noted, not made round.
            let part = match sf_custom::build(&shape, &profile()) {
                Ok(part) => part,
                Err(sf_custom::CustomError::InvalidShape(why)) => {
                    notes.push(format!("{} in {colour}: can't be printed with fused studs ({why}); order it or change the colour", n.part));
                    continue;
                }
                Err(e) => anyhow::bail!("{} in {colour}: {e}", n.part),
            };
            let made = write(&part, out, &name)?;
            csv.push_str(&format!(
                "{},{},{},{},{},{:.1}\n",
                n.part,
                colour,
                n.count,
                n.reason,
                made.stl.file_name().and_then(|f| f.to_str()).unwrap_or_default(),
                made.volume_mm3
            ));
            files.push(made.stl);
            files.push(made.three_mf);
        }
        if files.is_empty() {
            if notes.is_empty() {
                bail!("nothing to print");
            }
            return Ok((files, notes));
        }
        let list = out.join("custom-parts.csv");
        std::fs::write(&list, csv)?;
        files.push(list);
        Ok((files, notes))
    }
}

#[cfg(not(feature = "custom"))]
mod imp {
    use super::*;
    const OFF: &str = "this fushion was built without custom bricks (the `custom-bricks` feature)";
    pub(super) fn make(_: &Value, _: &Path, _: &str) -> Result<MadePart> {
        anyhow::bail!(OFF)
    }
    pub(super) fn from_commands(_: &Value, _: &Path, _: &str) -> Result<MadePart> {
        anyhow::bail!(OFF)
    }
    pub(super) fn for_needs(_: &[CustomNeed], _: &Palette, _: &Path) -> Result<(Vec<PathBuf>, Vec<String>)> {
        anyhow::bail!(OFF)
    }
    pub(super) fn print_set(_: &Model, _: &Palette, _: &Path, _: &(dyn Fn(usize, usize) + Sync)) -> Result<PrintSet> {
        anyhow::bail!(OFF)
    }
}

/// Generate a parametric custom part. `shape` is an `sf_custom::CustomShape`
/// as JSON, e.g. `{"kind":"slope45","w":2,"d":3,"plates":3}`. Every part gets
/// fused studs (`sf_custom::StudProfile::fused`); `_compliant_stud` is
/// accepted for old callers and ignored. A part with a stud that can't be
/// fused (a 1×1) is an error.
pub fn make_part(shape: &Value, _compliant_stud: bool, out: &Path, name: &str) -> Result<MadePart> {
    imp::make(shape, out, &slug(name))
}

/// Generate a freeform part from a SolidCraft command batch.
pub fn make_part_from_commands(batch: &Value, out: &Path, name: &str) -> Result<MadePart> {
    imp::from_commands(batch, out, &slug(name))
}

/// Print files for a model built to print. A detail built as a kit (its design in
/// `model.kits`) prints as its parts instead: each part once per colour, as
/// `out/<colour>/<design>_<part>.stl|.3mf`, its count the part's copies times the detail's
/// uses, in a "kit part" row naming the detail ids; part colours come from the placement
/// (role "main"), its `colors`, or the kit's role defaults.
///
/// Print files for a model built to print: one STL and 3MF per printed part (each id once,
/// made on as many threads as the machine has, up to 8), copied into a folder per colour as
/// `out/<colour>/<id>.stl|.3mf`, and `out/print-list.csv` listing every printed part (id,
/// kind, description, colour, count, plates, size, volume, grams of PLA, file) and every
/// catalog part to buy. Bricks are built from their cells and the fused units the packer
/// chose, with their id engraved underneath; details are rebuilt from the model's recorded
/// design. A part that fails is left out and named in `notes`. `progress(done, total)` is
/// called from the worker threads as each part finishes.
pub fn export_print_set(
    model: &Model,
    palette: &Palette,
    out: &Path,
    progress: impl Fn(usize, usize) + Sync,
) -> Result<PrintSet> {
    imp::print_set(model, palette, out, &progress)
}

/// Print files for every part the model needs printed. Returns the files
/// written and notes about parts that have no printable equivalent.
pub fn make_needed_parts(needs: &[CustomNeed], palette: &Palette, out: &Path) -> Result<(Vec<PathBuf>, Vec<String>)> {
    imp::for_needs(needs, palette, out)
}

#[cfg(all(test, feature = "custom"))]
mod tests {
    use sf_core::{CATALOG, print};
    use sf_custom::{CustomShape, StudProfile, fused_units};

    /// `sf-core`'s unit rule (used by the packer) matches the one `sf-custom` prints with.
    #[test]
    fn core_units_match_custom_units() {
        let mut checked = 0;
        for d in CATALOG.iter().filter(|d| print::printable(d)) {
            let Some(shape) = super::imp::shape_for(d.id) else {
                panic!("{} is printable but has no shape", d.id)
            };
            let theirs = fused_units(&shape, &StudProfile::fused())
                .unwrap_or_else(|e| panic!("{}: {e}", d.id));
            let ours = print::units_local(d).unwrap();
            let mut a: Vec<usize> = ours.iter().map(Vec::len).collect();
            let mut b: Vec<usize> = theirs.iter().map(Vec::len).collect();
            a.sort_unstable();
            b.sort_unstable();
            assert_eq!(a, b, "{}: unit sizes", d.id);
            if matches!(shape, CustomShape::Box { .. }) {
                // Boxes map straight: custom (i, j) = local (lx, lz).
                let norm = |u: Vec<Vec<(i32, i32)>>| {
                    let mut v: Vec<Vec<(i32, i32)>> = u
                        .into_iter()
                        .map(|mut g| {
                            g.sort();
                            g
                        })
                        .collect();
                    v.sort();
                    v
                };
                let ours: Vec<Vec<(i32, i32)>> = ours
                    .iter()
                    .map(|g| g.iter().map(|&(x, z)| (x as i32, z as i32)).collect())
                    .collect();
                assert_eq!(norm(ours), norm(theirs), "{}: unit cells", d.id);
            }
            checked += 1;
        }
        assert!(checked >= 20, "checked only {checked} parts");
    }

    /// Every catalog part sf-core calls unprintable is one sf-custom refuses (or can't shape).
    #[test]
    fn unprintable_parts_are_refused_by_custom() {
        // Tiles are excluded: the old export maps them to studded boxes (no smooth top yet).
        for d in CATALOG
            .iter()
            .filter(|d| !print::printable(d) && d.shape != sf_core::Shape::Tile)
        {
            if let Some(shape) = super::imp::shape_for(d.id) {
                assert!(
                    fused_units(&shape, &StudProfile::fused()).is_err(),
                    "{} should be refused",
                    d.id
                );
            }
        }
    }

    /// A print set from a small model: two placements of one brick id (in two colours), one
    /// other brick, one detail and a catalog part to buy.
    #[test]
    fn print_set_writes_one_file_per_id_and_colour() {
        use sf_core::{Cell, CustomDef, CustomKind, Model, Palette, Placement, Rot};
        let brick = |id: &str, cells: Vec<(u8, u8)>, units: Vec<Vec<(u8, u8)>>, plates: u8| CustomDef {
            id: id.into(),
            kind: CustomKind::Brick,
            studs: units.iter().flatten().copied().collect(),
            sockets: cells.clone(),
            cells,
            colors: Default::default(),
            plates,
            units,
        };
        let b1 = brick("B001", vec![(0, 0), (1, 0), (2, 0)], vec![vec![(0, 0), (1, 0), (2, 0)]], 3);
        let b2 = brick(
            "B002",
            vec![(0, 0), (1, 0), (0, 1), (1, 1)],
            vec![vec![(0, 0), (0, 1)], vec![(1, 0), (1, 1)]],
            1,
        );
        let dome = sf_custom::detail_template("eye_white", "eye", "dome", &serde_json::json!({}))
            .unwrap()
            .design;
        let mut model = Model { title: "t".into(), ..Default::default() };
        model.meshes.insert(dome.name.clone(), dome.mesh.clone());
        model.details.insert(dome.name.clone(), dome.source.clone());
        model.placements = vec![
            Placement::printed(b1.clone(), 4, Cell::new(0, 0, 0), Rot(0)),
            Placement::printed(b1.clone(), 15, Cell::new(0, 0, 2), Rot(0)),
            Placement::printed(b2, 4, Cell::new(0, 3, 0), Rot(1)),
            Placement::printed(dome.instance("D001"), 15, Cell::new(0, 4, 0), Rot(0)),
            Placement::new("3001", 1, Cell::new(5, 0, 5), Rot(0)),
        ];
        let out = std::env::temp_dir().join(format!("fushion-print-set-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        let calls = std::sync::Mutex::new(Vec::new());
        let set = super::export_print_set(&model, &Palette::builtin(), &out, |d, t| calls.lock().unwrap().push((d, t)))
            .unwrap();
        assert!(set.notes.is_empty(), "{:?}", set.notes);
        let pal = Palette::builtin();
        let (red, white) = (super::slug(&pal.name(4)), super::slug(&pal.name(15)));
        for f in [
            format!("{red}/B001.stl"),
            format!("{white}/B001.stl"),
            format!("{red}/B002.3mf"),
            format!("{white}/D001.stl"),
            "print-list.csv".into(),
        ] {
            assert!(out.join(&f).is_file(), "missing {f}");
        }
        // Three unique ids made, progress reached 3 of 3.
        assert!(calls.lock().unwrap().contains(&(3, 3)));
        let csv = std::fs::read_to_string(&set.list).unwrap();
        assert_eq!(csv.lines().count(), 1 + 4 + 1, "{csv}");
        assert!(csv.contains("B002,plate,"), "{csv}");
        assert!(csv.contains("D001,detail,"), "{csv}");
        assert!(csv.lines().next().unwrap().contains(",supports,"), "{csv}");
        assert!(set.rows.iter().all(|r| !r.supports), "a dome and bricks need no supports");
        assert!(csv.contains("3001,buy,Brick 2 x 4"), "{csv}");
        let b1_rows: Vec<_> = set.rows.iter().filter(|r| r.id == "B001").collect();
        assert_eq!(b1_rows.len(), 2);
        assert!(b1_rows.iter().all(|r| r.count == 1 && r.grams > 0.5));
        let _ = std::fs::remove_dir_all(&out);
    }

    /// A model using a kit: each kit part is printed in its role's colour, one row each.
    #[test]
    fn print_set_prints_kit_parts_by_role_colour() {
        use sf_core::{Cell, Model, Palette, Placement, Rot};
        let eye = sf_custom::detail_template("eye_kit", "eye", "eye", &serde_json::json!({})).unwrap().design;
        let kit = eye.kit.clone().expect("eye is a kit");
        let mut model = Model { title: "t".into(), ..Default::default() };
        model.meshes.insert(eye.name.clone(), eye.mesh.clone());
        model.details.insert(eye.name.clone(), eye.source.clone());
        model.kits.insert(eye.name.clone(), kit.clone());
        let pal = Palette::builtin();
        let mut d1 = eye.instance("D001");
        let d2 = eye.instance("D002");
        // The second eye's pupil is blue instead of the default black.
        d1.colors.insert("pupil".into(), pal.resolve("Blue").unwrap());
        model.placements = vec![
            Placement::printed(d1, 15, Cell::new(0, 0, 0), Rot(0)),
            Placement::printed(d2, 15, Cell::new(4, 0, 0), Rot(0)),
        ];
        let out = std::env::temp_dir().join(format!("fushion-kit-set-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        let set = super::export_print_set(&model, &pal, &out, |_, _| {}).unwrap();
        assert!(set.notes.is_empty(), "{:?}", set.notes);
        let white = super::slug(&pal.name(15));
        let black = super::slug("Black");
        let blue = super::slug("Blue");
        assert!(out.join(&white).join("eye-kit_eye.stl").is_file());
        assert!(out.join(&black).join("eye-kit_pupil.stl").is_file());
        assert!(out.join(&blue).join("eye-kit_pupil.3mf").is_file());
        let kit_rows: Vec<_> = set.rows.iter().filter(|r| r.kind == "kit part").collect();
        // eye ×2 in white; pupil ×1 black, ×1 blue.
        assert_eq!(kit_rows.len(), 3, "{kit_rows:?}");
        let eyes: Vec<_> = kit_rows.iter().filter(|r| r.description.starts_with("eye")).collect();
        assert_eq!(eyes.len(), 1);
        assert_eq!(eyes[0].count, 2);
        assert!(kit_rows.iter().all(|r| r.id.contains("D001") && r.id.contains("D002")));
        let csv = std::fs::read_to_string(&set.list).unwrap();
        assert!(csv.contains(",kit part,"), "{csv}");
        // No whole-detail print for a kit.
        assert!(!set.rows.iter().any(|r| r.kind == "detail"));
        let _ = std::fs::remove_dir_all(&out);
    }
}
