import Foundation
import XCTest

@testable import Dub

/// The live capture overview's accumulator.
///
/// The thing it exists to prevent: the overview used to keep every
/// peak chunk and re-decimate the whole buffer on the main thread
/// every second. A chunk is 64 samples, so a side arrives at ~750 a
/// second and the per-tick cost grew for as long as the record played
/// — "the UI was very laggy and the overview updated every 2 seconds"
/// (2026-09-22). Tiling makes the per-tick work bounded.
final class RipEnvelopeTilerTests: XCTestCase {

    /// Pack chunks the way `envelopeExtend` does: `(min, max, rms)`.
    private func payload(_ chunks: [(Float, Float, Float)]) -> Data {
        var out = Data(capacity: chunks.count * 12)
        for c in chunks {
            for v in [c.0, c.1, c.2] {
                withUnsafeBytes(of: v) { out.append(contentsOf: $0) }
            }
        }
        return out
    }

    private func flat(_ n: Int, amp: Float) -> Data {
        payload((0..<n).map { _ in (-amp, amp, amp * 0.7) })
    }

    /// The start index it asks for next is the count it has consumed —
    /// get this wrong and the envelope either doubles up or stalls.
    func testItTracksWhatItHasConsumed() {
        var t = RipEnvelopeTiler()
        XCTAssertEqual(t.chunksFetched, 0)
        t.feed(flat(300, amp: 0.5))
        XCTAssertEqual(t.chunksFetched, 300)
        t.feed(flat(120, amp: 0.5))
        XCTAssertEqual(t.chunksFetched, 420)
    }

    /// Chunks arriving split across fetches land in the same tiles as
    /// chunks arriving whole — a tile boundary is a position in the
    /// stream, not an artefact of when the poll fired.
    func testTilingIsIndependentOfHowThePayloadIsSplit() {
        let n = RipEnvelopeTiler.tileChunks * 3 + 17
        var whole = RipEnvelopeTiler()
        whole.feed(flat(n, amp: 0.4))

        var split = RipEnvelopeTiler()
        var fed = 0
        for piece in [7, 61, 128, 3, 200] where fed < n {
            let take = min(piece, n - fed)
            split.feed(flat(take, amp: 0.4))
            fed += take
        }
        split.feed(flat(n - fed, amp: 0.4))

        XCTAssertEqual(whole.tiles.count, split.tiles.count)
        for (a, b) in zip(whole.buckets(count: 64), split.buckets(count: 64)) {
            XCTAssertEqual(a.peak, b.peak, accuracy: 1e-6)
            XCTAssertEqual(a.rms, b.rms, accuracy: 1e-6)
        }
    }

    /// The level survives the reduction: a loud stretch reads loud
    /// after tiling *and* after bucketing, or the live shape lies.
    func testTheShapeSurvivesBothReductions() {
        var t = RipEnvelopeTiler()
        t.feed(flat(RipEnvelopeTiler.tileChunks * 4, amp: 0.2))
        t.feed(flat(RipEnvelopeTiler.tileChunks * 4, amp: 0.9))
        let b = t.buckets(count: 8)
        XCTAssertEqual(b.count, 8)
        XCTAssertEqual(b.first?.peak ?? 0, 0.2, accuracy: 1e-5)
        XCTAssertEqual(b.last?.peak ?? 0, 0.9, accuracy: 1e-5)
    }

    /// The tile still filling is drawn too, so the leading edge moves
    /// every tick instead of stepping once a tile.
    func testThePartialTileIsVisible() {
        var t = RipEnvelopeTiler()
        t.feed(flat(RipEnvelopeTiler.tileChunks / 4, amp: 0.6))
        XCTAssertTrue(t.tiles.isEmpty, "a quarter tile is not a tile yet")
        let b = t.buckets(count: 4)
        XCTAssertFalse(b.isEmpty, "the partial tile has to draw or the edge freezes")
        XCTAssertEqual(b.first?.peak ?? 0, 0.6, accuracy: 1e-5)
    }

    /// Fewer tiles than buckets: the buckets are the tiles, not a
    /// stretched or empty list.
    func testFewerTilesThanBucketsPassThrough() {
        var t = RipEnvelopeTiler()
        t.feed(flat(RipEnvelopeTiler.tileChunks * 3, amp: 0.5))
        XCTAssertEqual(t.buckets(count: 480).count, 3)
    }

    func testEmptyStaysEmpty() {
        var t = RipEnvelopeTiler()
        XCTAssertTrue(t.buckets(count: 480).isEmpty)
        t.feed(Data())
        XCTAssertEqual(t.chunksFetched, 0)
        XCTAssertTrue(t.buckets(count: 480).isEmpty)
    }

    /// The point of the whole thing: a long side reduces to a bounded
    /// number of tiles, so the per-tick pass does not grow with the
    /// recording. Forty minutes at 48 kHz is the session cap.
    func testALongSideStaysBounded() {
        let chunks = Int(40 * 60 * 48_000 / 64)
        let tiles = chunks / RipEnvelopeTiler.tileChunks
        XCTAssertLessThan(
            tiles, 20_000,
            "a whole side must reduce to a few thousand tiles, not \(tiles)")
        // And a tile still has to be finer than a bucket, or the live
        // shape is coarser than the picture it is drawn into.
        XCTAssertGreaterThan(tiles, 480)
    }
}
