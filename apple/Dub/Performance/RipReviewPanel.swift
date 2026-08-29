//
//  RipReviewPanel.swift
//  Dub
//
//  M26a — the rip review / encode surface that replaces the Prep pad
//  bar while a recorded side is being split, tagged, and imported.
//  Header: side summary + "+ Split at playhead". Body: horizontally
//  scrolling per-segment cards (duration chip, editable metadata
//  debounced into the session, audition buttons). Footer: destructive
//  two-step Discard + the filled "Encode & Import ▸" action, which
//  becomes per-segment progress dots + status (+ "Retry failed")
//  once the commit worker runs.
//
//  Pure function of `RipReviewPanelState` (no FFI) so the snapshot
//  suite renders every mode.
//

import SwiftUI

/// One segment as the panel renders it. Mirrors the FFI `RipSegment`
/// as a plain value; metadata fields are empty strings when unset.
struct RipSegmentUi: Equatable, Identifiable {
    var id: UInt32 { index }
    let index: UInt32
    var startSecs: Double
    var endSecs: Double
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

    var hasFailedSegment: Bool { jobDots.contains(.failed) }

    var headerText: String {
        let n = segments.count
        let clock = RipDuration.text(sideDurationSecs)
        var text = "SIDE — \(clock) · \(n) TRACK\(n == 1 ? "" : "S")"
        if trimmedSecs >= 1 {
            text += " · \(RipDuration.text(trimmedSecs)) TRIMMED"
        }
        return text
    }
}

struct RipReviewPanelCallbacks {
    var addSplitAtPlayhead: () -> Void = {}
    /// Replace every marker with detected track gaps (M26b).
    var autoSplit: () -> Void = {}
    /// Audition from an absolute side position (seconds).
    var audition: (Double) -> Void = { _ in }
    var setMetadata: (UInt32, RipSegmentMetadata) -> Void = { _, _ in }
    var cancel: () -> Void = {}
    var encode: () -> Void = {}
    var retry: () -> Void = {}
}

struct RipReviewPanel: View {

    let state: RipReviewPanelState
    var callbacks = RipReviewPanelCallbacks()

    /// Two-step destructive confirm for Discard — never a modal.
    @State private var discardArmed = false

    private var isEditable: Bool { state.mode == .review || state.mode == .failed }

    var body: some View {
        VStack(alignment: .leading, spacing: DubSpacing.sm) {
            header
            segmentRow
            footer
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    // MARK: Header

    private var header: some View {
        HStack(spacing: DubSpacing.md) {
            Text(state.headerText)
                .font(DubFont.caps)
                .tracking(1.2)
                .foregroundStyle(DubColor.textSecondary)
            Spacer(minLength: 0)
            if state.mode == .review {
                Button(action: callbacks.autoSplit) {
                    pillLabel("Auto-split")
                }
                .buttonStyle(.plain)
                .help("Replace the markers with detected track gaps")
                Button(action: callbacks.addSplitAtPlayhead) {
                    pillLabel("+ Split at playhead")
                }
                .buttonStyle(.plain)
                .help("Add a split marker at deck A's current position")
            }
        }
    }

    private func pillLabel(_ title: String) -> some View {
        Text(title)
            .font(DubFont.body)
            .foregroundStyle(DubColor.textPrimary)
            .padding(.horizontal, DubSpacing.md)
            .padding(.vertical, 2)
            .background(DubColor.surface2)
            .clipShape(Capsule())
    }

    // MARK: Segments

    private var segmentRow: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(alignment: .top, spacing: DubSpacing.md) {
                ForEach(state.segments) { segment in
                    RipSegmentCard(
                        segment: segment,
                        editable: isEditable,
                        jobDot: jobDot(for: segment),
                        onMetadata: { meta in
                            callbacks.setMetadata(segment.index, meta)
                        },
                        onAuditionInto: {
                            callbacks.audition(segment.startSecs)
                        },
                        onAuditionOutOf: {
                            callbacks.audition(max(segment.startSecs, segment.endSecs - 6))
                        })
                }
            }
            .padding(.vertical, 1)
        }
    }

    private func jobDot(for segment: RipSegmentUi) -> RipJobDot? {
        let idx = Int(segment.index)
        guard state.jobDots.indices.contains(idx) else { return nil }
        return state.jobDots[idx]
    }

    // MARK: Footer

    private var footer: some View {
        HStack(spacing: DubSpacing.md) {
            discardButton
            Spacer(minLength: 0)
            if !state.jobDots.isEmpty {
                dotsRow
            }
            if let status = state.overallStatus {
                Text(status)
                    .font(DubFont.micro)
                    .foregroundStyle(
                        state.mode == .failed
                            ? DubColor.stateError
                            : DubColor.textSecondary)
                    .lineLimit(1)
                    .truncationMode(.tail)
            }
            trailingAction
        }
    }

    private var dotsRow: some View {
        HStack(spacing: DubSpacing.xs) {
            ForEach(Array(state.jobDots.enumerated()), id: \.offset) { _, dot in
                Circle()
                    .fill(dot.color)
                    .frame(width: 8, height: 8)
            }
        }
    }

    @ViewBuilder
    private var trailingAction: some View {
        switch state.mode {
        case .review:
            Button(action: callbacks.encode) {
                Text("Encode & Import ▸")
                    .font(DubFont.body)
                    .foregroundStyle(DubColor.surface0)
                    .padding(.horizontal, DubSpacing.lg)
                    .padding(.vertical, DubSpacing.xs)
                    .background(DubColor.textPrimary)
                    .clipShape(Capsule())
            }
            .buttonStyle(.plain)
            .disabled(state.segments.isEmpty)
        case .encoding:
            HStack(spacing: DubSpacing.xs) {
                Circle()
                    .fill(DubColor.stateTentative)
                    .frame(width: 6, height: 6)
                Text("ENCODING…")
                    .font(DubFont.caps)
                    .tracking(1.0)
                    .foregroundStyle(DubColor.stateTentative)
            }
        case .done:
            HStack(spacing: DubSpacing.xs) {
                Image(systemName: "checkmark.circle.fill")
                    .font(.system(size: 11))
                    .foregroundStyle(DubColor.stateLocked)
                Text("IMPORTED")
                    .font(DubFont.caps)
                    .tracking(1.0)
                    .foregroundStyle(DubColor.stateLocked)
            }
        case .failed:
            if state.hasFailedSegment {
                Button(action: callbacks.retry) {
                    Text("Retry failed")
                        .font(DubFont.body)
                        .foregroundStyle(DubColor.textPrimary)
                        .padding(.horizontal, DubSpacing.md)
                        .padding(.vertical, DubSpacing.xs)
                        .background(DubColor.stateError.opacity(0.35))
                        .clipShape(Capsule())
                }
                .buttonStyle(.plain)
            } else {
                Button(action: callbacks.retry) {
                    Text("Retry")
                        .font(DubFont.body)
                        .foregroundStyle(DubColor.textPrimary)
                        .padding(.horizontal, DubSpacing.md)
                        .padding(.vertical, DubSpacing.xs)
                        .background(DubColor.stateError.opacity(0.35))
                        .clipShape(Capsule())
                }
                .buttonStyle(.plain)
            }
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
            Text(discardArmed ? "Really discard?" : "Cancel / Discard")
                .font(DubFont.body)
                .foregroundStyle(discardArmed ? .white : DubColor.stateError)
                .padding(.horizontal, DubSpacing.md)
                .padding(.vertical, 2)
                .background(
                    discardArmed
                        ? DubColor.stateError
                        : DubColor.stateError.opacity(0.12))
                .clipShape(Capsule())
        }
        .buttonStyle(.plain)
        .disabled(state.mode == .encoding)
        .help("Throw the recording away — nothing is imported")
    }
}

// MARK: - Segment card

/// One track-to-be: index + duration chip, editable metadata
/// (debounced 300 ms into `onMetadata`), audition into / out-of
/// buttons. Local `@State` mirrors the fields so typing never
/// round-trips through the 10 Hz session poll; the card re-seeds
/// only when its segment boundaries change (a split edit re-sliced
/// the side).
struct RipSegmentCard: View {

    let segment: RipSegmentUi
    var editable: Bool = true
    var jobDot: RipJobDot? = nil
    var onMetadata: (RipSegmentMetadata) -> Void = { _ in }
    var onAuditionInto: () -> Void = {}
    var onAuditionOutOf: () -> Void = {}

    @State private var title: String
    @State private var artist: String
    @State private var album: String
    @State private var genre: String
    @State private var year: String
    @State private var debounce: Task<Void, Never>? = nil

    init(
        segment: RipSegmentUi,
        editable: Bool = true,
        jobDot: RipJobDot? = nil,
        onMetadata: @escaping (RipSegmentMetadata) -> Void = { _ in },
        onAuditionInto: @escaping () -> Void = {},
        onAuditionOutOf: @escaping () -> Void = {}
    ) {
        self.segment = segment
        self.editable = editable
        self.jobDot = jobDot
        self.onMetadata = onMetadata
        self.onAuditionInto = onAuditionInto
        self.onAuditionOutOf = onAuditionOutOf
        _title = State(initialValue: segment.title)
        _artist = State(initialValue: segment.artist)
        _album = State(initialValue: segment.album)
        _genre = State(initialValue: segment.genre)
        _year = State(initialValue: segment.year)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: DubSpacing.xs) {
            HStack(spacing: DubSpacing.sm) {
                if let jobDot {
                    Circle()
                        .fill(jobDot.color)
                        .frame(width: 8, height: 8)
                }
                Text("TRACK \(segment.index + 1)")
                    .font(DubFont.caps)
                    .tracking(1.0)
                    .foregroundStyle(DubColor.textSecondary)
                Spacer(minLength: 0)
                Text(segment.durationText)
                    .font(DubFont.numericInline)
                    .foregroundStyle(DubColor.textSecondary)
                    .padding(.horizontal, DubSpacing.sm)
                    .padding(.vertical, 1)
                    .background(DubColor.surface3)
                    .clipShape(Capsule())
            }
            field("Title", text: $title)
            field("Artist", text: $artist)
            field("Album", text: $album)
            HStack(spacing: DubSpacing.xs) {
                field("Genre", text: $genre)
                field("Year", text: $year)
                    .frame(width: 56)
            }
            HStack(spacing: DubSpacing.xs) {
                auditionButton("▶ IN", help: "Audition into this track",
                               action: onAuditionInto)
                auditionButton("▶ OUT", help: "Audition out of this track",
                               action: onAuditionOutOf)
                Spacer(minLength: 0)
            }
        }
        .padding(DubSpacing.sm)
        .frame(width: 230)
        .background(DubColor.surface1)
        .clipShape(RoundedRectangle(cornerRadius: DubRadius.panel))
        .onChange(of: title) { _ in metadataEdited() }
        .onChange(of: artist) { _ in metadataEdited() }
        .onChange(of: album) { _ in metadataEdited() }
        .onChange(of: genre) { _ in metadataEdited() }
        .onChange(of: year) { _ in metadataEdited() }
        // Re-seed only on a boundary change — a split edit re-sliced
        // the side and the FFI's re-derived metadata is authoritative.
        .onChange(of: boundaryKey) { _ in reseed() }
    }

    private var boundaryKey: String {
        "\(segment.startSecs)-\(segment.endSecs)"
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
        guard editable else { return }
        debounce?.cancel()
        let meta = RipSegmentMetadata(
            title: title, artist: artist, album: album,
            genre: genre, year: year)
        debounce = Task { @MainActor in
            try? await Task.sleep(nanoseconds: 300_000_000)
            guard !Task.isCancelled else { return }
            onMetadata(meta)
        }
    }

    private func field(_ placeholder: String, text: Binding<String>) -> some View {
        TextField(placeholder, text: text)
            .textFieldStyle(.plain)
            .font(DubFont.body)
            .foregroundStyle(DubColor.textPrimary)
            .padding(.horizontal, DubSpacing.sm)
            .padding(.vertical, 3)
            .background(DubColor.surface2)
            .clipShape(RoundedRectangle(cornerRadius: 4))
            .disabled(!editable)
            .opacity(editable ? 1.0 : 0.6)
    }

    private func auditionButton(
        _ label: String, help: String, action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            Text(label)
                .font(DubFont.micro)
                .foregroundStyle(DubColor.textSecondary)
                .padding(.horizontal, DubSpacing.sm)
                .padding(.vertical, 2)
                .background(DubColor.surface2)
                .clipShape(Capsule())
        }
        .buttonStyle(.plain)
        .help(help)
    }
}

#Preview("review — 3 segments") {
    RipReviewPanel(state: RipReviewPanelState(
        mode: .review,
        sideDurationSecs: 1361,
        segments: [
            RipSegmentUi(index: 0, startSecs: 0, endSecs: 452,
                         title: "King Tubby Meets Rockers Uptown",
                         artist: "Augustus Pablo", album: "", genre: "Dub",
                         year: "1976"),
            RipSegmentUi(index: 1, startSecs: 452, endSecs: 878),
            RipSegmentUi(index: 2, startSecs: 878, endSecs: 1361),
        ]))
        .padding()
        .background(DubColor.surface0)
        .frame(width: 900)
}
