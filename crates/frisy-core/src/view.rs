//! Read-only queries over a scanned tree: the pruned view the charts draw,
//! list rows, largest files, search and the file-type breakdown.

use crate::tree::{NodeId, Tree, FLAG_INACCESSIBLE, FLAG_SYMLINK};
use serde::Serialize;
use std::collections::HashMap;

/// A node of the pruned tree sent to the UI. Children too small to draw are
/// folded into one group node (`id` is `None`, `group` is how many it holds).
#[derive(Serialize, Debug)]
pub struct ViewNode {
    pub id: Option<NodeId>,
    pub name: String,
    pub size: u64,
    pub files: u64,
    pub dir: bool,
    pub group: u32,
    pub children: Vec<ViewNode>,
}

/// Prune the subtree under `focus` to `max_depth` levels, folding children
/// smaller than `min_fraction` of the focus into a group.
pub fn view(tree: &Tree, focus: NodeId, max_depth: u32, min_fraction: f64) -> ViewNode {
    let total = tree.nodes[focus as usize].size.max(1) as f64;
    let min_size = (total * min_fraction) as u64;
    build(tree, focus, max_depth, min_size, true)
}

fn build(tree: &Tree, id: NodeId, depth_left: u32, min_size: u64, is_focus: bool) -> ViewNode {
    let n = &tree.nodes[id as usize];
    let mut children = Vec::new();
    if n.is_dir() && depth_left > 0 {
        let mut group_size = 0u64;
        let mut group_files = 0u64;
        let mut group_count = 0u32;
        for c in tree.sorted_children(id) {
            let cn = &tree.nodes[c as usize];
            if cn.size == 0 {
                continue;
            }
            if cn.size < min_size {
                group_size += cn.size;
                group_files += cn.files;
                group_count += 1;
            } else {
                children.push(build(tree, c, depth_left - 1, min_size, false));
            }
        }
        if group_count > 0 {
            children.push(ViewNode {
                id: None,
                name: format!("{group_count} smaller items"),
                size: group_size,
                files: group_files,
                dir: false,
                group: group_count,
                children: Vec::new(),
            });
        }
    }
    ViewNode {
        id: Some(id),
        name: if is_focus { tree.display_name(id) } else { n.name.to_string() },
        size: n.size,
        files: n.files,
        dir: n.is_dir(),
        group: 0,
        children,
    }
}

/// One line of a list in the side panel.
#[derive(Serialize, Debug, Clone)]
pub struct Row {
    pub id: NodeId,
    pub name: String,
    /// Folder the item is in; only filled for search and largest-file results.
    pub parent_path: Option<String>,
    pub size: u64,
    pub files: u64,
    pub dir: bool,
    pub symlink: bool,
    pub inaccessible: bool,
}

fn row(tree: &Tree, id: NodeId, with_parent: bool) -> Row {
    let n = &tree.nodes[id as usize];
    Row {
        id,
        name: n.name.to_string(),
        parent_path: (with_parent && n.parent != crate::tree::NO_PARENT).then(|| tree.path(n.parent)),
        size: n.size,
        files: n.files,
        dir: n.is_dir(),
        symlink: n.flags & FLAG_SYMLINK != 0,
        inaccessible: n.flags & FLAG_INACCESSIBLE != 0,
    }
}

/// Direct children of `focus`, largest first.
pub fn rows(tree: &Tree, focus: NodeId, limit: usize) -> Vec<Row> {
    tree.sorted_children(focus).into_iter().take(limit).map(|c| row(tree, c, false)).collect()
}

#[derive(Serialize, Debug)]
pub struct Crumb {
    pub id: NodeId,
    pub name: String,
}

pub fn crumbs(tree: &Tree, focus: NodeId) -> Vec<Crumb> {
    tree.chain(focus).into_iter().map(|id| Crumb { id, name: tree.display_name(id) }).collect()
}

/// The `limit` largest files at or below `focus`.
pub fn largest(tree: &Tree, focus: NodeId, limit: usize) -> Vec<Row> {
    let mut best: Vec<(u64, NodeId)> = Vec::new();
    let mut floor = 0u64;
    tree.walk_files(focus, |id, n| {
        if n.size > floor || best.len() < limit {
            best.push((n.size, id));
            if best.len() >= limit * 4 {
                best.sort_unstable_by(|a, b| b.cmp(a));
                best.truncate(limit);
                floor = best.last().map(|b| b.0).unwrap_or(0);
            }
        }
    });
    best.sort_unstable_by(|a, b| b.cmp(a));
    best.truncate(limit);
    best.into_iter().map(|(_, id)| row(tree, id, true)).collect()
}

/// Files and folders under `focus` whose name contains `query`, largest first.
pub fn search(tree: &Tree, focus: NodeId, query: &str, limit: usize) -> Vec<Row> {
    let q = query.to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<(u64, NodeId)> = Vec::new();
    let mut stack = vec![focus];
    while let Some(cur) = stack.pop() {
        for &c in &tree.nodes[cur as usize].children {
            let cn = &tree.nodes[c as usize];
            if cn.name.to_lowercase().contains(&q) {
                hits.push((cn.size, c));
            }
            if cn.is_dir() {
                stack.push(c);
            }
        }
    }
    hits.sort_unstable_by(|a, b| b.cmp(a));
    hits.truncate(limit);
    hits.into_iter().map(|(_, id)| row(tree, id, true)).collect()
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct TypeBucket {
    pub category: String,
    pub bytes: u64,
    pub count: u64,
}

/// Bytes and file counts per broad file category, largest first.
pub fn types(tree: &Tree, focus: NodeId) -> Vec<TypeBucket> {
    let mut buckets: HashMap<&'static str, (u64, u64)> = HashMap::new();
    tree.walk_files(focus, |_, n| {
        let b = buckets.entry(category(&n.name)).or_default();
        b.0 += n.size;
        b.1 += 1;
    });
    let mut out: Vec<TypeBucket> =
        buckets.into_iter().map(|(c, (bytes, count))| TypeBucket { category: c.to_string(), bytes, count }).collect();
    out.sort_by(|a, b| b.bytes.cmp(&a.bytes).then(a.category.cmp(&b.category)));
    out
}

pub fn category(name: &str) -> &'static str {
    let ext = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => ext.to_lowercase(),
        _ => return "No extension",
    };
    match ext.as_str() {
        "mp4" | "mov" | "mkv" | "avi" | "m4v" | "webm" | "mxf" | "braw" | "r3d" | "wmv" | "mts" => "Video",
        "mp3" | "wav" | "aiff" | "aif" | "flac" | "m4a" | "aac" | "ogg" | "caf" | "opus" | "wma" => "Audio",
        "jpg" | "jpeg" | "png" | "heic" | "tiff" | "tif" | "gif" | "raw" | "cr2" | "cr3" | "nef" | "arw" | "dng"
        | "psd" | "webp" | "bmp" | "svg" => "Images",
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "pages" | "numbers" | "key" | "txt" | "md"
        | "rtf" | "epub" => "Documents",
        "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "zst" | "7z" | "rar" | "pkg" | "xip" | "cab" | "msi" => "Archives",
        "dmg" | "iso" | "img" | "sparseimage" | "sparsebundle" | "qcow2" | "vmdk" | "vdi" | "vhd" | "vhdx" => "Disk images",
        "swift" | "c" | "h" | "cpp" | "m" | "mm" | "js" | "ts" | "tsx" | "jsx" | "py" | "go" | "rs" | "java" | "rb"
        | "sh" | "json" | "yaml" | "yml" | "html" | "css" | "map" | "cs" | "ps1" => "Code",
        "dylib" | "so" | "a" | "o" | "node" | "wasm" | "exe" | "dll" | "jar" | "lib" | "pdb" | "rlib" | "rmeta" => "Binaries",
        "db" | "sqlite" | "sqlite3" | "sqlite-wal" | "mdb" | "ldb" | "realm" => "Databases",
        "gguf" | "safetensors" | "bin" | "pt" | "pth" | "onnx" | "mlmodel" | "ckpt" => "ML models",
        "log" | "cache" | "tmp" | "pack" | "idx" | "etl" | "dmp" => "Logs and caches",
        _ => "Other",
    }
}

/// Decimal units, the convention Finder and Explorer's drive view use.
pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 6] = ["bytes", "KB", "MB", "GB", "TB", "PB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1000.0 && i < UNITS.len() - 1 {
        v /= 1000.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} bytes")
    } else if v >= 100.0 {
        format!("{v:.0} {}", UNITS[i])
    } else if v >= 10.0 {
        format!("{v:.1} {}", UNITS[i])
    } else {
        format!("{v:.2} {}", UNITS[i])
    }
}

/// 1234567 -> "1,234,567"
pub fn count(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}
