//! MCP server (stdio) so Claude Code, or any MCP client, can design models
//! with the user's own subscription. Every build is written to the session
//! directory, which the desktop app watches.

use anyhow::Result;
use base64::Engine as _;
use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt,
    handler::server::{tool::ToolRouter, wrapper::Parameters},
    model::*,
    schemars, tool, tool_handler, tool_router,
};
use serde::Deserialize;
use sf_agent::Session;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct Fushion {
    session: Arc<Mutex<Session>>,
    #[allow(dead_code)] // read by the #[tool_handler] macro
    tool_router: ToolRouter<Self>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct BuildParams {
    /// A complete fuSHion shape spec (see get_guide).
    pub spec: serde_json::Value,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ImageParams {
    /// Path to an image file (PNG, JPEG or WebP).
    pub path: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ExportParams {
    /// Folder to write the LDraw model, parts CSV and BrickLink list into.
    /// Defaults to the session's `export` folder.
    pub dir: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct CustomParams {
    /// The part, e.g. {"kind":"box","w":2,"d":4,"plates":3},
    /// {"kind":"slope45","w":2,"d":3,"plates":3}, {"kind":"curve","w":1,"d":3,"plates":3}
    /// or {"kind":"footprint","cells":[[0,0],[1,0],[1,1]],"plates":3}. Sizes in studs and
    /// plates (3 plates = 1 brick); slopes fall toward -Y.
    pub shape: serde_json::Value,
    /// File name for the part (without extension).
    pub name: String,
    /// Deprecated and ignored: every custom part has fused studs. Accepted so older calls
    /// still work.
    #[serde(default)]
    pub compliant_stud: bool,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct CustomCommandsParams {
    /// A SolidCraft batch: {"commands":[{"command":"solid.box","params":{...}}, ...],
    /// "expect":{"volume_mm3":..., "tol":..., "bodies":1, "bbox":{...}}}. Millimetres;
    /// only sketch.*, solid.*, parameters.* and body.rename commands are allowed.
    pub batch: serde_json::Value,
    /// File name for the part (without extension).
    pub name: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct DetailParams {
    /// Name to save the design under (letters, digits, `_`), used by the spec's `details`.
    pub name: String,
    /// Short human name for the parts list, e.g. "red rose". Defaults to the name.
    #[serde(default)]
    pub label: String,
    /// What to make: {"template": "rose", "params": {...}} (see list_details for templates
    /// and their parameters) or {"commands": [SolidCraft commands], "cells": [[0,0],[1,0]],
    /// "plates": 3, "sockets": [[0,0],[1,0]]} drawn in the detail's frame (column (i, j)
    /// spans x 8i..8i+8, y 8j..8j+8 mm; it stands on z = 0; the socket base is built for you
    /// with its top at z = 3.2 mm, and your commands should join onto it).
    pub design: serde_json::Value,
    /// Colour for the picture (the model sets each use's colour). Default Light Bluish Gray.
    #[serde(default)]
    pub color: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ViewDetailParams {
    /// A saved design's name.
    pub name: String,
    /// Colour for the picture.
    #[serde(default)]
    pub color: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct BudgetParams {
    /// Maximum number of parts, or null to use the spec's budget.
    pub budget: Option<u32>,
}

fn internal(e: impl std::fmt::Display) -> McpError {
    McpError::internal_error(e.to_string(), None)
}

/// One line about a detail design.
fn detail_summary(d: &sf_core::DetailDesign) -> String {
    format!(
        "{} (\"{}\"): cells {:?}, sockets {:?}, {} plates tall, {} triangles",
        d.name,
        d.label,
        d.cells,
        d.sockets,
        d.plates,
        d.mesh.indices.len() / 3
    )
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[tool_router]
impl Fushion {
    pub fn new(session: Session) -> Self {
        Self { session: Arc::new(Mutex::new(session)), tool_router: Self::tool_router() }
    }

    async fn with_session<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Session) -> Result<T> + Send + 'static,
    ) -> Result<T, McpError> {
        let s = self.session.clone();
        tokio::task::spawn_blocking(move || {
            let mut guard = s.lock().map_err(|_| internal("session lock poisoned"))?;
            // Pick up edits the desktop app made since the last call.
            let _ = guard.reload();
            f(&mut guard).map_err(|e| internal(format!("{e:#}")))
        })
        .await
        .map_err(internal)?
    }

    #[tool(description = "Read the fuSHion spec guide: the shape language, colours, modes and how to work. Call this first.")]
    async fn get_guide(&self) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::success(vec![ContentBlock::text(sf_spec::GUIDE)]))
    }

    #[tool(description = "Show the source picture for this session.")]
    async fn view_source_image(&self) -> Result<CallToolResult, McpError> {
        let png = self.with_session(|s| s.source_png(1568)).await?;
        Ok(CallToolResult::success(vec![ContentBlock::image(b64(&png), "image/png")]))
    }

    #[tool(description = "Set the source picture for this session from an image file path.")]
    async fn set_source_image(&self, Parameters(p): Parameters<ImageParams>) -> Result<CallToolResult, McpError> {
        let path = PathBuf::from(p.path);
        self.with_session(move |s| s.set_image(&path)).await?;
        Ok(CallToolResult::success(vec![ContentBlock::text("Source picture set.")]))
    }

    #[tool(description = "Set the part budget for builds in this session.")]
    async fn set_budget(&self, Parameters(p): Parameters<BudgetParams>) -> Result<CallToolResult, McpError> {
        self.with_session(move |s| s.set_budget(p.budget)).await?;
        Ok(CallToolResult::success(vec![ContentBlock::text("Budget set.")]))
    }

    #[tool(description = "Compile a shape spec into real parts, check the structure, and return the report plus a six-view render sheet (front, right, back, left, top, iso). Pass the complete spec each time.")]
    async fn build_model(&self, Parameters(p): Parameters<BuildParams>) -> Result<CallToolResult, McpError> {
        let spec = p.spec.to_string();
        match self.with_session(move |s| s.build_spec(&spec)).await {
            Ok(out) => Ok(CallToolResult::success(vec![
                ContentBlock::text(out.report_text),
                ContentBlock::image(b64(&out.sheet_png), "image/png"),
            ])),
            Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(e.message.to_string())])),
        }
    }

    #[tool(description = "Design a printed detail part with SolidCraft: a flower head (rose, daisy, aster, poppy...), a petal, a leaf or fern frond, a stem, an eye dome, a beak or horn, a round plate, or anything from your own SolidCraft commands. It is checked (one closed solid, watertight, socket base underneath that fits the studs below), saved under `name`, and returned with a picture from four sides (iso, front, right, top). Look at the picture and redesign until it reads as the feature, then place it with the spec's `details`.")]
    async fn design_detail(&self, Parameters(p): Parameters<DetailParams>) -> Result<CallToolResult, McpError> {
        match self.with_session(move |s| s.design_detail(&p.design, &p.name, &p.label, &p.color)).await {
            Ok((d, png)) => Ok(CallToolResult::success(vec![
                ContentBlock::text(detail_summary(&d)),
                ContentBlock::image(b64(&png), "image/png"),
            ])),
            Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(e.message.to_string())])),
        }
    }

    #[tool(description = "List this session's detail designs (name, label, cells, sockets, height) and the detail templates with their parameters.")]
    async fn list_details(&self) -> Result<CallToolResult, McpError> {
        let text = self
            .with_session(|s| {
                let designs = s.designs();
                let mut out = String::from("Designs:\n");
                if designs.is_empty() {
                    out.push_str("(none yet)\n");
                }
                for d in designs.values() {
                    out.push_str(&format!("- {}\n", detail_summary(d)));
                }
                out.push_str("\nTemplates (params in mm and degrees; base in studs):\n");
                out.push_str(&sf_agent::details::templates_text());
                Ok(out)
            })
            .await?;
        Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
    }

    #[tool(description = "Show a picture of a saved detail design from four sides (iso, front, right, top).")]
    async fn view_detail(&self, Parameters(p): Parameters<ViewDetailParams>) -> Result<CallToolResult, McpError> {
        match self.with_session(move |s| s.detail_picture(&p.name, &p.color)).await {
            Ok(png) => Ok(CallToolResult::success(vec![ContentBlock::image(b64(&png), "image/png")])),
            Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(e.message.to_string())])),
        }
    }

    #[tool(description = "Get the current spec and the latest report (for continuing an earlier session).")]
    async fn get_state(&self) -> Result<CallToolResult, McpError> {
        let text = self
            .with_session(|s| {
                let spec = serde_json::to_string_pretty(&s.spec)?;
                let report = s.report.as_ref().map(|r| r.to_text()).unwrap_or_else(|| "nothing built yet".into());
                let mode = if s.print { "3D printing (every brick designed for its place)" } else { "real LEGO parts" };
                let designs: Vec<String> = s.designs().keys().cloned().collect();
                Ok(format!(
                    "Budget: {:?}\nBuilding for: {mode}\nDetail designs: {}\nReport:\n{report}\nSpec:\n{spec}",
                    s.budget,
                    if designs.is_empty() { "none".into() } else { designs.join(", ") }
                ))
            })
            .await?;
        Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
    }

    #[tool(description = "Design a custom 3D-printable LEGO-compatible part from a template (box, 45° slope, curved slope, or any footprint of studs). Every part has fused studs: each row's studs are joined in pairs, plus one triple for an odd run, with leaf-spring fins that grip, so a part can't have a stud alone in its row (no 1x1; a 1-wide slope is refused). Bricks pushed onto it must start and end on a pair or triple, never between its studs. Writes STL and 3MF (mm) into the session's custom folder. Use it for parts the report lists as custom part candidates.")]
    async fn make_custom_part(&self, Parameters(p): Parameters<CustomParams>) -> Result<CallToolResult, McpError> {
        match self.with_session(move |s| s.make_custom_part(&p.shape, p.compliant_stud, &p.name)).await {
            Ok(m) => Ok(CallToolResult::success(vec![ContentBlock::text(format!(
                "Made {} ({:.0} mm³) and {}",
                m.stl.display(),
                m.volume_mm3,
                m.three_mf.display()
            ))])),
            Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(e.message.to_string())])),
        }
    }

    #[tool(description = "Design a freeform custom part with SolidCraft CAD commands (the same command language as SolidCraft's MCP server). The batch must include an `expect` block; the result is checked to be one closed solid no larger than 64 mm per side. Writes STL and 3MF into the session's custom folder.")]
    async fn make_custom_part_from_commands(
        &self,
        Parameters(p): Parameters<CustomCommandsParams>,
    ) -> Result<CallToolResult, McpError> {
        match self.with_session(move |s| s.make_custom_from_commands(&p.batch, &p.name)).await {
            Ok(m) => Ok(CallToolResult::success(vec![ContentBlock::text(format!(
                "Made {} ({:.0} mm³) and {}",
                m.stl.display(),
                m.volume_mm3,
                m.three_mf.display()
            ))])),
            Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(e.message.to_string())])),
        }
    }

    #[tool(description = "Export the current model: LDraw file with build steps, parts CSV, BrickLink wanted list, and STL/3MF files for parts whose colour was never produced or is rare.")]
    async fn export_model(&self, Parameters(p): Parameters<ExportParams>) -> Result<CallToolResult, McpError> {
        let files = self
            .with_session(move |s| {
                let dir = p.dir.map(PathBuf::from).unwrap_or_else(|| s.dir.join("export"));
                s.export(&dir)
            })
            .await?;
        let list: Vec<String> = files.iter().map(|f| f.display().to_string()).collect();
        Ok(CallToolResult::success(vec![ContentBlock::text(format!("Wrote:\n{}", list.join("\n")))]))
    }
}

#[tool_handler]
impl ServerHandler for Fushion {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "fuSHion turns a picture into a buildable brick model. Call get_guide, look at the picture with \
             view_source_image, then iterate with build_model until the renders read as the subject and the \
             report shows no structure issues. Design printed detail parts (flowers, petals, leaves, eyes, \
             beaks) with design_detail, check their pictures, and place them with the spec's `details`.",
        )
    }
}

pub fn serve(session: Session) -> Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    rt.block_on(async move {
        let service = Fushion::new(session).serve(rmcp::transport::stdio()).await?;
        service.waiting().await?;
        Ok(())
    })
}
