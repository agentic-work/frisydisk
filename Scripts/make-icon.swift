import AppKit

// Draws the FrisyDisk icon (a small sunburst) into an .iconset folder.
let out = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "AppIcon.iconset"
try? FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)

func render(_ px: Int) -> Data {
    let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: px, pixelsHigh: px, bitsPerSample: 8,
                               samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
                               colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    let s = CGFloat(px)
    let inset = s * 0.09
    let plate = NSBezierPath(roundedRect: NSRect(x: inset, y: inset, width: s - 2 * inset, height: s - 2 * inset),
                             xRadius: s * 0.19, yRadius: s * 0.19)
    NSGradient(starting: NSColor(calibratedRed: 0.13, green: 0.15, blue: 0.22, alpha: 1),
               ending: NSColor(calibratedRed: 0.05, green: 0.06, blue: 0.10, alpha: 1))!.draw(in: plate, angle: -90)

    let c = NSPoint(x: s / 2, y: s / 2)
    // (ring, start fraction, end fraction, hue)
    let rings: [[(Double, Double, Double)]] = [
        [(0.00, 0.42, 0.58), (0.42, 0.70, 0.83), (0.70, 0.88, 0.08), (0.88, 1.00, 0.30)],
        [(0.00, 0.20, 0.55), (0.20, 0.36, 0.62), (0.42, 0.58, 0.80), (0.70, 0.80, 0.11), (0.88, 0.95, 0.33)],
        [(0.00, 0.11, 0.52), (0.20, 0.29, 0.65), (0.42, 0.50, 0.86), (0.70, 0.75, 0.13)],
    ]
    let hole = s * 0.105, width = s * 0.085
    for (i, ring) in rings.enumerated() {
        let r0 = hole + CGFloat(i) * width + s * 0.008, r1 = hole + CGFloat(i + 1) * width
        for (a, b, hue) in ring {
            let start = CGFloat(90 - a * 360), end = CGFloat(90 - b * 360 + 1.6)
            let p = NSBezierPath()
            p.appendArc(withCenter: c, radius: r1, startAngle: start, endAngle: end, clockwise: true)
            p.appendArc(withCenter: c, radius: r0, startAngle: end, endAngle: start, clockwise: false)
            p.close()
            NSColor(calibratedHue: CGFloat(hue), saturation: 0.62 - CGFloat(i) * 0.06,
                    brightness: 0.95 - CGFloat(i) * 0.08, alpha: 1).setFill()
            p.fill()
        }
    }
    NSColor(calibratedWhite: 0.96, alpha: 1).setFill()
    NSBezierPath(ovalIn: NSRect(x: c.x - hole * 0.72, y: c.y - hole * 0.72, width: hole * 1.44, height: hole * 1.44)).fill()
    NSGraphicsContext.restoreGraphicsState()
    return rep.representation(using: .png, properties: [:])!
}

for (name, px) in [("16x16", 16), ("16x16@2x", 32), ("32x32", 32), ("32x32@2x", 64), ("128x128", 128),
                   ("128x128@2x", 256), ("256x256", 256), ("256x256@2x", 512), ("512x512", 512), ("512x512@2x", 1024)] {
    try! render(px).write(to: URL(fileURLWithPath: "\(out)/icon_\(name).png"))
}
