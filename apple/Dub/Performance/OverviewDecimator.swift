//
//  OverviewDecimator.swift
//  Dub
//
//  M26a — the whole-track overview decimator, extracted from
//  `TrackOverviewView` so the vinyl-rip live overview
//  (`RipLiveOverview`) can reuse it instead of duplicating the
//  packed-chunk parsing. `DubRipSession.envelopeExtend` returns the
//  exact same 12-byte little-endian `(min, max, rms)` f32 wire
//  format as `DubEngine.peaksExtend`, so one decimator serves both.
//
//  Pure functions + a value struct — no FFI, no view state — so the
//  rip snapshot suite can feed it synthetic buffers.
//

import Foundation

/// One decimated value per overview bucket: the broadband `peak` +
/// `rms` shape. The energy-map envelope (see `TrackOverviewView`'s
/// `drawBars`) blends them (mostly RMS) so the loud/quiet structure
/// reads — a breakdown dips, a drop rises — without a limited master
/// saturating into a block.
struct OverviewBucket: Equatable {
    /// Outer envelope amplitude — `max(|min|, |max|)` across the
    /// bucket's chunk range, clamped to `[0, 1]`.
    var peak: Float
    /// Inner RMS — averaged over the bucket's chunk range, also
    /// clamped to `[0, 1]`. Always `<= peak` by construction.
    var rms: Float
}

enum OverviewDecimator {

    /// Pure-function decimator. Takes the FFI's packed broadband
    /// `PeakChunk` buffer (12 bytes: min, max, rms — three f32 little-
    /// endian) and reduces it to `bucketCount` `OverviewBucket`s.
    /// Per-bucket `peak` is `max(|min|, |max|)` across the chunk range;
    /// `rms` is the RMS-of-RMS over the same range.
    static func decimate(data: Data, bucketCount: Int) -> [OverviewBucket] {
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
