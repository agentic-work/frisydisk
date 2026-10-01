import FrisyCore
import SwiftUI

struct ScanView: View {
    @EnvironmentObject var model: AppModel

    var body: some View {
        VStack(spacing: 0) {
            TopBar()
            Divider()
            HSplitView {
                VStack(spacing: 0) {
                    ChartView()
                    Divider()
                    StatusBar()
                }
                .frame(minWidth: 460)
                Sidebar()
                    .frame(minWidth: 340, idealWidth: 400, maxWidth: 620)
            }
        }
    }
}

private struct TopBar: View {
    @EnvironmentObject var model: AppModel

    var body: some View {
        HStack(spacing: 10) {
            Button {
                model.closeScan()
            } label: {
                Label("Volumes", systemImage: "chevron.left")
            }
            .help("Back to all volumes")

            Button {
                model.goUp()
            } label: {
                Image(systemName: "arrow.up")
            }
            .disabled(model.focus?.parent == nil)
            .help("Enclosing folder")

            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 2) {
                    ForEach(Array(model.crumbs.enumerated()), id: \.offset) { i, node in
                        if i > 0 {
                            Image(systemName: "chevron.right")
                                .font(.caption2)
                                .foregroundStyle(.tertiary)
                        }
                        Button(node.displayName) { model.setFocus(node) }
                            .buttonStyle(.plain)
                            .font(.callout.weight(i == model.crumbs.count - 1 ? .semibold : .regular))
                            .foregroundStyle(i == model.crumbs.count - 1 ? .primary : .secondary)
                            .padding(.horizontal, 4)
                    }
                }
            }
            .defaultScrollAnchor(.trailing)

            Spacer(minLength: 8)

            Picker("", selection: Binding(get: { model.mode }, set: { model.setMode($0) })) {
                ForEach(ChartMode.allCases) { Text($0.rawValue).tag($0) }
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .fixedSize()

            if model.isScanning {
                Button("Stop") { model.stopScan() }
            } else {
                Button {
                    model.rescan()
                } label: {
                    Image(systemName: "arrow.clockwise")
                }
                .help("Rescan")
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(Color(nsColor: .windowBackgroundColor))
    }
}

private struct StatusBar: View {
    @EnvironmentObject var model: AppModel

    var body: some View {
        HStack(spacing: 8) {
            if let h = model.hover {
                Image(systemName: h.node == nil ? "ellipsis.circle" : (h.isDirectory ? "folder.fill" : "doc"))
                    .foregroundStyle(.secondary)
                Text(h.name).lineLimit(1).truncationMode(.middle)
                Text(Fmt.bytes(h.size)).fontWeight(.semibold).monospacedDigit()
                if h.isDirectory {
                    Text("\(Fmt.count(h.fileCount)) files").foregroundStyle(.secondary)
                }
                if model.focusSize > 0 {
                    Text(String(format: "%.1f%%", Double(h.size) / Double(model.focusSize) * 100))
                        .foregroundStyle(.secondary)
                }
            } else if model.isScanning {
                ProgressView().controlSize(.small)
                Text("Scanning… \(Fmt.count(model.progress.files)) files, \(Fmt.bytes(model.progress.bytes))")
                    .monospacedDigit()
            } else {
                Text("\(Fmt.bytes(model.scanner?.tree.root.size ?? 0)) in \(Fmt.count(model.progress.files)) files, scanned in \(String(format: "%.1f", model.scanSeconds)) s")
                    .foregroundStyle(.secondary)
                    .monospacedDigit()
                if model.progress.inaccessible > 0 {
                    Button("\(Fmt.count(model.progress.inaccessible)) folders could not be read") {
                        if let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles") {
                            NSWorkspace.shared.open(url)
                        }
                    }
                    .buttonStyle(.link)
                    .help("Grant FrisyDisk Full Disk Access in System Settings, then rescan")
                }
            }
            Spacer()
        }
        .font(.callout)
        .padding(.horizontal, 12)
        .frame(height: 30)
        .background(Color(nsColor: .windowBackgroundColor))
    }
}

struct Sidebar: View {
    @EnvironmentObject var model: AppModel

    var body: some View {
        VStack(spacing: 0) {
            Picker("", selection: $model.tab) {
                ForEach(SideTab.allCases) { Text($0.rawValue).tag($0) }
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .padding(10)

            if model.tab != .advisor && model.tab != .types {
                HStack(spacing: 6) {
                    Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
                    TextField(model.isScanning ? "Search is available when the scan finishes" : "Search in this folder",
                              text: $model.searchText)
                        .textFieldStyle(.plain)
                        .disabled(model.isScanning)
                    if !model.searchText.isEmpty {
                        Button {
                            model.searchText = ""
                        } label: {
                            Image(systemName: "xmark.circle.fill").foregroundStyle(.secondary)
                        }
                        .buttonStyle(.plain)
                    }
                }
                .padding(.horizontal, 8)
                .padding(.vertical, 6)
                .background(RoundedRectangle(cornerRadius: 7).fill(Color.primary.opacity(0.06)))
                .padding(.horizontal, 10)
                .padding(.bottom, 8)
            }
            Divider()

            switch model.tab {
            case .advisor:
                AdvisorView()
            case .types:
                TypesList()
            case .contents, .largest:
                if model.searchText.trimmingCharacters(in: .whitespaces).count >= 2 {
                    RowList(rows: model.searchResults, empty: "No matches in this folder")
                } else if model.tab == .contents {
                    RowList(rows: model.rows, empty: model.isScanning ? "Scanning…" : "Empty folder")
                } else {
                    RowList(rows: model.largest, empty: model.isScanning ? "Available when the scan finishes" : "No files")
                }
            }

            if !model.collector.isEmpty {
                Divider()
                CollectorBar()
            }
        }
        .background(Color(nsColor: .windowBackgroundColor))
        .onChange(of: model.tab) { _, new in
            if new == .advisor { model.loadBackends() }
        }
        .onAppear {
            if model.tab == .advisor { model.loadBackends() }
        }
    }
}

private struct RowList: View {
    @EnvironmentObject var model: AppModel
    let rows: [Row]
    let empty: String

    var body: some View {
        if rows.isEmpty {
            Text(empty)
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else {
            ScrollView {
                LazyVStack(spacing: 0) {
                    ForEach(rows) { row in
                        RowView(row: row, hovered: model.hover?.node === row.node)
                    }
                }
                .padding(.vertical, 4)
            }
        }
    }
}

private struct RowView: View {
    @EnvironmentObject var model: AppModel
    let row: Row
    let hovered: Bool

    var body: some View {
        HStack(spacing: 8) {
            RoundedRectangle(cornerRadius: 3)
                .fill(Palette.color(hue: row.hue, depth: 1, isDirectory: row.isDirectory))
                .frame(width: 10, height: 10)
            Image(systemName: row.isDirectory ? "folder.fill" : "doc")
                .foregroundStyle(row.isDirectory ? Color.accentColor : Color.secondary)
                .frame(width: 16)
            VStack(alignment: .leading, spacing: 1) {
                Text(row.name)
                    .lineLimit(1)
                    .truncationMode(.middle)
                if let sub = row.subtitle {
                    Text(sub)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.head)
                }
            }
            Spacer(minLength: 6)
            if hovered && model.canStage(row.node) {
                Button {
                    model.stage(row.node)
                } label: {
                    Image(systemName: "tray.and.arrow.down")
                }
                .buttonStyle(.plain)
                .foregroundStyle(.secondary)
                .help("Add to Collector")
            }
            Text(Fmt.bytes(row.size))
                .monospacedDigit()
                .foregroundStyle(.secondary)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 5)
        .background(alignment: .leading) {
            GeometryReader { geo in
                Rectangle()
                    .fill(Color.accentColor.opacity(hovered ? 0.22 : 0.09))
                    .frame(width: geo.size.width * min(1, row.fraction))
            }
        }
        .background(hovered ? Color.primary.opacity(0.06) : .clear)
        .contentShape(Rectangle())
        .onHover { inside in
            if inside { model.hover(node: row.node) } else if model.hover?.node === row.node { model.hover(node: nil) }
        }
        .onTapGesture(count: 2) {
            if !row.isDirectory { model.quickLook(row.node) }
        }
        .onTapGesture {
            if row.isDirectory { model.setFocus(row.node) }
        }
        .contextMenu { NodeMenu(node: row.node) }
    }
}

private struct TypesList: View {
    @EnvironmentObject var model: AppModel

    var body: some View {
        if model.types.isEmpty {
            Text(model.isScanning ? "Available when the scan finishes" : "No files")
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else {
            let top = Double(model.types.first?.bytes ?? 1)
            ScrollView {
                VStack(spacing: 10) {
                    ForEach(model.types) { b in
                        VStack(alignment: .leading, spacing: 4) {
                            HStack {
                                Text(b.category).fontWeight(.medium)
                                Text("\(Fmt.count(b.count)) file\(b.count == 1 ? "" : "s")").font(.caption).foregroundStyle(.secondary)
                                Spacer()
                                Text(Fmt.bytes(b.bytes)).monospacedDigit().foregroundStyle(.secondary)
                            }
                            GeometryReader { geo in
                                ZStack(alignment: .leading) {
                                    Capsule().fill(Color.primary.opacity(0.08))
                                    Capsule().fill(Color.accentColor)
                                        .frame(width: max(3, geo.size.width * Double(b.bytes) / max(1, top)))
                                }
                            }
                            .frame(height: 6)
                        }
                    }
                }
                .padding(12)
            }
        }
    }
}

private struct CollectorBar: View {
    @EnvironmentObject var model: AppModel
    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "tray.full.fill").foregroundStyle(.orange)
            VStack(alignment: .leading, spacing: 0) {
                Text("Collector").fontWeight(.semibold)
                Text("\(model.collector.count) item\(model.collector.count == 1 ? "" : "s") · \(Fmt.bytes(model.collectorSize))")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .monospacedDigit()
            }
            Spacer()
            Button("Review") { model.showCollectorItems = true }
                .popover(isPresented: $model.showCollectorItems, arrowEdge: .top) {
                    VStack(alignment: .leading, spacing: 8) {
                        Text("Nothing here has been changed on disk.")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        ForEach(model.collector) { item in
                            HStack {
                                VStack(alignment: .leading) {
                                    Text(item.node.name).lineLimit(1)
                                    Text(item.path).font(.caption).foregroundStyle(.secondary).lineLimit(1).truncationMode(.head)
                                }
                                Spacer()
                                Text(Fmt.bytes(item.node.size)).monospacedDigit().foregroundStyle(.secondary)
                                Button("Put Back") { model.restore(item) }
                            }
                        }
                        Button("Put Everything Back") {
                            model.restoreAll()
                            model.showCollectorItems = false
                        }
                    }
                    .padding(14)
                    .frame(width: 440)
                }
            Button("Move to Trash…") { model.confirmTrash = true }
                .buttonStyle(.borderedProminent)
                .tint(.orange)
        }
        .padding(10)
        .confirmationDialog("Move \(model.collector.count) item\(model.collector.count == 1 ? "" : "s") (\(Fmt.bytes(model.collectorSize))) to the Trash?",
                            isPresented: $model.confirmTrash) {
            Button("Move to Trash", role: .destructive) { model.trashCollector() }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("The items go to the Trash, so you can still put them back from there. Space is freed only when you empty the Trash.")
        }
    }
}
