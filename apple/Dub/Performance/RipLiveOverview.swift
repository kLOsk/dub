//
//  RipLiveOverview.swift
//  Dub
//
//  M26a — the live capture envelope shown in the Prep overview-band
//  slot while a rip records. A horizontal Canvas fed by incremental
//  `DubRipSession.envelopeExtend(startIdx:)` pulls on a ~1 Hz
//  TimelineView: the packed 12-byte (min, max, rms) chunks are the
//  same wire format as `peaksExtend`.
//
//  **The accumulation is tiled, not kept raw.** It used to hold every
//  chunk and re-decimate the whole buffer on every tick, on the main
//  thread. A chunk is 64 samples, so a side arrives at 750 of them a
//  second: ten minutes in, each tick walked 450 000 chunks and a ~5 MB
//  `Data` grew under it, and the cost rose for as long as the record
//  played. That is the recording session where "the UI was very laggy
//  and the overview updated every 2 seconds" (2026-09-22). Incoming
//  chunks are folded into fixed-length tiles once, and only the tiles
//  — a few thousand for a whole side — are re-bucketed per tick.
//
//  Split into a live driver (`RipLiveOverview`, owns the fetch +
//  accumulation) and a pure Canvas (`RipLiveOverviewCanvas`) so the
//  drawing is snapshot-testable from synthetic buckets. The driver
//  takes a fetch closure — not the FFI session — for the same
//  reason.
//

import SwiftUI

/// Pure horizontal envelope canvas — the live twin of
/// `TrackOverviewView`'s horizontal energy map, deliberately built
/// on the same bucket model + end padding so what the DJ watches
/// grow during capture is the shape they get in review.
struct RipLiveOverviewCanvas: View {

    let buckets: [OverviewBucket]

    /// Same RMS-vs-peak blend + headroom as `TrackOverviewView`, so
    /// the live shape matches the review shape.
    private static let energyRmsWeight: Float = 0.78
    private static let energyHeadroom: Float = 0.9

    var body: some View {
        Canvas { ctx, size in
            drawBackground(ctx: ctx, size: size)
            if buckets.isEmpty {
                drawEmptyState(ctx: ctx, size: size)
            } else {
                drawEnvelope(ctx: ctx, size: size)
            }
        }
    }

    private func drawBackground(ctx: GraphicsContext, size: CGSize) {
        let rect = CGRect(origin: .zero, size: size)
        ctx.fill(Path(rect), with: .color(DubColor.surface1))
        let seam = CGRect(x: 0, y: size.height - 1, width: size.width, height: 1)
        ctx.fill(Path(seam), with: .color(DubColor.divider))
    }

    private func drawEmptyState(ctx: GraphicsContext, size: CGSize) {
        let y = size.height * 0.5
        let dash = Path { p in
            p.move(to: CGPoint(x: 0, y: y))
            p.addLine(to: CGPoint(x: size.width, y: y))
        }
        ctx.stroke(
            dash,
            with: .color(DubColor.divider),
            style: StrokeStyle(lineWidth: 1, dash: [3, 4]))
    }

    private func drawEnvelope(ctx: GraphicsContext, size: CGSize) {
        let n = buckets.count
        guard n > 0 else { return }
        let pad = OverviewLayout.endPadding

        var energy = [Float](repeating: 0, count: n)
        var maxEnergy: Float = 1e-6
        for (i, b) in buckets.enumerated() {
            let e = (1 - Self.energyRmsWeight) * b.peak + Self.energyRmsWeight * b.rms
            energy[i] = e
            if e > maxEnergy { maxEnergy = e }
        }
        let norm = Self.energyHeadroom / maxEnergy

        let axisStart = pad
        let axisLength = max(0, size.width - 2 * pad)
        let fullH = max(0, size.height - 3)
        let baseline = size.height

        func topY(_ i: Int) -> CGFloat {
            let h = CGFloat(min(1, max(0, energy[i] * norm)))
            return baseline - max(1, h * fullH)
        }

        var fill = Path()
        fill.move(to: CGPoint(x: axisStart, y: baseline))
        for i in 0..<n {
            let x = axisStart + axisLength * CGFloat(i) / CGFloat(n)
            fill.addLine(to: CGPoint(x: x, y: topY(i)))
        }
        let xEnd = axisStart + axisLength
        fill.addLine(to: CGPoint(x: xEnd, y: topY(n - 1)))
        fill.addLine(to: CGPoint(x: xEnd, y: baseline))
        fill.closeSubpath()

        var contour = Path()
        contour.move(to: CGPoint(x: axisStart, y: topY(0)))
        for i in 0..<n {
            let x = axisStart + axisLength * CGFloat(i) / CGFloat(n)
            contour.addLine(to: CGPoint(x: x, y: topY(i)))
        }
        contour.addLine(to: CGPoint(x: xEnd, y: topY(n - 1)))

        let tint = DubColor.deckATint
        ctx.fill(fill, with: .linearGradient(
            Gradient(colors: [tint.opacity(0.95), tint.opacity(0.5)]),
            startPoint: CGPoint(x: 0, y: size.height),
            endPoint: CGPoint(x: 0, y: 0)))
        ctx.stroke(contour, with: .color(tint), lineWidth: 1.25)

        // Recording head: a thin live-red edge at the growth front so
        // the band reads as "recording", not a finished overview.
        let head = CGRect(x: min(xEnd, axisStart + axisLength) - 1, y: 0,
                          width: 2, height: size.height)
        ctx.fill(Path(head), with: .color(DubColor.stateError.opacity(0.9)))
    }
}

/// Live driver: accumulates the packed envelope chunks and
/// re-decimates on a ~1 Hz cadence while mounted.
/// Folds the capture's packed peak chunks into fixed-length tiles as
/// they arrive, and hands out a bucket list on demand.
///
/// **Why tiles.** The bucket boundaries move as the side grows — 480
/// buckets over ten minutes is not 480 buckets over twenty — so the
/// bucket pass cannot be incremental. The *tiles* can: a tile is a
/// fixed number of chunks, so a chunk lands in exactly one tile and
/// never moves. Per tick the work is then the new chunks (a second's
/// worth) plus a walk over the tiles, instead of a walk over every
/// chunk since the needle dropped.
///
/// `tileChunks` is the whole trade-off. At 64 samples a chunk and
/// 48 kHz, 128 chunks is ~0.17 s — finer than a bucket stays until a
/// side runs past ~80 minutes, which is longer than any side and
/// longer than the session cap. Coarser tiles would start visibly
/// flattening transients in the live shape.
struct RipEnvelopeTiler {

    /// Chunks per tile. See the note above before changing it.
    static let tileChunks = 128
    /// The packed wire stride: `(min, max, rms)` as three `Float`s.
    static let chunkStride = MemoryLayout<Float>.size * 3

    private(set) var tiles: [OverviewBucket] = []
    /// Chunks consumed so far — what `envelopeExtend` wants as its
    /// start index.
    private(set) var chunksFetched: UInt64 = 0
    /// The tile still filling: its running peak, its summed squares
    /// and how many chunks are in it.
    private var partialPeak: Float = 0
    private var partialRmsSq: Float = 0
    private var partialCount = 0

    /// Fold a fresh `envelopeExtend` payload in. Whole tiles are
    /// published; the remainder stays pending until the chunks that
    /// complete it arrive, so a tile is never published twice.
    mutating func feed(_ data: Data) {
        let count = data.count / Self.chunkStride
        guard count > 0 else { return }
        chunksFetched &+= UInt64(count)
        data.withUnsafeBytes { (raw: UnsafeRawBufferPointer) in
            guard let base = raw.baseAddress else { return }
            for i in 0..<count {
                let p = base.advanced(by: i * Self.chunkStride)
                    .assumingMemoryBound(to: Float.self)
                let amp = max(abs(p[0]), abs(p[1]))
                let rms = p[2]
                if amp > partialPeak { partialPeak = amp }
                partialRmsSq += rms * rms
                partialCount += 1
                if partialCount == Self.tileChunks { closeTile() }
            }
        }
    }

    private mutating func closeTile() {
        guard partialCount > 0 else { return }
        tiles.append(
            OverviewBucket(
                peak: partialPeak,
                rms: (partialRmsSq / Float(partialCount)).squareRoot()))
        partialPeak = 0
        partialRmsSq = 0
        partialCount = 0
    }

    /// The live shape: the closed tiles plus whatever is in the tile
    /// still filling, reduced to `count` buckets. Including the
    /// partial one is what keeps the leading edge moving between tile
    /// boundaries instead of stepping a tile at a time.
    func buckets(count: Int) -> [OverviewBucket] {
        guard count > 0 else { return [] }
        var src = tiles
        if partialCount > 0 {
            src.append(
                OverviewBucket(
                    peak: partialPeak,
                    rms: (partialRmsSq / Float(partialCount)).squareRoot()))
        }
        guard !src.isEmpty else { return [] }
        guard src.count > count else { return src }
        var out = [OverviewBucket](repeating: OverviewBucket(peak: 0, rms: 0), count: count)
        for b in 0..<count {
            let start = (b * src.count) / count
            let end = max(start + 1, ((b + 1) * src.count) / count)
            var peak: Float = 0
            var rmsSq: Float = 0
            var n = 0
            for i in start..<min(end, src.count) {
                if src[i].peak > peak { peak = src[i].peak }
                rmsSq += src[i].rms * src[i].rms
                n += 1
            }
            out[b] = OverviewBucket(
                peak: peak, rms: n > 0 ? (rmsSq / Float(n)).squareRoot() : 0)
        }
        return out
    }
}

struct RipLiveOverview: View {

    /// `envelopeExtend(startIdx:)` — takes the number of chunks
    /// already fetched, returns the packed bytes for the rest.
    let fetchEnvelope: (UInt64) -> Data

    /// Bucket cap, matching `TrackOverviewView`'s overview.
    private static let bucketCount = 480

    @State private var tiler = RipEnvelopeTiler()
    @State private var buckets: [OverviewBucket] = []

    var body: some View {
        TimelineView(.periodic(from: .now, by: 1.0)) { context in
            RipLiveOverviewCanvas(buckets: buckets)
                .onChange(of: context.date) { _ in fetchMore() }
        }
        .onAppear(perform: fetchMore)
        .frame(height: DubLayout.deckOverviewHeight)
        .frame(maxWidth: .infinity)
    }

    private func fetchMore() {
        let more = fetchEnvelope(tiler.chunksFetched)
        guard !more.isEmpty || buckets.isEmpty else { return }
        tiler.feed(more)
        buckets = tiler.buckets(count: Self.bucketCount)
    }
}

#Preview("live envelope") {
    let buckets = (0..<480).map { i -> OverviewBucket in
        let t = Float(i) / 480
        let a = 0.3 + 0.55 * abs(sin(t * 27)) * (0.4 + 0.6 * t)
        return OverviewBucket(peak: a, rms: a * 0.7)
    }
    return RipLiveOverviewCanvas(buckets: buckets)
        .frame(width: 900, height: DubLayout.deckOverviewHeight)
}
