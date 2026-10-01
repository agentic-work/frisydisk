import AppKit

// Draws the FrisyDisk icon: a flying disc in three-quarter view.
// usage: make-icon OUT.png SIZE
let out = CommandLine.arguments[1]
let px = Int(CommandLine.arguments[2]) ?? 1024

let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: px, pixelsHigh: px, bitsPerSample: 8,
                           samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
                           colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
let s = CGFloat(px)

func rgb(_ hex: Int, _ a: CGFloat = 1) -> NSColor {
    NSColor(srgbRed: CGFloat((hex >> 16) & 255) / 255, green: CGFloat((hex >> 8) & 255) / 255,
            blue: CGFloat(hex & 255) / 255, alpha: a)
}

// Plate
let inset = s * 0.085
let plate = NSBezierPath(roundedRect: NSRect(x: inset, y: inset, width: s - 2 * inset, height: s - 2 * inset),
                         xRadius: s * 0.185, yRadius: s * 0.185)
NSGradient(starting: rgb(0x24394C), ending: rgb(0x0F1922))!.draw(in: plate, angle: -90)

NSGraphicsContext.current!.saveGraphicsState()
plate.addClip()

// Flight path: three fading arcs trailing behind the disc.
for (i, alpha) in [0.34, 0.22, 0.12].enumerated() {
    let trail = NSBezierPath()
    let dy = CGFloat(i) * s * 0.075
    trail.move(to: NSPoint(x: s * 0.10, y: s * 0.30 - dy))
    trail.curve(to: NSPoint(x: s * 0.50, y: s * 0.37 - dy * 0.55),
                controlPoint1: NSPoint(x: s * 0.22, y: s * 0.27 - dy),
                controlPoint2: NSPoint(x: s * 0.38, y: s * 0.30 - dy * 0.8))
    trail.lineWidth = s * 0.016
    trail.lineCapStyle = .round
    rgb(0x7CC4FF, CGFloat(alpha)).setStroke()
    trail.stroke()
}

// The disc, tilted.
let t = NSAffineTransform()
t.translateX(by: s * 0.53, yBy: s * 0.55)
t.rotate(byDegrees: 17)
t.concat()

func ellipse(_ w: CGFloat, _ h: CGFloat, dy: CGFloat = 0) -> NSBezierPath {
    NSBezierPath(ovalIn: NSRect(x: -w / 2, y: -h / 2 + dy, width: w, height: h))
}
let w = s * 0.66, h = s * 0.30

// Soft shadow under the disc.
rgb(0x000000, 0.28).setFill()
ellipse(w * 0.92, h * 0.5, dy: -s * 0.17).fill()

// Rim (the disc's thickness), then the top face.
rgb(0xC98A00).setFill()
ellipse(w, h, dy: -s * 0.045).fill()
let top = ellipse(w, h)
NSGradient(colors: [rgb(0xFFE79A), rgb(0xFFC83D), rgb(0xF2AE12)], atLocations: [0, 0.55, 1],
           colorSpace: .sRGB)!.draw(in: top, angle: -70)

// Ridges on the face.
rgb(0xB87D00, 0.55).setStroke()
let ridge = ellipse(w * 0.74, h * 0.72, dy: s * 0.004)
ridge.lineWidth = s * 0.012
ridge.stroke()
rgb(0xB87D00, 0.4).setStroke()
let inner = ellipse(w * 0.36, h * 0.34, dy: s * 0.006)
inner.lineWidth = s * 0.010
inner.stroke()

// Highlight along the far edge.
let shine = NSBezierPath()
shine.appendArc(withCenter: .zero, radius: 1, startAngle: 35, endAngle: 145)
let st = NSAffineTransform()
st.scaleX(by: w * 0.44, yBy: h * 0.42)
shine.transform(using: st as AffineTransform)
shine.lineWidth = s * 0.014
shine.lineCapStyle = .round
rgb(0xFFFFFF, 0.6).setStroke()
shine.stroke()

NSGraphicsContext.current!.restoreGraphicsState()
NSGraphicsContext.restoreGraphicsState()
try! rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: out))
