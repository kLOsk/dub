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
//      ├─ FX bar placeholder (M15 / M16 / M17) ──────────────────┤
//      ├─ library / FS browser (M10.5b) ─────────────────────────┤
//      └─────────────────────────────────────────────────────────┘
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

    var body: some View {
        VStack(spacing: 0) {
            statusStrip
            deckHeaders
            Rectangle().fill(DubColor.divider).frame(height: 1)
            waveformRegion
            Rectangle().fill(DubColor.divider).frame(height: 1)
            FXBarPlaceholder()
            Rectangle().fill(DubColor.divider).frame(height: 1)
            LibraryView(model: model)
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

    @ViewBuilder
    private var deckHeaders: some View {
        if model.engineMode == .prep {
            DeckHeader(side: .a,
                       state: headerState(side: .a),
                       callbacks: headerCallbacks(side: .a),
                       mirrored: false,
                       liveEngine: model.engine,
                       liveDeckIdx: 0)
                .background(DubColor.divider)
                .modifier(DeckDropTarget(model: model, side: .a))
        } else {
            HStack(spacing: 1) {
                DeckHeader(side: .a,
                           state: headerState(side: .a),
                           callbacks: headerCallbacks(side: .a),
                           mirrored: headerMirrored(side: .a),
                           liveEngine: model.engine,
                           liveDeckIdx: 0)
                    .modifier(DeckDropTarget(model: model, side: .a))
                DeckHeader(side: .b,
                           state: headerState(side: .b),
                           callbacks: headerCallbacks(side: .b),
                           mirrored: headerMirrored(side: .b),
                           liveEngine: model.engine,
                           liveDeckIdx: 1)
                    .modifier(DeckDropTarget(model: model, side: .b))
            }
            .background(DubColor.divider)
        }
    }

    // MARK: - Status strip

    private var statusStrip: some View {
        StatusStripContainer(
            engineVersion: engineVersion(),
            sampleRate: model.engine.sampleRate(),
            isRunning: model.isRunning,
            lastError: model.lastError,
            openPreferences: openPreferences)
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
            thruMode: model.engineMode == .timecode,
            isMaster: model.masterDeck == side,
            prepMode: model.engineMode == .prep)
    }

    /// Should this deck's header render horizontally mirrored?
    /// Two-deck Performance / Timecode mode mirrors deck B so its
    /// identity cluster ends up on the window's right edge instead
    /// of pinned against the divider. Prep mode is single-deck and
    /// never mirrors. See `DeckHeader.mirrored` for the rationale.
    private func headerMirrored(side: DeckSide) -> Bool {
        guard model.engineMode == .timecode else { return false }
        return side == .b
    }

    /// M10.6a Casual-Play transport callbacks for the deck header.
    /// Pure forwarders into the model — the header doesn't get a
    /// direct model reference, which keeps the view trivially
    /// snapshot-testable in M18.
    private func headerCallbacks(side: DeckSide) -> DeckHeaderCallbacks {
        DeckHeaderCallbacks(
            onPlay:    { model.play(side: side) },
            onPause:   { model.pause(side: side) },
            onPanicToggle: { model.panicToggle(side: side) })
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
                deckPane(side: .a, deckIdx: 0, enabled: deckAEnabled)
                    .frame(height: DubLayout.waveformPrepHeight)
            }
            .background(DubColor.divider)
        } else {
            HStack(spacing: 1) {
                deckPane(side: .a, deckIdx: 0, enabled: deckAEnabled)
                deckPane(side: .b, deckIdx: 1, enabled: deckBEnabled)
            }
            .frame(minHeight: DubLayout.waveformMinHeight)
            .background(DubColor.divider)
        }
    }

    /// Prep-mode horizontal Track-Overview strip stacked above
    /// the playing waveform. Always rendered — when no track is
    /// loaded `TrackOverviewView`'s empty-state path draws the
    /// faint dashed midline placeholder, which keeps the
    /// `VStack` layout from jumping when a track loads.
    @ViewBuilder
    private var prepOverviewBand: some View {
        TrackOverviewView(model: model, side: .a, deckIdx: 0,
                          orientation: .horizontal)
    }

    /// Orientation of the playing waveform for the current engine
    /// mode. Performance (Timecode) mode keeps the canonical PRD
    /// §9.1 vertical scroll; Prep mode rotates 90° to a horizontal
    /// strip so a single-deck workflow can spread the whole audible
    /// window across the screen width.
    private var waveformOrientation: WaveformOrientation {
        model.engineMode == .prep ? .horizontal : .vertical
    }

    /// Column width the playing waveform strip is rendered at in
    /// **vertical** orientation (Performance / Timecode mode). In
    /// Prep mode the strip is horizontal and uses
    /// `DubLayout.waveformPrepHeight` instead.
    private var waveformColumnWidth: CGFloat {
        DubLayout.deckColumnWidth
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
    private func deckPane(side: DeckSide, deckIdx: UInt64, enabled: Bool) -> some View {
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
                HStack(spacing: 0) {
                    if side == .a {
                        TrackOverviewView(
                            model: model, side: side, deckIdx: deckIdx)
                        Color.clear.frame(width: DubLayout.deckOverviewGap)
                        Spacer(minLength: 0)
                        playingColumn(
                            side: side, deckIdx: deckIdx,
                            hasSource: hasSource)
                        Spacer(minLength: 0)
                    } else {
                        Spacer(minLength: 0)
                        playingColumn(
                            side: side, deckIdx: deckIdx,
                            hasSource: hasSource)
                        Spacer(minLength: 0)
                        Color.clear.frame(width: DubLayout.deckOverviewGap)
                        TrackOverviewView(
                            model: model, side: side, deckIdx: deckIdx)
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
    private func playingColumn(side: DeckSide, deckIdx: UInt64, hasSource: Bool) -> some View {
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
                        deckState.isPlaying || model.scratchingDeck == side)
                    .background(DubColor.surface0)
            } else {
                idlePane(side: side)
            }
        }
        switch orientation {
        case .vertical:
            content
                .frame(width: waveformColumnWidth)
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
    private func scrubHandler(side: DeckSide) -> WaveformScrubHandler? {
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
    private func loadErrorOverlay(side: DeckSide, deckState: DeckState) -> some View {
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
    private var deckAEnabled: Bool {
        switch model.engineMode {
        case .timecode: return model.isRunning
        case .prep:     return model.isRunning
        }
    }

    /// Is deck B enabled for the current engine mode? In Prep mode
    /// deck B is intentionally off (PRD §3.1 — Prep is a
    /// single-deck shell).
    private var deckBEnabled: Bool {
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
    private func idlePane(side: DeckSide) -> some View {
        let orientation = waveformOrientation
        return GeometryReader { geo in
            ZStack(alignment: .topLeading) {
                DubColor.surface0
                Group {
                    switch orientation {
                    case .vertical:
                        Rectangle()
                            .fill(DubColor.deckTint(side).opacity(0.35))
                            .frame(width: geo.size.width, height: 1)
                            .offset(y: geo.size.height
                                * CGFloat(WaveformRenderer.pastRegionFraction))
                    case .horizontal:
                        Rectangle()
                            .fill(DubColor.deckTint(side).opacity(0.35))
                            .frame(width: 1, height: geo.size.height)
                            .offset(x: geo.size.width
                                * CGFloat(WaveformRenderer.pastRegionFraction))
                    }
                }
                VStack(spacing: DubSpacing.sm) {
                    Text(side.label)
                        .font(DubFont.caps)
                        .tracking(1.2)
                        .foregroundStyle(DubColor.deckTint(side).opacity(0.7))
                    Text(idleCaption(side: side))
                        .font(DubFont.caps)
                        .tracking(0.6)
                        .foregroundStyle(DubColor.textSecondary)
                    Text(idleHint(side: side))
                        .font(DubFont.body)
                        .foregroundStyle(DubColor.textPlaceholder)
                        .multilineTextAlignment(.center)
                        .padding(.horizontal, DubSpacing.lg)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .center)
            }
        }
    }

    private func idleCaption(side: DeckSide) -> String {
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

    private func idleHint(side: DeckSide) -> String {
        switch side {
        case .a:
            if !model.isRunning {
                return "Open Preferences (⌘,) to pick an input device and start."
            }
            return "Drag an audio file here, or press Space to load the browser selection."
        case .b:
            if !model.isRunning {
                return "Open Preferences (⌘,) to start the engine."
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

/// Per-deck drop modifier. M11d.5: applied to each deck's
/// vertical column (header + waveform + FX strip) so dragging
/// onto any part of the deck lands the load. Pre-fix the drop
/// modifier was scoped to the 80 px waveform strip only, which
/// the user reported as "I keep missing the strip; the header
/// should also accept drops". Behaviour matches the prior path:
/// macOS 13+ Transferable API, auto-play on a successful load
/// (matches the drag-to-play idiom from M10.5d), and a `true`
/// return value so SwiftUI knows the drop was consumed.
private struct DeckDropTarget: ViewModifier {
    let model: WaveformAppModel
    let side: DeckSide

    func body(content: Content) -> some View {
        content.dropDestination(for: URL.self) { urls, _ in
            guard let url = urls.first else { return false }
            Task { @MainActor in
                if await model.loadTrack(side: side, url: url) {
                    model.play(side: side)
                }
            }
            return true
        }
    }
}

#Preview("Performance — idle") {
    PerformanceView(model: WaveformAppModel(), openPreferences: {})
        .frame(width: 1440, height: 900)
}
