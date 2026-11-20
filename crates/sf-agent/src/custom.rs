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

    pub(super) fn profile(v2: bool) -> StudProfile {
        if v2 { StudProfile::slant_v2_experimental() } else { StudProfile::standard_fdm() }
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

    pub(super) fn make(shape: &Value, v2: bool, out: &Path, name: &str) -> Result<MadePart> {
        let shape: CustomShape = serde_json::from_value(shape.clone())
            .context("`shape` must be {\"kind\": \"box\"|\"slope45\"|\"curve\"|\"footprint\", ...}")?;
        let part = sf_custom::build(&shape, &profile(v2)).map_err(|e| anyhow::anyhow!("{e}"))?;
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
            let part = sf_custom::build(&shape, &profile(false)).map_err(|e| anyhow::anyhow!("{} in {colour}: {e}", n.part))?;
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
    pub(super) fn make(_: &Value, _: bool, _: &Path, _: &str) -> Result<MadePart> {
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
/// as JSON, e.g. `{"kind":"slope45","w":2,"d":3,"plates":3}`. `v2` selects
/// the experimental compliant stud.
pub fn make_part(shape: &Value, v2: bool, out: &Path, name: &str) -> Result<MadePart> {
    imp::make(shape, v2, out, &slug(name))
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
