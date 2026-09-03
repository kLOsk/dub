//
//  PerformanceView+DeckPane.swift
//  Dub
//
//  One deck's pane: the Metal waveform column, its overview strip, the
//  performance pads, and the idle placeholder that stands in when the
//  deck has no source.
//
//  Split out of `PerformanceView`, which was over a thousand lines and
//  holding both surfaces plus every deck internal — against
//  `.cursor/rules/swift.mdc`'s "views are small, extract subviews when
//  a body exceeds ~30 lines". Mechanical move; no behaviour change.
//  Members the pane needs from the parent lose `private` because Swift
//  scopes that to a single file.
//

import AppKit
import DubCore
import SwiftUI

extension PerformanceView {
    /// **M11d.6 round 13 jitter-isolation toggle.** Set to `false`
    /// to bypass `TrackOverviewView` entirely. Used to confirm
    /// that the round-13 Canvas split (static bars + markers
    /// separated from per-tick playhead overlay) actually
    /// removed the 4 Hz CoreAnimation contention that previously
    /// stole ~12 ms from the Metal render thread every 250 ms.
    /// With the overview off the trace showed σ Δplayhead =
    /// 0.0465 ms (= the noise floor) and zero vsync misses, so
    /// the residual user-visible jitter wasn't in playhead
    /// timing — it was in the beat-tick rasterisation width
    /// (see `beatHalfNDC` in `drawBeatGrid`). Toggle preserved
    /// for future regression isolation.
    static let overviewEnabled: Bool = true

    /// Orientation of the playing waveform for the current engine
    /// mode. Performance (Timecode) mode keeps the canonical PRD
    /// §9.1 vertical scroll; Prep mode rotates 90° to a horizontal
    /// strip so a single-deck workflow can spread the whole audible
    /// window across the screen width.
    var waveformOrientation: WaveformOrientation {
        model.engineMode == .prep ? .horizontal : .vertical
    }


    /// One deck's pane — Metal waveform when the deck has any
    /// source, idle placeholder otherwise. The pane (drop target,
    /// background, error-flash zone) spans the full half-window
    /// width, but the waveform *strip* itself is width-capped and
    /// centred. The remaining horizontal space is reserved for the
    /// M10.5c Track-Overview waveform and per-deck info chips.
    /// PRD §5.5: the pane is the drop target for Finder-drag file
    /// loads; PRD §6.4: the pane surfaces the 200 ms red flash when
    /// a load fails because the target deck is currently playing.
    @ViewBuilder
    func deckPane(side: DeckSide, deckIdx: UInt64, enabled: Bool) -> some View {
        let deckState = (side == .a) ? model.deckA : model.deckB
        let hasSource = enabled && (deckState.hasTrack
                                    || (model.engineMode == .timecode && model.isRunning))
        ZStack {
            switch waveformOrientation {
            case .vertical:
                // Per-deck vertical-mode row layout (PRD §9.2 /
                // §9.6.1):
                //   Deck A: [overview] [gap] [filler] [playing] [filler]
                //   Deck B: [filler] [playing] [filler] [gap] [overview]
                // The overview sits on each deck's **outer** edge —
                // window-left for deck A, window-right for deck B —
                // matching Serato Scratch Live, Traktor Scratch, and
                // rekordbox DVS. Pre-fix, both overviews were pinned
                // against the centre divider, which crowded the
                // beatmatch surface and read as "deck B is mirrored
                // wrong" at glance distance. Filler regions remain
                // reserved for forthcoming info chips (RPM toggle,
                // key-lock, beatgrid offset) and the M10.7 centre-
                // gutter Phase-Drift Trail.
                // M11d.5: the overview always renders, even with
                // no track loaded — its empty-state path draws a
                // faint dashed midline so the strip reads as
                // chrome the user can drop a track onto rather
                // than as "where did my overview go?". Pre-fix
                // the overview was conditional on
                // `deckState.hasTrack`, which left the strip
                // invisible at cold launch and made the deck
                // pane look bare in screenshots.
                // Redesign: a single outer `Spacer` pulls each deck's
                // waveform toward the **centre** so the two decks form
                // one tight cluster with the phase clock between them
                // (where the eyes converge during a mix), and the
                // overview sits on the deck's outer edge. Pre-redesign
                // two Spacers centred each waveform in its own half,
                // leaving the two strips marooned far apart in dead
                // space.
                // Scratch-Live-style deck pane. Inner→outer:
                //   waveform (hugs the centre phase clock) · overview ·
                //   performance pads · air on the outer edge.
                // Deck A is the window-left half (pads on the left),
                // deck B the right (pads on the right).
                //
                // The pad column is a fixed width and the waveform
                // takes what is left, up to its cap. It used to be the
                // other way round — the pads were `maxWidth: .infinity`
                // and the waveform a hard 200 — so the element PRD §9.2
                // calls "front and center" was the only one with a cap
                // while the pads ate ~575 pt per pane at full screen.
                //
                // `layoutPriority` on the waveform is load-bearing:
                // without it the HStack splits the slack evenly with
                // the outer `Spacer` and the strip never reaches its
                // cap.
                HStack(spacing: 0) {
                    if side == .a {
                        Spacer(minLength: 0)
                        PerformancePadsView(
                            side: side,
                            state: padsState(side: side, deckState: deckState),
                            callbacks: padsCallbacks(side: side))
                            .frame(width: DubLayout.performancePadColumnWidth)
                        if Self.overviewEnabled {
                            TrackOverviewView(
                                model: model, side: side, deckIdx: deckIdx)
                            Color.clear.frame(width: DubLayout.deckOverviewGap)
                        }
                        playingColumn(
                            side: side, deckIdx: deckIdx,
                            hasSource: hasSource)
                            .layoutPriority(1)
                    } else {
                        playingColumn(
                            side: side, deckIdx: deckIdx,
                            hasSource: hasSource)
                            .layoutPriority(1)
                        if Self.overviewEnabled {
                            Color.clear.frame(width: DubLayout.deckOverviewGap)
                            TrackOverviewView(
                                model: model, side: side, deckIdx: deckIdx)
                        }
                        PerformancePadsView(
                            side: side,
                            state: padsState(side: side, deckState: deckState),
                            callbacks: padsCallbacks(side: side))
                            .frame(width: DubLayout.performancePadColumnWidth)
                        Spacer(minLength: 0)
                    }
                }
            case .horizontal:
                // Prep-mode horizontal strip — playing waveform
                // fills the full pane width, no side spacers, no
                // overview (the Track Overview lives on a separate
                // surface in Prep mode). Stops the SwiftUI
                // `Spacer(minLength: 0)` siblings from competing
                // with `playingColumn`'s `maxWidth: .infinity` and
                // collapsing the strip.
                playingColumn(
                    side: side, deckIdx: deckIdx,
                    hasSource: hasSource)
            }
            loadErrorOverlay(side: side, deckState: deckState)
            // Timecode signal health, on the deck instead of buried in
            // Preferences: a slim SIGNAL tab on the deck's outer edge
            // slides the per-deck scope over the pads area. Overlay, so
            // toggling never reflows the Metal waveform column. Prep
            // mode is file-only — no timecode chrome there.
            if waveformOrientation == .vertical {
                DeckSignalSlideOut(model: model, side: side, deckIdx: deckIdx)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .modifier(DeckDropTarget(model: model, side: side))
    }

    /// The width-capped centre column inside a `deckPane` —
    /// either the playing waveform (Metal `WaveformView`) or the
    /// idle placeholder. Pulled out of `deckPane` so the two
    /// row-layouts (deck A vs deck B mirror) share the same
    /// rendering.
    @ViewBuilder
    func playingColumn(side: DeckSide, deckIdx: UInt64, hasSource: Bool) -> some View {
        let deckState = (side == .a) ? model.deckA : model.deckB
        let orientation = waveformOrientation
        let content = Group {
            if hasSource {
                WaveformView(
                    engine: model.engine, deckIdx: deckIdx,
                    palette: model.palette, side: side,
                    orientation: orientation,
                    scrubHandler: scrubHandler(side: side),
                    continuouslyRendering:
                        deckState.isPlaying || model.scratchingDeck == side,
                    seekGeneration: deckState.seekGeneration,
                    peaksGeneration: deckState.peaksGeneration,
                    timeAxisZoom: model.engineMode == .prep
                        ? WaveformRenderer.prepModeTimeAxisZoom
                        : 1.0,
                    hotCues: deckState.hotCues.compactMap { $0 },
                    loopActive: deckState.loopActive,
                    loopInSecs: deckState.loopInSecs,
                    loopOutSecs: deckState.loopOutSecs)
                    .background(DubColor.surface0)
            } else {
                idlePane(side: side)
            }
        }
        switch orientation {
        case .vertical:
            // Fixed, moderate width (Scratch-Live-style): the overview
            // hugs the waveform's outer edge and the remaining outer
            // space holds the performance pads. Full-bleed was wrong —
            // a vertical scratch waveform wants time-history, not a
            // metre of horizontal peak detail.
            content
                .frame(
                    minWidth: DubLayout.performanceWaveformMinWidth,
                    idealWidth: DubLayout.performanceWaveformWidth,
                    maxWidth: DubLayout.performanceWaveformWidthCap)
                .frame(maxHeight: .infinity)
        case .horizontal:
            // Horizontal Prep-mode strip: full window width, fixed
            // height. The deckPane's outer `.frame(height:)` already
            // constrains the vertical extent so we just let the
            // strip expand horizontally to fill its parent.
            content
                .frame(maxWidth: .infinity)
                .frame(maxHeight: .infinity)
        }
    }

    /// M10.5s vinyl-style scratch on the zoomed waveform. Returns
    /// a handler in both Prep and Performance modes — the user's
    /// "find the exact start of the kick" workflow needs audio
    /// under the cursor regardless of engine mode (PRD §1 update;
    /// rate-driven mouse scratching for cueing is allowed as a
    /// usability gesture). Returns `nil` only when the deck has no
    /// track loaded (the WaveformView still renders, but the
    /// gesture would have nothing to scratch).
    ///
    /// The handler shape (`onBegan` + `onOffsetChanged` + `onEnded`)
    /// lets the host own the rate-from-velocity polling timer + the
    /// Panic-Play-around-scratch bookkeeping in
    /// `WaveformAppModel.scratch*`. The view only reports raw
    /// pointer offsets in audio seconds; the host does all the
    /// derivative maths.
    func scrubHandler(side: DeckSide) -> WaveformScrubHandler? {
        let deck = (side == .a) ? model.deckA : model.deckB
        guard deck.hasTrack else { return nil }
        return WaveformScrubHandler(
            onBegan: { [weak modelRef = model] in
                modelRef?.scratchBegin(side: side)
            },
            onOffsetChanged: { [weak modelRef = model] offsetSecs in
                modelRef?.scratchPointerOffset(
                    side: side, offsetSecs: offsetSecs)
            },
            onEnded: { [weak modelRef = model] in
                modelRef?.scratchEnd(side: side)
            })
    }

    /// Red flash overlay surfaced for ~200 ms when a load is
    /// rejected because the deck is currently playing. The exact
    /// expiry timestamp lives on `DeckState.errorFlashUntil`; we
    /// rely on the 30 Hz poll inside the model to clear the field
    /// (which republishes and removes the overlay).
    @ViewBuilder
    func loadErrorOverlay(side: DeckSide, deckState: DeckState) -> some View {
        if let until = deckState.errorFlashUntil, until > Date() {
            ZStack {
                DubColor.stateError.opacity(0.55)
                Text("DECK IS PLAYING — LIFT THE NEEDLE")
                    .font(DubFont.caps)
                    .tracking(1.5)
                    .foregroundStyle(.white)
                    .padding(DubSpacing.lg)
                    .background(DubColor.stateError.opacity(0.95))
                    .clipShape(RoundedRectangle(cornerRadius: DubRadius.panel))
            }
            .allowsHitTesting(false)
            .transition(.opacity)
            .animation(.easeOut(duration: 0.15), value: until)
        }
    }

    /// Is deck A enabled for the current engine mode?
    var deckAEnabled: Bool {
        switch model.engineMode {
        case .timecode: return model.isRunning
        case .prep:     return model.isRunning
        }
    }

    /// Is deck B enabled for the current engine mode? In Prep mode
    /// deck B is intentionally off (PRD §3.1 — Prep is a
    /// single-deck shell).
    var deckBEnabled: Bool {
        switch model.engineMode {
        case .timecode: return model.isRunning && model.twoDeckMode
        case .prep:     return false
        }
    }

    /// Idle pane content — a 1-px deck-tinted playhead hairline at
    /// 25 % from the top (so the canonical orientation reads from
    /// the moment the app launches, even before any audio plays),
    /// plus a context-appropriate hint. Mirrors `WaveformView`'s
    /// `playheadOverlay` orientation logic: vertical mode draws a
    /// horizontal hairline at y = 25 % from the top, horizontal
    /// mode draws a vertical hairline at x = 25 % from the left.
    func idlePane(side: DeckSide) -> some View {
        let orientation = waveformOrientation
        return GeometryReader { geo in
            ZStack(alignment: .topLeading) {
                DubColor.surface0
                PlayheadMarker(
                    orientation: orientation,
                    size: geo.size,
                    subdued: true)
                VStack(spacing: DubSpacing.sm) {
                    Text(side.label)
                        .font(DubFont.caps)
                        .tracking(1.2)
                        .foregroundStyle(DubColor.deckTint(side).opacity(0.7))
                    Text(idleCaption(side: side))
                        .font(DubFont.caps)
                        .tracking(0.6)
                        .foregroundStyle(DubColor.textSecondary)
                    let hint = idleHint(side: side)
                    if !hint.isEmpty {
                        Text(hint)
                            .font(DubFont.body)
                            .foregroundStyle(DubColor.textPlaceholder)
                            .multilineTextAlignment(.center)
                            // U-19 — let the cue wrap and scale down on
                            // narrow windows (≲840 px) instead of
                            // truncating mid-word.
                            .lineLimit(nil)
                            .minimumScaleFactor(0.8)
                            .fixedSize(horizontal: false, vertical: true)
                            .padding(.horizontal, DubSpacing.lg)
                    }
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .center)
            }
        }
    }

    func idleCaption(side: DeckSide) -> String {
        switch side {
        case .a:
            return model.isRunning ? "DECK STOPPED" : "ENGINE STOPPED"
        case .b:
            if !model.isRunning { return "ENGINE STOPPED" }
            switch model.engineMode {
            case .timecode: return "SINGLE-DECK MODE"
            case .prep:     return "PREP MODE — DECK B OFF"
            }
        }
    }

    func idleHint(side: DeckSide) -> String {
        switch side {
        case .a:
            if !model.isRunning {
                // Surface the real reason the engine is stopped (a
                // start failure / mic denial) so the user sees the
                // actual error instead of a generic "configure" nudge.
                if let err = model.lastError {
                    return err
                }
                return "Open Preferences (⌘,) to pick an input device and start."
            }
            return "Drag an audio file here, or press Space to load the browser selection."
        case .b:
            if !model.isRunning {
                // U-20 — deck A already carries the engine-stopped
                // "Open Preferences" nudge (or the real start error).
                // Repeating it on deck B is the one genuinely
                // duplicated cue across the two panes, so deck B stays
                // quiet here; its caption ("ENGINE STOPPED") is enough.
                return ""
            }
            switch model.engineMode {
            case .timecode:
                return "Drag a file here, or configure deck B's channels in Preferences (⌘,) for Thru."
            case .prep:
                return "Prep mode shows a single deck. Switch to Performance in Preferences for two decks."
            }
        }
    }
}
