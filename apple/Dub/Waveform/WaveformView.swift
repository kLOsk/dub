//
//  WaveformView.swift
//  Dub
//
//  M10-B SwiftUI wrapper around an `MTKView` driven by
//  `WaveformRenderer`. The view holds a single `DubEngine` reference
//  (passed in by `MainView`) and polls it for new peaks each frame.
//
//  Threading: `MTKView` invokes its delegate on the main thread when
//  configured with `isPaused = false` and `enableSetNeedsDisplay =
//  false`, which is what we do here. The renderer is `@MainActor`
//  isolated; SwiftUI lifecycle methods run on the main actor; no
//  cross-thread hazards.
//

import SwiftUI
import MetalKit
import QuartzCore
import DubCore

/// **B-24 beat-grid master switch (SwiftUI Canvas path).**
///
/// `false` — beat grid renders in Metal inside `WaveformRenderer`
/// (production default since M11d.5 round 6). `true` re-enables
/// the legacy SwiftUI `Canvas` overlay for A/B comparison only.
private let beatGridOverlayEnabled = false

/// M10.5s vinyl-style scratch callbacks for the zoomed waveform.
/// Replaces the M10.5r seek-and-play loop with a rate-driven
/// scratch: the view reports the cursor's running offset from the
/// drag start (in audio seconds) and the host
/// (`WaveformAppModel.scratch*`) integrates that into a playback
/// rate. Mouse-still ⇒ offset stops changing ⇒ rate falls to 0 ⇒
/// silence, exactly like a stylus on a stationary platter.
///
/// Constructed as a value type so SwiftUI's diffing treats it as
/// stable across renders (the captured closures may differ, but
/// the surrounding `WaveformView` already rebuilds them per render
/// via `scrubHandler`-style factories, which is fine).
struct WaveformScrubHandler {
    /// Called on the first `onChanged` event of a drag, before any
    /// `onOffsetChanged`. Host captures pre-scratch transport,
    /// engages Panic Play in Timecode mode, freezes the playhead.
    let onBegan: () -> Void
    /// Called on every subsequent `onChanged` event with the
    /// cursor's running offset (in audio seconds) from the drag's
    /// start point. Positive = forward; negative = reverse. Host
    /// integrates these via a polling timer into a rate.
    let onOffsetChanged: (TimeInterval) -> Void
    /// Called on `onEnded`. Host stops the polling timer, restores
    /// pre-scratch transport, cancels Panic Play if engaged.
    let onEnded: () -> Void
}

/// SwiftUI host for the broadband waveform. The `engine` parameter is
/// the shared `DubEngine` from `MainView`; this view never starts /
/// stops the engine — that's the picker's job.
///
/// M10.4 — the view is now a *vertical* column (PRD §9.1). The Metal
/// pipeline renders past peaks in the top 25 % of the drawable; this
/// wrapper overlays a deck-tinted playhead hairline at 25 % from the
/// top to mark the "now playing" position.
///
/// M10.5c-b — accepts an `orientation` parameter (defaults to
/// `.vertical` so every existing call site renders unchanged). The
/// `.horizontal` variant lays the same waveform out left → right
/// with the playhead 25 % from the left edge, in preparation for
/// the Prep-mode shell shipping in M10.8.
struct WaveformView: View {

    let engine: DubEngine
    let deckIdx: UInt64
    /// Display scale of the host window's screen. Read from the
    /// SwiftUI environment so the B-24 beat-grid overlay can map
    /// the renderer's drawable-pixel cadence into the logical
    /// pixels SwiftUI's `Canvas` paints with. The Metal renderer
    /// addresses time in drawable pixels (via `drawableSize`); the
    /// overlay paints in logical pixels, so the two have to be
    /// reconciled per-frame to keep the grid locked to the
    /// waveform under it across mixed-DPI displays. Defaults to
    /// the environment value (2.0 on Retina, 1.0 on legacy 1×
    /// displays, fractional on scaled-mode external monitors).
    @Environment(\.displayScale) private var displayScale
    /// Shared snapshot the Metal renderer writes after each draw.
    /// The B-24 beat-grid overlay reads playhead, peak duration,
    /// and the cached beats list out of this snapshot rather than
    /// hitting the engine directly. Two effects: (1) both layers
    /// agree on `chunkF` for every frame (no more wobble from
    /// staggered `engine.position` reads), and (2) the per-frame
    /// engine.beatGrid FFI call — which clones a `Vec<f64>` across
    /// UniFFI — drops to "once per track load". The latter
    /// removes a noticeable contributor to the main-thread load
    /// that competed with Metal's draw callback and showed up as
    /// the waveform "blinking/jittering a bit" in Prep mode.
    /// `@StateObject` keeps the instance alive across SwiftUI body
    /// rebuilds; the snapshot itself has no `@Published` fields so
    /// writes don't bounce out of SwiftUI's diff machinery.
    @StateObject private var renderSnapshot = WaveformRenderSnapshot()
    /// M10.2: current palette. Changes flow into the renderer via
    /// `updateNSView`; the renderer reads it on the next frame.
    let palette: WaveformPalette
    /// M10.4: which deck this view belongs to. Drives playhead tint
    /// + future affordances. Defaults to deck A so the existing
    /// preview & single-deck call sites keep working.
    let side: DeckSide
    /// M10.5c-b: time-axis orientation. Vertical is the Performance-
    /// mode default; horizontal is reserved for Prep mode (M10.8)
    /// and other inspector surfaces.
    let orientation: WaveformOrientation
    /// M10.6a / M10.5r continuous-scrub handler (PRD §6.1).
    ///
    /// When non-nil, the view installs a `DragGesture` on top of the
    /// Metal layer that fires on every `onChanged` event — the
    /// pointer's offset from the playhead is converted to a signed
    /// seconds-offset and forwarded to the host, which feeds the
    /// engine via `WaveformAppModel.scrubAudioSeek`. The host owns
    /// the play/pause-around-scrub bookkeeping so audio plays under
    /// the cursor.
    ///
    /// The pre-M10.5r single-tap-only behaviour (fire on `onEnded`)
    /// felt unresponsive because the waveform didn't move with the
    /// mouse. Continuous drag fixes that and is the user-asked-for
    /// "find the exact position of a kick" workflow. Set to `nil`
    /// when the host doesn't want a scrub gesture (e.g. Thru-mode
    /// panes where there's no track to scrub).
    let scrubHandler: WaveformScrubHandler?

    /// Whether the Metal view should run a continuous 60 Hz draw
    /// loop. `false` when the deck is paused and not being
    /// scratched — the MTKView switches to on-demand redraw so
    /// the main thread is free for instant click / transport
    /// response instead of spending every vsync inside
    /// `WaveformRenderer.draw`.
    let continuouslyRendering: Bool

    init(engine: DubEngine, deckIdx: UInt64 = 0,
         palette: WaveformPalette = .serato,
         side: DeckSide = .a,
         orientation: WaveformOrientation = .vertical,
         scrubHandler: WaveformScrubHandler? = nil,
         continuouslyRendering: Bool = true) {
        self.engine = engine
        self.deckIdx = deckIdx
        self.palette = palette
        self.side = side
        self.orientation = orientation
        self.scrubHandler = scrubHandler
        self.continuouslyRendering = continuouslyRendering
    }

    var body: some View {
        GeometryReader { geo in
            ZStack(alignment: .topLeading) {
                // When the overlay is disabled we pass `nil` so the
                // renderer skips its snapshot write path entirely
                // (cheap, but the goal is a clean A/B that proves
                // nothing about the B-24 surface area can affect
                // the Metal pipeline's smoothness).
                WaveformMetalView(
                    engine: engine, deckIdx: deckIdx,
                    palette: palette, orientation: orientation,
                    side: side,
                    continuouslyRendering: continuouslyRendering,
                    renderSnapshot: beatGridOverlayEnabled ? renderSnapshot : nil)
                if let handler = scrubHandler {
                    scrubGestureOverlay(in: geo.size, handler: handler)
                }
                zeroCrossingOverlay(in: geo.size)
                if beatGridOverlayEnabled {
                    beatGridOverlay(in: geo.size)
                }
                playheadOverlay(in: geo.size)
            }
        }
    }

    /// M10.5e zero-crossing hairline. A 1-px line running along the
    /// amplitude=0 axis (i.e. *perpendicular* to the playhead,
    /// parallel to the time axis). Helps the eye read the bar's
    /// symmetry around silence, anchors the strip visually when
    /// the waveform is sparse, and — since M10.5t — provides the
    /// visible "needle is on the platter" baseline in the lead-in /
    /// lead-out empty-groove regions (PRD §9.6). Before M10.5t it
    /// used `DubColor.divider.opacity(0.55)`, which against a
    /// pure-black silent region rendered effectively invisible
    /// (~0x171A1F vs 0x000000); a Serato comparison made it
    /// obvious the dark groove needed a properly-visible
    /// centerline. White at ~20 % opacity matches the Serato
    /// reference: clearly visible against black, almost entirely
    /// hidden under the bars (which are centred on this axis), so
    /// it doesn't read as a separate UI element. Drawn underneath
    /// the playhead overlay so the deck-tinted playhead always
    /// wins where they cross.
    @ViewBuilder
    private func zeroCrossingOverlay(in size: CGSize) -> some View {
        let tint = Color.white.opacity(0.22)
        switch orientation {
        case .vertical:
            Rectangle()
                .fill(tint)
                .frame(width: 1, height: size.height)
                .offset(x: size.width * 0.5 - 0.5)
                .allowsHitTesting(false)
        case .horizontal:
            Rectangle()
                .fill(tint)
                .frame(width: size.width, height: 1)
                .offset(y: size.height * 0.5 - 0.5)
                .allowsHitTesting(false)
        }
    }

    /// Transparent hit-test layer that drives the M10.5s vinyl-
    /// style scratch. We report the cursor's running offset (in
    /// audio seconds) from the drag's start position; the host
    /// (`WaveformAppModel.scratch*`) derives a smoothed playback
    /// rate from the per-event Δoffset / Δt (M10.5t rework — the
    /// earlier 60 Hz timer polled snapshots of the offset, which
    /// aliased against the typical 60–120 Hz cursor-event rate
    /// and produced audible "jumping" on a steady drag). When the
    /// mouse is held still the cursor stops emitting events; the
    /// host's stall watchdog ramps the deck rate toward zero
    /// within ~25 ms, so a stationary mouse plays silence just
    /// like a record under a stationary stylus.
    ///
    /// Sits *under* the playhead overlay in the ZStack so the 1-px
    /// hairline doesn't eat gesture pixels (it has
    /// `allowsHitTesting(false)` anyway, but the order keeps the
    /// rendering intuition clean: gesture surface below, chrome on
    /// top).
    @ViewBuilder
    private func scrubGestureOverlay(
        in size: CGSize,
        handler: WaveformScrubHandler
    ) -> some View {
        let secsPerPixel = Double(WaveformRenderer.secsPerPixel(
            sampleRate: engine.sampleRate()))
        Color.clear
            .contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: 0)
                    .onChanged { value in
                        let deltaPx: CGFloat
                        switch orientation {
                        case .vertical:
                            // Time runs top → bottom under the
                            // stylus in the Metal renderer, so a
                            // downward drag = forward in time =
                            // positive offset. Matches "drag the
                            // record forward".
                            deltaPx = value.location.y - value.startLocation.y
                        case .horizontal:
                            // Past = left, future = right, and forward
                            // playback scrolls the waveform leftward
                            // through the playhead (PRD §9.6). So a
                            // leftward drag mirrors a platter being
                            // pushed forward: the user's finger moves
                            // with the content, and the audio under
                            // the playhead advances. Invert the sign
                            // here so leftward = positive offset =
                            // forward rate.
                            deltaPx = value.startLocation.x - value.location.x
                        }
                        let offsetSecs = Double(deltaPx) * secsPerPixel
                        // `onBegan` is idempotent on the host side
                        // (the model's `scratchBegin` ignores
                        // repeats), so we don't need to dedupe
                        // here — the lazy-begin pattern fires it
                        // on every `onChanged` until the host has
                        // captured pre-scratch state.
                        handler.onBegan()
                        handler.onOffsetChanged(offsetSecs)
                    }
                    .onEnded { _ in
                        handler.onEnded()
                    }
            )
    }

    /// **B-24 (M11d.5) — beat grid overlay on the playing strip.**
    ///
    /// The Metal renderer (`WaveformRenderer`) draws the broadband
    /// envelope only — no rhythmic structure. The Stage-1 BPM
    /// estimator (M7.5, `dub-bpm::analyze_beat_grid`) is invoked on
    /// every `loadTrack` call and the result is exposed via
    /// `DubEngine.beatGrid(deckIdx:)`; this overlay turns that data
    /// into the per-beat ticks the eye uses to count bars and align
    /// transients.
    ///
    /// The overlay is rendered in SwiftUI rather than as a second
    /// Metal pass so we don't have to grow the renderer's shader
    /// pipeline + vertex buffers. The cost is one CPU-side Canvas
    /// draw at the TimelineView's 30 Hz cadence — a few dozen line
    /// segments per frame, well below any perceptible budget on
    /// Apple Silicon. If the per-frame cost ever starts showing in
    /// Instruments traces, fold the pass into Metal with a thin
    /// "ticks" vertex buffer keyed off `engine.beatGrid`.
    ///
    /// **Cadence math (must mirror `WaveformRenderer.draw`).** The
    /// initial pass of this overlay computed `secsPerPx` from the
    /// static `WaveformRenderer.secsPerPixel(sampleRate:)` helper,
    /// which proved wrong twice:
    ///
    /// 1. The helper uses the *engine* sample rate, but the Metal
    ///    pipeline addresses peak chunks at the **track** sample
    ///    rate via `engine.peaksChunkDurationSecs(deckIdx:)`. For
    ///    the canonical 44.1 kHz track on a 48 kHz engine that
    ///    introduces a ~8.8 % systematic error and the grid
    ///    visibly scrolls at the wrong speed relative to the
    ///    waveform under it.
    /// 2. The helper bakes a `chunksPerPixel = 2` constant that
    ///    assumed the M10.5e cadence; the M10.5f 2× zoom-in (which
    ///    inserted `pixelsPerDrawnColumn = 2`) effectively halved
    ///    the per-drawable-pixel time, but the helper was never
    ///    updated. It happens to land on the correct *logical*-
    ///    pixel value on Retina (`displayScale = 2`) because the
    ///    two factors of 2 cancel — but on a scaled external
    ///    display the cancellation collapses and the helper is
    ///    off by another constant factor.
    ///
    /// Both problems disappear if we use the live values the
    /// renderer itself uses. The formula now is:
    ///
    ///   secsPerDrawablePx = chunksPerColumn × peakDur
    ///                       ÷ pixelsPerDrawnColumn
    ///   secsPerLogicalPx  = secsPerDrawablePx × displayScale
    ///   visibleSecs       = size.dim × secsPerLogicalPx
    ///   pastSecs          = visibleSecs × pastRegionFraction
    ///   visibleStartSec   = playhead − pastSecs
    ///
    /// Both `chunksPerColumn` (= 2) and `pixelsPerDrawnColumn`
    /// (= 2, hard-coded inside `WaveformRenderer.draw`) cancel out
    /// to `secsPerDrawablePx = peakDur` in the current shipping
    /// pipeline — we keep both terms in the formula above so a
    /// future zoom-level change to either constant updates this
    /// overlay in lockstep just by mirroring the values.
    ///
    /// **Downbeat heuristic.** Beats are indexed off `beats[0]`
    /// modulo `beats_per_bar`; the BPM estimator doesn't track
    /// absolute 4/4 phase today (M11e Serato importer will land
    /// anchored grids), so the first detected beat is the best
    /// zero we have. Pre-track and post-track empty-groove regions
    /// (PRD §9.6) are intrinsically clamped because the beat list
    /// only spans the loaded audio span.
    ///
    /// **Hidden conditions** (return early from `drawBeatGrid`):
    ///   • `grid.confidence == 0` (estimator didn't lock — drawing
    ///      would be misleading; B-24 spec #4).
    ///   • `grid.beats` empty (defensive across the FFI).
    ///   • `pos.hasTrack == false` (Thru capture, idle deck — no
    ///      track means no grid anchor).
    ///   • `playheadSecsUnclamped` not finite (defensive against a
    ///      mid-frame engine-state read race during load).
    ///   • `peakDur <= 0` (engine hasn't yet reported the chunk
    ///      cadence — happens for a single frame after `loadTrack`
    ///      before the peaks pipeline produces its first row; the
    ///      Metal renderer's own `if peakChunkDurationSecs > 0`
    ///      guard mirrors this).
    @ViewBuilder
    private func beatGridOverlay(in size: CGSize) -> some View {
        // **Cap at 60 Hz to match the Metal pipeline.** `MTKView`
        // is configured with `isPaused = false` +
        // `preferredFramesPerSecond = 60`, so the waveform layer
        // commits a new frame every 16.67 ms. We pin the overlay's
        // `TimelineView` to the same `1/60 s` so:
        //
        //   1. Each Canvas tick has a matching Metal tick within
        //      the same vsync interval. Both layers read the same
        //      `lastDrawnPlayheadSecsUnclamped` out of
        //      `renderSnapshot` and end up at the same `chunkF`.
        //   2. On ProMotion 120 Hz displays the Canvas doesn't
        //      get scheduled at 120 Hz against a 60 Hz Metal —
        //      that's the previous-revision behaviour that caused
        //      half of every Canvas tick to read a stale snapshot
        //      and wobble against the waveform.
        //   3. Every-other vsync that the Canvas skips becomes
        //      pure Metal time, which removes a contributor to
        //      the main-thread contention the user observed as
        //      "waveform blinking/jittering a bit" in Prep mode.
        //
        // Combined with the chunk-grid quantization inside
        // `drawBeatGrid` (snaps `playhead` to `floor(playhead /
        // peakDur) × peakDur` mirroring the Metal renderer) and
        // the shared `renderSnapshot` as the single source of
        // truth, the overlay and the waveform now share both
        // cadence and per-frame state. Wobble cannot exceed the
        // visual quantum of one drawable pixel = 0.5 logical
        // pixel on Retina, which is below the perceptual
        // threshold.
        TimelineView(.animation(minimumInterval: 1.0 / 60.0)) { context in
            Canvas(opaque: false) { ctx, canvasSize in
                // **Critical** — referencing `context.date` inside
                // the Canvas closure is what forces SwiftUI to
                // re-invoke this draw block on every TimelineView
                // tick. Without the read, Canvas treats the closure
                // as input-stable across body re-evaluations and
                // caches the draw output, so the snapshot's
                // updated playhead never re-renders and the grid
                // appears frozen on the screen even though the
                // Metal waveform underneath is scrolling. The read
                // is otherwise unused — the snapshot is the real
                // data source — but it pins the Canvas to the
                // animation schedule (same idiom Apple's
                // TimelineView + Canvas examples document).
                _ = context.date
                drawBeatGrid(into: ctx, size: canvasSize)
            }
            .frame(width: size.width, height: size.height)
            .allowsHitTesting(false)
        }
        .frame(width: size.width, height: size.height)
    }

    /// One-shot beat tick draw, called per TimelineView tick.
    /// Pulled out of the Canvas closure so the overlay's body stays
    /// readable and the math is easy to test in isolation when we
    /// add a snapshot test for B-24 alignment.
    ///
    /// **All inputs come from `renderSnapshot`, never from the
    /// engine FFI directly.** This is the fix for the wobble that
    /// survived the original chunk-quantization pass: the Metal
    /// renderer and the SwiftUI Canvas used to read
    /// `engine.position` at slightly different points in the
    /// main-thread runloop and occasionally landed on different
    /// continuous playhead values. By piping both through a
    /// shared snapshot that the Metal renderer writes on every
    /// draw, the two layers always agree on the playhead.
    /// Worst case the Canvas renders one Metal frame stale
    /// (16.67 ms ≈ 11 chunks at 60 fps) which is a *static*
    /// offset, not a varying wobble, and invisible at the chunk-
    /// sized pixel granularity. The same snapshot also caches the
    /// beats list so we don't pay the per-frame FFI cost of
    /// cloning `Vec<f64>` across the UniFFI boundary.
    private func drawBeatGrid(into ctx: GraphicsContext, size: CGSize) {
        guard renderSnapshot.hasTrack else { return }
        let playheadContinuous = renderSnapshot.lastDrawnPlayheadSecsUnclamped
        guard playheadContinuous.isFinite else { return }
        guard renderSnapshot.beatsConfidence > 0,
              !renderSnapshot.beats.isEmpty
        else { return }
        let peakDur = renderSnapshot.peakDurSecs
        guard peakDur > 0 else { return }

        // **Reproduce the renderer's geometry exactly.** The Metal
        // shader maps `chunkInWindow ∈ [0, chunksVisible - 1]` to
        // NDC linearly using `(chunksVisible - 1)` intervals across
        // the region's NDC span. That `-1` means the renderer's
        // effective secs-per-drawable-pixel is
        //   secsPerDrawablePxPast   = chunksPerColumn × peakDur
        //                           × (drawnAbove - 1) / pastPixels
        //   secsPerDrawablePxFuture = chunksPerColumn × peakDur
        //                           × (drawnBelow - 1) / futurePixels
        // which is ≈ peakDur × (1 − 1/N) per region. The naive
        // overlay rate `chunksPerColumn × peakDur / pixelsPerDrawn
        // Column = peakDur` (i.e. ignoring the off-by-one) is
        // *slightly* faster, so as the playhead advances the grid
        // drifts against the waveform by ~1.27 % per second in the
        // past region and ~0.42 % in the future — the "looks like
        // wobble but the grid is moving at a different speed"
        // symptom. Past and future rates differ from each other
        // too, so a single shared rate can't fix it; we mirror
        // the shader's piecewise NDC math instead.
        //
        // Convention recap (matches `Shaders.metal::waveformVertex`
        // and `WaveformRenderer.draw`):
        //   • `drawnAbove`  drawn columns in the past region, each
        //     spanning `chunksPerColumn` raw peak chunks.
        //   • Past column `chunkInWindow = drawnAbove - 1` is the
        //     column nearest the playhead; its rightmost chunk is
        //     index `playheadChunkSnapped`. Past column 0 is the
        //     oldest, at audio time
        //     `(playheadChunkSnapped + 1 - drawnAbove × chunksPer
        //     Column) × peakDur`.
        //   • Future column `chunkInWindow = 0` is the column just
        //     after the playhead; its leftmost chunk is index
        //     `playheadChunkSnapped + 1`.
        //   • Snapped chunk is `floor(continuousChunkF / chunksPer
        //     Column) × chunksPerColumn` (deterministic from the
        //     continuous playhead, no shared state required).
        //   • `subChunkOffsetNDC` adds the continuous sub-chunk
        //     slide; we reproduce it here so the grid floats
        //     forward with the waveform's geometry inside each
        //     chunk-pair quantum.
        let chunksPerColumn = Int(WaveformRenderer.chunksPerColumn)
        let pixelsPerDrawnColumn = WaveformRenderer.pixelsPerDrawnColumn
        let pastFrac = WaveformRenderer.pastRegionFraction
        let axisLengthLogical: Double
        switch orientation {
        case .vertical:
            axisLengthLogical = Double(size.height)
        case .horizontal:
            axisLengthLogical = Double(size.width)
        }
        guard axisLengthLogical > 1 else { return }
        let scale = Double(displayScale)
        let timeAxisDrawablePixels = max(1, Int((axisLengthLogical * scale).rounded()))
        let pastPixels = max(
            1, Int((Double(timeAxisDrawablePixels) * pastFrac).rounded()))
        let futurePixels = max(
            0, Int((Double(timeAxisDrawablePixels) * (1.0 - pastFrac)).rounded()))
        let drawnAbove = max(2, pastPixels / pixelsPerDrawnColumn)
        let drawnBelow = max(2, futurePixels / pixelsPerDrawnColumn)

        // Snap to the chunk-pair the renderer's data path uses.
        // `chunksPerColumn = 2` ⇒ snap to multiples of 2 chunks.
        let continuousChunkF = playheadContinuous / peakDur
        let snappedChunkF =
            (continuousChunkF / Double(chunksPerColumn)).rounded(.down)
            * Double(chunksPerColumn)
        let epsChunks = max(0.0, continuousChunkF - snappedChunkF)
        // Past region: NDC span 0.5 (top=+1.0 → playhead boundary=+0.5).
        let pastChunksDenom = max(Double(drawnAbove - 1) * Double(chunksPerColumn), 1.0)
        let futureChunksDenom = max(Double(drawnBelow - 1) * Double(chunksPerColumn), 1.0)
        let pastSubChunkOffsetNDC = epsChunks * 0.5 / pastChunksDenom
        let futureSubChunkOffsetNDC = epsChunks * 1.5 / futureChunksDenom

        // NDC → logical-pixel conversion. The drawable spans NDC
        // `[-1, +1]` along the time axis, with `+1` at the leading
        // edge (top in vertical, left in horizontal under the
        // shader's `xNDC = -timeNDC` mirror). Drawable pixel from
        // top/left: `(1 - timeNDC) × axisDrawablePx / 2`.
        let axisDrawablePx = Double(timeAxisDrawablePixels)

        // Visible-window early-out so the per-beat loop doesn't
        // walk every beat in the whole track. The audio time at
        // the leading edge of the past region (`chunkInWindow = 0`,
        // `frac = 0`, `NDC = 1.0 + pastSubChunkOffsetNDC`) is the
        // first past chunk's start; symmetrically the trailing
        // edge of the future region is the last future chunk's
        // end. Conservative ±1 column padding so beats touching
        // the edges still render.
        let pastFirstChunkSigned =
            Int64(snappedChunkF) + 1 - Int64(drawnAbove * chunksPerColumn)
        let futureLastChunkSigned =
            Int64(snappedChunkF) + Int64(drawnBelow * chunksPerColumn)
        let visibleStart =
            Double(pastFirstChunkSigned - Int64(chunksPerColumn)) * peakDur
        let visibleEnd =
            Double(futureLastChunkSigned + Int64(chunksPerColumn)) * peakDur

        let bpb = renderSnapshot.beatsPerBar
        let beats = renderSnapshot.beats
        let tint = DubColor.deckTint(side)
        let beatStroke = GraphicsContext.Shading.color(tint.opacity(0.35))
        let barStroke = GraphicsContext.Shading.color(tint.opacity(0.85))
        let beatLineWidth: CGFloat = 1
        let barLineWidth: CGFloat = 2

        for (idx, beat) in beats.enumerated() {
            if beat < visibleStart { continue }
            if beat > visibleEnd { break }
            // Continuous chunk index for the beat. Compute on the
            // same f64 path the renderer uses so the rounding is
            // bit-for-bit identical.
            let beatChunkF = beat / peakDur
            let chunksFromSnapped = beatChunkF - snappedChunkF
            // **Derivation of the chunkInWindow continuous embedding
            // (M11d.5 round 5 attempt 3).** This is the formula that
            // is C¹-continuous against the renderer's snap+slide
            // motion at BOTH the snap boundary (epsChunks→2 ⇒
            // snapped advances by 2) AND the past/future region
            // boundary (chunksFromSnapped = 1).
            //
            // Anchor points (no slide, no fractional beat positions):
            //   • Beat at chunk = snapped (chunksFromSnapped = 0) →
            //     past column `drawnAbove - 1` → NDC = 0.5 (the
            //     playhead overlay's NDC). The grid line sits on
            //     the playhead hairline. ✓
            //   • Beat at chunk = snapped + 2 (chunksFromSnapped = 2)
            //     → future column 1 ≈ NDC `0.5 − 1.5/(drawnBelow−1)`.
            //
            // Linearly extending those anchor points back into
            // fractional chunksFromSnapped values gives:
            //   chunkInWindow_past   = (drawnAbove − 1)
            //                        + chunksFromSnapped / chunksPerColumn
            //   chunkInWindow_future = chunksFromSnapped / chunksPerColumn
            //
            // Region cut at chunksFromSnapped = 1 (= playhead
            // boundary between past-last column and future-first
            // column). At the cut the two formulas evaluate to
            // NDC = 0.5 − 0.25/(drawnAbove − 1) (past side) and
            // NDC = 0.5 − 0.75/(drawnBelow − 1) (future side). With
            // `drawnBelow = 3 × drawnAbove` (from the 25 %/75 %
            // past/future split that the renderer enforces) those
            // two expressions are algebraically identical → the
            // boundary is C⁰ exactly, C¹ up to the `drawnAbove`
            // rounding that the renderer also pays.
            //
            // At the snap boundary (epsChunks crosses 2 ⇒ snapped
            // advances by `chunksPerColumn`), `chunksFromSnapped`
            // jumps down by 2, `chunkInWindow_past` correspondingly
            // jumps down by 1, and `pastSubChunkOffsetNDC` resets
            // from `0.5 · 2 / (drawnAbove − 1) / chunksPerColumn`
            // back to 0. Those two jumps exactly cancel: the +1
            // column step contributes +0.5 / (drawnAbove − 1) to
            // NDC, while the slide drop contributes
            // −0.5/(drawnAbove − 1) — net 0 visible motion across
            // the snap. Same algebra on the future side with the
            // 1.5 NDC span coefficient. The previous M11d.5
            // attempts ("first fix" with +0.5/−1.5 numerator
            // offsets, "second fix" with −2 future offset) each
            // broke one of these two cancellations and produced a
            // ~1-logical-pixel snap-boundary or region-boundary
            // jump that the user perceives as flicker / drift.
            let inFuture = chunksFromSnapped > 1.0

            let timeNDC: Double
            if inFuture {
                let chunkInWindow =
                    chunksFromSnapped / Double(chunksPerColumn)
                let frac = chunkInWindow / Double(drawnBelow - 1)
                timeNDC = 0.5 - 1.5 * frac + futureSubChunkOffsetNDC
            } else {
                let chunkInWindow =
                    Double(drawnAbove - 1)
                    + chunksFromSnapped / Double(chunksPerColumn)
                let frac = chunkInWindow / Double(drawnAbove - 1)
                timeNDC = 1.0 - 0.5 * frac + pastSubChunkOffsetNDC
            }

            let drawablePixel = (1.0 - timeNDC) * axisDrawablePx * 0.5
            let logicalPixel = drawablePixel / scale
            let isDownbeat = bpb > 0 && (idx % bpb == 0)
            switch orientation {
            case .vertical:
                let y = CGFloat(logicalPixel)
                let path = Path { p in
                    p.move(to: CGPoint(x: 0, y: y))
                    p.addLine(to: CGPoint(x: size.width, y: y))
                }
                ctx.stroke(
                    path,
                    with: isDownbeat ? barStroke : beatStroke,
                    lineWidth: isDownbeat ? barLineWidth : beatLineWidth)
            case .horizontal:
                let x = CGFloat(logicalPixel)
                let path = Path { p in
                    p.move(to: CGPoint(x: x, y: 0))
                    p.addLine(to: CGPoint(x: x, y: size.height))
                }
                ctx.stroke(
                    path,
                    with: isDownbeat ? barStroke : beatStroke,
                    lineWidth: isDownbeat ? barLineWidth : beatLineWidth)
            }
        }
    }

    /// 1-px deck-tinted hairline marking the "now playing" position
    /// the Metal renderer addresses (PRD §9.1 / §9.6). Vertical
    /// orientation draws a horizontal hairline at 25 % from the top;
    /// horizontal orientation draws a vertical hairline at 25 % from
    /// the left. Tinted in the deck's accent colour so the two
    /// columns stay disambiguated at a glance.
    @ViewBuilder
    private func playheadOverlay(in size: CGSize) -> some View {
        let fraction = CGFloat(WaveformRenderer.pastRegionFraction)
        switch orientation {
        case .vertical:
            Rectangle()
                .fill(DubColor.deckTint(side))
                .frame(width: size.width, height: 1)
                .offset(y: size.height * fraction)
                .allowsHitTesting(false)
        case .horizontal:
            Rectangle()
                .fill(DubColor.deckTint(side))
                .frame(width: 1, height: size.height)
                .offset(x: size.width * fraction)
                .allowsHitTesting(false)
        }
    }
}

/// Bare `MTKView` host — the SwiftUI/AppKit bridge. Separated from
/// `WaveformView` so the playhead overlay can live in pure SwiftUI
/// without forcing the `NSViewRepresentable` to host both layers.
private struct WaveformMetalView: NSViewRepresentable {

    let engine: DubEngine
    let deckIdx: UInt64
    let palette: WaveformPalette
    let orientation: WaveformOrientation
    let side: DeckSide
    /// When `false`, the MTKView stops its continuous draw loop
    /// and only repaints on demand (see `updateNSView`).
    let continuouslyRendering: Bool
    /// Shared snapshot the renderer publishes draw state into for
    /// the B-24 beat-grid overlay. See the `@StateObject` on
    /// `WaveformView` for the lifecycle reasoning. `nil` when the
    /// B-24 overlay is gated off via `beatGridOverlayEnabled` so
    /// the renderer's snapshot write path is fully bypassed.
    let renderSnapshot: WaveformRenderSnapshot?

    @MainActor
    final class Coordinator: NSObject, MTKViewDelegate {
        var renderer: WaveformRenderer?
        private weak var mtkView: MTKView?
        private var cvDisplayLink: CVDisplayLink?
        private var continuousRendering = false

        func setContinuousRendering(_ active: Bool, on view: MTKView) {
            guard active != continuousRendering else { return }
            continuousRendering = active
            mtkView = view
            stopDisplayLink()
            view.isPaused = true
            view.enableSetNeedsDisplay = true
            if active {
                startDisplayLink()
            } else {
                view.setNeedsDisplay(view.bounds)
            }
        }

        private func requestDraw() {
            guard let view = mtkView else { return }
            view.setNeedsDisplay(view.bounds)
        }

        private func startDisplayLink() {
            var link: CVDisplayLink?
            guard CVDisplayLinkCreateWithActiveCGDisplays(&link) == kCVReturnSuccess,
                  let link
            else { return }
            let selfPtr = Unmanaged.passUnretained(self).toOpaque()
            CVDisplayLinkSetOutputCallback(link, { _, _, _, _, _, userInfo in
                guard let userInfo else { return kCVReturnSuccess }
                let coordinator = Unmanaged<Coordinator>.fromOpaque(userInfo)
                    .takeUnretainedValue()
                // Post directly onto the main run loop — avoids the
                // `DispatchQueue.main.async` hop that jittered off vsync.
                CFRunLoopPerformBlock(CFRunLoopGetMain(), CFRunLoopMode.commonModes.rawValue) {
                    coordinator.requestDraw()
                }
                CFRunLoopWakeUp(CFRunLoopGetMain())
                return kCVReturnSuccess
            }, selfPtr)
            CVDisplayLinkStart(link)
            cvDisplayLink = link
        }

        private func stopDisplayLink() {
            if let cvDisplayLink {
                CVDisplayLinkStop(cvDisplayLink)
                self.cvDisplayLink = nil
            }
        }

        deinit {
            if let cvDisplayLink {
                CVDisplayLinkStop(cvDisplayLink)
            }
        }

        // MARK: MTKViewDelegate

        nonisolated func mtkView(_ view: MTKView, drawableSizeWillChange size: CGSize) {
            MainActor.assumeIsolated {
                renderer?.drawableSizeWillChange(size)
            }
        }

        nonisolated func draw(in view: MTKView) {
            MainActor.assumeIsolated {
                renderer?.draw(in: view)
            }
        }
    }

    func makeCoordinator() -> Coordinator {
        Coordinator()
    }

    func makeNSView(context: Context) -> MTKView {
        let mtkView = MTKView()
        mtkView.colorPixelFormat = .bgra8Unorm
        mtkView.clearColor = MTLClearColor(red: 0.07, green: 0.07, blue: 0.08, alpha: 1.0)
        mtkView.framebufferOnly = true
        mtkView.isPaused = true
        mtkView.enableSetNeedsDisplay = true
        mtkView.preferredFramesPerSecond = 60
        // 4× MSAA on the drawable. The waveform geometry is a stack
        // of trapezoid slices with sub-pixel edge slopes at high
        // zoom; without MSAA they stair-step into a visible
        // "venetian blind" pattern. MTKView allocates the
        // multisample texture itself when `sampleCount > 1` and the
        // render pass descriptor it hands us already has the
        // multisample → drawable resolve wired up.
        mtkView.sampleCount = WaveformRenderer.sampleCount

        if let device = MTLCreateSystemDefaultDevice() {
            mtkView.device = device
            do {
                let renderer = try WaveformRenderer(
                    device: device, engine: engine, deckIdx: deckIdx)
                renderer.palette = palette
                renderer.orientation = orientation
                renderer.side = side
                renderer.beatGridEnabled = true
                renderer.renderSnapshot = renderSnapshot
                context.coordinator.renderer = renderer
                mtkView.delegate = context.coordinator
            } catch {
                NSLog("WaveformView: renderer init failed: \(error.localizedDescription)")
            }
        } else {
            NSLog("WaveformView: MTLCreateSystemDefaultDevice() returned nil")
        }
        DispatchQueue.main.async {
            mtkView.setNeedsDisplay(mtkView.bounds)
        }
        return mtkView
    }

    func updateNSView(_ nsView: MTKView, context: Context) {
        // Push the current palette + orientation into the renderer.
        // Cheap — just property assignments; the next draw frame
        // picks both up via the uniforms buffer. M10.5c-b
        // orientation changes also implicitly remap which drawable
        // dimension drives `chunksVisible` (see the orientation
        // switch in `WaveformRenderer.draw`).
        //
        // `renderSnapshot` is also (re)assigned here — it's a
        // reference type so the assignment is cheap; importantly,
        // if SwiftUI ever rebuilds the parent view with a new
        // `@StateObject` instance (unusual, but legal), the
        // renderer picks up the new snapshot without needing a
        // full `makeNSView` cycle.
        context.coordinator.renderer?.palette = palette
        context.coordinator.renderer?.orientation = orientation
        context.coordinator.renderer?.side = side
        context.coordinator.renderer?.renderSnapshot = renderSnapshot

        // Demand-driven vsync via run-loop-integrated `CVDisplayLink`
        // while playing/scratching (no `DispatchQueue.main.async` hop).
        context.coordinator.setContinuousRendering(continuouslyRendering, on: nsView)
    }
}
