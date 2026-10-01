import CoreGraphics
import Foundation

// Prints the window id of the first on-screen window owned by the named app,
// so `screencapture -l <id>` can capture that window and nothing else.
let name = CommandLine.arguments.count > 1 ? CommandLine.arguments[1].lowercased() : "frisydisk"
let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] ?? []
for w in list {
    let owner = (w[kCGWindowOwnerName as String] as? String ?? "").lowercased()
    let layer = w[kCGWindowLayer as String] as? Int ?? 1
    if owner == name, layer == 0, let id = w[kCGWindowNumber as String] as? Int {
        print(id)
        exit(0)
    }
}
exit(1)
