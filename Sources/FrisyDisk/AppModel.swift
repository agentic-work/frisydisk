import AppKit
import FrisyCore
import SwiftUI

/// One line in the sidebar. A value snapshot, so views never touch the tree.
struct Row: Identifiable {
    let node: FileNode
    let name: String
    let subtitle: String?
    let size: Int64
    let fileCount: Int
    let isDirectory: Bool
    /// Share of the focused folder, 0...1.
    let fraction: Double
    let hue: Double
    var id: ObjectIdentifier { ObjectIdentifier(node) }
}

struct HoverInfo {
    let node: FileNode?
    let name: String
    let size: Int64
    let fileCount: Int
    let isDirectory: Bool
}

struct Staged: Identifiable {
    let node: FileNode
    let parent: FileNode
    let path: String
    var id: ObjectIdentifier { ObjectIdentifier(node) }
}

enum ChartMode: String, CaseIterable, Identifiable {
    case sunburst = "Sunburst"
    case treemap = "Treemap"
    var id: String { rawValue }
}

enum SideTab: String, CaseIterable, Identifiable {
    case contents = "Contents"
    case largest = "Largest"
    case types = "Types"
    case advisor = "Advisor"
    var id: String { rawValue }
}

@MainActor
final class AppModel: ObservableObject {
    static let sunburstDepth = 6

    // Scan state
    @Published var scanner: DiskScanner?
    @Published var isScanning = false
    @Published var progress = ScanProgress()
    @Published var scanSeconds: Double = 0
    @Published var volumes: [VolumeInfo] = []

    // What is on screen
    @Published var focus: FileNode?
    @Published var crumbs: [FileNode] = []
    @Published var focusSize: Int64 = 0
    @Published var focusFiles = 0
    @Published var segments: [SunburstSegment] = []
    @Published var tiles: [TreemapTile] = []
    @Published var rows: [Row] = []
    @Published var hover: HoverInfo?
    @Published var mode: ChartMode = .sunburst
    @Published var tab: SideTab = .contents
    var chartSize: CGSize = .zero

    // Analysis
    @Published var largest: [Row] = []
    @Published var types: [TypeBucket] = []
    @Published var searchText = "" {
        didSet { if searchText != oldValue { runSearch() } }
    }
    @Published var searchResults: [Row] = []

    // Collector
    @Published var collector: [Staged] = []
    @Published var notice: String?
    // View state kept here because the Command Line Tools cannot expand @State.
    @Published var showCollectorItems = false
    @Published var confirmTrash = false
    @Published var advisorQuestion = ""

    // Advisor
    @Published var backends: [AdvisorBackend] = []
    @Published var backend: AdvisorBackend?
    @Published var backendsLoaded = false
    @Published var chat: [ChatMessage] = []
    @Published var isAdvising = false
    @Published var advisorError: String?

    private var timer: Timer?
    private var scanStart = Date()
    private var advisorTask: Task<Void, Never>?
    private var analysisGeneration = 0
    private var snapshotPath: String?
    private var adviseOnFinish = false
    private var testTrashChild: String?

    var tree: ScanTree? { scanner?.tree }
    var collectorSize: Int64 { collector.reduce(0) { $0 + $1.node.size } }

    init() {
        volumes = SystemFacts.volumes()
    }

    // MARK: - Scanning

    func startScan(path: String) {
        stopScan()
        collector.removeAll()
        let s = DiskScanner(path: path)
        scanner = s
        focus = s.tree.root
        isScanning = true
        progress = ScanProgress()
        scanStart = Date()
        largest = []
        types = []
        searchText = ""
        chat = []
        hover = nil
        rebuild()

        Thread.detachNewThread { [weak self] in
            s.run()
            DispatchQueue.main.async { self?.scanFinished(s) }
        }
        timer = Timer.scheduledTimer(withTimeInterval: 0.4, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.tick() }
        }
    }

    private func tick() {
        guard let scanner, isScanning else { return }
        progress = scanner.progress
        scanSeconds = Date().timeIntervalSince(scanStart)
        rebuild()
    }

    private func scanFinished(_ s: DiskScanner) {
        guard s === scanner else { return }
        timer?.invalidate()
        timer = nil
        isScanning = false
        progress = s.progress
        scanSeconds = Date().timeIntervalSince(scanStart)
        rebuild()
        refreshAnalysis()
        if let name = testTrashChild {
            // Scripted test of the collector: stage one named child of the scan root and trash it.
            testTrashChild = nil
            if let node = s.tree.root.children.first(where: { $0.name == name }) {
                stage(node)
                let staged = collector.count
                trashCollector()
                print("test-trash: staged=\(staged) root=\(s.tree.root.size) notice=\(notice ?? "")")
                notice = nil
            } else {
                print("test-trash: no child named \(name)")
            }
        }
        if adviseOnFinish {
            adviseOnFinish = false
            tab = .advisor
            askAdvisor()
        } else {
            scheduleSnapshotIfRequested(after: 1.5)
        }
    }

    func stopScan() {
        timer?.invalidate()
        timer = nil
        if isScanning { scanner?.cancel() }
        isScanning = false
    }

    func closeScan() {
        stopScan()
        advisorTask?.cancel()
        for item in collector.reversed() { tree?.reattach(item.node, to: item.parent) }
        collector.removeAll()
        scanner = nil
        focus = nil
        segments = []
        tiles = []
        rows = []
        volumes = SystemFacts.volumes()
    }

    func rescan() {
        if let path = scanner?.rootPath { startScan(path: path) }
    }

    // MARK: - Navigation

    func setFocus(_ node: FileNode) {
        guard node.isDirectory, node !== focus else { return }
        focus = node
        hover = nil
        rebuild()
        if !isScanning { refreshAnalysis() }
    }

    func goUp() {
        if let parent = focus?.parent { setFocus(parent) }
    }

    /// Recompute everything the views draw from the tree, under the tree lock.
    func rebuild() {
        guard let tree, let focus else { return }
        tree.withLock {
            focusSize = focus.size
            focusFiles = focus.fileCount
            switch mode {
            case .sunburst:
                segments = SunburstLayout.layout(root: focus, maxDepth: Self.sunburstDepth)
            case .treemap:
                tiles = TreemapLayout.layout(root: focus, in: CGRect(origin: .zero, size: chartSize).insetBy(dx: 6, dy: 6))
            }
            let total = max(1, Double(focus.size))
            var cursor = 0.0
            var out: [Row] = []
            for child in focus.children.sorted(by: { $0.size > $1.size }).prefix(600) {
                let f = Double(child.size) / total
                out.append(Row(node: child, name: child.name, subtitle: nil, size: child.size,
                               fileCount: child.fileCount, isDirectory: child.isDirectory,
                               fraction: f, hue: cursor + f / 2))
                cursor += f
            }
            rows = out

            var chain: [FileNode] = []
            var n: FileNode? = focus
            while let cur = n {
                chain.append(cur)
                n = cur.parent
            }
            crumbs = chain.reversed()
        }
    }

    func setMode(_ m: ChartMode) {
        mode = m
        hover = nil
        rebuild()
    }

    func chartResized(_ size: CGSize) {
        guard size != chartSize else { return }
        chartSize = size
        if mode == .treemap { rebuild() }
    }

    // MARK: - Hover and click

    func hover(node: FileNode?) {
        guard let node else {
            hover = nil
            return
        }
        if hover?.node === node { return }
        let (size, count) = tree?.withLock { (node.size, node.fileCount) } ?? (0, 0)
        hover = HoverInfo(node: node, name: node.name, size: size, fileCount: count, isDirectory: node.isDirectory)
    }

    func hoverChart(at point: CGPoint?) {
        guard let point else {
            hover = nil
            return
        }
        switch mode {
        case .sunburst:
            let geo = SunburstGeometry(size: chartSize, maxDepth: Self.sunburstDepth)
            if case .segment(let i) = geo.hitTest(point, segments: segments) {
                let s = segments[i]
                if let node = s.node {
                    hover(node: node)
                } else {
                    hover = HoverInfo(node: nil, name: "\(Fmt.count(s.itemCount)) smaller items",
                                      size: s.size, fileCount: s.itemCount, isDirectory: false)
                }
            } else {
                hover = nil
            }
        case .treemap:
            if let i = TreemapLayout.hitTest(point, tiles: tiles) {
                let t = tiles[i]
                if let node = t.node {
                    hover(node: node)
                } else {
                    hover = HoverInfo(node: nil, name: "\(Fmt.count(t.itemCount)) smaller items",
                                      size: t.size, fileCount: t.itemCount, isDirectory: false)
                }
            } else {
                hover = nil
            }
        }
    }

    func clickChart(at point: CGPoint) {
        switch mode {
        case .sunburst:
            let geo = SunburstGeometry(size: chartSize, maxDepth: Self.sunburstDepth)
            switch geo.hitTest(point, segments: segments) {
            case .center: goUp()
            case .segment(let i):
                let s = segments[i]
                if let node = s.node, node.isDirectory { setFocus(node) } else { setFocus(s.parent) }
            case .none: break
            }
        case .treemap:
            guard let i = TreemapLayout.hitTest(point, tiles: tiles) else { return }
            let t = tiles[i]
            if let node = t.node, node.isDirectory { setFocus(node) } else { setFocus(t.parent) }
        }
    }

    // MARK: - Analysis

    private func makeRows(_ nodes: [FileNode], in tree: ScanTree, total: Int64) -> [Row] {
        tree.withLock {
            nodes.map { n in
                Row(node: n, name: n.name, subtitle: n.parent?.path, size: n.size, fileCount: n.fileCount,
                    isDirectory: n.isDirectory, fraction: Double(n.size) / Double(max(1, total)), hue: -1)
            }
        }
    }

    func refreshAnalysis() {
        guard let tree, let focus, !isScanning else { return }
        analysisGeneration += 1
        let gen = analysisGeneration
        Task.detached(priority: .userInitiated) { [weak self] in
            let big = Analysis.largestFiles(in: tree, under: focus, limit: 200)
            let buckets = Analysis.typeBreakdown(in: tree, under: focus)
            guard let self else { return }
            let total = await self.focusSize
            let rows = await self.makeRows(big, in: tree, total: total)
            await MainActor.run {
                guard gen == self.analysisGeneration else { return }
                self.largest = rows
                self.types = buckets
            }
        }
        runSearch()
    }

    private func runSearch() {
        let query = searchText.trimmingCharacters(in: .whitespaces)
        guard let tree, let focus, !isScanning, query.count >= 2 else {
            searchResults = []
            return
        }
        Task.detached(priority: .userInitiated) { [weak self] in
            let hits = Analysis.search(in: tree, under: focus, query: query)
            guard let self else { return }
            let total = await self.focusSize
            let rows = await self.makeRows(hits, in: tree, total: total)
            await MainActor.run {
                if self.searchText.trimmingCharacters(in: .whitespaces) == query { self.searchResults = rows }
            }
        }
    }

    // MARK: - File actions

    func reveal(_ node: FileNode) {
        NSWorkspace.shared.activateFileViewerSelecting([node.url])
    }

    func copyPath(_ node: FileNode) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(node.path, forType: .string)
    }

    func quickLook(_ node: FileNode) {
        QuickLook.shared.show(node.url)
    }

    // MARK: - Collector

    func canStage(_ node: FileNode) -> Bool {
        !isScanning && node.parent != nil && !collector.contains { $0.node === node }
    }

    /// Take an item out of the picture and hold it for review. Nothing on disk changes.
    func stage(_ node: FileNode) {
        guard canStage(node), let tree else { return }
        let path = node.path
        if let focus, focus.isDescendant(of: node), let parent = node.parent { self.focus = parent }
        guard let parent = tree.detach(node) else { return }
        collector.append(Staged(node: node, parent: parent, path: path))
        hover = nil
        rebuild()
        refreshAnalysis()
    }

    func restore(_ item: Staged) {
        guard let tree, let idx = collector.firstIndex(where: { $0.id == item.id }) else { return }
        collector.remove(at: idx)
        tree.reattach(item.node, to: item.parent)
        rebuild()
        refreshAnalysis()
    }

    func restoreAll() {
        for item in collector.reversed() { tree?.reattach(item.node, to: item.parent) }
        collector.removeAll()
        rebuild()
        refreshAnalysis()
    }

    /// Move every staged item to the Trash. Items that fail go back into the chart.
    func trashCollector() {
        var failed: [String] = []
        var freed: Int64 = 0
        var moved = 0
        for item in collector {
            do {
                try FileManager.default.trashItem(at: URL(fileURLWithPath: item.path), resultingItemURL: nil)
                freed += item.node.size
                moved += 1
            } catch {
                failed.append("\((item.path as NSString).lastPathComponent): \(error.localizedDescription)")
                tree?.reattach(item.node, to: item.parent)
            }
        }
        collector.removeAll()
        rebuild()
        refreshAnalysis()
        volumes = SystemFacts.volumes()
        if failed.isEmpty {
            notice = "Moved \(moved) item\(moved == 1 ? "" : "s") (\(Fmt.bytes(freed))) to the Trash. Empty the Trash to free the space."
        } else {
            notice = "Moved \(moved) to the Trash. Could not move: " + failed.joined(separator: "; ")
        }
    }

    // MARK: - Advisor

    func loadBackends() {
        guard !backendsLoaded else { return }
        Task.detached { [weak self] in
            let found = await Advisor.availableBackends()
            await MainActor.run { [weak self] in
                guard let self else { return }
                self.backends = found
                if self.backend == nil { self.backend = found.first }
                self.backendsLoaded = true
            }
        }
    }

    func askAdvisor(followUp: String? = nil) {
        guard !isAdvising else { return }
        isAdvising = true
        advisorError = nil
        let tree = self.tree
        let focus = self.focus
        let scanning = isScanning
        var history = chat
        if let followUp { history.append(ChatMessage(role: .user, text: followUp)) }

        advisorTask = Task { [weak self] in
            guard let self else { return }
            var history = history
            var backend = self.backend
            if backend == nil {
                let found = await Task.detached { await Advisor.availableBackends() }.value
                self.backends = found
                self.backendsLoaded = true
                backend = found.first
                self.backend = backend
            }
            guard let backend else {
                self.advisorError = "No local model found. Start Ollama (with a model pulled) or agenticode, then try again."
                self.isAdvising = false
                self.scheduleSnapshotIfRequested(after: 0.5)
                return
            }
            if history.isEmpty {
                let facts = await Task.detached { () -> String in
                    let machine = SystemFacts.machineReport()
                    var scan: String?
                    if let tree, let focus, !scanning { scan = SystemFacts.scanReport(tree: tree, focus: focus) }
                    return Advisor.userPrompt(machine: machine, scan: scan)
                }.value
                history = [ChatMessage(role: .system, text: Advisor.systemPrompt),
                           ChatMessage(role: .user, text: facts)]
            }
            self.chat = history + [ChatMessage(role: .assistant, text: "")]
            do {
                for try await chunk in Advisor.stream(history, backend: backend) {
                    if Task.isCancelled { break }
                    self.chat[self.chat.count - 1].text += chunk
                }
            } catch is CancellationError {
            } catch {
                self.advisorError = error.localizedDescription
            }
            if self.chat.last?.text.isEmpty == true { self.chat.removeLast() }
            self.isAdvising = false
            self.scheduleSnapshotIfRequested(after: 1.0)
        }
    }

    func stopAdvisor() {
        advisorTask?.cancel()
    }

    func resetAdvisor() {
        advisorTask?.cancel()
        chat = []
        advisorError = nil
    }

    // MARK: - Launch arguments (used for scripted testing)

    /// `--scan PATH`, `--mode treemap`, `--tab largest`, `--advise`, `--snapshot OUT.png`,
    /// `--test-trash-child NAME` (moves that child of the scan root to the Trash without asking)
    func handleLaunchArguments() {
        let args = CommandLine.arguments
        func value(_ name: String) -> String? {
            guard let i = args.firstIndex(of: name), i + 1 < args.count else { return nil }
            return args[i + 1]
        }
        if let m = value("--mode"), let parsed = ChartMode.allCases.first(where: { $0.rawValue.lowercased() == m.lowercased() }) {
            mode = parsed
        }
        if let t = value("--tab"), let parsed = SideTab.allCases.first(where: { $0.rawValue.lowercased() == t.lowercased() }) {
            tab = parsed
        }
        snapshotPath = value("--snapshot")
        adviseOnFinish = args.contains("--advise")
        testTrashChild = value("--test-trash-child")
        if let path = value("--scan") {
            startScan(path: (path as NSString).expandingTildeInPath)
        } else {
            scheduleSnapshotIfRequested(after: 1.5)
        }
    }

    private func scheduleSnapshotIfRequested(after delay: Double) {
        guard let path = snapshotPath else { return }
        snapshotPath = nil
        DispatchQueue.main.asyncAfter(deadline: .now() + delay) {
            guard let window = NSApp.windows.first(where: { $0.isVisible && $0.contentView != nil }),
                  let view = window.contentView,
                  let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds) else {
                FileHandle.standardError.write(Data("snapshot: no window\n".utf8))
                exit(1)
            }
            view.cacheDisplay(in: view.bounds, to: rep)
            if let data = rep.representation(using: .png, properties: [:]) {
                try? data.write(to: URL(fileURLWithPath: path))
            }
            exit(0)
        }
    }
}
