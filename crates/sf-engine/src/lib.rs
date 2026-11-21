//! The fushion engine: spec → voxels → parts → checks → steps.

pub mod curves;
pub mod details;
pub mod pack;
pub mod physics;
pub mod print_pack;
pub mod steps;

use anyhow::Result;
use serde::Serialize;
use sf_core::{Availability, Cell, ColorId, CustomKind, DetailDesign, Model, Palette, UnknownAvailability};
use sf_spec::{CompileOptions, Compiled, Spec, compile};
use std::collections::BTreeMap;
use std::sync::Arc;

pub use physics::{Analysis, analyze};

/// Default part budget when neither the spec nor the caller gives one.
pub const DEFAULT_BUDGET: u32 = 400;

#[derive(Clone)]
pub struct BuildOptions {
    pub palette: Palette,
    pub availability: Arc<dyn Availability>,
    /// Overrides the spec's budget.
    pub budget: Option<u32>,
    /// Overrides the spec's resolution (and skips the budget search).
    pub resolution: Option<u32>,
    pub slopes: bool,
    /// Build for 3D printing: every part is designed for its spot (merged polyomino bricks
    /// with fused studs laid out for what sits on them) instead of taken from LEGO's catalog.
    pub print: bool,
    /// Detail designs the spec's `details` may use, by name.
    pub details: BTreeMap<String, DetailDesign>,
    /// In print mode, exposed brick surfaces follow the true shape (smooth, no studs on
    /// exposed tops) instead of stair-stepping. Off by default: straight bricks step around
    /// curves and only details get custom shapes.
    pub curves: bool,
}

impl Default for BuildOptions {
    fn default() -> Self {
        Self {
            palette: Palette::builtin(),
            availability: Arc::new(UnknownAvailability),
            budget: None,
            resolution: None,
            slopes: true,
            print: false,
            details: BTreeMap::new(),
            curves: false,
        }
    }
}

pub struct BuildResult {
    pub model: Model,
    pub compiled: Compiled,
    pub analysis: Analysis,
    pub report: Report,
}

fn build_at(
    spec: &Spec,
    opts: &BuildOptions,
    image: Option<&image::RgbaImage>,
    res: Option<u32>,
    repair_rounds: u32,
) -> Result<(Compiled, pack::Packed)> {
    let c = compile(spec, &opts.palette, &CompileOptions { resolution: res, image })?;
    details::check_designs(&c.details, &opts.details)?;
    if opts.print {
        // Curves don't change part counts, so the budget search (no repair) skips them.
        let curves = opts.curves && repair_rounds > 0;
        let p = print_pack::pack_print(&c, &c.details, &opts.details, repair_rounds > 0, curves)?;
        let packed = pack::Packed {
            placements: p.placements,
            meshes: p.meshes,
            curve_stats: p.curve_stats,
            dropped: p.dropped,
            anchored: p.anchored,
            compiled: p.compiled,
            unavailable: Vec::new(),
            warnings: p.warnings,
        };
        return Ok((c, packed));
    }
    let mut p = pack::pack(&c, opts.availability.as_ref(), &pack::PackOptions { slopes: opts.slopes, repair_rounds });
    if !c.details.is_empty() {
        let placed = details::place_details(&c.details, &opts.details, &p.placements)?;
        p.placements.extend(placed.placements);
        p.warnings.extend(placed.warnings);
        for d in print_pack::unsupported_details(&p.placements) {
            p.warnings.push(format!("detail {d} has no stud under its base and will fall off"));
        }
    }
    Ok((c, p))
}

/// Build a model from a spec. Without a fixed resolution, picks the largest
/// size whose part count fits the budget.
pub fn build(spec: &Spec, opts: &BuildOptions, image: Option<&image::RgbaImage>) -> Result<BuildResult> {
    let budget = opts.budget.or(spec.budget).unwrap_or(DEFAULT_BUDGET);
    let fixed = opts.resolution.or(spec.resolution).or(spec.relief.as_ref().and_then(|r| r.width));
    let searchable = match &spec.relief {
        None => true,
        Some(r) => r.source == sf_spec::ReliefSource::Image,
    };
    let rounds = pack::PackOptions::default().repair_rounds;
    let (compiled, packed, resolution) = if let Some(r) = fixed {
        let (c, p) = build_at(spec, opts, image, Some(r), rounds)?;
        (c, p, Some(r))
    } else if !searchable {
        let (c, p) = build_at(spec, opts, image, None, rounds)?;
        (c, p, None)
    } else {
        // Binary search on the longest side, 4..=96 studs.
        let (mut lo, mut hi) = (4u32, 96u32);
        let mut best = None;
        while lo <= hi {
            let mid = (lo + hi) / 2;
            // Repair adds a few parts at most, so search without it.
            let (_, p) = build_at(spec, opts, image, Some(mid), 0)?;
            if p.placements.len() as u32 <= budget {
                best = Some(mid);
                lo = mid + 1;
            } else {
                if mid == 4 {
                    break;
                }
                hi = mid - 1;
            }
        }
        // The final build repairs and anchors, which can add a few parts;
        // step down until it really fits.
        let mut r = best.unwrap_or(4);
        let (mut c, mut p) = build_at(spec, opts, image, Some(r), rounds)?;
        while p.placements.len() as u32 > budget && r > 4 {
            r -= 1;
            (c, p) = build_at(spec, opts, image, Some(r), rounds)?;
        }
        (c, p, Some(r))
    };
    let (mut packed, mut compiled) = (packed, compiled);
    if let Some(c) = packed.compiled.take() {
        compiled = c;
    }
    let mut model = Model {
        title: spec.title.clone(),
        mode: spec.mode,
        placements: std::mem::take(&mut packed.placements),
        steps: Vec::new(),
        printable: opts.print,
        meshes: std::mem::take(&mut packed.meshes),
        ..Default::default()
    };
    details::attach_designs(&mut model, &opts.details);
    let analysis = analyze(&model);
    model.steps = steps::order_steps(&model, &analysis);
    let mut report = Report::new(&model, &analysis, &opts.palette, opts.availability.as_ref());
    report.budget = Some(budget);
    report.resolution = resolution;
    if let (Some(p), Some(c)) = (report.printed.as_mut(), packed.curve_stats.take()) {
        p.contacts = Some(c);
    }
    report.warnings.extend(compiled.warnings.iter().cloned());
    report.warnings.extend(packed.warnings.iter().cloned());
    if report.parts as u32 > budget {
        let why = match (fixed, searchable) {
            (Some(r), _) => format!("at the fixed size of {r} studs; lower `resolution` (or the relief width) or remove it"),
            (None, false) => "because a drawn relief is built at the size of its rows; draw fewer or shorter rows".to_string(),
            (None, true) => "even at the smallest size; simplify the shapes".to_string(),
        };
        report.warnings.push(format!("{} parts is over the budget of {budget} {why}", report.parts));
    }
    if packed.anchored > 0 {
        report.warnings.push(format!(
            "{} small feature(s) could not be held as drawn; filled a short gap under them or extended their colour one stud into the body",
            packed.anchored
        ));
    }
    if packed.dropped > 0 {
        report.warnings.push(format!(
            "removed {} unsupported part(s) sticking out sideways where no stud can hold them",
            packed.dropped
        ));
    }
    for (part, color) in packed.unavailable.iter().filter(|(pt, c)| model.placements.iter().any(|p| &p.part == pt && p.color == *c)) {
        report.warnings.push(format!(
            "{} in {} was never produced; placed anyway (custom part candidate)",
            part,
            opts.palette.name(*color)
        ));
    }
    Ok(BuildResult { model, compiled, analysis, report })
}

/// Recompute steps and the report after manual edits.
pub fn reanalyze(model: &mut Model, palette: &Palette, availability: &dyn Availability) -> (Analysis, Report) {
    let analysis = analyze(model);
    model.steps = steps::order_steps(model, &analysis);
    let report = Report::new(model, &analysis, palette, availability);
    (analysis, report)
}

#[derive(Clone, Debug, Serialize)]
pub struct ColorUse {
    pub color: String,
    pub code: ColorId,
    pub parts: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct Issue {
    pub kind: String,
    pub parts: usize,
    /// Rough location, e.g. "upper left front".
    pub where_: String,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct CustomCandidate {
    pub reason: String,
    pub part: String,
    pub color: String,
    pub count: usize,
}

/// What a model needs printed.
#[derive(Clone, Debug, Default, Serialize)]
pub struct PrintSummary {
    /// Parts to print: body bricks, plus every kit part of every detail (a detail without a
    /// kit counts as one part).
    pub total: usize,
    /// Distinct printed body bricks (one print file each).
    pub unique_bricks: usize,
    /// Printed body bricks.
    pub bricks: usize,
    /// Detail parts placed.
    pub details: usize,
    /// Distinct detail designs used.
    pub detail_designs: usize,
    /// Separately printed parts the details are built from (each kit's copies, times the
    /// details using it).
    pub kit_parts: usize,
    /// Details with no stud under their base, as "printed flower head D001 at (x, y, z)".
    pub unsupported_details: Vec<String>,
    /// Body bricks whose exposed surface follows the true shape.
    pub curved: usize,
    /// Curved bricks with an overhang steeper than 45° (print them with supports).
    pub need_supports: usize,
    /// How contacts were trimmed to the shape (set by the build, not by [`PrintSummary::of`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contacts: Option<print_pack::CurveStats>,
}

impl PrintSummary {
    /// `None` when the model has no printed parts.
    pub fn of(model: &Model, a: &Analysis) -> Option<Self> {
        let ps = &model.placements;
        let mut s = PrintSummary::default();
        let mut ids = std::collections::BTreeSet::new();
        let mut designs = std::collections::BTreeSet::new();
        for p in ps {
            match p.custom.as_deref().map(|c| &c.kind) {
                Some(CustomKind::Brick) => {
                    s.bricks += 1;
                    ids.insert(p.part.clone());
                    if let Some(t) = p.custom.as_ref().and_then(|c| c.trim.as_ref()) {
                        s.curved += 1;
                        s.need_supports += t.overhang as usize;
                    }
                }
                Some(CustomKind::Detail { mesh, .. }) => {
                    s.details += 1;
                    designs.insert(mesh.clone());
                    if let Some(kit) = model.kits.get(mesh) {
                        s.kit_parts += kit.parts.iter().map(|k| k.copies.len()).sum::<usize>();
                    }
                }
                None => {}
            }
        }
        if s.bricks + s.details == 0 {
            return None;
        }
        let kitless = ps
            .iter()
            .filter(|p| matches!(p.custom.as_deref().map(|c| &c.kind), Some(CustomKind::Detail { mesh, .. }) if !model.kits.contains_key(mesh)))
            .count();
        s.total = s.bricks + s.kit_parts + kitless;
        s.unique_bricks = ids.len();
        s.detail_designs = designs.len();
        let held: std::collections::HashSet<usize> = a.connections.iter().map(|&(_, b, _)| b).collect();
        for (i, p) in ps.iter().enumerate() {
            if details::is_detail(p) && p.origin.y > 0 && !held.contains(&i) {
                s.unsupported_details.push(format!("{} at ({}, {}, {})", p.name(), p.origin.x, p.origin.y, p.origin.z));
            }
        }
        Some(s)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub title: String,
    pub parts: usize,
    pub budget: Option<u32>,
    pub resolution: Option<u32>,
    /// Width (x) and depth (z) in studs, height in plates.
    pub size_studs: [i32; 3],
    pub size_mm: [f32; 3],
    pub steps: usize,
    pub colors: Vec<ColorUse>,
    pub part_types: BTreeMap<String, usize>,
    pub slopes: usize,
    pub issues: Vec<Issue>,
    pub balance_ok: bool,
    pub custom_candidates: Vec<CustomCandidate>,
    /// Printed parts, when the model has any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub printed: Option<PrintSummary>,
    pub warnings: Vec<String>,
}

fn region(c: (f32, f32, f32), lo: Cell, hi: Cell) -> String {
    let t = |v: f32, a: i32, b: i32| if b - a <= 0 { 0.5 } else { (v - a as f32) / (b - a) as f32 };
    let (fx, fy, fz) = (t(c.0, lo.x, hi.x), t(c.1, lo.y, hi.y), t(c.2, lo.z, hi.z));
    let v = if fy > 0.66 { "upper" } else if fy < 0.33 { "lower" } else { "middle" };
    // Viewer at -z: +x is the viewer's right.
    let h = if fx > 0.66 { "right" } else if fx < 0.33 { "left" } else { "centre" };
    let d = if fz > 0.66 { "back" } else if fz < 0.33 { "front" } else { "middle" };
    format!("{v} {h} {d} (viewer's left/right)")
}

impl Report {
    pub fn new(model: &Model, a: &Analysis, palette: &Palette, avail: &dyn Availability) -> Self {
        let ps = &model.placements;
        let (lo, hi) = model.bounds().unwrap_or((Cell::new(0, 0, 0), Cell::new(0, 0, 0)));
        let size = [hi.x - lo.x, hi.z - lo.z, hi.y - lo.y];
        let mut colors: BTreeMap<ColorId, usize> = BTreeMap::new();
        let mut types: BTreeMap<String, usize> = BTreeMap::new();
        let mut combos: BTreeMap<(String, ColorId), usize> = BTreeMap::new();
        for p in ps {
            *colors.entry(p.color).or_default() += 1;
            let name = p.name();
            *types.entry(name).or_default() += 1;
            if p.custom.is_none() {
                *combos.entry((p.part.clone(), p.color)).or_default() += 1;
            }
        }
        let mut colors: Vec<ColorUse> =
            colors.into_iter().map(|(code, n)| ColorUse { color: palette.name(code), code, parts: n }).collect();
        colors.sort_by_key(|c| std::cmp::Reverse(c.parts));
        let centre = |idx: &[usize]| -> (f32, f32, f32) {
            let n = idx.len().max(1) as f32;
            let s = idx.iter().fold((0.0, 0.0, 0.0), |acc, &i| {
                let p = &ps[i];
                let [fx, fz] = p.footprint();
                (acc.0 + p.origin.x as f32 + fx as f32 / 2.0, acc.1 + p.origin.y as f32, acc.2 + p.origin.z as f32 + fz as f32 / 2.0)
            });
            (s.0 / n, s.1 / n, s.2 / n)
        };
        let mut issues = Vec::new();
        if !a.overlaps.is_empty() {
            issues.push(Issue {
                kind: "overlap".into(),
                parts: a.overlaps.len() * 2,
                where_: region(centre(&a.overlaps.iter().flatten().copied().collect::<Vec<_>>()), lo, hi),
                detail: format!("{} pairs of parts occupy the same space", a.overlaps.len()),
            });
        }
        for g in a.components.iter().filter(|g| !g.iter().any(|&i| ps[i].origin.y == 0)) {
            issues.push(Issue {
                kind: "floating".into(),
                parts: g.len(),
                where_: region(centre(g), lo, hi),
                detail: format!("{} parts not connected to the rest or the ground", g.len()),
            });
        }
        for w in a.weak_joints.iter().take(8) {
            let c = (w.at.x as f32 + 0.5, w.at.y as f32, w.at.z as f32 + 0.5);
            issues.push(Issue {
                kind: "weak joint".into(),
                parts: w.parts_held,
                where_: region(c, lo, hi),
                detail: format!("{} parts hang on a single stud at height {} plates", w.parts_held, w.at.y),
            });
        }
        if !a.balance.ok && !ps.is_empty() {
            issues.push(Issue {
                kind: "off balance".into(),
                parts: ps.len(),
                where_: region((a.balance.center_of_mass[0], a.balance.center_of_mass[1], a.balance.center_of_mass[2]), lo, hi),
                detail: format!("centre of mass is {:.1} studs outside the footprint", -a.balance.margin.min(0.0)),
            });
        }
        let mut custom = Vec::new();
        for ((part, color), n) in &combos {
            let name = palette.name(*color);
            match avail.is_made(part, *color) {
                Some(false) => custom.push(CustomCandidate {
                    reason: "colour never made".into(),
                    part: part.clone(),
                    color: name,
                    count: *n,
                }),
                _ => {
                    if let Some(s) = avail.set_count(part, *color)
                        && s <= 3
                    {
                        custom.push(CustomCandidate {
                            reason: format!("rare (in {s} sets)"),
                            part: part.clone(),
                            color: name,
                            count: *n,
                        });
                    }
                }
            }
        }
        let small = ps.iter().filter(|p| p.def().is_some_and(|d| d.size == [1, 1])).count();
        if ps.len() >= 40 && small * 3 > ps.len() {
            custom.push(CustomCandidate {
                reason: format!("{small} of {} parts are 1 × 1; clusters could be merged into custom parts", ps.len()),
                part: "1x1".into(),
                color: String::new(),
                count: small,
            });
        }
        Report {
            title: model.title.clone(),
            parts: ps.len(),
            budget: None,
            resolution: None,
            size_studs: size,
            size_mm: [size[0] as f32 * 8.0, size[1] as f32 * 8.0, size[2] as f32 * 3.2],
            steps: model.steps.len(),
            colors,
            part_types: types,
            slopes: ps.iter().filter(|p| p.def().is_some_and(|d| d.is_slope())).count(),
            issues,
            balance_ok: a.balance.ok,
            custom_candidates: custom,
            printed: PrintSummary::of(model, a),
            warnings: Vec::new(),
        }
    }

    /// A compact text report for Claude and the UI.
    pub fn to_text(&self) -> String {
        use std::fmt::Write;
        let mut s = String::new();
        let _ = writeln!(
            s,
            "{} parts{} · {} × {} studs, {:.1} bricks tall ({:.0} × {:.0} × {:.0} mm) · {} steps · {} slopes",
            self.parts,
            self.budget.map(|b| format!(" (budget {b})")).unwrap_or_default(),
            self.size_studs[0],
            self.size_studs[1],
            self.size_studs[2] as f32 / 3.0,
            self.size_mm[0],
            self.size_mm[1],
            self.size_mm[2],
            self.steps,
            self.slopes,
        );
        let cols: Vec<String> = self.colors.iter().take(10).map(|c| format!("{} {}", c.color, c.parts)).collect();
        let _ = writeln!(s, "Colours: {}", cols.join(", "));
        if self.issues.is_empty() {
            let _ = writeln!(s, "Structure: OK (one connected piece, balanced, no single-stud joints holding 5+ parts)");
        } else {
            let _ = writeln!(s, "Structure issues:");
            for i in &self.issues {
                let _ = writeln!(s, "- {}: {} [{}]", i.kind, i.detail, i.where_);
            }
        }
        if let Some(p) = &self.printed {
            let kits = if p.kit_parts > 0 { format!(" built from {} kit parts", p.kit_parts) } else { String::new() };
            let _ = writeln!(
                s,
                "Printed: {} parts + {} details{kits} ({} to print in all; {} different brick files, {} detail design{})",
                p.bricks,
                p.details,
                p.total,
                p.unique_bricks,
                p.detail_designs,
                if p.detail_designs == 1 { "" } else { "s" }
            );
            if p.curved > 0 {
                let _ = writeln!(
                    s,
                    "Curved: {} bricks follow the true surface ({} need supports)",
                    p.curved, p.need_supports
                );
                if let Some(c) = &p.contacts {
                    let _ = writeln!(
                        s,
                        "Contacts: {} of {} between curved bricks trimmed to the shape ({} studs dropped){}",
                        c.contacts - c.covered - c.restored,
                        c.contacts,
                        c.studs_before.saturating_sub(c.studs_after),
                        if c.restored > 0 {
                            format!("; {} kept flat so {} part(s) stay held", c.restored, c.parts_restored)
                        } else {
                            String::new()
                        }
                    );
                    if c.ground_fallback {
                        let _ = writeln!(s, "Base: kept square (following the outline would tip the model over)");
                    } else if c.ground_columns > 0 {
                        let _ = writeln!(
                            s,
                            "Base: follows the outline; {} flat pad(s) where the shape doesn't reach the floor",
                            c.pads
                        );
                    }
                }
            }
            for d in &p.unsupported_details {
                let _ = writeln!(s, "Unsupported detail: {d} has no stud under its base and will fall off");
            }
        }
        for c in &self.custom_candidates {
            let _ = writeln!(s, "Custom part candidate: {} {} × {} ({})", c.part, c.color, c.count, c.reason);
        }
        for w in &self.warnings {
            let _ = writeln!(s, "Warning: {w}");
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_search_fits() {
        let spec = Spec::from_json(
            r#"{"title":"snowman","budget":150,"shapes":[
            {"shape":"ellipsoid","center":[0,4,0],"size":[8,8,8],"color":"White"},
            {"shape":"ellipsoid","center":[0,10,0],"size":[5.5,5.5,5.5],"color":"White"},
            {"shape":"cone","center":[0,10,-3.2],"size":[1,1,2],"axis":"z","rotate":[180,0,0],"color":"Orange"}
        ]}"#,
        )
        .unwrap();
        let r = build(&spec, &BuildOptions::default(), None).unwrap();
        assert!(r.report.parts <= 150, "{}", r.report.parts);
        assert!(r.report.parts > 40, "should use most of the budget: {}", r.report.parts);
        assert!(r.analysis.overlaps.is_empty());
        let total: usize = r.model.steps.iter().map(|s| s.len()).sum();
        assert_eq!(total, r.model.placements.len());
        println!("{}", r.report.to_text());
    }
}
