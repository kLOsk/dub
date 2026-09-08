//
//  LibraryColumnCell.swift
//  Dub
//
//  One library table cell, rendered from a value snapshot.
//
//  Extracted from `LibraryView.columnCell(for:track:)`, which read the
//  app model, the library model and several pieces of view `@State`
//  directly. A cell that reads live objects cannot be *reused*, and
//  cell reuse is the whole point of the `NSTableView` the table is
//  moving to: a reused cell is handed a value when it is configured,
//  and must not consult anything else.
//
//  It also makes the cell testable. Nothing in `apple/DubTests` covered
//  the library table at all, so a port that promises "identical
//  pixels" had nothing to check itself against; `LibraryCellSnapshotTests`
//  is that baseline.
//
//  Everything here is a pure function of `LibraryCellState`. The five
//  live values the old code reached for — the two deck ids, library
//  open, volume reachability — belong to the *row*, not the cell, and
//  live in `LibraryRowState`.
//

import DubCore
import SwiftUI

/// Everything a cell draws, as values.
struct LibraryCellState: Equatable {
    var track: LibraryTrack
    /// M11d-history — Session History rows show the track this one was
    /// mixed in from. Only that source populates it.
    var sessionFromTitle: String?
    var keyNotationMode: KeyNotationMode = .camelot
    /// Registry id → index into `track.extras`, from what the FFI
    /// actually applied (see `applyExtraColumns`).
    var extraIndexById: [String: Int] = [:]
}

/// What a cell can fire. Separate so the state stays `Equatable`.
struct LibraryCellActions {
    /// `nil` clears the rating.
    var onRate: (_ rating: UInt8?) -> Void = { _ in }
    /// `nil` clears the colour label.
    var onColor: (_ token: String?) -> Void = { _ in }
}

struct LibraryColumnCell: View {
    let field: LibraryColumnField
    let state: LibraryCellState
    var actions = LibraryCellActions()

    private var track: LibraryTrack { state.track }

    var body: some View {
        switch field {
        case .crateOrder:
            Text(track.crateOrdinal.map { String($0 + 1) } ?? "—")
                .font(DubFont.body)
                .foregroundStyle(DubColor.textTertiary)
                .monospacedDigit()
                .frame(maxWidth: .infinity, alignment: .trailing)
        case .artist:
            plain(track.artist)
        case .title:
            HStack(spacing: DubSpacing.sm) {
                Text(LibraryCellFormat.displayTitle(track))
                    .font(DubFont.body)
                    .foregroundStyle(DubColor.textPrimary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                if let from = state.sessionFromTitle {
                    Text("← from \(from)")
                        .font(DubFont.micro)
                        .foregroundStyle(DubColor.textTertiary)
                        .lineLimit(1)
                        .truncationMode(.tail)
                        .layoutPriority(-1)
                }
            }
        case .duration:
            Text(LibraryCellFormat.duration(track.durationMs))
                .font(DubFont.body)
                .foregroundStyle(DubColor.textSecondary)
                .monospacedDigit()
        case .bpm:
            bpmCell
        case .album:
            plain(track.album)
        case .genre:
            plain(track.genre)
        case .year:
            Text(track.year.map { String($0) } ?? "—")
                .font(DubFont.body)
                .foregroundStyle(DubColor.textSecondary)
                .monospacedDigit()
        case .key:
            Text(state.keyNotationMode.render(track.key))
                .font(DubFont.body)
                .foregroundStyle(DubColor.textSecondary)
                .help(state.keyNotationMode.tooltip(track.key))
        case .comment:
            plain(track.comment)
                .help(track.comment ?? "")
        case .versionTokens:
            plain(track.versionTokens)
        case .source:
            plain(track.source)
        case .composer:
            plain(track.composer)
        case .trackNumber:
            Text(track.trackNumber.map { String($0) } ?? "—")
                .font(DubFont.body)
                .foregroundStyle(DubColor.textSecondary)
                .monospacedDigit()
        case .rating:
            ratingCell
        case .color:
            LibraryColorCell(color: track.color, onPick: actions.onColor)
        case let .extra(id):
            extraCell(id: id)
        }
    }

    /// The shared "text, or an em-dash" cell.
    private func plain(_ value: String?) -> some View {
        Text(value ?? "—")
            .font(DubFont.body)
            .foregroundStyle(DubColor.textSecondary)
            .lineLimit(1)
            .truncationMode(.middle)
    }

    private var bpmCell: some View {
        HStack(spacing: 4) {
            Text(LibraryCellFormat.bpm(track.bpm))
            // PRD §8.3 — two sources' grids disagree on the tempo or
            // the downbeat. Shown ahead of the drift warning: "Serato
            // says 92, the audio looks like 184" is a bigger problem
            // than a slow drift, and the per-source BPM columns are
            // where the DJ goes to resolve it.
            if track.bpmDisagreement {
                Image(systemName: "exclamationmark.triangle.fill")
                    .font(.system(size: 9))
                    .foregroundStyle(DubColor.stateTentative)
                    .help(
                        "Imported and analysed grids disagree · "
                            + "enable the per-source BPM columns to compare")
            }
            if track.gridLocked {
                Image(systemName: "lock.fill")
                    .font(.system(size: 9))
                    .foregroundStyle(DubColor.textSecondary)
            } else if let drift = track.gridDriftQuality, abs(drift) >= 3 {
                Image(systemName: "exclamationmark.triangle.fill")
                    .font(.system(size: 9))
                    .foregroundStyle(.orange)
                    .help("May drift over a long mix · right-click → Lock grid to accept")
            }
        }
        .font(DubFont.body)
        .foregroundStyle(DubColor.textSecondary)
        .monospacedDigit()
    }

    /// Click the Nth star to rate; click the current rating to clear it.
    /// Empty stars always draw, so the cell is a constant five glyphs wide.
    private var ratingCell: some View {
        let current = Int(track.rating ?? 0)
        return HStack(spacing: 1) {
            ForEach(1..<6, id: \.self) { star in
                Button {
                    actions.onRate(star == current ? nil : UInt8(star))
                } label: {
                    Image(systemName: star <= current ? "star.fill" : "star")
                        .font(.system(size: 9))
                        .foregroundStyle(
                            star <= current
                                ? DubColor.stateTentative : DubColor.textTertiary)
                }
                .buttonStyle(.plain)
            }
        }
    }

    /// One configurable column's cell (PRD §8.5.3.1). Formatting comes
    /// from the registry's kind rather than a per-column branch, which
    /// is what lets the deeper groups ship without a Swift case each.
    private func extraCell(id: String) -> some View {
        let info = LibraryColumnCatalog.shared.info(for: id)
        let value: LibraryColumnValue = {
            guard let index = state.extraIndexById[id],
                  track.extras.indices.contains(index)
            else { return .empty }
            return track.extras[index]
        }()
        let kind = info?.kind ?? .text
        let text = value.display(kind: kind)
        return Text(text ?? "—")
            .font(DubFont.body)
            .foregroundStyle(text == nil ? DubColor.textTertiary : DubColor.textSecondary)
            .lineLimit(1)
            .truncationMode(kind == .text ? .middle : .tail)
            .monospacedDigit()
            .frame(maxWidth: .infinity, alignment: kind.isNumeric ? .trailing : .leading)
            .help(text ?? "")
    }
}

/// Pure cell formatting. Free of the view so the migration's reused
/// cells and the tests share exactly one implementation.
enum LibraryCellFormat {
    static func displayTitle(_ track: LibraryTrack) -> String {
        if let title = track.title, !title.isEmpty { return title }
        return "Untitled"
    }

    static func bpm(_ bpm: Double?) -> String {
        guard let bpm, bpm > 0 else { return "—" }
        return String(format: "%.0f", bpm)
    }

    static func duration(_ ms: UInt32) -> String {
        guard ms > 0 else { return "—" }
        let totalSecs = Int(ms) / 1000
        return String(format: "%d:%02d", totalSecs / 60, totalSecs % 60)
    }
}

/// Render a Camelot key as either Camelot (default) or musical
/// notation (opt-in). Returns `"—"` for `nil` keys; the visual
/// "we have no key" cue is the em-dash, not an empty cell.
///
/// On the mode rather than on `LibraryView`, so a reused table cell
/// can render a key from its value snapshot alone.
extension KeyNotationMode {
    func render(_ camelot: String?) -> String {
        guard let camelot, !camelot.isEmpty else { return "—" }
        switch self {
        case .camelot: return camelot
        case .musical: return Self.musicalFromCamelot(camelot) ?? camelot
        }
    }

    /// Tooltip = the *other* notation, so the user always sees both at
    /// a glance without re-clicking. Empty for nil keys.
    func tooltip(_ camelot: String?) -> String {
        guard let camelot, !camelot.isEmpty else { return "" }
        switch self {
        case .camelot: return Self.musicalFromCamelot(camelot) ?? ""
        case .musical: return camelot
        }
    }

    /// Convert a canonical Camelot string (e.g. `"8B"`) to its musical
    /// equivalent (e.g. `"C major"`). Returns `nil` for malformed
    /// inputs; the renderer falls back to the raw Camelot string.
    ///
    /// Same wheel layout as `dub-spectral::key::CAMELOT_MAJOR` /
    /// `CAMELOT_MINOR`. A static lookup because pushing this across the
    /// FFI for every row render would be silly — the table is 24
    /// entries.
    static func musicalFromCamelot(_ camelot: String) -> String? {
        let table: [String: String] = [
            "8B": "C major", "3B": "C♯ major", "10B": "D major",
            "5B": "D♯ major", "12B": "E major", "7B": "F major",
            "2B": "F♯ major", "9B": "G major", "4B": "G♯ major",
            "11B": "A major", "6B": "A♯ major", "1B": "B major",
            "5A": "C minor", "12A": "C♯ minor", "7A": "D minor",
            "2A": "D♯ minor", "9A": "E minor", "4A": "F minor",
            "11A": "F♯ minor", "6A": "G minor", "1A": "G♯ minor",
            "8A": "A minor", "3A": "A♯ minor", "10A": "B minor",
        ]
        return table[camelot.uppercased()]
    }
}
