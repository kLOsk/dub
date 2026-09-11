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
    /// The four Quick Scratch pads (PRD §7.2), in pad order. An untagged
    /// pad is drawn dark and dead so the hand's positions never move.
    var scratch: [ScratchPadState] = (0..<SampleBank.quickScratchCount).map {
        ScratchPadState(pad: $0)
    }
}

/// One Quick Scratch pad as the column draws it.
struct ScratchPadState: Equatable, Identifiable {
    var pad: Int
    /// The tagged sample's name; `nil` when no sample answers to the pad.
    var name: String?
    /// `true` while this pad's sample is on the deck.
    var engaged: Bool = false

    var id: Int { pad }
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
    /// Set while Quick Scratch has a sample on the deck: what is parked
    /// underneath and where it is. The identity block then shouts.
    var scratch: ScratchBadge?
    /// The master deck (PRD §6.4) — the one the crowd is hearing, the
    /// one the siren and sampler land on, the one Space-load avoids.
    var isMaster: Bool

    init(_ state: DeckHeaderState, scratch: ScratchBadge? = nil) {
        trackTitle = state.trackTitle
        trackArtist = state.trackArtist
        bpm = state.bpm
        key = state.key
        pitchTenths = state.pitchPercent.map { ($0 * 10).rounded() / 10 }
        sourceControl = state.sourceControl
        sourceControlOverridden = state.sourceControlOverridden
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
    /// Quick Scratch pad `pad` pressed: engage its sample, swap to it,
    /// or — if it is the one on the deck — release.
    var onScratch: (_ pad: Int) -> Void = { _ in }
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
            triggerRow
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

    /// Two title lines are reserved whether the title needs one or two,
    /// so the block is the same height with "No track loaded", a short
    /// name and a long one — the times, the overview and every row
    /// below used to shift by a line on load. The SCRATCH badge sits on
    /// the title's baseline for the same reason: a third line while a
    /// scratch is on would move the column under the DJ's hand.
    private var identity: some View {
        let outward = state.side == .a
        let tint = DubColor.deckTint(state.side)
        let scratching = state.header.scratch != nil
        return VStack(alignment: outward ? .leading : .trailing, spacing: 2) {
            HStack(alignment: .firstTextBaseline, spacing: DubSpacing.sm) {
                // A quick scratch in progress is a latched state on
                // stage, so the identity block says so in the deck's own
                // colour rather than quietly showing a sample's name as
                // a title. Outer side on both decks, like the title.
                if scratching && outward { scratchBadge(tint) }
                Text(state.header.trackTitle ?? "No track loaded")
                    .font(DubFont.title)
                    .foregroundStyle(
                        state.header.trackTitle == nil
                            ? DubColor.textPlaceholder
                            : scratching ? tint : DubColor.textPrimary)
                    .lineLimit(2, reservesSpace: true)
                    .multilineTextAlignment(outward ? .leading : .trailing)
                if scratching && !outward { scratchBadge(tint) }
            }
            // A step below the title, not level with it. The two ran
            // at `textPrimary` and `textSecondary`, which is a small
            // enough gap that at a glance the pair read as one block of
            // text rather than as a name and its artist. Under a
            // scratch this line is the parked tune and where it is,
            // so the DJ can see it is still there.
            Text(subtitle)
                .font(DubFont.body)
                .foregroundStyle(DubColor.textTertiary)
                .lineLimit(1)
        }
        .frame(maxWidth: .infinity, alignment: outward ? .leading : .trailing)
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

    private var scratchEngaged: Bool { state.scratch.contains { $0.engaged } }

    /// Pinned to `LoopEngine.minWidth`: the compact ladder the DJ asked
    /// for, at every width. The slack on the row is the scratch pads'.
    private var loop: some View {
        LoopEngine(
            activeBeats: state.activeLoopBeats,
            engaged: state.loopEngaged,
            hasTrack: state.hasTrack,
            height: DubLayout.deckColumnLoopHeight,
            onLoop: callbacks.onLoop,
            onScale: callbacks.onScaleLoop,
            onExit: callbacks.onExitLoop)
    }

    /// PRD §7.2: four Quick Scratch pads, drawn as what they are — four
    /// records. Press one and it is on the deck, under the needle; press
    /// it again and the tune comes back; press another while engaged and
    /// the record swaps, the park untouched. Unboxed on purpose: every
    /// neighbour on this row is a rounded rectangle, and the eye finds
    /// the records by silhouette alone. The one on the deck is a third
    /// bigger, its label in the deck's tint, and its sticker turns with
    /// the platter (`DubRecordGlyph`). Empty pads are a dashed outline
    /// where a record goes — the shelf's own empty convention.
    private var scratchPads: some View {
        HStack(spacing: DubSpacing.xs) {
            ForEach(state.scratch) { pad in
                scratchPad(pad)
            }
        }
    }

    /// Idle and on-deck record sizes. The slot they sit in is fixed so
    /// the name beneath does not move when one grows. (`DeckColumn` is
    /// generic over its overview, so these are computed, not stored.)
    private var recordIdle: CGFloat { 24 }
    private var recordOnDeck: CGFloat { 32 }
    private var recordSlot: CGFloat { 30 }

    @ViewBuilder
    private func scratchPad(_ pad: ScratchPadState) -> some View {
        let tint = DubColor.deckTint(state.side)
        let bound = pad.name != nil
        let look: DubRecordGlyph.Look = pad.engaged ? .onDeck : bound ? .shelved : .empty
        VStack(spacing: 3) {
            record(look: look, tint: tint)
                .frame(width: pad.engaged ? recordOnDeck : recordIdle,
                       height: pad.engaged ? recordOnDeck : recordIdle)
                .animation(.spring(response: 0.18, dampingFraction: 0.6), value: pad.engaged)
                .frame(height: recordSlot)
            Text(pad.name ?? "QS\(pad.pad + 1)")
                .font(bound
                    ? .system(size: 10, weight: .semibold, design: .rounded)
                    : .system(size: 9, weight: .medium, design: .monospaced))
                .lineLimit(1)
                .minimumScaleFactor(0.6)
                .foregroundStyle(
                    pad.engaged ? DubColor.textPrimary
                        : bound ? DubColor.textSecondary : DubColor.textPlaceholder)
        }
        .padding(.horizontal, 2)
        // Flexible from the floor up: the four pads share the row's
        // slack equally, and at the column's floor they sit at
        // `deckColumnScratchPadMinWidth` so the three blocks sum to the
        // inner width exactly.
        .frame(minWidth: DubLayout.deckColumnScratchPadMinWidth, maxWidth: .infinity)
        .frame(height: DubLayout.deckColumnEchoHeight)
        .contentShape(Rectangle())
        .onPressDown(enabled: bound) { callbacks.onScratch(pad.pad) }
        .help(scratchHelp(pad))
    }

    /// The record, turning with the platter while it is on the deck.
    /// The angle is read off the engine's lock-free position snapshot
    /// every frame — the same source the playhead uses — so it follows a
    /// scratch back and forth and stands still on a paused deck. Without
    /// an engine (snapshots, previews) it sits at twelve o'clock.
    @ViewBuilder
    private func record(look: DubRecordGlyph.Look, tint: Color) -> some View {
        if look == .onDeck, let liveEngine, let liveDeckIdx {
            TimelineView(.animation) { _ in
                DubRecordGlyph(
                    look: look, tint: tint,
                    angle: DubRecordGlyph.angle(
                        forElapsedSecs: liveEngine.positionSnapshot(deckIdx: liveDeckIdx).elapsedSecs))
            }
        } else {
            DubRecordGlyph(look: look, tint: tint)
        }
    }

    private func scratchHelp(_ pad: ScratchPadState) -> String {
        guard let name = pad.name else {
            return "Quick Scratch \(pad.pad + 1) — tag a sample with it: right-click a "
                + "SAMPLES tile"
        }
        return pad.engaged
            ? "\(name) is on the deck — press to bring the tune back"
            : "Put \(name) on the deck under the needle; the tune parks and comes back "
                + "when you press again"
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
