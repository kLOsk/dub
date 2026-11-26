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
/// The height a deck column's blocks take, padding included — the taller
/// of the two decks. The Performance surface cuts the deck region to it:
/// the waveform strip is no longer than the columns beside it, and what
/// used to be empty space under them is the library's (Daniel,
/// 2026-11-14).
struct DeckColumnHeightKey: PreferenceKey {
    static let defaultValue: CGFloat = 0
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) {
        value = max(value, nextValue())
    }
}

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
    /// `DeckState.hotCuesFrom`: the app the pads' cues came from.
    var cuesFrom: String?
    var activeLoopBeats: Double?
    var loopEngaged: Bool = false
    var echoEnabled: Bool = false
    var echoEngaged: Bool = false
    /// HOLD on the echo: the thrown beat sustains through a record swap.
    var echoHeld: Bool = false
    var hasTrack: Bool = false
    /// THRU: a live record with no track loaded. Echo out is the
    /// one-turntable throw, so it answers here too. The column then draws
    /// the record rather than a track — see `ThruDeck.swift`.
    var isThru: Bool = false
    /// THRU, and sound has come through since the needle last came up.
    var thruHasRecord: Bool = false
    /// Where a THRU deck's record comes in: the interface and its pair.
    var thruInput: String? = nil
    var isPlaying: Bool = false
    /// The four Quick Scratch pads (PRD §7.2), in pad order. An untagged
    /// pad is drawn dark and dead so the hand's positions never move.
    var scratch: [ScratchPadState] = (0..<SampleBank.quickScratchCount).map {
        ScratchPadState(pad: $0)
    }
    /// Whether the source switch grows its `DUB FX` position — the FX
    /// channel is a Preferences opt-in (F-38).
    var fxAvailable: Bool = false
    /// The key on ECHO OUT, from the map; `nil` until bound.
    var echoLegend: String?
    /// The key on HOLD, from the map; `nil` until bound.
    var echoHoldLegend: String?
}

/// One Quick Scratch pad as the column draws it.
struct ScratchPadState: Equatable, Identifiable {
    var pad: Int
    /// The tagged sample's name; `nil` when no sample answers to the pad.
    var name: String?
    /// The tagged sample's shape, as its SAMPLES pad draws it; empty
    /// until the engine has read it.
    var wave: [Float] = []
    /// `true` while this pad's sample is on the deck.
    var engaged: Bool = false
    /// The key that fires the pad, from the map; `nil` until bound.
    var legend: String?
    /// The tagged sample's file is gone or unreadable: the pad says so
    /// rather than showing a name with no shape that looks ready.
    var fault: SampleFault?

    var id: Int { pad }
}

/// The header's fields that the column actually renders.
struct DeckColumnHeader: Equatable {
    var trackTitle: String?
    var trackArtist: String?
    /// The tempo the record is **playing at** — `DeckHeaderState.liveBpm`,
    /// the grid BPM scaled by the platter's pitch. This used to be the
    /// raw grid BPM, so pitching a deck moved PITCH and left BPM
    /// standing (found on the rig, 2026-09-22).
    var bpm: Double?
    /// `DeckHeaderState.bpmLabel`: `BPM`, or `BPM · FOLLOW` on a grid
    /// that follows its band.
    var bpmLabel: String = "BPM"
    var key: String?
    /// M14 key lock: what the DJ set, and what the engine is doing with
    /// it (0 off · 1 standby · 2 engaged).
    var keyLockOn: Bool = false
    var keyLockState: UInt8 = 0
    /// Steadying, as the engine has it (0 off · 1 on, nothing held for
    /// now · 2 held to its tempo).
    var steadyState: UInt8 = 0
    /// Rounded to the tenth the readout prints. The raw value moves
    /// continuously on a timecode deck, and a difference the DJ cannot
    /// see is not a reason to rebuild the column.
    var pitchTenths: Double?
    /// What the live BPM readout multiplies — the grid's tempo and the
    /// settled pitch it falls back to past the fader's band. Both move
    /// rarely; the live pitch itself is read by the layer.
    var baseBpm: Double?
    var tempoPitchPercent: Double?
    /// A THRU deck's tempo — the record's, tracked or tapped. Replaces the
    /// track BPM in the readout.
    var thruTempo: ThruTempoReadout?
    /// The loaded track's grid is locked: the BPM takes no taps, and
    /// shows the padlock the Prep header does.
    var gridLocked: Bool = false
    /// The grid follows the band (`DeckHeaderState.followingGrid`): the
    /// BPM, its label and its padlock wear `DubColor.gridMoving`.
    var gridMoves: Bool = false
    /// `DeckHeaderState.gridFollowWord` / `.followChoiceOffered`.
    var gridFollowWord: Bool? = nil
    var followChoiceOffered: Bool = false
    /// "↝ usually: <track>" — the track most often mixed into from this
    /// one, `nil` with none or with the hint off in Preferences.
    var historyHint: String?
    var sourceControl: SourceControlStatus?
    var sourceControlOverridden: Bool
    var timecodeReadiness: TimecodeReadiness
    /// Set while Quick Scratch has a sample on the deck: what is parked
    /// underneath and where it is. The identity block then shouts.
    var scratch: ScratchBadge?
    /// The master deck (PRD §6.4) — the one the crowd is hearing, the
    /// one the siren and sampler land on, the one Space-load avoids.
    var isMaster: Bool

    init(_ state: DeckHeaderState, showsHistoryHint: Bool = true, scratch: ScratchBadge? = nil) {
        trackTitle = state.trackTitle
        trackArtist = state.trackArtist
        // Rounded to the tenth the readout prints, like `pitchTenths`:
        // it follows the live pitch now, and a raw value that moves on
        // every poll would rebuild the column — and re-lay-out the
        // window — for a difference nobody can see.
        bpm = state.liveBpm.map { ($0 * 10).rounded() / 10 }
        key = state.key
        keyLockOn = state.keyLockOn
        keyLockState = state.keyLockState
        steadyState = state.steadyState
        pitchTenths = state.pitchPercent.map { ($0 * 10).rounded() / 10 }
        baseBpm = state.bpm
        tempoPitchPercent = state.tempoPitchPercent
        thruTempo = state.thruTempo
        gridLocked = state.gridLocked
        gridMoves = state.followingGrid
        gridFollowWord = state.gridFollowWord
        followChoiceOffered = state.followChoiceOffered
        historyHint = showsHistoryHint ? state.historyHint : nil
        bpmLabel = state.bpmLabel
        sourceControl = state.sourceControl
        sourceControlOverridden = state.sourceControlOverridden
        timecodeReadiness = state.timecodeReadiness
        self.scratch = scratch
        isMaster = state.isMaster
    }
}

/// What the header says about a quick scratch in progress.
struct ScratchBadge: Equatable {
    /// The parked tune's title, or `nil` when the deck was empty.
    var parkedTitle: String?
    /// Where the parked tune is, whole seconds — it moves under slip,
    /// and whole seconds is what the `m:ss` readout can show, so the
    /// column is not rebuilt for a change it cannot draw.
    var parkedSecs: Int
    /// The deck the tune is playing on meanwhile, when it was doubled
    /// across; `nil` when it is parked silently underneath.
    var playingOn: DeckSide? = nil
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
    var onEchoHold: () -> Void = {}
    /// A tap on the BPM — THRU or track, `MainViewModel.tapBpm`.
    var onTapTempo: () -> Void = {}
    /// The track BPM's right-click, as in Prep's header: 2×, ½, back to
    /// the analysed grid, and the grid lock.
    var onDoubleBpm: () -> Void = {}
    var onHalveBpm: () -> Void = {}
    var onResetBpm: () -> Void = {}
    var onToggleGridLocked: () -> Void = {}
    var onSetGridFollow: (Bool?) -> Void = { _ in }
    /// A click on "↝ usually:" — show that track in the library, so
    /// Space loads it.
    var onHistoryHint: () -> Void = {}
    /// The BPM's right-click on a THRU deck: scale the tempo (×2, ÷2 …).
    var onScaleTempo: (_ factor: Double) -> Void = { _ in }
    /// The BPM's right-click: back to the tempo the tracker hears.
    var onResetTempo: () -> Void = {}
    /// Quick Scratch pad `pad` pressed: engage its sample, swap to it,
    /// or — if it is the one on the deck — release.
    var onScratch: (_ pad: Int) -> Void = { _ in }
    /// Rename… on a Quick Scratch pad: renames the SAMPLES slot behind it.
    var onRenameScratch: (_ pad: Int) -> Void = { _ in }
    var onSetInternal: () -> Void = {}
    var onPause: () -> Void = {}
    var onSetTimecode: () -> Void = {}
    var onSetThru: () -> Void = {}
    var onSetFx: () -> Void = {}
    /// M14 key lock on / off. `nil` hides the button — Prep has no
    /// platter to hold a pitch against.
    var onToggleKeyLock: (() -> Void)? = nil
    /// Steadying on / off, for a deck whose grid follows its band.
    var onToggleSteady: (() -> Void)? = nil
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
    /// Map mode draws its own cap on every pad; the pads' step aside.
    @Environment(\.dubMapping) private var mapping
    var callbacks = DeckColumnCallbacks()
    /// Elapsed and remaining tick once a second off the engine, so they
    /// come from a `TimelineView` subview rather than through state —
    /// the same `LiveDeckTimeText` the header band used. Previews and
    /// snapshots leave these nil and get a placeholder.
    var liveEngine: DubEngine?
    var liveDeckIdx: UInt64?
    /// The open tap session on this deck's track, for the tap count by
    /// the BPM. Observed by the readouts alone, so a tap does not
    /// rebuild the column.
    var tapSession: TapSessionViewModel?
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
        VStack(alignment: .leading, spacing: 0) {
            blocks
                // What the column draws, top to bottom: the height the
                // deck region is cut to, so the strip ends where the
                // trigger row does (`DeckColumnHeightKey`).
                .background(GeometryReader { geo in
                    Color.clear.preference(
                        key: DeckColumnHeightKey.self,
                        value: geo.size.height + DubSpacing.md * 2)
                })
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

    private var blocks: some View {
        VStack(alignment: .leading, spacing: DubSpacing.sm) {
            header
            // A little more than the stack's own gap above the three
            // blocks that start a new thought: the times, the marks and
            // the loop. Eight points between everything read as one
            // undifferentiated list.
            overviewAndTimes
                .padding(.top, DubSpacing.sm)
            if state.isThru {
                // A live record has no cues, loops or quick scratch: its
                // signal and its tempo take their places (2026-11-14).
                thruInputBlock
                    .padding(.top, DubSpacing.sm)
                thruTriggerRow
                    .padding(.top, DubSpacing.sm)
            } else {
                cueBank
                    .padding(.top, DubSpacing.sm)
                triggerRow
                    .padding(.top, DubSpacing.sm)
            }
        }
        .fixedSize(horizontal: false, vertical: true)
    }

    /// The signal slide-out's tab sits on the deck's outer edge, over
    /// the column. Without this it printed straight through the LOOP
    /// heading.
    private var signalTabInset: CGFloat { DubLayout.deckSignalTabWidth + DubSpacing.sm }

    /// The whole-track map, with elapsed under its start and remaining
    /// under its end — where a DJ reads them off a Serato overview.
    private var overviewAndTimes: some View {
        VStack(alignment: .leading, spacing: 4) {
            if state.isThru {
                // How long the record has played, not where a file is:
                // there is no end to count down to.
                HStack(alignment: .lastTextBaseline, spacing: DubSpacing.sm) {
                    thruClock
                    Text(state.thruHasRecord ? "SINCE THE NEEDLE WENT DOWN" : "NEEDLE UP")
                        .font(DubFont.micro)
                        .tracking(DubFont.capsTracking)
                        .foregroundStyle(DubColor.textTertiary)
                    Spacer(minLength: 0)
                    Text("THE SIDE SO FAR")
                        .font(DubFont.micro)
                        .tracking(DubFont.capsTracking)
                        .foregroundStyle(DubColor.textTertiary)
                }
            } else {
                HStack(alignment: .bottom, spacing: 0) {
                    time(.elapsed, colour: DubColor.textSecondary)
                    Spacer(minLength: DubSpacing.md)
                    historyHint
                    Spacer(minLength: DubSpacing.md)
                    time(.remaining, colour: DubColor.textPrimary)
                }
            }
            // No `.frame(height:)` here. The overview pins its own
            // height and the inner frame wins, so a wrapper only
            // reserves space the view then draws outside of. It takes
            // the height as a parameter instead — see
            // `TrackOverviewView.height`.
            // Calibration's progress is in the SIGNAL drawer, which opens
            // for it (`SignalDrawer`, 2026-11-25). It covered this
            // overview from 2026-09-23, when the drawer stayed shut and
            // a held deck said nothing anywhere else.
            overview()
        }
    }

    /// "↝ usually: <track>" in the time row's empty middle (PRD §9.5 row
    /// 3; Daniel picked the spot, 2026-11-16): the DJ's own past answer
    /// to "what next?", the moment a track lands. It gives way first —
    /// the times never move — and a click shows the track in the
    /// library, where Space loads it.
    @ViewBuilder
    private var historyHint: some View {
        if let hint = state.header.historyHint, !hint.isEmpty, state.hasTrack {
            Button(action: callbacks.onHistoryHint) {
                Text("↝ usually: \(hint)")
                    .font(DubFont.micro)
                    .foregroundStyle(DubColor.textTertiary)
                    .lineLimit(1)
                    .truncationMode(.tail)
            }
            .buttonStyle(.plain)
            // Sits on the big digits' baseline, not their box bottom.
            .padding(.bottom, 4)
            .layoutPriority(-1)
            .help("Show “\(hint)” in the library — Space loads it.")
        }
    }

    /// The large face, the same as BPM: remaining time is the "thirty
    /// seconds to mix" cue and is read at the same distance, with the
    /// same urgency. Elapsed matches it so the two ends of the overview
    /// read as one pair.
    @ViewBuilder
    private func time(_ slot: LiveDeckTimeText.Slot, colour: Color) -> some View {
        if let liveEngine, let liveDeckIdx, state.hasTrack {
            LiveDeckTimeText(
                engine: liveEngine, deckIdx: liveDeckIdx, slot: slot,
                size: DubFont.numericLargeSize, color: colour)
        } else {
            Text(slot == .remaining ? "-00:00" : "00:00")
                .font(DubFont.numericLarge)
                .monospacedDigit()
                .foregroundStyle(DubColor.textPlaceholder)
        }
    }

    @ViewBuilder
    private var thruClock: some View {
        if let liveEngine, let liveDeckIdx {
            ThruRecordClock(
                engine: liveEngine, deckIdx: liveDeckIdx,
                size: DubFont.numericLargeSize, color: DubColor.textPrimary)
                .fixedSize()
        } else {
            Text(state.thruHasRecord ? "00:00" : "--:--")
                .font(DubFont.numericLarge)
                .monospacedDigit()
                .foregroundStyle(DubColor.textPlaceholder)
        }
    }

    // MARK: - THRU

    /// The input, left and right, where the hot cues sit on a track: the
    /// one thing on a THRU deck that says a signal is coming in, and the
    /// first place a dead channel shows.
    private var thruInputBlock: some View {
        VStack(alignment: .leading, spacing: DubSpacing.sm) {
            SectionHeading(
                title: "INPUT", accent: DubColor.deckTint(state.side),
                trailing: state.thruHasRecord ? "● SIGNAL" : "○ NO SIGNAL",
                trailingAccent: state.thruHasRecord
                    ? DubColor.deckTint(state.side) : DubColor.textPlaceholder)
            ThruMeterPair(
                engine: liveEngine, deckIdx: liveDeckIdx ?? 0,
                tint: DubColor.deckTint(state.side))
                .frame(height: 30)
                .accessibilityLabel("Input level, left and right")
        }
    }

    /// TEMPO beside ECHO: the beat lamp and what the tempo is based on,
    /// then the one-turntable throw.
    private var thruTriggerRow: some View {
        HStack(alignment: .top, spacing: DubSpacing.md) {
            VStack(alignment: .leading, spacing: DubSpacing.sm) {
                SectionHeading(
                    title: "TEMPO", accent: DubColor.deckTint(state.side),
                    trailing: thruTempoStatus,
                    trailingAccent: state.header.thruTempo?.bpm == nil
                        ? DubColor.textPlaceholder : DubColor.deckTint(state.side))
                HStack(spacing: DubSpacing.sm) {
                    ThruBeatLamp(
                        engine: liveEngine, deckIdx: liveDeckIdx ?? 0,
                        bpm: state.header.thruTempo?.tempo,
                        tint: DubColor.deckTint(state.side))
                        .frame(width: 14, height: 14)
                    Text(thruTempoLine)
                        .font(DubFont.micro)
                        .foregroundStyle(DubColor.textSecondary)
                        .lineLimit(2)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .frame(height: DubLayout.deckColumnEchoHeight, alignment: .leading)
            }
            .frame(maxWidth: .infinity)
            echoSection
        }
    }

    private var thruTempoStatus: String {
        guard let t = state.header.thruTempo, t.bpm != nil else { return "○ LISTENING" }
        if t.tapped { return "● YOUR TAP" }
        if let scale = t.scale { return "● HEARD " + ThruTempoReadout.scaleText(scale) }
        return t.locked ? "● LOCKED" : "◐ TENTATIVE"
    }

    private var thruTempoLine: String {
        guard let t = state.header.thruTempo, t.bpm != nil else {
            return state.thruHasRecord
                ? "Listening for the beat. Tap BPM to set it now."
                : "Drop the needle."
        }
        if t.tapped { return "Holds until the needle comes up. Right-click BPM to go back." }
        if t.scale != nil { return "Follows what is heard. Right-click BPM to change it." }
        return "Heard from the record. Tap BPM to set it; right-click for ×2 · ÷2 · ×1.5."
    }

    // MARK: - Identity

    /// The switch, the names and the readouts: the deck's header, and all
    /// of it the handle for an instant double — drag it onto the other
    /// deck, the same thing ⌘⇧← / ⌘⇧→ do (Daniel, 2026-11-26; it was the
    /// title and artist text alone, a small target).
    ///
    /// **Behind the header, not on it.** A drag source on the block takes
    /// the press before any control inside sees it: on the identity block
    /// it left the key lock inert (2026-09-22). Laid underneath, it gets
    /// the presses that land on nothing — gaps, names, readouts, which let
    /// them through (`allowsHitTesting(false)`) — while the switch, BPM's
    /// tap pad, STEADY and the key lock answer their own.
    private var header: some View {
        VStack(alignment: .leading, spacing: DubSpacing.sm) {
            sourceRow
            identityAndReadouts
        }
        .background {
            Color.clear
                .contentShape(Rectangle())
                .onDrag(if: state.hasTrack) {
                    DeckDoubleDrag.provider(for: state.side)
                } preview: { doubleDragChip }
                .help(state.hasTrack ? "Drag onto the other deck for an instant double" : "")
        }
    }

    /// What the drag carries under the pointer: the library's chip,
    /// ♪ Artist — Title, so a double looks like the load it is. With the
    /// drag on a clear layer there was nothing to picture, and the
    /// pointer's + went alone (Daniel, 2026-11-26).
    private var doubleDragChip: some View {
        Image(nsImage: LibraryDragChip.image(
            artist: state.header.trackArtist, title: state.header.trackTitle ?? "—"))
    }

    /// The source switch is the only part of the old header band that
    /// was a *control* rather than a readout, so it heads the column it
    /// switches. INT is the play control — see `SourceControlView`.
    /// The switch sits on the column's *inner* edge — toward the
    /// strip it drives. Deck A's column is left of its waveform so the
    /// switch is right-aligned; deck B's is right of its waveform so it
    /// is left-aligned. The readouts under it mirror the same way (see
    /// `identityAndReadouts`); the marks and the loop do not.
    /// The source switch on the inner end, and the MASTER chip on the
    /// outer — the deck's top corner, where it reads from across the
    /// booth and moves nothing else. The chip is the one the header
    /// band carried before the column replaced it; it went missing in
    /// the move, and with the siren and the sampler both landing on the
    /// master deck it is the thing that says which deck that is.
    private var sourceRow: some View {
        HStack(spacing: 0) {
            if state.side == .a {
                masterChip
                Spacer(minLength: DubSpacing.lg)
                sourceSwitch
            } else {
                sourceSwitch
                Spacer(minLength: DubSpacing.lg)
                masterChip
            }
        }
        .padding(.bottom, DubSpacing.xs)
    }

    /// Drawn only on the master deck; the other deck's corner stays
    /// empty rather than printing a dimmed "not master".
    @ViewBuilder
    private var masterChip: some View {
        if state.header.isMaster {
            let tint = DubColor.deckTint(state.side)
            Text("MASTER")
                .font(DubFont.caps)
                .tracking(0.8)
                .foregroundStyle(tint)
                .padding(.horizontal, DubSpacing.sm)
                .padding(.vertical, 2)
                .overlay(Capsule(style: .continuous).stroke(tint, lineWidth: 1))
                // Part of the header's drag handle, and it keeps its own
                // tooltip, so the drag is on it rather than through it.
                .onDrag(if: state.hasTrack) {
                    DeckDoubleDrag.provider(for: state.side)
                } preview: { doubleDragChip }
                .help("The master deck — the one the siren and samples land on, "
                    + "and the one Space-load avoids.")
        }
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
            readiness: state.header.timecodeReadiness,
            overridden: state.header.sourceControlOverridden,
            isPlaying: state.isPlaying,
            side: state.side,
            onInternal: callbacks.onSetInternal,
            onPause: callbacks.onPause,
            onTimecode: callbacks.onSetTimecode,
            onThru: callbacks.onSetThru,
            showFx: state.fxAvailable,
            onFx: callbacks.onSetFx)
    }

    /// The artist shares a row with the readouts; the title has the next
    /// row to itself. The column is wide — that is the point of it
    /// taking the slack — and the title is the one string here whose
    /// length is not ours to choose, so it gets the full width to run
    /// out to the window's edge, while the artist, a step smaller, sits
    /// beside BPM · KEY · PITCH on the numbers' baseline. The two rows
    /// were the other way round until 2026-09-14 (title beside the
    /// readouts, artist under it): a long title wrapped at the numbers
    /// while the artist line ran under them with room to spare.
    ///
    /// The readouts take the column's *inner* edge on both decks —
    /// right on deck A, left on deck B — so the numbers sit beside the
    /// strip they describe. Mirrored like the source switch above them;
    /// the order inside stays BPM · KEY · PITCH on both, one reading
    /// order to learn.
    private var identityAndReadouts: some View {
        let outward = state.side == .a
        // The names let presses through to the header's drag (`header`).
        return VStack(alignment: outward ? .leading : .trailing, spacing: 2) {
            HStack(alignment: .lastTextBaseline, spacing: DubSpacing.lg) {
                if outward {
                    subtitleLine
                        .allowsHitTesting(false)
                    readouts
                } else {
                    readouts
                    subtitleLine
                        .allowsHitTesting(false)
                }
            }
            titleLine
                .allowsHitTesting(false)
        }
    }

    /// A step below the title, not level with it. The two ran at
    /// `textPrimary` and `textSecondary`, which is a small enough gap
    /// that at a glance the pair read as one block of text rather than
    /// as a name and its artist. Under a scratch this line is the parked
    /// tune and where it is, so the DJ can see it is still there.
    private var subtitleLine: some View {
        let outward = state.side == .a
        return Text(subtitle)
            .font(DubFont.body)
            .foregroundStyle(DubColor.textTertiary)
            .lineLimit(1)
            .frame(maxWidth: .infinity, alignment: outward ? .leading : .trailing)
    }

    /// Two title lines are reserved whether the title needs one or two,
    /// so the block is the same height with "No track loaded", a short
    /// name and a long one — the times, the overview and every row
    /// below used to shift by a line on load. The SCRATCH badge sits on
    /// the title's baseline for the same reason: a third line while a
    /// scratch is on would move the column under the DJ's hand.
    private var titleLine: some View {
        let outward = state.side == .a
        let tint = DubColor.deckTint(state.side)
        let scratching = state.header.scratch != nil
        return HStack(alignment: .firstTextBaseline, spacing: DubSpacing.sm) {
            // A quick scratch in progress is a latched state on stage,
            // so the identity block says so in the deck's own colour
            // rather than quietly showing a sample's name as a title.
            // Outer side on both decks, like the title.
            if scratching && outward { scratchBadge(tint) }
            // A THRU deck is not empty: the record on the platter is the
            // source, it just has no name (rig, 2026-11-14).
            Text(state.header.trackTitle ?? emptyTitle)
                .font(DubFont.title)
                .foregroundStyle(
                    state.header.trackTitle == nil
                        ? (state.isThru && state.thruHasRecord
                            ? DubColor.textSecondary : DubColor.textPlaceholder)
                        : scratching ? tint : DubColor.textPrimary)
                .lineLimit(2, reservesSpace: true)
                .multilineTextAlignment(outward ? .leading : .trailing)
            if scratching && !outward { scratchBadge(tint) }
        }
        .frame(maxWidth: .infinity, alignment: outward ? .leading : .trailing)
    }

    /// What the title line says with no track: a THRU deck is not empty —
    /// the record on the platter is the source, it just has no name (rig,
    /// 2026-11-14).
    private var emptyTitle: String {
        guard state.isThru else { return "No track loaded" }
        return state.thruHasRecord ? "Live record" : "Waiting for the needle"
    }

    private func scratchBadge(_ tint: Color) -> some View {
        Text("SCRATCH")
            .font(DubFont.caps)
            .tracking(DubFont.capsTracking)
            .foregroundStyle(tint)
            .padding(.horizontal, DubSpacing.xs)
            .padding(.vertical, 1)
            .overlay(
                RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous)
                    .stroke(tint.opacity(0.6), lineWidth: 1))
    }

    private var subtitle: String {
        guard let scratch = state.header.scratch else {
            if state.isThru, state.header.trackArtist == nil {
                return state.thruInput.map { "THRU · \($0)" } ?? "THRU"
            }
            return state.header.trackArtist ?? "—"
        }
        guard let parked = scratch.parkedTitle else { return "← empty deck" }
        let secs = scratch.parkedSecs
        let time = "\(secs / 60):\(String(format: "%02d", secs % 60))"
        if let on = scratch.playingOn {
            return "← \(parked) · playing on \(on == .a ? "A" : "B") · \(time)"
        }
        return "← \(parked) · \(time)"
    }

    private var readouts: some View {
        DeckReadouts(
            header: state.header,
            trailing: state.side == .a,
            side: state.side,
            live: liveEngine.flatMap { engine in
                liveDeckIdx.map { DeckReadouts.Live(engine: engine, deckIdx: $0) }
            },
            onToggleKeyLock: state.isThru ? nil : callbacks.onToggleKeyLock,
            onToggleSteady: state.isThru ? nil : callbacks.onToggleSteady,
            onTapTempo: state.header.thruTempo == nil ? nil : callbacks.onTapTempo,
            onScaleTempo: callbacks.onScaleTempo,
            onResetTempo: callbacks.onResetTempo,
            track: state.header.thruTempo == nil
                ? DeckReadouts.TrackTap(
                    onTap: callbacks.onTapTempo,
                    onDouble: callbacks.onDoubleBpm,
                    onHalve: callbacks.onHalveBpm,
                    onReset: callbacks.onResetBpm,
                    onToggleLock: callbacks.onToggleGridLocked,
                    onSetFollow: callbacks.onSetGridFollow)
                : nil,
            tapSession: tapSession ?? TapSessionViewModel())
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
            side: state.side,
            columns: 4,
            rowHeight: DubLayout.cueRowHeight,
            cuesFrom: state.cuesFrom)
    }

    /// One row, three controls at one height: SCRATCH · LOOP · ECHO. All
    /// three are fired during a transition, so the hand stays on one
    /// line for the whole move. The scratch pads take every point the
    /// other two leave — a sample's name is the one string on the row
    /// whose length is not ours to choose — while the loop and the echo
    /// sit at their own widths; the three floors together clear the
    /// column's floor exactly (`test_deckColumn_floorHoldsTheTriggerRow`).
    ///
    /// The loop had the row to itself with the echo, at 34 pt steppers
    /// and 40 pt lengths and every point of slack; the DJ asked for it
    /// smaller again so the scratch pads could join the row, and then
    /// for the pads to have the slack instead.
    private var triggerRow: some View {
        HStack(alignment: .top, spacing: DubSpacing.md) {
            VStack(alignment: .leading, spacing: DubSpacing.sm) {
                SectionHeading(
                    title: "SCRATCH", accent: DubColor.deckTint(state.side),
                    trailing: scratchEngaged ? "● ON DECK" : "○ OFF",
                    trailingAccent: scratchEngaged
                        ? DubColor.deckTint(state.side) : DubColor.textPlaceholder)
                scratchPads
            }
            .frame(maxWidth: .infinity)
            VStack(alignment: .leading, spacing: DubSpacing.sm) {
                SectionHeading(
                    title: "LOOP", accent: DubColor.loop,
                    trailing: state.loopEngaged ? "● ON" : "○ OFF",
                    trailingAccent: state.loopEngaged
                        ? DubColor.loop : DubColor.textPlaceholder)
                loop
            }
            .frame(width: LoopEngine.minWidth)
            echoSection
        }
    }

    /// ECHO OUT, and HOLD with the DUB FX set on. Shared by the track's
    /// trigger row and the THRU one.
    @ViewBuilder
    private var echoSection: some View {
            if state.echoEnabled {
                VStack(alignment: .leading, spacing: DubSpacing.sm) {
                    SectionHeading(
                        title: "ECHO", accent: DubColor.controlAccent,
                        trailing: state.echoHeld ? "● HOLD" : state.echoEngaged ? "● ON" : "○ OFF",
                        trailingAccent: state.echoHeld
                            ? DubColor.echo
                            : state.echoEngaged ? DubColor.controlAccent : DubColor.textPlaceholder)
                    // HOLD belongs to the one-turntable dub set the DUB FX
                    // channel is for; without that opt-in the echo is the
                    // one ECHO OUT it always was (Daniel, 2026-10-06).
                    if state.fxAvailable {
                        HStack(spacing: DubSpacing.xs) {
                            echoButton
                            holdButton
                        }
                    } else {
                        echoButton
                    }
                }
                .frame(width: DubLayout.deckColumnEchoWidth)
            }
    }

    private var scratchEngaged: Bool { state.scratch.contains { $0.engaged } }

    /// Pinned to `LoopEngine.minWidth`: the compact ladder the DJ asked
    /// for, at every width. The slack on the row is the scratch pads'.
    private var loop: some View {
        LoopEngine(
            activeBeats: state.activeLoopBeats,
            engaged: state.loopEngaged,
            hasTrack: state.hasTrack,
            height: DubLayout.deckColumnLoopHeight,
            side: state.side,
            onLoop: callbacks.onLoop,
            onScale: callbacks.onScaleLoop,
            onExit: callbacks.onExitLoop)
    }

    /// PRD §7.2: four Quick Scratch pads. Press one and its sample is on
    /// the deck, under the needle; press it again and the tune comes
    /// back; press another while engaged and the sample swaps, the park
    /// untouched.
    ///
    /// Flat pads, in the family of the SAMPLES and DUB SIREN pads
    /// (canvas *Dub Quick Scratch*, 2026-11-14): each shows its sample's
    /// own shape — the one its SAMPLES pad draws — so the DJ sees which
    /// sound they are reaching for, and its name. The one on the deck is
    /// lit in the deck's tint and draws the needle across that shape:
    /// played in the tint, still to come in white. They were four
    /// spinning records, which read as a fourth kind of control on a row
    /// of pads. Empty pads are a dashed outline with their number.
    private var scratchPads: some View {
        HStack(spacing: DubSpacing.xs) {
            ForEach(state.scratch) { pad in
                scratchPad(pad)
            }
        }
    }

    @ViewBuilder
    private func scratchPad(_ pad: ScratchPadState) -> some View {
        let tint = DubColor.deckTint(state.side)
        let bound = pad.name != nil
        VStack(spacing: 4) {
            if let name = pad.name {
                if let fault = pad.fault {
                    SampleFaultBadge(fault: fault, size: 7)
                } else if !pad.wave.isEmpty {
                    scratchShape(pad, tint: tint)
                        .frame(height: 12)
                        .padding(.horizontal, 6)
                }
                Text(name)
                    .font(.system(size: 9, weight: .bold))
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .foregroundStyle(
                        pad.engaged ? DubColor.textPrimary
                            : pad.fault != nil ? DubColor.textTertiary : DubColor.textSecondary)
                    .padding(.horizontal, 3)
            } else {
                Text("QS\(pad.pad + 1)")
                    .font(.system(size: 9, weight: .medium, design: .monospaced))
                    .foregroundStyle(DubColor.textPlaceholder)
            }
        }
        .frame(minWidth: DubLayout.deckColumnScratchPadMinWidth, maxWidth: .infinity)
        .frame(height: DubLayout.deckColumnEchoHeight)
        .background(
            RoundedRectangle(cornerRadius: 4, style: .continuous)
                .fill(pad.engaged ? tint.opacity(0.18) : bound ? DubColor.surface3 : Color.clear))
        .overlay(
            RoundedRectangle(cornerRadius: 4, style: .continuous)
                .strokeBorder(
                    pad.engaged ? tint
                        : pad.fault != nil ? DubColor.stateError.opacity(0.6) : DubColor.divider,
                    style: StrokeStyle(lineWidth: 1, dash: bound ? [] : [3, 3])))
        .overlay(alignment: .topTrailing) {
            if let legend = pad.legend, bound, mapping == nil {
                DubKeycap(key: legend, lit: pad.engaged)
            } else if bound, pad.fault == nil {
                // A faulted pad's badge takes the top line; the number
                // would crowd it, and the pad's place already says it.
                Text("QS\(pad.pad + 1)")
                    .font(.system(size: 7, weight: .bold, design: .monospaced))
                    .foregroundStyle(pad.engaged ? tint : DubColor.textTertiary)
                    .padding(.top, 2)
                    .padding(.trailing, 4)
            }
        }
        .contentShape(Rectangle())
        .onPressDown(enabled: bound) { callbacks.onScratch(pad.pad) }
        // Only while it is off the deck: a right-click mid-scratch is a
        // hand that slipped, not a request for a dialog.
        .onSecondaryClick {
            guard bound, !pad.engaged else { return .handled }
            return .menu([
                SecondaryMenuItem("Rename…") { callbacks.onRenameScratch(pad.pad) }
            ])
        }
        .help(scratchHelp(pad))
        .mappable(.quickScratch(state.side, pad.pad))
    }

    /// The pad's sample shape. On the deck the needle is drawn across it,
    /// read off the engine's lock-free position snapshot every frame — the
    /// source the playhead uses — so it follows a scratch back and forth
    /// and stands still on a paused deck. Without an engine (snapshots,
    /// previews) it stands at the start.
    @ViewBuilder
    private func scratchShape(_ pad: ScratchPadState, tint: Color) -> some View {
        if pad.engaged, let liveEngine, let liveDeckIdx {
            TimelineView(.animation) { _ in
                let p = liveEngine.positionSnapshot(deckIdx: liveDeckIdx)
                ScratchShape(
                    bars: pad.wave, played: p.durationSecs > 0 ? p.elapsedSecs / p.durationSecs : 0,
                    tint: tint, onDeck: true)
            }
        } else {
            ScratchShape(bars: pad.wave, played: 0, tint: tint, onDeck: pad.engaged)
        }
    }

    private func scratchHelp(_ pad: ScratchPadState) -> String {
        guard let name = pad.name else {
            return "Quick Scratch \(pad.pad + 1) — tag a sample with it: right-click a "
                + "SAMPLES tile"
        }
        if let fault = pad.fault {
            return "Quick Scratch \(pad.pad + 1): \(fault.sentence)"
        }
        return pad.engaged
            ? "\(name) is on the deck — press to bring the tune back"
            : "Put \(name) on the deck under the needle; the tune parks and comes back "
                + "when you press again"
    }

    private var echoButton: some View {
        Text(state.fxAvailable ? "OUT" : "ECHO OUT")
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
            .overlay(alignment: .topTrailing) {
                if let legend = state.echoLegend, mapping == nil {
                    DubKeycap(key: legend, lit: state.echoEngaged)
                        .padding(3)
                }
            }
            .contentShape(Rectangle())
            .onPressDown(enabled: state.hasTrack || state.isThru) { callbacks.onEchoToggle() }
            .help("Echo out — cut the dry signal and hand the deck to the echo")
            .mappable(.echoOut(state.side))
    }

    /// HOLD — a toggle, live only while the echo is on. The thrown beat
    /// sustains just under endless, so it carries the room while the
    /// needle comes up and the next record goes down; HOLD again lets it
    /// decay, OUT cuts it. Violet, the echo's own colour, not the amber
    /// every "on" on the surface wears: a different state of the same
    /// sound, and the one you must not forget is on.
    private var holdButton: some View {
        let live = state.echoEngaged
        return Text("HOLD")
            .font(DubFont.caps)
            .tracking(DubFont.capsTracking)
            .foregroundStyle(
                state.echoHeld ? DubColor.textPrimary
                    : live ? DubColor.textSecondary : DubColor.textPlaceholder)
            .frame(maxWidth: .infinity)
            .frame(height: DubLayout.deckColumnEchoHeight)
            .background(
                state.echoHeld ? DubColor.echo.opacity(0.26) : DubColor.surface2)
            .clipShape(RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous)
                    .stroke(state.echoHeld ? DubColor.echo : DubColor.divider, lineWidth: 1))
            .overlay(alignment: .topTrailing) {
                if let legend = state.echoHoldLegend, mapping == nil {
                    DubKeycap(key: legend, lit: state.echoHeld)
                        .padding(3)
                }
            }
            .contentShape(Rectangle())
            .onPressDown(enabled: live) { callbacks.onEchoHold() }
            .help("Hold the echo through a record swap — tap again to let it go")
            .mappable(.echoHold(state.side))
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
    /// Which deck's key lock the LOCK button is, for key mapping.
    /// Defaults to A so the layout tests, which render the row on its
    /// own, keep their one-argument call.
    var side: DeckSide = .a
    /// The engine, when there is one: BPM and PITCH are then read off
    /// it on layers (`LayerReadout`) rather than rebuilt as `Text` on
    /// every tenth the platter moves. Previews and snapshots have none
    /// and draw the header's values.
    var live: Live? = nil
    /// M14 key lock. `nil` leaves the button out entirely.
    var onToggleKeyLock: (() -> Void)? = nil
    /// Steadying. Drawn only on a grid that follows its band.
    var onToggleSteady: (() -> Void)? = nil
    /// Tap tempo on a THRU deck's BPM. Only with `header.thruTempo`.
    var onTapTempo: (() -> Void)? = nil
    /// The THRU BPM's right-click: scale the tempo, or go back to the
    /// heard one.
    var onScaleTempo: ((Double) -> Void)? = nil
    var onResetTempo: (() -> Void)? = nil
    /// The track BPM as a tap pad, as in Prep's header (Daniel,
    /// 2026-11-16: hitting the BPM sets the 1 in Performance too). `nil`
    /// leaves it a plain readout — the layout tests, and THRU.
    var track: TrackTap? = nil
    @ObservedObject var tapSession = TapSessionViewModel()

    struct TrackTap {
        var onTap: () -> Void
        var onDouble: () -> Void
        var onHalve: () -> Void
        var onReset: () -> Void
        var onToggleLock: () -> Void
        var onSetFollow: (Bool?) -> Void
    }

    struct Live {
        let engine: DubEngine
        let deckIdx: UInt64

        /// `nil` unless the deck is playing — a paused deck publishes
        /// rate 0, which is not a pitch.
        func pitch() -> Double? {
            guard engine.positionSnapshot(deckIdx: deckIdx).isPlaying else { return nil }
            return engine.deckTelemetry(deckIdx: deckIdx).pitchPercent
        }
    }

    var body: some View {
        HStack(alignment: .top, spacing: DubSpacing.md) {
            if let thru = header.thruTempo {
                // A live record has no key and plays at the speed it is
                // heard at: the tempo is the only number it has.
                thruBpm(thru)
            } else {
                // Each switch sits against the number it holds, outside
                // the BPM's tap pad: a press meant for STEADY must not
                // set the 1.
                HStack(alignment: .top, spacing: DubSpacing.xs) {
                    if let track {
                        trackBpm(track)
                    } else {
                        // Not a control: a press here is the header's
                        // drag for an instant double.
                        trackBpmReadout
                            .allowsHitTesting(false)
                    }
                    steadyButton
                }
            }
            if header.thruTempo == nil {
                keyGroup
                // Six places: the sign, three digits, the point and a tenth.
                // A scratch drives the smoothed rate well past ±100 %, and
                // the slot has to hold that too or the row jumps mid-cut.
                readout(
                    "PITCH", header.pitchTenths.map { PitchText.signed($0) },
                    font: DubFont.numericInline, widest: "-888.8",
                    live: live.map { l in { l.pitch().map { PitchText.signed($0) } } },
                    size: DubFont.numericInlineSize)
                    .allowsHitTesting(false)
            }
        }
        .fixedSize()
    }

    /// The key with its lock on the far side from the row's edge — after
    /// the key on deck B, before it on deck A — so the row is one mirror:
    /// KEY hangs over its key as BPM and PITCH do over theirs, the key's
    /// slot takes up the slack on the lock's side, and the gap to the lock
    /// is the same on both decks (Daniel, 2026-11-26). On deck A the lock
    /// came after the key too, which hugged it and left KEY over the key
    /// alone at the group's middle.
    private var keyGroup: some View {
        HStack(alignment: .top, spacing: DubSpacing.xs) {
            if trailing { keyLockButton }
            readout(
                "KEY", header.key,
                font: DubFont.numericInline, widest: "12B",
                tint: DubColor.camelotKey(header.key))
                .allowsHitTesting(false)
            if !trailing { keyLockButton }
        }
    }

    private var trackBpmReadout: some View {
        readout(
            bpmLabelText, header.bpm.map { String(format: "%.1f", $0) },
            font: DubFont.numericLarge, widest: "888.8",
            tint: header.gridMoves ? DubColor.gridMoving : nil,
            live: live.map { l in
                { [base = header.baseBpm, tempo = header.tempoPitchPercent] in
                    DeckHeaderState.liveBpm(base: base, pitch: l.pitch(), tempoPitch: tempo)
                        .map { String(format: "%.1f", $0) }
                }
            },
            size: DubFont.numericLargeSize,
            labelTint: tapSession.tapCount > 0
                ? DubColor.deckTint(side) : (header.gridMoves ? DubColor.gridMoving : nil),
            labelLock: header.gridLocked,
            lockTint: header.gridMoves ? DubColor.gridMoving : nil)
    }

    /// The label counts an open tap run, `BPM (2)` — the signal Prep's
    /// header gives, on the label because the number is drawn on a layer.
    private var bpmLabelText: String {
        tapSession.tapCount > 0 ? header.bpmLabel + " (\(tapSession.tapCount))" : header.bpmLabel
    }

    /// A loaded track's BPM, tapped like Prep's: 1–2 taps on the beat set
    /// the 1, 3+ re-take the tempo. Fires on mouse-down — the tap is the
    /// moment, and a button's mouse-up lands it late. A locked grid takes
    /// no taps, but its right-click still reaches Unlock grid.
    private func trackBpm(_ t: TrackTap) -> some View {
        trackBpmReadout
            .contentShape(Rectangle())
            .onPressDown(enabled: header.bpm != nil && !header.gridLocked) { t.onTap() }
            .contextMenu {
                if !header.gridLocked, header.bpm != nil {
                    Button("2×", action: t.onDouble)
                    Button("½", action: t.onHalve)
                    Divider()
                    Button("Reset to auto", action: t.onReset)
                    Divider()
                }
                Button(header.gridLocked ? "Unlock grid" : "Lock grid", action: t.onToggleLock)
                if header.followChoiceOffered, !header.gridLocked {
                    GridFollowMenu(word: header.gridFollowWord, onSet: t.onSetFollow)
                }
            }
            .help(header.gridLocked
                  ? "Grid is locked. Right-click to unlock before tapping."
                  : "Tap on the 1 to set the downbeat (1–2 taps), or tap the beat to re-take "
                    + "the tempo (3+). Right-click for 2× / ½ / Reset / Lock.")
            .mappable(.tapTempo(side))
    }

    /// A THRU deck's BPM: the record's tempo as the tracker hears it —
    /// dim while tentative, solid once locked — or as the DJ tapped it,
    /// marked TAP. The readout is its own tap pad: the number being set
    /// is the thing under the finger. No pitch scaling: the record is
    /// measured at the speed it plays.
    private func thruBpm(_ t: ThruTempoReadout) -> some View {
        VStack(alignment: trailing ? .trailing : .leading, spacing: 2) {
            Text(t.label)
                .font(DubFont.micro)
                .tracking(DubFont.capsTracking)
                .foregroundStyle(t.overridden ? DubColor.deckTint(side) : DubColor.textTertiary)
            staticValue(
                t.bpm.map { String(format: "%.1f", $0) },
                font: DubFont.numericLarge, widest: "888.8",
                tint: t.tapped || t.locked ? nil : DubColor.textSecondary)
        }
        .contentShape(Rectangle())
        .onPressDown(enabled: onTapTempo != nil) { onTapTempo?() }
        // The tracker's usual mistake is an octave — or, on drum & bass,
        // two-thirds — so the fix is one right-click, not four taps
        // (Daniel, 2026-11-14).
        .contextMenu {
            Button("Double — ×2") { onScaleTempo?(2) }
            Button("Half — ÷2") { onScaleTempo?(0.5) }
            Button("×1.5 — drum & bass read at two-thirds") { onScaleTempo?(1.5) }
            Button("×⅔") { onScaleTempo?(2.0 / 3) }
            Divider()
            Button("Back to the heard tempo") { onResetTempo?() }
                .disabled(!t.overridden)
        }
        .help("The record's tempo, heard live. Tap the beat here to set it, or right-click to "
            + "double, halve or take ×1.5 — either holds until the needle comes up.")
        .accessibilityLabel("Tempo")
        .accessibilityValue(t.bpm.map { String(format: "%.1f BPM", $0) } ?? "listening")
        .mappable(.tapTempo(side))
    }

    /// M14 key lock: a note beside the key it holds — an icon, not the
    /// labelled ON / OFF box it was, which took a readout's column for a
    /// switch (Daniel, 2026-11-21). A note and not a padlock: the padlock
    /// in this row is the locked grid's, on the BPM label.
    ///
    /// **Two states, not one.** The button lights from `keyLockOn` (the
    /// DJ's switch, immediate on tap); how strongly reports what the
    /// engine is actually doing — dim while the platter sits at unity or
    /// a loop has it bypassed, full once the stretcher is holding a
    /// pitch. A single lit state would claim work that isn't happening
    /// at +0.0 %.
    @ViewBuilder
    private var keyLockButton: some View {
        if let toggle = onToggleKeyLock {
            let on = header.keyLockOn
            iconSwitch(
                "music.note", on: on, working: header.keyLockState == 2,
                beside: DubFont.numericInline, action: toggle)
            .help(on
                  ? "Key lock on — the pitch holds while the platter moves the tempo"
                  : "Key lock off — pitch rides the platter, like a record")
            .accessibilityLabel("Key lock")
            .accessibilityValue(on ? "on" : "off")
            .mappable(.keyLock(side))
        }
    }

    /// Steadying (Alpha), a metronome beside the tempo it holds: keep a
    /// record whose grid follows its band at one tempo while a second
    /// record is brought in, and let the band play as it played
    /// otherwise. Only on such a record — one with an even grid has one
    /// tempo already, and a switch that does nothing is a switch too many.
    /// The BPM label says the rest: `BPM · FOLLOW`, `BPM · STEADY`.
    ///
    /// Lit from the DJ's switch; full while the engine is holding the
    /// tempo and dim while it is not for now (a scratch, a stretch
    /// without beats), as the key lock's is.
    @ViewBuilder
    private var steadyButton: some View {
        if let toggle = onToggleSteady, header.gridMoves || header.steadyState != 0 {
            let on = header.steadyState != 0
            iconSwitch(
                "metronome", on: on, working: header.steadyState == 2,
                beside: DubFont.numericLarge, action: toggle)
            .help(on
                  ? "Steady on — the record is held to its tempo; turn it off to let the band play as it played"
                  : "Steady off — the band plays as it played. Turn it on to hold one tempo while you mix")
            .accessibilityLabel("Steady")
            .accessibilityValue(on ? "on" : "off")
        }
    }

    /// A switch drawn as its icon, sat on the value row of the readout it
    /// belongs to: `beside` is that value's font, and the hidden label and
    /// digit give the button the readout's own two rows to centre on.
    ///
    /// The control accent, not the deck's tint: a lit control means "on"
    /// on either deck (`DubColor.controlAccent`). In the deck's tint the
    /// right deck's LOCK lit teal beside amber controls (Daniel,
    /// 2026-11-19).
    private func iconSwitch(
        _ symbol: String, on: Bool, working: Bool, beside font: Font, action: @escaping () -> Void
    ) -> some View {
        let tint = DubColor.controlAccent
        return VStack(spacing: 2) {
            Text(" ")
                .font(DubFont.micro)
                .hidden()
            Text("8")
                .font(font)
                .hidden()
                .frame(width: 22)
                .overlay {
                    Image(systemName: symbol)
                        .font(.system(size: 11, weight: .semibold))
                        .foregroundStyle(on ? tint.opacity(working ? 1.0 : 0.55) : DubColor.textTertiary)
                        .frame(width: 22, height: 20)
                        .background(on ? tint.opacity(0.16) : DubColor.surface2)
                        .clipShape(RoundedRectangle(cornerRadius: 4, style: .continuous))
                        .overlay(
                            RoundedRectangle(cornerRadius: 4, style: .continuous)
                                .stroke(on ? tint.opacity(0.55) : DubColor.divider, lineWidth: 1))
                }
        }
        // The whole two rows take the press, not the drawn 22 × 20.
        .contentShape(Rectangle())
        .onPressDown(perform: action)
        .accessibilityAddTraits(.isButton)
    }

    /// `widest` is drawn hidden underneath the value and sets the
    /// slot's width — measured, so the reservation follows the font
    /// rather than a hand-typed point count.
    private func readout(
        _ label: String, _ value: String?, font: Font, widest: String, tint: Color? = nil,
        live: (() -> String?)? = nil, size: CGFloat = 0,
        labelTint: Color? = nil, labelLock: Bool = false, lockTint: Color? = nil
    ) -> some View {
        VStack(alignment: trailing ? .trailing : .leading, spacing: 2) {
            readoutLabel(label, tint: labelTint, lock: labelLock, lockTint: lockTint)
            if let live {
                Text(widest)
                    .font(font)
                    .monospacedDigit()
                    .hidden()
                    .overlay {
                        LayerReadout(
                            read: live, size: size,
                            color: tint ?? DubColor.textPrimary,
                            placeholder: DubColor.textPlaceholder,
                            trailing: trailing)
                    }
            } else {
                staticValue(value, font: font, widest: widest, tint: tint)
            }
        }
    }

    private func readoutLabel(
        _ label: String, tint: Color? = nil, lock: Bool = false, lockTint: Color? = nil
    ) -> some View {
        HStack(spacing: 3) {
            Text(label)
                .font(DubFont.micro)
                .tracking(DubFont.capsTracking)
                .foregroundStyle(tint ?? DubColor.textTertiary)
            if lock {
                Image(systemName: "lock.fill")
                    .font(.system(size: 7))
                    .foregroundStyle(lockTint ?? DubColor.textTertiary)
                    .accessibilityLabel(lockTint == nil ? "Grid locked" : "Moving grid, accepted")
            }
        }
    }

    private func staticValue(_ value: String?, font: Font, widest: String, tint: Color?) -> some View {
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

extension View {
    /// `onDrag` that can be switched off: an empty deck has nothing to
    /// double, and a modifier that returns an empty provider still
    /// starts a drag image of the block.
    @ViewBuilder
    func onDrag(if enabled: Bool, _ data: @escaping () -> NSItemProvider) -> some View {
        if enabled {
            onDrag(data)
        } else {
            self
        }
    }

    /// The same, with what the drag shows under the pointer.
    @ViewBuilder
    func onDrag<Preview: View>(
        if enabled: Bool, _ data: @escaping () -> NSItemProvider,
        @ViewBuilder preview: () -> Preview
    ) -> some View {
        if enabled {
            onDrag(data, preview: preview)
        } else {
            self
        }
    }
}

/// A Quick Scratch pad's sample shape: one bar per slice, mirrored about
/// the middle. On the deck the bars the needle has passed take the deck's
/// tint and the rest are white; off it they are grey.
struct ScratchShape: View {
    let bars: [Float]
    /// How far the needle is through the sample, `0...1`.
    var played: Double = 0
    let tint: Color
    var onDeck: Bool = false

    var body: some View {
        Canvas { ctx, size in
            guard !bars.isEmpty else { return }
            let step = size.width / CGFloat(bars.count)
            let needle = min(max(played, 0), 1)
            for (i, a) in bars.enumerated() {
                let h = max(1.5, CGFloat(min(max(a, 0), 1)) * size.height)
                let rect = CGRect(
                    x: CGFloat(i) * step, y: (size.height - h) / 2,
                    width: max(1, step - 0.8), height: h)
                let ink: Color
                if onDeck {
                    ink = Double(i) / Double(bars.count) < needle ? tint : DubColor.textPrimary
                } else {
                    ink = DubColor.textSecondary
                }
                ctx.fill(Path(rect), with: .color(ink))
            }
        }
        .accessibilityHidden(true)
    }
}
