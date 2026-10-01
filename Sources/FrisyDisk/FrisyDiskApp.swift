import AppKit
import Quartz
import SwiftUI

@main
struct FrisyDiskApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate
    @StateObject private var model = AppModel()

    var body: some Scene {
        WindowGroup("FrisyDisk") {
            ContentView()
                .environmentObject(model)
                .frame(minWidth: 940, minHeight: 600)
                .onAppear { model.handleLaunchArguments() }
        }
        .defaultSize(width: 1240, height: 800)
        .commands {
            CommandGroup(replacing: .newItem) {
                Button("Scan Folder…") { chooseFolder(model) }
                    .keyboardShortcut("o")
                Button("Rescan") { model.rescan() }
                    .keyboardShortcut("r")
                    .disabled(model.scanner == nil)
            }
            CommandMenu("Go") {
                Button("Enclosing Folder") { model.goUp() }
                    .keyboardShortcut(.upArrow)
                    .disabled(model.focus?.parent == nil)
                Button("All Volumes") { model.closeScan() }
                    .keyboardShortcut("[")
                    .disabled(model.scanner == nil)
            }
        }
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.regular)
        NSApp.activate(ignoringOtherApps: true)
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }
}

@MainActor
func chooseFolder(_ model: AppModel) {
    let panel = NSOpenPanel()
    panel.canChooseDirectories = true
    panel.canChooseFiles = false
    panel.allowsMultipleSelection = false
    panel.prompt = "Scan"
    panel.message = "Choose a folder or volume to scan"
    if panel.runModal() == .OK, let url = panel.url {
        model.startScan(path: url.path)
    }
}

/// Shows the system Quick Look panel for one file.
final class QuickLook: NSObject, QLPreviewPanelDataSource {
    static let shared = QuickLook()
    private var url: URL?

    func show(_ url: URL) {
        self.url = url
        guard let panel = QLPreviewPanel.shared() else { return }
        panel.dataSource = self
        panel.reloadData()
        panel.makeKeyAndOrderFront(nil)
    }

    func numberOfPreviewItems(in panel: QLPreviewPanel!) -> Int { url == nil ? 0 : 1 }

    func previewPanel(_ panel: QLPreviewPanel!, previewItemAt index: Int) -> QLPreviewItem! {
        url as NSURL?
    }
}

struct ContentView: View {
    @EnvironmentObject var model: AppModel

    var body: some View {
        Group {
            if model.scanner == nil {
                StartView()
            } else {
                ScanView()
            }
        }
        .onDrop(of: [.fileURL], isTargeted: nil) { providers in
            guard let p = providers.first else { return false }
            _ = p.loadObject(ofClass: URL.self) { url, _ in
                guard let url else { return }
                var isDir: ObjCBool = false
                let path = FileManager.default.fileExists(atPath: url.path, isDirectory: &isDir) && isDir.boolValue
                    ? url.path : url.deletingLastPathComponent().path
                DispatchQueue.main.async { model.startScan(path: path) }
            }
            return true
        }
        .alert("FrisyDisk", isPresented: Binding(get: { model.notice != nil }, set: { if !$0 { model.notice = nil } })) {
            Button("OK") { model.notice = nil }
        } message: {
            Text(model.notice ?? "")
        }
    }
}

enum Palette {
    /// Colour for a chart item. `hue` is its 0...1 position around the focused folder.
    static func color(hue: Double, depth: Int, isDirectory: Bool, highlighted: Bool = false) -> Color {
        if hue < 0 { return Color.gray.opacity(0.55) }
        let d = Double(depth - 1)
        let saturation = (isDirectory ? 0.62 : 0.34) - d * 0.03
        let brightness = (highlighted ? 1.0 : 0.88) - d * 0.045
        return Color(hue: (hue * 0.92 + 0.58).truncatingRemainder(dividingBy: 1),
                     saturation: max(0.15, saturation), brightness: max(0.4, brightness))
    }

    static let group = Color.gray.opacity(0.22)
}
