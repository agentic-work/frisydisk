//! Asks a model running on this machine for storage and I/O suggestions.
//! The advisor only ever produces text; nothing it suggests is run.

use crate::settings::Settings;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Message {
    pub role: String,
    pub text: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Backend {
    /// "ollama", "agenticwork", "anthropic" or "openai"
    pub kind: String,
    pub model: String,
    pub label: String,
    /// The scan summary leaves this computer when this backend is used.
    #[serde(default)]
    pub remote: bool,
}

pub fn system_prompt() -> String {
    let os = match std::env::consts::OS {
        "macos" => "macOS",
        "windows" => "Windows",
        other => other,
    };
    format!(
        "You are a storage and I/O advisor built into FrisyDisk, a disk-usage app. This machine runs {os}. \
You are given facts collected from this one machine: its volumes (including any NAS or network shares), \
its system disk, and a disk-usage scan.\n\n\
Rules:\n\
- Suggest NON-DESTRUCTIVE improvements only. Never recommend deleting, erasing, reformatting or \
overwriting data. Prefer moving, archiving to the NAS, relocating caches or model stores, links, \
compression, mount and SMB tuning, snapshot and backup settings, and scheduling.\n\
- No step may contain rm, del, Remove-Item, \"delete\", \"clear\", \"purge\", \"empty\" or \"erase\" as an action, \
not even for caches, logs or build output. If something looks disposable, suggest moving it to the NAS \
instead, where it can be brought back.\n\
- Every suggestion must be reversible, and you must say how to undo it.\n\
- Copy first, verify the copy, and only then switch over (for example with a link). Say so in the steps.\n\
- Ground every suggestion in the facts given. Name the actual folders, shares and sizes. \
If a fact you need is missing, say what to check instead of guessing.\n\
- You cannot run anything. Show commands for the person to review and run themselves, written for {os}.\n\n\
Answer in Markdown. Give 5 to 8 suggestions ranked by benefit. For each: a short title, what to do, \
why it helps (space freed on the system disk or I/O gained), how (concrete steps or commands), and \
risk plus how to undo. Finish with one line naming the single best first step."
    )
}

pub fn user_prompt(machine: &str, scan: Option<&str>) -> String {
    let mut s = format!("Here are the facts about this machine.\n\n{machine}");
    if let Some(scan) = scan {
        s.push_str("\n\n");
        s.push_str(scan);
    }
    s.push_str(
        "\n\nWhat non-destructive changes would most improve I/O performance and make better use of the storage that already exists here?",
    );
    s
}

/// Lines of a model answer that would delete or overwrite data if run.
/// Models do not always follow the non-destructive rule, so the app checks.
pub fn destructive_lines(answer: &str) -> Vec<String> {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    let patterns = PATTERNS.get_or_init(|| {
        [
            r"(^|[\s;&|])(sudo\s+)?rm\s",
            r"\bdiskutil\s+(erase|reformat|apfs\s+delete)",
            r"\bnewfs|\bmkfs",
            r"\bdd\s+.*\bof=",
            r"\btmutil\s+delete",
            r"\bfind\b.*-delete\b",
            r"rsync\b.*--(delete|remove-source-files)",
            r"\bshred\b|\bsrm\b",
            r"(?i)^(>\s*|PS [^>]*>\s*)?(del|erase|rd|rmdir)\s+\S",
            r"(?i)\bRemove-Item\b|\bClear-Content\b|\bClear-RecycleBin\b|\bFormat-Volume\b|\bClear-Disk\b",
            r"(?i)(^|[\s;&|])format\s+[a-z]:",
            r"(?i)\brobocopy\b.*\s/(mir|purge|mov|move)\b",
        ]
        .iter()
        .map(|p| Regex::new(p).unwrap())
        .collect()
    });
    answer
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#') && patterns.iter().any(|p| p.is_match(l)))
        .map(String::from)
        .collect()
}

pub const ANTHROPIC: &str = "https://api.anthropic.com";
pub const OPENAI: &str = "https://api.openai.com";
/// Used when a provider's model list cannot be fetched.
pub const ANTHROPIC_DEFAULT_MODEL: &str = "claude-opus-5-5";
pub const OPENAI_DEFAULT_MODEL: &str = "gpt-4o";
pub const AGENTICWORK_DEFAULT_MODEL: &str = "auto";

fn agent(read_secs: u64) -> ureq::Agent {
    ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(4)).timeout_read(Duration::from_secs(read_secs)).build()
}

fn http_error(who: &str, e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, r) => {
            let body = r.into_string().unwrap_or_default();
            let detail = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| v["error"]["message"].as_str().or(v["message"].as_str()).or(v["error"].as_str()).map(String::from))
                .unwrap_or_else(|| body.chars().take(300).collect());
            match code {
                401 | 403 => format!("{who} rejected the API key (HTTP {code}). {detail}"),
                _ => format!("{who} returned HTTP {code}. {detail}"),
            }
        }
        other => format!("Could not reach {who}: {other}"),
    }
}

/// Text-capable Ollama models at `base`, biggest first. Empty when Ollama is not reachable.
pub fn ollama_models(base: &str) -> Vec<String> {
    let Ok(resp) = agent(5).get(&format!("{base}/api/tags")).call() else { return Vec::new() };
    let Ok(body) = resp.into_json::<serde_json::Value>() else { return Vec::new() };
    let mut models: Vec<(u64, String)> = body["models"]
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or(&[])
        .iter()
        .filter_map(|m| {
            let name = m["name"].as_str()?.to_string();
            let size = m["size"].as_u64().unwrap_or(0);
            let chat = m["capabilities"].as_array().map(|c| c.iter().any(|v| v == "completion")).unwrap_or(true);
            // Skip embedding-only models and tiny vision models that give poor advice.
            (chat && size > 3_000_000_000).then_some((size, name))
        })
        .collect();
    models.sort_by(|a, b| b.cmp(a));
    models.into_iter().map(|m| m.1).collect()
}

/// Model ids from an OpenAI-style or Anthropic `/v1/models` endpoint.
fn list_models(base: &str, headers: &[(&str, &str)]) -> Option<Vec<String>> {
    let mut req = agent(6).get(&format!("{base}/v1/models"));
    for (k, v) in headers {
        req = req.set(k, v);
    }
    let body = req.call().ok()?.into_json::<serde_json::Value>().ok()?;
    Some(body["data"].as_array()?.iter().filter_map(|m| {
        // AgenticWork lists image and embedding models too; keep chat ones.
        let kind = m["model_type"].as_str().unwrap_or("chat");
        (kind == "chat").then(|| m["id"].as_str().map(String::from)).flatten()
    }).collect())
}

fn backend(kind: &str, model: &str, label: String, remote: bool) -> Backend {
    Backend { kind: kind.into(), model: model.into(), label, remote }
}

/// Put `first` at the front of the list, adding it if the list is empty.
fn prefer(mut models: Vec<String>, first: &str) -> Vec<String> {
    if let Some(i) = models.iter().position(|m| m == first) {
        let m = models.remove(i);
        models.insert(0, m);
    } else if models.is_empty() {
        models.push(first.into());
    }
    models
}

/// Every model the advisor can use with these settings. Empty means the
/// advisor has nothing to talk to, and the UI hides it.
pub fn backends(settings: &Settings) -> Vec<Backend> {
    if !settings.advisor_enabled {
        return Vec::new();
    }
    let s = settings;
    std::thread::scope(|scope| {
        let ollama = scope.spawn(|| {
            let local = crate::settings::is_local(&s.ollama_host);
            let place = if local { "Ollama".to_string() } else { format!("Ollama at {}", s.ollama_host.split("://").nth(1).unwrap_or(&s.ollama_host)) };
            ollama_models(&s.ollama_host).into_iter().map(|m| backend("ollama", &m, format!("{place}: {m}"), !local)).collect::<Vec<_>>()
        });
        let agenticwork = scope.spawn(|| {
            if s.agenticwork_key.is_empty() || s.agenticwork_url.is_empty() {
                return Vec::new();
            }
            let auth = format!("Bearer {}", s.agenticwork_key);
            let models = list_models(&s.agenticwork_url, &[("Authorization", &auth)]).unwrap_or_default();
            prefer(models, AGENTICWORK_DEFAULT_MODEL).into_iter().take(40).map(|m| backend("agenticwork", &m, format!("AgenticWork: {m}"), true)).collect()
        });
        let anthropic = scope.spawn(|| {
            if s.anthropic_key.is_empty() {
                return Vec::new();
            }
            let models = list_models(ANTHROPIC, &[("x-api-key", &s.anthropic_key), ("anthropic-version", "2023-06-01")]).unwrap_or_default();
            prefer(models, ANTHROPIC_DEFAULT_MODEL).into_iter().take(20).map(|m| backend("anthropic", &m, format!("Anthropic: {m}"), true)).collect()
        });
        let openai = scope.spawn(|| {
            if s.openai_key.is_empty() {
                return Vec::new();
            }
            let auth = format!("Bearer {}", s.openai_key);
            let mut models: Vec<String> = list_models(OPENAI, &[("Authorization", &auth)])
                .unwrap_or_default()
                .into_iter()
                .filter(|m| (m.starts_with("gpt-") || m.starts_with("o") || m.starts_with("chatgpt")) && !m.contains("audio") && !m.contains("realtime") && !m.contains("image") && !m.contains("tts") && !m.contains("transcribe"))
                .collect();
            models.sort_by(|a, b| b.cmp(a));
            prefer(models, OPENAI_DEFAULT_MODEL).into_iter().take(25).map(|m| backend("openai", &m, format!("OpenAI: {m}"), true)).collect()
        });
        let mut out = ollama.join().unwrap_or_default();
        out.extend(agenticwork.join().unwrap_or_default());
        out.extend(anthropic.join().unwrap_or_default());
        out.extend(openai.join().unwrap_or_default());
        out
    })
}

/// Stream the reply, calling `on_chunk` with each piece of text. Stops early
/// when `stop` is set. Keys come from `settings`, never from the UI.
pub fn stream(settings: &Settings, messages: &[Message], backend: &Backend, stop: &AtomicBool, on_chunk: &mut dyn FnMut(&str)) -> Result<(), String> {
    match backend.kind.as_str() {
        "ollama" => stream_ollama(&settings.ollama_host, messages, &backend.model, stop, on_chunk),
        "anthropic" => stream_anthropic(ANTHROPIC, &settings.anthropic_key, messages, &backend.model, stop, on_chunk),
        "openai" => stream_openai("OpenAI", OPENAI, &settings.openai_key, messages, &backend.model, stop, on_chunk),
        "agenticwork" => stream_openai("AgenticWork", &settings.agenticwork_url, &settings.agenticwork_key, messages, &backend.model, stop, on_chunk),
        other => Err(format!("unknown provider {other}")),
    }
}

pub fn stream_ollama(base: &str, messages: &[Message], model: &str, stop: &AtomicBool, on_chunk: &mut dyn FnMut(&str)) -> Result<(), String> {
    let body = serde_json::json!({
        "model": model,
        "stream": true,
        "think": false,
        "options": { "temperature": 0.3, "num_ctx": 12288, "num_predict": 1800 },
        "messages": messages.iter().map(|m| serde_json::json!({ "role": m.role, "content": m.text })).collect::<Vec<_>>(),
    });
    let resp = agent(600).post(&format!("{base}/api/chat")).send_json(body).map_err(|e| http_error("Ollama", e))?;
    for line in BufReader::new(resp.into_reader()).lines() {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let line = line.map_err(|e| e.to_string())?;
        let Ok(obj) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
        if let Some(err) = obj["error"].as_str() {
            return Err(format!("Ollama: {err}"));
        }
        if let Some(text) = obj["message"]["content"].as_str() {
            if !text.is_empty() {
                on_chunk(text);
            }
        }
        if obj["done"].as_bool() == Some(true) {
            break;
        }
    }
    Ok(())
}

/// Call `on_event` with the JSON of each `data:` line of a server-sent event
/// stream. Return `Ok(false)` from it to stop reading.
fn read_sse(reader: impl std::io::Read, stop: &AtomicBool, mut on_event: impl FnMut(serde_json::Value) -> Result<bool, String>) -> Result<(), String> {
    for line in BufReader::new(reader).lines() {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let line = line.map_err(|e| e.to_string())?;
        let Some(data) = line.strip_prefix("data:") else { continue };
        let data = data.trim();
        if data == "[DONE]" {
            break;
        }
        let Ok(obj) = serde_json::from_str::<serde_json::Value>(data) else { continue };
        if !on_event(obj)? {
            break;
        }
    }
    Ok(())
}

/// Anthropic Messages API, streamed. The system prompt is a top-level field.
pub fn stream_anthropic(base: &str, key: &str, messages: &[Message], model: &str, stop: &AtomicBool, on_chunk: &mut dyn FnMut(&str)) -> Result<(), String> {
    let system = messages.iter().filter(|m| m.role == "system").map(|m| m.text.as_str()).collect::<Vec<_>>().join("\n\n");
    let body = serde_json::json!({
        "model": model,
        "max_tokens": 16000,
        "stream": true,
        "system": system,
        "messages": messages.iter().filter(|m| m.role != "system").map(|m| serde_json::json!({ "role": m.role, "content": m.text })).collect::<Vec<_>>(),
    });
    let resp = agent(600)
        .post(&format!("{base}/v1/messages"))
        .set("x-api-key", key)
        .set("anthropic-version", "2023-06-01")
        .send_json(body)
        .map_err(|e| http_error("Anthropic", e))?;
    read_sse(resp.into_reader(), stop, |ev| match ev["type"].as_str() {
        Some("content_block_delta") => {
            if ev["delta"]["type"] == "text_delta" {
                if let Some(text) = ev["delta"]["text"].as_str() {
                    on_chunk(text);
                }
            }
            Ok(true)
        }
        Some("message_delta") if ev["delta"]["stop_reason"] == "refusal" => Err("The model declined to answer this request.".into()),
        Some("error") => Err(format!("Anthropic: {}", ev["error"]["message"].as_str().unwrap_or("stream error"))),
        Some("message_stop") => Ok(false),
        _ => Ok(true),
    })
}

/// OpenAI-style chat completions, streamed. AgenticWork speaks the same protocol.
pub fn stream_openai(who: &str, base: &str, key: &str, messages: &[Message], model: &str, stop: &AtomicBool, on_chunk: &mut dyn FnMut(&str)) -> Result<(), String> {
    let body = serde_json::json!({
        "model": model,
        "stream": true,
        "messages": messages.iter().map(|m| serde_json::json!({ "role": m.role, "content": m.text })).collect::<Vec<_>>(),
    });
    let resp = agent(600)
        .post(&format!("{base}/v1/chat/completions"))
        .set("Authorization", &format!("Bearer {key}"))
        .send_json(body)
        .map_err(|e| http_error(who, e))?;
    read_sse(resp.into_reader(), stop, |ev| {
        if let Some(err) = ev["error"]["message"].as_str().or(ev["error"].as_str()) {
            return Err(format!("{who}: {err}"));
        }
        if let Some(text) = ev["choices"][0]["delta"]["content"].as_str() {
            if !text.is_empty() {
                on_chunk(text);
            }
        }
        Ok(true)
    })
}

/// One advisor conversation, filled in by a background thread and read by polling.
#[derive(Default)]
pub struct Session {
    pub messages: Vec<Message>,
    pub running: bool,
    pub error: Option<String>,
}

#[derive(Clone, Default)]
pub struct Advisor {
    pub session: Arc<Mutex<Session>>,
    stop: Arc<AtomicBool>,
}

impl Advisor {
    /// Start (or continue) the conversation. `facts` builds the opening user
    /// message and is only called for the first question.
    pub fn ask(&self, settings: Settings, backend: Backend, follow_up: Option<String>, facts: impl FnOnce() -> String + Send + 'static) -> Result<(), String> {
        {
            let mut s = self.session.lock().unwrap();
            if s.running {
                return Err("The advisor is already answering.".into());
            }
            s.running = true;
            s.error = None;
            if let Some(q) = follow_up.filter(|_| !s.messages.is_empty()) {
                s.messages.push(Message { role: "user".into(), text: q });
            }
        }
        self.stop.store(false, Ordering::SeqCst);
        let session = self.session.clone();
        let stop = self.stop.clone();
        std::thread::spawn(move || {
            let history = {
                let mut s = session.lock().unwrap();
                if s.messages.is_empty() {
                    drop(s);
                    let opening = facts();
                    s = session.lock().unwrap();
                    s.messages.push(Message { role: "system".into(), text: system_prompt() });
                    s.messages.push(Message { role: "user".into(), text: opening });
                }
                let history = s.messages.clone();
                s.messages.push(Message { role: "assistant".into(), text: String::new() });
                history
            };
            let result = stream(&settings, &history, &backend, &stop, &mut |chunk| {
                if let Some(last) = session.lock().unwrap().messages.last_mut() {
                    last.text.push_str(chunk);
                }
            });
            let mut s = session.lock().unwrap();
            if let Err(e) = result {
                s.error = Some(e);
            }
            if s.messages.last().is_some_and(|m| m.role == "assistant" && m.text.is_empty()) {
                s.messages.pop();
            }
            s.running = false;
        });
        Ok(())
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }

    pub fn reset(&self) {
        self.stop();
        let mut s = self.session.lock().unwrap();
        s.messages.clear();
        s.error = None;
    }
}
