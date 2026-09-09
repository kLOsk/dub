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
struct DeckColumnState: Equatable {
    var side: DeckSide = .a

    /// Only what the column draws — *not* the whole `DeckHeaderState`.
    ///
    /// It used to hold the header wholesale, which meant the column's
    /// equality check saw fields it never renders. `measureProgress` is
    /// a continuous 0…1 bar phase and `pitchPercent` moves with the
    /// platter, so while a deck played the state differed on every
    /// 10 Hz poll and `.equatable()` bought nothing: eight cue rows,
    /// the loop and the readouts were rebuilt ten times a second
    /// through a mix. Narrowing the state to the drawn fields is what
    /// makes the comparison mean something.
    var header: DeckColumnHeader
    var cues: [CueSlotState] = (0..<DeckState.hotCueCount).map { CueSlotState(index: $0) }
    var activeLoopBeats: Double?
    var loopEngaged: Bool = false
    var echoEnabled: Bool = false
    var echoEngaged: Bool = false
    var hasTrack: Bool = false
    var isPlaying: Bool = false
}

/// The header's fields that the column actually renders.
struct DeckColumnHeader: Equatable {
    var trackTitle: String?
    var trackArtist: String?
    var bpm: Double?
    var key: String?
    /// Rounded to the tenth the readout prints. The raw value moves
    /// continuously on a timecode deck, and a difference the DJ cannot
    /// see is not a reason to rebuild the column.
    var pitchTenths: Double?
    var sourceControl: SourceControlStatus?
    var sourceControlOverridden: Bool

    init(_ state: DeckHeaderState) {
        trackTitle = state.trackTitle
        trackArtist = state.trackArtist
        bpm = state.bpm
        key = state.key
        pitchTenths = state.pitchPercent.map { ($0 * 10).rounded() / 10 }
        sourceControl = state.sourceControl
        sourceControlOverridden = state.sourceControlOverridden
    }
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

/// Skippable: SwiftUI compares `state` and rebuilds only when it moved.
///
/// The model polls the engine ten times a second and republishes when
/// any field of `DeckState` changes — a pitch reading or a lock byte is
/// enough. Without this, each of those rebuilt both columns entire:
/// eight cue rows, the loop, the readouts, the lot. The callbacks and
/// the overview closure are deliberately not compared; they are rebuilt
/// every time by their call site and comparing them is impossible, but
/// they are pure forwarders into the model and carry no state of their
/// own.
extension DeckColumn: Equatable {
    static func == (lhs: DeckColumn, rhs: DeckColumn) -> Bool {
        lhs.state == rhs.state
    }
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
        // **No `ViewThatFits` here.** It was a scroll fallback for short
        // panes, and it cost a core. `ViewThatFits` builds *every*
        // candidate to measure it, so wrapping the stack made two
        // copies of it — and `CueRowBank` inside has a `ViewThatFits`
        // of its own, so each copy built the eight cue rows twice
        // again. Four full builds of the bank per layout pass, per
        // deck, re-run on every pass: the main thread sat at 100 % with
        // the app idle, and `CueRowBank.grid` was the top frame in the
        // sample. One `ViewThatFits` is affordable; nesting them is
        // not.
        //
        // A pane too short for the column now clips rather than
        // scrolls. That is the honest trade at 900 pt — see
        // `PerformanceLayoutTests`.
        stack
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        // `surface1` — the signal panel's ground, traded with it: the
        // column is the larger, quieter surface of the two and wants
        // the lower step, while the panel slides *over* the column and
        // needs to sit above it. It has to paint something opaque
        // regardless: the deck region paints `DubColor.divider` behind
        // its children so the 1 pt seams between panes show through —
        // a seam colour, not a surface — and a child that paints
        // nothing reads as a grey slab the width of the column.
        .background(DubColor.surface1)
    }

    private var stack: some View {
        VStack(alignment: .leading, spacing: DubSpacing.sm) {
            sourceRow
            identityAndReadouts
            // A little more than the stack's own gap above the three
            // blocks that start a new thought: the times, the marks and
            // the loop. Eight points between everything read as one
            // undifferentiated list.
            overviewAndTimes
                .padding(.top, DubSpacing.sm)
            cueBank
                .padding(.top, DubSpacing.sm)
            loopAndEcho
                .padding(.top, DubSpacing.sm)
            Spacer(minLength: 0)
        }
        // Tight, with the slack at the bottom. Distributing it between
        // the blocks was tried and read as five things drifting apart
        // rather than one instrument: a control surface wants its
        // groups close and the empty space in one place.
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
            HStack(spacing: 0) {
                time(.elapsed, colour: DubColor.textSecondary)
                Spacer(minLength: 0)
                time(.remaining, colour: DubColor.textPrimary)
            }
            // No `.frame(height:)` here. The overview pins its own
            // height and the inner frame wins, so a wrapper only
            // reserves space the view then draws outside of. It takes
            // the height as a parameter instead — see
            // `TrackOverviewView.height`.
            overview()
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
        HStack(spacing: 0) {
            Spacer(minLength: DubSpacing.lg)
            sourceSwitch
        }
        .padding(.bottom, DubSpacing.xs)
    }

    private var sourceSwitch: some View {
        // Always drawn, and `.internalPlay` — not `.off` — when the
        // deck has no timecode input. A deck with no input *is* on its
        // internal clock; `.off` left the INT segment unlit, so it
        // called `onInternal` forever and never `onPause`. That is why
        // the play button did nothing and why a deck started by a hot
        // cue could not be stopped: the one control that pauses it was
        // permanently in its "start" state.
        SourceControlView(
            status: state.header.sourceControl ?? .internalPlay,
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
            readout("KEY", state.header.key, tint: DubColor.camelotKey(state.header.key))
            readout("PITCH", state.header.pitchTenths.map { String(format: "%+.1f", $0) })
        }
        .fixedSize()
    }

    private func readout(_ label: String, _ value: String?, tint: Color? = nil) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(label)
                .font(DubFont.micro)
                .tracking(DubFont.capsTracking)
                .foregroundStyle(DubColor.textTertiary)
            Text(value ?? "—")
                .font(DubFont.numericLarge)
                .monospacedDigit()
                .foregroundStyle(
                    value == nil ? DubColor.textPlaceholder : (tint ?? DubColor.textPrimary))
        }
    }

    // MARK: - The two things you play

    /// Named rows, as in Prep — a cue is a *named position*, and it has
    /// to be the same object on both surfaces. Two columns at every
    /// width: eight split three ways is a ragged 3/3/2, and a bank that
    /// measures candidates to choose costs more than it is worth (see
    /// `CueRowBank.columns`).
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
            columns: 2,
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
            .frame(height: DubLayout.deckColumnEchoHeight)
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
