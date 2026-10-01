import Foundation

public struct ScanProgress: Sendable, Equatable {
    public var files = 0
    public var directories = 0
    public var bytes: Int64 = 0
    public var inaccessible = 0

    public init() {}
}

/// Parallel directory scanner built on `getattrlistbulk`.
///
/// Sizes are allocated (on-disk) bytes. Hard-linked files are counted once.
/// The scan stays on the root's volume; when scanning "/" it also follows
/// firmlinks into the Data volume, which is where user data lives.
public final class DiskScanner: @unchecked Sendable {
    public let tree: ScanTree
    public let rootPath: String

    private struct Job {
        let node: FileNode
        let path: String
    }

    private let cond = NSCondition()
    private var jobs: [Job] = []
    private var active = 0
    private var cancelled = false
    private var finished = false

    private let statLock = NSLock()
    private var stats = ScanProgress()
    private var seenLinks = Set<LinkKey>()

    private var allowedDevices = Set<dev_t>()
    private var skippedPaths = Set<String>()

    private struct LinkKey: Hashable {
        let dev: dev_t
        let inode: UInt64
    }

    public init(path: String) {
        var resolved = [CChar](repeating: 0, count: Int(PATH_MAX))
        let real = realpath(path, &resolved) != nil ? String(cString: resolved) : path
        rootPath = real
        tree = ScanTree(root: FileNode(name: real, isDirectory: true))

        var st = stat()
        if stat(real, &st) == 0 { allowedDevices.insert(st.st_dev) }
        if real == "/" {
            let data = "/System/Volumes/Data"
            if stat(data, &st) == 0 {
                allowedDevices.insert(st.st_dev)
                skippedPaths.insert(data)
            }
        }
    }

    public var progress: ScanProgress {
        statLock.lock()
        defer { statLock.unlock() }
        return stats
    }

    public var isFinished: Bool {
        cond.lock()
        defer { cond.unlock() }
        return finished
    }

    public var isCancelled: Bool {
        cond.lock()
        defer { cond.unlock() }
        return cancelled
    }

    public func cancel() {
        cond.lock()
        cancelled = true
        cond.broadcast()
        cond.unlock()
    }

    /// Scan the whole tree. Blocks until done or cancelled.
    public func run(threads: Int = max(2, ProcessInfo.processInfo.activeProcessorCount)) {
        cond.lock()
        jobs.append(Job(node: tree.root, path: rootPath))
        cond.unlock()

        let group = DispatchGroup()
        for i in 0..<threads {
            group.enter()
            let t = Thread { [self] in
                worker()
                group.leave()
            }
            t.name = "frisy-scan-\(i)"
            t.stackSize = 1 << 20
            t.start()
        }
        group.wait()
        tree.sortAll()
        cond.lock()
        finished = true
        cond.unlock()
    }

    private func worker() {
        // Never pull cloud-only (dataless) directories down just to measure them.
        setiopolicy_np(IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES, IOPOL_SCOPE_THREAD,
                       IOPOL_MATERIALIZE_DATALESS_FILES_OFF)
        let bufSize = 256 * 1024
        let buf = UnsafeMutableRawPointer.allocate(byteCount: bufSize, alignment: 8)
        defer { buf.deallocate() }

        while true {
            cond.lock()
            while jobs.isEmpty && active > 0 && !cancelled { cond.wait() }
            if cancelled || jobs.isEmpty {
                cond.broadcast()
                cond.unlock()
                return
            }
            let job = jobs.removeLast()
            active += 1
            cond.unlock()

            let subdirs = scanDirectory(job, buf: buf, bufSize: bufSize)

            cond.lock()
            jobs.append(contentsOf: subdirs)
            active -= 1
            cond.broadcast()
            cond.unlock()
        }
    }

    private func scanDirectory(_ job: Job, buf: UnsafeMutableRawPointer, bufSize: Int) -> [Job] {
        if skippedPaths.contains(job.path) { return [] }
        let fd = open(job.path, O_RDONLY | O_DIRECTORY | O_NOFOLLOW)
        if fd < 0 {
            let err = errno
            // EDEADLK: cloud-only directory we refused to materialize. Not a permission problem.
            if err != EDEADLK && err != ENOENT {
                tree.withLock { job.node.inaccessible = true }
                statLock.lock()
                stats.inaccessible += 1
                statLock.unlock()
            }
            return []
        }
        defer { close(fd) }

        var st = stat()
        guard fstat(fd, &st) == 0, allowedDevices.contains(st.st_dev) else { return [] }
        let dev = st.st_dev

        var attrs = attrlist()
        attrs.bitmapcount = UInt16(ATTR_BIT_MAP_COUNT)
        attrs.commonattr = attrgroup_t(ATTR_CMN_RETURNED_ATTRS) | attrgroup_t(ATTR_CMN_NAME)
            | attrgroup_t(ATTR_CMN_ERROR) | attrgroup_t(ATTR_CMN_OBJTYPE) | attrgroup_t(ATTR_CMN_FILEID)
        attrs.fileattr = attrgroup_t(ATTR_FILE_LINKCOUNT) | attrgroup_t(ATTR_FILE_ALLOCSIZE)

        var kids: [FileNode] = []
        var subdirs: [Job] = []
        var bytes: Int64 = 0
        var files = 0
        let prefix = job.path.hasSuffix("/") ? job.path : job.path + "/"

        while true {
            let n = getattrlistbulk(fd, &attrs, buf, bufSize, 0)
            if n < 0 {
                if errno == EINTR { continue }
                break
            }
            if n == 0 { break }

            var entry = buf
            for _ in 0..<n {
                let length = Int(entry.loadUnaligned(as: UInt32.self))
                var field = entry + 4
                let returned = field.loadUnaligned(as: attribute_set_t.self)
                field += MemoryLayout<attribute_set_t>.size

                var error: UInt32 = 0
                if returned.commonattr & attrgroup_t(ATTR_CMN_ERROR) != 0 {
                    error = field.loadUnaligned(as: UInt32.self)
                    field += 4
                }
                var name = ""
                if returned.commonattr & attrgroup_t(ATTR_CMN_NAME) != 0 {
                    let ref = field.loadUnaligned(as: attrreference_t.self)
                    name = String(cString: (field + Int(ref.attr_dataoffset)).assumingMemoryBound(to: CChar.self))
                    field += MemoryLayout<attrreference_t>.size
                }
                var objType: UInt32 = 0
                if returned.commonattr & attrgroup_t(ATTR_CMN_OBJTYPE) != 0 {
                    objType = field.loadUnaligned(as: UInt32.self)
                    field += 4
                }
                var fileID: UInt64 = 0
                if returned.commonattr & attrgroup_t(ATTR_CMN_FILEID) != 0 {
                    fileID = field.loadUnaligned(as: UInt64.self)
                    field += 8
                }
                var linkCount: UInt32 = 1
                if returned.fileattr & attrgroup_t(ATTR_FILE_LINKCOUNT) != 0 {
                    linkCount = field.loadUnaligned(as: UInt32.self)
                    field += 4
                }
                var alloc: Int64 = 0
                if returned.fileattr & attrgroup_t(ATTR_FILE_ALLOCSIZE) != 0 {
                    alloc = field.loadUnaligned(as: Int64.self)
                    field += 8
                }
                entry += length

                if name.isEmpty || error != 0 { continue }

                if objType == VDIR.rawValue {
                    let child = FileNode(name: name, isDirectory: true, parent: job.node)
                    kids.append(child)
                    subdirs.append(Job(node: child, path: prefix + name))
                } else {
                    if linkCount > 1 && objType == VREG.rawValue {
                        statLock.lock()
                        let fresh = seenLinks.insert(LinkKey(dev: dev, inode: fileID)).inserted
                        statLock.unlock()
                        if !fresh { alloc = 0 }
                    }
                    kids.append(FileNode(name: name, isDirectory: false,
                                         isSymlink: objType == VLNK.rawValue,
                                         size: alloc, parent: job.node))
                    bytes += alloc
                    files += 1
                }
            }
        }

        tree.attach(kids, to: job.node)
        statLock.lock()
        stats.files += files
        stats.bytes += bytes
        stats.directories += 1
        statLock.unlock()
        return subdirs
    }
}
