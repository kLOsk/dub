//
//  RipReviewPanel.swift
//  Dub
//
//  M26a — the rip review / encode surface that replaces the Prep pad
//  bar while a recorded side is being split, tagged, and imported.
//
//  Laid out as the back of a sleeve (rig, 2026-09-25): a toolbar that
//  says what the side is and what Import will do; a track map — the
//  side cut into its tracks, each as long as it is; and a tracklist
//  numbered the way a DJ reads a record, A1 A2 A3, one row per track,
//  filling the width. The cards it replaced sat in a corner of a panel
//  two thirds empty, and a nine-second lead-in got a card as big as a
//  seven-minute track.
//
//  Pure function of `RipReviewPanelState` (no FFI) so the snapshot
//  suite renders every mode.
//

import AppKit
import SwiftUI

/// One segment as the panel renders it. Mirrors the FFI `RipSegment`
/// as a plain value; metadata fields are empty strings when unset.
struct RipSegmentUi: Equatable, Identifiable {
    var id: UInt32 { index }
    let index: UInt32
    var startSecs: Double
    var endSecs: Double
    /// Left out of the commit — the card dims and the overview marker
    /// greys, but the audio stays in the side archive.
    var dropped: Bool = false
    var title: String = ""
    var artist: String = ""
    var album: String = ""
    var genre: String = ""
    var year: String = ""

    var durationSecs: Double { max(0, endSecs - startSecs) }

    var durationText: String { RipDuration.text(durationSecs) }
}

/// Editable metadata bundle a card reports after its 300 ms debounce.
struct RipSegmentMetadata: Equatable {
    var title: String
    var artist: String
    var album: String
    var genre: String
    var year: String
}

/// Per-segment commit dot.
enum RipJobDot: Equatable {
    case pending
    case running
    case done
    case failed

    var color: Color {
        switch self {
        case .pending: return DubColor.textPlaceholder
        case .running: return DubColor.stateTentative
        case .done:    return DubColor.stateLocked
        case .failed:  return DubColor.stateError
        }
    }
}

struct RipReviewPanelState: Equatable {
    enum Mode: Equatable {
        case review
        case encoding
        case done
        case failed
    }

    var mode: Mode
    /// Length of the side that will actually commit — the capture
    /// less whatever the trims discard.
    var sideDurationSecs: Double
    /// Seconds of lead-in + run-out being dropped, for the header
    /// note. `0` when nothing is trimmed, and the note disappears.
    var trimmedSecs: Double = 0
    var segments: [RipSegmentUi]
    /// One dot per segment once a commit has been requested.
    var jobDots: [RipJobDot] = []
    /// Footer status line (encode progress / failure summary).
    var overallStatus: String? = nil
    /// M26c recognition: `nil` until a pass has been asked for.
    var recognition: RipRecognitionUi? = nil
    /// The segment the side's playhead is inside, while it rolls.
    /// `nil` when paused or outside every segment. One card shows a
    /// pause glyph; the rest show play.
    ///
    /// An index rather than a playhead in seconds: the cards carry
    /// live `TextField`s, and republishing a moving playhead at the
    /// poll's 10 Hz would rebuild them all for a value none of them
    /// prints.
    var playingIndex: UInt32? = nil

    var hasFailedSegment: Bool { jobDots.contains(.failed) }

    /// The record's side, for the numbering: A1, A2… or B1, B2….
    var sideLetter: String = "A"
    /// The collection's genres, for the genre field's completion.
    var genres: [String] = []
    /// Changes when names land in the plan from outside the rows (Use
    /// all); the rows re-seed on it, and on nothing else but a re-cut.
    var metadataRevision: Int = 0

    /// A piece this short at either end of the side is the needle-drop
    /// or the run-out, not a track — the row suggests leaving it out.
    static let shortPieceSecs: Double = 20

    var keptCount: Int { segments.filter { !$0.dropped }.count }

    /// `A1`, `A2`…, counting only the tracks that will import — a left-
    /// out lead-in does not take A1 from the first real track. `nil`
    /// for a left-out segment.
    func position(of segment: RipSegmentUi) -> String? {
        guard !segment.dropped else { return nil }
        let n = segments.filter { !$0.dropped && $0.index <= segment.index }.count
        return "\(sideLetter)\(n)"
    }

    /// Why a piece looks like it is not a track, or `nil`.
    func shortPieceReason(_ segment: RipSegmentUi) -> String? {
        guard segment.durationSecs < Self.shortPieceSecs else { return nil }
        if segment.index == segments.first?.index { return "probably the lead-in" }
        if segment.index == segments.last?.index { return "probably the run-out" }
        return "very short for a track"
    }

    /// What Identify suggests for a segment, when it differs from what
    /// the row already says.
    func suggestion(for segment: RipSegmentUi) -> RipSegmentMetadata? {
        guard let s = recognition?.suggestions[segment.index] else { return nil }
        let current = RipSegmentMetadata(
            title: segment.title, artist: segment.artist, album: segment.album,
            genre: segment.genre, year: segment.year)
        return s == current ? nil : s
    }

    /// `22:50 · 3 of 4 import · 0:12 trimmed`.
    var summaryText: String {
        var text = RipDuration.text(sideDurationSecs)
        let n = segments.count
        text += keptCount == n
            ? " · \(n) track\(n == 1 ? "" : "s")"
            : " · \(keptCount) of \(n) import"
        if trimmedSecs >= 1 {
            text += " · \(RipDuration.text(trimmedSecs)) trimmed"
        }
        return text
    }

    /// The primary action says what it will do.
    var importTitle: String {
        "Import \(keptCount) track\(keptCount == 1 ? "" : "s")"
    }
}


/// What the review panel shows about a recognition pass (M26c): whether
/// it is running, how much it found, and — per track — the names it
/// suggests, which a row shows in amber until the DJ takes them. Nothing
/// is written into the plan until then: a wrong match committed into the
/// library is worse than no match.
struct RipRecognitionUi: Equatable {
    var running: Bool = false
    var finished: Bool = false
    var named: Int = 0
    var total: Int = 0
    var error: String? = nil
    /// Release line, when a pressing was identified.
    var release: String? = nil
    /// Per segment, what the pass named it — shown in the row until the
    /// DJ takes it with Use.
    var suggestions: [UInt32: RipSegmentMetadata] = [:]

    var summary: String {
        if running { return "Identifying…" }
        if let error { return error }
        guard finished else { return "" }
        if named == 0 { return "No match — not in the database" }
        var text = "Named \(named) of \(total)"
        if let release { text += " · \(release)" }
        return text
    }

    /// Only worth offering "Use all" when something was found.
    var canApply: Bool { finished && named > 0 && error == nil }
}

struct RipReviewPanelCallbacks {
    var addSplitAtPlayhead: () -> Void = {}
    /// Replace every marker with detected track gaps (M26b).
    var autoSplit: () -> Void = {}
    /// Audition from an absolute side position (seconds).
    var audition: (Double) -> Void = { _ in }
    /// Drop a segment from the commit, or put it back.
    var setDropped: (UInt32, Bool) -> Void = { _, _ in }
    /// Play a track, or pause it if it is the one running.
    var togglePlay: (UInt32) -> Void = { _ in }
    var setMetadata: (UInt32, RipSegmentMetadata) -> Void = { _, _ in }
    /// Which side of the record this is — the A / B of the numbering.
    var setSideLetter: (String) -> Void = { _ in }
    var cancel: () -> Void = {}
    /// Ask AcoustID what these tracks are (M26c).
    var identify: () -> Void = {}
    /// Write the recognised names into the metadata cards.
    var applyRecognition: () -> Void = {}
    var encode: () -> Void = {}
    var retry: () -> Void = {}
}

struct RipReviewPanel: View {

    let state: RipReviewPanelState
    var callbacks = RipReviewPanelCallbacks()

    /// Two-step destructive confirm for Discard — never a modal.
    @State private var discardArmed = false

    /// Rows are editable in review, and after a failure that never got
    /// as far as encoding; once a commit has run they report progress.
    private var showsProgress: Bool {
        switch state.mode {
        case .encoding, .done: return true
        case .failed: return !state.jobDots.isEmpty
        case .review: return false
        }
    }

    /// Height of one row plus the gap under it.
    static let rowPitch: CGFloat = 48
    /// Rows shown before the list scrolls. A side rarely holds more;
    /// past it the list scrolls rather than pushing the library away.
    static let visibleRows = 5
    static let toolbarHeight: CGFloat = 36
    static let mapHeight: CGFloat = 30
    static let columnHeaderHeight: CGFloat = 16

    /// What the panel asks of the Prep region for `rows` tracks.
    static func preferredHeight(rows: Int) -> CGFloat {
        let shown = CGFloat(min(max(rows, 1), visibleRows))
        return toolbarHeight + mapHeight + columnHeaderHeight + shown * rowPitch
            + 3 * DubSpacing.sm
    }

    var body: some View {
        VStack(alignment: .leading, spacing: DubSpacing.sm) {
            toolbar
            RipTrackMap(state: state, onSelect: callbacks.togglePlay)
                .frame(height: Self.mapHeight)
            VStack(alignment: .leading, spacing: 0) {
                columnHeader
                    .frame(height: Self.columnHeaderHeight)
                ScrollView(.vertical, showsIndicators: state.segments.count > Self.visibleRows) {
                    rows
                }
                .frame(height: CGFloat(min(max(state.segments.count, 1), Self.visibleRows))
                    * Self.rowPitch)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    // MARK: Toolbar

    private var toolbar: some View {
        HStack(spacing: DubSpacing.md) {
            HStack(spacing: DubSpacing.sm) {
                Text("SIDE")
                    .font(DubFont.caps)
                    .tracking(DubFont.capsTracking)
                    .foregroundStyle(DubColor.textSecondary)
                sideSwitch
                Text(state.summaryText)
                    .font(DubFont.numericInline)
                    .foregroundStyle(DubColor.textSecondary)
                    .lineLimit(1)
                    .fixedSize()
            }
            if state.mode == .review {
                toolGroup
            }
            if let summary = state.recognition?.summary, !summary.isEmpty,
               state.mode == .review
            {
                Text(summary)
                    .font(DubFont.micro)
                    .foregroundStyle(
                        state.recognition?.error == nil
                            ? DubColor.textSecondary : DubColor.stateError)
                    .lineLimit(1)
                    .truncationMode(.tail)
            }
            if let status = state.overallStatus {
                Text(status)
                    .font(DubFont.micro)
                    .foregroundStyle(
                        state.mode == .failed ? DubColor.stateError : DubColor.textSecondary)
                    .lineLimit(1)
                    .truncationMode(.tail)
            }
            Spacer(minLength: 0)
            if state.mode != .encoding, state.mode != .done {
                discardButton
            }
            trailingAction
        }
        .frame(height: Self.toolbarHeight)
    }

    /// A / B: which side of the record, so the tracks number A1… or B1….
    private var sideSwitch: some View {
        HStack(spacing: 0) {
            ForEach(["A", "B"], id: \.self) { letter in
                let on = state.sideLetter == letter
                Button { callbacks.setSideLetter(letter) } label: {
                    Text(letter)
                        .font(.system(size: 12, weight: .bold))
                        .foregroundStyle(on ? DubColor.surface0 : DubColor.textSecondary)
                        .frame(width: 24, height: 20)
                        .background(on ? DubColor.deckATint : Color.clear)
                        .clipShape(RoundedRectangle(cornerRadius: 4))
                }
                .buttonStyle(.plain)
                .disabled(showsProgress)
                .accessibilityLabel("Side \(letter)")
                .accessibilityAddTraits(on ? .isSelected : [])
            }
        }
        .padding(2)
        .background(DubColor.surface2)
        .clipShape(RoundedRectangle(cornerRadius: 6))
        .help("Which side of the record — the tracks number A1… or B1…")
    }

    /// Splitting and naming, as one group: they are one job.
    private var toolGroup: some View {
        HStack(spacing: 0) {
            tool("waveform.path", "Auto-split", help: "Replace the splits with detected track gaps",
                 action: callbacks.autoSplit)
            divider
            tool("scissors", "Split at playhead", help: "Add a split at deck A's current position",
                 action: callbacks.addSplitAtPlayhead)
            divider
            tool("magnifyingglass",
                 state.recognition?.running == true ? "Identifying…" : "Identify",
                 help: "Ask AcoustID for the artist and title of each track",
                 action: callbacks.identify)
                .disabled(state.recognition?.running == true)
            if state.recognition?.canApply == true {
                divider
                tool("checkmark", "Use all",
                     help: "Write every recognised name into the tracklist",
                     action: callbacks.applyRecognition)
            }
        }
        .background(DubColor.surface1)
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(DubColor.divider, lineWidth: 1))
        .clipShape(RoundedRectangle(cornerRadius: 8))
    }

    private var divider: some View {
        Rectangle().fill(DubColor.divider).frame(width: 1, height: 16)
    }

    private func tool(
        _ symbol: String, _ title: String, help: String, action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            HStack(spacing: 6) {
                Image(systemName: symbol)
                    .font(.system(size: 11, weight: .medium))
                    .foregroundStyle(DubColor.textSecondary)
                Text(title)
                    .font(DubFont.body)
                    .foregroundStyle(DubColor.textPrimary)
            }
            .padding(.horizontal, DubSpacing.md)
            .frame(height: 28)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help(help)
    }

    @ViewBuilder
    private var trailingAction: some View {
        switch state.mode {
        case .review:
            primaryButton(state.importTitle, action: callbacks.encode)
                .disabled(state.keptCount == 0)
                .help("Encode the tracks to FLAC and add them to the library")
        case .encoding:
            statusLabel("ENCODING…", color: DubColor.stateTentative, symbol: nil)
        case .done:
            statusLabel("IMPORTED", color: DubColor.stateLocked, symbol: "checkmark.circle.fill")
        case .failed:
            primaryButton(state.hasFailedSegment ? "Retry failed" : "Retry", action: callbacks.retry)
        }
    }

    private func primaryButton(_ title: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            HStack(spacing: 6) {
                Text(title)
                    .font(.system(size: 14, weight: .semibold))
                Text("▸").font(.system(size: 12))
            }
            .foregroundStyle(DubColor.surface0)
            .padding(.horizontal, DubSpacing.lg)
            .frame(height: 32)
            .background(DubColor.textPrimary)
            .clipShape(Capsule())
        }
        .buttonStyle(.plain)
    }

    private func statusLabel(_ text: String, color: Color, symbol: String?) -> some View {
        HStack(spacing: DubSpacing.xs) {
            if let symbol {
                Image(systemName: symbol).font(.system(size: 12)).foregroundStyle(color)
            } else {
                Circle().fill(color).frame(width: 7, height: 7)
            }
            Text(text)
                .font(DubFont.caps)
                .tracking(1.0)
                .foregroundStyle(color)
        }
    }

    private var discardButton: some View {
        Button {
            if discardArmed {
                discardArmed = false
                callbacks.cancel()
            } else {
                discardArmed = true
                Task { @MainActor in
                    try? await Task.sleep(nanoseconds: 3_000_000_000)
                    discardArmed = false
                }
            }
        } label: {
            Text(discardArmed ? "Really discard the side?" : "Discard side…")
                .font(DubFont.body)
                .foregroundStyle(discardArmed ? .white : DubColor.stateError)
                .padding(.horizontal, DubSpacing.md)
                .frame(height: 28)
                .background(discardArmed ? DubColor.stateError : Color.clear)
                .clipShape(Capsule())
        }
        .buttonStyle(.plain)
        .help("Throw the recording away — nothing is imported")
    }

    // MARK: Tracklist

    private var columnHeader: some View {
        HStack(spacing: RipTrackRow.spacing) {
            Color.clear.frame(width: RipTrackRow.playWidth)
            Color.clear.frame(width: RipTrackRow.positionWidth)
            ForEach(showsProgress ? ["TITLE", "PROGRESS"] : ["TITLE", "ARTIST", "ALBUM"], id: \.self) {
                columnLabel($0).frame(maxWidth: .infinity, alignment: .leading)
            }
            if !showsProgress {
                columnLabel("GENRE").frame(width: RipTrackRow.genreWidth, alignment: .leading)
            }
            columnLabel("LENGTH").frame(width: RipTrackRow.lengthWidth, alignment: .trailing)
            Color.clear.frame(width: RipTrackRow.endWidth)
        }
        .padding(.horizontal, RipTrackRow.inset)
    }

    private func columnLabel(_ text: String) -> some View {
        Text(text)
            .font(.system(size: 10, weight: .semibold))
            .tracking(DubFont.capsTracking)
            .foregroundStyle(DubColor.textTertiary)
    }

    private var rows: some View {
        VStack(spacing: Self.rowPitch - RipTrackRow.height) {
            ForEach(state.segments) { segment in
                if showsProgress {
                    RipProgressRow(
                        segment: segment,
                        position: state.position(of: segment),
                        dot: jobDot(for: segment))
                } else if segment.dropped {
                    RipDroppedRow(
                        segment: segment,
                        reason: state.shortPieceReason(segment),
                        onPutBack: { callbacks.setDropped(segment.index, false) })
                } else {
                    RipTrackRow(
                        segment: segment,
                        position: state.position(of: segment) ?? "",
                        isPlaying: state.playingIndex == segment.index,
                        shortReason: state.shortPieceReason(segment),
                        suggestion: state.suggestion(for: segment),
                        genres: state.genres,
                        revision: state.metadataRevision,
                        onMetadata: { callbacks.setMetadata(segment.index, $0) },
                        onTogglePlay: { callbacks.togglePlay(segment.index) },
                        onLeaveOut: { callbacks.setDropped(segment.index, true) })
                }
            }
        }
    }

    private func jobDot(for segment: RipSegmentUi) -> RipJobDot? {
        let idx = Int(segment.index)
        guard state.jobDots.indices.contains(idx) else { return nil }
        return state.jobDots[idx]
    }
}

// MARK: - Track map

/// The side cut into its tracks, each block as long as its track. The
/// overview above has the waveform and the split handles; this says
/// which stretch is A1 and which is A2 at a glance, and a click plays
/// that track.
struct RipTrackMap: View {
    let state: RipReviewPanelState
    var onSelect: (UInt32) -> Void = { _ in }

    var body: some View {
        GeometryReader { geo in
            let total = max(state.segments.map(\.endSecs).max() ?? 0, 1)
            let origin = state.segments.map(\.startSecs).min() ?? 0
            let span = max(total - origin, 1)
            ZStack(alignment: .topLeading) {
                ForEach(state.segments) { segment in
                    let x = (segment.startSecs - origin) / span * geo.size.width
                    let w = max(2, segment.durationSecs / span * geo.size.width - 2)
                    block(segment, width: w)
                        .frame(width: w, height: geo.size.height)
                        .offset(x: x)
                }
            }
        }
    }

    private func block(_ segment: RipSegmentUi, width: CGFloat) -> some View {
        let playing = state.playingIndex == segment.index
        let position = state.position(of: segment)
        return Button { onSelect(segment.index) } label: {
            ZStack(alignment: .leading) {
                RoundedRectangle(cornerRadius: 4)
                    .fill(segment.dropped
                          ? DubColor.surface1
                          : (playing ? DubColor.deckATint.opacity(0.28) : DubColor.surface2))
                if segment.dropped {
                    RipHatch().clipShape(RoundedRectangle(cornerRadius: 4))
                }
                if width > 56 {
                    HStack(spacing: 6) {
                        Text(position ?? "–")
                            .font(.system(size: 11, weight: .semibold, design: .monospaced))
                            .foregroundStyle(playing ? DubColor.deckATint : DubColor.textSecondary)
                        Text(segment.title.isEmpty ? "Untitled" : segment.title)
                            .font(DubFont.micro)
                            .foregroundStyle(playing ? DubColor.textPrimary : DubColor.textTertiary)
                            .lineLimit(1)
                    }
                    .padding(.horizontal, 8)
                }
                if let bar = progressColor(segment) {
                    VStack {
                        Spacer(minLength: 0)
                        Rectangle().fill(bar).frame(height: 3)
                    }
                    .clipShape(RoundedRectangle(cornerRadius: 4))
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help(segment.dropped ? "Left out" : "Play \(position ?? "")")
    }

    private func progressColor(_ segment: RipSegmentUi) -> Color? {
        let idx = Int(segment.index)
        guard state.jobDots.indices.contains(idx), !segment.dropped else { return nil }
        switch state.jobDots[idx] {
        case .pending: return nil
        case .running: return DubColor.stateTentative
        case .done: return DubColor.stateLocked
        case .failed: return DubColor.stateError
        }
    }
}

/// Diagonal stripes: what is left out of the side.
struct RipHatch: View {
    var body: some View {
        Canvas { ctx, size in
            var path = Path()
            var x: CGFloat = -size.height
            while x < size.width {
                path.move(to: CGPoint(x: x, y: size.height))
                path.addLine(to: CGPoint(x: x + size.height, y: 0))
                x += 8
            }
            ctx.stroke(path, with: .color(DubColor.surface3.opacity(0.7)), lineWidth: 2)
        }
    }
}

// MARK: - Rows

/// One track: play, its position on the side, what it is called, how
/// long it is, and leave-out. Local `@State` mirrors the fields so
/// typing never round-trips through the 10 Hz session poll; the row
/// re-seeds only when its segment boundaries change (a split edit
/// re-sliced the side). Edits are debounced 300 ms into `onMetadata`.
struct RipTrackRow: View {
    static let height: CGFloat = 44
    static let spacing: CGFloat = 8
    static let inset: CGFloat = 10
    static let playWidth: CGFloat = 32
    static let positionWidth: CGFloat = 32
    /// Wide enough for the long names a collection accumulates
    /// ("Reggae / Dancehall / Roots") — title, artist and album share
    /// what is left.
    static let genreWidth: CGFloat = 240
    static let lengthWidth: CGFloat = 52
    static let endWidth: CGFloat = 28

    let segment: RipSegmentUi
    let position: String
    var isPlaying: Bool = false
    /// Set when the piece is short enough to be a lead-in or run-out.
    var shortReason: String? = nil
    /// Identify's names for this track, until taken with Use.
    var suggestion: RipSegmentMetadata? = nil
    /// The collection's genres, offered as the genre is typed.
    var genres: [String] = []
    /// See `RipReviewPanelState.metadataRevision`.
    var revision: Int = 0
    var onMetadata: (RipSegmentMetadata) -> Void = { _ in }
    var onTogglePlay: () -> Void = {}
    var onLeaveOut: () -> Void = {}

    @State private var title: String
    @State private var artist: String
    @State private var album: String
    @State private var genre: String
    @State private var year: String
    @State private var debounce: Task<Void, Never>? = nil

    init(
        segment: RipSegmentUi,
        position: String,
        isPlaying: Bool = false,
        shortReason: String? = nil,
        suggestion: RipSegmentMetadata? = nil,
        genres: [String] = [],
        revision: Int = 0,
        onMetadata: @escaping (RipSegmentMetadata) -> Void = { _ in },
        onTogglePlay: @escaping () -> Void = {},
        onLeaveOut: @escaping () -> Void = {}
    ) {
        self.segment = segment
        self.position = position
        self.isPlaying = isPlaying
        self.shortReason = shortReason
        self.suggestion = suggestion
        self.genres = genres
        self.revision = revision
        self.onMetadata = onMetadata
        self.onTogglePlay = onTogglePlay
        self.onLeaveOut = onLeaveOut
        _title = State(initialValue: segment.title)
        _artist = State(initialValue: segment.artist)
        _album = State(initialValue: segment.album)
        _genre = State(initialValue: segment.genre)
        _year = State(initialValue: segment.year)
    }

    var body: some View {
        HStack(spacing: Self.spacing) {
            Button(action: onTogglePlay) {
                Image(systemName: isPlaying ? "pause.fill" : "play.fill")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(isPlaying ? DubColor.surface0 : DubColor.textPrimary)
                    .frame(width: Self.playWidth, height: Self.playWidth)
                    .background(isPlaying ? DubColor.deckATint : DubColor.surface3)
                    .clipShape(Circle())
            }
            .buttonStyle(.plain)
            .help(isPlaying ? "Pause" : "Play \(position)")
            .accessibilityLabel(isPlaying ? "Pause \(position)" : "Play \(position)")
            Text(position)
                .font(.system(size: 15, weight: .semibold, design: .monospaced))
                .foregroundStyle(isPlaying ? DubColor.deckATint : DubColor.textSecondary)
                .frame(width: Self.positionWidth, alignment: .leading)
            RipField(placeholder: "Title", text: $title, suggested: suggestion?.title, large: true)
                .overlay(alignment: .trailing) { titleAccessory }
                .frame(maxWidth: .infinity)
            RipField(placeholder: "Artist", text: $artist, suggested: suggestion?.artist)
                .frame(maxWidth: .infinity)
            RipField(placeholder: "Album", text: $album, suggested: suggestion?.album)
                .frame(maxWidth: .infinity)
            // No Year column: nobody types a pressing year while
            // splitting a side. Identify's year still reaches the tags
            // through Use (rig, 2026-09-25).
            RipGenreField(text: $genre, genres: genres, suggested: suggestion?.genre)
                .padding(.leading, DubSpacing.xs)
                .frame(width: Self.genreWidth, height: 30)
                .background(DubColor.surface0.opacity(0.55))
                .clipShape(RoundedRectangle(cornerRadius: 5))
            Text(segment.durationText)
                .font(DubFont.numericInline)
                .foregroundStyle(shortReason == nil ? DubColor.textSecondary : DubColor.stateTentative)
                .frame(width: Self.lengthWidth, alignment: .trailing)
            Button(action: onLeaveOut) {
                Image(systemName: "trash")
                    .font(.system(size: 12))
                    .foregroundStyle(DubColor.textTertiary)
                    .frame(width: Self.endWidth, height: Self.endWidth)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help("Leave \(position) out — the audio stays in the side archive")
            .accessibilityLabel("Leave \(position) out")
        }
        .padding(.horizontal, Self.inset)
        .frame(height: Self.height)
        .background(isPlaying ? DubColor.surface2 : DubColor.surface1)
        .clipShape(RoundedRectangle(cornerRadius: 8))
        .onChange(of: title) { _ in metadataEdited() }
        .onChange(of: artist) { _ in metadataEdited() }
        .onChange(of: album) { _ in metadataEdited() }
        .onChange(of: genre) { _ in metadataEdited() }
        .onChange(of: year) { _ in metadataEdited() }
        // Re-seed on a re-cut, or when names land from outside the row
        // (Use all) — never on the row's own edits coming back through
        // the 10 Hz poll. Comparing against the session did that, and a
        // letter typed between the send and the echo was reset: the
        // field blinked (rig, 2026-09-25).
        .onChange(of: seedKey) { _ in reseed() }
    }

    /// Inside the title field's trailing edge: Use for a suggestion, or
    /// the short-piece hint — either is about this row's name.
    @ViewBuilder
    private var titleAccessory: some View {
        if let suggestion {
            Button { take(suggestion) } label: {
                HStack(spacing: 4) {
                    Image(systemName: "checkmark").font(.system(size: 10, weight: .bold))
                    Text("Use").font(DubFont.micro)
                }
                .foregroundStyle(DubColor.stateTentative)
                .padding(.horizontal, 8)
                .frame(height: 22)
                .overlay(Capsule().stroke(DubColor.stateTentative.opacity(0.8), lineWidth: 1))
            }
            .buttonStyle(.plain)
            .padding(.trailing, 5)
            .help("Take the names Identify found for this track")
        } else if let shortReason {
            Button(action: onLeaveOut) {
                Text(Self.shortLabel(shortReason))
                    .font(DubFont.micro)
                    .foregroundStyle(DubColor.stateTentative)
                    .padding(.horizontal, 8)
                    .frame(height: 22)
                    .overlay(Capsule().stroke(DubColor.stateTentative.opacity(0.6), lineWidth: 1))
            }
            .buttonStyle(.plain)
            .padding(.trailing, 5)
            .help("\(shortReason.prefix(1).uppercased() + shortReason.dropFirst()) — a piece this short is usually not a track")
        }
    }

    /// Short enough to sit in the title field without covering it.
    static func shortLabel(_ reason: String) -> String {
        if reason.contains("lead-in") { return "Lead-in? Leave out" }
        if reason.contains("run-out") { return "Run-out? Leave out" }
        return "Short — leave out?"
    }

    private var seedKey: String {
        "\(segment.startSecs)-\(segment.endSecs)|\(revision)"
    }

    private func take(_ s: RipSegmentMetadata) {
        if !s.title.isEmpty { title = s.title }
        if !s.artist.isEmpty { artist = s.artist }
        if !s.album.isEmpty { album = s.album }
        if !s.genre.isEmpty { genre = s.genre }
        if !s.year.isEmpty { year = s.year }
    }

    private func reseed() {
        debounce?.cancel()
        title = segment.title
        artist = segment.artist
        album = segment.album
        genre = segment.genre
        year = segment.year
    }

    private func metadataEdited() {
        debounce?.cancel()
        let meta = RipSegmentMetadata(
            title: title, artist: artist, album: album, genre: genre, year: year)
        debounce = Task { @MainActor in
            try? await Task.sleep(nanoseconds: 300_000_000)
            guard !Task.isCancelled else { return }
            onMetadata(meta)
        }
    }
}

/// A text field whose empty state can show Identify's suggestion, in
/// amber, where the placeholder would be — a suggestion reads as one,
/// never as what the track is called, until it is taken.
struct RipField: View {
    let placeholder: String
    @Binding var text: String
    var suggested: String? = nil
    var large: Bool = false

    var body: some View {
        ZStack(alignment: .leading) {
            if text.isEmpty {
                let s = suggested ?? ""
                Text(s.isEmpty ? placeholder : s)
                    .font(large ? .system(size: 14) : DubFont.body)
                    .foregroundStyle(s.isEmpty ? DubColor.textPlaceholder : DubColor.stateTentative)
                    .lineLimit(1)
                    .allowsHitTesting(false)
            }
            TextField("", text: $text)
                .textFieldStyle(.plain)
                .font(large ? .system(size: 14) : DubFont.body)
                .foregroundStyle(DubColor.textPrimary)
                .accessibilityLabel(placeholder)
        }
        .padding(.horizontal, DubSpacing.sm)
        .frame(height: 30)
        .background(DubColor.surface0.opacity(0.55))
        .clipShape(RoundedRectangle(cornerRadius: 5))
    }
}

/// A left-out piece: a thin hatched line, not a full row — it is not
/// going anywhere, and a nine-second lead-in should not look like a
/// track.
struct RipDroppedRow: View {
    let segment: RipSegmentUi
    var reason: String? = nil
    var onPutBack: () -> Void = {}

    var body: some View {
        HStack(spacing: RipTrackRow.spacing) {
            Color.clear.frame(width: RipTrackRow.playWidth)
            Text("–")
                .font(.system(size: 15, weight: .semibold, design: .monospaced))
                .foregroundStyle(DubColor.textPlaceholder)
                .frame(width: RipTrackRow.positionWidth, alignment: .leading)
            Text("Left out · \(segment.durationText)\(reason.map { " · \($0)" } ?? "")")
                .font(DubFont.body)
                .foregroundStyle(DubColor.textTertiary)
                .lineLimit(1)
            Spacer(minLength: 0)
            Button(action: onPutBack) {
                HStack(spacing: 5) {
                    Image(systemName: "arrow.uturn.backward").font(.system(size: 11))
                    Text("Put back").font(DubFont.body)
                }
                .foregroundStyle(DubColor.textSecondary)
                .padding(.horizontal, DubSpacing.sm)
                .frame(height: 26)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help("Import this one after all")
        }
        .padding(.horizontal, RipTrackRow.inset)
        .frame(height: RipTrackRow.height)
        .background(RipHatch().opacity(0.6))
        .overlay(
            RoundedRectangle(cornerRadius: 8)
                .strokeBorder(DubColor.divider, style: StrokeStyle(lineWidth: 1, dash: [4, 3])))
        .clipShape(RoundedRectangle(cornerRadius: 8))
    }
}

/// A track during and after the commit: where it is, what it is
/// called, and how far it got.
struct RipProgressRow: View {
    let segment: RipSegmentUi
    let position: String?
    let dot: RipJobDot?

    var body: some View {
        HStack(spacing: RipTrackRow.spacing) {
            statusGlyph.frame(width: RipTrackRow.playWidth)
            Text(position ?? "–")
                .font(.system(size: 15, weight: .semibold, design: .monospaced))
                .foregroundStyle(DubColor.textSecondary)
                .frame(width: RipTrackRow.positionWidth, alignment: .leading)
            Text(segment.dropped ? "Left out" : (segment.title.isEmpty ? "Untitled" : segment.title))
                .font(.system(size: 14))
                .foregroundStyle(segment.dropped ? DubColor.textTertiary : DubColor.textPrimary)
                .lineLimit(1)
                .frame(maxWidth: .infinity, alignment: .leading)
            HStack(spacing: DubSpacing.md) {
                ZStack(alignment: .leading) {
                    Capsule().fill(DubColor.surface3)
                    Capsule().fill(color).frame(width: nil).opacity(fill > 0 ? 1 : 0)
                        .scaleEffect(x: fill, anchor: .leading)
                }
                .frame(height: 4)
                Text(statusText)
                    .font(DubFont.micro)
                    .foregroundStyle(color)
                    .frame(width: 110, alignment: .leading)
            }
            .frame(maxWidth: .infinity)
            Text(segment.durationText)
                .font(DubFont.numericInline)
                .foregroundStyle(DubColor.textSecondary)
                .frame(width: RipTrackRow.lengthWidth, alignment: .trailing)
            Color.clear.frame(width: RipTrackRow.endWidth)
        }
        .padding(.horizontal, RipTrackRow.inset)
        .frame(height: RipTrackRow.height)
        .background(DubColor.surface1)
        .clipShape(RoundedRectangle(cornerRadius: 8))
        .opacity(segment.dropped ? 0.5 : 1)
    }

    /// No percentage: the worker reports a track as waiting, running or
    /// finished, and a bar that pretended to know more would lie.
    private var fill: CGFloat {
        switch dot {
        case .done, .failed: return 1
        case .running: return 0.5
        default: return 0
        }
    }

    private var color: Color {
        guard !segment.dropped else { return DubColor.textTertiary }
        switch dot {
        case .done: return DubColor.stateLocked
        case .failed: return DubColor.stateError
        case .running: return DubColor.stateTentative
        default: return DubColor.textTertiary
        }
    }

    private var statusText: String {
        guard !segment.dropped else { return "Not imported" }
        switch dot {
        case .done: return "In library"
        case .failed: return "Failed"
        case .running: return "Encoding…"
        default: return "Waiting"
        }
    }

    @ViewBuilder
    private var statusGlyph: some View {
        switch dot {
        case .done where !segment.dropped:
            Image(systemName: "checkmark.circle.fill")
                .font(.system(size: 14)).foregroundStyle(DubColor.stateLocked)
        case .failed where !segment.dropped:
            Image(systemName: "exclamationmark.circle.fill")
                .font(.system(size: 14)).foregroundStyle(DubColor.stateError)
        default:
            Circle().stroke(color, lineWidth: 2).frame(width: 12, height: 12)
        }
    }
}

#Preview("review — 4 segments") {
    RipReviewPanel(state: RipReviewPanelState(
        mode: .review,
        sideDurationSecs: 1370,
        segments: [
            RipSegmentUi(index: 0, startSecs: 0, endSecs: 9, dropped: true),
            RipSegmentUi(index: 1, startSecs: 9, endSecs: 461,
                         title: "King Tubby Meets Rockers Uptown",
                         artist: "Augustus Pablo", album: "", genre: "Dub", year: "1976"),
            RipSegmentUi(index: 2, startSecs: 461, endSecs: 887),
            RipSegmentUi(index: 3, startSecs: 887, endSecs: 1370),
        ],
        playingIndex: 2))
        .padding()
        .background(DubColor.surface0)
        .frame(width: 1300)
}

// MARK: - Genre

/// The genre, completed from the collection's own genres: typing "hi"
/// fills in "Hip-Hop" as the library spells it, the arrow lists them
/// all, and anything else typed is a new genre (rig, 2026-09-25). A
/// fourth spelling of a genre the library already has is how a crate
/// filter stops finding records.
///
/// `NSComboBox`, not a hand-built SwiftUI list: inline completion, the
/// drop-down and its keyboard handling are AppKit's, and on macOS 13
/// SwiftUI has no key handling to build them with.
struct RipGenreField: NSViewRepresentable {
    @Binding var text: String
    var genres: [String]
    /// Identify's genre, shown where the placeholder would be.
    var suggested: String? = nil

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    func makeNSView(context: Context) -> NSComboBox {
        let box = NSComboBox()
        box.completes = true
        box.usesDataSource = false
        box.isBordered = false
        box.isButtonBordered = false
        box.drawsBackground = false
        box.focusRingType = .none
        box.font = NSFont.systemFont(ofSize: 13)
        box.textColor = NSColor(DubColor.textPrimary)
        box.numberOfVisibleItems = 10
        box.delegate = context.coordinator
        box.setAccessibilityLabel("Genre")
        configure(box)
        return box
    }

    func updateNSView(_ box: NSComboBox, context: Context) {
        context.coordinator.parent = self
        configure(box)
    }

    private func configure(_ box: NSComboBox) {
        if box.objectValues as? [String] != genres {
            box.removeAllItems()
            box.addItems(withObjectValues: genres)
        }
        // Never overwrite what is being typed: the field editor owns
        // the text while the box has focus.
        if box.currentEditor() == nil, box.stringValue != text {
            box.stringValue = text
        }
        let hint = suggested ?? ""
        box.placeholderAttributedString = NSAttributedString(
            string: hint.isEmpty ? "Genre" : hint,
            attributes: [
                .foregroundColor: NSColor(hint.isEmpty ? DubColor.textPlaceholder : DubColor.stateTentative),
                .font: NSFont.systemFont(ofSize: 13),
            ])
    }

    final class Coordinator: NSObject, NSComboBoxDelegate {
        var parent: RipGenreField

        init(_ parent: RipGenreField) {
            self.parent = parent
        }

        func controlTextDidChange(_ note: Notification) {
            guard let box = note.object as? NSComboBox else { return }
            parent.text = box.stringValue
        }

        func comboBoxSelectionDidChange(_ note: Notification) {
            guard let box = note.object as? NSComboBox,
                  box.indexOfSelectedItem >= 0,
                  let value = box.objectValueOfSelectedItem as? String
            else { return }
            parent.text = value
        }

        func controlTextDidEndEditing(_ note: Notification) {
            guard let box = note.object as? NSComboBox else { return }
            parent.text = box.stringValue
        }
    }
}
