//! The Claude Code backend: runs the user's own `claude` CLI (their
//! subscription) headless, with studforge's MCP server attached to the same
//! session directory. The MCP server writes every build into the session
//! directory, which the app picks up.

use crate::session::Session;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Is the `claude` CLI on PATH?
pub fn available() -> bool {
    Command::new("claude").arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}

fn write_mcp_config(session: &Session, studforge_exe: &Path) -> Result<PathBuf> {
    let cfg = json!({
        "mcpServers": {
            "studforge": {
                "command": studforge_exe,
                "args": ["mcp", "--session", session.dir],
            }
        }
    });
    let path = session.dir.join("mcp.json");
    std::fs::write(&path, serde_json::to_string_pretty(&cfg)?)?;
    Ok(path)
}

/// Run one turn through Claude Code. Returns Claude's final text.
pub fn run_turn(session: &Session, studforge_exe: &Path, user_text: &str) -> Result<String> {
    let id_path = session.dir.join("claude_code_session.txt");
    let resume = std::fs::read_to_string(&id_path).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let budget = session.budget.unwrap_or(sf_engine::DEFAULT_BUDGET);
    let prompt = if resume.is_none() {
        format!(
            "Use the studforge tools to build the picture at {} as a brick model (part budget {budget}). \
             First call get_guide and view_source_image, then build with build_model and iterate on the \
             renders, at most 5 builds. Finish with two or three plain sentences for the user. {}",
            session.source_png_path().display(),
            user_text
        )
    } else {
        format!("{user_text}\n(Part budget: {budget}. Edit the previous spec with build_model; at most 5 builds.)")
    };
    let mcp = write_mcp_config(session, studforge_exe)?;
    let mut cmd = Command::new("claude");
    cmd.arg("-p")
        .arg("--output-format")
        .arg("json")
        .arg("--mcp-config")
        .arg(&mcp)
        .arg("--strict-mcp-config")
        .arg("--allowedTools")
        .arg("mcp__studforge__get_guide mcp__studforge__view_source_image mcp__studforge__build_model mcp__studforge__get_state")
        // studforge only needs its own tools; keep Claude out of the shell and files.
        .arg("--disallowedTools")
        .arg("Bash Edit Write NotebookEdit WebFetch WebSearch")
        .current_dir(&session.dir);
    if let Some(id) = &resume {
        cmd.arg("--resume").arg(id);
    }
    // `--` ends the options, so a prompt starting with `-` stays a prompt.
    cmd.arg("--").arg(&prompt);
    let out = cmd.output().context("running `claude` (is Claude Code installed and logged in?)")?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: Result<Value> = serde_json::from_str(stdout.trim()).with_context(|| {
        format!("Claude Code returned unexpected output: {}{}", stdout.chars().take(400).collect::<String>(), String::from_utf8_lossy(&out.stderr))
    });
    let v = match parsed {
        Ok(v) => v,
        Err(e) => {
            // A stale or broken resume id would fail every later turn the same way.
            if resume.is_some() {
                let _ = std::fs::remove_file(&id_path);
            }
            return Err(e);
        }
    };
    if let Some(id) = v["session_id"].as_str() {
        std::fs::write(&id_path, id)?;
    }
    if v["is_error"].as_bool() == Some(true) {
        bail!("Claude Code reported an error: {}", v["result"].as_str().unwrap_or("unknown"));
    }
    Ok(v["result"].as_str().unwrap_or_default().to_string())
}

/// Forget the Claude Code conversation (start fresh next turn).
pub fn reset(session: &Session) {
    let _ = std::fs::remove_file(session.dir.join("claude_code_session.txt"));
}
