import Foundation
import FrisyCore

// frisyscan PATH [--top N] [--advise [MODEL]] [--facts]
// Command-line front end to the FrisyDisk scanner and advisor.

var args = Array(CommandLine.arguments.dropFirst())
func flag(_ name: String) -> Bool {
    if let i = args.firstIndex(of: name) {
        args.remove(at: i)
        return true
    }
    return false
}
func option(_ name: String) -> String? {
    guard let i = args.firstIndex(of: name) else { return nil }
    args.remove(at: i)
    guard i < args.count, !args[i].hasPrefix("--") else { return "" }
    return args.remove(at: i)
}

let top = Int(option("--top") ?? "") ?? 15
let adviseModel = option("--advise")
let factsOnly = flag("--facts")

if factsOnly {
    print(SystemFacts.machineReport())
    exit(0)
}
guard let path = args.first else {
    FileHandle.standardError.write(Data("usage: frisyscan PATH [--top N] [--advise [MODEL]] | --facts\n".utf8))
    exit(2)
}

let scanner = DiskScanner(path: path)
let started = Date()
scanner.run()
let elapsed = Date().timeIntervalSince(started)
let root = scanner.tree.root
let p = scanner.progress

print("\(root.path)")
print("bytes=\(root.size) files=\(root.fileCount) dirs=\(p.directories) inaccessible=\(p.inaccessible) seconds=\(String(format: "%.2f", elapsed))")
print("total \(Fmt.bytes(root.size)) in \(Fmt.count(root.fileCount)) files")
for c in root.children.prefix(top) {
    print(String(format: "%12@  %@%@", Fmt.bytes(c.size) as NSString, c.name as NSString, (c.isDirectory ? "/" : "") as NSString))
}

if let adviseModel {
    let models = await Advisor.ollamaModels()
    let backends = await Advisor.availableBackends()
    let backend: AdvisorBackend
    if !adviseModel.isEmpty {
        backend = adviseModel == "agenticode" ? .agenticode : .ollama(model: adviseModel)
    } else if let first = backends.first {
        backend = first
    } else {
        FileHandle.standardError.write(Data("no local model backend found (ollama models: \(models))\n".utf8))
        exit(1)
    }
    print("\n--- advisor (\(backend.label)) ---")
    let messages = [
        ChatMessage(role: .system, text: Advisor.systemPrompt),
        ChatMessage(role: .user, text: Advisor.userPrompt(machine: SystemFacts.machineReport(),
                                                          scan: SystemFacts.scanReport(tree: scanner.tree, focus: root))),
    ]
    do {
        for try await chunk in Advisor.stream(messages, backend: backend) {
            FileHandle.standardOutput.write(Data(chunk.utf8))
        }
        print()
    } catch {
        FileHandle.standardError.write(Data("advisor failed: \(error.localizedDescription)\n".utf8))
        exit(1)
    }
}
