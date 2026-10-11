//! The one interface the UI talks to. Every call takes a command name and JSON
//! arguments and returns JSON, so the desktop app and the `frisyscan serve`
//! development server expose exactly the same behaviour.

use crate::advisor::{self, Advisor, Backend};
use crate::benchmark::{self, BenchOptions};
use crate::clean::{self, Found, Mode, Places};
use crate::hidden::{self, Hidden};
use crate::facts;
use crate::scan::Scan;
use crate::settings::Settings;
use crate::tree::{NodeId, Tree, FLAG_DETACHED};
use crate::view;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Finished (or still-running) scans kept warm so leaving one and coming back
/// does not mean scanning it all over again. Bounds how much memory old scans
/// can hold onto; the oldest is evicted (and cancelled, if still running) once
/// a new one would push the cache over this.
const MAX_CACHED_SCANS: usize = 4;

pub struct Api {
    /// The scan the UI is currently looking at.
    scan: Mutex<Option<Arc<Scan>>>,
    /// Other scans kept warm, oldest first, so reopening one is instant.
    cache: Mutex<VecDeque<Arc<Scan>>>,
    /// Nodes staged in the collector, in the order they were added.
    collector: Mutex<Vec<NodeId>>,
    advisor: Advisor,
    settings: Mutex<Settings>,
    /// Hidden space for the open scan, worked out once it finishes.
    hidden: Mutex<Option<Option<Hidden>>>,
    /// The last cleanup measurement; `clean_run` only removes what it lists.
    cleanup: Mutex<Vec<Found>>,
    /// The most recent I/O benchmark report, folded into the advisor's facts.
    benchmark: Mutex<Option<String>>,
}

impl Default for Api {
    fn default() -> Api {
        Api {
            scan: Mutex::new(None),
            cache: Mutex::new(VecDeque::new()),
            collector: Mutex::new(Vec::new()),
            advisor: Advisor::default(),
            settings: Mutex::new(Settings::load()),
            hidden: Mutex::new(None),
            cleanup: Mutex::new(Vec::new()),
            benchmark: Mutex::new(None),
        }
    }
}

fn id_arg(args: &Value, key: &str) -> Result<NodeId, String> {
    args[key].as_u64().map(|v| v as NodeId).ok_or_else(|| format!("missing `{key}`"))
}

impl Api {
    pub fn new() -> Api {
        Api::default()
    }

    /// The folder the open scan started from.
    pub fn scan_root(&self) -> Option<String> {
        self.scan.lock().unwrap().as_ref().map(|s| s.root_path.clone())
    }

    fn scan(&self) -> Result<Arc<Scan>, String> {
        self.scan.lock().unwrap().clone().ok_or_else(|| "No scan is open.".to_string())
    }

    /// Run `f` on the tree, after checking that `id` exists.
    fn with_node<T>(&self, id: NodeId, f: impl FnOnce(&Tree) -> T) -> Result<T, String> {
        let scan = self.scan()?;
        let tree = scan.tree.lock().unwrap();
        if tree.get(id).is_none() {
            return Err(format!("No item {id}."));
        }
        Ok(f(&tree))
    }

    /// The same root path a scan of `path` would end up with, without starting one.
    /// Used to look a path up in the cache before deciding whether to scan it.
    fn root_of(&self, path: &str) -> String {
        std::fs::canonicalize(path).map(|p| crate::scan::display_path(&p)).unwrap_or_else(|_| path.to_string())
    }

    /// Take the active scan out, reattaching anything staged in the collector
    /// first so switching away from a scan never loses track of it, then keep
    /// the scan warm in the cache.
    fn stash_active(&self) {
        let Some(scan) = self.scan.lock().unwrap().take() else {
            self.collector.lock().unwrap().clear();
            return;
        };
        {
            let mut ids = self.collector.lock().unwrap();
            if !ids.is_empty() {
                let mut tree = scan.tree.lock().unwrap();
                for id in ids.drain(..).rev() {
                    tree.reattach(id);
                }
            }
        }
        self.cache_push(scan);
    }

    fn cache_take(&self, root: &str) -> Option<Arc<Scan>> {
        let mut cache = self.cache.lock().unwrap();
        let pos = cache.iter().position(|s| s.root_path == root)?;
        cache.remove(pos)
    }

    fn cache_remove(&self, root: &str) {
        self.cache.lock().unwrap().retain(|s| s.root_path != root);
    }

    /// Keep a scan warm so reopening it is instant. Evicts the oldest scan once
    /// the cache is over `MAX_CACHED_SCANS`; an evicted scan that is still
    /// running is cancelled, since nothing will be looking at it any more.
    fn cache_push(&self, scan: Arc<Scan>) {
        let mut cache = self.cache.lock().unwrap();
        cache.retain(|s| s.root_path != scan.root_path);
        cache.push_back(scan);
        while cache.len() > MAX_CACHED_SCANS {
            if let Some(evicted) = cache.pop_front() {
                evicted.cancel();
            }
        }
    }

    /// Open `path`. With a warm scan of the same folder and `force` false, that
    /// scan is reopened instead of scanning again — including one still running
    /// in the background after the UI left it. `force` always starts fresh.
    pub fn start_scan(&self, path: &str, force: bool) -> Result<Value, String> {
        if !std::path::Path::new(path).is_dir() {
            return Err(format!("{path} is not a folder."));
        }
        let root = self.root_of(path);
        self.stash_active();
        self.advisor.reset();
        *self.hidden.lock().unwrap() = None;

        if force {
            self.cache_remove(&root);
        } else if let Some(cached) = self.cache_take(&root) {
            let (size, files) = {
                let tree = cached.tree.lock().unwrap();
                (tree.nodes[0].size, tree.nodes[0].files)
            };
            let out = json!({ "root": cached.root_path, "cached": true, "finished": cached.is_finished(), "size": size, "files": files });
            *self.scan.lock().unwrap() = Some(cached);
            return Ok(out);
        }

        let scan = Scan::start(path);
        let root = scan.root_path.clone();
        let out = json!({ "root": root, "cached": false, "finished": scan.is_finished() });
        *self.scan.lock().unwrap() = Some(scan);
        Ok(out)
    }

    fn collector_json(&self, tree: &Tree) -> Value {
        let ids = self.collector.lock().unwrap();
        let items: Vec<Value> = ids
            .iter()
            .map(|&id| {
                let n = &tree.nodes[id as usize];
                json!({ "id": id, "name": n.name, "path": tree.path(id), "size": n.size })
            })
            .collect();
        let total: u64 = ids.iter().map(|&id| tree.nodes[id as usize].size).sum();
        json!({ "items": items, "size": total })
    }

    pub fn call(&self, cmd: &str, args: &Value) -> Result<Value, String> {
        let to_json = |v: &dyn erased::Ser| v.to_value();
        match cmd {
            // Each volume gains a `scan` field when a warm or active scan covers it,
            // so the Volumes screen can offer to reopen it instead of scanning again.
            "volumes" => {
                let vols = facts::volumes();
                let cache = self.cache.lock().unwrap();
                let active = self.scan.lock().unwrap().clone();
                let warm = |root: &str| -> Option<Value> {
                    let scan = cache.iter().find(|s| s.root_path == root).cloned().or_else(|| active.as_ref().filter(|s| s.root_path == root).cloned())?;
                    let (size, files) = {
                        let tree = scan.tree.lock().unwrap();
                        (tree.nodes[0].size, tree.nodes[0].files)
                    };
                    Some(json!({ "size": size, "files": files, "scanning": !scan.is_finished(), "age_seconds": scan.age_seconds() }))
                };
                let out: Vec<Value> = vols
                    .into_iter()
                    .map(|v| {
                        let mut j = serde_json::to_value(&v).unwrap_or(Value::Null);
                        if let Some(scan) = warm(&self.root_of(&v.mount)) {
                            j["scan"] = scan;
                        }
                        j
                    })
                    .collect();
                Ok(json!(out))
            }
            "home" => Ok(json!(std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_default())),
            "platform" => Ok(json!(std::env::consts::OS)),
            "start_scan" => self.start_scan(args["path"].as_str().ok_or("missing `path`")?, args["force"].as_bool().unwrap_or(false)),
            "stop_scan" => {
                self.scan()?.cancel();
                Ok(Value::Null)
            }
            "close_scan" => {
                self.stash_active();
                self.advisor.reset();
                Ok(Value::Null)
            }
            "status" => {
                let Some(scan) = self.scan.lock().unwrap().clone() else { return Ok(json!({ "open": false })) };
                let size = scan.tree.lock().unwrap().nodes[0].size;
                Ok(json!({ "open": true, "root": scan.root_path, "size": size, "progress": scan.progress() }))
            }
            // Everything a screen needs in one round trip.
            "view" => {
                let focus = id_arg(args, "focus")?;
                let (cfg_depth, cfg_min) = {
                    let s = self.settings.lock().unwrap();
                    (s.scan_depth, s.scan_min_fraction)
                };
                let depth = args["depth"].as_u64().map(|v| v as u32).unwrap_or(cfg_depth);
                let min = args["min_fraction"].as_f64().unwrap_or(cfg_min);
                let limit = args["rows"].as_u64().unwrap_or(500) as usize;
                let scan = self.scan()?;
                let tree = scan.tree.lock().unwrap();
                let node = tree.get(focus).ok_or("No such folder.")?;
                if !node.is_dir() || node.flags & FLAG_DETACHED != 0 {
                    return Err("That folder is no longer in view.".into());
                }
                let size = tree.nodes[0].size;
                drop(tree);
                // Hidden space only makes sense for a finished scan of a whole volume.
                let hidden_space = if focus == 0 && scan.is_finished() {
                    let mut cached = self.hidden.lock().unwrap();
                    cached.get_or_insert_with(|| hidden::hidden_space(&scan.root_path, size)).clone()
                } else {
                    None
                };
                let tree = scan.tree.lock().unwrap();
                Ok(json!({
                    "hidden": hidden_space,
                    "tree": view::view(&tree, focus, depth, min),
                    "rows": view::rows(&tree, focus, limit),
                    "crumbs": view::crumbs(&tree, focus),
                    "path": tree.path(focus),
                    "collector": self.collector_json(&tree),
                    "progress": scan.progress(),
                }))
            }
            "largest" => {
                let focus = id_arg(args, "focus")?;
                let limit = args["limit"].as_u64().unwrap_or(200) as usize;
                self.with_node(focus, |t| to_json(&view::largest(t, focus, limit)))
            }
            "search" => {
                let focus = id_arg(args, "focus")?;
                let q = args["query"].as_str().unwrap_or("").to_string();
                self.with_node(focus, |t| to_json(&view::search(t, focus, &q, 200)))
            }
            "types" => {
                let focus = id_arg(args, "focus")?;
                self.with_node(focus, |t| to_json(&view::types(t, focus)))
            }
            "path" => {
                let id = id_arg(args, "id")?;
                self.with_node(id, |t| json!(t.path(id)))
            }
            "reveal" => {
                let id = id_arg(args, "id")?;
                let path = self.with_node(id, |t| t.path(id))?;
                reveal(&path)
            }
            "open" => {
                let id = id_arg(args, "id")?;
                let path = self.with_node(id, |t| t.path(id))?;
                open(&path)
            }
            "open_privacy_settings" => {
                #[cfg(target_os = "macos")]
                let _ = std::process::Command::new("open")
                    .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles")
                    .spawn();
                Ok(Value::Null)
            }

            // Collector: staging changes nothing on disk.
            "stage" => {
                let id = id_arg(args, "id")?;
                let scan = self.scan()?;
                if !scan.is_finished() {
                    return Err("Wait for the scan to finish before collecting items.".into());
                }
                let mut tree = scan.tree.lock().unwrap();
                if !tree.detach(id) {
                    return Err("That item cannot be collected.".into());
                }
                self.collector.lock().unwrap().push(id);
                let parent = tree.nodes[id as usize].parent;
                Ok(json!({ "parent": parent, "collector": self.collector_json(&tree) }))
            }
            "unstage" => {
                let scan = self.scan()?;
                let mut tree = scan.tree.lock().unwrap();
                let mut ids = self.collector.lock().unwrap();
                let wanted = args["id"].as_u64().map(|v| v as NodeId);
                // Put back in reverse order so nested items land in the right place.
                let mut keep = Vec::new();
                for &id in ids.clone().iter().rev() {
                    if wanted.is_none() || wanted == Some(id) {
                        tree.reattach(id);
                    } else {
                        keep.push(id);
                    }
                }
                keep.reverse();
                *ids = keep;
                drop(ids);
                Ok(self.collector_json(&tree))
            }
            "trash_collector" => {
                let scan = self.scan()?;
                let staged: Vec<(NodeId, String, u64)> = {
                    let tree = scan.tree.lock().unwrap();
                    self.collector.lock().unwrap().iter().map(|&id| (id, tree.path(id), tree.nodes[id as usize].size)).collect()
                };
                let mut moved = 0u64;
                let mut freed = 0u64;
                let mut failed: Vec<String> = Vec::new();
                let mut back = Vec::new();
                for (id, path, size) in staged {
                    match trash::delete(&path) {
                        Ok(()) => {
                            moved += 1;
                            freed += size;
                        }
                        Err(e) => {
                            failed.push(format!("{path}: {e}"));
                            back.push(id);
                        }
                    }
                }
                let mut tree = scan.tree.lock().unwrap();
                for id in back.into_iter().rev() {
                    tree.reattach(id);
                }
                self.collector.lock().unwrap().clear();
                Ok(json!({ "moved": moved, "freed": freed, "failed": failed }))
            }

            // Cleanup
            "elevated" => Ok(json!(clean::is_elevated())),
            "clean_scan" => {
                let places = Places::of_this_machine();
                let targets = clean::targets(&places, std::env::consts::OS);
                let mut found: Vec<Found> = std::thread::scope(|s| {
                    let handles: Vec<_> = targets.iter().map(|t| s.spawn(move || clean::measure(t))).collect();
                    handles.into_iter().filter_map(|h| h.join().ok()).collect()
                });
                let days = args["stale_days"].as_u64().unwrap_or_else(|| self.settings.lock().unwrap().stale_days);
                if let Some(scan) = self.scan.lock().unwrap().clone().filter(|s| s.is_finished()) {
                    let tree = scan.tree.lock().unwrap();
                    found.push(clean::stale_build_folders(&tree, days));
                }
                found.retain(|f| f.count > 0);
                found.sort_by(|a, b| b.bytes.cmp(&a.bytes));
                let shown = to_json(&found);
                *self.cleanup.lock().unwrap() = found;
                Ok(json!({ "targets": shown, "elevated": clean::is_elevated(), "scan_open": self.scan.lock().unwrap().is_some() }))
            }
            "clean_run" => {
                let ids: Vec<String> = args["ids"].as_array().ok_or("missing `ids`")?.iter().filter_map(|v| v.as_str().map(String::from)).collect();
                let mode = match args["mode"].as_str() {
                    Some("delete") => Mode::Delete,
                    Some("trash") => Mode::Trash,
                    _ => return Err("`mode` must be trash or delete".into()),
                };
                let chosen: Vec<Found> = self.cleanup.lock().unwrap().iter().filter(|f| ids.contains(&f.id)).cloned().collect();
                if chosen.is_empty() {
                    return Err("Nothing selected. Measure again and pick what to clean.".into());
                }
                let report = clean::clean(&chosen, mode);
                self.cleanup.lock().unwrap().clear();
                Ok(to_json(&report))
            }

            // Advisor
            "backends" => {
                let settings = self.settings.lock().unwrap().clone();
                Ok(to_json(&advisor::backends(&settings)))
            }
            "settings" => Ok(self.settings.lock().unwrap().public()),
            "set_settings" => {
                let mut settings = self.settings.lock().unwrap();
                settings.apply(args);
                settings.save()?;
                Ok(settings.public())
            }
            // Check one provider with the values typed in the form (not saved), else the saved ones.
            "test_provider" => {
                let kind = args["kind"].as_str().ok_or("missing kind")?.to_string();
                let mut settings = self.settings.lock().unwrap().clone();
                settings.apply(&args["settings"]);
                Ok(to_json(&advisor::test_provider(&settings, &kind)))
            }
            "benchmark" => {
                let path = args["path"].as_str().ok_or("missing `path`")?;
                let cfg = self.settings.lock().unwrap().clone();
                let opts = BenchOptions {
                    total_bytes: args["total_bytes"].as_u64().unwrap_or(cfg.bench_total_bytes),
                    block_bytes: args["block_bytes"].as_u64().unwrap_or(cfg.bench_block_bytes),
                    random_ops: args["random_ops"].as_u64().map(|n| n as u32).unwrap_or(cfg.bench_random_ops),
                    write: args["write"].as_bool().unwrap_or(cfg.bench_write),
                };
                let result = benchmark::run(std::path::Path::new(path), &opts)?;
                // Keep a plain-text version for the advisor to reason over.
                let mut text = format!(
                    "## I/O benchmark of {}\nSequential read: {:.0} MB/s",
                    result.path, result.read_mbps
                );
                if let Some(w) = result.write_mbps {
                    text.push_str(&format!(", sequential write: {w:.0} MB/s"));
                }
                text.push_str(&format!(
                    ".\nRandom read: {:.0} IOPS at {:.0} us mean latency.\nLikely profile: {}.",
                    result.random_iops, result.random_latency_us, benchmark::classify(&result)
                ));
                *self.benchmark.lock().unwrap() = Some(text);
                Ok(to_json(&result))
            }
            "advisor_ask" => {
                let backend: Backend = serde_json::from_value(args["backend"].clone()).map_err(|_| "Pick a model first.")?;
                let follow_up = args["follow_up"].as_str().map(String::from);
                let scan = self.scan.lock().unwrap().clone();
                let focus = args["focus"].as_u64().unwrap_or(0) as NodeId;
                let settings = self.settings.lock().unwrap().clone();
                let bench = self.benchmark.lock().unwrap().clone();
                if !settings.advisor_enabled {
                    return Err("The advisor is turned off in settings.".into());
                }
                self.advisor.ask(settings, backend, follow_up, move || {
                    let mut machine = facts::machine_report();
                    if let Some(b) = bench {
                        machine.push_str("\n\n");
                        machine.push_str(&b);
                    }
                    let report = scan.filter(|s| s.is_finished()).and_then(|s| {
                        let tree = s.tree.lock().unwrap();
                        tree.get(focus).map(|_| facts::scan_report(&tree, focus))
                    });
                    advisor::user_prompt(&machine, report.as_deref())
                })?;
                Ok(Value::Null)
            }
            "advisor_poll" => {
                let s = self.advisor.session.lock().unwrap();
                // The system prompt and the long facts message are not shown.
                let shown: Vec<&advisor::Message> = s.messages.iter().skip(2).collect();
                let risky: Vec<String> = if s.running {
                    Vec::new()
                } else {
                    shown.iter().filter(|m| m.role == "assistant").flat_map(|m| advisor::destructive_lines(&m.text)).collect()
                };
                Ok(json!({ "messages": shown, "running": s.running, "error": s.error, "risky": risky, "started": !s.messages.is_empty() || s.running }))
            }
            "advisor_stop" => {
                self.advisor.stop();
                Ok(Value::Null)
            }
            "advisor_reset" => {
                self.advisor.reset();
                Ok(Value::Null)
            }
            other => Err(format!("unknown command `{other}`")),
        }
    }
}

mod erased {
    /// Lets `call` turn any serialisable value into JSON through one closure.
    pub trait Ser {
        fn to_value(&self) -> serde_json::Value;
    }
    impl<T: serde::Serialize> Ser for T {
        fn to_value(&self) -> serde_json::Value {
            serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
        }
    }
}

/// Show an item in Finder / Explorer.
fn reveal(path: &str) -> Result<Value, String> {
    let mut cmd;
    if cfg!(target_os = "macos") {
        cmd = std::process::Command::new("open");
        cmd.args(["-R", path]);
    } else if cfg!(windows) {
        cmd = std::process::Command::new("explorer");
        cmd.arg(format!("/select,{path}"));
    } else {
        cmd = std::process::Command::new("xdg-open");
        cmd.arg(std::path::Path::new(path).parent().unwrap_or(std::path::Path::new("/")));
    }
    cmd.spawn().map(|_| Value::Null).map_err(|e| e.to_string())
}

/// Preview a file: Quick Look on macOS, the default app elsewhere.
fn open(path: &str) -> Result<Value, String> {
    let mut cmd;
    if cfg!(target_os = "macos") {
        cmd = std::process::Command::new("qlmanage");
        cmd.args(["-p", path]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    } else if cfg!(windows) {
        cmd = std::process::Command::new("explorer");
        cmd.arg(path);
    } else {
        cmd = std::process::Command::new("xdg-open");
        cmd.arg(path);
    }
    cmd.spawn().map(|_| Value::Null).map_err(|e| e.to_string())
}
