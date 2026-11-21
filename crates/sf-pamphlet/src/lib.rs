//! Printable build instructions for a fuSHion model, as a US Letter PDF booklet:
//!
//! 1. **Cover**: title, the finished model, its size and part counts.
//! 2. **Before you build**: how fused studs go together, print settings, how to read a step.
//! 3. **What to print** / **What to buy**: one row per part and colour (thumbnail, number,
//!    description, quantity, a box to tick), grouped by colour.
//! 4. **Steps**: two per page (one when a step adds many kinds of part). The model so far,
//!    earlier parts pale, this step's parts in colour, seen from the iso corner that shows
//!    them best; beside it a callout of the parts to add.
//! 5. **Finished**: the model from two corners.
//!
//! Rendering uses sf-render's CPU renderer; the PDF is written by a small built-in writer
//! (base-14 Helvetica, JPEG images).

pub mod pdf;
#[doc(hidden)]
pub mod sample;
pub mod view;

use anyhow::{Context, Result};
use image::{RgbImage, RgbaImage};
use pdf::{Font, ImageId, PAGE_H, PAGE_W, Page, Pdf};
use sf_core::{ColorId, CustomKind, Model, Palette, Placement, Rot};
use sf_ldraw::MeshLibrary;
use sf_render::{RenderOptions, View, render_part};
use std::collections::BTreeMap;
use std::path::Path;
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
    let entries = b.entries(model)?;
    let steps = steps_of(model);

    let mut info = PamphletInfo { steps: steps.len(), ..Default::default() };
    for e in entries.values() {
        match e.kind {
            Kind::Printed => {
                info.printed_designs += 1;
                info.printed_parts += e.count;
            }
            Kind::Detail => {
                info.detail_designs += 1;
                info.detail_parts += e.count;
            }
            Kind::Catalog => info.catalog_parts += e.count,
        }
    }
    // Designs are counted per part id, not per colour.
    info.printed_designs = distinct(entries.values().filter(|e| e.kind == Kind::Printed));
    info.detail_designs = distinct(entries.values().filter(|e| e.kind == Kind::Detail));

    b.cover(model, &info, steps.len())?;
    b.before_you_build(&info);
    b.parts_pages(&entries, true);
    b.parts_pages(&entries, false);
    b.step_pages(model, &steps, &entries)?;
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
    Catalog,
}

fn kind(p: &Placement) -> Kind {
    match p.custom.as_deref().map(|c| &c.kind) {
        Some(CustomKind::Brick) => Kind::Printed,
        Some(CustomKind::Detail { .. }) => Kind::Detail,
        None => Kind::Catalog,
    }
}

/// One row of the parts lists: a part in one colour.
struct Entry {
    kind: Kind,
    part: String,
    color: ColorId,
    count: usize,
    name: String,
    thumb: ImageId,
}

const MARGIN: f32 = 40.0;
const INK: [u8; 3] = [30, 32, 36];
const MUTED: [u8; 3] = [110, 114, 120];
const RULE: [u8; 3] = [200, 203, 208];
const PANEL: [u8; 3] = [244, 245, 247];
const ACCENT: [u8; 3] = [214, 64, 52];

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

    /// Every part × colour in the model, with a thumbnail each.
    fn entries(&mut self, model: &Model) -> Result<BTreeMap<(String, ColorId), Entry>> {
        let mut samples: BTreeMap<(String, ColorId), (Placement, usize)> = BTreeMap::new();
        for p in &model.placements {
            samples.entry((p.part.clone(), p.color)).or_insert_with(|| (p.clone(), 0)).1 += 1;
        }
        // Thumbnails render in parallel.
        let keys: Vec<(String, ColorId)> = samples.keys().cloned().collect();
        let (palette, meshes, size) = (self.palette, self.meshes, self.opts.thumb_px.max(32));
        let thumb_model = |p: &Placement| -> RgbaImage {
            let mut p = p.clone();
            p.rot = Rot(0);
            let mut m = model.clone();
            m.steps.clear();
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
            out.insert(k.clone(), Entry { kind: kind(p), part: k.0.clone(), color: k.1, count: *count, name: p.name(), thumb });
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
        }
        let mut parts: Vec<String> = Vec::new();
        if info.printed_parts > 0 {
            parts.push(format!("{} printed bricks ({} designs)", info.printed_parts, info.printed_designs));
        }
        if info.detail_parts > 0 {
            parts.push(format!("{} printed details ({} designs)", info.detail_parts, info.detail_designs));
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
            page.text_center(PAGE_W / 2.0, y, 12.0, Font::Regular, INK, &f);
            y += 20.0;
        }
        Ok(())
    }

    fn before_you_build(&mut self, info: &PamphletInfo) {
        let printed = info.printed_parts + info.detail_parts > 0;
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
            y = page.paragraph(MARGIN, y, w, 11.0, Font::Regular, INK, num) + 10.0;

            y = section(page, y, "Print settings");
            let lines = if settings.is_empty() {
                vec!["Use the settings you print LEGO-compatible bricks with. Print studs up.".to_string()]
            } else {
                settings
            };
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
        let rows: Vec<&Entry> = entries.values().filter(|e| (e.kind != Kind::Catalog) == printed).collect();
        if rows.is_empty() {
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
        let total: usize = groups.iter().flat_map(|(_, v)| v.iter().map(|e| e.count)).sum();
        let kinds: usize = groups.iter().map(|(_, v)| v.len()).sum();
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
                        page.text(x + 66.0, y + 18.0, 11.5, Font::Bold, INK, &e.part);
                        let qty = format!("×{}", e.count);
                        page.text_right(x + COL_W, y + 18.0, 13.0, Font::Bold, INK, &qty);
                        let desc = match e.kind {
                            Kind::Catalog => e.name.clone(),
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
    }

    fn step_pages(&mut self, model: &Model, steps: &[Vec<usize>], entries: &BTreeMap<(String, ColorId), Entry>) -> Result<()> {
        if steps.is_empty() {
            return Ok(());
        }
        // Render every step (in parallel): the picture and the corner it was seen from.
        let size = self.opts.step_image_px.max(64);
        let (palette, meshes) = (self.palette, self.meshes);
        let idx: Vec<usize> = (0..steps.len()).collect();
        let rendered: Vec<(RgbaImage, u8)> = parallel_map(&idx, |&k| {
            let mut pal = ScratchPalette::new(palette);
            let (m, new) = view::step_model(model, steps, k, &mut pal, true);
            let turn = view::best_turn(&m, &new, meshes);
            (view::trim(&view::render_turned(&m, &pal.palette, meshes, turn, size), 10), turn)
        });

        let mut slot_free = false; // the lower half of the current page is free
        for (k, (img, turn)) in rendered.into_iter().enumerate() {
            let id = self.add(&img)?;
            // This step's parts, grouped.
            let mut group: BTreeMap<(String, ColorId), usize> = BTreeMap::new();
            for &i in &steps[k] {
                if let Some(p) = model.placements.get(i) {
                    *group.entry((p.part.clone(), p.color)).or_default() += 1;
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
            let page = self.doc.pages.last_mut().expect("page");
            page.text(MARGIN, y0 + 30.0, 32.0, Font::Bold, INK, &format!("{}", k + 1));
            if turn != 0 {
                page.text(MARGIN, y0 + 46.0, 9.0, Font::Regular, ACCENT, &format!("Seen from the {}", corner(turn)));
            }
            let rows: Vec<(&Entry, usize)> = group.iter().filter_map(|(key, &n)| entries.get(key).map(|e| (e, n))).collect();
            if full {
                // Picture across the page, callout grid below it.
                let cols = 3usize;
                let n_rows = rows.len().div_ceil(cols);
                let call_h = 14.0 + n_rows as f32 * 46.0;
                let pic_top = y0 + 54.0;
                let pic_h = h - 54.0 - call_h - 12.0;
                page.image(id, MARGIN, pic_top, PAGE_W - 2.0 * MARGIN, pic_h);
                let cy = pic_top + pic_h + 12.0;
                let cw = PAGE_W - 2.0 * MARGIN;
                page.fill_rect(MARGIN, cy, cw, call_h, PANEL);
                page.stroke_rect(MARGIN, cy, cw, call_h, 0.75, RULE);
                for (j, (e, n)) in rows.iter().enumerate() {
                    let x = MARGIN + 8.0 + (j % cols) as f32 * (cw - 16.0) / cols as f32;
                    let y = cy + 7.0 + (j / cols) as f32 * 46.0;
                    callout_row(page, x, y, (cw - 16.0) / cols as f32 - 8.0, e, *n, self.palette);
                }
            } else {
                let call_w = 170.0;
                let call_h = 14.0 + rows.len() as f32 * 46.0;
                let cx = PAGE_W - MARGIN - call_w;
                page.image(id, MARGIN + 44.0, y0 + 4.0, cx - MARGIN - 52.0, h - 8.0);
                let cy = y0 + 10.0;
                page.fill_rect(cx, cy, call_w, call_h, PANEL);
                page.stroke_rect(cx, cy, call_w, call_h, 0.75, RULE);
                for (j, (e, n)) in rows.iter().enumerate() {
                    callout_row(page, cx + 8.0, cy + 7.0 + j as f32 * 46.0, call_w - 16.0, e, *n, self.palette);
                }
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

/// The parts-list wording for a printed part: drop the id `describe()` repeats.
fn describe_printed(name: &str, id: &str) -> String {
    let s = name.replace(&format!(" {id}"), "");
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => s,
    }
}

#[allow(clippy::too_many_arguments)]
fn callout_row(page: &mut Page, x: f32, y: f32, w: f32, e: &Entry, n: usize, palette: &Palette) {
    page.fill_rect(x, y, 38.0, 38.0, [255, 255, 255]);
    page.image(e.thumb, x + 1.0, y + 1.0, 36.0, 36.0);
    let label = pdf::fit(&format!("{} ×{n}", e.part), Font::Bold, 11.0, w - 46.0);
    page.text(x + 46.0, y + 16.0, 11.0, Font::Bold, INK, &label);
    let sw = palette.rgb(e.color);
    page.fill_rect(x + 46.0, y + 23.0, 8.0, 8.0, sw);
    page.stroke_rect(x + 46.0, y + 23.0, 8.0, 8.0, 0.4, MUTED);
    let cname = pdf::fit(&palette.name(e.color), Font::Regular, 8.5, w - 60.0);
    page.text(x + 58.0, y + 30.5, 8.5, Font::Regular, MUTED, &cname);
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
