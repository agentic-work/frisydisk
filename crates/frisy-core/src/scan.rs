//! Parallel directory scanner.
//!
//! Sizes are bytes on disk where the platform reports them cheaply (macOS and
//! other Unix systems); on Windows they are logical file sizes, with cloud-only
//! placeholder files counted as zero. Hard links are counted once on Unix.
//! The scan stays on the volume it started on and never follows symlinks,
//! junctions or mount points.

use crate::tree::{Entry, NodeId, Tree, FLAG_INACCESSIBLE};
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

/// What a platform reader hands back for one directory.
pub(crate) struct RawEntry {
    pub name: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    /// (device, inode) for a file with more than one hard link.
    pub link: Option<(u64, u64)>,
}

pub(crate) enum ReadError {
    /// Permission problem or similar: worth telling the person about.
    Denied,
    /// Not ours to scan (another volume, vanished, cloud-only): skip quietly.
    Skip,
}

/// Volume boundaries for one scan.
#[cfg_attr(windows, allow(dead_code))]
pub(crate) struct Bounds {
    pub devices: Vec<u64>,
    pub skip_paths: Vec<PathBuf>,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct Progress {
    pub files: u64,
    pub directories: u64,
    pub bytes: u64,
    pub inaccessible: u64,
    pub seconds: f64,
    pub scanning: bool,
}

struct Queue {
    jobs: Vec<(NodeId, PathBuf)>,
    active: usize,
}

pub struct Scan {
    pub root_path: String,
    pub tree: Mutex<Tree>,
    queue: Mutex<Queue>,
    wake: Condvar,
    cancelled: AtomicBool,
    finished: AtomicBool,
    files: AtomicU64,
    directories: AtomicU64,
    bytes: AtomicU64,
    inaccessible: AtomicU64,
    links: Mutex<HashSet<(u64, u64)>>,
    bounds: Bounds,
    started: Instant,
    elapsed: Mutex<Option<f64>>,
}

impl Scan {
    /// Start scanning `path` on background threads and return at once.
    pub fn start(path: &str) -> Arc<Scan> {
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).max(2);
        Scan::start_with(path, threads)
    }

    pub fn start_with(path: &str, threads: usize) -> Arc<Scan> {
        let root = std::fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path));
        let root_str = display_path(&root);
        let scan = Arc::new(Scan {
            root_path: root_str.clone(),
            tree: Mutex::new(Tree::new(&root_str)),
            queue: Mutex::new(Queue { jobs: vec![(Tree::ROOT, root.clone())], active: 0 }),
            wake: Condvar::new(),
            cancelled: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            files: AtomicU64::new(0),
            directories: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            inaccessible: AtomicU64::new(0),
            links: Mutex::new(HashSet::new()),
            bounds: platform::bounds(&root),
            started: Instant::now(),
            elapsed: Mutex::new(None),
        });
        let coordinator = scan.clone();
        std::thread::spawn(move || {
            let workers: Vec<_> = (0..threads)
                .map(|_| {
                    let s = coordinator.clone();
                    std::thread::spawn(move || s.worker())
                })
                .collect();
            for w in workers {
                let _ = w.join();
            }
            coordinator.tree.lock().unwrap().sort_all();
            *coordinator.elapsed.lock().unwrap() = Some(coordinator.started.elapsed().as_secs_f64());
            coordinator.finished.store(true, Ordering::SeqCst);
        });
        scan
    }

    /// Scan `path` and block until it is done.
    pub fn run(path: &str) -> Arc<Scan> {
        let scan = Scan::start(path);
        scan.wait();
        scan
    }

    pub fn wait(&self) {
        while !self.is_finished() {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::SeqCst)
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.wake.notify_all();
    }

    pub fn progress(&self) -> Progress {
        let done = *self.elapsed.lock().unwrap();
        Progress {
            files: self.files.load(Ordering::Relaxed),
            directories: self.directories.load(Ordering::Relaxed),
            bytes: self.bytes.load(Ordering::Relaxed),
            inaccessible: self.inaccessible.load(Ordering::Relaxed),
            seconds: done.unwrap_or_else(|| self.started.elapsed().as_secs_f64()),
            scanning: !self.is_finished(),
        }
    }

    fn worker(&self) {
        let mut reader = platform::Reader::new();
        loop {
            let (node, path) = {
                let mut q = self.queue.lock().unwrap();
                loop {
                    if self.cancelled.load(Ordering::SeqCst) {
                        self.wake.notify_all();
                        return;
                    }
                    if let Some(job) = q.jobs.pop() {
                        q.active += 1;
                        break job;
                    }
                    if q.active == 0 {
                        self.wake.notify_all();
                        return;
                    }
                    q = self.wake.wait(q).unwrap();
                }
            };

            let subdirs = self.scan_directory(&mut reader, node, &path);

            let mut q = self.queue.lock().unwrap();
            q.jobs.extend(subdirs);
            q.active -= 1;
            drop(q);
            self.wake.notify_all();
        }
    }

    fn scan_directory(&self, reader: &mut platform::Reader, node: NodeId, path: &Path) -> Vec<(NodeId, PathBuf)> {
        let raw = match reader.read(path, &self.bounds) {
            Ok(raw) => raw,
            Err(ReadError::Denied) => {
                self.tree.lock().unwrap().nodes[node as usize].flags |= FLAG_INACCESSIBLE;
                self.inaccessible.fetch_add(1, Ordering::Relaxed);
                return Vec::new();
            }
            Err(ReadError::Skip) => return Vec::new(),
        };

        let mut bytes = 0u64;
        let mut files = 0u64;
        let mut dir_names = Vec::new();
        let mut entries = Vec::with_capacity(raw.len());
        {
            let mut links = None;
            for r in raw {
                let mut size = r.size;
                if let Some(key) = r.link {
                    let set = links.get_or_insert_with(|| self.links.lock().unwrap());
                    if !set.insert(key) {
                        size = 0;
                    }
                }
                if r.is_dir {
                    dir_names.push(r.name.clone());
                } else {
                    bytes += size;
                    files += 1;
                }
                entries.push(Entry { name: r.name, is_dir: r.is_dir, is_symlink: r.is_symlink, size });
            }
        }

        let dir_ids = self.tree.lock().unwrap().attach(node, entries);
        self.files.fetch_add(files, Ordering::Relaxed);
        self.bytes.fetch_add(bytes, Ordering::Relaxed);
        self.directories.fetch_add(1, Ordering::Relaxed);
        dir_ids.into_iter().zip(dir_names).map(|(id, name)| (id, path.join(name))).collect()
    }
}

/// A path as text, without the `\\?\` prefix Windows adds when canonicalising.
pub fn display_path(p: &Path) -> String {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC") => rest.to_string(),
        _ => s.into_owned(),
    }
}

#[cfg(target_os = "macos")]
#[path = "scan_macos.rs"]
mod platform;

#[cfg(not(target_os = "macos"))]
#[path = "scan_portable.rs"]
mod platform;
