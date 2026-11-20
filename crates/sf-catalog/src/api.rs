//! Minimal, rate-limited Rebrickable API client.
//!
//! The key comes from `REBRICKABLE_API_KEY` or
//! `dirs::config_dir()/studforge/rebrickable.key`. It is only ever placed in
//! the `Authorization` header and never appears in errors or logs.

use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::time::{Duration, Instant};

const BASE: &str = "https://rebrickable.com/api/v3/lego";
const MIN_GAP: Duration = Duration::from_millis(1100);
const MAX_RETRIES: u32 = 5;

pub(crate) struct Api {
    client: reqwest::blocking::Client,
    key: String,
    last: Option<Instant>,
}

/// Read the API key without exposing it.
fn load_key() -> Result<String> {
    if let Ok(k) = std::env::var("REBRICKABLE_API_KEY") {
        let k = k.trim().to_string();
        if !k.is_empty() {
            return Ok(k);
        }
    }
    let path = dirs::config_dir().context("no config directory")?.join("studforge").join("rebrickable.key");
    let k = std::fs::read_to_string(&path)
        .with_context(|| format!("no Rebrickable API key: set REBRICKABLE_API_KEY or create {}", path.display()))?;
    let k = k.trim().to_string();
    if k.is_empty() {
        bail!("Rebrickable API key file {} is empty", path.display());
    }
    Ok(k)
}

impl Api {
    pub fn new() -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .user_agent(concat!("studforge/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self { client, key: load_key()?, last: None })
    }

    /// GET `BASE + path_and_query` as JSON, keeping ≥ 1.1 s between calls and
    /// backing off on HTTP 429.
    pub fn get(&mut self, path_and_query: &str) -> Result<Value> {
        let url = format!("{BASE}{path_and_query}");
        let mut backoff = Duration::from_secs(2);
        for attempt in 0..=MAX_RETRIES {
            if let Some(last) = self.last {
                let since = last.elapsed();
                if since < MIN_GAP {
                    std::thread::sleep(MIN_GAP - since);
                }
            }
            self.last = Some(Instant::now());
            let resp = self
                .client
                .get(&url)
                .header("Authorization", format!("key {}", self.key))
                .header("Accept", "application/json")
                .send()
                .map_err(|e| anyhow::anyhow!("request to {url} failed: {}", e.without_url()))?;
            let status = resp.status();
            if status.as_u16() == 429 && attempt < MAX_RETRIES {
                let wait = resp
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.trim().parse::<u64>().ok())
                    .map(Duration::from_secs)
                    .unwrap_or(backoff)
                    .min(Duration::from_secs(60));
                std::thread::sleep(wait);
                backoff *= 2;
                continue;
            }
            if !status.is_success() {
                bail!("{url}: HTTP {status}");
            }
            return resp.json::<Value>().with_context(|| format!("{url}: invalid JSON"));
        }
        bail!("{url}: still rate limited after {MAX_RETRIES} retries")
    }

    /// Follow `next` links and collect every `results[]` item.
    pub fn get_all(&mut self, path_and_query: &str) -> Result<Vec<Value>> {
        let mut out = Vec::new();
        let mut next = Some(path_and_query.to_string());
        while let Some(pq) = next.take() {
            let v = self.get(&pq)?;
            if let Some(r) = v.get("results").and_then(Value::as_array) {
                out.extend(r.iter().cloned());
            }
            next = v
                .get("next")
                .and_then(Value::as_str)
                .and_then(|n| n.strip_prefix(BASE))
                .map(str::to_string);
        }
        Ok(out)
    }
}

/// `external_ids.<system>.ext_ids` (numbers or strings) as strings.
pub(crate) fn ext_ids(item: &Value, system: &str) -> Vec<String> {
    item.get("external_ids")
        .and_then(|e| e.get(system))
        .and_then(|e| {
            // Colours: {"ext_ids": [...]}; parts: a plain array.
            e.get("ext_ids").and_then(Value::as_array).or_else(|| e.as_array())
        })
        .map(|a| {
            a.iter()
                .filter_map(|v| match v {
                    Value::String(s) => Some(s.clone()),
                    Value::Number(n) => Some(n.to_string()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}
