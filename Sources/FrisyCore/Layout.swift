import CoreGraphics
import Foundation

// MARK: - Sunburst

/// One arc of the sunburst. `start`/`end` are fractions of a full turn (0...1),
/// measured clockwise from 12 o'clock. Depth 1 is the innermost ring.
public struct SunburstSegment {
    /// `nil` for the "smaller items" group that collects children too thin to draw.
    public let node: FileNode?
    public let parent: FileNode
    public let depth: Int
    public let start: Double
    public let end: Double
    public let size: Int64
    /// Number of children folded into a group segment (1 for a real node).
    public let itemCount: Int

    public var mid: Double { (start + end) / 2 }
    public var isGroup: Bool { node == nil }
}

public enum SunburstLayout {
    /// Lay out `root`'s descendants. Must be called with the tree lock held
    /// while a scan is running.
    public static func layout(root: FileNode, maxDepth: Int = 6, minFraction: Double = 0.004) -> [SunburstSegment] {
        var out: [SunburstSegment] = []
        guard root.size > 0 else { return out }
        place(root, depth: 1, start: 0, end: 1, maxDepth: maxDepth, minFraction: minFraction, into: &out)
        return out
    }

    private static func place(_ node: FileNode, depth: Int, start: Double, end: Double,
                              maxDepth: Int, minFraction: Double, into out: inout [SunburstSegment]) {
        guard depth <= maxDepth, node.size > 0 else { return }
        let span = end - start
        let total = Double(node.size)
        let kids = node.children.sorted { $0.size > $1.size }
        var cursor = start
        var groupSize: Int64 = 0
        var groupCount = 0
        for child in kids where child.size > 0 {
            let width = span * Double(child.size) / total
            if width < minFraction {
                groupSize += child.size
                groupCount += 1
                continue
            }
            let childEnd = min(end, cursor + width)
            out.append(SunburstSegment(node: child, parent: node, depth: depth,
                                       start: cursor, end: childEnd, size: child.size, itemCount: 1))
            if child.isDirectory {
                place(child, depth: depth + 1, start: cursor, end: childEnd,
                      maxDepth: maxDepth, minFraction: minFraction, into: &out)
            }
            cursor = childEnd
        }
        if groupCount > 0 {
            let width = span * Double(groupSize) / total
            if width >= minFraction / 8 {
                out.append(SunburstSegment(node: nil, parent: node, depth: depth,
                                           start: cursor, end: min(end, cursor + width),
                                           size: groupSize, itemCount: groupCount))
            }
        }
    }
}

/// Screen geometry of a sunburst, shared by drawing and hit testing.
public struct SunburstGeometry {
    public let center: CGPoint
    public let holeRadius: CGFloat
    public let ringWidth: CGFloat
    public let maxDepth: Int

    public init(size: CGSize, maxDepth: Int) {
        let radius = max(10, min(size.width, size.height) / 2 - 8)
        center = CGPoint(x: size.width / 2, y: size.height / 2)
        holeRadius = radius * 0.24
        ringWidth = (radius - holeRadius) / CGFloat(maxDepth)
        self.maxDepth = maxDepth
    }

    public func innerRadius(depth: Int) -> CGFloat { holeRadius + CGFloat(depth - 1) * ringWidth }
    public func outerRadius(depth: Int) -> CGFloat { holeRadius + CGFloat(depth) * ringWidth }

    public enum Hit: Equatable {
        case none
        case center
        case segment(Int)
    }

    public func hitTest(_ p: CGPoint, segments: [SunburstSegment]) -> Hit {
        let dx = Double(p.x - center.x), dy = Double(p.y - center.y)
        let r = (dx * dx + dy * dy).squareRoot()
        if r < Double(holeRadius) { return .center }
        let depth = Int((r - Double(holeRadius)) / Double(ringWidth)) + 1
        guard depth <= maxDepth else { return .none }
        var frac = (atan2(dy, dx) + .pi / 2) / (2 * .pi)
        if frac < 0 { frac += 1 }
        for (i, s) in segments.enumerated() where s.depth == depth && frac >= s.start && frac < s.end {
            return .segment(i)
        }
        return .none
    }
}

// MARK: - Treemap

public struct TreemapTile {
    public let node: FileNode?
    public let parent: FileNode
    public let rect: CGRect
    public let depth: Int
    /// 0...1 position used for colouring; inherited from the top-level ancestor.
    public let hue: Double
    public let size: Int64
    public let itemCount: Int
    /// The tile's children are drawn inside it.
    public let hasChildren: Bool
}

public enum TreemapLayout {
    public static let headerHeight: CGFloat = 15

    /// Squarified treemap of `root`'s descendants. Parents come before their
    /// children, so drawing in order and hit testing in reverse both work.
    public static func layout(root: FileNode, in rect: CGRect, maxDepth: Int = 5, minSide: CGFloat = 5) -> [TreemapTile] {
        var out: [TreemapTile] = []
        guard root.size > 0, rect.width > 1, rect.height > 1 else { return out }
        place(root, rect: rect, depth: 1, hue: nil, hueRange: (0, 1), maxDepth: maxDepth, minSide: minSide, into: &out)
        return out
    }

    public static func hitTest(_ p: CGPoint, tiles: [TreemapTile]) -> Int? {
        tiles.indices.reversed().first { tiles[$0].rect.contains(p) }
    }

    private static func place(_ node: FileNode, rect: CGRect, depth: Int, hue: Double?, hueRange: (Double, Double),
                              maxDepth: Int, minSide: CGFloat, into out: inout [TreemapTile]) {
        let total = Double(node.size)
        let area = Double(rect.width * rect.height)
        let minArea = Double(minSide * minSide)
        var items: [(node: FileNode?, size: Int64, count: Int)] = []
        var groupSize: Int64 = 0
        var groupCount = 0
        for child in node.children.sorted(by: { $0.size > $1.size }) where child.size > 0 {
            if Double(child.size) / total * area < minArea {
                groupSize += child.size
                groupCount += 1
            } else {
                items.append((child, child.size, 1))
            }
        }
        if groupCount > 0 { items.append((nil, groupSize, groupCount)) }
        guard !items.isEmpty else { return }

        let itemTotal = items.reduce(0.0) { $0 + Double($1.size) }
        let rects = squarify(items.map { Double($0.size) / itemTotal * area }, in: rect)
        var cursor = 0.0
        for (item, r) in zip(items, rects) {
            let share = Double(item.size) / itemTotal
            let lo = hueRange.0 + (hueRange.1 - hueRange.0) * cursor
            let hi = hueRange.0 + (hueRange.1 - hueRange.0) * (cursor + share)
            cursor += share
            let tileHue = hue ?? (lo + hi) / 2
            let inner = r.insetBy(dx: 2, dy: 2)
            let canNest = item.node?.isDirectory == true && depth < maxDepth
                && inner.width > minSide * 2 && inner.height > minSide * 2 + headerHeight
            out.append(TreemapTile(node: item.node, parent: node, rect: r, depth: depth, hue: tileHue,
                                   size: item.size, itemCount: item.count, hasChildren: canNest))
            if canNest, let child = item.node {
                let body = CGRect(x: inner.minX, y: inner.minY + headerHeight,
                                  width: inner.width, height: inner.height - headerHeight)
                place(child, rect: body, depth: depth + 1, hue: tileHue, hueRange: (lo, hi),
                      maxDepth: maxDepth, minSide: minSide, into: &out)
            }
        }
    }

    /// Squarified treemap (Bruls, Huizing, van Wijk). `areas` must be sorted
    /// descending and sum to the area of `rect`.
    public static func squarify(_ areas: [Double], in rect: CGRect) -> [CGRect] {
        var result: [CGRect] = []
        result.reserveCapacity(areas.count)
        var free = rect
        var i = 0
        while i < areas.count {
            let side = Double(min(free.width, free.height))
            guard side > 0 else {
                result.append(contentsOf: Array(repeating: CGRect(origin: free.origin, size: .zero), count: areas.count - i))
                break
            }
            var row = [areas[i]]
            var sum = areas[i]
            var j = i + 1
            while j < areas.count {
                let next = areas[j]
                if worst(row + [next], sum + next, side) <= worst(row, sum, side) {
                    row.append(next)
                    sum += next
                    j += 1
                } else {
                    break
                }
            }
            let thickness = CGFloat(sum / side)
            var offset: CGFloat = 0
            let horizontal = free.width >= free.height  // row stacks along the short (vertical) side
            for a in row {
                let len = CGFloat(a / sum * side)
                if horizontal {
                    result.append(CGRect(x: free.minX, y: free.minY + offset, width: thickness, height: len))
                } else {
                    result.append(CGRect(x: free.minX + offset, y: free.minY, width: len, height: thickness))
                }
                offset += len
            }
            if horizontal {
                free = CGRect(x: free.minX + thickness, y: free.minY, width: max(0, free.width - thickness), height: free.height)
            } else {
                free = CGRect(x: free.minX, y: free.minY + thickness, width: free.width, height: max(0, free.height - thickness))
            }
            i = j
        }
        return result
    }

    private static func worst(_ row: [Double], _ sum: Double, _ side: Double) -> Double {
        guard let maxA = row.max(), let minA = row.min(), minA > 0, sum > 0 else { return .infinity }
        let s2 = sum * sum, w2 = side * side
        return max(w2 * maxA / s2, s2 / (w2 * minA))
    }
}
