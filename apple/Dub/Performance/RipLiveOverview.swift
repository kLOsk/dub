//
//  RipLiveOverview.swift
//  Dub
//
//  M26a — the live capture envelope shown in the Prep overview-band
//  slot while a rip records. A horizontal Canvas fed by incremental
//  `DubRipSession.envelopeExtend(startIdx:)` pulls on a ~1 Hz
//  TimelineView: the packed 12-byte (min, max, rms) chunks are the
//  same wire format as `peaksExtend`, so the accumulated buffer is
//  re-decimated to 480 buckets with the shared `OverviewDecimator`.
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
struct RipLiveOverview: View {

    /// `envelopeExtend(startIdx:)` — takes the number of chunks
    /// already fetched, returns the packed bytes for the rest.
    let fetchEnvelope: (UInt64) -> Data

    /// Bucket cap, matching `TrackOverviewView`'s overview.
    private static let bucketCount = 480

    @State private var raw = Data()
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
        let stride = MemoryLayout<Float>.size * 3
        let fetched = UInt64(raw.count / stride)
        let more = fetchEnvelope(fetched)
        guard !more.isEmpty || buckets.isEmpty else { return }
        raw.append(more)
        buckets = OverviewDecimator.decimate(data: raw, bucketCount: Self.bucketCount)
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
