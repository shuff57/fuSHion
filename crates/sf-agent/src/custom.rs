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
}
