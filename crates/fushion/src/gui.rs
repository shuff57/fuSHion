//! The desktop app: eframe + wgpu.
//!
//! Layout: Design (left), 3D viewer with build steps (centre), Report and
//! export (right), status line (bottom). Claude turns run on worker threads;
//! the session folder is polled so builds written by another process (the MCP
//! server driven by Claude Code) show up live.

mod clip;
mod design;
mod edit;
mod gpu;
mod report;
mod viewer;
mod worker;

use anyhow::{Result, anyhow};
use egui::{Color32, RichText};
use sf_agent::Session;
use sf_core::{ColorId, Model, Placement, Rot};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use viewer::{Camera, Preset};
use worker::{Backend, Resources, Worker};

/// The one accent colour.
pub const ACCENT: Color32 = Color32::from_rgb(232, 163, 61);
const ERROR: Color32 = Color32::from_rgb(235, 110, 100);
const OK: Color32 = Color32::from_rgb(120, 200, 140);

/// Options for `fushion gui`.
#[derive(Default)]
pub struct GuiOptions {
    pub session: Option<PathBuf>,
    /// Render a few frames, save a screenshot here, and exit.
    pub screenshot: Option<PathBuf>,
}

pub fn run_with(opts: GuiOptions) -> Result<()> {
    let res = Resources::load();
    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("fuSHion")
            .with_app_id("fushion")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([960.0, 600.0])
            .with_drag_and_drop(true),
        renderer: eframe::Renderer::Wgpu,
        multisampling: gpu::MSAA as u16,
        depth_buffer: 32,
        ..Default::default()
    };
    eframe::run_native(
        "fushion",
        native,
        Box::new(move |cc| {
            let Some(rs) = cc.wgpu_render_state.as_ref() else {
                return Err("the wgpu renderer is not available".into());
            };
            gpu::install(rs);
            let srgb = rs.target_format.is_srgb();
            cc.egui_ctx
                .data_mut(|d| d.insert_temp(egui::Id::unique("fushion_srgb_target"), srgb));
            apply_theme(&cc.egui_ctx);
            Ok(Box::new(App::new(&cc.egui_ctx, res, opts)))
        }),
    )
    .map_err(|e| anyhow!("{e}"))
}

fn apply_theme(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    let mut v = egui::Visuals::dark();
    v.panel_fill = Color32::from_rgb(27, 29, 33);
    v.window_fill = Color32::from_rgb(32, 34, 39);
    v.extreme_bg_color = Color32::from_rgb(19, 20, 23);
    v.faint_bg_color = Color32::from_rgb(33, 35, 40);
    v.selection.bg_fill = ACCENT.gamma_multiply(0.55);
    v.selection.stroke.color = ACCENT;
    v.hyperlink_color = ACCENT;
    v.widgets.noninteractive.bg_stroke.color = Color32::from_rgb(44, 47, 53);
    v.widgets.inactive.weak_bg_fill = Color32::from_rgb(42, 45, 51);
    v.widgets.inactive.bg_fill = Color32::from_rgb(42, 45, 51);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(54, 58, 66);
    v.widgets.active.weak_bg_fill = Color32::from_rgb(62, 66, 75);
    v.slider_trailing_fill = true;
    ctx.set_visuals_of(egui::Theme::Dark, v);
    ctx.style_mut_of(egui::Theme::Dark, |s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(8.0, 4.0);
        s.spacing.interact_size.y = 24.0;
    });
}

#[derive(Clone, Debug, PartialEq)]
enum ChatKind {
    User,
    Claude,
    Progress,
    Error,
}

#[derive(Clone, Debug)]
struct ChatLine {
    kind: ChatKind,
    text: String,
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum Tool {
    Select,
    Add,
}

struct Shot {
    path: PathBuf,
    frames: u32,
    requested: bool,
    done: Arc<Mutex<Option<Result<(), String>>>>,
}

pub struct App {
    res: Resources,
    session: Option<Session>,
    status: Option<(String, bool)>,
    worker: Worker,

    // Viewer.
    cam: Camera,
    fit_pending: bool,
    scene: Arc<gpu::Scene>,
    scene_dirty: bool,
    generation: u64,
    edges: bool,
    vis: viewer::Visibility,
    viewer_aspect: f32,

    // Steps.
    step_mode: bool,
    step: usize,
    playing: bool,
    last_tick: f64,

    // Editing.
    selected: Option<usize>,
    undo: Vec<Model>,
    tool: Tool,
    add_part: &'static str,
    add_color: ColorId,
    add_rot: Rot,
    ghost: Option<Placement>,

    // Design.
    source_tex: Option<egui::TextureHandle>,
    budget: u32,
    /// Build for 3D printing (merged printed bricks and designed details) rather than from
    /// real LEGO parts. Kept per session in `print.txt`.
    print_mode: bool,
    /// Printed bricks follow the true curved surface. Kept per session in `curves.txt`.
    curves: bool,
    backend: Backend,
    api_status: Result<(), String>,
    cc_status: Option<bool>,
    effort: String,
    chat: Vec<ChatLine>,
    input: String,
    busy: Option<(Backend, f64)>,
    key_input: String,
    key_saved: bool,

    /// Pictures of the model's printed parts, by (part id, colour), for the report panel.
    thumbs: std::collections::HashMap<(String, sf_core::ColorId), egui::TextureHandle>,
    /// The model file the pictures were made from.
    thumbs_for: Option<SystemTime>,

    // Sync.
    model_mtime: Option<SystemTime>,
    source_mtime: Option<SystemTime>,
    last_poll: f64,

    exported: Vec<PathBuf>,
    /// A running export (custom parts make it take a few seconds).
    export_task: Option<ExportSlot>,
    shot: Option<Shot>,
    /// Screenshot mode: simulate a click at this point (logical px).
    debug_click: Option<egui::Pos2>,
    /// Screenshot mode: open the settings popover.
    debug_settings: bool,
}

/// Where a background export leaves its result.
type ExportSlot = Arc<Mutex<Option<Result<Vec<PathBuf>, String>>>>;

/// Session file holding the build mode: `1` for 3D printing, `0` for real LEGO parts.
const PRINT_FILE: &str = "print.txt";

/// The session's build mode (printing when the file is missing).
fn read_print_mode(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join(PRINT_FILE)).map_or(true, |t| t.trim() != "0")
}

/// Whether printed bricks follow curves in this session (off when the file is missing).
fn read_curves(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join("curves.txt")).is_ok_and(|t| t.trim() == "1")
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// Session folders, most recently modified first.
fn recent_sessions() -> Vec<(PathBuf, SystemTime)> {
    let Ok(rd) = std::fs::read_dir(sf_agent::session::default_sessions_dir()) else {
        return Vec::new();
    };
    let mut v: Vec<(PathBuf, SystemTime)> = rd
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| {
            let p = e.path();
            let t = ["model.json", "source.png", "spec.json"]
                .iter()
                .filter_map(|f| mtime(&p.join(f)))
                .max()
                .or_else(|| mtime(&p))
                .unwrap_or(SystemTime::UNIX_EPOCH);
            (p, t)
        })
        .collect();
    v.sort_by_key(|a| std::cmp::Reverse(a.1));
    v
}

fn new_session_dir() -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let base = sf_agent::session::default_sessions_dir();
    let mut dir = base.join(format!("s{stamp}"));
    let mut n = 1;
    while dir.exists() {
        dir = base.join(format!("s{stamp}-{n}"));
        n += 1;
    }
    dir
}

fn ago(t: SystemTime) -> String {
    let s = SystemTime::now()
        .duration_since(t)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    match s {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", s / 60),
        3600..86400 => format!("{} h ago", s / 3600),
        _ => format!("{} days ago", s / 86400),
    }
}

impl App {
    fn new(ctx: &egui::Context, res: Resources, opts: GuiOptions) -> Self {
        let worker = Worker::new();
        worker.probe_claude_code(ctx.clone());
        let mut app = Self {
            res,
            session: None,
            status: None,
            worker,
            cam: Camera::default(),
            fit_pending: true,
            scene: Arc::new(gpu::Scene {
                generation: 0,
                palette: sf_core::Palette::builtin(),
                batches: Vec::new(),
                lines: Vec::new(),
                overlay_lines: Vec::new(),
                edges: true,
            }),
            scene_dirty: true,
            generation: 1,
            edges: true,
            vis: viewer::Visibility {
                visible: Vec::new(),
                flags: Vec::new(),
            },
            viewer_aspect: 1.5,
            step_mode: false,
            step: 0,
            playing: false,
            last_tick: 0.0,
            selected: None,
            undo: Vec::new(),
            tool: Tool::Select,
            add_part: "3001",
            add_color: 4,
            add_rot: Rot(0),
            ghost: None,
            source_tex: None,
            budget: sf_engine::DEFAULT_BUDGET,
            print_mode: true,
            curves: false,
            backend: Backend::Api,
            api_status: Ok(()),
            cc_status: None,
            effort: "high".into(),
            chat: Vec::new(),
            input: String::new(),
            busy: None,
            key_input: String::new(),
            key_saved: false,
            thumbs: Default::default(),
            thumbs_for: None,
            model_mtime: None,
            source_mtime: None,
            last_poll: 0.0,
            exported: Vec::new(),
            export_task: None,
            debug_click: None,
            debug_settings: std::env::var("FUSHION_SHOT_SETTINGS").is_ok(),
            shot: opts.screenshot.map(|path| Shot {
                path,
                frames: 0,
                requested: false,
                done: Arc::new(Mutex::new(None)),
            }),
        };
        app.refresh_api_status();
        if app.api_status.is_err() {
            app.backend = Backend::ClaudeCode;
        }
        let dir = opts
            .session
            .or_else(|| recent_sessions().into_iter().next().map(|(p, _)| p))
            .unwrap_or_else(new_session_dir);
        app.open_session(ctx, &dir);
        app.apply_debug_env();
        app
    }

    /// Hidden knobs for screenshots: `FUSHION_SHOT_STEP` (1-based),
    /// `FUSHION_SHOT_SELECT` (placement index), `FUSHION_SHOT_VIEW`.
    fn apply_debug_env(&mut self) {
        if let Some(k) = std::env::var("FUSHION_SHOT_STEP")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
        {
            self.step_mode = true;
            self.step = k.saturating_sub(1);
        }
        if let Some(i) = std::env::var("FUSHION_SHOT_SELECT")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
        {
            self.selected = Some(i);
        }
        if let Ok(v) = std::env::var("FUSHION_SHOT_VIEW")
            && let Some(p) = Preset::ALL
                .iter()
                .find(|p| p.label().eq_ignore_ascii_case(&v))
        {
            self.cam.set_preset(*p);
        }
        if let Ok(v) = std::env::var("FUSHION_SHOT_CLICK")
            && let Some((x, y)) = v.split_once(',')
            && let (Ok(x), Ok(y)) = (x.trim().parse::<f32>(), y.trim().parse::<f32>())
        {
            self.debug_click = Some(egui::pos2(x, y));
        }
        if std::env::var("FUSHION_SHOT_ADD").is_ok() {
            self.tool = Tool::Add;
        }
        self.clamp_state();
    }

    fn set_status(&mut self, msg: impl Into<String>, error: bool) {
        self.status = Some((msg.into(), error));
    }

    fn refresh_api_status(&mut self) {
        self.api_status = sf_agent::claude_api::ClaudeConfig::from_env()
            .map(|_| ())
            .map_err(|_| "No API key yet: add one in Settings".to_string());
    }

    fn model(&self) -> Option<&Model> {
        self.session.as_ref().and_then(|s| s.model.as_ref())
    }

    fn open_session(&mut self, ctx: &egui::Context, dir: &Path) {
        if self.busy.is_some() {
            self.set_status("Wait for Claude to finish before switching sessions", true);
            return;
        }
        match self.res.open(dir) {
            Ok(s) => {
                self.budget = s.budget.unwrap_or(sf_engine::DEFAULT_BUDGET);
                self.print_mode = read_print_mode(&s.dir);
                self.curves = read_curves(&s.dir);
                self.chat = worker::transcript_from_conversation(&s)
                    .into_iter()
                    .map(|(u, t)| ChatLine {
                        kind: if u { ChatKind::User } else { ChatKind::Claude },
                        text: t,
                    })
                    .collect();
                self.model_mtime = mtime(&s.dir.join("model.json"));
                self.source_mtime = mtime(&s.source_png_path());
                self.session = Some(s);
                self.selected = None;
                self.undo.clear();
                self.exported.clear();
                self.step_mode = false;
                self.step = 0;
                self.playing = false;
                self.fit_pending = true;
                self.scene_dirty = true;
                self.load_source_texture(ctx);
                let name = dir
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.set_status(format!("Opened session {name}"), false);
            }
            Err(e) => self.set_status(format!("Could not open {}: {e:#}", dir.display()), true),
        }
    }

    fn load_source_texture(&mut self, ctx: &egui::Context) {
        self.source_tex = self
            .session
            .as_ref()
            .and_then(|s| s.image.as_ref())
            .map(|img| {
                let (w, h) = img.dimensions();
                let f = (512.0 / w.max(h) as f32).min(1.0);
                let small = if f < 1.0 {
                    image::imageops::resize(
                        img,
                        ((w as f32 * f) as u32).max(1),
                        ((h as f32 * f) as u32).max(1),
                        image::imageops::FilterType::Triangle,
                    )
                } else {
                    img.clone()
                };
                let ci = egui::ColorImage::from_rgba_unmultiplied(
                    [small.width() as usize, small.height() as usize],
                    small.as_raw(),
                );
                ctx.load_texture("source", ci, egui::TextureOptions::LINEAR)
            });
    }

    fn set_image(&mut self, ctx: &egui::Context, path: &Path) {
        let Some(s) = &mut self.session else { return };
        match s.set_image(path) {
            Ok(()) => {
                self.source_mtime = mtime(&s.source_png_path());
                self.load_source_texture(ctx);
                self.set_status(format!("Picture set: {}", path.display()), false);
            }
            Err(e) => self.set_status(format!("{e:#}"), true),
        }
    }

    /// Use the picture on the clipboard as the source image.
    fn paste_image(&mut self, ctx: &egui::Context) {
        let Some(s) = &mut self.session else { return };
        match clip::read_image().and_then(|img| s.set_image_rgba(img).map_err(|e| format!("{e:#}")))
        {
            Ok(()) => {
                self.source_mtime = mtime(&s.source_png_path());
                self.load_source_texture(ctx);
                self.set_status("Picture pasted from the clipboard", false);
            }
            Err(e) => self.set_status(e, true),
        }
    }

    /// Keep step and selection in range after the model changed.
    fn clamp_state(&mut self) {
        let (n, steps) = self
            .model()
            .map(|m| (m.placements.len(), m.steps.len()))
            .unwrap_or((0, 0));
        if self.selected.is_some_and(|i| i >= n) {
            self.selected = None;
        }
        if steps == 0 {
            self.step_mode = false;
            self.step = 0;
        } else {
            self.step = self.step.min(steps - 1);
        }
        self.scene_dirty = true;
    }

    /// Check the session folder for changes written by another process.
    fn poll_files(&mut self, ctx: &egui::Context, now: f64, force: bool) {
        if !force && now - self.last_poll < 0.5 {
            return;
        }
        self.last_poll = now;
        let Some(s) = &mut self.session else { return };
        let m = mtime(&s.dir.join("model.json"));
        let src = mtime(&s.source_png_path());
        if m == self.model_mtime && src == self.source_mtime {
            return;
        }
        let before = s.model.as_ref().map(|m| m.placements.len());
        if let Err(e) = s.reload() {
            self.status = Some((format!("Reload failed: {e:#}"), true));
            return;
        }
        let model_changed = m != self.model_mtime;
        let src_changed = src != self.source_mtime;
        self.model_mtime = m;
        self.source_mtime = src;
        if src_changed {
            self.load_source_texture(ctx);
        }
        if model_changed {
            let after = self.model().map(|m| m.placements.len());
            self.undo.clear();
            self.fit_pending = before.is_none() || self.busy.is_some();
            self.clamp_state();
            if let Some(n) = after {
                if self.busy.is_some() {
                    self.chat.push(ChatLine {
                        kind: ChatKind::Progress,
                        text: format!("Model updated · {n} parts"),
                    });
                }
                self.set_status(format!("Loaded new build: {n} parts"), false);
            }
        }
    }

    fn handle_worker(&mut self, ctx: &egui::Context, now: f64) {
        while let Ok(msg) = self.worker.rx.try_recv() {
            match msg {
                worker::Msg::ClaudeCodeAvailable(ok) => {
                    self.cc_status = Some(ok);
                    if !ok && self.backend == Backend::ClaudeCode && self.api_status.is_ok() {
                        self.backend = Backend::Api;
                    }
                }
                worker::Msg::Event(e) => {
                    use sf_agent::claude_api::Event;
                    match e {
                        Event::Text(t) => self.chat.push(ChatLine {
                            kind: ChatKind::Claude,
                            text: t,
                        }),
                        Event::Building(t) => self.chat.push(ChatLine {
                            kind: ChatKind::Progress,
                            text: format!("Building “{t}”…"),
                        }),
                        Event::Built(r) => {
                            let first = r.lines().next().unwrap_or_default().to_string();
                            self.chat.push(ChatLine {
                                kind: ChatKind::Progress,
                                text: format!("Built: {first}"),
                            });
                            self.poll_files(ctx, now, true);
                        }
                        Event::BuildFailed(e) => self.chat.push(ChatLine {
                            kind: ChatKind::Error,
                            text: format!("Build failed: {e}"),
                        }),
                        Event::Done => {}
                    }
                }
                worker::Msg::Finished(r) => {
                    let backend = self.busy.map(|b| b.0);
                    self.busy = None;
                    match r {
                        Ok(text) => {
                            // The API backend already streamed its text.
                            if backend == Some(Backend::ClaudeCode) && !text.trim().is_empty() {
                                self.chat.push(ChatLine {
                                    kind: ChatKind::Claude,
                                    text,
                                });
                            }
                            self.set_status("Claude finished", false);
                        }
                        Err(e) => {
                            self.chat.push(ChatLine {
                                kind: ChatKind::Error,
                                text: e.clone(),
                            });
                            self.set_status(e, true);
                        }
                    }
                    self.poll_files(ctx, now, true);
                }
            }
        }
    }

    fn start_turn(&mut self, ctx: &egui::Context, prompt: String, fresh: bool, now: f64) {
        if let Err(e) = self.session_budget_sync() {
            self.set_status(e, true);
            return;
        }
        let Some(s) = &self.session else { return };
        let shown = if prompt.trim().is_empty() {
            "Design from the picture".to_string()
        } else {
            prompt.clone()
        };
        self.chat.push(ChatLine {
            kind: ChatKind::User,
            text: shown,
        });
        let req = worker::TurnRequest {
            backend: self.backend,
            dir: s.dir.clone(),
            prompt,
            fresh,
            effort: self.effort.clone(),
            resources: self.res.clone(),
        };
        self.busy = Some((self.backend, now));
        self.selected = None;
        self.tool = Tool::Select;
        self.worker.start_turn(req, ctx.clone());
        self.set_status(format!("Working with {}…", self.backend.label()), false);
    }

    /// Save the print-mode choice into the session (`print.txt`, read into every build).
    fn session_print_sync(&mut self) -> Result<(), String> {
        let print = self.print_mode;
        let Some(s) = &mut self.session else {
            return Ok(());
        };
        s.set_print(print).map_err(|e| format!("Saving build mode: {e:#}"))
    }

    /// Save the smooth-curves choice into the session (`curves.txt`, read into every build).
    fn session_curves_sync(&mut self) -> Result<(), String> {
        let curves = self.curves;
        let Some(s) = &mut self.session else {
            return Ok(());
        };
        s.set_curves(curves).map_err(|e| format!("Saving smooth curves: {e:#}"))
    }

    fn session_budget_sync(&mut self) -> Result<(), String> {
        let b = self.budget;
        let Some(s) = &mut self.session else {
            return Ok(());
        };
        if s.budget != Some(b) {
            s.set_budget(Some(b))
                .map_err(|e| format!("Saving budget: {e:#}"))?;
        }
        Ok(())
    }

    fn rebuild_scene(&mut self) {
        let vis = match self.model() {
            Some(m) => viewer::visibility(m, self.step_mode.then_some(self.step)),
            None => viewer::Visibility {
                visible: Vec::new(),
                flags: Vec::new(),
            },
        };
        self.vis = vis;
        self.generation += 1;
        let Some(s) = &self.session else { return };
        let ghost = if self.tool == Tool::Add {
            self.ghost.as_ref()
        } else {
            None
        };
        let inp = viewer::SceneInput {
            model: s.model.as_ref(),
            palette: &s.palette,
            meshes: &s.meshes,
            vis: &self.vis,
            selected: self.selected,
            ghost,
            accent: ACCENT,
            edges: self.edges,
        };
        self.scene = Arc::new(viewer::build_scene(self.generation, &inp));
        self.scene_dirty = false;
    }

    fn fit(&mut self) {
        let (lo, hi) = viewer::model_bounds(self.model());
        self.cam.fit(lo, hi, self.viewer_aspect);
    }

    fn handle_screenshot(&mut self, ctx: &egui::Context) {
        let Some(shot) = &mut self.shot else { return };
        shot.frames += 1;
        ctx.request_repaint();
        if !shot.requested && shot.frames >= 12 {
            shot.requested = true;
            let path = shot.path.clone();
            let done = shot.done.clone();
            let ctx2 = ctx.clone();
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(
                egui::ScreenshotCallback::new(move |img| {
                    let [w, h] = img.size;
                    let bytes: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
                    let r = image::RgbaImage::from_raw(w as u32, h as u32, bytes)
                        .ok_or_else(|| "bad screenshot buffer".to_string())
                        .and_then(|im| im.save(&path).map_err(|e| e.to_string()));
                    if let Ok(mut d) = done.lock() {
                        *d = Some(r);
                    }
                    ctx2.request_repaint();
                }),
            ));
        }
        let finished = shot.done.lock().ok().and_then(|d| d.clone());
        if let Some(r) = finished {
            match r {
                Ok(()) => eprintln!("screenshot saved: {}", shot.path.display()),
                Err(e) => eprintln!("screenshot failed: {e}"),
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        egui::Panel::top("top")
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(22, 24, 27))
                    .inner_margin(egui::Margin::symmetric(12, 6)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("fuSHion").strong().size(15.0).color(ACCENT));
                    ui.separator();
                    let mut open: Option<PathBuf> = None;
                    ui.menu_button("Session", |ui| {
                        if ui.button("New session").clicked() {
                            open = Some(new_session_dir());
                        }
                        if ui.button("Open folder…").clicked()
                            && let Some(d) = rfd::FileDialog::new()
                                .set_directory(sf_agent::session::default_sessions_dir())
                                .pick_folder()
                        {
                            open = Some(d);
                        }
                        ui.separator();
                        ui.label(RichText::new("Recent").small().weak());
                        let current = self.session.as_ref().map(|s| s.dir.clone());
                        for (p, t) in recent_sessions().into_iter().take(12) {
                            let name = p
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default();
                            let title = std::fs::read_to_string(p.join("model.json"))
                                .ok()
                                .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
                                .and_then(|v| v["title"].as_str().map(String::from))
                                .filter(|t| !t.is_empty());
                            let label = match title {
                                Some(title) => format!("{title}  ·  {name}  ·  {}", ago(t)),
                                None => format!("{name}  ·  {}", ago(t)),
                            };
                            let is_current = current.as_ref() == Some(&p);
                            if ui.selectable_label(is_current, label).clicked() {
                                open = Some(p);
                            }
                        }
                    });
                    if let Some(d) = open {
                        self.open_session(&ctx, &d);
                    }
                    if let Some(s) = &self.session {
                        let name = s
                            .dir
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        let title = s
                            .model
                            .as_ref()
                            .map(|m| m.title.clone())
                            .filter(|t| !t.is_empty());
                        ui.label(
                            RichText::new(title.unwrap_or_else(|| "Untitled".into())).strong(),
                        );
                        ui.label(RichText::new(name).weak().small())
                            .on_hover_text(s.dir.display().to_string());
                    }
                });
            });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("status")
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(22, 24, 27))
                    .inner_margin(egui::Margin::symmetric(12, 4)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    match &self.status {
                        Some((t, true)) => {
                            ui.label(RichText::new(t).color(ERROR));
                        }
                        Some((t, false)) => {
                            ui.label(RichText::new(t).weak());
                        }
                        None => {
                            ui.label(RichText::new("Ready").weak());
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.res.meshes.is_procedural() {
                            ui.label(
                                RichText::new(
                                    "Simple part shapes (run `fushion sync` for LDraw parts)",
                                )
                                .weak()
                                .small(),
                            );
                        }
                    });
                });
            });
    }
}

impl eframe::App for App {
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        visuals.panel_fill.to_normalized_gamma_f32()
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        self.handle_worker(&ctx, now);
        self.poll_files(&ctx, now, false);
        // Keep polling the session folder even when idle.
        ctx.request_repaint_after(std::time::Duration::from_millis(500));

        // Image dropped onto the window.
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        });
        if let Some(p) = dropped.first() {
            self.set_image(&ctx, p);
        }

        // Ctrl+V pastes a picture (unless a text box has the keyboard, where it pastes text).
        if !ctx.egui_wants_keyboard_input()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::V))
        {
            self.paste_image(&ctx);
        }

        self.handle_keys(&ctx);

        self.top_bar(ui);
        self.status_bar(ui);
        egui::Panel::left("design")
            .resizable(true)
            .default_size(320.0)
            .size_range(260.0..=480.0)
            .frame(
                egui::Frame::new()
                    .fill(ui.visuals().panel_fill)
                    .inner_margin(egui::Margin::same(12)),
            )
            .show(ui, |ui| self.design_panel(ui, now));
        egui::Panel::right("report")
            .resizable(true)
            .default_size(300.0)
            .size_range(250.0..=460.0)
            .frame(
                egui::Frame::new()
                    .fill(ui.visuals().panel_fill)
                    .inner_margin(egui::Margin::same(12)),
            )
            .show(ui, |ui| self.report_panel(ui));
        egui::CentralPanel::no_frame().show(ui, |ui| self.centre(ui, now));

        // Drop hint.
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
        if hovering {
            let screen = ctx.content_rect();
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::unique("drop"),
            ));
            painter.rect_filled(screen, 0.0, Color32::from_black_alpha(160));
            painter.text(
                screen.center(),
                egui::Align2::CENTER_CENTER,
                "Drop a picture to use it",
                egui::FontId::proportional(22.0),
                Color32::WHITE,
            );
        }

        self.handle_screenshot(&ctx);
    }
}
