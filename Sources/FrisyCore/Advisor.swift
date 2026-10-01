import Foundation

public struct ChatMessage: Sendable, Equatable {
    public enum Role: String, Sendable { case system, user, assistant }
    public let role: Role
    public var text: String
    public init(role: Role, text: String) {
        self.role = role
        self.text = text
    }
}

public enum AdvisorBackend: Hashable, Sendable {
    case ollama(model: String)
    case agenticode

    public var label: String {
        switch self {
        case .ollama(let m): return "Ollama · \(m)"
        case .agenticode: return "agenticode"
        }
    }
}

public enum AdvisorError: LocalizedError {
    case unavailable(String)
    public var errorDescription: String? {
        switch self {
        case .unavailable(let why): return why
        }
    }
}

/// Asks a model running on this Mac for storage and I/O suggestions.
/// The advisor only ever produces text; it never runs anything it suggests.
public enum Advisor {
    public static let systemPrompt = """
    You are a storage and I/O advisor built into FrisyDisk, a macOS disk-usage app. \
    You are given facts collected from this one Mac: its volumes (including any NAS or network shares), \
    its startup disk, and a disk-usage scan.

    Rules:
    - Suggest NON-DESTRUCTIVE improvements only. Never recommend deleting, erasing, reformatting or \
    overwriting data. Prefer moving, archiving to the NAS, relocating caches or model stores, symlinks, \
    compression, deduplication by cloning, mount and SMB tuning, snapshot and backup settings, and scheduling.
    - No step may contain rm, "delete", "clear", "purge", "empty" or "erase" as an action, not even for caches, \
    logs or build output. If something looks disposable, suggest moving it to the NAS instead, where it can be \
    brought back.
    - Every suggestion must be reversible, and you must say how to undo it.
    - Copy first, verify the copy, and only then switch over (for example with a symlink). Say so in the steps.
    - Ground every suggestion in the facts given. Name the actual folders, shares and sizes. \
    If a fact you need is missing, say what to check instead of guessing.
    - You cannot run anything. Show commands for the person to review and run themselves.

    Answer in Markdown. Give 5 to 8 suggestions ranked by benefit. For each: a short title, \
    what to do, why it helps (space freed on the startup disk or I/O gained), how (concrete steps or commands), \
    and risk plus how to undo. Finish with one line naming the single best first step.
    """

    public static func userPrompt(machine: String, scan: String?) -> String {
        var s = "Here are the facts about this Mac.\n\n" + machine
        if let scan { s += "\n\n" + scan }
        s += "\n\nWhat non-destructive changes would most improve I/O performance and make better use of the storage that already exists here?"
        return s
    }

    /// Lines of a model answer that would delete or overwrite data if run.
    /// Models do not always follow the non-destructive rule, so the app checks.
    public static func destructiveLines(in answer: String) -> [String] {
        let patterns = [#"(^|[\s;&|])(sudo\s+)?rm\s"#, #"\bdiskutil\s+(erase|reformat|apfs\s+delete)"#, #"\bnewfs|\bmkfs"#,
                        #"\bdd\s+.*\bof="#, #"\btmutil\s+delete"#, #"\bfind\b.*-delete\b"#, #"rsync\b.*--(delete|remove-source-files)"#,
                        #"\bmv\s+.*\s~?/?\.Trash"#, #"\bshred\b|\bsrm\b"#]
        return answer.components(separatedBy: "\n").map { $0.trimmingCharacters(in: .whitespaces) }.filter { line in
            !line.isEmpty && patterns.contains { line.range(of: $0, options: .regularExpression) != nil }
        }
    }

    // MARK: Backends

    public static var ollamaBase = URL(string: "http://127.0.0.1:11434")!

    /// Text-capable Ollama models installed locally, biggest first. Empty if Ollama is not running.
    public static func ollamaModels() async -> [String] {
        var req = URLRequest(url: ollamaBase.appendingPathComponent("api/tags"))
        req.timeoutInterval = 3
        guard let (data, _) = try? await URLSession.shared.data(for: req),
              let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let models = obj["models"] as? [[String: Any]] else { return [] }
        return models
            .filter { m in
                let caps = m["capabilities"] as? [String] ?? ["completion"]
                let size = (m["size"] as? NSNumber)?.int64Value ?? 0
                // Skip embedding-only models and tiny vision models that give poor advice.
                return caps.contains("completion") && size > 3_000_000_000
            }
            .sorted { (($0["size"] as? NSNumber)?.int64Value ?? 0) > (($1["size"] as? NSNumber)?.int64Value ?? 0) }
            .compactMap { $0["name"] as? String }
    }

    public static let agenticodePaths = [
        NSHomeDirectory() + "/.local/bin/agenticode",
        "/usr/local/bin/agenticode",
        "/opt/homebrew/bin/agenticode",
    ]

    /// Path of an agenticode CLI that actually launches, if any.
    public static func agenticodeCLI() -> String? {
        agenticodePaths.first { SystemFacts.run($0, ["--version"], timeout: 8) != nil }
    }

    public static func availableBackends() async -> [AdvisorBackend] {
        var out: [AdvisorBackend] = []
        if agenticodeCLI() != nil { out.append(.agenticode) }
        out += await ollamaModels().map { .ollama(model: $0) }
        return out
    }

    /// Stream the assistant's reply. Chunks are partial text to append.
    public static func stream(_ messages: [ChatMessage], backend: AdvisorBackend) -> AsyncThrowingStream<String, Error> {
        switch backend {
        case .ollama(let model): return streamOllama(messages, model: model)
        case .agenticode: return streamAgenticode(messages)
        }
    }

    private static func streamOllama(_ messages: [ChatMessage], model: String) -> AsyncThrowingStream<String, Error> {
        AsyncThrowingStream { continuation in
            let task = Task {
                do {
                    var req = URLRequest(url: ollamaBase.appendingPathComponent("api/chat"))
                    req.httpMethod = "POST"
                    req.timeoutInterval = 600
                    req.setValue("application/json", forHTTPHeaderField: "Content-Type")
                    req.httpBody = try JSONSerialization.data(withJSONObject: [
                        "model": model,
                        "stream": true,
                        "think": false,
                        "options": ["temperature": 0.3, "num_ctx": 12288, "num_predict": 1800],
                        "messages": messages.map { ["role": $0.role.rawValue, "content": $0.text] },
                    ] as [String: Any])
                    let (bytes, response) = try await URLSession.shared.bytes(for: req)
                    if let http = response as? HTTPURLResponse, http.statusCode != 200 {
                        var body = ""
                        for try await line in bytes.lines { body += line }
                        throw AdvisorError.unavailable("Ollama returned HTTP \(http.statusCode): \(body.prefix(300))")
                    }
                    for try await line in bytes.lines {
                        guard let obj = try? JSONSerialization.jsonObject(with: Data(line.utf8)) as? [String: Any] else { continue }
                        if let err = obj["error"] as? String { throw AdvisorError.unavailable("Ollama: \(err)") }
                        if let msg = obj["message"] as? [String: Any], let text = msg["content"] as? String, !text.isEmpty {
                            continuation.yield(text)
                        }
                        if obj["done"] as? Bool == true { break }
                    }
                    continuation.finish()
                } catch {
                    continuation.finish(throwing: error)
                }
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }

    /// agenticode is an agent CLI. It is run in print mode with every tool
    /// disallowed, so it can only answer in text.
    private static func streamAgenticode(_ messages: [ChatMessage]) -> AsyncThrowingStream<String, Error> {
        AsyncThrowingStream { continuation in
            guard let cli = agenticodeCLI() else {
                continuation.finish(throwing: AdvisorError.unavailable("The agenticode CLI could not be launched."))
                return
            }
            let prompt = messages.map { m -> String in
                switch m.role {
                case .system: return m.text
                case .user: return "USER:\n" + m.text
                case .assistant: return "ASSISTANT:\n" + m.text
                }
            }.joined(separator: "\n\n")
            let p = Process()
            p.executableURL = URL(fileURLWithPath: cli)
            p.arguments = ["-p", prompt, "--allowedTools", ""]
            p.currentDirectoryURL = FileManager.default.temporaryDirectory
            let pipe = Pipe()
            p.standardOutput = pipe
            p.standardError = FileHandle.nullDevice
            p.standardInput = FileHandle.nullDevice
            pipe.fileHandleForReading.readabilityHandler = { h in
                let d = h.availableData
                if !d.isEmpty { continuation.yield(String(decoding: d, as: UTF8.self)) }
            }
            p.terminationHandler = { proc in
                pipe.fileHandleForReading.readabilityHandler = nil
                if proc.terminationStatus == 0 {
                    continuation.finish()
                } else {
                    continuation.finish(throwing: AdvisorError.unavailable("agenticode exited with status \(proc.terminationStatus)."))
                }
            }
            do { try p.run() } catch { continuation.finish(throwing: error) }
            continuation.onTermination = { _ in if p.isRunning { p.terminate() } }
        }
    }
}
