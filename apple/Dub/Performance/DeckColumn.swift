//
//  DeckColumn.swift
//  Dub
//
//  Performance's per-deck column: everything about one deck that is not
//  the playing waveform.
//
//  ## Why the header came down here
//
//  Performance used to spend a full-width 108 pt band on two blocks of
//  text and six numbers, stacked above both decks. That band was the
//  block making the vertical budget impossible — `deckHeaderHeight` 108
//  + `waveformMinHeight` 280 + `rackBarHeight` 92 + `libraryMinHeight`
//  200 is 680 pt of minimums inside a 600 pt minimum window — and it was
//  the only one of the four whose content had somewhere else to go.
//  Turning it ninety degrees is Scratch Live's own answer: deck identity
//  beside the record rather than above it, and the strip gets the height
//  back.
//
//  ## Why the column is not a fixed width
//
//  It used to be `performancePadColumnWidth` exactly, with the slack
//  left over from the capped waveform sitting unclaimed at each outer
//  edge — several hundred points of nothing on a laptop screen. The
//  column now takes that space, and the sections inside it use it: the
//  overview gets real horizontal resolution, and `CueRowBank` picks its
//  column count from the width it is handed, so eight cues are one
//  column when the window is narrow and two or three when it is not.
//
//  ## What it does not own
//
//  The playing waveform, the phase clock and the rack. Sampler, siren,
//  quick scratch and FX are one-at-a-time tabs in `GlobalRackBar` —
//  they are shared across decks, so a per-deck column is the wrong home
//  for them.
//

import SwiftUI

/// Everything the column draws. `header` is the same value the old
/// header band consumed, so title, tempo, key, pitch and the source
/// switch keep one source of truth.
struct DeckColumnState {
    var side: DeckSide = .a
    var header: DeckHeaderState
    var cues: [CueSlotState] = (0..<DeckState.hotCueCount).map { CueSlotState(index: $0) }
    var activeLoopBeats: Double?
    var loopEngaged: Bool = false
    var echoEnabled: Bool = false
    var echoEngaged: Bool = false
    var hasTrack: Bool = false
    var isPlaying: Bool = false
}

/// Every write the column makes, as closures — the same shape as
/// `PrepRackCallbacks`, so the two surfaces wire up identically.
struct DeckColumnCallbacks {
    var onCue: (_ index: Int, _ clear: Bool) -> Void = { _, _ in }
    var onPreviewDown: (_ index: Int) -> Void = { _ in }
    var onPreviewUp: () -> Void = {}
    var onRenameCue: (_ index: Int) -> Void = { _ in }
    var onColorCue: (_ index: Int, _ token: String?) -> Void = { _, _ in }
    var onLoop: (_ beats: Double) -> Void = { _ in }
    var onScaleLoop: (_ double: Bool) -> Void = { _ in }
    var onExitLoop: () -> Void = {}
    var onEchoToggle: () -> Void = {}
    var onSetInternal: () -> Void = {}
    var onPause: () -> Void = {}
    var onSetTimecode: () -> Void = {}
    var onSetThru: () -> Void = {}
    var onRecalibrate: () -> Void = {}
}

/// One deck's column.
///
/// The overview arrives as a closure rather than as state because it is
/// a Metal view driven by the model, and the rest of this file is
/// values-in / closures-out so it can be snapshot-tested. Tests pass a
/// placeholder of the same height.
struct DeckColumn<Overview: View>: View {
    let state: DeckColumnState
    var callbacks = DeckColumnCallbacks()
    @ViewBuilder var overview: () -> Overview

    var body: some View {
        VStack(alignment: .leading, spacing: DubSpacing.sm) {
            sourceRow
            identityAndReadouts
            // Full column width. The overview was a 26 pt vertical
            // sliver on the deck's outer edge — the one orientation
            // that makes a whole-track map hard to read, and the reason
            // it was easy to ignore. Across the column it gets the
            // width to be a map.
            overview()
                .frame(height: DubLayout.deckColumnOverviewHeight)
            cueBank
            loopAndEcho
            Spacer(minLength: 0)
        }
        .padding(.horizontal, DubSpacing.lg)
        .padding(.vertical, DubSpacing.sm)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    // MARK: - Identity

    /// The source switch is the only part of the old header band that
    /// was a *control* rather than a readout, so it heads the column it
    /// switches. INT is the play control — see `SourceControlView`.
    @ViewBuilder
    private var sourceRow: some View {
        if let status = state.header.sourceControl {
            SourceControlView(
                status: status,
                overridden: state.header.sourceControlOverridden,
                isPlaying: state.isPlaying,
                side: state.side,
                onInternal: callbacks.onSetInternal,
                onPause: callbacks.onPause,
                onTimecode: callbacks.onSetTimecode,
                onThru: callbacks.onSetThru,
                onRecalibrate: callbacks.onRecalibrate)
        }
    }

    /// Identity and readouts share a row. The column is wide — that is
    /// the point of it taking the slack — and stacking a 46 pt readout
    /// strip under a 40 pt title block spends height the strip beside it
    /// wants. The old header band put them side by side for the same
    /// reason.
    private var identityAndReadouts: some View {
        HStack(alignment: .top, spacing: DubSpacing.lg) {
            identity
            readouts
        }
    }

    private var identity: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(state.header.trackTitle ?? "No track loaded")
                .font(DubFont.title)
                .foregroundStyle(
                    state.header.trackTitle == nil
                        ? DubColor.textPlaceholder : DubColor.textPrimary)
                .lineLimit(2)
            Text(state.header.trackArtist ?? "—")
                .font(DubFont.body)
                .foregroundStyle(DubColor.textSecondary)
                .lineLimit(1)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// Tempo, key and pitch. Pitch tracks the platter in Performance, so
    /// it stays — unlike Prep, where it can only ever read `+0.0 %`.
    private var readouts: some View {
        HStack(alignment: .top, spacing: DubSpacing.md) {
            readout("BPM", state.header.bpm.map { String(format: "%.1f", $0) })
            readout("KEY", state.header.key)
            readout("PITCH", state.header.pitchPercent.map { String(format: "%+.1f", $0) })
        }
        .fixedSize()
    }

    private func readout(_ label: String, _ value: String?) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(label)
                .font(DubFont.micro)
                .tracking(DubFont.capsTracking)
                .foregroundStyle(DubColor.textTertiary)
            Text(value ?? "—")
                .font(DubFont.numericLarge)
                .monospacedDigit()
                .foregroundStyle(value == nil ? DubColor.textPlaceholder : DubColor.textPrimary)
        }
    }

    // MARK: - The two things you play

    /// Named rows, as in Prep — a cue is a *named position*, and it has
    /// to be the same object on both surfaces. The column count follows
    /// the width: `ViewThatFits` takes the widest arrangement that fits,
    /// so a wide window gets three columns and the minimum window one.
    private var cueBank: some View {
        ViewThatFits(in: .horizontal) {
            bank(columns: 3)
            bank(columns: 2)
            bank(columns: 1)
        }
    }

    private func bank(columns: Int) -> some View {
        CueRowBank(
            slots: state.cues,
            hasTrack: state.hasTrack,
            isPlaying: state.isPlaying,
            onCue: callbacks.onCue,
            onPreviewDown: callbacks.onPreviewDown,
            onPreviewUp: callbacks.onPreviewUp,
            onRename: callbacks.onRenameCue,
            onColor: callbacks.onColorCue,
            columns: columns)
    }

    /// The loop keeps its natural width rather than stretching to the
    /// column — it is an instrument with a fixed shape, and a ×2 button
    /// 300 pt from the ÷2 is a worse control, not a bigger one. Echo out
    /// sits under it: both are fired during a transition, so the hand
    /// stays in one place for the whole move.
    private var loopAndEcho: some View {
        VStack(alignment: .leading, spacing: DubSpacing.sm) {
            LoopEngine(
                activeBeats: state.activeLoopBeats,
                engaged: state.loopEngaged,
                hasTrack: state.hasTrack,
                onLoop: callbacks.onLoop,
                onScale: callbacks.onScaleLoop,
                onExit: callbacks.onExitLoop)
                .frame(width: DubLayout.prepLoopSection, alignment: .leading)
            if state.echoEnabled {
                echoButton
            }
        }
    }

    private var echoButton: some View {
        Text("ECHO OUT")
            .font(DubFont.caps)
            .tracking(DubFont.capsTracking)
            .foregroundStyle(
                state.echoEngaged ? DubColor.textPrimary : DubColor.textSecondary)
            .padding(.horizontal, DubSpacing.md)
            .frame(height: 28)
            .background(
                state.echoEngaged
                    ? DubColor.deckTint(state.side).opacity(0.26) : DubColor.surface2)
            .clipShape(RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous)
                    .stroke(
                        state.echoEngaged
                            ? DubColor.deckTint(state.side) : DubColor.divider,
                        lineWidth: 1))
            .contentShape(Rectangle())
            .onPressDown(enabled: state.hasTrack) { callbacks.onEchoToggle() }
            .help("Echo out — cut the dry signal and hand the deck to the echo")
    }
}
