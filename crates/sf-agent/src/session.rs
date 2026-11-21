//! A build session: the source image, the current spec, the built model and
//! its report, all mirrored to files in one directory so the desktop app, the
//! CLI and the MCP server (a separate process) see the same state.
//!
//! Files: `source.png`, `spec.json`, `model.json`, `report.json`,
//! `report.txt`, `render.png`, `conversation.json`, `history/NNN-spec.json`,
//! `budget.txt`, `print.txt` ("1"/"0", build for 3D printing; missing means on) and
//! `details/<name>.json` (detail designs) with `details/<name>.png` (their pictures).

use anyhow::{Context, Result, bail};
use sf_core::{Availability, DetailDesign, Model, Palette, UnknownAvailability};
use sf_engine::{BuildOptions, Report};
use sf_ldraw::MeshLibrary;
use sf_render::{RenderOptions, View};
use sf_spec::Spec;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Output of one build, ready to hand to Claude.
pub struct BuildOutput {
    pub report: Report,
    pub report_text: String,
    /// PNG bytes of the multi-view render sheet.
    pub sheet_png: Vec<u8>,
}

pub struct Session {
    pub dir: PathBuf,
    pub palette: Palette,
    pub availability: Arc<dyn Availability>,
    pub catalog: Option<Arc<sf_catalog::CatalogIndex>>,
    pub meshes: Arc<MeshLibrary>,
    pub image: Option<image::RgbaImage>,
    pub spec: Option<Spec>,
    pub model: Option<Model>,
    pub report: Option<Report>,
    /// Part budget the user chose (overrides the spec's).
    pub budget: Option<u32>,
    /// Build for 3D printing: every brick designed for its place (default on).
    pub print: bool,
    source_mtime: Option<std::time::SystemTime>,
}

/// Views in the render sheet, in order.
pub const SHEET_VIEWS: [View; 6] = [View::Front, View::Right, View::Back, View::Left, View::Top, View::Iso];
/// Views for reliefs (a picture lying flat): top and iso, larger.
pub const RELIEF_VIEWS: [View; 2] = [View::Top, View::Iso];

pub fn default_sessions_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("fushion").join("sessions")
}

/// Load the shared resources (catalog, meshes). Missing data degrades
/// gracefully: no catalog means unknown availability; no LDraw library means
/// procedural meshes.
pub fn load_resources() -> (Palette, Arc<dyn Availability>, Option<Arc<sf_catalog::CatalogIndex>>, Arc<MeshLibrary>) {
    let catalog = sf_catalog::CatalogIndex::load_default().ok().flatten().map(Arc::new);
    let palette = catalog.as_ref().map(|c| c.palette()).unwrap_or_else(Palette::builtin);
    let availability: Arc<dyn Availability> = match &catalog {
        Some(c) => c.clone(),
        None => Arc::new(UnknownAvailability),
    };
    let meshes = sf_ldraw::ensure_installed()
        .ok()
        .and_then(|dir| MeshLibrary::open(&dir).ok())
        .unwrap_or_else(MeshLibrary::procedural);
    (palette, availability, catalog, Arc::new(meshes))
}

impl Session {
    /// Open (or create) a session directory, loading any saved state.
    pub fn open(dir: &Path) -> Result<Self> {
        let (palette, availability, catalog, meshes) = load_resources();
        Self::open_with(dir, palette, availability, catalog, meshes)
    }

    pub fn open_with(
        dir: &Path,
        palette: Palette,
        availability: Arc<dyn Availability>,
        catalog: Option<Arc<sf_catalog::CatalogIndex>>,
        meshes: Arc<MeshLibrary>,
    ) -> Result<Self> {
        std::fs::create_dir_all(dir.join("history")).with_context(|| format!("creating {}", dir.display()))?;
        // Absolute, so paths stay valid for child processes run elsewhere.
        let dir = std::fs::canonicalize(dir).with_context(|| format!("resolving {}", dir.display()))?;
        let mut s = Session {
            dir,
            palette,
            availability,
            catalog,
            meshes,
            image: None,
            spec: None,
            model: None,
            report: None,
            budget: None,
            print: true,
            source_mtime: None,
        };
        s.reload()?;
        Ok(s)
    }

    /// Re-read state files written by another process.
    pub fn reload(&mut self) -> Result<()> {
        let src = self.dir.join("source.png");
        let mtime = std::fs::metadata(&src).and_then(|m| m.modified()).ok();
        if mtime.is_some() && (mtime != self.source_mtime || self.image.is_none()) {
            self.image = Some(image::open(&src)?.to_rgba8());
            self.source_mtime = mtime;
        }
        if let Ok(t) = std::fs::read_to_string(self.dir.join("spec.json")) {
            self.spec = serde_json::from_str(&t).ok();
        }
        if let Ok(t) = std::fs::read_to_string(self.dir.join("budget.txt")) {
            self.budget = t.trim().parse().ok();
        }
        self.print = std::fs::read_to_string(self.dir.join("print.txt")).map(|t| t.trim() != "0").unwrap_or(true);
        if let Ok(t) = std::fs::read_to_string(self.dir.join("model.json")) {
            self.model = serde_json::from_str(&t).ok();
        }
        if let Some(m) = &mut self.model {
            // Saved models may use old part ids.
            for p in &mut m.placements {
                p.part = sf_core::parts::canonical(&p.part).to_string();
            }
        }
        self.report = None;
        if let Some(mut m) = self.model.take() {
            self.report = Some(self.analyze(&mut m));
            self.model = Some(m);
        }
        Ok(())
    }

    pub fn set_budget(&mut self, budget: Option<u32>) -> Result<()> {
        self.budget = budget;
        match budget {
            Some(b) => std::fs::write(self.dir.join("budget.txt"), b.to_string())?,
            None => {
                let _ = std::fs::remove_file(self.dir.join("budget.txt"));
            }
        }
        Ok(())
    }

    /// Build for 3D printing (every brick designed for its place) or with real LEGO parts.
    pub fn set_print(&mut self, print: bool) -> Result<()> {
        self.print = print;
        std::fs::write(self.dir.join("print.txt"), if print { "1" } else { "0" })?;
        Ok(())
    }

    /// The folder holding this session's detail designs.
    pub fn details_dir(&self) -> PathBuf {
        self.dir.join("details")
    }

    /// Every detail design saved in this session, by name.
    pub fn designs(&self) -> BTreeMap<String, DetailDesign> {
        crate::details::load_all(&self.details_dir())
    }

    /// Design a detail part with SolidCraft (see [`crate::details::make`]), save it under
    /// `name` (replacing an older design of that name) and return it with a PNG picture of
    /// it from four sides in `color` (also saved as `details/<name>.png`).
    pub fn design_detail(&self, request: &serde_json::Value, name: &str, label: &str, color: &str) -> Result<(DetailDesign, Vec<u8>)> {
        let name = crate::details::clean_name(name)?;
        let label = if label.trim().is_empty() { name.replace('_', " ") } else { label.trim().to_string() };
        let design = crate::details::make(request, &name, &label)?;
        let colour = self.palette.resolve(color).unwrap_or(sf_core::color::FILLER);
        let png = crate::details::preview_png(&design, colour, &self.palette, &self.meshes)?;
        let dir = self.details_dir();
        std::fs::create_dir_all(&dir)?;
        write_atomic(&dir.join(format!("{name}.json")), &serde_json::to_string(&design)?)?;
        write_bytes_atomic(&dir.join(format!("{name}.png")), &png)?;
        Ok((design, png))
    }

    /// A picture of a saved design from four sides, as PNG bytes.
    pub fn detail_picture(&self, name: &str, color: &str) -> Result<Vec<u8>> {
        let name = crate::details::clean_name(name)?;
        let designs = self.designs();
        let Some(d) = designs.get(&name) else {
            let have: Vec<&str> = designs.keys().map(String::as_str).collect();
            bail!("no design named `{name}`; designs: {}", if have.is_empty() { "none yet".into() } else { have.join(", ") });
        };
        let colour = self.palette.resolve(color).unwrap_or(sf_core::color::FILLER);
        crate::details::preview_png(d, colour, &self.palette, &self.meshes)
    }

    /// Set the source image (any format `image` reads). Stored as PNG.
    pub fn set_image(&mut self, path: &Path) -> Result<()> {
        let img = image::open(path).with_context(|| format!("reading image {}", path.display()))?.to_rgba8();
        write_bytes_atomic(&self.dir.join("source.png"), &encode_png(&img)?)?;
        self.image = Some(img);
        Ok(())
    }

    /// Set the source image from pixels already in memory (a pasted picture).
    pub fn set_image_rgba(&mut self, img: image::RgbaImage) -> Result<()> {
        write_bytes_atomic(&self.dir.join("source.png"), &encode_png(&img)?)?;
        self.image = Some(img);
        Ok(())
    }

    pub fn source_png_path(&self) -> PathBuf {
        self.dir.join("source.png")
    }

    /// The source image as PNG bytes, downscaled so its long side is at most
    /// `max_side` pixels (keeps API requests small).
    pub fn source_png(&self, max_side: u32) -> Result<Vec<u8>> {
        let Some(img) = &self.image else { bail!("no source image set") };
        let (w, h) = img.dimensions();
        let scaled = if w.max(h) > max_side {
            let f = max_side as f32 / w.max(h) as f32;
            image::imageops::resize(
                img,
                ((w as f32 * f) as u32).max(1),
                ((h as f32 * f) as u32).max(1),
                image::imageops::FilterType::Triangle,
            )
        } else {
            img.clone()
        };
        encode_png(&scaled)
    }

    /// Build a spec (JSON text), save everything, and return the report and
    /// render sheet.
    pub fn build_spec(&mut self, spec_json: &str) -> Result<BuildOutput> {
        let spec = Spec::from_json(spec_json)?;
        let opts = BuildOptions {
            palette: self.palette.clone(),
            availability: self.availability.clone(),
            budget: self.budget,
            print: self.print,
            details: self.designs(),
            ..Default::default()
        };
        let result = sf_engine::build(&spec, &opts, self.image.as_ref())?;
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        let name = format!("{stamp}-{}-spec.json", std::process::id());
        write_atomic(&self.dir.join("history").join(name), &serde_json::to_string_pretty(&spec)?)?;
        self.spec = Some(spec);
        self.model = Some(result.model);
        self.report = Some(result.report);
        self.save()?;
        let report = self.report.clone().expect("just set");
        let text = report.to_text();
        let sheet = self.render_sheet()?;
        Ok(BuildOutput { report, report_text: text, sheet_png: sheet })
    }

    /// Replace the model after manual edits; recompute steps and report.
    /// Steps and report for a model, with the session's budget filled in.
    fn analyze(&self, model: &mut Model) -> Report {
        let (_, mut r) = sf_engine::reanalyze(model, &self.palette, self.availability.as_ref());
        r.budget = self.budget.or(self.spec.as_ref().and_then(|s| s.budget));
        r
    }

    pub fn set_model(&mut self, mut model: Model) -> Result<Report> {
        let r = self.analyze(&mut model);
        self.model = Some(model);
        self.report = Some(r.clone());
        self.save()?;
        Ok(r)
    }

    /// Render the current model's 6-view sheet as PNG bytes and save it.
    pub fn render_sheet(&self) -> Result<Vec<u8>> {
        let Some(model) = &self.model else { bail!("nothing built yet") };
        let relief = model.mode == sf_core::ModelMode::Relief;
        let (views, size): (&[View], u32) = if relief { (&RELIEF_VIEWS, 576) } else { (&SHEET_VIEWS, 384) };
        let opts = RenderOptions { view_size: size, edges: true, background: [236, 238, 241] };
        let img = sf_render::render_sheet(model, &self.palette, &self.meshes, views, &opts);
        let png = encode_png(&img)?;
        write_bytes_atomic(&self.dir.join("render.png"), &png)?;
        Ok(png)
    }

    fn save(&self) -> Result<()> {
        write_atomic(&self.dir.join("spec.json"), &serde_json::to_string_pretty(&self.spec)?)?;
        if let Some(m) = &self.model {
            write_atomic(&self.dir.join("model.json"), &serde_json::to_string(m)?)?;
        }
        if let Some(r) = &self.report {
            write_atomic(&self.dir.join("report.json"), &serde_json::to_string_pretty(r)?)?;
            write_atomic(&self.dir.join("report.txt"), &r.to_text())?;
        }
        Ok(())
    }

    /// Write the LDraw model, parts CSV, BrickLink wanted list and any
    /// custom printable parts into `out`.
    pub fn export(&self, out: &Path) -> Result<Vec<PathBuf>> {
        self.export_job()?.run(out)
    }

    /// Everything an export needs, detached from the session so it can run
    /// on another thread (custom parts take about a second each).
    pub fn export_job(&self) -> Result<ExportJob> {
        let Some(model) = &self.model else { bail!("nothing built yet") };
        Ok(ExportJob {
            model: model.clone(),
            palette: self.palette.clone(),
            catalog: self.catalog.clone(),
            availability: self.availability.clone(),
            meshes: self.meshes.clone(),
        })
    }

    /// Generate a custom part into the session's `custom` folder.
    pub fn make_custom_part(&self, shape: &serde_json::Value, v2: bool, name: &str) -> Result<crate::custom::MadePart> {
        crate::custom::make_part(shape, v2, &self.dir.join("custom"), name)
    }

    /// Generate a freeform custom part from SolidCraft commands into the
    /// session's `custom` folder.
    pub fn make_custom_from_commands(&self, batch: &serde_json::Value, name: &str) -> Result<crate::custom::MadePart> {
        crate::custom::make_part_from_commands(batch, &self.dir.join("custom"), name)
    }
}

/// A detached export (see [`Session::export_job`]).
#[derive(Clone)]
pub struct ExportJob {
    pub model: Model,
    pub palette: Palette,
    pub catalog: Option<Arc<sf_catalog::CatalogIndex>>,
    pub availability: Arc<dyn Availability>,
    pub meshes: Arc<MeshLibrary>,
}

/// Print advice for the pamphlet's "Before you build" page.
pub const PRINT_SETTINGS: &[&str] = &[
    "Print every part studs up, on its base, exactly as the files are oriented.",
    "0.2 mm layers with a 0.2 mm first layer; every height in these parts is a multiple of 0.2 mm.",
    "0.4 mm nozzle: Arachne (variable line width) on, 0.40-0.42 mm lines, XY compensation 0, no ironing.",
    "PETG holds the fused studs' grip longer than PLA; PLA works for display models.",
    "Bricks have their number engraved underneath (B017 is engraved 017). Details are listed with a picture.",
    "Details marked \"supports\" in print-list.csv need support material under their petals or leaves.",
];

impl ExportJob {
    pub fn run(&self, out: &Path) -> Result<Vec<PathBuf>> {
        let model = &self.model;
        std::fs::create_dir_all(out)?;
        let cat = self.catalog.as_deref();
        let mut written = Vec::new();
        let slug = slug(&model.title);
        let ldr = out.join(format!("{slug}.ldr"));
        std::fs::write(&ldr, sf_export::write_ldr(model, &self.palette))?;
        written.push(ldr);
        let csv = out.join(format!("{slug}-parts.csv"));
        std::fs::write(&csv, sf_export::write_parts_csv(model, &self.palette, cat))?;
        written.push(csv);
        match sf_export::write_bricklink_xml(model, cat, &self.palette) {
            Ok(xml) => {
                let p = out.join(format!("{slug}-bricklink.xml"));
                std::fs::write(&p, xml)?;
                written.push(p);
            }
            Err(e) => eprintln!("BrickLink wanted list skipped: {e}"),
        }
        // Printed parts: one STL/3MF per part design, per colour, plus print-list.csv.
        let printed = model.placements.iter().any(|p| p.custom.is_some());
        if printed && crate::custom::ENABLED {
            let set = crate::custom::export_print_set(model, &self.palette, &out.join("print"), |_, _| {})?;
            written.extend(set.files);
            if !set.notes.is_empty() {
                let p = out.join("print").join("notes.txt");
                std::fs::write(&p, set.notes.join("\n"))?;
                written.push(p);
            }
        }
        // Instructions pamphlet.
        let opts = sf_pamphlet::PamphletOptions {
            title: Some(model.title.clone()).filter(|t| !t.trim().is_empty()),
            print_settings: if printed { PRINT_SETTINGS.iter().map(|s| s.to_string()).collect() } else { Vec::new() },
            numbers_on_parts: printed,
            ..Default::default()
        };
        let pdf = out.join(format!("{slug}-instructions.pdf"));
        sf_pamphlet::write_pamphlet(model, &self.palette, &self.meshes, &opts, &pdf)?;
        written.push(pdf);
        let needs = crate::custom::custom_needs(model, self.availability.as_ref());
        if !needs.is_empty() && crate::custom::ENABLED {
            let (files, notes) = crate::custom::make_needed_parts(&needs, &self.palette, &out.join("custom"))?;
            written.extend(files);
            if !notes.is_empty() {
                let p = out.join("custom").join("notes.txt");
                std::fs::create_dir_all(out.join("custom"))?;
                std::fs::write(&p, notes.join("\n"))?;
                written.push(p);
            }
        }
        Ok(written)
    }
}

pub fn slug(title: &str) -> String {
    let s: String = title
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if s.is_empty() { "model".into() } else { s }
}

pub fn encode_png(img: &image::RgbaImage) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    image::DynamicImage::ImageRgba8(img.clone()).write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)?;
    Ok(buf)
}

/// Write via a uniquely named temporary file and rename, so readers in
/// other processes never see a half-written file.
pub fn write_atomic(path: &Path, text: &str) -> Result<()> {
    write_bytes_atomic(path, text.as_bytes())
}

pub fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let tmp = path.with_file_name(format!(".{name}.{}.{n}.tmp", std::process::id()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })?;
    Ok(())
}
