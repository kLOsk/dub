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
    /// The following grid is on, so a saved tempo map is what the deck
    /// plays and the BPM shows it (`LibraryAppModel.movingGridsShown`).
    var movingGridsShown: Bool = false
    /// Played this session (`LibraryAppModel.playedThisSession`): the
    /// row is drawn dimmed.
    var played: Bool = false
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
        case .bpm:
            bpmCell
        case .rating:
            ratingCell
        case .color:
            LibraryColorCell(color: track.color, onPick: actions.onColor)
        default:
            // A column that is a line of text is `LibraryCellText`,
            // drawn by `LibraryTextCellView` without a SwiftUI host.
            EmptyView()
        }
    }

    private var bpmCell: some View {
        let moving = state.movingGridsShown && track.tempoMapKind == "followed"
        let issues = LibraryCellFormat.bpmIssues(
            disagreement: track.bpmDisagreement, locked: track.gridLocked,
            drift: moving ? nil : track.gridDriftQuality)
        return HStack(spacing: 4) {
            // A moving grid's BPM in its colour; its padlock too, once
            // locked: moving, and accepted.
            Text(LibraryCellFormat.bpm(track.bpm))
                .foregroundStyle(moving ? DubColor.gridMoving : DubColor.textSecondary)
                .help(moving ? LibraryCellFormat.movingGridHelp(
                    min: track.tempoBpmMin, max: track.tempoBpmMax, locked: track.gridLocked) : "")
            // One mark, never two side by side (Daniel, 2026-11-16: the
            // double ⚠ "is bad"). It takes the colour of the worse
            // problem and its tooltip names every one.
            if let worst = issues.first {
                Image(systemName: "exclamationmark.triangle.fill")
                    .font(.system(size: 9))
                    .foregroundStyle(worst == .disagreement ? DubColor.stateTentative : .orange)
                    .help(issues.map(\.sentence).joined(separator: "\n"))
            }
            if track.gridLocked {
                Image(systemName: "lock.fill")
                    .font(.system(size: 9))
                    .foregroundStyle(moving ? DubColor.gridMoving : DubColor.textSecondary)
                    .help(moving
                        ? "Moving grid, accepted · it plays as saved · right-click → Unlock grid to change it"
                        : "Grid locked · right-click → Unlock grid to change it")
            }
        }
        .libraryRowText()
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
}

/// A column that is one line of text, as the values to draw: every
/// column but the three `LibraryColumnCell` above draws. Data rather than
/// a view so that `LibraryTextCellView` can draw it without a SwiftUI
/// host per cell (see there for what a host costs).
struct LibraryCellText: Equatable {
    enum Ink: Equatable {
        case primary, secondary, tertiary
        /// A Camelot key's colour on the wheel.
        case key(String)
    }

    var text: String
    var ink: Ink = .secondary
    /// A smaller, fainter remark after the text.
    var note: String?
    /// Digits of one width, so a column of numbers lines up.
    var digits = false
    var truncation: NSLineBreakMode = .byTruncatingTail
    /// Set against the column's right edge.
    var trailing = false
    var toolTip: String?

    static func isText(_ field: LibraryColumnField) -> Bool {
        switch field {
        case .bpm, .rating, .color: return false
        default: return true
        }
    }

    /// `nil` for a column that is more than text.
    static func make(_ field: LibraryColumnField, _ state: LibraryCellState) -> LibraryCellText? {
        let track = state.track
        func plain(_ value: String?) -> LibraryCellText {
            LibraryCellText(text: value ?? "—", truncation: .byTruncatingMiddle)
        }
        func number(_ value: String?) -> LibraryCellText {
            LibraryCellText(text: value ?? "—", digits: true)
        }
        switch field {
        case .bpm, .rating, .color:
            return nil
        case .crateOrder:
            return LibraryCellText(
                text: track.crateOrdinal.map { String($0 + 1) } ?? "—",
                ink: .tertiary, digits: true, trailing: true)
        case .artist: return plain(track.artist)
        case .album: return plain(track.album)
        case .genre: return plain(track.genre)
        case .versionTokens: return plain(track.versionTokens)
        case .source: return plain(track.source)
        case .composer: return plain(track.composer)
        case .comment:
            var cell = plain(track.comment)
            cell.toolTip = track.comment
            return cell
        case .title:
            var cell = LibraryCellText(
                text: LibraryCellFormat.displayTitle(track), ink: .primary,
                truncation: .byTruncatingMiddle)
            // M11d-history: the remark gives way first, as it did with a
            // lower layout priority, so the line is cut at its end.
            if let from = state.sessionFromTitle {
                cell.note = "← from \(from)"
                cell.truncation = .byTruncatingTail
            }
            return cell
        case .duration: return number(LibraryCellFormat.duration(track.durationMs))
        case .year: return number(track.year.map { String($0) })
        case .trackNumber: return number(track.trackNumber.map { String($0) })
        case .key:
            // Coloured by the Camelot wheel, so a compatible key can be
            // spotted down a list without reading it; plain for anything
            // that is not a Camelot string.
            var cell = LibraryCellText(text: state.keyNotationMode.render(track.key))
            if let key = track.key, DubColor.camelotKey(key) != nil { cell.ink = .key(key) }
            cell.toolTip = state.keyNotationMode.tooltip(track.key)
            return cell
        case let .extra(id):
            // One configurable column's cell (PRD §8.5.3.1). Formatting
            // comes from the registry's kind rather than a per-column
            // branch, which is what lets the deeper groups ship without
            // a Swift case each.
            let value: LibraryColumnValue = {
                guard let index = state.extraIndexById[id],
                      track.extras.indices.contains(index)
                else { return .empty }
                return track.extras[index]
            }()
            let kind = LibraryColumnCatalog.shared.info(for: id)?.kind ?? .text
            let text = value.display(kind: kind)
            return LibraryCellText(
                text: text ?? "—", ink: text == nil ? .tertiary : .secondary, digits: true,
                truncation: kind == .text ? .byTruncatingMiddle : .byTruncatingTail,
                trailing: kind.isNumeric, toolTip: text)
        }
    }
}

/// Pure cell formatting. Free of the view so the migration's reused
/// The DJ's word on one track's beat grid, as the library row's and the
/// deck BPM's right-click both offer it. One wording for both.
enum GridFollowChoice: CaseIterable {
    case automatic, follow, even

    init(word: Bool?) {
        switch word {
        case nil: self = .automatic
        case true?: self = .follow
        case false?: self = .even
        }
    }

    var word: Bool? {
        switch self {
        case .automatic: return nil
        case .follow: return true
        case .even: return false
        }
    }

    var title: String {
        switch self {
        case .automatic: return "Automatic"
        case .follow: return "Follow the band"
        case .even: return "Even grid"
        }
    }

    static let menuTitle = "Beat grid"
}

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

    /// The tooltip on a moving grid's BPM: the band's tempo range across
    /// the saved map, and whether it is accepted.
    static func movingGridHelp(min: Double?, max: Double?, locked: Bool) -> String {
        var text = "Follows the band"
        if let min, let max, min > 0, max > 0 {
            let lo = Int(min.rounded()), hi = Int(max.rounded())
            text += lo == hi ? " · \(lo) BPM" : " · \(lo)–\(hi) BPM"
        }
        return text + (locked ? " · locked: plays as saved" : " · lock to accept it")
    }

    /// What can be wrong with a track's grid, worst first.
    enum BpmIssue: Equatable {
        /// Two sources' grids disagree on the tempo or the downbeat (PRD
        /// §8.3) — "Serato says 92, the audio looks like 184".
        case disagreement
        /// The grid may wander late in a long track.
        case drift

        var sentence: String {
            switch self {
            case .disagreement:
                return "Imported and analysed grids disagree · "
                    + "enable the per-source BPM columns to compare"
            case .drift:
                return "The grid's tempo is slightly off, so its lines slide over a long mix · "
                    + "right-click → Re-analyze to correct it, or Lock grid to accept"
            }
        }
    }

    /// The drift, in ms per minute either way, from which a grid is
    /// marked as drifting. `GridQuality::DRIFT_WARNING_MS_PER_MIN` in
    /// `dub-bpm` is the same number and carries the measurement behind
    /// it; change the two together. It was 3 until 2026-11-19, which
    /// marked *Hey Ya* at 5.6 — a grid Daniel called perfect — and would
    /// have marked 28 % of the grids he locked in Traktor.
    static let driftWarningMsPerMin: Float = 8

    /// Whether a stored drift reading is worth the mark.
    static func drifts(_ drift: Float?) -> Bool {
        guard let drift else { return false }
        return abs(drift) >= driftWarningMsPerMin
    }

    /// The BPM cell's problems, worst first; the cell draws one mark for
    /// all of them. A locked grid has been accepted, so it never warns of
    /// drift — but a disagreement is about the sources, and still shows.
    static func bpmIssues(disagreement: Bool, locked: Bool, drift: Float?) -> [BpmIssue] {
        var out: [BpmIssue] = []
        if disagreement { out.append(.disagreement) }
        if !locked, drifts(drift) { out.append(.drift) }
        return out
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
