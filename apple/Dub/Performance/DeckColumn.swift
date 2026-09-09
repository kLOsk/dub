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

import DubCore
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
    /// Elapsed and remaining tick once a second off the engine, so they
    /// come from a `TimelineView` subview rather than through state —
    /// the same `LiveDeckTimeText` the header band used. Previews and
    /// snapshots leave these nil and get a placeholder.
    var liveEngine: DubEngine?
    var liveDeckIdx: UInt64?
    @ViewBuilder var overview: () -> Overview

    var body: some View {
        // A narrow window cannot hold eight named rows plus everything
        // above them: at the column's floor the bank is one column and
        // the stack wants about 540 pt against a ~280 pt pane. It
        // scrolls there rather than clipping — the same fallback the
        // pad column carried, and for the same reason. On any window
        // wide enough for two cue columns the plain stack fits and no
        // scroll view is built.
        ViewThatFits(in: .vertical) {
            stack
            ScrollView(.vertical, showsIndicators: false) { stack }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        // Opaque, and a step below the app's ground. The deck region
        // paints `DubColor.divider` behind its children so the 1 pt
        // seams between panes show through — a seam colour, not a
        // surface — so a child that paints nothing reads as a grey slab
        // the width of the column. This one paints `surfaceRecessed`,
        // which also gives the column an edge against the waveform
        // beside it without spending a border on it.
        .background(DubColor.surfaceRecessed)
    }

    private var stack: some View {
        VStack(alignment: .leading, spacing: DubSpacing.sm) {
            sourceRow
            identityAndReadouts
            // The map wants air on both sides of it. At the stack's own
            // spacing it sat hard against the artist line above and the
            // HOTCUE heading below, and three blocks with nothing
            // between them read as one crowded block.
            Spacer(minLength: 0)
            overviewAndTimes
            Spacer(minLength: 0)
            cueBank
            Spacer(minLength: 0)
            loopAndEcho
            Spacer(minLength: 0)
        }
        // Flexible spacers rather than fixed gaps: the pane is much
        // taller than the column needs on any real window, and the
        // slack is better spent between the blocks than pooled under
        // the last one. `minLength` is what the fit tests measure, so a
        // short pane still collapses to the tight arrangement.
        .padding(.leading, state.side == .a ? signalTabInset : DubSpacing.md)
        .padding(.trailing, state.side == .a ? DubSpacing.md : signalTabInset)
        .padding(.vertical, DubSpacing.md)
        .frame(maxWidth: .infinity, alignment: .topLeading)
    }

    /// The signal slide-out's tab sits on the deck's outer edge, over
    /// the column. Without this it printed straight through the LOOP
    /// heading.
    private var signalTabInset: CGFloat { DubLayout.deckSignalTabWidth + DubSpacing.sm }

    /// The whole-track map, with elapsed under its start and remaining
    /// under its end — where a DJ reads them off a Serato overview.
    private var overviewAndTimes: some View {
        VStack(alignment: .leading, spacing: 4) {
            overview()
                .frame(height: DubLayout.deckColumnOverviewHeight)
            HStack(spacing: 0) {
                time(.elapsed, colour: DubColor.textSecondary)
                Spacer(minLength: 0)
                time(.remaining, colour: DubColor.textPrimary)
            }
        }
    }

    @ViewBuilder
    private func time(_ slot: LiveDeckTimeText.Slot, colour: Color) -> some View {
        if let liveEngine, let liveDeckIdx, state.hasTrack {
            LiveDeckTimeText(engine: liveEngine, deckIdx: liveDeckIdx, slot: slot)
                .font(DubFont.numericInline)
                .monospacedDigit()
                .foregroundStyle(colour)
        } else {
            Text(slot == .remaining ? "-00:00" : "00:00")
                .font(DubFont.numericInline)
                .monospacedDigit()
                .foregroundStyle(DubColor.textPlaceholder)
        }
    }

    // MARK: - Identity

    /// The source switch is the only part of the old header band that
    /// was a *control* rather than a readout, so it heads the column it
    /// switches. INT is the play control — see `SourceControlView`.
    private var sourceRow: some View {
        // Always drawn, even with no timecode input — `.off` is the
        // status for exactly that, and it renders the switch with no
        // segment lit. The old header band hid the switch and showed
        // bare transport glyphs instead, so on a machine with no
        // interface there was no INT to press; here INT *is* the play
        // control, so hiding it removes the transport.
        SourceControlView(
            status: state.header.sourceControl ?? .off,
            overridden: state.header.sourceControlOverridden,
            isPlaying: state.isPlaying,
            side: state.side,
            onInternal: callbacks.onSetInternal,
            onPause: callbacks.onPause,
            onTimecode: callbacks.onSetTimecode,
            onThru: callbacks.onSetThru,
            onRecalibrate: callbacks.onRecalibrate)
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
    /// to be the same object on both surfaces. `columns: nil` lets the
    /// bank pick the widest arrangement its width affords: three at a
    /// full-screen window, one at the column's floor.
    private var cueBank: some View {
        CueRowBank(
            slots: state.cues,
            hasTrack: state.hasTrack,
            isPlaying: state.isPlaying,
            onCue: callbacks.onCue,
            onPreviewDown: callbacks.onPreviewDown,
            onPreviewUp: callbacks.onPreviewUp,
            onRename: callbacks.onRenameCue,
            onColor: callbacks.onColorCue,
            columns: nil,
            rowHeight: DubLayout.cueRowHeight)
    }

    /// The loop keeps its natural width rather than stretching to the
    /// column — it is an instrument with a fixed shape, and a ×2 button
    /// 300 pt from the ÷2 is a worse control, not a bigger one. Echo out
    /// sits *beside* it: both are fired during a transition, so the hand
    /// stays in one place for the whole move. On a column too narrow to
    /// hold both it drops underneath rather than squeezing the loop.
    private var loopAndEcho: some View {
        HStack(alignment: .top, spacing: DubSpacing.md) {
            VStack(alignment: .leading, spacing: DubSpacing.sm) {
                SectionHeading(
                    title: "LOOP", accent: DubColor.loop,
                    trailing: state.loopEngaged ? "● ACTIVE" : "○ IDLE",
                    trailingAccent: state.loopEngaged
                        ? DubColor.loop : DubColor.textPlaceholder)
                loop
            }
            if state.echoEnabled {
                VStack(alignment: .leading, spacing: DubSpacing.sm) {
                    SectionHeading(
                        title: "ECHO", accent: DubColor.deckTint(state.side),
                        trailing: state.echoEngaged ? "● OUT" : nil,
                        trailingAccent: DubColor.deckTint(state.side))
                    echoButton
                }
                .frame(width: DubLayout.deckColumnEchoWidth)
            }
        }
    }

    /// The loop keeps its own shape rather than stretching to the
    /// column — a ×2 button 300 pt from the ÷2 is a worse control, not
    /// a bigger one — but it may *compress*, because echo out sits
    /// beside it at every width and the pair has to clear the column's
    /// floor. `LoopEngine`'s size buttons flex between 40 and 52.
    private var loop: some View {
        LoopEngine(
            activeBeats: state.activeLoopBeats,
            engaged: state.loopEngaged,
            hasTrack: state.hasTrack,
            showsHeading: false,
            contentHeight: DubLayout.deckColumnLoopHeight,
            onLoop: callbacks.onLoop,
            onScale: callbacks.onScaleLoop,
            onExit: callbacks.onExitLoop)
            .frame(maxWidth: DubLayout.prepLoopSection, alignment: .leading)
    }

    private var echoButton: some View {
        Text("ECHO OUT")
            .font(DubFont.caps)
            .tracking(DubFont.capsTracking)
            .foregroundStyle(
                state.echoEngaged ? DubColor.textPrimary : DubColor.textSecondary)
            .frame(maxWidth: .infinity)
            .frame(height: 44)
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
