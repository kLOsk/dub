//
//  TrackOverviewView.swift
//  Dub
//
//  M10.5c per-deck Track Overview. A thin vertical strip
//  (`DubLayout.deckOverviewWidth` ≈ 36 px) on the deck's *outside*
//  edge showing the *whole* track top→bottom with a playhead
//  bracket at the current position. Click or click-and-drag on the
//  overview seeks the deck to absolute track positions (Traktor-
//  style overview scrub). Transport is left alone — paused stays
//  paused, playing keeps playing from each new position.
//
//  Design notes (PRD §9.6.1):
//      • Vertical orientation, time runs **top → bottom**. This
//        matches Serato's overview (and the convention every DJ
//        already knows). The *playing waveform* is bottom-→-top
//        because that has to mirror platter rotation under the
//        playhead; the *overview* is a static map of the whole
//        track and doesn't have a playhead-vs-hand-motion
//        constraint, so the top-→-bottom reading order wins for
//        glance-ability.
//      • Rendered via SwiftUI `Canvas`, *not* Metal. The overview
//        is low-cadence (redraws only when the playhead chunk
//        changes ≈ 30 Hz) and fully-known-up-front (entire peak
//        array fetched once at load). Adding a second Metal
//        renderer would double our shader inventory for zero
//        performance benefit; `Canvas` keeps the pipeline simple.
//      • Decimated to a fixed bucket count (`Self.bucketCount`)
//        regardless of strip height. The bucket cap caps both
//        memory and draw cost; the Canvas's own scaling handles
//        the strip-height variation.
//      • Source-swap aware via `model.engine.peaksGeneration` —
//        a Thru → File / File → File swap forces a re-decimation
//        on the next render, same signal the playing-waveform
//        renderer uses to reset its ring.
//

import SwiftUI

import DubCore

/// One decimated amplitude value per overview bucket (M10.5r
/// rebuild). Carries the full broadband peak + RMS shape so the
/// renderer can paint the same two-tone envelope the Metal
/// playing-waveform uses: a bright outer hull at `peak`, a darker
/// inner core at `rms`. Matches the visual vocabulary of the main
/// strip without sharing any of its Metal pipeline.
private struct OverviewBucket {
    /// Outer envelope amplitude — `max(|min|, |max|)` across the
    /// bucket's chunk range, clamped to `[0, 1]`.
    var peak: Float
    /// Inner RMS — averaged over the bucket's chunk range, also
    /// clamped to `[0, 1]`. Always `<= peak` by construction.
    var rms: Float
}

/// Background padding around the bar field. The user-facing fix
/// from the M10.5t "no warping at the start and end" feedback —
/// without it the bars touch the top / bottom (or left / right)
/// edges of the strip and read as a solid block, especially on
/// loud-throughout material. The padding doubles as a hit-test
/// dead-zone so a stray click at the very edge doesn't snap to
/// `0` or `durationSecs` — clicks inside the padding are clamped
/// to the nearest bar.
private enum OverviewLayout {
    /// Padding (in points) reserved as dark background at each end
    /// of the time axis. 8 pt at 2× DPR = 16 device pixels = three
    /// bar widths at the default 480-bucket cap, so the empty
    /// edges read as visibly intentional rather than as cropping.
    static let endPadding: CGFloat = 8
}

struct TrackOverviewView: View {

    @ObservedObject var model: WaveformAppModel
    let side: DeckSide
    let deckIdx: UInt64
    /// Time-axis orientation. `.vertical` is the canonical
    /// Performance-mode column (top → bottom). `.horizontal`
    /// stacks the overview as a thin band across the top of the
    /// Prep-mode horizontal playing waveform (left → right).
    /// Defaults to `.vertical` so every existing call site keeps
    /// rendering unchanged.
    var orientation: WaveformOrientation = .vertical

    /// Bucket count for the decimated overview. 480 is enough to
    /// resolve every visible pixel on a typical 600 px-tall strip
    /// at 2× DPR (1200 device pixels) without being so dense that
    /// the bars merge into a smear. Tunable knob; smaller values
    /// look cleaner on very short tracks.
    private static let bucketCount: Int = 480

    /// Decimated peak data. `nil` until the deck has a track and we
    /// have a peak count to read from the engine. `[]` is the
    /// "loaded but empty / not enough data yet" state — the strip
    /// renders an empty background.
    @State private var buckets: [OverviewBucket]? = nil

    /// `peaks_generation` value that produced `buckets`. When the
    /// engine's current generation differs we know the source has
    /// swapped (Thru → File on `load_track`, or File → File on a
    /// second load) and re-decimate.
    @State private var lastSeenGeneration: UInt64 = 0

    /// While the user drags on the overview, the playhead bracket
    /// tracks the finger immediately instead of waiting for the
    /// engine seek + TimelineView tick. Cleared on gesture end.
    @State private var dragPlayheadFraction: Double? = nil

    /// Throttle engine seeks during a drag so fast mouse motion
    /// doesn't flood the FFI layer; the bracket still follows the
    /// finger every frame via `dragPlayheadFraction`.
    @State private var lastOverviewSeekUptime: TimeInterval = 0

    /// Last fraction we actually dispatched a `seekDeck(...)` for.
    /// Used by the `onEnded` branch in `handleOverviewScrub` to
    /// suppress a duplicate seek when the user clicked-and-released
    /// without moving the mouse: the click already drove the
    /// playhead to this fraction inside the `onChanged` branch, the
    /// track has since been advancing under that anchor, and a
    /// second seek to the same fraction would yank the playhead
    /// *back* to the click point. That backward yank is the
    /// user-visible "jump on mouse-up" reported against the M10.5t
    /// overview gesture. `nil` whenever a gesture isn't in flight
    /// or the track has changed under us (reset by `reloadIfStale`).
    @State private var lastSeekedFraction: Double? = nil

    /// Tolerance (in normalised axis fractions, 0.0–1.0) below
    /// which a final-up fraction is considered "the same point" as
    /// the last seek we dispatched. Sized so that a fingernail-
    /// width nudge during click-and-release still suppresses the
    /// duplicate seek, while any deliberate drag clears it on
    /// the first `onChanged` past the threshold. Picked rather
    /// than `== 0` because the AppKit drag pipeline can report a
    /// `value.location` ~1 px off the click-down point even
    /// without intentional motion (mouse hardware jitter), and
    /// we don't want a sub-pixel drift to look like a drag.
    private static let overviewSeekDedupTolerance: Double = 0.002

    private static let overviewSeekMinInterval: TimeInterval = 1.0 / 30.0

    private var deckState: DeckState {
        switch side {
        case .a: return model.deckA
        case .b: return model.deckB
        }
    }

    var body: some View {
        // GeometryReader gives us the strip's actual rendered
        // height inside the closure — which we need both for the
        // Canvas's draw math and for click-to-jump's fraction
        // calculation. SwiftUI gestures don't expose the view
        // bounds; reading them off the geo proxy is the
        // idiomatic workaround.
        //
        // M11d.5 round 5: the Canvas is wrapped in a 10 Hz
        // `TimelineView` and reads the playhead position from
        // `engine.position(deckIdx:)` directly. Pre-fix the
        // bracket advanced because `DeckState.elapsedSecs`
        // republished every second, which invalidated the parent
        // `PerformanceView` and triggered a full-tree body
        // re-eval — that was the residual "subtle leftward jump
        // every second" the user reported after round 3. With the
        // self-driven timeline, only this Canvas closure re-runs
        // on the bracket cadence; nothing above the timeline ever
        // observes the per-second tick.
        // 4 Hz overview tick. Pre-fix this was 10 Hz, which
        // sounds reasonable for a position indicator but actually
        // costs two SwiftUI body re-evals + two Canvas redraws per
        // 100 ms (both decks share the runloop), and the visible
        // playhead bracket moves at most a fraction of a pixel
        // per tick on any realistic-length DJ track (e.g. a 4-min
        // track at 60 fps overview width means ~0.15 px / tick at
        // 10 Hz, 0.36 px / tick at 4 Hz — neither is humanly
        // discernible from "smooth"). The Metal pipeline already
        // drives the *zoomed* waveform's high-frequency playhead
        // separately, so the overview only needs to feel
        // "advancing" to the eye. 4 Hz saves runloop budget that
        // was contending with the 60 Hz Metal draw callback.
        TimelineView(.periodic(from: .now, by: 0.25)) { context in
            GeometryReader { geo in
                ZStack {
                    Canvas { ctx, size in
                        _ = context.date
                        drawBackground(ctx: ctx, size: size)
                        if let buckets, !buckets.isEmpty {
                            drawBars(ctx: ctx, size: size, buckets: buckets)
                            drawMinuteMarkers(ctx: ctx, size: size)
                            drawPlayhead(ctx: ctx, size: size)
                        } else {
                            drawEmptyState(ctx: ctx, size: size)
                        }
                    }
                    if let buckets, !buckets.isEmpty {
                        minuteMarkerLabels(in: geo.size)
                    }
                }
                .contentShape(Rectangle())
                .gesture(
                    // Traktor-style overview scrub: click or click-and-
                    // drag to seek to absolute track positions. Unlike
                    // the zoomed waveform's rate-driven vinyl scratch,
                    // this is coarse position navigation — the deck
                    // jumps to each point under the cursor. Transport
                    // is left alone (paused stays paused; playing
                    // keeps playing from each new position).
                    DragGesture(minimumDistance: 0)
                        .onChanged { value in
                            handleOverviewScrub(
                                at: value.location,
                                in: geo.size,
                                isFinal: false)
                        }
                        .onEnded { value in
                            handleOverviewScrub(
                                at: value.location,
                                in: geo.size,
                                isFinal: true)
                        })
            }
        }
        .modifier(OverviewSizing(orientation: orientation))
        .onAppear(perform: reloadIfStale)
        .onChange(of: deckState.sourceURL) { _ in reloadIfStale() }
        .onChange(of: deckState.hasTrack) { _ in reloadIfStale() }
        .onChange(of: deckState.peaksGeneration) { _ in reloadIfStale() }
    }

    // MARK: - Drawing

    private func drawBackground(ctx: GraphicsContext, size: CGSize) {
        let rect = CGRect(origin: .zero, size: size)
        ctx.fill(Path(rect), with: .color(DubColor.surface1))
        // 1-px hairline marking the seam against the playing
        // waveform. Vertical mode: seam on the inner edge (right
        // for deck A, left for deck B). Horizontal Prep mode: the
        // overview sits *above* the playing strip so the seam is
        // along the overview's bottom edge.
        let seam: CGRect
        switch orientation {
        case .vertical:
            seam = side == .a
                ? CGRect(x: size.width - 1, y: 0, width: 1, height: size.height)
                : CGRect(x: 0, y: 0, width: 1, height: size.height)
        case .horizontal:
            seam = CGRect(x: 0, y: size.height - 1, width: size.width, height: 1)
        }
        ctx.fill(Path(seam), with: .color(DubColor.divider))
    }

    private func drawBars(ctx: GraphicsContext, size: CGSize, buckets: [OverviewBucket]) {
        let n = buckets.count
        guard n > 0 else { return }
        // Two-tone envelope matching the Metal playing-waveform's
        // Serato-faithful look (M10.5r refresh): bright outer hull
        // at `peak`, slightly transparent darker core at `rms`.
        //
        // M10.5t: bars live inside `[axisStart, axisEnd]` so the
        // very-first and very-last bar don't kiss the strip edges
        // (the "warping" the user reported). `axisLength` is the
        // axis-aligned length minus 2× endPadding.
        let peakColor = peakBarColor()
        let rmsColor = rmsBarColor()
        let pad = OverviewLayout.endPadding
        switch orientation {
        case .vertical:
            let axisStart = pad
            let axisLength = max(0, size.height - 2 * pad)
            let centreX = size.width * 0.5
            let halfW = size.width * 0.5 - 2
            var peakPath = Path()
            var rmsPath = Path()
            for (i, bucket) in buckets.enumerated() {
                let y0 = axisStart + axisLength * CGFloat(i) / CGFloat(n)
                let y1 = axisStart + axisLength * CGFloat(i + 1) / CGFloat(n)
                let height = max(1, y1 - y0)
                let peakW = max(1, CGFloat(bucket.peak.clamped01) * halfW)
                peakPath.addRect(CGRect(
                    x: centreX - peakW, y: y0,
                    width: peakW * 2, height: height))
                let rmsW = max(0.5, CGFloat(bucket.rms.clamped01) * halfW)
                rmsPath.addRect(CGRect(
                    x: centreX - rmsW, y: y0,
                    width: rmsW * 2, height: height))
            }
            ctx.fill(peakPath, with: .color(peakColor))
            ctx.fill(rmsPath, with: .color(rmsColor))
        case .horizontal:
            let axisStart = pad
            let axisLength = max(0, size.width - 2 * pad)
            let centreY = size.height * 0.5
            let halfH = size.height * 0.5 - 2
            var peakPath = Path()
            var rmsPath = Path()
            for (i, bucket) in buckets.enumerated() {
                let x0 = axisStart + axisLength * CGFloat(i) / CGFloat(n)
                let x1 = axisStart + axisLength * CGFloat(i + 1) / CGFloat(n)
                let width = max(1, x1 - x0)
                let peakH = max(1, CGFloat(bucket.peak.clamped01) * halfH)
                peakPath.addRect(CGRect(
                    x: x0, y: centreY - peakH,
                    width: width, height: peakH * 2))
                let rmsH = max(0.5, CGFloat(bucket.rms.clamped01) * halfH)
                rmsPath.addRect(CGRect(
                    x: x0, y: centreY - rmsH,
                    width: width, height: rmsH * 2))
            }
            ctx.fill(peakPath, with: .color(peakColor))
            ctx.fill(rmsPath, with: .color(rmsColor))
        }
    }

    /// Minute-boundary ticks along the time axis so the DJ can
    /// read track length and current position at a glance.
    private func drawMinuteMarkers(ctx: GraphicsContext, size: CGSize) {
        guard let duration = overviewDurationSecs(), duration > 0 else { return }
        let pad = OverviewLayout.endPadding
        let tickColor = Color.white.opacity(0.28)
        let majorTickColor = Color.white.opacity(0.42)
        let intervalSecs = duration >= 120 ? 60.0 : 30.0
        var marker = intervalSecs
        while marker < duration {
            let fraction = marker / duration
            switch orientation {
            case .vertical:
                let axisStart = pad
                let axisLength = max(0, size.height - 2 * pad)
                let y = axisStart + axisLength * CGFloat(fraction)
                let isMinute = intervalSecs >= 60
                let tick = Path { path in
                    path.move(to: CGPoint(x: 0, y: y))
                    path.addLine(to: CGPoint(x: size.width, y: y))
                }
                ctx.stroke(
                    tick,
                    with: .color(isMinute ? majorTickColor : tickColor),
                    lineWidth: isMinute ? 1.25 : 0.75)
            case .horizontal:
                let axisStart = pad
                let axisLength = max(0, size.width - 2 * pad)
                let x = axisStart + axisLength * CGFloat(fraction)
                let isMinute = intervalSecs >= 60
                let tick = Path { path in
                    path.move(to: CGPoint(x: x, y: 0))
                    path.addLine(to: CGPoint(x: x, y: size.height))
                }
                ctx.stroke(
                    tick,
                    with: .color(isMinute ? majorTickColor : tickColor),
                    lineWidth: isMinute ? 1.25 : 0.75)
            }
            marker += intervalSecs
        }
    }

    @ViewBuilder
    private func minuteMarkerLabels(in size: CGSize) -> some View {
        if let duration = overviewDurationSecs(), duration > 0 {
            let intervalSecs = duration >= 120 ? 60.0 : 30.0
            let labels = minuteLabelPoints(duration: duration, intervalSecs: intervalSecs)
            ForEach(labels, id: \.self) { point in
                switch orientation {
                case .vertical:
                    Text(point.label)
                        .font(.system(size: 7, weight: .semibold, design: .rounded))
                        .foregroundColor(Color.white.opacity(0.72))
                        .position(x: size.width - 6, y: axisPosition(fraction: point.fraction, size: size) ?? 0)
                case .horizontal:
                    Text(point.label)
                        .font(.system(size: 7, weight: .semibold, design: .rounded))
                        .foregroundColor(Color.white.opacity(0.72))
                        .position(x: axisPosition(fraction: point.fraction, size: size) ?? 0, y: 7)
                }
            }
        }
    }

    private struct MinuteLabelPoint: Hashable {
        let label: String
        let fraction: Double
    }

    private func minuteLabelPoints(duration: Double, intervalSecs: Double) -> [MinuteLabelPoint] {
        var points: [MinuteLabelPoint] = [MinuteLabelPoint(label: "0", fraction: 0)]
        var marker = intervalSecs
        while marker < duration {
            points.append(
                MinuteLabelPoint(
                    label: Self.formatMarkerLabel(secs: marker, intervalSecs: intervalSecs),
                    fraction: marker / duration))
            marker += intervalSecs
        }
        return points
    }

    /// Sub-minute intervals get either `"Ns"` (under a minute) or
    /// `"M:SS"` (past a minute) so adjacent labels never collapse
    /// to the same string (e.g. 60s and 90s both being `"1m"`).
    /// Minute intervals stay terse as `"Nm"`.
    private static func formatMarkerLabel(secs: Double, intervalSecs: Double) -> String {
        let total = Int(secs.rounded())
        if intervalSecs >= 60 || total % 60 == 0 {
            return "\(total / 60)m"
        }
        if total < 60 {
            return "\(total)s"
        }
        return String(format: "%d:%02d", total / 60, total % 60)
    }

    private func axisPosition(fraction: Double, size: CGSize) -> CGFloat? {
        let pad = OverviewLayout.endPadding
        switch orientation {
        case .vertical:
            let axisLength = max(0, size.height - 2 * pad)
            guard axisLength > 0 else { return nil }
            return pad + axisLength * CGFloat(fraction)
        case .horizontal:
            let axisLength = max(0, size.width - 2 * pad)
            guard axisLength > 0 else { return nil }
            return pad + axisLength * CGFloat(fraction)
        }
    }

    /// Track duration on the same peak-chunk grid as the bars and
    /// playhead bracket.
    private func overviewDurationSecs() -> Double? {
        // M11d.6 round 5 — lock-free FFI snapshot. The renderer
        // and the chrome consumers all read the same atomic
        // `DeckSharedState` values, so there is no longer a need
        // for the host model to deduplicate position calls.
        let pos = model.engine.positionSnapshot(deckIdx: deckIdx)
        let peaksLen = model.engine.peaksLen(deckIdx: deckIdx)
        let chunkDur = model.engine.peaksChunkDurationSecs(deckIdx: deckIdx)
        if peaksLen > 0, chunkDur > 0 {
            return Double(peaksLen) * chunkDur
        }
        if pos.durationSecs > 0 {
            return pos.durationSecs
        }
        return nil
    }

    private func drawPlayhead(ctx: GraphicsContext, size: CGSize) {
        guard let fraction = playheadFraction() else { return }
        let chevronSize = DubLayout.playheadChevronSize
        let coreW = DubLayout.playheadCoreWidth
        let haloW = DubLayout.playheadHaloWidth
        let accent = DubColor.playheadAccent
        let haloColor = Color.black.opacity(0.55)
        let pad = OverviewLayout.endPadding
        switch orientation {
        case .vertical:
            let axisLength = max(0, size.height - 2 * pad)
            let y = pad + axisLength * CGFloat(fraction)
            let halo = CGRect(x: 0, y: y - haloW * 0.5,
                              width: size.width, height: haloW)
            ctx.fill(Path(halo), with: .color(haloColor))
            let line = CGRect(x: 0, y: y - coreW * 0.5,
                              width: size.width, height: coreW)
            ctx.fill(Path(line), with: .color(accent))
            let leftChevron = Path { p in
                p.move(to: CGPoint(x: 0, y: y - chevronSize))
                p.addLine(to: CGPoint(x: chevronSize, y: y))
                p.addLine(to: CGPoint(x: 0, y: y + chevronSize))
                p.closeSubpath()
            }
            let rightChevron = Path { p in
                p.move(to: CGPoint(x: size.width, y: y - chevronSize))
                p.addLine(to: CGPoint(x: size.width - chevronSize, y: y))
                p.addLine(to: CGPoint(x: size.width, y: y + chevronSize))
                p.closeSubpath()
            }
            ctx.fill(leftChevron, with: .color(accent))
            ctx.fill(rightChevron, with: .color(accent))
        case .horizontal:
            let axisLength = max(0, size.width - 2 * pad)
            let x = pad + axisLength * CGFloat(fraction)
            let halo = CGRect(x: x - haloW * 0.5, y: 0,
                              width: haloW, height: size.height)
            ctx.fill(Path(halo), with: .color(haloColor))
            let line = CGRect(x: x - coreW * 0.5, y: 0,
                              width: coreW, height: size.height)
            ctx.fill(Path(line), with: .color(accent))
            let topChevron = Path { p in
                p.move(to: CGPoint(x: x - chevronSize, y: 0))
                p.addLine(to: CGPoint(x: x, y: chevronSize))
                p.addLine(to: CGPoint(x: x + chevronSize, y: 0))
                p.closeSubpath()
            }
            let bottomChevron = Path { p in
                p.move(to: CGPoint(x: x - chevronSize, y: size.height))
                p.addLine(to: CGPoint(x: x, y: size.height - chevronSize))
                p.addLine(to: CGPoint(x: x + chevronSize, y: size.height))
                p.closeSubpath()
            }
            ctx.fill(topChevron, with: .color(accent))
            ctx.fill(bottomChevron, with: .color(accent))
        }
    }

    /// Compute the playhead's fractional position **on the same
    /// chunk grid the bars are laid out on**. This is the M10.5t
    /// fix for the "overview drifts towards the end of the song"
    /// feedback: the previous code used `elapsedSecs / durationSecs`
    /// as the playhead fraction, which is *almost* but not exactly
    /// the same as the bar grid's denominator. The bar grid is laid
    /// out on `peaksLen × chunkDurationSecs` (= `chunkCount *
    /// samples_per_chunk / track_sr`), whereas `durationSecs` is
    /// `track.frames() / track_sr`. The two differ by up to one
    /// chunk's worth of frames at the very end of the file (the
    /// offline decimator flushes a partial last chunk so the bar
    /// array covers `≥` track frames). For typical 44.1 kHz tracks
    /// the difference is sub-millisecond and bounded, but the user
    /// perceives any drift between "where the bracket sits" and
    /// "which bar represents the audible material" as an off-by-N
    /// error in the visible mapping. Computing the playhead on the
    /// *exact same grid as the bars* eliminates the entire class.
    ///
    /// Mirrors the M10.5n principle used by the main Metal
    /// waveform: convert time → chunk index via `chunkDurationSecs`
    /// (the f64 the engine reports directly), not via the
    /// round-tripped engine-SR sample count.
    ///
    /// Falls back to `elapsedSecs / durationSecs` when peak data
    /// isn't loaded yet (e.g. Thru mode, fresh deck before
    /// `reloadIfStale` finishes); returns `nil` when nothing
    /// useful can be computed.
    ///
    /// **M11d.5 round 5**: reads `engine.position(deckIdx:)`
    /// directly instead of `deckState.elapsedSecs`. The deck-state
    /// field was removed as part of the per-second-republish fix
    /// (the `TimelineView` wrapping this Canvas now drives the
    /// playhead's own update cadence). The duration fallback also
    /// reads `pos.durationSecs` instead of `deckState.durationSecs`
    /// for the same reason — keeps the fraction calculation in
    /// sync with the engine on the same tick.
    private func playheadFraction() -> Double? {
        if let dragPlayheadFraction {
            return max(0, min(1, dragPlayheadFraction))
        }
        // M11d.6 round 5 — lock-free FFI snapshot.
        let pos = model.engine.positionSnapshot(deckIdx: deckIdx)
        let elapsed = pos.elapsedSecs
        let peaksLen = model.engine.peaksLen(deckIdx: deckIdx)
        let chunkDur = model.engine.peaksChunkDurationSecs(deckIdx: deckIdx)
        if peaksLen > 0 && chunkDur > 0 {
            let totalSecs = Double(peaksLen) * chunkDur
            guard totalSecs > 0 else { return nil }
            return max(0, min(1, elapsed / totalSecs))
        }
        guard pos.durationSecs > 0 else { return nil }
        return max(0, min(1, elapsed / pos.durationSecs))
    }

    private func drawEmptyState(ctx: GraphicsContext, size: CGSize) {
        // Faint dashed midline along the strip's *time* axis so
        // the empty state reads as a container, not a missing
        // element. Runs top → bottom in vertical mode, left →
        // right in horizontal mode.
        let dash: Path
        switch orientation {
        case .vertical:
            let x = size.width * 0.5
            dash = Path { p in
                p.move(to: CGPoint(x: x, y: 0))
                p.addLine(to: CGPoint(x: x, y: size.height))
            }
        case .horizontal:
            let y = size.height * 0.5
            dash = Path { p in
                p.move(to: CGPoint(x: 0, y: y))
                p.addLine(to: CGPoint(x: size.width, y: y))
            }
        }
        ctx.stroke(
            dash,
            with: .color(DubColor.divider),
            style: StrokeStyle(lineWidth: 1, dash: [3, 4]))
    }

    /// Outer-envelope colour for the overview's peak hull. M10.5r
    /// refresh: matches the playing-waveform's deck tint at a
    /// slightly reduced saturation so the overview still reads as
    /// secondary chrome, but with the same hue as the main strip
    /// so the two pieces visually agree. The pre-M10.5r muted
    /// `deckAOverview` / `deckBOverview` tones read too brown /
    /// teal-grey to feel related to the bright deck tint.
    private func peakBarColor() -> Color {
        DubColor.deckTint(side).opacity(0.78)
    }

    /// Inner RMS-core colour. Brighter version of the peak tint to
    /// give the envelope a two-tone look identical to the Metal
    /// playing-waveform's outer / inner split. Sits *on top of* the
    /// peak fill, so its opacity stacks with the peak's — keep it
    /// near 1.0 to read clean.
    private func rmsBarColor() -> Color {
        DubColor.deckTint(side).opacity(0.95)
    }

    // MARK: - Overview scrub (click + drag)

    /// Maps a point in overview coordinates to a `[0, 1]` fraction
    /// along the padded time axis (same grid as the bars).
    private func overviewFraction(at point: CGPoint, in size: CGSize) -> Double? {
        let pad = OverviewLayout.endPadding
        switch orientation {
        case .vertical:
            let axisLength = size.height - 2 * pad
            guard axisLength > 0 else { return nil }
            let local = max(0, min(axisLength, point.y - pad))
            return Double(local / axisLength)
        case .horizontal:
            let axisLength = size.width - 2 * pad
            guard axisLength > 0 else { return nil }
            let local = max(0, min(axisLength, point.x - pad))
            return Double(local / axisLength)
        }
    }

    /// Converts an overview fraction to absolute seek seconds on
    /// the peak-chunk grid (M10.5t canonical mapping).
    private func overviewSeekSecs(for fraction: Double) -> Double? {
        guard deckState.hasTrack, deckState.durationSecs > 0 else { return nil }
        let peaksLen = model.engine.peaksLen(deckIdx: deckIdx)
        let chunkDur = model.engine.peaksChunkDurationSecs(deckIdx: deckIdx)
        if peaksLen > 0 && chunkDur > 0 {
            return fraction * Double(peaksLen) * chunkDur
        }
        return fraction * deckState.durationSecs
    }

    /// Traktor-style overview scrub. Single tap is a one-shot seek;
    /// click-and-drag continuously seeks as the finger moves.
    /// Transport state is left alone — paused stays paused, playing
    /// keeps playing from each new position.
    ///
    /// During a drag the playhead bracket follows the finger
    /// immediately via `dragPlayheadFraction`; engine seeks are
    /// throttled to ~30 Hz so fast motion doesn't flood the FFI.
    /// The final `onEnded` seek is **gated** on the cursor having
    /// actually moved past `overviewSeekDedupTolerance` since the
    /// last dispatched seek. Pre-fix the `onEnded` branch always
    /// re-issued the seek, which on a playing deck cancelled the
    /// playback advancement the click had triggered: the user saw
    /// the playhead snap forward on mouse-down (the seek), drift
    /// forward while held (normal playback), then snap *back* to
    /// the click point on mouse-up (the duplicate `onEnded`
    /// seek). Skipping the no-op final seek removes that backward
    /// yank while keeping a real drag's final-position seek
    /// unthrottled.
    private func handleOverviewScrub(
        at point: CGPoint,
        in size: CGSize,
        isFinal: Bool
    ) {
        guard let fraction = overviewFraction(at: point, in: size),
              let seekSecs = overviewSeekSecs(for: fraction)
        else {
            if isFinal {
                dragPlayheadFraction = nil
                lastSeekedFraction = nil
            }
            return
        }

        dragPlayheadFraction = fraction

        // Bug #1 — duplicate `onEnded` seek on a click-and-release
        // without drag. If we already dispatched a seek to (within
        // tolerance of) this fraction during the gesture, the
        // final-up event should NOT re-issue it; doing so on a
        // playing deck yanks the playhead back to the click point.
        if isFinal,
           let last = lastSeekedFraction,
           abs(fraction - last) < Self.overviewSeekDedupTolerance
        {
            dragPlayheadFraction = nil
            lastSeekedFraction = nil
            return
        }

        let now = ProcessInfo.processInfo.systemUptime
        let shouldSeek = isFinal
            || now - lastOverviewSeekUptime >= Self.overviewSeekMinInterval
        guard shouldSeek else { return }

        lastOverviewSeekUptime = now
        lastSeekedFraction = fraction
        model.seekDeck(side: side, absoluteSecs: seekSecs)

        if isFinal {
            dragPlayheadFraction = nil
            lastSeekedFraction = nil
        }
    }

    // MARK: - Decimation

    /// Pull peaks via FFI and decimate to `bucketCount` buckets.
    /// Idempotent given the same (`hasTrack`, `sourceURL`,
    /// generation) tuple; cheap enough to call from
    /// `.onChange(of: sourceURL)` and `.onAppear` without
    /// debouncing.
    private func reloadIfStale() {
        let currentGen = model.engine.peaksGeneration(deckIdx: deckIdx)
        // Bug #1 — any source swap clears the gesture-bookkeeping
        // anchor so the next overview click starts with a clean
        // "last dispatched seek" state. Without this, a click on
        // deck A → load track → click overview at a fraction that
        // happens to match the prior anchor would incorrectly
        // suppress the seek as a duplicate.
        lastSeekedFraction = nil
        // No track → drop any cached buckets so the empty-state
        // path renders. This also covers engine-stopped, where
        // `peaks_generation` returns 0 and `hasTrack` is false.
        guard deckState.hasTrack else {
            buckets = nil
            lastSeenGeneration = currentGen
            return
        }
        let len = model.engine.peaksLen(deckIdx: deckIdx)
        guard len > 0 else {
            buckets = []
            lastSeenGeneration = currentGen
            return
        }
        // Pull the entire broadband peak array. `peaks_extend`
        // with start_idx = 0 returns every chunk that has been
        // produced so far; for File-mode sources that's the
        // whole track (computed offline at load time per M10.5a).
        let data = model.engine.peaksExtend(deckIdx: deckIdx, startIdx: 0)
        buckets = Self.decimate(data: data, bucketCount: Self.bucketCount)
        lastSeenGeneration = currentGen
    }

    /// Pure-function decimator. Takes the FFI's packed
    /// `PeakChunk` byte buffer (12 bytes per chunk: min, max, rms
    /// — three f32 little-endian) and reduces it to `bucketCount`
    /// (`peak`, `rms`) pairs. Per-bucket `peak` is the max
    /// `max(|min|, |max|)` across its chunk range; per-bucket
    /// `rms` is the *RMS-of-RMS* (sqrt-of-mean-of-squares) across
    /// the same range, which preserves loudness when chunks are
    /// aggregated.
    fileprivate static func decimate(data: Data, bucketCount: Int) -> [OverviewBucket] {
        let stride = MemoryLayout<Float>.size * 3 // f32 × 3
        let chunkCount = data.count / stride
        guard chunkCount > 0, bucketCount > 0 else { return [] }
        var out = [OverviewBucket](
            repeating: OverviewBucket(peak: 0, rms: 0),
            count: bucketCount)
        data.withUnsafeBytes { (raw: UnsafeRawBufferPointer) in
            guard let base = raw.baseAddress else { return }
            for b in 0..<bucketCount {
                // `[start, end)` chunk indices for this bucket.
                let start = (b * chunkCount) / bucketCount
                let endRaw = ((b + 1) * chunkCount) / bucketCount
                let end = max(start + 1, endRaw)
                var peak: Float = 0
                var rmsAccum: Float = 0
                var rmsN: Int = 0
                for i in start..<min(end, chunkCount) {
                    let p = base.advanced(by: i * stride)
                        .assumingMemoryBound(to: Float.self)
                    let mn = p[0]
                    let mx = p[1]
                    let rms = p[2]
                    let a = max(abs(mn), abs(mx))
                    if a > peak { peak = a }
                    rmsAccum += rms * rms
                    rmsN += 1
                }
                let rmsAvg: Float = rmsN > 0
                    ? (rmsAccum / Float(rmsN)).squareRoot()
                    : 0
                out[b] = OverviewBucket(peak: peak, rms: rmsAvg)
            }
        }
        return out
    }
}

private extension Float {
    /// Saturating clamp to `[0, 1]` for amplitude / fraction maths.
    /// Kept on `Float` (not `BinaryFloatingPoint`) to avoid the
    /// generic-conformance cost — every caller in this file uses
    /// `Float` directly.
    var clamped01: Float {
        max(0, min(1, self))
    }
}

/// Pin the overview to its orientation-appropriate intrinsic
/// dimension: a fixed width (filling height) in vertical mode, a
/// fixed height (filling width) in horizontal Prep-mode mode.
private struct OverviewSizing: ViewModifier {
    let orientation: WaveformOrientation
    func body(content: Content) -> some View {
        switch orientation {
        case .vertical:
            content
                .frame(width: DubLayout.deckOverviewWidth)
                .frame(maxHeight: .infinity)
        case .horizontal:
            content
                .frame(height: DubLayout.deckOverviewHeight)
                .frame(maxWidth: .infinity)
        }
    }
}

#Preview {
    TrackOverviewView(model: WaveformAppModel(), side: .a, deckIdx: 0)
        .frame(width: DubLayout.deckOverviewWidth, height: 600)
}
