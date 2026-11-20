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
pub struct Studforge {
    session: Arc<Mutex<Session>>,
    #[allow(dead_code)] // read by the #[tool_handler] macro
    tool_router: ToolRouter<Self>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct BuildParams {
    /// A complete studforge shape spec (see get_guide).
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
    /// Use the experimental compliant stud instead of the standard FDM stud.
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
pub struct BudgetParams {
    /// Maximum number of parts, or null to use the spec's budget.
    pub budget: Option<u32>,
}

fn internal(e: impl std::fmt::Display) -> McpError {
    McpError::internal_error(e.to_string(), None)
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[tool_router]
impl Studforge {
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

    #[tool(description = "Read the studforge spec guide: the shape language, colours, modes and how to work. Call this first.")]
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

    #[tool(description = "Get the current spec and the latest report (for continuing an earlier session).")]
    async fn get_state(&self) -> Result<CallToolResult, McpError> {
        let text = self
            .with_session(|s| {
                let spec = serde_json::to_string_pretty(&s.spec)?;
                let report = s.report.as_ref().map(|r| r.to_text()).unwrap_or_else(|| "nothing built yet".into());
                Ok(format!("Budget: {:?}\nReport:\n{report}\nSpec:\n{spec}", s.budget))
            })
            .await?;
        Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
    }

    #[tool(description = "Design a custom 3D-printable LEGO-compatible part from a template (box, 45° slope, curved slope, or any footprint of studs). Writes STL and 3MF (mm) into the session's custom folder. Use it for parts the report lists as custom part candidates.")]
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
impl ServerHandler for Studforge {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "studforge turns a picture into a buildable brick model. Call get_guide, look at the picture with \
             view_source_image, then iterate with build_model until the renders read as the subject and the \
             report shows no structure issues.",
        )
    }
}

pub fn serve(session: Session) -> Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    rt.block_on(async move {
        let service = Studforge::new(session).serve(rmcp::transport::stdio()).await?;
        service.waiting().await?;
        Ok(())
    })
}
