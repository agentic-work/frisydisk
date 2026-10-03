//! Cleanup: temporary files, caches, logs and build folders nobody has used in
//! months. Nothing is removed until the person picks categories and confirms;
//! even then only items inside a known cleanup folder can be touched.

use crate::scan::Scan;
use crate::tree::{NodeId, Tree, FLAG_DIR};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime};

/// What gets removed for a target.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Everything inside the folder, but not the folder itself.
    Contents,
    /// The folder itself (used for build folders).
    Whole,
}

#[derive(Clone, Debug)]
pub struct Target {
    pub id: String,
    pub label: String,
    pub detail: String,
    /// temp, cache, logs, dev, trash
    pub group: &'static str,
    pub roots: Vec<PathBuf>,
    pub kind: Kind,
    /// Items changed more recently than this are left alone (they may be in use).
    pub min_age: Duration,
    /// Only items owned by this user (shared temp folders on Linux).
    pub own_only: bool,
    pub needs_admin: bool,
    /// Moving these to the Trash makes no sense (they are the Trash).
    pub delete_only: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Found {
    pub id: String,
    pub label: String,
    pub detail: String,
    pub group: &'static str,
    pub bytes: u64,
    pub count: usize,
    pub needs_admin: bool,
    pub delete_only: bool,
    /// A few example paths so people can see what this is.
    pub examples: Vec<String>,
    #[serde(skip)]
    pub items: Vec<(PathBuf, u64)>,
    #[serde(skip)]
    pub roots: Vec<PathBuf>,
    /// Identity of each item when it was measured. An item whose identity has
    /// changed since (swapped for a link, replaced) is skipped.
    #[serde(skip)]
    pub fingerprints: HashMap<PathBuf, Fingerprint>,
    #[serde(skip)]
    pub min_age: Duration,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct Report {
    pub removed: usize,
    pub freed: u64,
    pub skipped: usize,
    pub errors: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mode {
    Trash,
    Delete,
}

/// Where things live on this machine; a struct so tests can point it at a temp folder.
#[derive(Clone, Debug)]
pub struct Places {
    pub home: PathBuf,
    pub temp: PathBuf,
    /// Other shared temporary folders (Linux: /var/tmp). Only items owned by
    /// this user are ever considered there.
    pub shared_temp: Vec<PathBuf>,
    /// %LOCALAPPDATA% on Windows.
    pub local_app_data: Option<PathBuf>,
    pub windows_dir: Option<PathBuf>,
}

impl Places {
    pub fn of_this_machine() -> Places {
        let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
        Places {
            home: env("HOME").or_else(|| env("USERPROFILE")).unwrap_or_default(),
            temp: std::env::temp_dir(),
            shared_temp: if cfg!(target_os = "linux") { vec![PathBuf::from("/var/tmp")] } else { Vec::new() },
            local_app_data: env("LOCALAPPDATA"),
            windows_dir: env("SystemRoot").or_else(|| env("windir")),
        }
    }
}

const HOUR: Duration = Duration::from_secs(3600);
const DAY: Duration = Duration::from_secs(86_400);

fn target(id: &str, label: &str, detail: &str, group: &'static str, roots: Vec<PathBuf>, min_age: Duration) -> Target {
    Target {
        id: id.into(),
        label: label.into(),
        detail: detail.into(),
        group,
        roots,
        kind: Kind::Contents,
        min_age,
        own_only: false,
        needs_admin: false,
        delete_only: false,
    }
}

/// Cleanup targets for this operating system.
pub fn targets(p: &Places, os: &str) -> Vec<Target> {
    let h = &p.home;
    let mut t = Vec::new();
    match os {
        "macos" => {
            t.push(target("temp", "Temporary files", "Your temporary folder; items untouched for a day", "temp", vec![p.temp.clone()], DAY));
            t.push(target("caches", "App caches", "~/Library/Caches: rebuilt by each app when needed", "cache", vec![h.join("Library/Caches")], HOUR));
            t.push(target("logs", "Logs", "~/Library/Logs", "logs", vec![h.join("Library/Logs")], DAY));
            t.push(target("xcode", "Xcode build data", "DerivedData and simulator caches", "dev",
                vec![h.join("Library/Developer/Xcode/DerivedData"), h.join("Library/Developer/CoreSimulator/Caches")], HOUR));
        }
        "windows" => {
            let local = p.local_app_data.clone().unwrap_or_else(|| h.join("AppData/Local"));
            t.push(target("temp", "Temporary files", "Your %TEMP% folder; items untouched for a day", "temp", vec![p.temp.clone()], DAY));
            if let Some(win) = &p.windows_dir {
                let mut sys = target("windows-temp", "Windows temporary files", "C:\\Windows\\Temp", "temp", vec![win.join("Temp")], DAY);
                sys.needs_admin = true;
                t.push(sys);
                let mut wu = target("windows-update", "Windows Update downloads", "Installers Windows has already applied", "cache",
                    vec![win.join("SoftwareDistribution/Download")], DAY);
                wu.needs_admin = true;
                t.push(wu);
            }
            t.push(target("crash-dumps", "Crash dumps", "%LOCALAPPDATA%\\CrashDumps", "logs", vec![local.join("CrashDumps")], HOUR));
            t.push(target("browser", "Browser caches", "Edge and Chrome cache folders", "cache", vec![
                local.join("Microsoft/Edge/User Data/Default/Cache/Cache_Data"),
                local.join("Google/Chrome/User Data/Default/Cache/Cache_Data"),
                local.join("Microsoft/Windows/INetCache"),
            ], HOUR));
            t.push(target("dev-caches", "Developer caches", "npm, pip and Yarn download caches", "dev", vec![
                local.join("npm-cache/_cacache"),
                local.join("pip/Cache"),
                local.join("Yarn/Cache"),
            ], HOUR));
        }
        _ => {
            let mut roots = vec![p.temp.clone()];
            roots.extend(p.shared_temp.iter().cloned());
            let mut tmp = target("temp", "Temporary files", "Your files in /tmp and /var/tmp untouched for a day", "temp", roots, DAY);
            tmp.own_only = true;
            t.push(tmp);
            t.push(target("caches", "App caches", "~/.cache: rebuilt by each app when needed", "cache", vec![h.join(".cache")], HOUR));
            let mut trash = target("trash", "Trash", "Files you already deleted", "trash",
                vec![h.join(".local/share/Trash/files"), h.join(".local/share/Trash/info")], Duration::ZERO);
            trash.delete_only = true;
            t.push(trash);
        }
    }
    // The same on every system.
    if os != "windows" {
        t.push(target("dev-caches", "Developer caches", "npm, Cargo and Gradle download caches", "dev", vec![
            h.join(".npm/_cacache"),
            h.join(".cargo/registry/cache"),
            h.join(".gradle/caches"),
        ], HOUR));
    } else {
        if let Some(dev) = t.iter_mut().find(|x| x.id == "dev-caches") {
            dev.roots.push(h.join(".cargo/registry/cache"));
            dev.roots.push(h.join(".gradle/caches"));
        }
    }
    if os == "macos" {
        let mut trash = target("trash", "Trash", "Files you already deleted", "trash", vec![h.join(".Trash")], Duration::ZERO);
        trash.delete_only = true;
        t.push(trash);
    }
    t
}

/// File type plus, on Unix, device and inode.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fingerprint {
    is_dir: bool,
    is_link: bool,
    id: (u64, u64),
}

fn fingerprint(meta: &std::fs::Metadata) -> Fingerprint {
    #[cfg(unix)]
    let id = {
        use std::os::unix::fs::MetadataExt;
        (meta.dev(), meta.ino())
    };
    #[cfg(not(unix))]
    let id = (0, 0);
    Fingerprint { is_dir: meta.is_dir(), is_link: meta.file_type().is_symlink(), id }
}

/// Time since the item was last modified or created, whichever is later. A file
/// copied or extracted just now keeps its old modified time but is new, and may
/// be in use.
fn age(meta: &std::fs::Metadata) -> Duration {
    let newest = [meta.modified().ok(), Some(meta.created().ok().unwrap_or(SystemTime::UNIX_EPOCH))]
        .into_iter()
        .flatten()
        .max()
        .unwrap_or(SystemTime::UNIX_EPOCH);
    SystemTime::now().duration_since(newest).unwrap_or(Duration::ZERO)
}

#[cfg(unix)]
fn owned_by_me(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    meta.uid() == unsafe { libc::geteuid() }
}

#[cfg(not(unix))]
fn owned_by_me(_: &std::fs::Metadata) -> bool {
    true
}

/// Measure one target: which items qualify and how much they hold.
pub fn measure(t: &Target) -> Found {
    let mut items: Vec<(PathBuf, u64)> = Vec::new();
    for root in &t.roots {
        let Ok(listing) = std::fs::read_dir(root) else { continue };
        let candidates: Vec<PathBuf> = listing
            .flatten()
            .filter_map(|e| {
                let path = e.path();
                let meta = std::fs::symlink_metadata(&path).ok()?;
                if age(&meta) < t.min_age || (t.own_only && !owned_by_me(&meta)) {
                    return None;
                }
                Some(path)
            })
            .collect();
        if candidates.is_empty() {
            continue;
        }
        // One scan of the folder gives every child's size at once.
        let scan = Scan::run(&root.to_string_lossy());
        let tree = scan.tree.lock().unwrap();
        let sizes: std::collections::HashMap<&str, u64> =
            tree.nodes[0].children.iter().map(|&c| (&*tree.nodes[c as usize].name, tree.nodes[c as usize].size)).collect();
        for path in candidates {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let size = sizes.get(name.as_str()).copied().unwrap_or(0);
            items.push((path, size));
        }
    }
    items.sort_by(|a, b| b.1.cmp(&a.1));
    found(t, items)
}

fn found(t: &Target, items: Vec<(PathBuf, u64)>) -> Found {
    let fingerprints = items
        .iter()
        .filter_map(|(p, _)| std::fs::symlink_metadata(p).ok().map(|m| (p.clone(), fingerprint(&m))))
        .collect();
    Found {
        id: t.id.clone(),
        label: t.label.clone(),
        detail: t.detail.clone(),
        group: t.group,
        bytes: items.iter().map(|i| i.1).sum(),
        count: items.len(),
        needs_admin: t.needs_admin && !is_elevated(),
        delete_only: t.delete_only,
        examples: items.iter().take(3).map(|i| crate::scan::display_path(&i.0)).collect(),
        items,
        roots: t.roots.clone(),
        min_age: t.min_age,
        fingerprints,
    }
}

/// Build folders in projects nobody has touched for `stale_days`, found in an
/// existing scan: node_modules, Rust `target`, `.next` and similar. Each can be
/// recreated by the project's own build or install step.
pub fn stale_build_folders(tree: &Tree, stale_days: u64) -> Found {
    let mut items = Vec::new();
    let mut stack: Vec<NodeId> = vec![0];
    while let Some(id) = stack.pop() {
        let node = &tree.nodes[id as usize];
        let sibling = |name: &str| node.children.iter().any(|&c| &*tree.nodes[c as usize].name == name);
        for &c in &node.children {
            let child = &tree.nodes[c as usize];
            if child.flags & FLAG_DIR == 0 {
                continue;
            }
            let is_build = match &*child.name {
                "node_modules" => sibling("package.json"),
                "target" => sibling("Cargo.toml"),
                ".next" | ".nuxt" | ".svelte-kit" | ".turbo" | ".parcel-cache" | ".angular" => sibling("package.json"),
                "Pods" => sibling("Podfile"),
                ".gradle" | "build" => sibling("build.gradle") || sibling("build.gradle.kts"),
                _ => false,
            };
            if is_build {
                if child.size >= 1_000_000 {
                    items.push((c, child.size));
                }
                // Never look inside a build folder for more.
            } else if &*child.name != ".git" {
                stack.push(c);
            }
        }
    }
    let cutoff = Duration::from_secs(stale_days * 86_400);
    let mut out: Vec<(PathBuf, u64)> = Vec::new();
    for (id, size) in items {
        let path = PathBuf::from(tree.path(id));
        let Some(project) = path.parent() else { continue };
        if project_age(project, &path) >= cutoff {
            out.push((path, size));
        }
    }
    out.sort_by(|a, b| b.1.cmp(&a.1));
    let mut t = target("unused-builds", "Unused build folders",
        &format!("node_modules, target and similar in projects untouched for {stale_days} days; rebuilt by installing or building again"),
        "dev", Vec::new(), Duration::ZERO);
    t.kind = Kind::Whole;
    // Each item is its own root: only that folder may be removed.
    t.roots = out.iter().map(|i| i.0.clone()).collect();
    found(&t, out)
}

/// How long since anything at the top of a project was modified (ignoring the
/// build folder). Modified time only: checkouts and copies reset creation times.
fn project_age(project: &Path, skip: &Path) -> Duration {
    let mut newest = Duration::MAX;
    if let Ok(listing) = std::fs::read_dir(project) {
        for e in listing.flatten() {
            if e.path() == skip {
                continue;
            }
            if let Ok(m) = std::fs::symlink_metadata(e.path()) {
                let modified = m.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                newest = newest.min(SystemTime::now().duration_since(modified).unwrap_or(Duration::ZERO));
            }
        }
    }
    newest
}

/// True when `path` is strictly inside one of `roots` (or is a root, for build folders).
fn allowed(path: &Path, roots: &[PathBuf], kind: Kind) -> bool {
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return false;
    }
    roots.iter().any(|r| match kind {
        Kind::Contents => path.starts_with(r) && path != r.as_path(),
        Kind::Whole => path == r.as_path(),
    })
}

/// Remove the items of the given measured targets.
pub fn clean(found: &[Found], mode: Mode) -> Report {
    let mut report = Report::default();
    for f in found {
        let kind = if f.id == "unused-builds" { Kind::Whole } else { Kind::Contents };
        let mode = if f.delete_only { Mode::Delete } else { mode };
        for (path, size) in &f.items {
            let Ok(meta) = std::fs::symlink_metadata(path) else {
                report.skipped += 1; // already gone
                continue;
            };
            // Re-check: it may have been used or replaced since it was measured.
            let same = f.fingerprints.get(path) == Some(&fingerprint(&meta));
            if !same || !allowed(path, &f.roots, kind) || age(&meta) < f.min_age {
                report.skipped += 1;
                continue;
            }
            let result = match mode {
                Mode::Trash => trash::delete(path).map_err(|e| e.to_string()),
                Mode::Delete => {
                    if meta.is_dir() {
                        std::fs::remove_dir_all(path).map_err(|e| e.to_string())
                    } else {
                        std::fs::remove_file(path).map_err(|e| e.to_string())
                    }
                }
            };
            match result {
                Ok(()) => {
                    report.removed += 1;
                    report.freed += size;
                }
                Err(e) => {
                    if report.errors.len() < 20 {
                        report.errors.push(format!("{}: {e}", crate::scan::display_path(path)));
                    }
                    report.skipped += 1;
                }
            }
        }
    }
    report
}

/// Running as root / an elevated administrator.
#[cfg(unix)]
pub fn is_elevated() -> bool {
    unsafe { libc::geteuid() == 0 }
}

#[cfg(windows)]
pub fn is_elevated() -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            &mut elevation as *mut _ as *mut core::ffi::c_void,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        );
        CloseHandle(token);
        ok != 0 && elevation.TokenIsElevated != 0
    }
}
