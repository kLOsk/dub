import XCTest

@testable import Dub

/// A THRU deck's BPM: the record's tempo as the tracker hears it, unless
/// the DJ tapped one — which holds for that record and gives way when the
/// needle comes up and the next one goes down (Daniel, 2026-11-11).
final class ThruTempoTests: XCTestCase {

    private func thruDeck() -> DeckState {
        var deck = DeckState()
        deck.controlMode = 2
        return deck
    }

    func testOffThruThereIsNoThruTempo() {
        var deck = DeckState()
        deck.controlMode = 1
        deck.thruBpm = 92.5
        XCTAssertNil(deck.thruReadout)
    }

    /// Listening, then a tentative number, then a locked one.
    func testTheTrackerReadsTentativeThenLocked() {
        var deck = thruDeck()
        XCTAssertEqual(deck.thruReadout, ThruTempoReadout(bpm: nil))
        deck.thruBpm = 133.4
        XCTAssertEqual(deck.thruReadout, ThruTempoReadout(bpm: 133.4, locked: false))
        deck.thruLocked = true
        XCTAssertEqual(deck.thruReadout, ThruTempoReadout(bpm: 133.4, locked: true))
    }

    /// The tracker read a reggae record at double time; the DJ taps the
    /// half. The tap wins while that record plays, whatever the tracker
    /// keeps saying.
    func testATapWinsForTheRecordItWasTappedOn() {
        var deck = thruDeck()
        deck.thruBpm = 133.4
        deck.thruLocked = true
        deck.thruRecordSeq = 3
        deck.thruTap = DeckState.ThruTap(bpm: 66.7, recordSeq: 3)
        XCTAssertEqual(deck.thruReadout, ThruTempoReadout(bpm: 66.7, tapped: true, locked: true))
        XCTAssertEqual(deck.thruReadout?.tempo, 66.7)
    }

    /// The needle comes up, the next record goes down: the tracker counts
    /// a new record, and the old tap no longer speaks for it.
    func testTheNextRecordRetiresTheTap() {
        var deck = thruDeck()
        deck.thruRecordSeq = 4
        deck.thruTap = DeckState.ThruTap(bpm: 66.7, recordSeq: 3)
        deck.thruBpm = 174.0
        XCTAssertEqual(deck.thruReadout, ThruTempoReadout(bpm: 174.0, tapped: false, locked: false))
    }

    /// Four taps half a second apart are 120.
    func testFourTapsMakeATempo() {
        XCTAssertEqual(FxRackControls.tappedBpm([0, 0.5, 1.0, 1.5]) ?? 0, 120, accuracy: 0.01)
        XCTAssertNil(FxRackControls.tappedBpm([0]))
    }

    /// Right-click ×1.5 on a drum & bass record read at two-thirds: the
    /// readout follows the tracker, scaled, and says so.
    func testAScaleRidesOnTheHeardTempo() {
        var deck = thruDeck()
        deck.thruBpm = 116.6
        deck.thruLocked = true
        deck.thruRecordSeq = 2
        deck.thruScale = DeckState.ThruScale(factor: 1.5, recordSeq: 2)
        XCTAssertEqual(deck.thruReadout?.bpm ?? 0, 174.9, accuracy: 0.001)
        XCTAssertEqual(deck.thruReadout?.label, "BPM · ×1.5")
        XCTAssertEqual(deck.thruReadout?.overridden, true)
        deck.thruBpm = 116.8
        XCTAssertEqual(deck.thruReadout?.bpm ?? 0, 175.2, accuracy: 0.001)
    }

    /// The next record retires the scale, as it does a tap.
    func testTheNextRecordRetiresTheScale() {
        var deck = thruDeck()
        deck.thruBpm = 90
        deck.thruRecordSeq = 3
        deck.thruScale = DeckState.ThruScale(factor: 2, recordSeq: 2)
        XCTAssertEqual(deck.thruReadout, ThruTempoReadout(bpm: 90))
        XCTAssertEqual(deck.thruReadout?.label, "BPM")
    }

    /// A tap wins over a scale for the same record.
    func testATapWinsOverAScale() {
        var deck = thruDeck()
        deck.thruBpm = 133.4
        deck.thruScale = DeckState.ThruScale(factor: 0.5, recordSeq: 0)
        deck.thruTap = DeckState.ThruTap(bpm: 70, recordSeq: 0)
        XCTAssertEqual(deck.thruReadout?.label, "BPM · TAP")
        XCTAssertEqual(deck.thruReadout?.tempo, 70)
    }

    func testScalesReadAsTheDJChoseThem() {
        XCTAssertEqual(ThruTempoReadout.scaleText(2), "×2")
        XCTAssertEqual(ThruTempoReadout.scaleText(0.5), "÷2")
        XCTAssertEqual(ThruTempoReadout.scaleText(1.5), "×1.5")
        XCTAssertEqual(ThruTempoReadout.scaleText(2.0 / 3), "×⅔")
        XCTAssertEqual(ThruTempoReadout.scaleText(4), "×4")
        XCTAssertEqual(ThruTempoReadout.scaleText(1.25), "×1.25")
    }

    /// The predicted beats run on from the heard one at the period, and
    /// only those inside the window are drawn.
    func testPredictedBeatsCountOnFromTheHeardOne() {
        let p = WaveformRenderer.PredictedBeats(lastBeatSecs: 10.0, periodSecs: 0.5)
        XCTAssertEqual(p.beats(from: 9.2, to: 11.1), [9.5, 10.0, 10.5, 11.0])
        XCTAssertEqual(p.beats(from: 11.1, to: 9.0), [])
        XCTAssertEqual(
            WaveformRenderer.PredictedBeats(lastBeatSecs: 0, periodSecs: 0).beats(from: 0, to: 10), [])
        XCTAssertEqual(p.beats(from: 0, to: 1_000).count, 128)
    }

    func testTheRecordClockReadsMinutesAndSeconds() {
        XCTAssertEqual(ThruRecordClock.text(secs: -1), "--:--")
        XCTAssertEqual(ThruRecordClock.text(secs: 0), "00:00")
        XCTAssertEqual(ThruRecordClock.text(secs: 134.9), "02:14")
        XCTAssertEqual(ThruRecordClock.text(secs: .nan), "--:--")
    }

    /// The side's axis: six minutes, then a minute at a time past it.
    func testTheSideAxisStartsAtSixMinutesAndGrows() {
        XCTAssertEqual(ThruSideOverviewView.axisSecs(forElapsed: 0), 360)
        XCTAssertEqual(ThruSideOverviewView.axisSecs(forElapsed: 300), 360)
        XCTAssertEqual(ThruSideOverviewView.axisSecs(forElapsed: 340), 420)
        XCTAssertEqual(ThruSideOverviewView.axisSecs(forElapsed: 1_200), 1_260)
    }

    /// The meters' scale: −48 dBFS at the left edge, full scale at the right.
    func testTheMeterScaleRunsFromMinus48ToZero() {
        XCTAssertEqual(ThruMeterPairView.fraction(0), 0)
        XCTAssertEqual(ThruMeterPairView.fraction(1), 1, accuracy: 1e-6)
        XCTAssertEqual(ThruMeterPairView.fraction(pow(10, -24.0 / 20)), 0.5, accuracy: 1e-3)
        XCTAssertEqual(ThruMeterPairView.fraction(pow(10, -60.0 / 20)), 0)
    }

    /// The ghost draws the bar just heard forward: a future chunk reads
    /// the same place one (or two) bars back, inside what was fetched.
    func testTheGhostRepeatsTheLastBar() {
        // Chunks 1000…1099 heard; a "bar" of 40 chunks; a hit at 1090.
        var amps = [Float](repeating: 0.1, count: 100)
        amps[90] = 0.9
        let ghost = WaveformRenderer.ThruGhost(startChunk: 1_000, amps: amps, barChunks: 40)
        // One bar on from the hit: 1130 reads 1090.
        XCTAssertEqual(ghost.amplitude(atChunk: 1_130, span: 1), 0.9)
        // Two bars on: 1170 reads 1090 too.
        XCTAssertEqual(ghost.amplitude(atChunk: 1_170, span: 1), 0.9)
        // Between hits it reads the quiet.
        XCTAssertEqual(ghost.amplitude(atChunk: 1_150, span: 1), 0.1)
        // The loudest of the span.
        XCTAssertEqual(ghost.amplitude(atChunk: 1_129, span: 2), 0.9)
        XCTAssertNil(WaveformRenderer.ThruGhost(startChunk: 0, amps: [], barChunks: 40)
            .amplitude(atChunk: 10, span: 1))
    }
}
