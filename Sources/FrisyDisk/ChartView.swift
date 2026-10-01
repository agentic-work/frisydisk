import FrisyCore
import SwiftUI

struct ChartView: View {
    @EnvironmentObject var model: AppModel

    var body: some View {
        GeometryReader { geo in
            ZStack {
                Canvas { ctx, size in
                    switch model.mode {
                    case .sunburst: drawSunburst(&ctx, size: size)
                    case .treemap: drawTreemap(&ctx)
                    }
                }
                if model.mode == .sunburst {
                    centerLabel(size: geo.size)
                }
                if model.focusSize == 0 && !model.isScanning {
                    Text("This folder is empty")
                        .foregroundStyle(.secondary)
                }
            }
            .contentShape(Rectangle())
            .onContinuousHover { phase in
                switch phase {
                case .active(let p): model.hoverChart(at: p)
                case .ended: model.hoverChart(at: nil)
                }
            }
            .gesture(SpatialTapGesture().onEnded { model.clickChart(at: $0.location) })
            .contextMenu {
                if let node = model.hover?.node { NodeMenu(node: node) }
            }
            .onAppear { model.chartResized(geo.size) }
            .onChange(of: geo.size) { _, new in model.chartResized(new) }
        }
        .background(Color(nsColor: .underPageBackgroundColor))
    }

    // MARK: Sunburst

    private func drawSunburst(_ ctx: inout GraphicsContext, size: CGSize) {
        let geo = SunburstGeometry(size: size, maxDepth: AppModel.sunburstDepth)
        let gap = Color(nsColor: .underPageBackgroundColor)
        let hovered = model.hover?.node
        var highlight: Path?

        for s in model.segments {
            let a0 = Angle(radians: s.start * 2 * .pi - .pi / 2)
            let a1 = Angle(radians: s.end * 2 * .pi - .pi / 2)
            var path = Path()
            path.addArc(center: geo.center, radius: geo.outerRadius(depth: s.depth), startAngle: a0, endAngle: a1, clockwise: false)
            path.addArc(center: geo.center, radius: geo.innerRadius(depth: s.depth), startAngle: a1, endAngle: a0, clockwise: true)
            path.closeSubpath()

            let isHovered = hovered != nil && s.node === hovered
            let color: Color = s.isGroup
                ? Palette.group
                : Palette.color(hue: s.mid, depth: s.depth, isDirectory: s.node?.isDirectory ?? false, highlighted: isHovered)
            ctx.fill(path, with: .color(color))
            ctx.stroke(path, with: .color(gap), lineWidth: 1)
            if isHovered { highlight = path }
        }
        if let highlight {
            ctx.stroke(highlight, with: .color(.primary), lineWidth: 2)
        }

        let hole = Path(ellipseIn: CGRect(x: geo.center.x - geo.holeRadius + 3, y: geo.center.y - geo.holeRadius + 3,
                                          width: (geo.holeRadius - 3) * 2, height: (geo.holeRadius - 3) * 2))
        ctx.fill(hole, with: .color(Color(nsColor: .controlBackgroundColor)))
    }

    private func centerLabel(size: CGSize) -> some View {
        let geo = SunburstGeometry(size: size, maxDepth: AppModel.sunburstDepth)
        let side = geo.holeRadius * 1.5
        return VStack(spacing: 3) {
            Text(model.focus?.displayName ?? "")
                .font(.system(size: 13, weight: .semibold))
                .lineLimit(2)
                .multilineTextAlignment(.center)
            Text(Fmt.bytes(model.focusSize))
                .font(.system(size: 20, weight: .bold, design: .rounded))
                .monospacedDigit()
            Text("\(Fmt.count(model.focusFiles)) files")
                .font(.caption)
                .foregroundStyle(.secondary)
            if model.focus?.parent != nil {
                Image(systemName: "chevron.up")
                    .font(.caption2)
                    .foregroundStyle(.tertiary)
            }
        }
        .minimumScaleFactor(0.6)
        .frame(width: side, height: side)
        .allowsHitTesting(false)
    }

    // MARK: Treemap

    private func drawTreemap(_ ctx: inout GraphicsContext) {
        let hovered = model.hover?.node
        var highlight: CGRect?
        for t in model.tiles {
            let r = t.rect.insetBy(dx: 0.5, dy: 0.5)
            let isHovered = hovered != nil && t.node === hovered
            let shape = Path(roundedRect: r, cornerRadius: t.hasChildren ? 3 : 2)
            if t.node == nil {
                ctx.fill(shape, with: .color(Palette.group))
            } else if t.hasChildren {
                ctx.fill(shape, with: .color(Palette.color(hue: t.hue, depth: t.depth, isDirectory: true).opacity(0.28)))
            } else {
                ctx.fill(shape, with: .color(Palette.color(hue: t.hue, depth: t.depth, isDirectory: t.node?.isDirectory ?? false,
                                                           highlighted: isHovered)))
            }
            if isHovered { highlight = r }

            let name = t.node?.name ?? "\(t.itemCount) smaller"
            if t.hasChildren, r.width > 50 {
                label(&ctx, "\(name)  \(Fmt.bytes(t.size))", in: CGRect(x: r.minX + 4, y: r.minY + 2, width: r.width - 8,
                                                                      height: TreemapLayout.headerHeight), size: 10, weight: .semibold)
            } else if !t.hasChildren, r.width > 64, r.height > 18 {
                label(&ctx, name, in: CGRect(x: r.minX + 4, y: r.minY + 3, width: r.width - 8, height: 14), size: 10, weight: .medium,
                      color: .black.opacity(0.8))
                if r.height > 34 {
                    label(&ctx, Fmt.bytes(t.size), in: CGRect(x: r.minX + 4, y: r.minY + 16, width: r.width - 8, height: 14),
                          size: 10, weight: .regular, color: .black.opacity(0.6))
                }
            }
        }
        if let highlight {
            ctx.stroke(Path(roundedRect: highlight, cornerRadius: 2), with: .color(.primary), lineWidth: 2)
        }
    }

    private func label(_ ctx: inout GraphicsContext, _ text: String, in rect: CGRect, size: CGFloat,
                       weight: Font.Weight, color: Color = .primary) {
        ctx.drawLayer { layer in
            layer.clip(to: Path(rect))
            layer.draw(Text(text).font(.system(size: size, weight: weight)).foregroundColor(color),
                       at: CGPoint(x: rect.minX, y: rect.minY), anchor: .topLeading)
        }
    }
}

/// Actions for one file or folder, shared by the chart and the lists.
struct NodeMenu: View {
    @EnvironmentObject var model: AppModel
    let node: FileNode

    var body: some View {
        if node.isDirectory {
            Button("Open in FrisyDisk") { model.setFocus(node) }
        }
        Button("Reveal in Finder") { model.reveal(node) }
        Button("Quick Look") { model.quickLook(node) }
        Button("Copy Path") { model.copyPath(node) }
        Divider()
        Button("Add to Collector") { model.stage(node) }
            .disabled(!model.canStage(node))
    }
}
