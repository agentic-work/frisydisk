import Foundation
import FrisyCore

// Self-contained test runner: `Scripts/build.sh test` builds and runs it.

var failures = 0
var checks = 0

func expect(_ condition: @autoclosure () throws -> Bool, file: StaticString = #fileID, line: UInt = #line) {
    checks += 1
    let ok = (try? condition()) ?? false
    if !ok {
        failures += 1
        print("    FAILED \(file):\(line)")
    }
}

func fail(_ message: String, file: StaticString = #fileID, line: UInt = #line) {
    checks += 1
    failures += 1
    print("    FAILED \(file):\(line): \(message)")
}

func run(_ name: String, _ body: () throws -> Void) {
    let before = failures
    do { try body() } catch {
        failures += 1
        print("    THREW \(error)")
    }
    print("\(failures == before ? "ok  " : "FAIL") \(name)")
}

/// Builds a throwaway directory tree and removes it afterwards.
final class Fixture {
    let url: URL

    init() throws {
        url = FileManager.default.temporaryDirectory.appendingPathComponent("frisy-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
    }

    deinit { try? FileManager.default.removeItem(at: url) }

    func write(_ rel: String, bytes: Int) throws {
        let f = url.appendingPathComponent(rel)
        try FileManager.default.createDirectory(at: f.deletingLastPathComponent(), withIntermediateDirectories: true)
        // Random data so APFS cannot store it sparsely or compressed.
        var data = Data(count: bytes)
        data.withUnsafeMutableBytes { arc4random_buf($0.baseAddress, bytes) }
        try data.write(to: f)
    }

    func allocated(_ rel: String) -> Int64 {
        var st = stat()
        lstat(url.appendingPathComponent(rel).path, &st)
        return Int64(st.st_blocks) * 512
    }
}

func scan(_ url: URL) -> DiskScanner {
    let s = DiskScanner(path: url.path)
    s.run(threads: 4)
    return s
}

struct ScannerTests {
    func totalsMatchAllocatedSizes() throws {
        let fx = try Fixture()
        try fx.write("a.bin", bytes: 100_000)
        try fx.write("sub/b.bin", bytes: 300_000)
        try fx.write("sub/deep/c.bin", bytes: 50_000)
        try FileManager.default.createDirectory(at: fx.url.appendingPathComponent("empty"), withIntermediateDirectories: true)

        let s = scan(fx.url)
        let root = s.tree.root
        let expected = fx.allocated("a.bin") + fx.allocated("sub/b.bin") + fx.allocated("sub/deep/c.bin")
        expect(root.size == expected)
        expect(root.fileCount == 3)
        expect(s.isFinished)
        expect(s.progress.directories == 4)

        // Children are sorted largest first and directory totals include descendants.
        expect(root.children.map { $0.name } == ["sub", "a.bin", "empty"])
        let sub = root.children[0]
        expect(sub.size == fx.allocated("sub/b.bin") + fx.allocated("sub/deep/c.bin"))
        expect(sub.children[0].path == fx.url.resolvingSymlinksInPath().path + "/sub/b.bin"
            || sub.children[0].path.hasSuffix("/sub/b.bin"))
    }

    func hardLinksAreCountedOnce() throws {
        let fx = try Fixture()
        try fx.write("one.bin", bytes: 200_000)
        try FileManager.default.linkItem(at: fx.url.appendingPathComponent("one.bin"),
                                         to: fx.url.appendingPathComponent("two.bin"))
        let root = scan(fx.url).tree.root
        expect(root.size == fx.allocated("one.bin"))
        expect(root.fileCount == 2)
    }

    func symlinksAreNotFollowed() throws {
        let fx = try Fixture()
        let other = try Fixture()
        try other.write("big.bin", bytes: 500_000)
        try fx.write("small.bin", bytes: 10_000)
        try FileManager.default.createSymbolicLink(at: fx.url.appendingPathComponent("link"), withDestinationURL: other.url)
        let root = scan(fx.url).tree.root
        expect(root.size < 100_000)
        expect(root.children.contains { $0.name == "link" && $0.isSymlink && !$0.isDirectory })
    }

    func unreadableDirectoryIsReportedNotFatal() throws {
        let fx = try Fixture()
        try fx.write("ok.bin", bytes: 10_000)
        try fx.write("locked/secret.bin", bytes: 10_000)
        let locked = fx.url.appendingPathComponent("locked").path
        chmod(locked, 0)
        defer { chmod(locked, 0o755) }
        let s = scan(fx.url)
        expect(s.progress.inaccessible == 1)
        expect(s.tree.root.fileCount == 1)
        expect(s.tree.root.children.first { $0.name == "locked" }?.inaccessible == true)
    }

    func manyFilesAcrossManyDirectories() throws {
        let fx = try Fixture()
        for d in 0..<40 {
            for f in 0..<25 { try fx.write("d\(d)/n\(d % 3)/f\(f).dat", bytes: 4096) }
        }
        let root = scan(fx.url).tree.root
        expect(root.fileCount == 1000)
        expect(root.size == Int64(1000) * fx.allocated("d0/n0/f0.dat"))
        expect(root.children.count == 40)
    }

    func detachAndReattachKeepTotalsConsistent() throws {
        let fx = try Fixture()
        try fx.write("a/x.bin", bytes: 100_000)
        try fx.write("b/y.bin", bytes: 200_000)
        let tree = scan(fx.url).tree
        let full = tree.root.size
        let b = tree.root.children[0]
        expect(b.name == "b")
        let parent = tree.detach(b)
        expect(parent === tree.root)
        expect(tree.root.size == full - b.size)
        expect(tree.root.fileCount == 1)
        tree.reattach(b, to: tree.root)
        expect(tree.root.size == full)
        expect(tree.root.children.map { $0.name } == ["b", "a"])
    }
}

/// Hand-built tree: root(100) = big(60: x 40, y 20) + mid(30) + 10 files of 1.
func sampleTree() -> FileNode {
    let root = FileNode(name: "/r", isDirectory: true)
    let big = FileNode(name: "big", isDirectory: true, parent: root)
    let x = FileNode(name: "x.mov", isDirectory: false, size: 40, parent: big)
    let y = FileNode(name: "y.zip", isDirectory: false, size: 20, parent: big)
    big.setChildren([x, y])
    let mid = FileNode(name: "mid.mov", isDirectory: false, size: 30, parent: root)
    let small = (0..<10).map { FileNode(name: "s\($0).txt", isDirectory: false, size: 1, parent: root) }
    root.setChildren([big, mid] + small)
    return root
}

struct LayoutTests {
    func sunburstCoversTheFullCircleAndGroupsSmallItems() {
        let segs = SunburstLayout.layout(root: sampleTree(), minFraction: 0.05)
        let ring1 = segs.filter { $0.depth == 1 }
        expect(ring1.count == 3)  // big, mid, group of ten small files
        expect(ring1[0].node?.name == "big")
        expect(abs(ring1[0].end - 0.6) < 1e-9)
        let group = ring1[2]
        expect(group.isGroup && group.itemCount == 10 && group.size == 10)
        expect(abs(group.end - 1.0) < 1e-9)

        // Children sit inside their parent's arc.
        let ring2 = segs.filter { $0.depth == 2 }
        expect(ring2.map { $0.node?.name } == ["x.mov", "y.zip"])
        expect(ring2[0].start == 0 && abs(ring2[1].end - 0.6) < 1e-9)
    }

    func sunburstHitTesting() {
        let segs = SunburstLayout.layout(root: sampleTree(), minFraction: 0.05)
        let geo = SunburstGeometry(size: CGSize(width: 400, height: 400), maxDepth: 4)
        expect(geo.hitTest(geo.center, segments: segs) == .center)
        // Just right of 12 o'clock in ring 1 is the start of "big".
        let r1 = (geo.innerRadius(depth: 1) + geo.outerRadius(depth: 1)) / 2
        let p = CGPoint(x: geo.center.x + 2, y: geo.center.y - r1)
        guard case .segment(let i) = geo.hitTest(p, segments: segs) else {
            fail("expected a segment")
            return
        }
        expect(segs[i].node?.name == "big")
        // Just left of 12 o'clock is the end of the circle: the group.
        let q = CGPoint(x: geo.center.x - 2, y: geo.center.y - r1)
        guard case .segment(let j) = geo.hitTest(q, segments: segs) else {
            fail("expected a segment")
            return
        }
        expect(segs[j].isGroup)
        expect(geo.hitTest(CGPoint(x: 1, y: 1), segments: segs) == SunburstGeometry.Hit.none)
    }

    func squarifyFillsTheRectangleWithoutOverlap() {
        let rect = CGRect(x: 0, y: 0, width: 600, height: 400)
        let weights: [Double] = [6, 6, 4, 3, 2, 2, 1]
        let total = weights.reduce(0, +)
        let rects = TreemapLayout.squarify(weights.map { $0 / total * 240_000 }, in: rect)
        expect(rects.count == weights.count)
        for (w, r) in zip(weights, rects) {
            expect(abs(Double(r.width * r.height) - w / total * 240_000) < 1)
            expect(rect.insetBy(dx: -0.01, dy: -0.01).contains(r))
            expect(max(r.width / r.height, r.height / r.width) < 4)
        }
        for i in rects.indices {
            for j in rects.indices where j > i {
                let overlap = rects[i].intersection(rects[j])
                expect(overlap.isNull || overlap.width * overlap.height < 0.01)
            }
        }
    }

    func treemapNestsChildrenInsideParents() {
        let tiles = TreemapLayout.layout(root: sampleTree(), in: CGRect(x: 0, y: 0, width: 800, height: 600))
        let big = tiles.first { $0.node?.name == "big" }!
        let x = tiles.first { $0.node?.name == "x.mov" }!
        expect(big.hasChildren)
        expect(big.rect.contains(x.rect))
        expect(x.depth == 2)
        let hit = TreemapLayout.hitTest(CGPoint(x: x.rect.midX, y: x.rect.midY), tiles: tiles)
        expect(hit.map { tiles[$0].node?.name } == "x.mov")
    }
}

struct AnalysisTests {
    func largestSearchAndTypes() {
        let tree = ScanTree(root: sampleTree())
        expect(Analysis.largestFiles(in: tree, under: tree.root, limit: 3).map { $0.name } == ["x.mov", "mid.mov", "y.zip"])
        expect(Analysis.search(in: tree, under: tree.root, query: "MOV").map { $0.name } == ["x.mov", "mid.mov"])
        let types = Analysis.typeBreakdown(in: tree, under: tree.root)
        expect(types.first?.category == "Video" && types.first?.bytes == 70)
        expect(types.first { $0.category == "Documents" }?.count == 10)
    }

    func pathsAndFormatting() {
        let root = sampleTree()
        expect(root.children[0].children[0].path == "/r/big/x.mov")
        expect(root.displayName == "r")
        expect(Fmt.bytes(1_500_000_000) == "1.50 GB")
        expect(Fmt.bytes(999) == "999 bytes")
    }

    func advisorPromptIsNonDestructiveAndCarriesFacts() {
        expect(Advisor.systemPrompt.contains("NON-DESTRUCTIVE"))
        expect(Advisor.systemPrompt.contains("Never recommend deleting"))
        let prompt = Advisor.userPrompt(machine: "## Mounted volumes\n- NAS", scan: "## Disk usage scan of /x")
        expect(prompt.contains("- NAS") && prompt.contains("/x"))
        let answer = "cp -av a /Volumes/nas/\n  rm -rf ~/work/a\nln -s /Volumes/nas/a ~/work/a\nsudo rm /x\nrsync -a --delete a b\nformat the report\nls && rm b"
        expect(Advisor.destructiveLines(in: answer) == ["rm -rf ~/work/a", "sudo rm /x", "rsync -a --delete a b", "ls && rm b"])
        let tree = ScanTree(root: sampleTree())
        let report = SystemFacts.scanReport(tree: tree, focus: tree.root)
        expect(report.contains("big/: 60 bytes") && report.contains("Video"))
    }
}

run("ScannerTests.totalsMatchAllocatedSizes") { try ScannerTests().totalsMatchAllocatedSizes() }
run("ScannerTests.hardLinksAreCountedOnce") { try ScannerTests().hardLinksAreCountedOnce() }
run("ScannerTests.symlinksAreNotFollowed") { try ScannerTests().symlinksAreNotFollowed() }
run("ScannerTests.unreadableDirectoryIsReportedNotFatal") { try ScannerTests().unreadableDirectoryIsReportedNotFatal() }
run("ScannerTests.manyFilesAcrossManyDirectories") { try ScannerTests().manyFilesAcrossManyDirectories() }
run("ScannerTests.detachAndReattachKeepTotalsConsistent") { try ScannerTests().detachAndReattachKeepTotalsConsistent() }
run("LayoutTests.sunburstCoversTheFullCircleAndGroupsSmallItems") { LayoutTests().sunburstCoversTheFullCircleAndGroupsSmallItems() }
run("LayoutTests.sunburstHitTesting") { LayoutTests().sunburstHitTesting() }
run("LayoutTests.squarifyFillsTheRectangleWithoutOverlap") { LayoutTests().squarifyFillsTheRectangleWithoutOverlap() }
run("LayoutTests.treemapNestsChildrenInsideParents") { LayoutTests().treemapNestsChildrenInsideParents() }
run("AnalysisTests.largestSearchAndTypes") { AnalysisTests().largestSearchAndTypes() }
run("AnalysisTests.pathsAndFormatting") { AnalysisTests().pathsAndFormatting() }
run("AnalysisTests.advisorPromptIsNonDestructiveAndCarriesFacts") { AnalysisTests().advisorPromptIsNonDestructiveAndCarriesFacts() }

print("\(checks) checks, \(failures) failures")
exit(failures == 0 ? 0 : 1)
