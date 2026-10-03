//! Hidden space: what a volume reports as used but a scan could not see, such
//! as swap, snapshots, purgeable space, system volumes and unreadable folders.

use crate::facts::{self, run};
use serde::Serialize;
use std::time::Duration;

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Part {
    pub name: String,
    pub bytes: u64,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Hidden {
    pub bytes: u64,
    /// Known pieces, largest first. Whatever is left is `other`.
    pub parts: Vec<Part>,
    pub other: u64,
    pub volume_used: u64,
}

fn same_path(a: &str, b: &str) -> bool {
    let norm = |s: &str| {
        let t = s.trim_end_matches(['/', '\\']);
        if t.is_empty() { "/".to_string() } else { t.to_lowercase() }
    };
    norm(a) == norm(b)
}

/// Hidden space for a scan of `root` that found `scanned` bytes. `None` when
/// `root` is not the top of a volume, or when nothing meaningful is hidden.
pub fn hidden_space(root: &str, scanned: u64) -> Option<Hidden> {
    let vol = facts::volumes().into_iter().find(|v| same_path(&v.mount, root))?;
    let used = vol.total.saturating_sub(vol.free);
    split(used, scanned, if vol.mount == "/" { system_parts() } else { Vec::new() })
}

/// Pure arithmetic, separate so it can be tested.
pub fn split(used: u64, scanned: u64, known: Vec<Part>) -> Option<Hidden> {
    let bytes = used.saturating_sub(scanned);
    // Below half a percent of the volume it is noise from files changing during the scan.
    if bytes == 0 || bytes < used / 200 {
        return None;
    }
    let mut parts: Vec<Part> = known.into_iter().filter(|p| p.bytes > 0).collect();
    parts.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    // Never claim more than the hidden total.
    let mut left = bytes;
    for p in parts.iter_mut() {
        p.bytes = p.bytes.min(left);
        left -= p.bytes;
    }
    parts.retain(|p| p.bytes > 0);
    Some(Hidden { bytes, parts, other: left, volume_used: used })
}

/// macOS: the APFS volumes that share the startup container but are not
/// reached by scanning "/" (the System and Data volumes are).
#[cfg(target_os = "macos")]
fn system_parts() -> Vec<Part> {
    let t = Duration::from_secs(10);
    let Some(container) = run("/bin/sh", &["-c", "diskutil info -plist / | plutil -extract APFSContainerReference raw -"], t, 100) else {
        return Vec::new();
    };
    let Some(json) = run("/bin/sh", &["-c", "diskutil apfs list -plist | plutil -convert json -o - -"], t, 200_000) else {
        return Vec::new();
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&json) else { return Vec::new() };
    let mut out = Vec::new();
    for c in v["Containers"].as_array().into_iter().flatten() {
        if c["ContainerReference"].as_str() != Some(container.trim()) {
            continue;
        }
        for vol in c["Volumes"].as_array().into_iter().flatten() {
            let roles: Vec<&str> = vol["Roles"].as_array().into_iter().flatten().filter_map(|r| r.as_str()).collect();
            let name = match roles.first().copied() {
                Some("VM") => "Swap (VM volume)",
                Some("Preboot") => "Preboot volume",
                Some("Recovery") => "Recovery volume",
                Some("Update") => "Update volume",
                _ => continue, // System and Data are scanned
            };
            out.push(Part { name: name.into(), bytes: vol["CapacityInUse"].as_u64().unwrap_or(0) });
        }
    }
    out
}

#[cfg(not(target_os = "macos"))]
fn system_parts() -> Vec<Part> {
    let _ = (run, Duration::from_secs(0));
    Vec::new()
}
