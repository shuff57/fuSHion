//! Kits (details built from several printed parts): grouping their uses into sub-builds and
//! rendering their pieces, steps and exploded views (through sf-render's kit renderer).

use crate::view::trim;
use image::RgbaImage;
use sf_core::{ColorId, CustomKind, Kit, KitPart, Model, Palette, Placement};
use sf_ldraw::MeshLibrary;
use sf_render::{RenderOptions, View, kit_role_colors, render_kit_exploded, render_kit_part, render_kit_step};
use std::collections::BTreeMap;

/// One sub-build: every use of a kit design with the same colours.
#[derive(Clone, Debug)]
pub struct SubBuild {
    /// The design (key into `Model::kits` and `Model::meshes`).
    pub design: String,
    /// Human name, e.g. "rose".
    pub label: String,
    /// The placement colour ("main" role).
    pub main: ColorId,
    /// Every role's colour for these uses.
    pub colors: BTreeMap<String, ColorId>,
    /// Each kit part's colour, by part index.
    pub part_colors: Vec<ColorId>,
    /// Placements (indices into the model) that use it.
    pub uses: Vec<usize>,
}

impl SubBuild {
    /// Key used for the parts-list entry and step callouts of the finished detail.
    pub fn key(&self, index: usize) -> String {
        format!("kit#{index}")
    }
}

/// The kit a placement uses, if it is a kit detail of `model`: (design, kit, label).
pub fn kit_of<'a>(model: &'a Model, p: &'a Placement) -> Option<(&'a str, &'a Kit, &'a str)> {
    let def = p.custom.as_deref()?;
    let CustomKind::Detail { mesh, name } = &def.kind else { return None };
    let (k, kit) = model.kits.get_key_value(mesh)?;
    Some((k.as_str(), kit, name.as_str()))
}

/// Groups the model's kit uses into sub-builds (same design, same colour for every role),
/// in the order they first appear in `steps`.
pub fn sub_builds(model: &Model, steps: &[Vec<usize>], palette: &Palette) -> Vec<SubBuild> {
    let mut out: Vec<SubBuild> = Vec::new();
    let empty = BTreeMap::new();
    for &i in steps.iter().flatten() {
        let Some(p) = model.placements.get(i) else { continue };
        let Some((design, kit, label)) = kit_of(model, p) else { continue };
        let overrides = p.custom.as_deref().map(|d| &d.colors).unwrap_or(&empty);
        let colors = kit_role_colors(kit, p.color, overrides, palette);
        match out.iter_mut().find(|s| s.design == design && s.colors == colors && s.main == p.color) {
            Some(s) => s.uses.push(i),
            None => {
                let part_colors = kit.parts.iter().map(|part| colors.get(&part.role).copied().unwrap_or(p.color)).collect();
                out.push(SubBuild { design: design.to_string(), label: label.to_string(), main: p.color, colors, part_colors, uses: vec![i] })
            }
        }
    }
    out
}

/// How far the exploded picture pulls pieces apart (1.0 doubles every distance).
const SPREAD: f32 = 1.5;

/// Kit pictures, trimmed to their content, on white.
pub struct KitRenderer<'a> {
    pub palette: &'a Palette,
    pub meshes: &'a MeshLibrary,
    pub size: u32,
}

impl KitRenderer<'_> {
    fn opts(&self) -> RenderOptions {
        RenderOptions { view_size: self.size, edges: true, background: [255, 255, 255] }
    }

    /// One copy of a kit piece, on its own, in `color`.
    pub fn part(&self, part: &KitPart, color: ColorId) -> RgbaImage {
        trim(&render_kit_part(part, color, self.palette, self.meshes, View::Iso, &self.opts()), 6)
    }

    /// The assembled kit, every piece in its colour.
    pub fn assembled(&self, kit: &Kit, colors: &BTreeMap<String, ColorId>) -> RgbaImage {
        trim(&render_kit_exploded(kit, colors, self.palette, self.meshes, View::Iso, &self.opts(), 0.0), 6)
    }

    /// The kit pulled apart.
    pub fn exploded(&self, kit: &Kit, colors: &BTreeMap<String, ColorId>) -> RgbaImage {
        trim(&render_kit_exploded(kit, colors, self.palette, self.meshes, View::Iso, &self.opts(), SPREAD), 6)
    }

    /// The kit after each of its steps: earlier pieces pale, the step's in colour, later ones
    /// absent. All framed on the whole kit and cropped alike, so the scale stays put.
    pub fn steps(&self, kit: &Kit, colors: &BTreeMap<String, ColorId>) -> Vec<RgbaImage> {
        let full: Vec<RgbaImage> = (0..kit.steps.len())
            .map(|k| render_kit_step(kit, k, colors, self.palette, self.meshes, View::Iso, &self.opts(), true))
            .collect();
        crate::view::trim_all(&full, 6)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::sample_model;

    #[test]
    fn uses_group_by_colours() {
        let m = sample_model();
        let steps = crate::steps_of(&m);
        let subs = sub_builds(&m, &steps, &Palette::builtin());
        assert_eq!(subs.len(), 2, "{subs:?}");
        assert!(subs.iter().all(|s| s.design == "rose" && s.uses.len() == 1));
        assert_ne!(subs[0].part_colors, subs[1].part_colors);
        // Defaults fill the roles the placement doesn't set: hub green, centre yellow.
        assert_eq!(subs[0].part_colors, vec![2, 4, 14]);
        assert_eq!(subs[1].part_colors, vec![2, 14, 25]);
        // Same colours twice: one sub-build of two.
        let mut m2 = m.clone();
        let i = subs[1].uses[0];
        m2.placements[i].color = m.placements[subs[0].uses[0]].color;
        m2.placements[i].custom.as_mut().unwrap().colors.clear();
        let subs2 = sub_builds(&m2, &steps, &Palette::builtin());
        assert_eq!(subs2.len(), 1);
        assert_eq!(subs2[0].uses.len(), 2);
    }

    #[test]
    fn kit_renders_show_the_pieces() {
        let m = sample_model();
        let kit = &m.kits["rose"];
        let pal = Palette::builtin();
        let r = KitRenderer { palette: &pal, meshes: &MeshLibrary::procedural(), size: 200 };
        let colors = kit_role_colors(kit, 4, &BTreeMap::new(), &pal);
        let ink = |img: &RgbaImage| img.pixels().filter(|p| p[0] < 245 || p[1] < 245 || p[2] < 245).count();
        let whole = r.assembled(kit, &colors);
        let apart = r.exploded(kit, &colors);
        let steps = r.steps(kit, &colors);
        assert_eq!(steps.len(), kit.steps.len());
        assert!(steps.iter().all(|i| i.dimensions() == steps[0].dimensions()), "steps share one crop");
        let first = steps[0].clone();
        let petal = r.part(&kit.parts[1], 4);
        for (name, img) in [("assembled", &whole), ("exploded", &apart), ("step", &first), ("part", &petal)] {
            assert!(ink(img) > 300, "{name} is empty");
        }
    }
}
