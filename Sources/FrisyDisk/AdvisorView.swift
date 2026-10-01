import FrisyCore
import SwiftUI

/// Asks a model running on this Mac for non-destructive storage and I/O ideas.
struct AdvisorView: View {
    @EnvironmentObject var model: AppModel
    private var replies: [ChatMessage] {
        // Skip the system prompt and the long facts message.
        Array(model.chat.dropFirst(2))
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            if model.chat.isEmpty && model.advisorError == nil {
                intro
            } else {
                transcript
            }
            if !model.chat.isEmpty {
                Divider()
                followUp
            }
        }
    }

    private var header: some View {
        HStack(spacing: 8) {
            if model.backends.isEmpty {
                Text(model.backendsLoaded ? "No local model found" : "Looking for local models…")
                    .foregroundStyle(.secondary)
                    .font(.callout)
            } else {
                Picker("Model", selection: Binding(get: { model.backend ?? model.backends[0] }, set: { model.backend = $0 })) {
                    ForEach(model.backends, id: \.self) { Text($0.label).tag($0) }
                }
                .labelsHidden()
                .disabled(model.isAdvising)
            }
            Spacer()
            if model.isAdvising {
                ProgressView().controlSize(.small)
                Button("Stop") { model.stopAdvisor() }
            } else if !model.chat.isEmpty {
                Button("Start Over") { model.resetAdvisor() }
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 8)
    }

    private var intro: some View {
        VStack(alignment: .leading, spacing: 12) {
            Image(systemName: "lightbulb.max.fill")
                .font(.system(size: 30))
                .foregroundStyle(.yellow)
            Text("Storage advisor")
                .font(.title3.weight(.semibold))
            Text("Sends this scan and a summary of your volumes, network shares and startup disk to a model running on this Mac, and asks for ways to speed up I/O and make better use of the storage you already have.")
                .foregroundStyle(.secondary)
            Label("Suggestions only. Nothing is moved, changed or deleted.", systemImage: "hand.raised.fill")
                .font(.callout)
            Label("Ollama models run on this Mac. agenticode may use the provider it is configured with.", systemImage: "lock.fill")
                .font(.callout)
            Button {
                model.askAdvisor()
            } label: {
                Label(model.isScanning ? "Wait for the scan to finish" : "Ask for Suggestions", systemImage: "sparkles")
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            .disabled(model.isScanning || (model.backendsLoaded && model.backends.isEmpty))
            if model.backendsLoaded && model.backends.isEmpty {
                Text("Start Ollama with a chat model pulled (for example `ollama pull gemma3`), then reopen this tab.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer()
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var transcript: some View {
        ScrollViewReader { proxy in
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    ForEach(Array(replies.enumerated()), id: \.offset) { _, msg in
                        if msg.role == .user {
                            Text(msg.text)
                                .padding(8)
                                .background(RoundedRectangle(cornerRadius: 8).fill(Color.accentColor.opacity(0.15)))
                                .frame(maxWidth: .infinity, alignment: .trailing)
                        } else {
                            MarkdownText(text: msg.text)
                        }
                    }
                    if model.isAdvising && (replies.last?.text.isEmpty ?? true) {
                        Text("Collecting facts and waiting for the model…")
                            .foregroundStyle(.secondary)
                            .font(.callout)
                    }
                    if let err = model.advisorError {
                        Label(err, systemImage: "exclamationmark.triangle.fill")
                            .foregroundStyle(.orange)
                            .font(.callout)
                    }
                    let risky = model.isAdvising ? [] : replies.filter { $0.role == .assistant }.flatMap { Advisor.destructiveLines(in: $0.text) }
                    if !risky.isEmpty {
                        VStack(alignment: .leading, spacing: 4) {
                            Label("The model ignored the non-destructive rule in \(risky.count) line\(risky.count == 1 ? "" : "s"). Skip these:",
                                  systemImage: "exclamationmark.octagon.fill")
                                .fontWeight(.semibold)
                            ForEach(Array(risky.enumerated()), id: \.offset) { _, line in
                                Text(line).font(.system(size: 11.5, design: .monospaced)).lineLimit(2)
                            }
                        }
                        .font(.callout)
                        .foregroundStyle(.red)
                        .padding(8)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(RoundedRectangle(cornerRadius: 6).fill(Color.red.opacity(0.12)))
                    }
                    if !model.isAdvising && !replies.isEmpty {
                        Label("Written by a local model. FrisyDisk ran none of this. Read each command before you run it.",
                              systemImage: "exclamationmark.shield")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    Color.clear.frame(height: 1).id("end")
                }
                .padding(12)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .onChange(of: model.chat.last?.text.count ?? 0) { _, _ in
                if model.isAdvising { proxy.scrollTo("end", anchor: .bottom) }
            }
        }
    }

    private var followUp: some View {
        HStack(spacing: 6) {
            TextField("Ask a follow-up", text: $model.advisorQuestion)
                .textFieldStyle(.roundedBorder)
                .onSubmit(send)
            Button("Send", action: send)
                .disabled(model.isAdvising || model.advisorQuestion.trimmingCharacters(in: .whitespaces).isEmpty)
        }
        .padding(10)
    }

    private func send() {
        let q = model.advisorQuestion.trimmingCharacters(in: .whitespaces)
        guard !q.isEmpty, !model.isAdvising else { return }
        model.advisorQuestion = ""
        model.askAdvisor(followUp: q)
    }
}

/// Minimal Markdown renderer: headings, bullets, code fences and inline styles.
struct MarkdownText: View {
    let text: String

    private enum Block {
        case heading(Int, String)
        case bullet(String, String)
        case code(String)
        case paragraph(String)
        case rule
    }

    private var blocks: [Block] {
        var out: [Block] = []
        var code: [String]?
        for raw in text.components(separatedBy: "\n") {
            let line = raw.trimmingCharacters(in: .whitespaces)
            if line.hasPrefix("```") {
                if let c = code {
                    out.append(.code(c.joined(separator: "\n")))
                    code = nil
                } else {
                    code = []
                }
                continue
            }
            if code != nil {
                code?.append(raw)
                continue
            }
            if line.isEmpty { continue }
            if line == "---" || line == "***" {
                out.append(.rule)
            } else if line.hasPrefix("#") {
                let level = line.prefix { $0 == "#" }.count
                out.append(.heading(level, String(line.dropFirst(level)).trimmingCharacters(in: .whitespaces)))
            } else if line.hasPrefix("- ") || line.hasPrefix("* ") {
                out.append(.bullet("•", String(line.dropFirst(2))))
            } else if let dot = line.firstIndex(of: "."), line[..<dot].allSatisfy(\.isNumber), !line[..<dot].isEmpty,
                      line.index(after: dot) < line.endIndex, line[line.index(after: dot)] == " " {
                out.append(.bullet(String(line[...dot]), String(line[line.index(dot, offsetBy: 2)...])))
            } else {
                out.append(.paragraph(line))
            }
        }
        if let c = code { out.append(.code(c.joined(separator: "\n"))) }
        return out
    }

    private func inline(_ s: String) -> AttributedString {
        (try? AttributedString(markdown: s, options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace)))
            ?? AttributedString(s)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 7) {
            ForEach(Array(blocks.enumerated()), id: \.offset) { _, block in
                switch block {
                case .heading(let level, let s):
                    Text(inline(s))
                        .font(level <= 2 ? .title3.weight(.bold) : .headline)
                        .padding(.top, 6)
                case .bullet(let mark, let s):
                    HStack(alignment: .firstTextBaseline, spacing: 6) {
                        Text(mark).foregroundStyle(.secondary).monospacedDigit()
                        Text(inline(s))
                    }
                case .code(let s):
                    Text(s)
                        .font(.system(size: 11.5, design: .monospaced))
                        .padding(8)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(RoundedRectangle(cornerRadius: 6).fill(Color.primary.opacity(0.07)))
                case .paragraph(let s):
                    Text(inline(s))
                case .rule:
                    Divider()
                }
            }
        }
        .textSelection(.enabled)
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}
