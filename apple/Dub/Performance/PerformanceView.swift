//
//  PerformanceView.swift
//  Dub
//
//  Top-level performance layout per PRD §9.2.
//
//  Layout (top → bottom):
//
//      ┌─ status strip ──────────────────────────────────────────┐
//      │ DUB · 48.0 kHz · LIVE         21:47   🔋 87%             │
//      ├──────────────────────────────┬──────────────────────────┤
//      │ deck A header (3 rows in     │ deck B header            │
//      │ File mode — adds track-time) │                          │
//      ├──────────────────────────────┼──────────────────────────┤
//      │                              │                          │
//      │  Metal waveform A            │  Metal waveform B        │
//      │  (or idle pane if A          │  (or idle pane if B      │
//      │   offline)                   │   offline)               │
//      │                              │                          │
//      │   playhead at 25 % from top, deck-tinted hairline       │
//      │                                                         │
//      ├─ global rack bar: siren → A · quick scratch · sampler ──┤
//      ╞═ draggable divider ═════════════════════════════════════╡
//      ├─ library / FS browser (M10.5b) ─────────────────────────┤
//      └─────────────────────────────────────────────────────────┘
//
//  The rack bar holds the three racks that were never per-deck: one
//  siren keymap firing the focused deck, and two four-slot tables whose
//  slots each carry their own target deck. Prep omits it — its siren
//  has a section in `PrepRack` and its vertical budget is the
//  tightest on either surface.
//
//  M10.5b deck panes accept Finder-drop URLs onto each pane,
//  surface a 200 ms red overlay when a load fails because the target
//  deck is currently playing (PRD §5.5 + §6.4), and render the Metal
//  waveform whenever the deck is *either* live Thru *or* has a File
//  track loaded — not just when Thru is capturing.
//

import SwiftUI
import UniformTypeIdentifiers
import DubCore

/// Top-level performance surface. Driven by `WaveformAppModel`.
struct PerformanceView: View {

    @ObservedObject var model: WaveformAppModel
    /// Callback the status-strip gear button hits to open the
    /// Preferences sheet — owned by `MainView`, passed down so
    /// `PerformanceView` itself stays free of sheet bindings.
    let openPreferences: () -> Void
    /// Callback the status-strip wordmark hits to open the About
    /// sheet — also owned by `MainView`.
    let openAbout: () -> Void

    var body: some View {
        VStack(spacing: 0) {
            statusStrip
            deckHeaders
            Rectangle().fill(DubColor.divider).frame(height: 1)
            DeckLibrarySplit(
                mode: model.engineMode,
                deckChrome: deckChromeHeight,
                deckMinimum: model.engineMode == .prep
                    ? DubLayout.prepRegionMinHeight + prepRipLaneHeight
                    : DubLayout.waveformMinHeight
            ) { _ in
                VStack(spacing: 0) {
                    waveformRegion
                    // Prep gets no rack bar. Its vertical budget is the
                    // tightest on either surface, its siren has its own
                    // column, and the 100 pt this slot used to cost it
                    // went to "Coming soon" cards for two features that
                    // had already shipped.
                    if model.engineMode != .prep {
                        Rectangle().fill(DubColor.divider).frame(height: 1)
                        GlobalRackBar(
                            state: rackBarState, callbacks: rackBarCallbacks)
                    }
                }
            } library: {
                LibraryView(model: model, libraryModel: model.libraryModel)
            }
        }
        .background(DubColor.surface0)
    }

    // MARK: - Deck headers
    //
    // M11d.5: the header view tree gets the same `DeckDropTarget`
    // modifier the waveform pane uses, so dragging a file onto the
    // deck header lands the same load the user would get dropping
    // on the strip itself. Pre-fix the drop was on the 80 px
    // waveform column only, which the user reported as "I keep
    // missing the strip — drag should also accept on the header".

    /// **Prep only.** Performance's header band is gone — its contents
    /// moved into `DeckColumn`, beside each waveform instead of above
    /// both. That band was 108 pt of the vertical budget spent on two
    /// blocks of text and six numbers, on the surface where the strip's
    /// height is the whole point; see `DeckColumn`'s file comment.
    @ViewBuilder
    private var deckHeaders: some View {
        if model.engineMode == .prep {
            DeckHeader(side: .a,
                       state: headerState(side: .a),
                       callbacks: headerCallbacks(side: .a),
                       mirrored: false,
                       prepMode: true,
                       tapSession: model.tapSession(for: .a),
                       liveEngine: model.engine,
                       liveDeckIdx: 0)
                .background(DubColor.divider)
                .modifier(DeckDropTarget(model: model, side: .a))
        }
    }

    // MARK: - Status strip

    private var statusStrip: some View {
        StatusStripContainer(
            engineVersion: engineVersion(),
            sampleRate: model.engine.sampleRate(),
            isRunning: model.isRunning,
            lastError: model.lastError,
            modeSwitch: modeSwitchState,
            onSelectMode: { mode in
                model.setModeOverride(mode)
            },
            openPreferences: openPreferences,
            openAbout: openAbout)
    }

    /// M26a — the manual PREP / PERF switch appears only while the
    /// vinyl-recording feature is on and a DJ interface is present
    /// (auto-detect rules everything else). Disabled while a rip
    /// flow is live so a stray click can't tear the capture down —
    /// finish or discard the rip first.
    private var modeSwitchState: ModeSwitchState? {
        guard model.vinylRecordingEnabled,
              !model.performanceDevices.isEmpty
        else { return nil }
        return ModeSwitchState(
            mode: model.engineMode,
            isEnabled: model.ripPhase == .none)
    }

    // MARK: - Deck header derivation

    private func headerState(side: DeckSide) -> DeckHeaderState {
        let enabled: Bool
        switch side {
        case .a: enabled = deckAEnabled
        case .b: enabled = deckBEnabled
        }
        let deck = (side == .a) ? model.deckA : model.deckB
        return DeckHeaderState.from(
            side: side,
            deckState: deck,
            engineRunning: model.isRunning,
            deckEnabled: enabled,
            thruMode: model.engineMode == .timecode && !model.isInternalMixer,
            isMaster: model.masterDeck == side,
            prepMode: model.engineMode == .prep || model.isInternalMixer)
    }

    /// Deck headers are **never** mirrored. Real turntables aren't
    /// mirror images of each other, so neither are Dub's decks: both
    /// headers read identically left-to-right, so your eye always
    /// finds PITCH / BPM / KEY in the same place regardless of which
    /// deck you're looking at. (Pre-redesign, deck B was mirrored,
    /// which forced a left/right re-parse on every glance.)
    private func headerMirrored(side _: DeckSide) -> Bool {
        false
    }

    /// M10.6a Casual-Play transport callbacks for the deck header.
    /// Pure forwarders into the model — the header doesn't get a
    /// direct model reference, which keeps the view trivially
    /// snapshot-testable in M18.
    private func headerCallbacks(side: DeckSide) -> DeckHeaderCallbacks {
        DeckHeaderCallbacks(
            onPlay:    { model.play(side: side) },
            onPause:   { model.pause(side: side) },
            onPanicToggle: { model.panicToggle(side: side) },
            onTapBpm: {
                model.handleTapForGrid(side)
            },
            onDoubleBpm: {
                model.scaleLoadedDeckBpm(side: side, multiplier: 2.0)
            },
            onHalveBpm: {
                model.scaleLoadedDeckBpm(side: side, multiplier: 0.5)
            },
            onResetBpm: {
                model.resetLoadedDeckBeatGrid(side: side)
            },
            onToggleGridLocked: {
                model.toggleLoadedDeckGridLocked(side: side)
            },
            onSetInternal: { model.setDeckInternal(side: side) },
            onSetTimecode: { model.setDeckTimecode(side: side) },
            onSetThru: { model.setDeckThru(side: side) },
            onRecalibrate: { model.recalibrateDeck(side: side) },
            onHistoryHintTap: { model.revealHistoryHint(side: side) })
    }

    // MARK: - Performance pad column

    /// Snapshot of the model for one deck's pad column, mirroring
    /// `headerState(side:)`. Both deck panes used to spell this out as
    /// a twenty-argument initialiser, written twice.
    /// The deck column's values. `headerState` is reused verbatim so the
    /// identity block, the readouts and the source switch cannot drift
    /// from what the header showed before it moved.
    func deckColumnState(side: DeckSide) -> DeckColumnState {
        let deck = (side == .a) ? model.deckA : model.deckB
        return DeckColumnState(
            side: side,
            header: DeckColumnHeader(headerState(side: side)),
            cues: (0..<DeckState.hotCueCount).map {
                CueSlotState(index: $0, mark: deck.hotCues[$0])
            },
            activeLoopBeats: deck.activeLoopBeats,
            loopEngaged: deck.loopActive,
            echoEnabled: model.echoOutEnabled,
            echoEngaged: deck.echoDivision != nil,
            hasTrack: deck.hasTrack,
            isPlaying: deck.isPlaying)
    }

    /// Pure forwarders into the model, as with `headerCallbacks`. The
    /// source-control four are the header's own, so the switch keeps
    /// behaving exactly as it did in the band.
    func deckColumnCallbacks(side: DeckSide) -> DeckColumnCallbacks {
        let header = headerCallbacks(side: side)
        return DeckColumnCallbacks(
            onCue: { index, clear in
                model.handleHotCue(side, index: index, clear: clear)
            },
            onPreviewDown: { index in model.beginHotCuePreview(side, index: index) },
            onPreviewUp: { model.endHotCuePreview(side) },
            onRenameCue: { index in renameCue(index, side: side) },
            onColorCue: { index, token in
                let deck = (side == .a) ? model.deckA : model.deckB
                model.setHotCueLabel(
                    side, index: index,
                    name: deck.hotCues[index]?.name, color: token)
            },
            onLoop: { beats in model.handleLoopBeats(side, beats: beats) },
            onScaleLoop: { double in model.scaleLoop(side, double: double) },
            onExitLoop: { model.exitLoop(side) },
            onEchoToggle: { model.toggleEchoOut(side) },
            // Select Internal *and* start playing, which is the
            // contract `SourceControlView` documents for the segment
            // ("Select Internal and start playing the loaded file").
            // `setDeckInternal` only sets the control mode, so on its
            // own the button selected a mode and left the deck sitting
            // there — indistinguishable from nothing happening.
            onSetInternal: {
                header.onSetInternal?()
                model.play(side: side)
            },
            onPause: header.onPause,
            onSetTimecode: { header.onSetTimecode?() },
            onSetThru: { header.onSetThru?() },
            onRecalibrate: { header.onRecalibrate?() })
    }

    func padsState(side: DeckSide, deckState: DeckState) -> PerformancePadsState {
        PerformancePadsState(
            // Performance still draws cues as numbered pads; its own
            // redesign is open. Positions only until then.
            cues: deckState.hotCues.map { $0?.positionSecs },
            activeLoopBars: deckState.activeLoopBars,
            isPlaying: deckState.isPlaying,
            loopEngaged: deckState.loopActive,
            loopInArmed: deckState.pendingLoopInSecs != nil,
            echoEnabled: model.echoOutEnabled,
            echoEngaged: deckState.echoDivision != nil,
            rackEnabled: model.rackFxEnabled,
            rackActive: deckState.rackActive,
            rackMacro: deckState.rackMacro)
    }

    /// Pure forwarders into the model, as with `headerCallbacks`.
    func padsCallbacks(side: DeckSide) -> PerformancePadsCallbacks {
        PerformancePadsCallbacks(
            onCue: { index, clear in
                model.handleHotCue(side, index: index, clear: clear)
            },
            onPreviewDown: { index in model.beginHotCuePreview(side, index: index) },
            onPreviewUp: { model.endHotCuePreview(side) },
            onLoop: { bars in model.handleLoop(side, bars: bars) },
            onLoopIn: { model.setLoopIn(side) },
            onLoopOut: { model.setLoopOut(side) },
            onExit: { model.exitLoop(side) },
            onEchoToggle: { model.toggleEchoOut(side) },
            onRackToggle: { idx in model.toggleRackFx(side, idx) },
            onRackMacro: { idx, value in model.setRackMacro(side, idx, value) })
    }

    /// Fixed chrome carried on the deck side of the divider.
    private var deckChromeHeight: CGFloat {
        model.engineMode == .prep ? 0 : DubLayout.rackBarHeight + 1
    }

    // MARK: - Global rack bar

    /// Snapshot for the bar. The siren block is omitted in Prep, where
    /// the siren and its Expert panel already have their own column.
    private var rackBarState: GlobalRackBarState {
        let focus = model.focusedDeckForGridNudge
        let deck = (focus == .a) ? model.deckA : model.deckB
        return GlobalRackBarState(
            siren: (model.sirenEnabled && model.engineMode != .prep)
                ? SirenRackState(
                    focusedDeck: focus,
                    presetNames: model.sirenLabels(for: focus),
                    sounding: deck.sirenState == 1,
                    unit: deck.sirenUnit,
                    dubMacro: deck.sirenDubMacro)
                : nil,
            quickScratch: (0..<QuickScratchSlots.count).map { index in
                let slot = model.quickScratch.slot(index)
                return TriggerPadState(
                    index: index,
                    key: QuickScratchSlots.keyLabels[index],
                    sampleName: slot.map { SampleBank.label(for: $0.url) },
                    deck: slot?.deck)
            },
            sampler: sampleShelfState(focusedDeck: focus))
    }

    /// The sampler as drawn on either surface: the bank's names over
    /// the engine's lamps. `focusedDeck` names the deck the pads fire on;
    /// Prep passes `nil` — one deck, no pill.
    private func sampleShelfState(focusedDeck: DeckSide?) -> SampleShelfState {
        let voices = model.samplerVoices
        return SampleShelfState(
            slots: (0..<SampleBank.count).map { index in
                let voice = voices.indices.contains(index) ? voices[index] : nil
                return SampleSlotState(
                    index: index,
                    name: model.sampleBank.slot(index).map { SampleBank.label(for: $0) },
                    playing: voice?.playing ?? false,
                    progress: Double(voice?.progress ?? 0))
            },
            focusedDeck: focusedDeck)
    }

    private var sampleShelfCallbacks: SampleShelfCallbacks {
        SampleShelfCallbacks(
            onTrigger: { index in model.triggerSampler(index) },
            onStop: { index in model.stopSampler(index) },
            onDrop: { index, url in model.setSampleSlot(index, url: url) },
            onUnload: { index in model.clearSampleSlot(index) })
    }

    /// Prep's surface, as values. Read fresh each render; nothing here
    /// holds the model, which is what lets `PrepRack` be snapshotted.
    private var prepRackState: PrepRackState {
        PrepRackState(
            cues: (0..<DeckState.hotCueCount).map {
                CueSlotState(index: $0, mark: model.deckA.hotCues[$0])
            },
            activeLoopBeats: model.deckA.activeLoopBeats,
            loopEngaged: model.deckA.loopActive,
            samples: sampleShelfState(focusedDeck: nil),
            hasTrack: model.deckA.hasTrack,
            isPlaying: model.deckA.isPlaying)
    }

    private var prepRackCallbacks: PrepRackCallbacks {
        PrepRackCallbacks(
            onCue: { index, clear in model.handleHotCue(.a, index: index, clear: clear) },
            onPreviewDown: { index in model.beginHotCuePreview(.a, index: index) },
            onPreviewUp: { model.endHotCuePreview(.a) },
            onRenameCue: { index in renameCue(index) },
            onColorCue: { index, token in
                model.setHotCueLabel(
                    .a, index: index,
                    name: model.deckA.hotCues[index]?.name, color: token)
            },
            onLoop: { beats in model.handleLoopBeats(.a, beats: beats) },
            onScaleLoop: { double in model.scaleLoop(.a, double: double) },
            onExitLoop: { model.exitLoop(.a) },
            samples: sampleShelfCallbacks)
    }

    /// Name a cue. A sheet would be heavier than the gesture deserves —
    /// this is a one-field edit on a mark you just dropped — so it is an
    /// `NSAlert` with a text field, the same weight as Finder's rename.
    private func renameCue(_ index: Int, side: DeckSide = .a) {
        let deck = (side == .a) ? model.deckA : model.deckB
        guard let mark = deck.hotCues[index] else { return }
        let alert = NSAlert()
        alert.messageText = "Name cue \(index + 1)"
        alert.informativeText = "At \(CueTimecode.format(mark.positionSecs))."
        alert.addButton(withTitle: "Save")
        alert.addButton(withTitle: "Cancel")
        let field = NSTextField(frame: NSRect(x: 0, y: 0, width: 220, height: 24))
        field.stringValue = mark.name ?? ""
        field.placeholderString = "INTRO"
        alert.accessoryView = field
        guard alert.runModal() == .alertFirstButtonReturn else { return }
        let trimmed = field.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        model.setHotCueLabel(
            side, index: index,
            name: trimmed.isEmpty ? nil : trimmed,
            color: mark.color)
    }


    /// The siren's focused deck is re-read inside each closure rather
    /// than captured, so a master switch between render and click
    /// routes the press to the deck that has focus *now* — the same
    /// discipline `KeyEventMonitorHost` uses.
    private var rackBarCallbacks: GlobalRackBarCallbacks {
        GlobalRackBarCallbacks(
            onSirenPreset: { idx in
                model.fireSirenPreset(model.focusedDeckForGridNudge, index: idx)
            },
            onSirenUnit: { unit in
                model.setSirenUnit(model.focusedDeckForGridNudge, unit)
            },
            onSirenDubMacro: { value in
                model.setSirenDub(model.focusedDeckForGridNudge, value)
            },
            onQuickScratch: { idx in model.triggerQuickScratch(idx) },
            sampler: sampleShelfCallbacks)
    }

    // MARK: - Waveform region

    /// Centre region. **Two-deck modes** keep the §9.2 symmetric
    /// layout invariant (both deck panes side-by-side, idle
    /// placeholder when one deck has no source). **Prep mode**
    /// collapses to a single full-width deck-A pane — Prep mode
    /// is a single-deck shell (PRD §3.1 / M10.8); a phantom "OFF"
    /// deck-B pane is just noise.
    /// Centre region. **Two-deck modes** keep the §9.2 symmetric
    /// layout invariant (both deck panes side-by-side, idle
    /// placeholder when one deck has no source). **Prep mode**
    /// collapses to a single full-width deck-A pane — Prep mode
    /// is a single-deck shell (PRD §3.1 / M10.8); a phantom "OFF"
    /// deck-B pane is just noise.
    @ViewBuilder
    private var waveformRegion: some View {
        if model.engineMode == .prep {
            VStack(spacing: 1) {
                prepOverviewBand
                // Squeeze the waveform, never the controls. When the
                // region is short the strip gives up height first: a
                // shorter waveform on a prep surface is a graceful
                // degradation, an unreachable STOP button is not.
                deckPane(side: .a, deckIdx: 0, enabled: deckAEnabled)
                    .frame(
                        minHeight: DubLayout.waveformPrepMinHeight,
                        idealHeight: DubLayout.waveformPrepHeight,
                        maxHeight: DubLayout.waveformPrepHeight)
                prepPadBar
                    .layoutPriority(1)
            }
            .frame(minHeight: DubLayout.prepRegionMinHeight)
            .background(DubColor.divider)
        } else {
            // **One measurement, then fixed frames.** The deck column
            // used to be `maxWidth: .infinity` and the strip beside it
            // `layoutPriority(1)` with a cap, so SwiftUI negotiated the
            // split on every layout pass — and AppKit lays this window
            // out on every display cycle. With the column's contents
            // being what they now are, that negotiation cost 25 points
            // of CPU on an idle surface: measured 46 % flexible against
            // 19 % with the width pinned. Reading the pane width once
            // and handing both panes a resolved number makes the pass
            // arithmetic instead of a search.
            GeometryReader { geo in
                perfDeckRow(columnWidth: perfColumnWidth(paneWidth: geo.size.width))
            }
            .background(DubColor.divider)
        }
    }

    /// Half of what the strips and the centre gutter do not take, never
    /// below the column's floor.
    /// Zoom is shared: two strips at different scales is a
    /// beatmatching aid that lies. Session state rather than a stored
    /// preference — a zoom you left on last week is not what you want
    /// to find when you open the app in a booth.
    @State private var waveZoomIndex = WaveformZoom.defaultIndex

    /// Drives the zoom control's fade. The *strips* and the gutter
    /// between them count, not the deck columns either side — a
    /// control that appears while you are reading cue names is
    /// answering a question nobody asked.
    @State var waveformHovered = false

    private func perfColumnWidth(paneWidth: CGFloat) -> CGFloat {
        let claimed = DubLayout.performanceWaveformWidthCap * 2
            + DubLayout.stillpointGutterWidth + 2
        return max(
            DubLayout.performanceDeckColumnMinWidth,
            ((paneWidth - claimed) / 2).rounded(.down))
    }

    @ViewBuilder
    private func perfDeckRow(columnWidth: CGFloat) -> some View {
        HStack(spacing: 1) {
            deckPane(
                side: .a, deckIdx: 0, enabled: deckAEnabled,
                columnWidth: columnWidth,
                zoom: WaveformZoom.steps[waveZoomIndex])
            // Centre gutter: Stillpoint, the round-3 beatmatch aid
            // (docs/investigations/BEATMATCH-AID-STILLPOINT.md).
            // One incoming-tinted band on the lock line: drifts =
            // tempo off, frozen = matched, seated on the line =
            // in, green line grows per beat held. Replaces the
            // rejected round-2 candidates (`BeatmatchStackView`,
            // kept in-tree until the rig verdict).
            //
            // Stillpoint takes the gutter's full height and the zoom
            // control floats over its top edge. It used to *head* the
            // gutter as a row above the canvas, which cost three things
            // at once: the 32 pt slot let the region's seam colour show
            // through as a grey box; the control is wider than the
            // gutter, so deck B — drawn after it — covered its right
            // half; and the canvas measured its "25 % from the top" lock
            // line from 32 pt down, which put it 24 pt under the strips'
            // playheads that the sub-spec says it must be collinear with.
            StillpointView(model: model)
                .frame(width: DubLayout.stillpointGutterWidth)
                .frame(maxHeight: .infinity)
                .overlay(alignment: .top) {
                    // The one place both strips can see it, and out of
                    // the way of either.
                    WaveformZoomControl(index: $waveZoomIndex, visible: waveformHovered)
                        .padding(.top, DubSpacing.sm)
                }
                // Above both panes, so the overhang is not clipped by
                // whichever sibling the stack draws later.
                .zIndex(1)
                // The gutter counts as "over the waveform" so the pointer
                // can travel to the control without dismissing it.
                .onHover { waveformHovered = $0 }
            deckPane(
                side: .b, deckIdx: 1, enabled: deckBEnabled,
                columnWidth: columnWidth,
                zoom: WaveformZoom.steps[waveZoomIndex])
        }
        // No `minHeight` here. `DeckLibrarySplit` assigns this region
        // an explicit height and owns the floor. A `minHeight` inside
        // an explicitly-framed parent reports and draws its minimum
        // regardless of the frame — which is precisely how the pad
        // column came to be painted over by the bar below it.
    }

    /// Prep-mode horizontal Track-Overview strip stacked above
    /// the playing waveform. Always rendered — when no track is
    /// loaded `TrackOverviewView`'s empty-state path draws the
    /// faint dashed midline placeholder, which keeps the
    /// `VStack` layout from jumping when a track loads.
    @ViewBuilder
    private var prepOverviewBand: some View {
        if model.ripPhase == .capture, let session = model.ripSession {
            // M26a — while a rip records, the overview slot shows the
            // live capture envelope instead of the (empty) deck-A
            // overview. Same wire format + decimator as the overview,
            // so the growing shape matches what review shows.
            RipLiveOverview(fetchEnvelope: { startIdx in
                session.envelopeExtend(startIdx: startIdx)
            })
        } else if Self.overviewEnabled {
            ZStack {
                TrackOverviewView(model: model, side: .a, deckIdx: 0,
                                  orientation: .horizontal)
                // M26a — split-marker editing over the loaded spill
                // during rip review. Shares the overview's fraction
                // grid (`OverviewLayout.endPadding`).
                if model.ripPhase == .review {
                    RipSplitMarkerOverlay(
                        markers: model.ripSplits.map {
                            RipMarkerUi(id: $0.id, secs: $0.secs)
                        },
                        durationSecs: ripSideDurationSecs,
                        trim: ripTrim,
                        callbacks: RipSplitOverlayCallbacks(
                            addSplit: { secs in model.addRipSplit(atSecs: secs) },
                            moveSplit: { id, secs in
                                model.moveRipSplit(id: id, toSecs: secs)
                            },
                            removeSplit: { id in model.removeRipSplit(id: id) },
                            audition: { secs in
                                model.ripAudition(fromSecs: secs - 3)
                            },
                            scrub: { secs in
                                model.seekDeck(side: .a, absoluteSecs: secs)
                            },
                            setSideStart: { secs in model.moveRipSideStart(toSecs: secs) },
                            setSideEnd: { secs in model.moveRipSideEnd(toSecs: secs) }))
                }
            }
            .frame(height: DubLayout.deckOverviewHeight)
        }
    }

    /// Duration for the marker overlay: the **whole capture**, not the
    /// trimmed side. The axis has to span the lead-in and run-out for
    /// the shaded regions to be visible at all.
    ///
    /// Deck A's loaded spill once the audition load lands; until then
    /// the recorded length off the status. (The old fallback — the
    /// segment plan's end — is now the *trimmed* end, which would have
    /// hidden the run-out exactly while the decode was in flight.)
    private var ripSideDurationSecs: Double {
        if model.deckA.durationSecs > 0 { return model.deckA.durationSecs }
        if let elapsed = model.ripStatus?.elapsedSecs, elapsed > 0 { return elapsed }
        return model.ripSegments.last?.endSecs ?? 0
    }

    /// Length of the side that actually commits — the capture less the
    /// discarded lead-in and run-out. The review panel reports this
    /// while the overlay's axis spans the whole capture: two different
    /// numbers, and the panel must not overstate what it is about to
    /// write to disk.
    private var ripCommittedSideSecs: Double {
        guard let trim = ripTrim else { return ripSideDurationSecs }
        return max(0, trim.endSecs - trim.startSecs)
    }

    /// The side's bounds for the overlay, or `nil` when nothing is
    /// trimmed — a manual rip and every pre-trim session then render
    /// exactly as they did before, with no shade and no brackets.
    private var ripTrim: RipTrimUi? {
        guard let status = model.ripStatus else { return nil }
        let total = ripSideDurationSecs
        guard total > 0 else { return nil }
        let start = max(0, min(status.sideStartSecs, total))
        let end = min(total, max(status.sideEndSecs, start))
        guard start > 0.01 || end < total - 0.01 else { return nil }
        return RipTrimUi(startSecs: start, endSecs: end)
    }

    /// Prep-mode pad bar under the waveform. Prep is the prepare-and-
    /// **test** surface (PRD §3.1): the DJ sets markers and auditions
    /// them, so every control is clickable — usability, not a
    /// performance concern. The CUE row is live (click empty → set at
    /// playhead, click set → jump, ⇧-click → clear). The LOOP row is
    /// live too: each length pad (½/1/2/4 bar) triggers a grid-snapped
    /// reverse loop of the bars just heard, ✕ exits. Cues show as
    /// magenta markers and the loop as a green band on the overview +
    /// strip.
    @ViewBuilder
    private var prepPadBar: some View {
        Group {
            if showsRipReviewPanel {
                // M26a — rip review / encode replaces the pad rows
                // while the recorded side is split, tagged, imported.
                RipReviewPanel(
                    state: ripReviewPanelState,
                    callbacks: ripReviewPanelCallbacks)
            } else {
                prepPadRows
            }
        }
        .padding(.horizontal, DubSpacing.lg)
        .padding(.vertical, DubSpacing.sm)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(DubColor.surface0)
    }

    /// Extra height the pad bar needs while the rip lane is showing.
    ///
    /// The bar used to absorb this in a `ScrollView`; without one the
    /// region has to actually ask for the space, or a capture would push
    /// the rack off its own surface. Rip is leaving Prep for its own
    /// surface, at which point this goes with it.
    private var prepRipLaneHeight: CGFloat {
        var extra: CGFloat = 0
        if model.ripRecoverable.first != nil { extra += 56 }
        if ripBarState != nil { extra += 56 }
        return extra
    }

    @ViewBuilder
    private var prepPadRows: some View {
        VStack(alignment: .leading, spacing: DubSpacing.sm) {
            // The rip lane spans all three columns and stays *above*
            // them. During a capture it carries the REC clock, the
            // level meter and STOP; burying STOP under ~200 pt of pads
            // inside a scroll view would be a reliability regression,
            // and `RipRecoveryBanner` is by its own contract offered
            // back before anything else in the rip surface.
            if let first = model.ripRecoverable.first {
                RipRecoveryBanner(
                    state: RipRecoveryBannerState(
                        recordedSecs: first.recordedSecs,
                        wasInterrupted: first.wasInterrupted,
                        others: model.ripRecoverable.count - 1),
                    onReview: { model.resumeRip(first) },
                    onDismiss: { model.dismissRecoverableRips() })
            }
            if let ripBarState {
                PrepRipBar(state: ripBarState, callbacks: ripBarCallbacks)
            }
            // No `ScrollView`. It was the escape hatch for the DS01E
            // Expert panel's ~250 pt of user-triggered content, and the
            // Expert panel left Prep with the rest of the FX. A scroll
            // view is greedy — it takes every pixel offered — so with
            // nothing left to expand it was silently claiming the space
            // below the rack and holding it empty, which is the gap the
            // three height re-measurements above kept failing to close.
            // The rack is a fixed height now; the bar sizes to it.
            PrepRack(state: prepRackState, callbacks: prepRackCallbacks)
        }
    }

    // MARK: - Vinyl rip state mapping (M26a)

    /// Review / encoding (and their terminal states) replace the pad
    /// rows with the review panel; a capture failure with nothing
    /// recorded stays in the compact bar.
    private var showsRipReviewPanel: Bool {
        switch model.ripPhase {
        case .review, .encoding, .done:
            return true
        case .failed:
            return !model.ripSegments.isEmpty
        case .none, .capture:
            return false
        }
    }

    /// `nil` hides the rip row entirely (feature off / no interface).
    private var ripBarState: PrepRipBarState? {
        switch model.ripPhase {
        case .none:
            guard model.canStartRipCapture else { return nil }
            return PrepRipBarState(phase: .idle)
        case .capture:
            // Armed vs recording comes straight from the FFI phase:
            // with auto-start the worker, not the button, decides when
            // the rip actually begins.
            return PrepRipBarState(
                phase: model.ripStatus?.phase == .armed ? .armed : .recording,
                elapsedSecs: model.ripStatus?.elapsedSecs ?? 0,
                levelPeak: model.ripStatus?.levelPeak ?? 0)
        case .failed:
            return PrepRipBarState(
                phase: .failed,
                errorMessage: model.ripStatus?.error ?? "Recording failed.")
        case .review, .encoding, .done:
            // The review panel owns these phases.
            return nil
        }
    }

    private var ripBarCallbacks: PrepRipBarCallbacks {
        PrepRipBarCallbacks(
            onRecord: { model.startRipCapture() },
            onStop: { model.stopRip() },
            onDismiss: { model.dismissRip() },
            onDiscard: { model.dismissRip() })
    }

    private var ripReviewPanelState: RipReviewPanelState {
        let mode: RipReviewPanelState.Mode
        switch model.ripPhase {
        case .encoding: mode = .encoding
        case .done:     mode = .done
        case .failed:   mode = .failed
        default:        mode = .review
        }
        let segments = model.ripSegments.map { seg in
            RipSegmentUi(
                index: seg.index,
                startSecs: seg.startSecs,
                endSecs: seg.endSecs,
                title: seg.title ?? "",
                artist: seg.artist ?? "",
                album: seg.album ?? "",
                genre: seg.genre ?? "",
                year: seg.year.map(String.init) ?? "")
        }
        var dots: [RipJobDot] = []
        if let jobs = model.ripJobs, !jobs.perSegment.isEmpty {
            // M26b reports real per-segment progress, so a dot means
            // what it says: exactly the segment being encoded reads
            // amber, the ones behind it are still pending.
            dots = jobs.perSegment
                .sorted { $0.index < $1.index }
                .map { job in
                    switch job.state {
                    case .pending: return .pending
                    case .running: return .running
                    case .done:    return .done
                    case .failed:  return .failed
                    }
                }
        }
        let status: String?
        switch mode {
        case .encoding:
            status = "Encoding \(segments.count) track\(segments.count == 1 ? "" : "s")…"
        case .failed:
            status = model.ripStatus?.error ?? "Some tracks failed to import."
        case .review:
            // Say how the side ended when the operator wasn't the one
            // who ended it — otherwise a rip that stopped itself looks
            // indistinguishable from one that was cut short.
            switch model.ripStatus?.stopReason {
            case .silence:     status = "Side ended — stopped in the run-out."
            case .maxDuration: status = "Hit the 40-minute recording cap."
            case .inputLost:   status = "Input was lost — everything up to that point was kept."
            default:           status = nil
            }
        case .done:
            status = nil
        }
        return RipReviewPanelState(
            mode: mode,
            sideDurationSecs: ripCommittedSideSecs,
            trimmedSecs: max(0, ripSideDurationSecs - ripCommittedSideSecs),
            segments: segments,
            jobDots: dots,
            overallStatus: status,
            recognition: model.ripRecognition)
    }

    private var ripReviewPanelCallbacks: RipReviewPanelCallbacks {
        RipReviewPanelCallbacks(
            addSplitAtPlayhead: { model.addRipSplitAtPlayhead() },
            autoSplit: { model.autoSplitRip() },
            audition: { secs in model.ripAudition(fromSecs: secs) },
            setMetadata: { index, meta in
                model.setRipSegmentMetadata(
                    index: index,
                    title: meta.title.isEmpty ? nil : meta.title,
                    artist: meta.artist.isEmpty ? nil : meta.artist,
                    album: meta.album.isEmpty ? nil : meta.album,
                    genre: meta.genre.isEmpty ? nil : meta.genre,
                    year: Int32(meta.year.trimmingCharacters(in: .whitespaces)))
            },
            cancel: { model.cancelRip() },
            identify: { model.identifyRip() },
            applyRecognition: { model.applyRipRecognition() },
            encode: { model.confirmRip() },
            retry: { model.confirmRip() })
    }

}

/// Per-deck drop modifier. M11d.5: applied to each deck's
/// vertical column (header + waveform + FX strip) so dragging
/// onto any part of the deck lands the load. Pre-fix the drop
/// modifier was scoped to the 80 px waveform strip only, which
/// the user reported as "I keep missing the strip; the header
/// should also accept drops". Behaviour: macOS 13+ Transferable
/// API, auto-play on a successful load **in Prep mode only**
/// (the drag-to-play idiom from M10.5d), and a `true` return
/// value so SwiftUI knows the drop was consumed. In Performance
/// mode the drop loads but does not start the deck — playback is
/// driven by the control vinyl (or an explicit Play press).
struct DeckDropTarget: ViewModifier {
    let model: WaveformAppModel
    let side: DeckSide

    func body(content: Content) -> some View {
        content.dropDestination(for: URL.self) { urls, _ in
            guard let url = urls.first else { return false }
            Task { @MainActor in
                if await model.loadTrack(side: side, url: url) {
                    // Drag-to-play idiom (M10.5d) applies in Prep mode
                    // only. In Performance mode the loaded track must
                    // wait for the control vinyl (or an explicit Play
                    // press). Auto-playing here calls `play(side:)`,
                    // which in Timecode mode engages *user-initiated*
                    // Panic-Play — internal playback that ignores
                    // timecode until the deck is paused. That was the
                    // "load auto-starts internal play and the record
                    // does nothing" bug.
                    if model.engineMode == .prep {
                        model.play(side: side)
                    }
                }
            }
            return true
        }
    }
}

#Preview("Performance — idle") {
    PerformanceView(
        model: WaveformAppModel(),
        openPreferences: {},
        openAbout: {})
        .frame(width: 1440, height: 900)
}
