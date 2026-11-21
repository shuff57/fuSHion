//! Detail parts Claude designs for a model (a flower head, a leaf, an eye, a beak):
//! making them with SolidCraft (`sf-custom`), keeping them in the session's `details`
//! folder, and rendering a picture of each so Claude (and the user) can check it.

use anyhow::{Context, Result, bail};
use serde_json::Value;
use sf_core::{Cell, ColorId, DetailDesign, Model, Palette, Placement, Rot};
use sf_ldraw::MeshLibrary;
use sf_render::{RenderOptions, View};
use std::collections::BTreeMap;
use std::path::Path;

/// Views in a detail's preview sheet.
pub const PREVIEW_VIEWS: [View; 4] = [View::Iso, View::Front, View::Right, View::Top];

/// A design name as stored: lower case letters, digits and `_`.
pub fn clean_name(name: &str) -> Result<String> {
    let n: String = name
        .trim()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' })
        .collect();
    let n = n.trim_matches('_').to_string();
    if n.is_empty() || n.len() > 48 {
        bail!("a design name is 1 to 48 letters, digits or underscores, e.g. \"daisy_head\"");
    }
    Ok(n)
}

/// Make a detail from a request: `{"template": "rose", "params": {...}}` (see
/// [`templates_text`]) or `{"commands": [...], "cells": [[0,0],[1,0]], "plates": 3,
/// "sockets": [[0,0],[1,0]]}` (a SolidCraft batch in the detail's frame).
#[cfg(feature = "custom")]
pub fn make(request: &Value, name: &str, label: &str) -> Result<DetailDesign> {
    let err = |e: sf_custom::CustomError| anyhow::anyhow!("{e}");
    if let Some(t) = request.get("template").and_then(Value::as_str) {
        let params = request.get("params").cloned().unwrap_or_else(|| serde_json::json!({}));
        return Ok(sf_custom::detail_template(name, label, t, &params).map_err(err)?.design);
    }
    if let Some(cmds) = request.get("commands") {
        let pairs = |k: &str| -> Result<Vec<(i32, i32)>> {
            serde_json::from_value(request.get(k).cloned().unwrap_or(Value::Null))
                .with_context(|| format!("`{k}` must be a list of [column, row] pairs"))
        };
        let plates = request.get("plates").and_then(Value::as_u64).context("`plates` (height in plates) is required")? as u32;
        let batch = serde_json::json!({"commands": cmds});
        return Ok(sf_custom::detail_from_commands(&batch, &pairs("cells")?, plates, &pairs("sockets")?, name, label)
            .map_err(err)?
            .design);
    }
    bail!("a detail request is {{\"template\": name, \"params\": {{...}}}} or {{\"commands\": [...], \"cells\": ..., \"plates\": n, \"sockets\": ...}}")
}

#[cfg(not(feature = "custom"))]
pub fn make(_: &Value, _: &str, _: &str) -> Result<DetailDesign> {
    bail!("this fushion was built without custom bricks (the `custom-bricks` feature)")
}

/// The detail templates and their parameters, one per line.
pub fn templates_text() -> String {
    #[cfg(feature = "custom")]
    {
        sf_custom::TEMPLATES.iter().map(|(n, p)| format!("- {n}: {p}")).collect::<Vec<_>>().join("\n")
    }
    #[cfg(not(feature = "custom"))]
    {
        String::from("(custom parts are not available in this build)")
    }
}

/// A one-part model holding `design` in `color`, for pictures.
pub fn preview_model(design: &DetailDesign, color: ColorId) -> Model {
    let mut model = Model { title: design.label.clone(), printable: true, ..Default::default() };
    model.meshes.insert(design.name.clone(), design.mesh.clone());
    model.details.insert(design.name.clone(), design.source.clone());
    model.placements.push(Placement::printed(design.instance(&design.name), color, Cell::new(0, 0, 0), Rot(0)));
    model
}

/// A picture of a detail from four sides (iso, front, right, top), as PNG bytes.
pub fn preview_png(design: &DetailDesign, color: ColorId, palette: &Palette, meshes: &MeshLibrary) -> Result<Vec<u8>> {
    let model = preview_model(design, color);
    let opts = RenderOptions { view_size: 320, edges: true, background: [236, 238, 241] };
    let img = sf_render::render_sheet(&model, palette, meshes, &PREVIEW_VIEWS, &opts);
    crate::session::encode_png(&img)
}

/// A few lines about a design for Claude: its footprint, height, and for a kit its parts
/// (with copies and whether they need supports) and colour roles.
pub fn summary(d: &DetailDesign) -> String {
    let mut s = format!(
        "{} (\"{}\"): cells {:?}, sockets {:?}, {} plates tall",
        d.name, d.label, d.cells, d.sockets, d.plates
    );
    match &d.kit {
        Some(k) => {
            let parts: Vec<String> = k
                .parts
                .iter()
                .map(|p| format!("{} ×{} ({}{})", p.label, p.copies.len(), p.role, if p.supports { ", needs supports" } else { "" }))
                .collect();
            let roles: Vec<String> = k.roles.iter().map(|(r, c)| format!("{r} (default {c})")).collect();
            s.push_str(&format!(
                ". Kit of {} printed parts in {} kinds: {}. Colour roles: {}; set \"color\" for main and \"colors\": {{role: colour}} for the rest in the spec's details.",
                k.parts.iter().map(|p| p.copies.len()).sum::<usize>(),
                k.parts.len(),
                parts.join(", "),
                roles.join(", ")
            ));
        }
        None => s.push_str(". One printed piece."),
    }
    s
}

/// Every design saved in `dir`, by name. Unreadable files are skipped with a note on stderr.
pub fn load_all(dir: &Path) -> BTreeMap<String, DetailDesign> {
    let mut out = BTreeMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return out };
    for e in entries.flatten() {
        let p = e.path();
        if p.extension().is_none_or(|x| x != "json") {
            continue;
        }
        match std::fs::read_to_string(&p).map_err(anyhow::Error::from).and_then(|t| Ok(serde_json::from_str::<DetailDesign>(&t)?)) {
            Ok(d) => {
                out.insert(d.name.clone(), d);
            }
            Err(err) => eprintln!("fushion: skipping detail design {}: {err:#}", p.display()),
        }
    }
    out
}
