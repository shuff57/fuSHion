//! The Claude Messages API backend (raw HTTP; Rust has no official SDK).
//!
//! Claude gets the source image and one tool, `build_model`, which compiles a
//! spec, packs it into parts, checks it, and returns the report plus a
//! six-view render. The loop runs until Claude ends its turn or the round
//! limit is reached. The whole conversation is kept in `conversation.json`
//! so chat edits continue where the last turn stopped. Assistant content is
//! appended back unchanged (thinking and fallback blocks included).

use crate::session::Session;
use anyhow::{Context, Result, bail};
use base64::Engine as _;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::Duration;

pub const DEFAULT_MODEL: &str = "claude-opus-5-5";
const API_URL: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2026-10-21";
/// Server-side refusal fallback (re-runs a declined request on a fallback
/// model inside the same call).
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";

#[derive(Clone, Debug)]
pub struct ClaudeConfig {
    pub api_key: String,
    pub model: String,
    /// `low` | `medium` | `high` | `xhigh` | `max`.
    pub effort: String,
    /// Maximum `build_model` calls per turn.
    pub max_builds: u32,
}

/// Where the API key may be stored (mode 600). `ANTHROPIC_API_KEY` wins.
pub fn api_key_path() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("studforge").join("anthropic.key")
}

impl ClaudeConfig {
    pub fn from_env() -> Result<Self> {
        let key = match std::env::var("ANTHROPIC_API_KEY") {
            Ok(k) if !k.trim().is_empty() => k,
            _ => std::fs::read_to_string(api_key_path())
                .map(|s| s.trim().to_string())
                .ok()
                .filter(|s| !s.is_empty())
                .context("no Anthropic API key: set ANTHROPIC_API_KEY or save one in the app's settings")?,
        };
        Ok(Self { api_key: key, model: DEFAULT_MODEL.into(), effort: "high".into(), max_builds: 5 })
    }
}

/// Progress events for the UI.
#[derive(Clone, Debug)]
pub enum Event {
    /// Claude's visible text.
    Text(String),
    /// A build started (spec title).
    Building(String),
    /// A build finished (report text).
    Built(String),
    /// A build failed (error text Claude will see).
    BuildFailed(String),
    /// The turn is over.
    Done,
}

pub fn system_prompt() -> String {
    format!(
        "You are studforge's designer. You turn a picture into a buildable model made of \
LEGO-compatible bricks. You work by writing a shape spec and calling `build_model`; the engine \
places every brick, checks that the model holds together, and shows you six rendered views \
(front, right, back, left, top, iso, in that order, left to right then top to bottom; reliefs show only top and iso).\n\n\
Compare each render with the source picture and fix the spec until the model clearly reads as \
the subject and the report shows no structure issues. Use at most {} builds per request, then \
reply with two or three plain sentences: what you built, what you changed last, and anything the \
user might want to adjust. When the user asks for a change, edit your previous spec rather than \
starting over.\n\n{}",
        5,
        sf_spec::GUIDE
    )
}

fn tools() -> Value {
    json!([{
        "name": "build_model",
        "description": "Compile a studforge shape spec into real parts, check the structure, and return the report plus a six-view render sheet (front, right, back, left, top, iso). Call it with the complete spec every time.",
        "input_schema": {
            "type": "object",
            "properties": {
                "spec": {"type": "object", "description": "A complete spec in the format the system prompt describes."}
            },
            "required": ["spec"]
        }
    }])
}

pub struct Conversation {
    pub messages: Vec<Value>,
    path: PathBuf,
}

impl Conversation {
    pub fn load(session: &Session) -> Self {
        let path = session.dir.join("conversation.json");
        let messages = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Self { messages, path }
    }
    pub fn save(&self) -> Result<()> {
        std::fs::write(&self.path, serde_json::to_string(&self.messages)?)?;
        Ok(())
    }
    pub fn clear(&mut self) -> Result<()> {
        self.messages.clear();
        self.save()
    }
}

fn post(cfg: &ClaudeConfig, body: &Value) -> Result<Value> {
    let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(900)).build()?;
    let mut delay = Duration::from_secs(2);
    for attempt in 0..4 {
        let resp = client
            .post(API_URL)
            .header("x-api-key", &cfg.api_key)
            .header("anthropic-version", API_VERSION)
            .header("anthropic-beta", FALLBACK_BETA)
            .header("content-type", "application/json")
            .json(body)
            .send();
        match resp {
            Ok(r) => {
                let status = r.status();
                let retry_after = r
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok());
                let should_retry_hdr = r.headers().get("x-should-retry").and_then(|v| v.to_str().ok()).map(|v| v == "true");
                let text = r.text().unwrap_or_default();
                if status.is_success() {
                    return serde_json::from_str(&text).context("API returned invalid JSON");
                }
                let retryable = should_retry_hdr.unwrap_or(status.as_u16() == 429 || status.as_u16() == 408 || status.is_server_error());
                if retryable && attempt < 3 {
                    std::thread::sleep(retry_after.map(Duration::from_secs).unwrap_or(delay));
                    delay *= 2;
                    continue;
                }
                let msg = serde_json::from_str::<Value>(&text)
                    .ok()
                    .and_then(|v| v["error"]["message"].as_str().map(String::from))
                    .unwrap_or(text);
                bail!("Claude API error {status}: {msg}");
            }
            Err(e) if attempt < 3 && (e.is_timeout() || e.is_connect()) => {
                std::thread::sleep(delay);
                delay *= 2;
            }
            Err(e) => return Err(e.into()),
        }
    }
    bail!("Claude API: out of retries")
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Run one user turn: the first turn sends the source image; later turns
/// are chat edits. Returns Claude's final text.
pub fn run_turn(
    session: &mut Session,
    cfg: &ClaudeConfig,
    convo: &mut Conversation,
    user_text: &str,
    on_event: &mut dyn FnMut(Event),
) -> Result<String> {
    let mut content = Vec::new();
    if convo.messages.is_empty() {
        let png = session.source_png(1568)?;
        content.push(json!({"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": b64(&png)}}));
        let budget = session.budget.unwrap_or(sf_engine::DEFAULT_BUDGET);
        content.push(json!({"type": "text", "text": format!(
            "Build this picture as a brick model. Part budget: {budget}. {}",
            if user_text.trim().is_empty() { "Pick the mode that suits the picture.".to_string() } else { user_text.to_string() }
        )}));
    } else {
        let mut t = user_text.to_string();
        if let Some(b) = session.budget {
            t.push_str(&format!("\n(Part budget: {b}.)"));
        }
        content.push(json!({"type": "text", "text": t}));
    }
    // A previous turn may have stopped right after tool results; merge into
    // that user message so roles keep alternating.
    match convo.messages.last_mut() {
        Some(last) if last["role"] == "user" => {
            if let Some(arr) = last["content"].as_array_mut() {
                arr.extend(content);
            }
        }
        _ => convo.messages.push(json!({"role": "user", "content": content})),
    }
    convo.save()?;

    let mut builds = 0u32;
    let mut final_text = String::new();
    for _ in 0..(cfg.max_builds + 3) {
        let body = json!({
            "model": cfg.model,
            "max_tokens": 16000,
            "system": [{"type": "text", "text": system_prompt(), "cache_control": {"type": "ephemeral"}}],
            "tools": tools(),
            "thinking": {"type": "adaptive"},
            "output_config": {"effort": cfg.effort},
            "fallbacks": "default",
            "messages": convo.messages,
        });
        let resp = post(cfg, &body)?;
        let stop = resp["stop_reason"].as_str().unwrap_or("").to_string();
        let blocks = resp["content"].as_array().cloned().unwrap_or_default();
        convo.messages.push(json!({"role": "assistant", "content": blocks}));
        convo.save()?;
        for b in &blocks {
            if b["type"] == "text"
                && let Some(t) = b["text"].as_str()
                && !t.trim().is_empty()
            {
                final_text = t.to_string();
                on_event(Event::Text(t.to_string()));
            }
        }
        match stop.as_str() {
            "tool_use" => {
                let mut results = Vec::new();
                for b in blocks.iter().filter(|b| b["type"] == "tool_use") {
                    let id = b["id"].as_str().unwrap_or_default();
                    if b["name"] != "build_model" {
                        results.push(json!({"type": "tool_result", "tool_use_id": id, "is_error": true,
                            "content": format!("unknown tool {}", b["name"])}));
                        continue;
                    }
                    if builds >= cfg.max_builds {
                        results.push(json!({"type": "tool_result", "tool_use_id": id, "is_error": true,
                            "content": "Build limit for this request reached. Do not build again; summarise for the user now."}));
                        continue;
                    }
                    builds += 1;
                    let spec = &b["input"]["spec"];
                    let title = spec["title"].as_str().unwrap_or("model").to_string();
                    on_event(Event::Building(title));
                    match session.build_spec(&spec.to_string()) {
                        Ok(out) => {
                            on_event(Event::Built(out.report_text.clone()));
                            results.push(json!({"type": "tool_result", "tool_use_id": id, "content": [
                                {"type": "text", "text": format!("Build {builds} of at most {}.\n{}", cfg.max_builds, out.report_text)},
                                {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": b64(&out.sheet_png)}}
                            ]}));
                        }
                        Err(e) => {
                            let msg = format!("{e:#}");
                            on_event(Event::BuildFailed(msg.clone()));
                            results.push(json!({"type": "tool_result", "tool_use_id": id, "is_error": true, "content": msg}));
                        }
                    }
                }
                convo.messages.push(json!({"role": "user", "content": results}));
                convo.save()?;
            }
            "end_turn" | "stop_sequence" => break,
            "max_tokens" => {
                final_text.push_str("\n(Claude's reply was cut off at the token limit.)");
                break;
            }
            "refusal" => {
                let why = resp["stop_details"]["explanation"].as_str().unwrap_or("no reason given");
                bail!("Claude declined this request: {why}");
            }
            other => bail!("unexpected stop reason `{other}`"),
        }
    }
    on_event(Event::Done);
    Ok(final_text)
}
