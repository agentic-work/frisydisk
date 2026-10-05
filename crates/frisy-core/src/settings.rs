//! Advisor settings, kept in a small JSON file in the user's config folder.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const DEFAULT_OLLAMA: &str = "http://127.0.0.1:11434";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub advisor_enabled: bool,
    /// Where Ollama listens. Can point at another machine.
    pub ollama_host: String,
    /// Base URL of an AgenticWork deployment, for example https://chat.example.com
    pub agenticwork_url: String,
    pub agenticwork_key: String,
    pub anthropic_key: String,
    pub openai_key: String,
    /// GitHub Copilot through the agenticode VS Code bridge, found by its lockfile.
    pub copilot_enabled: bool,
    /// Optional bridge address and token, for when the lockfile is elsewhere.
    pub copilot_url: String,
    pub copilot_token: String,

    // ---- Appearance ----
    /// "auto" follows the OS, or force "light" / "dark".
    pub theme: String,
    /// "decimal" (1 KB = 1000 B, the default) or "binary" (1 KiB = 1024 B).
    pub units: String,

    // ---- Scanning ----
    /// How many folder levels the sunburst/chart draws. 1..=12.
    pub scan_depth: u32,
    /// Slices smaller than this fraction of the total are folded away. 0.0..=0.2.
    pub scan_min_fraction: f64,

    // ---- Cleanup ----
    /// Default cleanup action offered: "trash" or "delete".
    pub clean_mode: String,
    /// A project's build folder counts as stale after this many days untouched. 1..=3650.
    pub stale_days: u64,

    // ---- I/O benchmark ----
    pub bench_total_bytes: u64,
    pub bench_block_bytes: u64,
    pub bench_random_ops: u32,
    pub bench_write: bool,

    // ---- Advisor behaviour ----
    /// Ask the advisor automatically as soon as a scan finishes.
    pub auto_ask: bool,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            advisor_enabled: true,
            ollama_host: DEFAULT_OLLAMA.into(),
            agenticwork_url: String::new(),
            agenticwork_key: String::new(),
            anthropic_key: String::new(),
            openai_key: String::new(),
            copilot_enabled: true,
            copilot_url: String::new(),
            copilot_token: String::new(),
            theme: "auto".into(),
            units: "decimal".into(),
            scan_depth: 6,
            scan_min_fraction: 0.004,
            clean_mode: "trash".into(),
            stale_days: 90,
            bench_total_bytes: 64 * 1024 * 1024,
            bench_block_bytes: 1024 * 1024,
            bench_random_ops: 1024,
            bench_write: true,
            auto_ask: false,
        }
    }
}

/// `FRISYDISK_CONFIG_DIR` overrides the location (used by tests).
pub fn config_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("FRISYDISK_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    let home = PathBuf::from(std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_default());
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/FrisyDisk")
    } else if cfg!(windows) {
        std::env::var("APPDATA").map(PathBuf::from).unwrap_or(home).join("FrisyDisk")
    } else {
        std::env::var("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|_| home.join(".config")).join("frisydisk")
    }
}

pub fn path() -> PathBuf {
    config_dir().join("settings.json")
}

impl Settings {
    pub fn load() -> Settings {
        std::fs::read_to_string(path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    /// Write the file readable by this user only, since it can hold API keys.
    pub fn save(&self) -> Result<(), String> {
        let file = path();
        std::fs::create_dir_all(file.parent().unwrap()).map_err(|e| e.to_string())?;
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&file).map_err(|e| e.to_string())?;
            f.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600));
            Ok(())
        }
        #[cfg(not(unix))]
        std::fs::write(&file, text).map_err(|e| e.to_string())
    }

    /// Apply a change from the UI. A key field that is absent keeps its value,
    /// an empty string clears it.
    pub fn apply(&mut self, change: &serde_json::Value) {
        let text = |k: &str| change[k].as_str().map(|s| s.trim().to_string());
        if let Some(b) = change["advisor_enabled"].as_bool() {
            self.advisor_enabled = b;
        }
        if let Some(v) = text("ollama_host") {
            self.ollama_host = if v.is_empty() { DEFAULT_OLLAMA.into() } else { normalize_url(&v) };
        }
        if let Some(v) = text("agenticwork_url") {
            self.agenticwork_url = if v.is_empty() { v } else { normalize_url(&v) };
        }
        if let Some(v) = text("agenticwork_key") {
            self.agenticwork_key = v;
        }
        if let Some(v) = text("anthropic_key") {
            self.anthropic_key = v;
        }
        if let Some(v) = text("openai_key") {
            self.openai_key = v;
        }
        if let Some(b) = change["copilot_enabled"].as_bool() {
            self.copilot_enabled = b;
        }
        if let Some(v) = text("copilot_url") {
            self.copilot_url = if v.is_empty() { v } else { normalize_url(&v) };
        }
        if let Some(v) = text("copilot_token") {
            self.copilot_token = v;
        }
        if let Some(v) = text("theme") {
            if matches!(v.as_str(), "auto" | "light" | "dark") {
                self.theme = v;
            }
        }
        if let Some(v) = text("units") {
            if matches!(v.as_str(), "decimal" | "binary") {
                self.units = v;
            }
        }
        if let Some(n) = change["scan_depth"].as_u64() {
            self.scan_depth = (n as u32).clamp(1, 12);
        }
        if let Some(f) = change["scan_min_fraction"].as_f64() {
            self.scan_min_fraction = f.clamp(0.0, 0.2);
        }
        if let Some(v) = text("clean_mode") {
            if matches!(v.as_str(), "trash" | "delete") {
                self.clean_mode = v;
            }
        }
        if let Some(n) = change["stale_days"].as_u64() {
            self.stale_days = n.clamp(1, 3650);
        }
        if let Some(n) = change["bench_total_bytes"].as_u64() {
            self.bench_total_bytes = n.clamp(4 * 1024 * 1024, crate::benchmark::MAX_TOTAL);
        }
        if let Some(n) = change["bench_block_bytes"].as_u64() {
            self.bench_block_bytes = n.clamp(4096, 64 * 1024 * 1024);
        }
        if let Some(n) = change["bench_random_ops"].as_u64() {
            self.bench_random_ops = (n as u32).clamp(1, 100_000);
        }
        if let Some(b) = change["bench_write"].as_bool() {
            self.bench_write = b;
        }
        if let Some(b) = change["auto_ask"].as_bool() {
            self.auto_ask = b;
        }
    }

    /// What the UI may see: never the keys themselves.
    pub fn public(&self) -> serde_json::Value {
        serde_json::json!({
            "advisor_enabled": self.advisor_enabled,
            "ollama_host": self.ollama_host,
            "agenticwork_url": self.agenticwork_url,
            "has_agenticwork_key": !self.agenticwork_key.is_empty(),
            "has_anthropic_key": !self.anthropic_key.is_empty(),
            "has_openai_key": !self.openai_key.is_empty(),
            "copilot_enabled": self.copilot_enabled,
            "copilot_url": self.copilot_url,
            "has_copilot_token": !self.copilot_token.is_empty(),
            "theme": self.theme,
            "units": self.units,
            "scan_depth": self.scan_depth,
            "scan_min_fraction": self.scan_min_fraction,
            "clean_mode": self.clean_mode,
            "stale_days": self.stale_days,
            "bench_total_bytes": self.bench_total_bytes,
            "bench_block_bytes": self.bench_block_bytes,
            "bench_random_ops": self.bench_random_ops,
            "bench_write": self.bench_write,
            "auto_ask": self.auto_ask,
            "file": path().to_string_lossy(),
        })
    }
}

/// "nas.local:11434/" -> "http://nas.local:11434"
pub fn normalize_url(v: &str) -> String {
    let v = v.trim().trim_end_matches('/');
    let v = v.strip_suffix("/v1").unwrap_or(v);
    if v.starts_with("http://") || v.starts_with("https://") {
        v.to_string()
    } else {
        format!("http://{v}")
    }
}

/// True when the address is this machine.
pub fn is_local(url: &str) -> bool {
    let host = url.split("://").nth(1).unwrap_or(url);
    let host = host.split('/').next().unwrap_or(host);
    let host = host.rsplit_once(':').map_or(host, |(h, _)| h);
    matches!(host, "127.0.0.1" | "localhost" | "[::1]" | "::1")
}
