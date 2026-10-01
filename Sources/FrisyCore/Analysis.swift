import Foundation

public struct TypeBucket: Sendable, Identifiable {
    public let category: String
    public var bytes: Int64
    public var count: Int
    public var id: String { category }
}

/// Read-only queries over a scanned tree. Each takes the tree lock.
public enum Analysis {
    /// The `limit` largest files at or below `root`.
    public static func largestFiles(in tree: ScanTree, under root: FileNode, limit: Int = 100) -> [FileNode] {
        tree.withLock {
            var best: [FileNode] = []
            var floor: Int64 = 0
            walkFiles(root) { f in
                guard f.size > floor || best.count < limit else { return }
                best.append(f)
                if best.count >= limit * 4 {
                    best.sort { $0.size > $1.size }
                    best.removeLast(best.count - limit)
                    floor = best.last?.size ?? 0
                }
            }
            best.sort { $0.size > $1.size }
            return Array(best.prefix(limit))
        }
    }

    /// Files and folders whose name contains `query` (case-insensitive), largest first.
    public static func search(in tree: ScanTree, under root: FileNode, query: String, limit: Int = 200) -> [FileNode] {
        let q = query.lowercased()
        guard !q.isEmpty else { return [] }
        return tree.withLock {
            var hits: [FileNode] = []
            var stack = [root]
            while let n = stack.popLast() {
                for c in n.children {
                    if c.name.lowercased().contains(q) { hits.append(c) }
                    if c.isDirectory { stack.append(c) }
                }
            }
            hits.sort { $0.size > $1.size }
            return Array(hits.prefix(limit))
        }
    }

    /// Bytes and file counts per broad file category, largest first.
    public static func typeBreakdown(in tree: ScanTree, under root: FileNode) -> [TypeBucket] {
        tree.withLock {
            var buckets: [String: TypeBucket] = [:]
            walkFiles(root) { f in
                let cat = category(forExtension: f.fileExtension)
                buckets[cat, default: TypeBucket(category: cat, bytes: 0, count: 0)].bytes += f.size
                buckets[cat]!.count += 1
            }
            return buckets.values.sorted { $0.bytes > $1.bytes }
        }
    }

    private static func walkFiles(_ root: FileNode, _ visit: (FileNode) -> Void) {
        if !root.isDirectory {
            visit(root)
            return
        }
        var stack = [root]
        while let n = stack.popLast() {
            for c in n.children {
                if c.isDirectory { stack.append(c) } else { visit(c) }
            }
        }
    }

    public static func category(forExtension ext: String) -> String {
        categoryByExtension[ext] ?? (ext.isEmpty ? "No extension" : "Other")
    }

    private static let categoryByExtension: [String: String] = {
        let groups: [String: [String]] = [
            "Video": ["mp4", "mov", "mkv", "avi", "m4v", "webm", "mxf", "prores", "braw", "r3d", "wmv", "mts"],
            "Audio": ["mp3", "wav", "aiff", "aif", "flac", "m4a", "aac", "ogg", "caf", "alac", "opus"],
            "Images": ["jpg", "jpeg", "png", "heic", "tiff", "tif", "gif", "raw", "cr2", "cr3", "nef", "arw", "dng", "psd", "webp", "bmp", "svg"],
            "Documents": ["pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "pages", "numbers", "key", "txt", "md", "rtf", "epub"],
            "Archives": ["zip", "tar", "gz", "tgz", "bz2", "xz", "zst", "7z", "rar", "pkg", "xip"],
            "Disk images": ["dmg", "iso", "img", "sparseimage", "sparsebundle", "qcow2", "vmdk", "vdi"],
            "Code": ["swift", "c", "h", "cpp", "m", "mm", "js", "ts", "tsx", "jsx", "py", "go", "rs", "java", "rb", "sh", "json", "yaml", "yml", "html", "css", "map"],
            "Binaries": ["dylib", "so", "a", "o", "node", "wasm", "exe", "dll", "jar", "framework"],
            "Databases": ["db", "sqlite", "sqlite3", "sqlite-wal", "mdb", "ldb", "realm"],
            "ML models": ["gguf", "safetensors", "bin", "pt", "pth", "onnx", "mlmodel", "mlpackage", "ckpt"],
            "Logs & caches": ["log", "cache", "tmp", "pack", "idx"],
        ]
        var map: [String: String] = [:]
        for (cat, exts) in groups {
            for e in exts { map[e] = cat }
        }
        return map
    }()
}
