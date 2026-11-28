//
//  DubDataDir.swift
//  Dub
//
//  `DUB_DATA_DIR` moves everything Dub keeps on disk — the library,
//  its waveforms, rip sessions, the grid-calibration log — into one
//  folder, so a test run (the rig checklist's robot and loop items)
//  never writes into the DJ's own library. The Rust side reads the same
//  variable in `dub_library::paths`; this is the Swift half.
//
//      open --env DUB_DATA_DIR=/abs/path Dub.app
//

import Foundation

enum DubDataDir {
    /// The folder, when Dub was launched with one. A relative path is
    /// ignored here: the library refuses it on open and says so.
    static let current: URL? = folder(from: ProcessInfo.processInfo.environment)

    static func folder(from environment: [String: String]) -> URL? {
        guard let raw = environment["DUB_DATA_DIR"], raw.hasPrefix("/") else { return nil }
        return URL(fileURLWithPath: raw, isDirectory: true)
    }

    /// Put in front of the library's track count while a test folder is
    /// open, so an empty or small library on screen reads as what it
    /// is rather than as the DJ's library gone.
    static func statusPrefix(for folder: URL?) -> String {
        folder == nil ? "" : "SCRATCH LIBRARY · "
    }
}
