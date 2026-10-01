import Foundation

public enum Fmt {
    /// Decimal units, the same convention Finder uses.
    public static func bytes(_ n: Int64) -> String {
        let units = ["bytes", "KB", "MB", "GB", "TB", "PB"]
        var v = Double(n)
        var i = 0
        while abs(v) >= 1000 && i < units.count - 1 {
            v /= 1000
            i += 1
        }
        if i == 0 { return "\(n) bytes" }
        return String(format: v >= 100 ? "%.0f %@" : (v >= 10 ? "%.1f %@" : "%.2f %@"), v, units[i])
    }

    public static func count(_ n: Int) -> String {
        let f = NumberFormatter()
        f.numberStyle = .decimal
        return f.string(from: NSNumber(value: n)) ?? "\(n)"
    }
}

public struct VolumeInfo: Sendable, Identifiable, Equatable {
    public let mountPoint: String
    public let name: String
    public let fsType: String
    public let source: String
    public let total: Int64
    public let free: Int64
    public let isNetwork: Bool
    public let isReadOnly: Bool
    public var id: String { mountPoint }
    public var used: Int64 { max(0, total - free) }
    public var usedFraction: Double { total > 0 ? Double(used) / Double(total) : 0 }
}

/// Read-only facts about this Mac's storage. Nothing here changes any state.
public enum SystemFacts {
    /// Mounted volumes a person would recognise: the startup disk, external
    /// drives and network shares. Internal helper volumes are left out.
    public static func volumes() -> [VolumeInfo] {
        var list: UnsafeMutablePointer<statfs>?
        let n = getmntinfo(&list, MNT_NOWAIT)
        guard n > 0, let list else { return [] }
        var out: [VolumeInfo] = []
        for i in 0..<Int(n) {
            var fs = list[i]
            let mount = withUnsafePointer(to: &fs.f_mntonname) {
                $0.withMemoryRebound(to: CChar.self, capacity: Int(MAXPATHLEN)) { String(cString: $0) }
            }
            let type = withUnsafePointer(to: &fs.f_fstypename) {
                $0.withMemoryRebound(to: CChar.self, capacity: Int(MFSTYPENAMELEN)) { String(cString: $0) }
            }
            let source = withUnsafePointer(to: &fs.f_mntfromname) {
                $0.withMemoryRebound(to: CChar.self, capacity: Int(MAXPATHLEN)) { String(cString: $0) }
            }
            let flags = Int32(bitPattern: fs.f_flags)
            if ["devfs", "autofs"].contains(type) { continue }
            if flags & MNT_DONTBROWSE != 0 && mount != "/" { continue }
            let block = Int64(fs.f_bsize)
            var total = Int64(fs.f_blocks) * block
            var free = Int64(fs.f_bavail) * block
            var name = mount == "/" ? "Macintosh HD" : (mount as NSString).lastPathComponent
            let url = URL(fileURLWithPath: mount)
            if let v = try? url.resourceValues(forKeys: [.volumeNameKey, .volumeTotalCapacityKey,
                                                         .volumeAvailableCapacityForImportantUsageKey]) {
                if let n = v.volumeName { name = n }
                if flags & MNT_LOCAL != 0 {
                    if let t = v.volumeTotalCapacity { total = Int64(t) }
                    if let f = v.volumeAvailableCapacityForImportantUsage, f > 0 { free = f }
                }
            }
            out.append(VolumeInfo(mountPoint: mount, name: name, fsType: type, source: source,
                                  total: total, free: free,
                                  isNetwork: flags & MNT_LOCAL == 0, isReadOnly: flags & MNT_RDONLY != 0 && mount != "/"))
        }
        return out
    }

    /// Run a read-only command and return its output, or nil on failure/timeout.
    static func run(_ tool: String, _ args: [String], timeout: TimeInterval = 6, maxChars: Int = 2500) -> String? {
        guard FileManager.default.isExecutableFile(atPath: tool) else { return nil }
        let p = Process()
        p.executableURL = URL(fileURLWithPath: tool)
        p.arguments = args
        let pipe = Pipe()
        p.standardOutput = pipe
        p.standardError = FileHandle.nullDevice
        p.standardInput = FileHandle.nullDevice
        do { try p.run() } catch { return nil }
        let killer = DispatchWorkItem { if p.isRunning { p.terminate() } }
        DispatchQueue.global().asyncAfter(deadline: .now() + timeout, execute: killer)
        let data = pipe.fileHandleForReading.readDataToEndOfFile()
        p.waitUntilExit()
        killer.cancel()
        guard p.terminationStatus == 0 else { return nil }
        let s = String(decoding: data, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
        return s.isEmpty ? nil : String(s.prefix(maxChars))
    }

    private static func sysctlString(_ name: String) -> String? {
        var size = 0
        guard sysctlbyname(name, nil, &size, nil, 0) == 0, size > 0 else { return nil }
        var buf = [CChar](repeating: 0, count: size)
        guard sysctlbyname(name, &buf, &size, nil, 0) == 0 else { return nil }
        return String(cString: buf)
    }

    /// A plain-text description of the machine's storage, for the advisor prompt.
    public static func machineReport() -> String {
        var lines: [String] = []
        let info = ProcessInfo.processInfo
        lines.append("## Machine")
        lines.append("Model: \(sysctlString("hw.model") ?? "unknown"), CPU: \(sysctlString("machdep.cpu.brand_string") ?? "unknown"), \(info.activeProcessorCount) cores")
        lines.append("RAM: \(Fmt.bytes(Int64(info.physicalMemory))), macOS \(info.operatingSystemVersionString)")
        var swap = xsw_usage()
        var swapSize = MemoryLayout<xsw_usage>.size
        if sysctlbyname("vm.swapusage", &swap, &swapSize, nil, 0) == 0 {
            lines.append("Swap in use: \(Fmt.bytes(Int64(swap.xsu_used))) of \(Fmt.bytes(Int64(swap.xsu_total)))")
        }

        lines.append("\n## Mounted volumes")
        for v in volumes() {
            let kind = v.isNetwork ? "NETWORK share" : "local"
            lines.append("- \(v.name) at \(v.mountPoint): \(kind), \(v.fsType), source \(v.source), "
                + "\(Fmt.bytes(v.total)) total, \(Fmt.bytes(v.free)) free (\(Int(v.usedFraction * 100))% used)")
        }

        if let d = run("/usr/sbin/diskutil", ["info", "/"]) {
            let keep = ["Device / Media Name", "Solid State", "Protocol", "SMART Status", "File System Personality",
                        "Container Total Space", "Container Free Space", "FileVault", "Media Type"]
            let picked = d.split(separator: "\n").filter { l in keep.contains { l.contains($0) } }
                .map { $0.trimmingCharacters(in: .whitespaces).replacingOccurrences(of: "  ", with: "") }
            if !picked.isEmpty {
                lines.append("\n## Startup disk (diskutil info /)")
                lines.append(contentsOf: picked)
            }
        }
        if let snaps = run("/usr/bin/tmutil", ["listlocalsnapshots", "/"]) {
            let count = snaps.split(separator: "\n").filter { $0.contains("com.apple") }.count
            lines.append("\n## Time Machine\nLocal APFS snapshots on startup disk: \(count)")
        }
        if let dest = run("/usr/bin/tmutil", ["destinationinfo"]) {
            let picked = dest.split(separator: "\n").filter { $0.contains("Name") || $0.contains("Kind") || $0.contains("URL") }
            lines.append("Time Machine destinations:\n" + picked.joined(separator: "\n"))
        }
        if let smb = run("/usr/bin/smbutil", ["statshares", "-a"], maxChars: 3000) {
            let keep = ["SHARE", "SMB_VERSION", "SMB_NEGOTIATE", "SIGNING_ON", "SIGNING_REQUIRED", "ENCRYPTION",
                        "MULTICHANNEL", "SERVER_NAME", "CUR_RWND", "---", "volume", "Studio", "TimeMachine"]
            let picked = smb.split(separator: "\n").filter { l in keep.contains { l.contains($0) } }
            if !picked.isEmpty {
                lines.append("\n## SMB session attributes (smbutil statshares -a)")
                lines.append(contentsOf: picked.prefix(45).map(String.init))
            }
        }
        return lines.joined(separator: "\n")
    }

    /// A plain-text summary of a finished scan: biggest folders, file types and files.
    public static func scanReport(tree: ScanTree, focus: FileNode) -> String {
        var lines: [String] = []
        let (top, path, size, files): ([(String, Int64, [(String, Int64)])], String, Int64, Int) = tree.withLock {
            let kids = focus.children.sorted { $0.size > $1.size }.prefix(12)
            let rows = kids.map { k -> (String, Int64, [(String, Int64)]) in
                let sub = k.isDirectory ? k.children.sorted { $0.size > $1.size }.prefix(5).map { ($0.name, $0.size) } : []
                return (k.name + (k.isDirectory ? "/" : ""), k.size, Array(sub))
            }
            return (rows, focus.path, focus.size, focus.fileCount)
        }
        lines.append("## Disk usage scan of \(path)")
        lines.append("Total: \(Fmt.bytes(size)) in \(Fmt.count(files)) files")
        lines.append("\nLargest items (with their largest children):")
        for (name, sz, sub) in top where sz > 0 {
            lines.append("- \(name): \(Fmt.bytes(sz))")
            for (n, s) in sub where s > sz / 50 { lines.append("    - \(n): \(Fmt.bytes(s))") }
        }
        lines.append("\nBy file type:")
        for b in Analysis.typeBreakdown(in: tree, under: focus).prefix(10) {
            lines.append("- \(b.category): \(Fmt.bytes(b.bytes)) in \(Fmt.count(b.count)) files")
        }
        lines.append("\nLargest single files:")
        let big = Analysis.largestFiles(in: tree, under: focus, limit: 15)
        let rows = tree.withLock { big.map { ($0.path, $0.size) } }
        for (p, s) in rows { lines.append("- \(p): \(Fmt.bytes(s))") }
        return lines.joined(separator: "\n")
    }
}
