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
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("fushion").join("anthropic.key")
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
    system_prompt_with(5)
}

pub fn system_prompt_with(max_builds: u32) -> String {
    format!(
        "You are fuSHion's designer. You turn a picture into a buildable model made of \
LEGO-compatible bricks. You work by writing a shape spec and calling `build_model`; the engine \
places every brick, checks that the model holds together, and shows you six rendered views \
(front, right, back, left, top, iso, in that order, left to right then top to bottom; reliefs show only top and iso).\n\n\
Compare each render with the source picture and fix the spec until the model clearly reads as \
the subject and the report shows no structure issues.\n\nBricks alone can't make a petal, a flower, a leaf, an eye or a beak. For every feature \
like that, design a printed detail part with `design_detail` (a template such as rose, daisy, aster, gerbera, \
poppy, cornflower, spike, umbel, fern, leaf, petal, stem, dome, cone or round_plate, or your own SolidCraft \
commands), look at the picture it returns, adjust it until it reads as the feature, then place it with the \
spec's `details`. For flowers, follow LEGO's Botanical bouquets (10280 Flower Bouquet, 10313 Wildflower \
Bouquet): each head is a hub with petals around it, often two staggered layers and a separate centre; spike \
flowers (snapdragon, lupin, lavender, larkspur) stack rings of florets up a stem; foliage is separate leaves \
and fronds. Match the picture's species, petal count, colours and size; replicate the picture as closely \
as you can. A cut flower or a bouquet goes on printed stems with the spec's `bouquet` (see the guide's \
\"Bouquets\" section): build the vase from body shapes with a flat studded top, give every flower its own \
stem (each design suggests its own stem style, like LEGO's: a forked daisy sprig, brown lavender sprigs, a \
snapdragon stalk with buds, a grass fan), lean the stems out in a fan, size heads to the bouquet (about 6-8 \
studs across in a bouquet 30 studs wide), and never build stems or flowers on brick columns.\n\n\
Detail templates and their parameters (mm, degrees; base in studs):\n{}\n\n\
Use at most {} builds per request, then \
reply with two or three plain sentences: what you built, what you changed last, and anything the \
user might want to adjust. When the user asks for a change, edit your previous spec rather than \
starting over.\n\n{}",
        crate::details::templates_text(),
        max_builds,
        sf_spec::GUIDE
    )
}

fn tools() -> Value {
    json!([{
        "name": "design_detail",
        "description": "Design a printed detail part with SolidCraft (a flower head, petal, leaf, fern, stem, eye dome, beak or horn, round plate, or your own SolidCraft commands). It is checked, saved under `name`, and returned with a picture from four sides (iso, front, right, top). Place it with the spec's `details`.",
        "input_schema": {
            "type": "object",
            "properties": {
                "name": {"type": "string", "description": "Design name (letters, digits, _), used by the spec's `details`."},
                "label": {"type": "string", "description": "Short human name, e.g. \"red rose\"."},
                "design": {"type": "object", "description": "{\"template\": name, \"params\": {...}} or {\"commands\": [...], \"cells\": [[0,0],[1,0]], \"plates\": n, \"sockets\": [[0,0],[1,0]]} in the detail frame (column (i,j) spans x 8i..8i+8, y 8j..8j+8 mm, z up from 0; the socket base is built for you, top at z = 3.2 mm)."},
                "color": {"type": "string", "description": "Colour for the picture."}
            },
            "required": ["name", "design"]
        }
    }, {
        "name": "build_model",
        "description": "Compile a fuSHion shape spec into real parts, check the structure, and return the report plus a six-view render sheet (front, right, back, left, top, iso). Call it with the complete spec every time.",
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

/// Start a fresh conversation once this many renders are in it, carrying the
/// current spec over, so requests stay small.
const MAX_IMAGES: usize = 16;
/// Detail designs Claude may make per request.
const MAX_DESIGNS: u32 = 12;

impl Conversation {
    /// Load the saved conversation. A corrupt file is moved to
    /// `conversation.json.bak` and the conversation starts fresh.
    pub fn load(session: &Session) -> Self {
        let path = session.dir.join("conversation.json");
        let messages = match std::fs::read_to_string(&path) {
            Ok(t) => match serde_json::from_str(&t) {
                Ok(m) => m,
                Err(_) => {
                    let _ = std::fs::rename(&path, path.with_extension("json.bak"));
                    Vec::new()
                }
            },
            Err(_) => Vec::new(),
        };
        Self { messages, path }
    }
    pub fn save(&self) -> Result<()> {
        crate::session::write_atomic(&self.path, &serde_json::to_string(&self.messages)?)
    }
    pub fn clear(&mut self) -> Result<()> {
        self.messages.clear();
        self.save()
    }
    fn image_count(&self) -> usize {
        fn count(v: &Value) -> usize {
            match v {
                Value::Object(o) => usize::from(o.get("type").and_then(Value::as_str) == Some("image")) + o.values().map(count).sum::<usize>(),
                Value::Array(a) => a.iter().map(count).sum(),
                _ => 0,
            }
        }
        self.messages.iter().map(count).sum()
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
/// are chat edits. Returns Claude's final text. Always ends with
/// [`Event::Done`], also on errors.
pub fn run_turn(
    session: &mut Session,
    cfg: &ClaudeConfig,
    convo: &mut Conversation,
    user_text: &str,
    on_event: &mut dyn FnMut(Event),
) -> Result<String> {
    let r = run_turn_inner(session, cfg, convo, user_text, on_event);
    on_event(Event::Done);
    r
}

fn first_message(session: &Session, user_text: &str, carry_spec: Option<String>) -> Result<Vec<Value>> {
    let mut content = Vec::new();
    let png = session.source_png(1568)?;
    content.push(json!({"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": b64(&png)}}));
    let budget = session.budget.unwrap_or(sf_engine::DEFAULT_BUDGET);
    let mut text = format!(
        "Build this picture as a brick model. Part budget: {budget}. {}",
        if user_text.trim().is_empty() { "Pick the mode that suits the picture." } else { user_text }
    );
    if let Some(spec) = carry_spec {
        text = format!(
            "We are continuing an earlier design of this picture (part budget {budget}). The current spec is below; \
             build it first, then apply this request: {}\n\nCurrent spec:\n{spec}",
            if user_text.trim().is_empty() { "keep improving it." } else { user_text }
        );
    }
    content.push(json!({"type": "text", "text": text}));
    Ok(content)
}

fn run_turn_inner(
    session: &mut Session,
    cfg: &ClaudeConfig,
    convo: &mut Conversation,
    user_text: &str,
    on_event: &mut dyn FnMut(Event),
) -> Result<String> {
    // Long conversations: start over with the current spec carried along.
    if convo.image_count() > MAX_IMAGES {
        let spec = session.spec.as_ref().map(serde_json::to_string_pretty).transpose()?;
        convo.messages.clear();
        let content = first_message(session, user_text, spec)?;
        convo.messages.push(json!({"role": "user", "content": content}));
    } else if convo.messages.is_empty() {
        let content = first_message(session, user_text, None)?;
        convo.messages.push(json!({"role": "user", "content": content}));
    } else {
        let mut t = user_text.to_string();
        if let Some(b) = session.budget {
            t.push_str(&format!("\n(Part budget: {b}.)"));
        }
        let content = vec![json!({"type": "text", "text": t})];
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
    }
    convo.save()?;

    let mut builds = 0u32;
    let mut designs = 0u32;
    let mut final_text = String::new();
    for _ in 0..(cfg.max_builds + MAX_DESIGNS + 3) {
        let body = json!({
            "model": cfg.model,
            "max_tokens": 16000,
            "system": [{"type": "text", "text": system_prompt_with(cfg.max_builds), "cache_control": {"type": "ephemeral"}}],
            "cache_control": {"type": "ephemeral"},
            "tools": tools(),
            "thinking": {"type": "adaptive"},
            "output_config": {"effort": cfg.effort},
            "fallbacks": "default",
            "messages": convo.messages,
        });
        let resp = post(cfg, &body)?;
        let stop = resp["stop_reason"].as_str().unwrap_or("").to_string();
        let blocks = resp["content"].as_array().cloned().unwrap_or_default();
        if stop == "refusal" {
            // Nothing from a declined turn goes into the saved conversation.
            let why = resp["stop_details"]["explanation"].as_str().unwrap_or("no reason given");
            bail!("Claude declined this request: {why}");
        }
        if blocks.is_empty() {
            bail!("Claude returned an empty reply (stop reason `{stop}`); try again");
        }
        convo.messages.push(json!({"role": "assistant", "content": blocks}));
        for b in &blocks {
            if b["type"] == "text"
                && let Some(t) = b["text"].as_str()
                && !t.trim().is_empty()
            {
                final_text = t.to_string();
                on_event(Event::Text(t.to_string()));
            }
        }
        let tool_uses: Vec<&Value> = blocks.iter().filter(|b| b["type"] == "tool_use").collect();
        if stop != "tool_use" {
            // A reply cut short can still end in a tool_use block; answer it so
            // the next request is valid.
            if !tool_uses.is_empty() {
                let results: Vec<Value> = tool_uses
                    .iter()
                    .map(|b| json!({"type": "tool_result", "tool_use_id": b["id"], "is_error": true,
                        "content": "This call was cut off and did not run."}))
                    .collect();
                convo.messages.push(json!({"role": "user", "content": results}));
            }
            convo.save()?;
            match stop.as_str() {
                "end_turn" | "stop_sequence" => {}
                "max_tokens" => final_text.push_str("\n(Claude's reply was cut off at the token limit.)"),
                "model_context_window_exceeded" => {
                    final_text.push_str("\n(The conversation is too long; it will restart with the current spec next time.)")
                }
                "pause_turn" => continue,
                other => final_text.push_str(&format!("\n(Stopped: {other}.)")),
            }
            break;
        }
        let mut results = Vec::new();
        for b in tool_uses {
            let id = b["id"].as_str().unwrap_or_default();
            if b["name"] == "design_detail" {
                if designs >= MAX_DESIGNS {
                    results.push(json!({"type": "tool_result", "tool_use_id": id, "is_error": true,
                        "content": "Detail design limit for this request reached; use the designs you have."}));
                    continue;
                }
                designs += 1;
                let input = &b["input"];
                let name = input["name"].as_str().unwrap_or_default();
                on_event(Event::Text(format!("Designing detail {name}…")));
                match session.design_detail(&input["design"], name, input["label"].as_str().unwrap_or(""), input["color"].as_str().unwrap_or("")) {
                    Ok((d, png)) => results.push(json!({"type": "tool_result", "tool_use_id": id, "content": [
                        {"type": "text", "text": format!("Saved design {}", crate::details::summary(&d))},
                        {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": b64(&png)}}
                    ]})),
                    Err(e) => results.push(json!({"type": "tool_result", "tool_use_id": id, "is_error": true, "content": format!("{e:#}")})),
                }
                continue;
            }
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
    Ok(final_text)
}
