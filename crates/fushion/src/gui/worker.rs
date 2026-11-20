//! Background work: Claude turns run on worker threads and report back over a
//! channel. The worker opens its own `Session` on the same folder (sharing the
//! loaded catalog and meshes); builds land on disk, and the app's live sync
//! picks them up.

use sf_agent::Session;
use sf_agent::claude_api::{self, ClaudeConfig, Conversation, Event};
use sf_agent::claude_code;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Backend {
    Api,
    ClaudeCode,
}

impl Backend {
    pub fn label(self) -> &'static str {
        match self {
            Backend::Api => "Claude API (your key)",
            Backend::ClaudeCode => "Claude Code (your subscription)",
        }
    }
}

pub enum Msg {
    Event(Event),
    /// The turn finished: Claude's final text, or an error.
    Finished(Result<String, String>),
    /// `claude_code::available()` result.
    ClaudeCodeAvailable(bool),
}

pub struct TurnRequest {
    pub backend: Backend,
    pub dir: PathBuf,
    pub prompt: String,
    /// Start a fresh conversation (the Design button).
    pub fresh: bool,
    pub effort: String,
    pub resources: Resources,
}

/// Shared, already-loaded resources so a worker session opens instantly.
#[derive(Clone)]
pub struct Resources {
    pub palette: sf_core::Palette,
    pub availability: std::sync::Arc<dyn sf_core::Availability>,
    pub catalog: Option<std::sync::Arc<sf_catalog::CatalogIndex>>,
    pub meshes: std::sync::Arc<sf_ldraw::MeshLibrary>,
}

impl Resources {
    pub fn load() -> Self {
        let (palette, availability, catalog, meshes) = sf_agent::session::load_resources();
        Self {
            palette,
            availability,
            catalog,
            meshes,
        }
    }

    pub fn open(&self, dir: &std::path::Path) -> anyhow::Result<Session> {
        Session::open_with(
            dir,
            self.palette.clone(),
            self.availability.clone(),
            self.catalog.clone(),
            self.meshes.clone(),
        )
    }
}

pub struct Worker {
    pub tx: Sender<Msg>,
    pub rx: Receiver<Msg>,
}

impl Worker {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        Self { tx, rx }
    }

    pub fn probe_claude_code(&self, ctx: egui::Context) {
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let ok = claude_code::available();
            let _ = tx.send(Msg::ClaudeCodeAvailable(ok));
            ctx.request_repaint();
        });
    }

    pub fn start_turn(&self, req: TurnRequest, ctx: egui::Context) {
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = run(req, &tx, &ctx).map_err(|e| format!("{e:#}"));
            let _ = tx.send(Msg::Finished(result));
            ctx.request_repaint();
        });
    }
}

fn run(req: TurnRequest, tx: &Sender<Msg>, ctx: &egui::Context) -> anyhow::Result<String> {
    let mut session = req.resources.open(&req.dir)?;
    match req.backend {
        Backend::Api => {
            let mut cfg = ClaudeConfig::from_env()?;
            cfg.effort = req.effort;
            let mut convo = Conversation::load(&session);
            if req.fresh {
                convo.clear()?;
            }
            let mut on_event = |e: Event| {
                let _ = tx.send(Msg::Event(e));
                ctx.request_repaint();
            };
            claude_api::run_turn(&mut session, &cfg, &mut convo, &req.prompt, &mut on_event)
        }
        Backend::ClaudeCode => {
            if req.fresh {
                claude_code::reset(&session);
            }
            let exe = std::env::current_exe()?;
            claude_code::run_turn(&session, &exe, &req.prompt)
        }
    }
}

/// Rebuild a readable transcript from `conversation.json` (API backend).
pub fn transcript_from_conversation(session: &Session) -> Vec<(bool, String)> {
    let convo = Conversation::load(session);
    let mut out = Vec::new();
    for m in &convo.messages {
        let is_user = m["role"] == "user";
        let Some(blocks) = m["content"].as_array() else {
            continue;
        };
        for b in blocks {
            if b["type"] == "text"
                && let Some(t) = b["text"].as_str()
            {
                let t = t.trim();
                if t.is_empty() {
                    continue;
                }
                // Hide the generated first-turn instruction; keep what the user typed.
                let shown = if is_user && t.starts_with("Build this picture as a brick model.") {
                    "Design from the picture".to_string()
                } else {
                    t.lines()
                        .filter(|l| !l.starts_with("(Part budget:"))
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                if !shown.trim().is_empty() {
                    out.push((is_user, shown));
                }
            }
        }
    }
    out
}
