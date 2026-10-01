import Foundation

/// One file or directory in a scanned tree.
///
/// `name`, `isDirectory` and `isSymlink` never change. `size`, `fileCount`,
/// `children` and `parent` are only read or written while holding the owning
/// `ScanTree`'s lock (or after the scan has finished, from one thread).
public final class FileNode: @unchecked Sendable {
    public let name: String
    public let isDirectory: Bool
    public let isSymlink: Bool
    public internal(set) weak var parent: FileNode?
    /// Allocated bytes on disk, including everything below a directory.
    public internal(set) var size: Int64
    /// Number of non-directory items at or below this node.
    public internal(set) var fileCount: Int
    public internal(set) var children: [FileNode] = []
    /// The directory could not be opened (permissions, cloud-only, ...).
    public internal(set) var inaccessible = false

    public init(name: String, isDirectory: Bool, isSymlink: Bool = false, size: Int64 = 0, parent: FileNode? = nil) {
        self.name = name
        self.isDirectory = isDirectory
        self.isSymlink = isSymlink
        self.size = size
        self.fileCount = isDirectory ? 0 : 1
        self.parent = parent
    }

    /// Replace this directory's children and recompute its own totals from them.
    /// For building trees by hand; the scanner uses `ScanTree.attach`.
    public func setChildren(_ kids: [FileNode]) {
        children = kids
        size = 0
        fileCount = 0
        for k in kids {
            k.parent = self
            size += k.size
            fileCount += k.fileCount
        }
    }

    /// Full path. The root node's `name` holds the absolute path of the scan root.
    public var path: String {
        var parts: [String] = []
        var node: FileNode? = self
        while let n = node {
            parts.append(n.name)
            node = n.parent
        }
        let root = parts.removeLast()
        if parts.isEmpty { return root }
        let tail = parts.reversed().joined(separator: "/")
        return root.hasSuffix("/") ? root + tail : root + "/" + tail
    }

    public var url: URL { URL(fileURLWithPath: path, isDirectory: isDirectory) }

    /// Last path component, also for the root node.
    public var displayName: String {
        if parent != nil { return name }
        if name == "/" { return "/" }
        return (name as NSString).lastPathComponent
    }

    public var fileExtension: String {
        guard !isDirectory, let dot = name.lastIndex(of: "."), dot != name.startIndex else { return "" }
        return name[name.index(after: dot)...].lowercased()
    }

    public func isDescendant(of other: FileNode) -> Bool {
        var node: FileNode? = self
        while let n = node {
            if n === other { return true }
            node = n.parent
        }
        return false
    }
}

/// A scanned tree plus the lock that guards its mutable state.
public final class ScanTree: @unchecked Sendable {
    public let root: FileNode
    private let lock = NSLock()

    public init(root: FileNode) { self.root = root }

    public func withLock<T>(_ body: () throws -> T) rethrows -> T {
        lock.lock()
        defer { lock.unlock() }
        return try body()
    }

    /// Attach freshly read children to `node` and roll their totals up to the root.
    func attach(_ kids: [FileNode], to node: FileNode) {
        var bytes: Int64 = 0
        var files = 0
        for k in kids {
            bytes += k.size
            files += k.fileCount
        }
        withLock {
            node.children = kids
            var n: FileNode? = node
            while let cur = n {
                cur.size += bytes
                cur.fileCount += files
                n = cur.parent
            }
        }
    }

    /// Remove `node` from the tree (used when staging an item in the collector).
    /// Returns the former parent so the node can be put back.
    @discardableResult
    public func detach(_ node: FileNode) -> FileNode? {
        withLock {
            guard let parent = node.parent,
                  let idx = parent.children.firstIndex(where: { $0 === node }) else { return nil }
            parent.children.remove(at: idx)
            var n: FileNode? = parent
            while let cur = n {
                cur.size -= node.size
                cur.fileCount -= node.fileCount
                n = cur.parent
            }
            return parent
        }
    }

    /// Put a detached node back under `parent`, keeping children sorted by size.
    public func reattach(_ node: FileNode, to parent: FileNode) {
        withLock {
            node.parent = parent
            let idx = parent.children.firstIndex(where: { $0.size < node.size }) ?? parent.children.count
            parent.children.insert(node, at: idx)
            var n: FileNode? = parent
            while let cur = n {
                cur.size += node.size
                cur.fileCount += node.fileCount
                n = cur.parent
            }
        }
    }

    /// Sort every directory's children largest first.
    public func sortAll() {
        withLock {
            var stack = [root]
            while let n = stack.popLast() {
                guard !n.children.isEmpty else { continue }
                n.children.sort { $0.size > $1.size }
                for c in n.children where c.isDirectory { stack.append(c) }
            }
        }
    }
}
