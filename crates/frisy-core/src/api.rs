//! The one interface the UI talks to. Every call takes a command name and JSON
//! arguments and returns JSON, so the desktop app and the `frisyscan serve`
//! development server expose exactly the same behaviour.

use crate::advisor::{self, Advisor, Backend};
use crate::facts;
use crate::scan::Scan;
use crate::settings::Settings;
use crate::tree::{NodeId, Tree, FLAG_DETACHED};
use crate::view;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

pub struct Api {
    scan: Mutex<Option<Arc<Scan>>>,
    /// Nodes staged in the collector, in the order they were added.
    collector: Mutex<Vec<NodeId>>,
    advisor: Advisor,
    settings: Mutex<Settings>,
}

impl Default for Api {
    fn default() -> Api {
        Api { scan: Mutex::new(None), collector: Mutex::new(Vec::new()), advisor: Advisor::default(), settings: Mutex::new(Settings::load()) }
    }
}

fn id_arg(args: &Value, key: &str) -> Result<NodeId, String> {
    args[key].as_u64().map(|v| v as NodeId).ok_or_else(|| format!("missing `{key}`"))
}

impl Api {
    pub fn new() -> Api {
        Api::default()
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

    pub fn start_scan(&self, path: &str) -> Result<Value, String> {
        if !std::path::Path::new(path).is_dir() {
            return Err(format!("{path} is not a folder."));
        }
        if let Some(old) = self.scan.lock().unwrap().take() {
            old.cancel();
        }
        self.collector.lock().unwrap().clear();
        self.advisor.reset();
        let scan = Scan::start(path);
        let root = scan.root_path.clone();
        *self.scan.lock().unwrap() = Some(scan);
        Ok(json!({ "root": root }))
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
            "volumes" => Ok(to_json(&facts::volumes())),
            "home" => Ok(json!(std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_default())),
            "platform" => Ok(json!(std::env::consts::OS)),
            "start_scan" => self.start_scan(args["path"].as_str().ok_or("missing `path`")?),
            "stop_scan" => {
                self.scan()?.cancel();
                Ok(Value::Null)
            }
            "close_scan" => {
                if let Some(s) = self.scan.lock().unwrap().take() {
                    s.cancel();
                }
                self.collector.lock().unwrap().clear();
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
                let depth = args["depth"].as_u64().unwrap_or(6) as u32;
                let min = args["min_fraction"].as_f64().unwrap_or(0.004);
                let limit = args["rows"].as_u64().unwrap_or(500) as usize;
                let scan = self.scan()?;
                let tree = scan.tree.lock().unwrap();
                let node = tree.get(focus).ok_or("No such folder.")?;
                if !node.is_dir() || node.flags & FLAG_DETACHED != 0 {
                    return Err("That folder is no longer in view.".into());
                }
                Ok(json!({
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
            "advisor_ask" => {
                let backend: Backend = serde_json::from_value(args["backend"].clone()).map_err(|_| "Pick a model first.")?;
                let follow_up = args["follow_up"].as_str().map(String::from);
                let scan = self.scan.lock().unwrap().clone();
                let focus = args["focus"].as_u64().unwrap_or(0) as NodeId;
                let settings = self.settings.lock().unwrap().clone();
                if !settings.advisor_enabled {
                    return Err("The advisor is turned off in settings.".into());
                }
                self.advisor.ask(settings, backend, follow_up, move || {
                    let machine = facts::machine_report();
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
