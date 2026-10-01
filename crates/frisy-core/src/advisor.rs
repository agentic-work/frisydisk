//! Asks a model running on this machine for storage and I/O suggestions.
//! The advisor only ever produces text; nothing it suggests is run.

use crate::facts;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};
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
    /// "ollama" or "agenticode"
    pub kind: String,
    pub model: String,
    pub label: String,
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

pub const OLLAMA: &str = "http://127.0.0.1:11434";

fn agent(timeout_secs: u64) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(3))
        .timeout_read(Duration::from_secs(timeout_secs))
        .build()
}

/// Text-capable Ollama models installed locally, biggest first. Empty when Ollama is not running.
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

/// Path of an agenticode CLI that actually launches, if any.
pub fn agenticode_cli() -> Option<String> {
    let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_default();
    let exe = if cfg!(windows) { "agenticode.exe" } else { "agenticode" };
    [format!("{home}/.local/bin/{exe}"), format!("/usr/local/bin/{exe}"), format!("/opt/homebrew/bin/{exe}")]
        .into_iter()
        .find(|p| std::path::Path::new(p).is_file() && facts::run(p, &["--version"], Duration::from_secs(8), 200).is_some())
}

pub fn backends() -> Vec<Backend> {
    let mut out = Vec::new();
    if agenticode_cli().is_some() {
        out.push(Backend { kind: "agenticode".into(), model: String::new(), label: "agenticode".into() });
    }
    for m in ollama_models(OLLAMA) {
        out.push(Backend { kind: "ollama".into(), label: format!("Ollama: {m}"), model: m });
    }
    out
}

/// Stream the reply, calling `on_chunk` with each piece of text. Stops early
/// when `stop` is set.
pub fn stream(messages: &[Message], backend: &Backend, stop: &AtomicBool, on_chunk: &mut dyn FnMut(&str)) -> Result<(), String> {
    match backend.kind.as_str() {
        "ollama" => stream_ollama(OLLAMA, messages, &backend.model, stop, on_chunk),
        "agenticode" => stream_agenticode(messages, stop, on_chunk),
        other => Err(format!("unknown backend {other}")),
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
    let resp = agent(600).post(&format!("{base}/api/chat")).send_json(body).map_err(|e| match e {
        ureq::Error::Status(code, r) => format!("Ollama returned HTTP {code}: {}", r.into_string().unwrap_or_default().chars().take(300).collect::<String>()),
        other => format!("Could not reach Ollama: {other}"),
    })?;
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

/// agenticode is an agent CLI. It is run in print mode with every tool
/// disallowed, so it can only answer in text.
fn stream_agenticode(messages: &[Message], stop: &AtomicBool, on_chunk: &mut dyn FnMut(&str)) -> Result<(), String> {
    let cli = agenticode_cli().ok_or("The agenticode CLI could not be launched.")?;
    let prompt = messages
        .iter()
        .map(|m| match m.role.as_str() {
            "system" => m.text.clone(),
            "user" => format!("USER:\n{}", m.text),
            _ => format!("ASSISTANT:\n{}", m.text),
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let mut child = Command::new(cli)
        .args(["-p", &prompt, "--allowedTools", ""])
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let mut out = child.stdout.take().ok_or("no output from agenticode")?;
    let mut buf = [0u8; 2048];
    loop {
        if stop.load(Ordering::SeqCst) {
            let _ = child.kill();
            break;
        }
        match out.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => on_chunk(&String::from_utf8_lossy(&buf[..n])),
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if status.success() || stop.load(Ordering::SeqCst) {
        Ok(())
    } else {
        Err(format!("agenticode exited with {status}"))
    }
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
    pub fn ask(&self, backend: Backend, follow_up: Option<String>, facts: impl FnOnce() -> String + Send + 'static) -> Result<(), String> {
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
            let result = stream(&history, &backend, &stop, &mut |chunk| {
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
