import FrisyCore
import SwiftUI

struct StartView: View {
    @EnvironmentObject var model: AppModel

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                VStack(alignment: .leading, spacing: 4) {
                    Text("FrisyDisk")
                        .font(.system(size: 34, weight: .bold, design: .rounded))
                    Text("See what is using your disks. Pick a volume, or drop a folder anywhere on this window.")
                        .foregroundStyle(.secondary)
                }
                .padding(.bottom, 6)

                ForEach(model.volumes) { v in
                    VolumeCard(volume: v)
                }

                HStack(spacing: 10) {
                    Button {
                        model.startScan(path: NSHomeDirectory())
                    } label: {
                        Label("Scan Home Folder", systemImage: "house")
                    }
                    Button {
                        chooseFolder(model)
                    } label: {
                        Label("Scan Folder…", systemImage: "folder")
                    }
                    Spacer()
                    Button {
                        model.volumes = SystemFacts.volumes()
                    } label: {
                        Label("Refresh", systemImage: "arrow.clockwise")
                    }
                }
                .controlSize(.large)
                .padding(.top, 4)
            }
            .padding(32)
            .frame(maxWidth: 820, alignment: .leading)
            .frame(maxWidth: .infinity)
        }
        .background(Color(nsColor: .windowBackgroundColor))
    }
}

struct VolumeCard: View {
    @EnvironmentObject var model: AppModel
    let volume: VolumeInfo

    private var barColor: Color {
        volume.usedFraction > 0.9 ? .red : (volume.usedFraction > 0.75 ? .orange : .accentColor)
    }

    var body: some View {
        HStack(spacing: 16) {
            Image(systemName: volume.isNetwork ? "externaldrive.connected.to.line.below.fill"
                : (volume.mountPoint == "/" ? "internaldrive.fill" : "externaldrive.fill"))
                .font(.system(size: 30))
                .foregroundStyle(barColor)
                .frame(width: 44)

            VStack(alignment: .leading, spacing: 6) {
                HStack(alignment: .firstTextBaseline) {
                    Text(volume.name).font(.headline)
                    Text(volume.isNetwork ? "Network share · \(volume.source)" : volume.fsType.uppercased())
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                    Spacer()
                    Text("\(Fmt.bytes(volume.free)) free of \(Fmt.bytes(volume.total))")
                        .font(.callout)
                        .monospacedDigit()
                        .foregroundStyle(.secondary)
                }
                GeometryReader { geo in
                    ZStack(alignment: .leading) {
                        Capsule().fill(Color.primary.opacity(0.1))
                        Capsule().fill(barColor)
                            .frame(width: max(4, geo.size.width * volume.usedFraction))
                    }
                }
                .frame(height: 8)
            }

            Button("Scan") { model.startScan(path: volume.mountPoint) }
                .buttonStyle(.borderedProminent)
                .controlSize(.large)
        }
        .padding(16)
        .background(RoundedRectangle(cornerRadius: 12).fill(Color(nsColor: .controlBackgroundColor)))
        .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(Color.primary.opacity(0.08)))
    }
}
