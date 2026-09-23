import XCTest

@testable import Dub

/// The waveform ring's delta ingest (2026-09-23).
///
/// Each frame the renderer keeps a window of chunks around the playhead
/// in a GPU ring. It used to re-copy the whole window whenever the
/// playhead crossed a chunk — for the colour bands ~87 s of data a
/// frame, 79 % of the render thread's time in rip review. Now it copies
/// only what the buffer does not hold. The failure mode to guard
/// against is not slowness but a *wrong picture*: a slot credited with
/// a chunk it does not hold draws stale waveform, and nothing crashes.
/// So these pin the arithmetic that decides what is held.
final class PeakRingCoverageTests: XCTestCase {

    private let cap: UInt64 = 131_072

    private func cov(_ lo: UInt64, _ hi: UInt64) -> PeakRingCoverage {
        PeakRingCoverage(lo: lo, hi: hi)
    }

    /// Nothing held: the whole window, once.
    func testAnEmptyBufferFetchesTheWholeWindow() {
        let p = PeakRingCoverage().plan(window: 100..<8292, capacity: cap)
        XCTAssertEqual(p.fetch, [100..<8292])
        XCTAssertEqual(p.after, cov(100, 8292))
    }

    /// Steady play, the case this exists for: the window slides one
    /// chunk and one chunk is fetched, not 8 192.
    func testSteadyPlayFetchesOnlyTheLeadingEdge() {
        let p = cov(100, 8292).plan(window: 101..<8293, capacity: cap)
        XCTAssertEqual(p.fetch, [8292..<8293])
        // Coverage keeps what is behind: the chunk the window left is
        // still valid, and a rewind to it costs nothing.
        XCTAssertEqual(p.after, cov(100, 8293))
    }

    /// Nothing moved: nothing fetched.
    func testAStillWindowFetchesNothing() {
        let p = cov(0, 9000).plan(window: 100..<8292, capacity: cap)
        XCTAssertEqual(p.fetch, [])
        XCTAssertEqual(p.after, cov(0, 9000))
    }

    /// A rewind or a scratch back past what was held fetches the
    /// trailing edge, the mirror of steady play.
    func testGoingBackwardFetchesTheTrailingEdge() {
        let p = cov(5000, 13192).plan(window: 4990..<13182, capacity: cap)
        XCTAssertEqual(p.fetch, [4990..<5000])
        XCTAssertEqual(p.after, cov(4990, 13192))
    }

    /// A window wider than what is held on both sides — the first
    /// frame after the window widened — fetches both edges and
    /// nothing in between.
    func testAWindowPastBothEndsFetchesBothEdges() {
        let p = cov(1000, 2000).plan(window: 500..<2500, capacity: cap)
        XCTAssertEqual(p.fetch, [500..<1000, 2000..<2500])
        XCTAssertEqual(p.after, cov(500, 2500))
    }

    /// A seek somewhere else entirely: the whole window, and the old
    /// coverage is dropped rather than merged across the gap — the
    /// chunks in the gap were never fetched.
    func testASeekFetchesTheWholeWindowAndForgetsTheRest() {
        let p = cov(0, 8192).plan(window: 50_000..<58_192, capacity: cap)
        XCTAssertEqual(p.fetch, [50_000..<58_192])
        XCTAssertEqual(p.after, cov(50_000, 58_192))
    }

    /// Adjacent is not a gap: a window that starts exactly where the
    /// coverage ends extends it.
    func testAnAdjacentWindowExtendsRatherThanRestarts() {
        let p = cov(0, 8192).plan(window: 8192..<16_384, capacity: cap)
        XCTAssertEqual(p.fetch, [8192..<16_384])
        XCTAssertEqual(p.after, cov(0, 16_384))
    }

    /// Past the ring's capacity the far end's slots have been reused
    /// for new chunks, so claiming them would draw the wrong audio.
    /// The plan fetches the window whole and starts again — one full
    /// copy per ring's worth of playback (~23 min of bands).
    func testCoverageNeverExceedsTheRing() {
        let p = cov(0, cap).plan(window: 1..<cap + 1, capacity: cap)
        XCTAssertEqual(p.fetch, [1..<cap + 1])
        XCTAssertEqual(p.after, cov(1, cap + 1))
        XCTAssertLessThanOrEqual(p.after.hi - p.after.lo, cap)
    }

    /// A long play-through, frame by frame: coverage is always the
    /// window's superset, never wider than the ring, and the total
    /// fetched is the audio played plus one window — not a window per
    /// frame.
    func testALongPlayThroughFetchesEachChunkOnce() {
        let window: UInt64 = 8192
        var held = PeakRingCoverage()
        var fetched: UInt64 = 0
        let frames: UInt64 = 20_000
        for ph in 0..<frames {
            let start = ph > window / 2 ? ph - window / 2 : 0
            let w = start..<(start + window)
            let p = held.plan(window: w, capacity: cap)
            fetched += p.fetch.reduce(0) { $0 + UInt64($1.count) }
            held = p.after
            XCTAssertLessThanOrEqual(held.lo, w.lowerBound)
            XCTAssertGreaterThanOrEqual(held.hi, w.upperBound)
            XCTAssertLessThanOrEqual(held.hi - held.lo, cap)
        }
        let played = frames - window / 2 + window
        XCTAssertLessThanOrEqual(fetched, played + window,
            "fetched \(fetched) chunks for \(played) of audio")
    }

    /// An empty window changes nothing.
    func testAnEmptyWindowIsANoOp() {
        let p = cov(10, 20).plan(window: 30..<30, capacity: cap)
        XCTAssertEqual(p.fetch, [])
        XCTAssertEqual(p.after, cov(10, 20))
    }
}
