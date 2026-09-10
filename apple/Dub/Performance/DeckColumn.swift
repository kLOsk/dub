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

    /// The large face, the same as BPM: remaining time is the "thirty
    /// seconds to mix" cue and is read at the same distance, with the
    /// same urgency. Elapsed matches it so the two ends of the overview
    /// read as one pair.
    @ViewBuilder
    private func time(_ slot: LiveDeckTimeText.Slot, colour: Color) -> some View {
        if let liveEngine, let liveDeckIdx, state.hasTrack {
            LiveDeckTimeText(engine: liveEngine, deckIdx: liveDeckIdx, slot: slot)
                .font(DubFont.numericLarge)
                .monospacedDigit()
                .foregroundStyle(colour)
        } else {
            Text(slot == .remaining ? "-00:00" : "00:00")
                .font(DubFont.numericLarge)
                .monospacedDigit()
                .foregroundStyle(DubColor.textPlaceholder)
        }
    }

    // MARK: - Identity

    /// The source switch is the only part of the old header band that
    /// was a *control* rather than a readout, so it heads the column it
    /// switches. INT is the play control — see `SourceControlView`.
    /// The switch sits on the column's *inner* edge — toward the
    /// strip it drives. Deck A's column is left of its waveform so the
    /// switch is right-aligned; deck B's is right of its waveform so it
    /// is left-aligned. The readouts under it mirror the same way (see
    /// `identityAndReadouts`); the marks and the loop do not.
    private var sourceRow: some View {
        HStack(spacing: 0) {
            if state.side == .a { Spacer(minLength: DubSpacing.lg) }
            sourceSwitch
            if state.side == .b { Spacer(minLength: DubSpacing.lg) }
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
    ///
    /// The readouts take the column's *inner* edge on both decks —
    /// right on deck A, left on deck B — so the numbers sit beside the
    /// strip they describe and the title runs out to the window's edge.
    /// Mirrored like the source switch above them; the order inside
    /// stays BPM · KEY · PITCH on both, one reading order to learn.
    private var identityAndReadouts: some View {
        HStack(alignment: .top, spacing: DubSpacing.lg) {
            if state.side == .a {
                identity
                readouts
            } else {
                readouts
                identity
            }
        }
    }

    private var identity: some View {
        let outward = state.side == .a
        return VStack(alignment: outward ? .leading : .trailing, spacing: 2) {
            Text(state.header.trackTitle ?? "No track loaded")
                .font(DubFont.title)
                .foregroundStyle(
                    state.header.trackTitle == nil
                        ? DubColor.textPlaceholder : DubColor.textPrimary)
                .lineLimit(2)
                .multilineTextAlignment(outward ? .leading : .trailing)
            // A step below the title, not level with it. The two ran
            // at `textPrimary` and `textSecondary`, which is a small
            // enough gap that at a glance the pair read as one block of
            // text rather than as a name and its artist.
            Text(state.header.trackArtist ?? "—")
                .font(DubFont.body)
                .foregroundStyle(DubColor.textTertiary)
                .lineLimit(1)
        }
        .frame(maxWidth: .infinity, alignment: outward ? .leading : .trailing)
    }

    private var readouts: some View {
        DeckReadouts(header: state.header, trailing: state.side == .a)
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
            columns: 4,
            rowHeight: DubLayout.cueRowHeight)
    }

    /// One row, two controls at one height: the loop runs from the
    /// column's edge to the echo, and the echo keeps its fixed width.
    /// Both are fired during a transition, so the hand stays in one
    /// place for the whole move.
    ///
    /// The loop used to keep a natural width of its own, on the
    /// argument that a ×2 button far from the ÷2 is a worse control.
    /// What that left was a stroked box 268 pt wide with the column's
    /// slack sitting empty between it and the echo — the DJ asked for
    /// the loop to fill that gap and to stand as tall as the echo, and
    /// the steppers stay at the row's two ends where the hand already
    /// finds them.
    private var loopAndEcho: some View {
        HStack(alignment: .top, spacing: DubSpacing.md) {
            VStack(alignment: .leading, spacing: DubSpacing.sm) {
                SectionHeading(
                    title: "LOOP", accent: DubColor.loop,
                    trailing: state.loopEngaged ? "● ON" : "○ OFF",
                    trailingAccent: state.loopEngaged
                        ? DubColor.loop : DubColor.textPlaceholder)
                loop
            }
            if state.echoEnabled {
                VStack(alignment: .leading, spacing: DubSpacing.sm) {
                    SectionHeading(
                        title: "ECHO", accent: DubColor.controlAccent,
                        trailing: state.echoEngaged ? "● ON" : "○ OFF",
                        trailingAccent: state.echoEngaged
                            ? DubColor.controlAccent : DubColor.textPlaceholder)
                    echoButton
                }
                .frame(width: DubLayout.deckColumnEchoWidth)
            }
        }
    }

    /// Takes every point between the column's edge and the echo. It may
    /// also *compress*, because echo out sits beside it at every width
    /// and the pair has to clear the column's floor —
    /// `LoopEngine.minWidth` is what the floor has to hold.
    private var loop: some View {
        LoopEngine(
            activeBeats: state.activeLoopBeats,
            engaged: state.loopEngaged,
            hasTrack: state.hasTrack,
            height: DubLayout.deckColumnLoopHeight,
            onLoop: callbacks.onLoop,
            onScale: callbacks.onScaleLoop,
            onExit: callbacks.onExitLoop)
            .frame(maxWidth: .infinity)
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
                    ? DubColor.controlAccent.opacity(0.26) : DubColor.surface2)
            .clipShape(RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous)
                    .stroke(
                        state.echoEngaged
                            ? DubColor.controlAccent : DubColor.divider,
                        lineWidth: 1))
            .contentShape(Rectangle())
            .onPressDown(enabled: state.hasTrack) { callbacks.onEchoToggle() }
            .help("Echo out — cut the dry signal and hand the deck to the echo")
    }
}

/// Tempo, key and pitch. Pitch tracks the platter in Performance, so
/// it stays — unlike Prep, where it can only ever read `+0.0 %`.
///
/// Each value reserves the width of the widest string it can print,
/// so a neighbour never moves when a digit arrives: pitch going from
/// `+0.4` to `+10.4` used to shove BPM and KEY sideways, on a row the
/// DJ reads while beatmatching. BPM is the number that matters and
/// keeps the large face; key and pitch step down to the inline size,
/// which is what the type ramp names them for.
///
/// Its own view so `PerformanceLayoutTests` can hold the width steady
/// across values without reaching into the column.
struct DeckReadouts: View {
    let header: DeckColumnHeader
    /// Hang each slot's label and value from its trailing edge rather
    /// than its leading one. The slots are wider than most values —
    /// pitch holds six places for a `+0.4` — so the values should hug
    /// the edge the row sits against: trailing on deck A, where the
    /// readouts end at the column's inner edge, leading on deck B.
    var trailing: Bool = false

    var body: some View {
        HStack(alignment: .top, spacing: DubSpacing.md) {
            readout(
                "BPM", header.bpm.map { String(format: "%.1f", $0) },
                font: DubFont.numericLarge, widest: "888.8")
            readout(
                "KEY", header.key,
                font: DubFont.numericInline, widest: "12B",
                tint: DubColor.camelotKey(header.key))
            // Six places: the sign, three digits, the point and a tenth.
            // A scratch drives the smoothed rate well past ±100 %, and
            // the slot has to hold that too or the row jumps mid-cut.
            readout(
                "PITCH", header.pitchTenths.map { String(format: "%+.1f", $0) },
                font: DubFont.numericInline, widest: "-888.8")
        }
        .fixedSize()
    }

    /// `widest` is drawn hidden underneath the value and sets the
    /// slot's width — measured, so the reservation follows the font
    /// rather than a hand-typed point count.
    private func readout(
        _ label: String, _ value: String?, font: Font, widest: String, tint: Color? = nil
    ) -> some View {
        VStack(alignment: trailing ? .trailing : .leading, spacing: 2) {
            Text(label)
                .font(DubFont.micro)
                .tracking(DubFont.capsTracking)
                .foregroundStyle(DubColor.textTertiary)
            ZStack(alignment: trailing ? .trailing : .leading) {
                Text(widest)
                    .font(font)
                    .monospacedDigit()
                    .hidden()
                Text(value ?? "—")
                    .font(font)
                    .monospacedDigit()
                    .foregroundStyle(
                        value == nil ? DubColor.textPlaceholder : (tint ?? DubColor.textPrimary))
            }
        }
    }
}
