// swift-tools-version:5.9
import PackageDescription

// Scripts/build.sh builds the same targets with swiftc directly, for machines
// that only have the Command Line Tools.
let package = Package(
    name: "FrisyDisk",
    platforms: [.macOS(.v14)],
    targets: [
        .target(name: "FrisyCore"),
        .executableTarget(name: "FrisyDisk", dependencies: ["FrisyCore"]),
        .executableTarget(name: "frisyscan", dependencies: ["FrisyCore"]),
        .executableTarget(name: "frisytests", dependencies: ["FrisyCore"], path: "Tests/frisytests"),
    ]
)
