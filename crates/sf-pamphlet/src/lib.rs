//! Printable build instructions for a fuSHion model, as a US Letter PDF booklet:
//!
//! 1. **Cover**: title, the finished model, its size and part counts.
//! 2. **Before you build**: how fused studs go together, print settings, how to read a step.
//! 3. **What to print** / **What to buy**: one row per part and colour (thumbnail, number,
//!    description, quantity, a box to tick), grouped by colour.
//! 4. **Steps**: two per page (one when a step adds many kinds of part). The model so far,
//!    earlier parts pale, this step's parts in colour, seen from the iso corner that shows
//!    them best; beside it a callout of the parts to add. A detail built from a kit of
//!    printed parts (a flower from a hub, petals and a centre) gets a **sub-build** first,
//!    LEGO style: a tinted box headed "Build ×N" with an exploded picture and the kit's own
//!    steps; the main step then adds the finished detail, pointing back to that page.
//!    Bouquet stems get the same treatment: a "Build ×N" page per kind of stem (head pushed
//!    on, leaves clipped on), then main steps plugging the stems into the hub's numbered
//!    holes, right after the step that places the hub.
//! 5. **Finished**: the model from two corners.
//!
//! Rendering uses sf-render's CPU renderer; the PDF is written by a small built-in writer
//! (base-14 Helvetica, JPEG images).

pub mod kit;
pub mod pdf;
#[doc(hidden)]
pub mod sample;
pub mod stem;
pub mod view;

use anyhow::{Context, Result};
use image::{RgbImage, RgbaImage};
use pdf::{Font, ImageId, PAGE_H, PAGE_W, Page, Pdf};
use sf_core::{ColorId, CustomKind, Model, Palette, Placement, Rot};
use sf_ldraw::MeshLibrary;
use sf_render::{RenderOptions, View, render_part};
use std::collections::BTreeMap;
use std::path::Path;
use kit::{KitRenderer, SubBuild};
use stem::MainStep;
use view::ScratchPalette;

/// Settings for [`write_pamphlet`].
#[derive(Clone, Debug)]
pub struct PamphletOptions {
    /// Title on the cover and in the footer (default: the model's title, else "fuSHion model").
    pub title: Option<String>,
    /// Lines for the "Print settings" box, e.g. "0.2 mm layers, PETG, studs up".
    pub print_settings: Vec<String>,
    /// Printed parts carry their number on the underside (changes the wording).
    pub numbers_on_parts: bool,
    /// Pixel size of step and cover renders (square, before trimming). Default 1100.
    pub step_image_px: u32,
    /// Pixel size of part thumbnails. Default 200.
    pub thumb_px: u32,
    /// JPEG quality 1–100. Default 85.
    pub jpeg_quality: u8,
    /// A step adding more kinds of part than this gets a page of its own. Default 5.
    pub max_kinds_half_page: usize,
    /// Stems plugged into the hub per main step. Default 1.
    pub stems_per_step: usize,
}

impl Default for PamphletOptions {
    fn default() -> Self {
        Self {
            title: None,
            print_settings: Vec::new(),
            numbers_on_parts: false,
            step_image_px: 1100,
            thumb_px: 200,
            jpeg_quality: 85,
            max_kinds_half_page: 5,
            stems_per_step: 1,
        }
    }
}

/// What was written.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PamphletInfo {
    pub pages: usize,
    pub steps: usize,
    pub bytes: usize,
    /// Printed body bricks: distinct designs and total parts.
    pub printed_designs: usize,
    pub printed_parts: usize,
    /// Printed detail parts (flower heads, eyes…): distinct designs and total parts.
    pub detail_designs: usize,
    pub detail_parts: usize,
    /// Catalog (LEGO) parts to buy.
    pub catalog_parts: usize,
    /// Kit details: designs, uses, separately printed pieces and sub-builds.
    pub kit_designs: usize,
    pub kit_uses: usize,
    pub kit_parts: usize,
    pub sub_builds: usize,
    /// Printed bricks trimmed to the model's curved surface, and how many need supports.
    pub curved_parts: usize,
    pub curved_with_supports: usize,
    /// Page (1-based) where each sub-build starts, in order.
    pub sub_build_pages: Vec<usize>,
    /// Page (1-based) of each main build step.
    pub step_pages: Vec<usize>,
    /// Stems: how many, distinct print ids, and distinct builds (identical stems share one).
    pub stems: usize,
    pub stem_ids: usize,
    pub stem_builds: usize,
    /// Page (1-based) of each stem build, in order.
    pub stem_build_pages: Vec<usize>,
    /// The main step (1-based) each stem (by index in `Model::stems`) is plugged in.
    pub stem_steps: Vec<usize>,
}

/// Write the instructions for `model` to `out` (a `.pdf`).
pub fn write_pamphlet(
    model: &Model,
    palette: &Palette,
    meshes: &MeshLibrary,
    opts: &PamphletOptions,
    out: &Path,
) -> Result<PamphletInfo> {
    let (bytes, info) = pamphlet_bytes(model, palette, meshes, opts)?;
    if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    std::fs::write(out, &bytes).with_context(|| format!("writing {}", out.display()))?;
    Ok(info)
}

/// The instructions as PDF bytes.
pub fn pamphlet_bytes(model: &Model, palette: &Palette, meshes: &MeshLibrary, opts: &PamphletOptions) -> Result<(Vec<u8>, PamphletInfo)> {
    let title = opts
        .title
        .clone()
        .filter(|t| !t.trim().is_empty())
        .or_else(|| Some(model.title.clone()).filter(|t| !t.trim().is_empty()))
        .unwrap_or_else(|| "fuSHion model".to_string());
    let mut b = Builder { doc: Pdf::default(), palette, meshes, opts, title, thumbs: BTreeMap::new() };
    let steps = steps_of(model);
    let seq = stem::sequence(model, &steps, opts.stems_per_step);
    let groups = stem::groups(model);
    let subs = kit::sub_builds(model, &steps, palette);
    // Each placement's parts-list key: a kit use goes under its sub-build.
    let mut keys: Vec<(String, ColorId)> = model.placements.iter().map(|p| (p.part.clone(), p.color)).collect();
    for (g, sb) in subs.iter().enumerate() {
        for &i in &sb.uses {
            keys[i] = (sb.key(g), sb.main);
        }
    }
    let entries = b.entries(model, &keys, &subs, &groups)?;

    let mut info = PamphletInfo {
        steps: seq.len(),
        sub_builds: subs.len(),
        stems: model.stems.len(),
        stem_builds: groups.len(),
        ..Default::default()
    };
    for e in entries.values() {
        match e.kind {
            Kind::Printed => {
                info.printed_parts += e.count;
                if e.curved {
                    info.curved_parts += e.count;
                    if e.supports {
                        info.curved_with_supports += e.count;
                    }
                }
            }
            Kind::Detail => info.detail_parts += e.count,
            Kind::KitPart => info.kit_parts += e.count,
            Kind::Kit => info.kit_uses += e.count,
            Kind::Catalog => info.catalog_parts += e.count,
            Kind::Stem | Kind::StemBuild => {}
        }
    }
    info.stem_ids = distinct(entries.values().filter(|e| e.kind == Kind::Stem));
    // Designs are counted per part id, not per colour.
    info.printed_designs = distinct(entries.values().filter(|e| e.kind == Kind::Printed));
    info.detail_designs = distinct(entries.values().filter(|e| e.kind == Kind::Detail));
    let mut designs: Vec<&str> = subs.iter().map(|s| s.design.as_str()).collect();
    designs.sort_unstable();
    designs.dedup();
    info.kit_designs = designs.len();

    b.cover(model, &info, seq.len())?;
    b.before_you_build(&info);
    b.parts_pages(&entries, true);
    b.parts_pages(&entries, false);
    let pages = b.step_pages(model, &steps, &seq, &entries, &keys, &subs, &groups)?;
    info.sub_build_pages = pages.sub_builds;
    info.step_pages = pages.steps;
    info.stem_build_pages = pages.stem_builds;
    info.stem_steps = pages.stem_steps;
    b.finished(model)?;
    b.footers();

    let bytes = b.doc.to_bytes(&b.title)?;
    info.pages = b.doc.pages.len();
    info.bytes = bytes.len();
    Ok((bytes, info))
}

fn distinct<'a>(it: impl Iterator<Item = &'a Entry>) -> usize {
    let mut ids: Vec<&str> = it.map(|e| e.part.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    ids.len()
}

/// The model's build steps, or (when it has none, or they miss parts) one step per layer of
/// up to ten parts, bottom up.
pub fn steps_of(model: &Model) -> Vec<Vec<usize>> {
    let n = model.placements.len();
    let mut seen = vec![false; n];
    let mut ok = !model.steps.is_empty();
    for s in &model.steps {
        for &i in s {
            if i >= n || seen[i] {
                ok = false;
            } else {
                seen[i] = true;
            }
        }
    }
    if ok && seen.iter().all(|&s| s) {
        return model.steps.iter().filter(|s| !s.is_empty()).cloned().collect();
    }
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&i| {
        let p = &model.placements[i];
        (p.origin.y, p.origin.z, p.origin.x)
    });
    let mut steps: Vec<Vec<usize>> = Vec::new();
    for i in order {
        let y = model.placements[i].origin.y;
        match steps.last_mut() {
            Some(s) if s.len() < 10 && model.placements[s[0]].origin.y == y => s.push(i),
            _ => steps.push(vec![i]),
        }
    }
    steps
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Printed,
    Detail,
    /// One printed piece of a kit (a petal).
    KitPart,
    /// A finished kit detail placed on the model (not printed as one piece).
    Kit,
    Catalog,
    /// A printed stem (its own section of the print list).
    Stem,
    /// A finished stem with its head and leaves (step callouts only).
    StemBuild,
}

fn kind(p: &Placement) -> Kind {
    match p.custom.as_deref().map(|c| &c.kind) {
        Some(CustomKind::Brick) => Kind::Printed,
        Some(CustomKind::Detail { .. }) => Kind::Detail,
        None => Kind::Catalog,
    }
}

/// One row of the parts lists (or one callout line): a part in one colour.
struct Entry {
    kind: Kind,
    /// Key: the part id, `"<design>/<part>"` for a kit piece, `"kit#<n>"` for a sub-build.
    part: String,
    /// What the row shows in bold: the id, or "rose petal" for a kit piece.
    title: String,
    color: ColorId,
    count: usize,
    name: String,
    thumb: ImageId,
    /// Needs support material: a kit piece laid out with overhangs, or a curved brick
    /// whose trimmed surface overhangs when printed studs up.
    supports: bool,
    /// A printed brick trimmed to the model's curved surface.
    curved: bool,
}

const MARGIN: f32 = 40.0;
const INK: [u8; 3] = [30, 32, 36];
const MUTED: [u8; 3] = [110, 114, 120];
const RULE: [u8; 3] = [200, 203, 208];
const PANEL: [u8; 3] = [244, 245, 247];
const ACCENT: [u8; 3] = [214, 64, 52];
/// Sub-build boxes.
const TINT: [u8; 3] = [229, 238, 248];
const TINT_EDGE: [u8; 3] = [170, 192, 220];
const TINT_INK: [u8; 3] = [32, 66, 112];

struct Builder<'a> {
    doc: Pdf,
    palette: &'a Palette,
    meshes: &'a MeshLibrary,
    opts: &'a PamphletOptions,
    title: String,
    thumbs: BTreeMap<(String, ColorId), ImageId>,
}

fn rgb(img: &RgbaImage) -> RgbImage {
    image::DynamicImage::ImageRgba8(img.clone()).to_rgb8()
}

/// Top-left corner label for a quarter turn of the model (where the viewer stands).
fn corner(turns: u8) -> &'static str {
    match turns % 4 {
        0 => "front left",
        1 => "front right",
        2 => "back right",
        _ => "back left",
    }
}

impl Builder<'_> {
    fn add(&mut self, img: &RgbaImage) -> Result<ImageId> {
        self.doc.add_image(&rgb(img), self.opts.jpeg_quality)
    }

    /// Every part × colour in the model, with a thumbnail each; kit uses as one entry per
    /// sub-build, plus one per kit piece × colour.
    fn entries(
        &mut self,
        model: &Model,
        keys: &[(String, ColorId)],
        subs: &[SubBuild],
        groups: &[Vec<usize>],
    ) -> Result<BTreeMap<(String, ColorId), Entry>> {
        let mut samples: BTreeMap<(String, ColorId), (Placement, usize)> = BTreeMap::new();
        for (p, k) in model.placements.iter().zip(keys) {
            if !k.0.starts_with("kit#") {
                samples.entry(k.clone()).or_insert_with(|| (p.clone(), 0)).1 += 1;
            }
        }
        // Thumbnails render in parallel.
        let keys: Vec<(String, ColorId)> = samples.keys().cloned().collect();
        let (palette, meshes, size) = (self.palette, self.meshes, self.opts.thumb_px.max(32));
        let thumb_model = |p: &Placement| -> RgbaImage {
            let mut p = p.clone();
            p.rot = Rot(0);
            let mut m = stem::blank(model);
            m.placements = vec![p];
            let opts = RenderOptions { view_size: size, edges: true, background: [255, 255, 255] };
            let img = if m.placements[0].custom.is_some() {
                // Printed parts may need the model's detail meshes; render through the model.
                sf_render::render_view(&m, palette, meshes, View::Iso, &opts)
            } else {
                render_part(&m.placements[0], palette, meshes, View::Iso, &opts)
            };
            square(&view::trim(&img, 4))
        };
        let imgs: Vec<RgbaImage> = parallel_map(&keys, |k| thumb_model(&samples[k].0));
        let mut out = BTreeMap::new();
        for (k, img) in keys.into_iter().zip(imgs) {
            let (p, count) = &samples[&k];
            let thumb = self.add(&img)?;
            self.thumbs.insert(k.clone(), thumb);
            let trim = p.custom.as_deref().and_then(|c| c.trim.as_deref());
            let name = match trim {
                Some(_) => curved_name(&p.name()),
                None => p.name(),
            };
            let name = match hub_holes(model, p) {
                Some(n) => format!("{name}, {n} stem holes"),
                None => name,
            };
            out.insert(
                k.clone(),
                Entry {
                    kind: kind(p),
                    part: k.0.clone(),
                    title: k.0.clone(),
                    color: k.1,
                    count: *count,
                    name,
                    thumb,
                    supports: trim.is_some_and(|t| t.overhang),
                    curved: trim.is_some(),
                },
            );
        }

        // Kits: the finished detail per sub-build, and every piece in every colour.
        let kr = KitRenderer { palette, meshes, size };
        let whole: Vec<RgbaImage> = parallel_map(subs, |sb| square(&kr.assembled(&model.kits[&sb.design], &sb.colors)));
        for ((g, sb), img) in subs.iter().enumerate().zip(whole) {
            let thumb = self.add(&img)?;
            let key = (sb.key(g), sb.main);
            out.insert(
                key.clone(),
                Entry { kind: Kind::Kit, part: key.0.clone(), title: sb.label.clone(), color: sb.main, count: sb.count(), name: sb.label.clone(), thumb, supports: false, curved: false },
            );
        }
        // (design, part index) and count for each piece × colour.
        let mut pieces: BTreeMap<(String, ColorId), (String, usize, usize, String)> = BTreeMap::new();
        for sb in subs {
            let kit = &model.kits[&sb.design];
            for (pi, part) in kit.parts.iter().enumerate() {
                let key = (format!("{}/{}", sb.design, part.key), sb.part_colors[pi]);
                pieces.entry(key).or_insert_with(|| (sb.design.clone(), pi, 0, sb.label.clone())).2 += part.copies.len() * sb.count();
            }
        }
        let pkeys: Vec<(String, ColorId)> = pieces.keys().cloned().collect();
        let imgs: Vec<RgbaImage> = parallel_map(&pkeys, |k| {
            let (design, pi, ..) = &pieces[k];
            square(&kr.part(&model.kits[design].parts[*pi], k.1))
        });
        for (k, img) in pkeys.into_iter().zip(imgs) {
            let (design, pi, count, label) = &pieces[&k];
            let part = &model.kits[design].parts[*pi];
            let thumb = self.add(&img)?;
            self.thumbs.insert(k.clone(), thumb);
            let per = part.copies.len();
            out.insert(
                k.clone(),
                Entry {
                    kind: Kind::KitPart,
                    part: k.0.clone(),
                    title: format!("{label} {}", part.label),
                    color: k.1,
                    count: *count,
                    name: format!("Kit piece, {per} per {label}"),
                    thumb,
                    supports: part.supports,
                    curved: false,
                },
            );
        }

        // Stems: a row per print id and colour, drawn lying flat as printed.
        let mut stem_rows: BTreeMap<(String, ColorId), (usize, usize)> = BTreeMap::new();
        for (si, sp) in model.stems.iter().enumerate() {
            stem_rows.entry((format!("stem:{}", sp.def.id), sp.color)).or_insert((si, 0)).1 += 1;
        }
        let skeys: Vec<(String, ColorId)> = stem_rows.keys().cloned().collect();
        let imgs: Vec<RgbaImage> = parallel_map(&skeys, |k| {
            let sp = &model.stems[stem_rows[k].0];
            stem::flat_thumb(&sp.def, sp.color, palette, meshes, size * 2)
        });
        for (k, img) in skeys.into_iter().zip(imgs) {
            let (si, count) = stem_rows[&k];
            let def = &model.stems[si].def;
            let thumb = self.add(&img)?;
            let e = Entry { kind: Kind::Stem, part: k.0.clone(), title: stem::title(def), color: k.1, count, name: stem::describe(def), thumb, supports: false, curved: false };
            out.insert(k, e);
        }
        // Each stem build, finished, for step callouts.
        let imgs: Vec<RgbaImage> = parallel_map(groups, |g| square(&stem::finished_thumb(model, &model.stems[g[0]], palette, meshes, size)));
        for (gi, (g, img)) in groups.iter().zip(imgs).enumerate() {
            let sp = &model.stems[g[0]];
            let thumb = self.add(&img)?;
            let key = (format!("stems#{gi}"), sp.color);
            let e = Entry { kind: Kind::StemBuild, part: key.0.clone(), title: sp.def.id.clone(), color: sp.color, count: g.len(), name: String::new(), thumb, supports: false, curved: false };
            out.insert(key, e);
        }
        // Heads and leaves printed in one piece (kits are sub-builds, above).
        let mut plain: BTreeMap<(String, ColorId), (String, usize, bool)> = BTreeMap::new();
        for sp in &model.stems {
            let heads = sp.head.iter().chain(&sp.branch_heads).map(|a| (a, true));
            for (a, head) in heads.chain(sp.leaves.iter().map(|a| (a, false))) {
                if !model.kits.contains_key(&a.design) {
                    plain.entry((format!("design:{}", a.design), a.color)).or_insert((a.design.clone(), 0, head)).1 += 1;
                }
            }
        }
        let pkeys: Vec<(String, ColorId)> = plain.keys().cloned().collect();
        let imgs: Vec<RgbaImage> = parallel_map(&pkeys, |k| square(&stem::design_thumb(model, &plain[k].0, k.1, palette, meshes, size)));
        for (k, img) in pkeys.into_iter().zip(imgs) {
            let (design, count, head) = &plain[&k];
            let thumb = self.add(&img)?;
            self.thumbs.insert(k.clone(), thumb);
            let name = if *head { "Printed head, pushes onto a stem's tip" } else { "Printed leaf, clips onto a stem" };
            let e = Entry { kind: Kind::Detail, part: k.0.clone(), title: stem::humanize(design), color: k.1, count: *count, name: name.into(), thumb, supports: false, curved: false };
            out.insert(k, e);
        }
        Ok(out)
    }

    fn new_page(&mut self) -> &mut Page {
        self.doc.pages.push(Page::default());
        self.doc.pages.last_mut().expect("just pushed")
    }

    fn heading(page: &mut Page, title: &str, sub: &str) -> f32 {
        page.text(MARGIN, MARGIN + 22.0, 24.0, Font::Bold, INK, title);
        page.fill_rect(MARGIN, MARGIN + 30.0, 48.0, 3.0, ACCENT);
        if sub.is_empty() {
            return MARGIN + 52.0;
        }
        page.paragraph(MARGIN, MARGIN + 50.0, PAGE_W - 2.0 * MARGIN, 10.5, Font::Regular, MUTED, sub) + 6.0
    }

    fn cover(&mut self, model: &Model, info: &PamphletInfo, steps: usize) -> Result<()> {
        let size = self.opts.step_image_px.max(64);
        let img = view::trim(&view::render_turned(model, self.palette, self.meshes, 0, size), 12);
        let id = self.add(&img)?;
        let title = self.title.clone();
        let page = self.new_page();
        let mut y = MARGIN + 34.0;
        for line in pdf::wrap(&title, Font::Bold, 30.0, PAGE_W - 2.0 * MARGIN) {
            page.text_center(PAGE_W / 2.0, y, 30.0, Font::Bold, INK, &line);
            y += 36.0;
        }
        page.text_center(PAGE_W / 2.0, y - 8.0, 11.0, Font::Regular, MUTED, "Build instructions");
        page.fill_rect(PAGE_W / 2.0 - 24.0, y, 48.0, 3.0, ACCENT);
        let top = y + 18.0;
        let bottom = PAGE_H - 170.0;
        page.image(id, MARGIN, top, PAGE_W - 2.0 * MARGIN, bottom - top);

        let mut facts: Vec<String> = Vec::new();
        if let Some((lo, hi)) = model.bounds() {
            let (w, d, h) = (hi.x - lo.x, hi.z - lo.z, hi.y - lo.y);
            facts.push(format!(
                "Size: {w} × {d} studs, {:.1} bricks tall ({:.0} × {:.0} × {:.0} mm)",
                h as f32 / 3.0,
                w as f32 * 8.0,
                d as f32 * 8.0,
                h as f32 * 3.2
            ));
            // Stems reach above the bricks: give the bouquet's full height too.
            if let Some((top, _)) = sf_render::stems_bounds(model) {
                // LDraw -Y is up; 2.5 LDU to the millimetre.
                let tall = (-(lo.y as f32) * 8.0 - top[1]) / 2.5;
                if tall > h as f32 * 3.2 + 1.0 {
                    facts.push(format!("{tall:.0} mm tall with its stems"));
                }
            }
        }
        let mut parts: Vec<String> = Vec::new();
        if info.printed_parts > 0 {
            parts.push(format!("{} printed bricks ({} designs)", info.printed_parts, info.printed_designs));
        }
        if info.detail_parts > 0 {
            parts.push(format!("{} printed details ({} designs)", info.detail_parts, info.detail_designs));
        }
        if info.kit_parts > 0 {
            parts.push(format!("{} printed kit pieces for {} sub-builds", info.kit_parts, info.kit_uses));
        }
        if info.stems > 0 {
            parts.push(format!("{} printed stem{}", info.stems, if info.stems == 1 { "" } else { "s" }));
        }
        if info.catalog_parts > 0 {
            parts.push(format!("{} LEGO parts", info.catalog_parts));
        }
        if !parts.is_empty() {
            facts.push(format!("Parts: {}", parts.join(" · ")));
        }
        facts.push(format!("{steps} build step{}", if steps == 1 { "" } else { "s" }));
        let page = self.doc.pages.last_mut().expect("cover");
        let mut y = bottom + 34.0;
        for f in facts {
            for line in pdf::wrap(&f, Font::Regular, 12.0, PAGE_W - 2.0 * MARGIN) {
                page.text_center(PAGE_W / 2.0, y, 12.0, Font::Regular, INK, &line);
                y += 17.0;
            }
            y += 3.0;
        }
        Ok(())
    }

    fn before_you_build(&mut self, info: &PamphletInfo) {
        let printed = info.printed_parts + info.detail_parts + info.kit_parts + info.stems > 0;
        let stems = info.stems > 0;
        let kits = info.sub_builds > 0;
        let numbers = self.opts.numbers_on_parts;
        let settings = self.opts.print_settings.clone();
        // Sample colours for the legend.
        let mut pal = ScratchPalette::new(self.palette);
        let red = self.palette.rgb(4);
        let g = pal.ghost(4);
        let ghost = pal.palette.rgb(g);

        let page = self.new_page();
        let mut y = Self::heading(page, "Before you build", "");
        let w = PAGE_W - 2.0 * MARGIN;
        let section = |page: &mut Page, y: f32, s: &str| -> f32 {
            page.text(MARGIN, y + 14.0, 14.0, Font::Bold, INK, s);
            y + 24.0
        };
        if printed {
            y = section(page, y, "Printed bricks");
            y = page.paragraph(
                MARGIN,
                y + 4.0,
                w,
                11.0,
                Font::Regular,
                INK,
                "Printed bricks have fused studs: pairs and triples of studs joined by a narrow waist, with a springy fin \
                 at each end that grips the brick on top. Press every brick on firmly until it sits flat. A brick never \
                 ends between two fused studs, so if one won't seat, check its position against the picture.",
            ) + 4.0;
            let num = if numbers {
                "Every printed part has its number (for example B017) stamped underneath. Find it in the parts list and \
                 in the step pictures."
            } else {
                "Every printed part has a number (for example B017) in the parts list and the step pictures. Keep each \
                 print with its number as you go."
            };
            y = page.paragraph(MARGIN, y, w, 11.0, Font::Regular, INK, num) + 4.0;
            if info.curved_parts > 0 {
                let mut line = String::from(
                    "Curved bricks are printed bricks whose outside faces follow the model's shape instead of \
                     stepping in whole studs. They go on like any other brick.",
                );
                if info.curved_with_supports > 0 {
                    line.push_str(" Print the ones tagged SUPPORTS in the parts list with support material.");
                }
                y = page.paragraph(MARGIN, y, w, 11.0, Font::Regular, INK, &line) + 4.0;
            }
            if stems {
                y = page.paragraph(
                    MARGIN,
                    y,
                    w,
                    11.0,
                    Font::Regular,
                    INK,
                    "Stems are long printed rods. Each one gets its head and leaves on a \"Build\" page, then plugs into a \
                     numbered hole in the hub. The flat on a stem's peg matches the flat in its hole, so it only goes in \
                     one way round.",
                ) + 4.0;
            }
            if kits {
                page.fill_rect(MARGIN, y, w, 40.0, TINT);
                page.stroke_rect(MARGIN, y, w, 40.0, 0.75, TINT_EDGE);
                page.paragraph(
                    MARGIN + 8.0,
                    y + 15.0,
                    w - 16.0,
                    10.5,
                    Font::Regular,
                    TINT_INK,
                    "Some details, like flowers, are built from several printed pieces. They are built first, in a \
                     tinted box headed \"Build ×2\" (make two); the next step adds them to the model.",
                );
                y += 46.0;
            }
            y += 6.0;

            y = section(page, y, "Print settings");
            let mut lines = if settings.is_empty() {
                vec!["Use the settings you print LEGO-compatible bricks with. Print studs up.".to_string()]
            } else {
                settings
            };
            if stems {
                lines.push("Print stems lying flat, as laid out in the file, with the flat side down.".to_string());
            }
            for l in lines {
                page.text(MARGIN + 4.0, y + 11.0, 11.0, Font::Regular, ACCENT, "•");
                y = page.paragraph(MARGIN + 16.0, y + 11.0, w - 16.0, 11.0, Font::Regular, INK, &l) - 11.0 + 4.0;
            }
            y += 12.0;
        }

        y = section(page, y, "How to read a step");
        let bx = MARGIN;
        page.fill_rect(bx, y + 4.0, 40.0, 24.0, ghost);
        page.stroke_rect(bx, y + 4.0, 40.0, 24.0, 0.5, RULE);
        page.text(bx + 52.0, y + 20.0, 11.0, Font::Regular, INK, "Pale parts were added in earlier steps.");
        y += 34.0;
        page.fill_rect(bx, y + 4.0, 40.0, 24.0, red);
        page.text(bx + 52.0, y + 20.0, 11.0, Font::Regular, INK, "Parts in full colour are the ones to add now.");
        y += 40.0;
        // A mock callout.
        page.fill_rect(bx, y, 150.0, 44.0, PANEL);
        page.stroke_rect(bx, y, 150.0, 44.0, 0.75, RULE);
        page.fill_rect(bx + 8.0, y + 8.0, 28.0, 28.0, red);
        page.text(bx + 44.0, y + 20.0, 11.0, Font::Bold, INK, "B017 ×2");
        page.text(bx + 44.0, y + 33.0, 8.5, Font::Regular, MUTED, "Red");
        page.paragraph(
            bx + 166.0,
            y + 14.0,
            w - 166.0,
            11.0,
            Font::Regular,
            INK,
            "The box beside each picture lists what to add: the part's number, how many, and the colour. When the \
             picture shows the model from another side, the step says so under its number.",
        );
        y += 64.0;
        let page = self.doc.pages.last_mut().expect("page");
        page.stroke_rect(MARGIN, y + 2.0, 10.0, 10.0, 0.9, INK);
        page.text(MARGIN + 18.0, y + 11.0, 11.0, Font::Regular, INK, "Tick each part in the parts list as you print or find it.");
    }

    /// "What to print" (printed bricks and details) or "What to buy" (catalog parts).
    fn parts_pages(&mut self, entries: &BTreeMap<(String, ColorId), Entry>, printed: bool) {
        let rows: Vec<&Entry> = entries
            .values()
            .filter(|e| !matches!(e.kind, Kind::Kit | Kind::Stem | Kind::StemBuild) && (e.kind != Kind::Catalog) == printed)
            .collect();
        let stems: Vec<&Entry> = if printed { entries.values().filter(|e| e.kind == Kind::Stem).collect() } else { Vec::new() };
        if rows.is_empty() && stems.is_empty() {
            return;
        }
        // Group by colour, biggest colour first; within a colour by kind, then id.
        let mut by_color: BTreeMap<ColorId, Vec<&Entry>> = BTreeMap::new();
        for e in rows {
            by_color.entry(e.color).or_default().push(e);
        }
        let mut groups: Vec<(ColorId, Vec<&Entry>)> = by_color.into_iter().collect();
        groups.sort_by_key(|(c, v)| (std::cmp::Reverse(v.iter().map(|e| e.count).sum::<usize>()), *c));
        for (_, v) in &mut groups {
            v.sort_by(|a, b| (a.kind, &a.part).cmp(&(b.kind, &b.part)));
        }
        let total: usize = groups.iter().flat_map(|(_, v)| v.iter().map(|e| e.count)).sum::<usize>() + stems.iter().map(|e| e.count).sum::<usize>();
        let kinds: usize = groups.iter().map(|(_, v)| v.len()).sum::<usize>() + stems.len();
        let (title, sub) = if printed {
            ("What to print", format!("{total} parts, {kinds} kinds. Tick each one when it's printed."))
        } else {
            ("What to buy", format!("{total} LEGO parts, {kinds} kinds (LDraw / BrickLink numbers). Tick each one when you have it."))
        };

        const COL_W: f32 = (PAGE_W - 2.0 * MARGIN - 20.0) / 2.0;
        const ROW_H: f32 = 46.0;
        const HEAD_H: f32 = 26.0;
        let bottom = PAGE_H - 48.0;
        let mut col = 0;
        let mut y;
        {
            let page = self.new_page();
            y = Self::heading(page, title, &sub);
        }
        let mut top = y;
        let palette = self.palette;
        for (color, list) in groups {
            let mut items: Vec<Option<&Entry>> = vec![None];
            items.extend(list.iter().map(|e| Some(*e)));
            let n_in_color: usize = list.iter().map(|e| e.count).sum();
            for (k, item) in items.into_iter().enumerate() {
                let h = if item.is_none() { HEAD_H } else { ROW_H };
                // Keep a colour header with at least its first row.
                let need = if item.is_none() { HEAD_H + ROW_H } else { h };
                if y + need > bottom {
                    if col == 0 {
                        col = 1;
                        y = top;
                    } else {
                        col = 0;
                        let page = self.new_page();
                        y = Self::heading(page, &format!("{title} (continued)"), "");
                        top = y;
                    }
                }
                let x = MARGIN + col as f32 * (COL_W + 20.0);
                let page = self.doc.pages.last_mut().expect("page");
                match item {
                    None => {
                        let c = palette.rgb(color);
                        page.fill_rect(x, y + 8.0, 13.0, 13.0, c);
                        page.stroke_rect(x, y + 8.0, 13.0, 13.0, 0.5, MUTED);
                        let label = format!("{} · {n_in_color}", palette.name(color));
                        page.text(x + 20.0, y + 19.0, 12.0, Font::Bold, INK, &label);
                        page.line(x, y + h - 1.0, x + COL_W, y + h - 1.0, 0.75, RULE);
                    }
                    Some(e) => {
                        page.stroke_rect(x + 1.0, y + 17.0, 10.0, 10.0, 0.9, INK);
                        page.image(e.thumb, x + 18.0, y + 3.0, 40.0, 40.0);
                        let title = pdf::fit(&e.title, Font::Bold, 11.5, COL_W - 140.0);
                        page.text(x + 66.0, y + 18.0, 11.5, Font::Bold, INK, &title);
                        if e.supports {
                            let tx = x + 70.0 + pdf::text_width(&title, Font::Bold, 11.5);
                            let tw = pdf::text_width("SUPPORTS", Font::Bold, 6.5) + 8.0;
                            page.fill_rect(tx, y + 9.5, tw, 10.0, ACCENT);
                            page.text(tx + 4.0, y + 17.0, 6.5, Font::Bold, [255, 255, 255], "SUPPORTS");
                        }
                        let qty = format!("×{}", e.count);
                        page.text_right(x + COL_W, y + 18.0, 13.0, Font::Bold, INK, &qty);
                        let desc = match e.kind {
                            Kind::Catalog | Kind::KitPart | Kind::Kit => e.name.clone(),
                            _ => describe_printed(&e.name, &e.part),
                        };
                        let desc = pdf::fit(&desc, Font::Regular, 8.5, COL_W - 70.0);
                        page.text(x + 66.0, y + 31.0, 8.5, Font::Regular, MUTED, &desc);
                        if k + 1 < list.len() + 1 {
                            page.line(x + 66.0, y + h - 0.5, x + COL_W, y + h - 0.5, 0.25, RULE);
                        }
                    }
                }
                y += h;
            }
            y += 6.0;
        }
        if stems.is_empty() {
            return;
        }
        // Stems: full-width rows below both columns.
        const STEM_ROW: f32 = 50.0;
        if col == 1 || y + HEAD_H + STEM_ROW > bottom {
            let page = self.new_page();
            y = Self::heading(page, &format!("{title} (continued)"), "");
        }
        let full_w = PAGE_W - 2.0 * MARGIN;
        let n_stems: usize = stems.iter().map(|e| e.count).sum();
        {
            let page = self.doc.pages.last_mut().expect("page");
            page.text(MARGIN, y + 19.0, 12.0, Font::Bold, INK, &format!("Stems · {n_stems}"));
            page.text(MARGIN + 80.0, y + 19.0, 8.5, Font::Regular, MUTED, "Shown lying flat, as they print. Each stem's number is under its peg.");
            page.line(MARGIN, y + HEAD_H - 1.0, MARGIN + full_w, y + HEAD_H - 1.0, 0.75, RULE);
        }
        y += HEAD_H;
        for e in stems {
            if y + STEM_ROW > bottom {
                let page = self.new_page();
                y = Self::heading(page, &format!("{title} (continued)"), "");
            }
            let page = self.doc.pages.last_mut().expect("page");
            let x = MARGIN;
            page.stroke_rect(x + 1.0, y + 19.0, 10.0, 10.0, 0.9, INK);
            page.image(e.thumb, x + 18.0, y + 4.0, 200.0, 42.0);
            page.text(x + 230.0, y + 20.0, 11.5, Font::Bold, INK, &e.title);
            page.text(x + 230.0, y + 33.0, 8.5, Font::Regular, MUTED, &pdf::fit(&e.name, Font::Regular, 8.5, 170.0));
            page.fill_rect(x + 410.0, y + 14.0, 9.0, 9.0, palette.rgb(e.color));
            page.stroke_rect(x + 410.0, y + 14.0, 9.0, 9.0, 0.4, MUTED);
            page.text(x + 424.0, y + 22.0, 8.5, Font::Regular, MUTED, &pdf::fit(&palette.name(e.color), Font::Regular, 8.5, 60.0));
            page.text_right(x + full_w, y + 22.0, 13.0, Font::Bold, INK, &format!("×{}", e.count));
            page.line(x + 18.0, y + STEM_ROW - 0.5, x + full_w, y + STEM_ROW - 0.5, 0.25, RULE);
            y += STEM_ROW;
        }
    }

    /// The main build steps, each kit sub-build and stem build just before the step that
    /// first needs it.
    #[allow(clippy::too_many_arguments)]
    fn step_pages(
        &mut self,
        model: &Model,
        steps: &[Vec<usize>],
        seq: &[MainStep],
        entries: &BTreeMap<(String, ColorId), Entry>,
        keys: &[(String, ColorId)],
        subs: &[SubBuild],
        groups: &[Vec<usize>],
    ) -> Result<StepPages> {
        let mut out = StepPages {
            sub_builds: vec![0; subs.len()],
            stem_builds: vec![0; groups.len()],
            stem_steps: vec![0; model.stems.len()],
            steps: Vec::with_capacity(seq.len()),
        };
        if seq.is_empty() {
            return Ok(out);
        }
        let size = self.opts.step_image_px.max(64);
        let (palette, meshes) = (self.palette, self.meshes);
        // Which hubs each main step shows the numbered holes of, and which holes it fills.
        let hubs_of = |j: usize| -> (Vec<usize>, Vec<u32>) {
            let mut hubs: Vec<usize> = Vec::new();
            let mut lit = Vec::new();
            match &seq[j] {
                MainStep::Parts(k) => {
                    for sp in &model.stems {
                        if let Some(h) = stem::hub_index(model, sp)
                            && steps[*k].contains(&h)
                            && !hubs.contains(&h)
                        {
                            hubs.push(h);
                        }
                    }
                }
                MainStep::Stems(list) => {
                    for &si in list {
                        let sp = &model.stems[si];
                        lit.push(sp.hole);
                        if let Some(h) = stem::hub_index(model, sp)
                            && !hubs.contains(&h)
                        {
                            hubs.push(h);
                        }
                    }
                }
            }
            (hubs, lit)
        };
        // Render every main step (in parallel): the picture, the corner it was seen from,
        // and where the hub's hole numbers go in it.
        let idx: Vec<usize> = (0..seq.len()).collect();
        let rendered: Vec<StepPic> = parallel_map(&idx, |&j| {
            let mut pal = ScratchPalette::new(palette);
            let (m, new) = stem::scene(model, steps, seq, j, &mut pal, palette, true);
            let turn = view::best_turn(&m, &new, meshes);
            let (img, ox, oy) = view::trim_at(&view::render_turned(&m, &pal.palette, meshes, turn, size), 10);
            let (hubs, lit) = hubs_of(j);
            let mut holes = Vec::new();
            for h in hubs {
                let Some(id) = model.placements[h].custom.as_deref().map(|c| c.id.clone()) else { continue };
                let Some(in_scene) = m.placements.iter().position(|p| p.custom.as_deref().is_some_and(|c| c.id == id)) else { continue };
                let list: Vec<(u32, [f32; 3])> = model.stems.iter().filter(|s| s.hub == id).map(|s| (s.hole, s.pose.position)).collect();
                for (n, x, y) in stem::hole_points(&m, in_scene, &list, turn, meshes, size) {
                    holes.push((n, x - ox as f32, y - oy as f32, lit.contains(&n)));
                }
            }
            StepPic { img, turn, holes }
        });
        // Sub-build pictures: the exploded kit, then each of its steps.
        let kr = KitRenderer { palette, meshes, size: (size * 3 / 4).max(64) };
        let sub_imgs: Vec<(RgbaImage, Vec<RgbaImage>)> = parallel_map(subs, |sb| {
            let kit = &model.kits[&sb.design];
            (kr.exploded(kit, &sb.colors), kr.steps(kit, &sb.colors))
        });
        let mut sub_imgs: Vec<Option<(RgbaImage, Vec<RgbaImage>)>> = sub_imgs.into_iter().map(Some).collect();
        // Stem build pictures, and the main steps each kind of stem goes in.
        let stem_imgs: Vec<Vec<RgbaImage>> =
            parallel_map(groups, |g| stem::build_pictures(model, &model.stems[g[0]], palette, meshes, (size * 3 / 4).max(64)));
        let mut stem_imgs: Vec<Option<Vec<RgbaImage>>> = stem_imgs.into_iter().map(Some).collect();
        let mut group_of = vec![0; model.stems.len()];
        for (g, list) in groups.iter().enumerate() {
            for &si in list {
                group_of[si] = g;
            }
        }
        for (j, ms) in seq.iter().enumerate() {
            if let MainStep::Stems(list) = ms {
                for &si in list {
                    out.stem_steps[si] = j + 1;
                }
            }
        }
        let many_hubs = {
            let mut h: Vec<&str> = model.stems.iter().map(|s| s.hub.as_str()).collect();
            h.sort_unstable();
            h.dedup();
            h.len() > 1
        };

        let mut slot_free = false; // the lower half of the current page is free
        for (j, pic) in rendered.into_iter().enumerate() {
            // Sub-builds first: kits this step places, then (for stems) the kits on them and
            // the stems themselves.
            match &seq[j] {
                MainStep::Parts(k) => {
                    for g in 0..subs.len() {
                        if sub_imgs[g].is_some() && subs[g].uses.iter().any(|u| steps[*k].contains(u)) {
                            let (exploded, kit_steps) = sub_imgs[g].take().expect("checked");
                            out.sub_builds[g] = self.doc.pages.len() + 1;
                            self.sub_build(model, &subs[g], &format!("for step {}", j + 1), &exploded, &kit_steps, entries)?;
                            slot_free = false;
                        }
                    }
                }
                MainStep::Stems(list) => {
                    for &si in list {
                        let g = group_of[si];
                        let Some(pics) = stem_imgs[g].take() else { continue };
                        let sp = &model.stems[si];
                        for a in stem::attachments(sp) {
                            if let Some(k) = kit::of_attach(subs, model, a, palette)
                                && let Some((exploded, kit_steps)) = sub_imgs[k].take()
                            {
                                out.sub_builds[k] = self.doc.pages.len() + 1;
                                let what = format!("for stem {}", sp.def.id);
                                self.sub_build(model, &subs[k], &what, &exploded, &kit_steps, entries)?;
                            }
                        }
                        out.stem_builds[g] = self.doc.pages.len() + 1;
                        let when: Vec<usize> = groups[g].iter().map(|&s| out.stem_steps[s]).collect();
                        self.stem_build(model, &groups[g], &when, &pics, entries, subs, &out.sub_builds)?;
                        slot_free = false;
                    }
                }
            }
            let id = self.add(&pic.img)?;
            // This step's rows: parts grouped by key, or stems by build.
            let mut group: BTreeMap<(String, ColorId), usize> = BTreeMap::new();
            let mut lines: Vec<String> = Vec::new();
            match &seq[j] {
                MainStep::Parts(k) => {
                    for &i in &steps[*k] {
                        if let Some(key) = keys.get(i) {
                            *group.entry(key.clone()).or_default() += 1;
                        }
                    }
                    if !pic.holes.is_empty() {
                        lines.push("The hub's holes are numbered in the picture: each stem goes in its own hole.".into());
                    }
                }
                MainStep::Stems(list) => {
                    for &si in list {
                        let sp = &model.stems[si];
                        *group.entry((format!("stems#{}", group_of[si]), sp.color)).or_default() += 1;
                        let hub = if many_hubs { format!(" of hub {}", sp.hub) } else { String::new() };
                        lines.push(format!("Plug {} into hole {}{hub}.", sp.def.id, sp.hole));
                    }
                }
            }
            let full = group.len() > self.opts.max_kinds_half_page;
            let (y0, h) = if full {
                self.new_page();
                slot_free = false;
                (MARGIN, PAGE_H - MARGIN - 48.0)
            } else if slot_free {
                slot_free = false;
                let mid = PAGE_H / 2.0;
                self.doc.pages.last_mut().expect("page").line(MARGIN, mid - 4.0, PAGE_W - MARGIN, mid - 4.0, 0.75, RULE);
                (mid + 6.0, PAGE_H / 2.0 - 54.0)
            } else {
                self.new_page();
                slot_free = true;
                (MARGIN, PAGE_H / 2.0 - MARGIN - 10.0)
            };
            out.steps.push(self.doc.pages.len());
            let page = self.doc.pages.last_mut().expect("page");
            page.text(MARGIN, y0 + 30.0, 32.0, Font::Bold, INK, &format!("{}", j + 1));
            if pic.turn != 0 {
                page.text(MARGIN, y0 + 46.0, 9.0, Font::Regular, ACCENT, &format!("Seen from the {}", corner(pic.turn)));
            }
            let rows: Vec<(&Entry, usize, Option<String>)> = group
                .iter()
                .filter_map(|(key, &n)| {
                    let e = entries.get(key)?;
                    let num = |p: &str| key.0.strip_prefix(p).and_then(|g| g.parse::<usize>().ok());
                    let note = num("kit#")
                        .map(|g| format!("Built on page {}", out.sub_builds[g]))
                        .or_else(|| num("stems#").map(|g| format!("Built on page {}", out.stem_builds[g])));
                    Some((e, n, note))
                })
                .collect();
            let text_w = |w: f32| -> Vec<String> { lines.iter().flat_map(|l| pdf::wrap(l, Font::Regular, 9.0, w)).collect() };
            let pic_box = if full {
                // Picture across the page, callout grid below it.
                let cols = 3usize;
                let n_rows = rows.len().div_ceil(cols);
                let cw = PAGE_W - 2.0 * MARGIN;
                let wrapped = text_w(cw - 16.0);
                let call_h = 14.0 + n_rows as f32 * 46.0 + wrapped.len() as f32 * 12.0;
                let pic_top = y0 + 54.0;
                let pic_h = h - 54.0 - call_h - 12.0;
                let cy = pic_top + pic_h + 12.0;
                page.fill_rect(MARGIN, cy, cw, call_h, PANEL);
                page.stroke_rect(MARGIN, cy, cw, call_h, 0.75, RULE);
                for (r, (e, n, note)) in rows.iter().enumerate() {
                    let x = MARGIN + 8.0 + (r % cols) as f32 * (cw - 16.0) / cols as f32;
                    let y = cy + 7.0 + (r / cols) as f32 * 46.0;
                    callout_row(page, x, y, (cw - 16.0) / cols as f32 - 8.0, e, *n, note.as_deref(), self.palette);
                }
                for (l, line) in wrapped.iter().enumerate() {
                    page.text(MARGIN + 8.0, cy + 14.0 + n_rows as f32 * 46.0 + l as f32 * 12.0, 9.0, Font::Bold, INK, line);
                }
                (MARGIN, pic_top, PAGE_W - 2.0 * MARGIN, pic_h)
            } else {
                let call_w = 170.0;
                let wrapped = text_w(call_w - 16.0);
                let call_h = 14.0 + rows.len() as f32 * 46.0 + wrapped.len() as f32 * 12.0;
                let cx = PAGE_W - MARGIN - call_w;
                let cy = y0 + 10.0;
                page.fill_rect(cx, cy, call_w, call_h, PANEL);
                page.stroke_rect(cx, cy, call_w, call_h, 0.75, RULE);
                for (r, (e, n, note)) in rows.iter().enumerate() {
                    callout_row(page, cx + 8.0, cy + 7.0 + r as f32 * 46.0, call_w - 16.0, e, *n, note.as_deref(), self.palette);
                }
                for (l, line) in wrapped.iter().enumerate() {
                    page.text(cx + 8.0, cy + 14.0 + rows.len() as f32 * 46.0 + l as f32 * 12.0, 9.0, Font::Bold, INK, line);
                }
                (MARGIN + 44.0, y0 + 4.0, cx - MARGIN - 52.0, h - 8.0)
            };
            page.image(id, pic_box.0, pic_box.1, pic_box.2, pic_box.3);
            // Hole numbers over the picture; the holes filled in this step stand out.
            let (ix, iy, sc) = id.fit(pic_box.0, pic_box.1, pic_box.2, pic_box.3);
            for &(n, x, y, lit) in &pic.holes {
                let (px, py) = (ix + x * sc, iy + y * sc);
                let (fill, ink) = if lit { (ACCENT, [255, 255, 255]) } else { ([255, 255, 255], INK) };
                page.circle(px, py, 6.5, Some(fill), Some((INK, 0.6)));
                page.text_center(px, py + 2.6, 7.5, Font::Bold, ink, &n.to_string());
            }
        }
        Ok(out)
    }

    /// A stem build page (tinted, like a kit's): "Build ×N stem S03", what one stem needs,
    /// then the head pushed on, each leaf clipped on, and the finished stem, three across.
    #[allow(clippy::too_many_arguments)]
    fn stem_build(
        &mut self,
        model: &Model,
        group: &[usize],
        when: &[usize],
        pics: &[RgbaImage],
        entries: &BTreeMap<(String, ColorId), Entry>,
        subs: &[SubBuild],
        sub_pages: &[usize],
    ) -> Result<()> {
        let sp = &model.stems[group[0]];
        let palette = self.palette;
        let ids: Vec<ImageId> = pics.iter().map(|i| self.add(i)).collect::<Result<_>>()?;
        let n = group.len();
        let mut when: Vec<usize> = when.to_vec();
        when.sort_unstable();
        when.dedup();
        let for_steps = match when.as_slice() {
            [one] => format!("for step {one}"),
            [first, .., last] => format!("for steps {first}–{last}"),
            [] => String::new(),
        };
        let title = format!("{}, {}", stem::title(&sp.def), palette.name(sp.color));
        let box_page = |b: &mut Self, cont: bool| -> f32 {
            let page = b.new_page();
            let (x, y, w, h) = (MARGIN - 10.0, MARGIN - 12.0, PAGE_W - 2.0 * MARGIN + 20.0, PAGE_H - 52.0 - (MARGIN - 12.0));
            page.fill_rect(x, y, w, h, TINT);
            page.stroke_rect(x, y, w, h, 1.0, TINT_EDGE);
            let head = format!("Build ×{n}");
            page.text(MARGIN + 2.0, MARGIN + 16.0, 24.0, Font::Bold, TINT_INK, &head);
            let hx = MARGIN + 14.0 + pdf::text_width(&head, Font::Bold, 24.0);
            page.fill_rect(hx, MARGIN + 4.0, 12.0, 12.0, palette.rgb(sp.color));
            page.stroke_rect(hx, MARGIN + 4.0, 12.0, 12.0, 0.5, MUTED);
            let what = format!("{title}{}", if cont { " (continued)" } else { "" });
            page.text(hx + 18.0, MARGIN + 14.5, 13.0, Font::Bold, TINT_INK, &what);
            page.text_right(PAGE_W - MARGIN - 2.0, MARGIN + 14.0, 9.0, Font::Regular, TINT_INK, &for_steps);
            MARGIN + 30.0
        };
        let mut y = box_page(self, false);
        let inner_w = PAGE_W - 2.0 * MARGIN - 4.0;
        let x0 = MARGIN + 2.0;

        // What one stem needs: the stem, its head, its leaves.
        let kit_page = |a: &sf_core::StemAttach| kit::of_attach(subs, model, a, palette).map(|k| (k, sub_pages[k]));
        let attach_entry = |a: &sf_core::StemAttach| -> Option<&Entry> {
            match kit::of_attach(subs, model, a, palette) {
                Some(k) => entries.get(&(subs[k].key(k), subs[k].main)),
                None => entries.get(&(format!("design:{}", a.design), a.color)),
            }
        };
        let mut need: Vec<(&Entry, usize, String)> = Vec::new();
        for a in stem::attachments(sp) {
            let Some(e) = attach_entry(a) else { continue };
            match need.iter_mut().find(|(x, ..)| std::ptr::eq(*x, e)) {
                Some(r) => r.1 += 1,
                None => {
                    let note = match kit_page(a) {
                        Some((_, page)) if page > 0 => format!("built on page {page}"),
                        _ => palette.name(a.color),
                    };
                    need.push((e, 1, note));
                }
            }
        }
        {
            let page = self.doc.pages.last_mut().expect("page");
            page.fill_rect(x0, y, inner_w, 64.0, [255, 255, 255]);
            page.stroke_rect(x0, y, inner_w, 64.0, 0.6, TINT_EDGE);
            page.text(x0 + 8.0, y + 13.0, 8.5, Font::Regular, MUTED, "For each stem you need");
            if let Some(e) = entries.get(&(format!("stem:{}", sp.def.id), sp.color)) {
                page.image(e.thumb, x0 + 8.0, y + 18.0, 150.0, 40.0);
                page.text(x0 + 164.0, y + 36.0, 11.0, Font::Bold, INK, &format!("{} ×1", sp.def.id));
                let what = if sp.def.style.trim().is_empty() { "stem" } else { sp.def.style.trim() };
                page.text(x0 + 164.0, y + 48.0, 8.5, Font::Regular, MUTED, &pdf::fit(&format!("{:.0} mm {what}", sp.def.length), Font::Regular, 8.5, 70.0));
            }
            let mut x = x0 + 236.0;
            for (e, count, note) in &need {
                if x + 90.0 > x0 + inner_w {
                    break;
                }
                page.image(e.thumb, x, y + 18.0, 38.0, 38.0);
                page.text(x + 42.0, y + 34.0, 10.0, Font::Bold, INK, &pdf::fit(&format!("{} ×{count}", e.title), Font::Bold, 10.0, 66.0));
                page.text(x + 42.0, y + 46.0, 8.0, Font::Regular, MUTED, &pdf::fit(note, Font::Regular, 8.0, 66.0));
                x += 116.0;
            }
        }
        y += 74.0;

        // The steps of the build, as the pictures go: head, leaves, finished.
        // One line per step of the plan (as the pictures go), then the finished stem.
        let mut texts: Vec<String> = Vec::new();
        for slots in stem::plan(sp) {
            let Some(a) = slots.first().and_then(|&s| stem::slot(sp, s)) else { continue };
            let name = attach_entry(a).map(|e| e.title.clone()).unwrap_or_else(|| stem::humanize(&a.design));
            if let stem::Slot::Leaf(i) = slots[0] {
                let (bar, at, side) = stem::leaf_place(model, sp, i);
                let place = match (bar, at) {
                    (Some(b), Some(at)) => format!(" onto bar {b} ({at:.0} mm up the stem)"),
                    _ => String::new(),
                };
                texts.push(format!("Clip a {name}{place}, {side}."));
                continue;
            }
            let first = match kit_page(a) {
                Some((_, page)) if page > 0 && slots.len() > 1 => format!("Take {} {name}s you built on page {page}. ", slots.len()),
                Some((_, page)) if page > 0 => format!("Take a {name} you built on page {page}. "),
                _ => String::new(),
            };
            let on_tip = slots.contains(&stem::Slot::Head);
            let text = match (slots.len(), slots[0]) {
                (1, stem::Slot::Branch(b)) => {
                    format!("Push the {name} onto the bar at the end of branch {}, its face pointing {} as shown.", b + 1, stem::facing(sp, a))
                }
                (1, _) => format!("Push the {name} onto the bar at the tip of the stem, its face pointing {} as shown.", stem::facing(sp, a)),
                (n, _) if on_tip => format!("Push a {name} onto each of the {n} bar ends (the stem's tip and {} branch ends), faces as shown.", n - 1),
                (n, _) => format!("Push a {name} onto each of the {n} branch ends, faces as shown."),
            };
            texts.push(format!("{first}{text}"));
        }
        let heads = usize::from(sp.head.is_some()) + sp.branch_heads.len();
        let mut on: Vec<String> = Vec::new();
        match heads {
            0 => {}
            1 => on.push("its head".into()),
            n => on.push(format!("its {n} heads")),
        }
        match sp.leaves.len() {
            0 => {}
            1 => on.push(if on.is_empty() { "its leaf".into() } else { "leaf".into() }),
            n => on.push(if on.is_empty() { format!("its {n} leaves") } else { format!("{n} leaves") }),
        }
        let extras = if on.is_empty() { String::new() } else { format!(" with {}", on.join(" and ")) };
        texts.push(format!("Stem {}{extras}, ready to plug in.", sp.def.id));

        const CELL_H: f32 = 270.0;
        let cell_w = (inner_w - 20.0) / 3.0;
        let bottom = PAGE_H - 60.0;
        for (k, (id, text)) in ids.iter().zip(&texts).enumerate() {
            let col = k % 3;
            if col == 0 && y + CELL_H > bottom {
                y = box_page(self, true);
            }
            let x = x0 + col as f32 * (cell_w + 10.0);
            let page = self.doc.pages.last_mut().expect("page");
            page.fill_rect(x, y, cell_w, CELL_H, [255, 255, 255]);
            page.stroke_rect(x, y, cell_w, CELL_H, 0.6, TINT_EDGE);
            page.text(x + 8.0, y + 22.0, 18.0, Font::Bold, TINT_INK, &format!("{}", k + 1));
            page.image(*id, x + 8.0, y + 30.0, cell_w - 16.0, 180.0);
            let mut ty = y + 226.0;
            for line in pdf::wrap(text, Font::Regular, 8.5, cell_w - 16.0).into_iter().take(4) {
                page.text(x + 8.0, ty, 8.5, Font::Regular, INK, &line);
                ty += 11.0;
            }
            if col == 2 {
                y += CELL_H + 10.0;
            }
        }
        Ok(())
    }

    /// A tinted sub-build box: "Build ×N", the exploded kit, then the kit's steps two
    /// across (four on the first page, six on the next ones).
    #[allow(clippy::too_many_arguments)]
    fn sub_build(
        &mut self,
        model: &Model,
        sb: &SubBuild,
        for_what: &str,
        exploded: &RgbaImage,
        kit_steps: &[RgbaImage],
        entries: &BTreeMap<(String, ColorId), Entry>,
    ) -> Result<()> {
        let kit = &model.kits[&sb.design];
        let exploded = self.add(exploded)?;
        let step_ids: Vec<ImageId> = kit_steps.iter().map(|i| self.add(i)).collect::<Result<_>>()?;
        let palette = self.palette;
        let n = sb.count();
        let label = sb.label.clone();
        let box_page = |b: &mut Self, cont: bool| -> f32 {
            let page = b.new_page();
            let (x, y, w, h) = (MARGIN - 10.0, MARGIN - 12.0, PAGE_W - 2.0 * MARGIN + 20.0, PAGE_H - 52.0 - (MARGIN - 12.0));
            page.fill_rect(x, y, w, h, TINT);
            page.stroke_rect(x, y, w, h, 1.0, TINT_EDGE);
            page.text(MARGIN + 2.0, MARGIN + 16.0, 24.0, Font::Bold, TINT_INK, &format!("Build ×{n}"));
            let hx = MARGIN + 14.0 + pdf::text_width(&format!("Build ×{n}"), Font::Bold, 24.0);
            page.fill_rect(hx, MARGIN + 4.0, 12.0, 12.0, palette.rgb(sb.main));
            page.stroke_rect(hx, MARGIN + 4.0, 12.0, 12.0, 0.5, MUTED);
            let what = format!("{label}, {}{}", palette.name(sb.main), if cont { " (continued)" } else { "" });
            page.text(hx + 18.0, MARGIN + 14.5, 13.0, Font::Bold, TINT_INK, &what);
            page.text_right(PAGE_W - MARGIN - 2.0, MARGIN + 14.0, 9.0, Font::Regular, TINT_INK, for_what);
            MARGIN + 30.0
        };
        let mut y = box_page(self, false);
        let inner_w = PAGE_W - 2.0 * MARGIN - 4.0;
        let x0 = MARGIN + 2.0;
        {
            let page = self.doc.pages.last_mut().expect("page");
            page.fill_rect(x0, y, inner_w, 210.0, [255, 255, 255]);
            page.stroke_rect(x0, y, inner_w, 210.0, 0.6, TINT_EDGE);
            page.text(x0 + 8.0, y + 14.0, 8.5, Font::Regular, MUTED, &format!("Every piece of one {label}"));
            page.image(exploded, x0 + 8.0, y + 20.0, inner_w - 16.0, 184.0);
        }
        y += 220.0;
        const CELL_H: f32 = 200.0;
        let cell_w = (inner_w - 10.0) / 2.0;
        let bottom = PAGE_H - 60.0;
        for (j, (st, id)) in kit.steps.iter().zip(step_ids).enumerate() {
            let col = j % 2;
            if col == 0 && y + CELL_H > bottom {
                y = box_page(self, true);
            }
            let x = x0 + col as f32 * (cell_w + 10.0);
            let page = self.doc.pages.last_mut().expect("page");
            page.fill_rect(x, y, cell_w, CELL_H, [255, 255, 255]);
            page.stroke_rect(x, y, cell_w, CELL_H, 0.6, TINT_EDGE);
            page.text(x + 8.0, y + 22.0, 18.0, Font::Bold, TINT_INK, &format!("{}", j + 1));
            page.image(id, x + 30.0, y + 6.0, cell_w - 38.0, 118.0);
            let mut ty = y + 138.0;
            if !st.note.is_empty() {
                for line in pdf::wrap(&st.note, Font::Regular, 8.5, cell_w - 16.0).into_iter().take(2) {
                    page.text(x + 8.0, ty, 8.5, Font::Regular, INK, &line);
                    ty += 11.0;
                }
            }
            // Pieces used in this step, by kind.
            let mut used: BTreeMap<usize, usize> = BTreeMap::new();
            for &(pi, _) in &st.parts {
                *used.entry(pi).or_default() += 1;
            }
            let rows: Vec<(usize, usize)> = used.into_iter().collect();
            let room = ((y + CELL_H - 4.0 - ty.max(y + 150.0)) / 22.0).floor().max(1.0) as usize;
            let ry0 = y + CELL_H - 4.0 - rows.len().min(room) as f32 * 22.0;
            for (r, &(pi, count)) in rows.iter().enumerate() {
                let ry = ry0 + r as f32 * 22.0;
                if r + 1 == room && rows.len() > room {
                    page.text(x + 8.0, ry + 14.0, 8.5, Font::Regular, MUTED, &format!("+{} more kinds", rows.len() - r));
                    break;
                }
                let part = &kit.parts[pi];
                let color = sb.part_colors[pi];
                let key = (format!("{}/{}", sb.design, part.key), color);
                if let Some(e) = entries.get(&key) {
                    page.image(e.thumb, x + 8.0, ry, 20.0, 20.0);
                }
                page.text(x + 32.0, ry + 14.0, 10.0, Font::Bold, INK, &format!("×{count}"));
                let tx = x + 36.0 + pdf::text_width(&format!("×{count}"), Font::Bold, 10.0);
                page.fill_rect(tx, ry + 7.0, 8.0, 8.0, palette.rgb(color));
                page.stroke_rect(tx, ry + 7.0, 8.0, 8.0, 0.4, MUTED);
                let what = pdf::fit(&format!("{}, {}", part.label, palette.name(color)), Font::Regular, 8.5, cell_w - (tx - x) - 20.0);
                page.text(tx + 12.0, ry + 14.0, 8.5, Font::Regular, MUTED, &what);
            }
            if col == 1 {
                y += CELL_H + 10.0;
            }
        }
        Ok(())
    }

    fn finished(&mut self, model: &Model) -> Result<()> {
        if model.placements.is_empty() {
            return Ok(());
        }
        let size = self.opts.step_image_px.max(64);
        let turns = [0u8, 2];
        let imgs: Vec<RgbaImage> =
            parallel_map(&turns, |&t| view::trim(&view::render_turned(model, self.palette, self.meshes, t, size), 10));
        let ids: Vec<ImageId> = imgs.iter().map(|i| self.add(i)).collect::<Result<_>>()?;
        let page = self.new_page();
        let top = Self::heading(page, "Finished!", "");
        let h = (PAGE_H - 60.0 - top) / 2.0 - 16.0;
        for (j, (id, t)) in ids.iter().zip(turns).enumerate() {
            let y = top + j as f32 * (h + 24.0);
            let page = self.doc.pages.last_mut().expect("page");
            page.image(*id, MARGIN, y, PAGE_W - 2.0 * MARGIN, h);
            page.text_center(PAGE_W / 2.0, y + h + 14.0, 9.0, Font::Regular, MUTED, &format!("From the {}", corner(t)));
        }
        Ok(())
    }

    fn footers(&mut self) {
        let n = self.doc.pages.len();
        let title = pdf::fit(&self.title, Font::Regular, 8.5, 360.0);
        for (i, page) in self.doc.pages.iter_mut().enumerate() {
            let y = PAGE_H - 24.0;
            page.line(MARGIN, y - 12.0, PAGE_W - MARGIN, y - 12.0, 0.5, RULE);
            page.text(MARGIN, y, 8.5, Font::Regular, MUTED, &title);
            page.text_right(PAGE_W - MARGIN, y, 8.5, Font::Regular, MUTED, &format!("page {} of {n}", i + 1));
        }
    }
}

/// Where each kind of build went: 1-based pages, and the main step of each stem.
struct StepPages {
    sub_builds: Vec<usize>,
    stem_builds: Vec<usize>,
    stem_steps: Vec<usize>,
    steps: Vec<usize>,
}

/// A main step's picture: the corner it was seen from, and the hub's hole numbers in it
/// (number, x, y in the picture's pixels, filled in this step).
struct StepPic {
    img: RgbaImage,
    turn: u8,
    holes: Vec<(u32, f32, f32, bool)>,
}

/// How many stem holes a hub placement has (its design's source is `{"hub": Hub}`).
fn hub_holes(model: &Model, p: &Placement) -> Option<usize> {
    let CustomKind::Detail { mesh, .. } = &p.custom.as_deref()?.kind else { return None };
    let hub = model.details.get(mesh)?.source.get("hub")?;
    hub.get("holes")?.as_array().map(Vec::len)
}

/// A trimmed brick's description: "printed brick …" becomes "curved brick …".
fn curved_name(name: &str) -> String {
    for (from, to) in [("printed brick", "curved brick"), ("printed plate", "curved plate")] {
        if let Some(rest) = name.strip_prefix(from) {
            return format!("{to}{rest}");
        }
    }
    format!("curved {name}")
}

/// The parts-list wording for a printed part: drop the id `describe()` repeats.
fn describe_printed(name: &str, id: &str) -> String {
    let s = name.replace(&format!(" {id}"), "");
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => s,
    }
}

/// One line of a step's callout: thumbnail, "B017 ×2", and the colour (or `note`, such as
/// "Built on page 7" for a sub-build).
#[allow(clippy::too_many_arguments)]
fn callout_row(page: &mut Page, x: f32, y: f32, w: f32, e: &Entry, n: usize, note: Option<&str>, palette: &Palette) {
    page.fill_rect(x, y, 38.0, 38.0, [255, 255, 255]);
    page.image(e.thumb, x + 1.0, y + 1.0, 36.0, 36.0);
    let label = pdf::fit(&format!("{} ×{n}", e.title), Font::Bold, 11.0, w - 46.0);
    page.text(x + 46.0, y + 16.0, 11.0, Font::Bold, INK, &label);
    let sw = palette.rgb(e.color);
    page.fill_rect(x + 46.0, y + 23.0, 8.0, 8.0, sw);
    page.stroke_rect(x + 46.0, y + 23.0, 8.0, 8.0, 0.4, MUTED);
    let (text, ink) = match note {
        Some(t) => (t.to_string(), TINT_INK),
        None => (palette.name(e.color), MUTED),
    };
    let text = pdf::fit(&text, Font::Regular, 8.5, w - 60.0);
    page.text(x + 58.0, y + 30.5, 8.5, Font::Regular, ink, &text);
}

/// Pad an image to a square on white (thumbnails line up in their boxes).
fn square(img: &RgbaImage) -> RgbaImage {
    let (w, h) = img.dimensions();
    let s = w.max(h);
    let mut out = RgbaImage::from_pixel(s, s, image::Rgba([255, 255, 255, 255]));
    image::imageops::overlay(&mut out, img, ((s - w) / 2) as i64, ((s - h) / 2) as i64);
    out
}

/// `f` over `items` on all cores, results in order.
fn parallel_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 16);
    let chunk = items.len().div_ceil(threads).max(1);
    std::thread::scope(|s| {
        let handles: Vec<_> = items.chunks(chunk).map(|c| s.spawn(|| c.iter().map(&f).collect::<Vec<R>>())).collect();
        handles.into_iter().flat_map(|h| h.join().expect("render thread panicked")).collect()
    })
}

#[cfg(test)]
mod tests;
