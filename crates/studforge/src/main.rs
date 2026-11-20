//! studforge: turn a picture into a buildable brick model.
//!
//! With no subcommand, opens the desktop app.

mod gui;
mod mcp;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use sf_agent::{Session, claude_api, claude_code};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "studforge", version, about = "Turn a picture into a buildable brick model")]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Copy, Clone, ValueEnum)]
enum Backend {
    /// Anthropic API with your own key (ANTHROPIC_API_KEY).
    Api,
    /// Your Claude Code CLI (your Claude subscription).
    ClaudeCode,
}

#[derive(Subcommand)]
enum Cmd {
    /// Open the desktop app (default).
    Gui {
        #[arg(long)]
        session: Option<PathBuf>,
        /// Render a few frames, save a PNG screenshot here, and exit.
        #[arg(long, hide = true)]
        screenshot: Option<PathBuf>,
    },
    /// Let Claude design a model from a picture, headless.
    Design {
        image: PathBuf,
        #[arg(long)]
        budget: Option<u32>,
        /// Extra instructions, e.g. "make it a relief".
        #[arg(long, default_value = "")]
        prompt: String,
        #[arg(long, value_enum, default_value_t = Backend::Api)]
        backend: Backend,
        #[arg(long)]
        session: Option<PathBuf>,
    },
    /// Build a spec file and render it (no Claude).
    Build {
        spec: PathBuf,
        #[arg(long)]
        image: Option<PathBuf>,
        #[arg(long)]
        budget: Option<u32>,
        #[arg(long)]
        session: Option<PathBuf>,
    },
    /// Write the LDraw model, parts CSV and BrickLink list for a session.
    Export {
        #[arg(long)]
        session: PathBuf,
        out: PathBuf,
    },
    /// Download/refresh the LDraw parts and the Rebrickable catalog.
    Sync {
        /// Skip Rebrickable API lookups (BrickLink ids).
        #[arg(long)]
        offline: bool,
    },
    /// Run the MCP server on stdio (for Claude Code and other MCP clients).
    Mcp {
        #[arg(long)]
        session: Option<PathBuf>,
    },
    /// Make a custom 3D-printable part (STL + 3MF), e.g.
    /// `studforge custom '{"kind":"slope45","w":2,"d":3,"plates":3}' --out parts`.
    Custom {
        /// The part as JSON: kind box | slope45 | curve | footprint (see `studforge guide`).
        shape: String,
        #[arg(long, default_value = ".")]
        out: PathBuf,
        #[arg(long, default_value = "custom-part")]
        name: String,
        /// Use the experimental compliant stud.
        #[arg(long)]
        compliant_stud: bool,
    },
    /// Print the spec guide.
    Guide,
}

fn new_session_dir() -> PathBuf {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    sf_agent::session::default_sessions_dir().join(format!("s{stamp}"))
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd.unwrap_or(Cmd::Gui { session: None, screenshot: None }) {
        Cmd::Gui { session, screenshot } => gui::run_with(gui::GuiOptions { session, screenshot }),
        Cmd::Custom { shape, out, name, compliant_stud } => {
            let shape: serde_json::Value = serde_json::from_str(&shape).context("the shape must be JSON")?;
            let made = sf_agent::custom::make_part(&shape, compliant_stud, &out, &name)?;
            println!("{} ({:.0} mm³)\n{}", made.stl.display(), made.volume_mm3, made.three_mf.display());
            Ok(())
        }
        Cmd::Guide => {
            print!("{}", sf_spec::GUIDE);
            Ok(())
        }
        Cmd::Mcp { session } => {
            let dir = session.unwrap_or_else(new_session_dir);
            let s = Session::open(&dir)?;
            mcp::serve(s)
        }
        Cmd::Build { spec, image, budget, session } => {
            let dir = session.unwrap_or_else(new_session_dir);
            let mut s = Session::open(&dir)?;
            if let Some(img) = image {
                s.set_image(&img)?;
            }
            if budget.is_some() {
                s.set_budget(budget)?;
            }
            let text = std::fs::read_to_string(&spec).with_context(|| format!("reading {}", spec.display()))?;
            let out = s.build_spec(&text)?;
            println!("{}", out.report_text);
            println!("Session: {}", dir.display());
            println!("Render: {}", dir.join("render.png").display());
            Ok(())
        }
        Cmd::Design { image, budget, prompt, backend, session } => {
            let dir = session.unwrap_or_else(new_session_dir);
            let mut s = Session::open(&dir)?;
            s.set_image(&image)?;
            s.set_budget(budget)?;
            let text = match backend {
                Backend::Api => {
                    let cfg = claude_api::ClaudeConfig::from_env()?;
                    let mut convo = claude_api::Conversation::load(&s);
                    claude_api::run_turn(&mut s, &cfg, &mut convo, &prompt, &mut |e| match e {
                        claude_api::Event::Building(t) => eprintln!("building {t}..."),
                        claude_api::Event::Built(r) => eprintln!("{r}"),
                        claude_api::Event::BuildFailed(e) => eprintln!("build failed: {e}"),
                        _ => {}
                    })?
                }
                Backend::ClaudeCode => {
                    let exe = std::env::current_exe()?;
                    claude_code::run_turn(&s, &exe, &prompt)?
                }
            };
            println!("{text}");
            println!("Session: {}", dir.display());
            Ok(())
        }
        Cmd::Export { session, out } => {
            let s = Session::open(&session)?;
            for f in s.export(&out)? {
                println!("{}", f.display());
            }
            Ok(())
        }
        Cmd::Sync { offline } => {
            let dir = sf_ldraw::ensure_installed()?;
            println!("LDraw parts: {}", dir.display());
            let downloads = sf_catalog::default_downloads_dir().context("no cache dir")?;
            let mut opts = sf_catalog::SyncOptions::new(downloads, !offline);
            opts.download_missing = true;
            opts.previous = sf_catalog::CatalogIndex::load_default().ok().flatten();
            let idx = sf_catalog::CatalogIndex::sync(opts)?;
            let path = idx.save_default()?;
            println!("Catalog synced: {}", path.display());
            Ok(())
        }
    }
}
