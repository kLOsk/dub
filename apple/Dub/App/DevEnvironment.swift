//
//  DevEnvironment.swift
//  Dub
//
//  Keys a developer keeps for test runs, out of Preferences and out of
//  git: the repo's `.env` (gitignored; `.env.example` is the tracked
//  template). A Debug build reads it from the checkout it was built
//  from; a Release build never looks for the file. `dub recognize` reads
//  the same file with the same rules (`recognize.rs`).
//

import Foundation

enum DevEnvironment {
    /// `name` from the process environment, else from the repo's `.env`
    /// in a Debug build. Empty counts as unset.
    static func value(_ name: String) -> String? {
        let launched = ProcessInfo.processInfo.environment[name]?
            .trimmingCharacters(in: .whitespaces)
        if let launched, !launched.isEmpty { return launched }
        return dotEnv[name]
    }

    #if DEBUG
    /// The repo root is four levels above this source file
    /// (`apple/Dub/App/DevEnvironment.swift`), wherever it was checked out.
    private static let dotEnv: [String: String] = {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<4 { root.deleteLastPathComponent() }
        let file = root.appendingPathComponent(".env")
        guard let text = try? String(contentsOf: file, encoding: .utf8) else { return [:] }
        return parse(text)
    }()
    #else
    private static let dotEnv: [String: String] = [:]
    #endif

    /// `NAME=value` lines, `export` allowed, `#` comments, quotes around
    /// the value dropped, the last line winning. An empty value is left
    /// out, so `.env.example` copied and not yet filled in reads as no key.
    static func parse(_ text: String) -> [String: String] {
        var values: [String: String] = [:]
        for raw in text.components(separatedBy: .newlines) {
            var line = raw.trimmingCharacters(in: .whitespaces)
            if line.hasPrefix("#") { continue }
            if line.hasPrefix("export ") {
                line = String(line.dropFirst("export ".count)).trimmingCharacters(in: .whitespaces)
            }
            guard let equals = line.firstIndex(of: "=") else { continue }
            let name = line[..<equals].trimmingCharacters(in: .whitespaces)
            var value = line[line.index(after: equals)...].trimmingCharacters(in: .whitespaces)
            if let quote = value.first, quote == "\"" || quote == "'" {
                value = String(value.dropFirst().prefix { $0 != quote })
            } else if let comment = value.range(of: " #") {
                value = value[..<comment.lowerBound].trimmingCharacters(in: .whitespaces)
            }
            values[name] = value.isEmpty ? nil : value
        }
        return values
    }
}

#if DEBUG
/// `DUB_PRINT_CHANGES=1` in a Debug build: the app's top-level views print
/// what made SwiftUI run their body again (`_printChanges`). For finding
/// what invalidates the whole window — a click into a rip review field
/// stalled it ~150 ms per click (rig, 2026-11-27).
enum DebugTrace {
    static let printChanges = ProcessInfo.processInfo.environment["DUB_PRINT_CHANGES"] != nil
}
#endif
