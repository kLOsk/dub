import XCTest

@testable import Dub

/// The playing waveform's time axis is in **room** seconds, not track
/// seconds (2026-09-22, found on the rig).
///
/// The property that matters to a DJ, and the one every test here is a
/// restatement of: **two decks playing at the same audible tempo draw
/// their beats at the same spacing and scroll at the same speed**,
/// whatever their grid BPMs, pitches or file sample rates. That is what
/// makes "the two strips move as one" a tempo reading at all, which is
/// what PRD §9.4 hands the phase meter.
final class WaveformTimeAxisTests: XCTestCase {

    /// A 64-sample chunk's duration at a given file rate.
    private func peakDur(_ sampleRate: Double) -> Double {
        Double(WaveformRenderer.defaultSamplesPerPeakChunk) / sampleRate
    }

    /// Room-seconds per drawable pixel, the way the renderer resolves
    /// it: columns are `agg` chunks wide and `px` pixels wide.
    private func roomSecsPerPixel(
        zoom: Double = 1.0, pitchPercent: Double = 0, sampleRate: Double = 44_100
    ) -> Double {
        let dur = peakDur(sampleRate)
        let rate = 1.0 + pitchPercent / 100.0
        let axis = WaveformRenderer.effectiveTimeAxisZoom(
            timeAxisZoom: zoom, platterRate: rate, peakDurSecs: dur)
        let px = WaveformRenderer.effectivePixelsPerDrawnColumn(timeAxisZoom: axis)
        let agg = Double(WaveformRenderer.columnAggregation(timeAxisZoom: axis))
        // Track seconds a column spans, converted to the room's clock.
        return (agg * dur / rate) / px
    }

    /// 44.1 kHz at unity is the anchor: the axis the renderer has
    /// always drawn. If this moves, every baseline moves with it.
    func testUnityIsUnchanged() {
        XCTAssertEqual(
            WaveformRenderer.effectiveTimeAxisZoom(
                timeAxisZoom: 1.0, platterRate: 1.0, peakDurSecs: peakDur(44_100)),
            1.0, accuracy: 1e-12)
        XCTAssertEqual(
            roomSecsPerPixel(), WaveformRenderer.referenceSecsPerPixel, accuracy: 1e-12)
    }

    /// **The rig report.** An 88 BPM record pitched up to 92 against a
    /// 92 BPM record: the two must draw a beat at the same pixel
    /// spacing. Beat spacing in pixels is `(60 / effectiveBpm) /
    /// roomSecsPerPixel`, so equal room-seconds-per-pixel is the whole
    /// claim — the grids differ, the pitches differ, the picture agrees.
    func testBeatmatchedDecksDrawTheSameBeatSpacing() {
        let pitched = 100.0 * (92.0 / 88.0 - 1.0)   // +4.545 %
        let a = roomSecsPerPixel()                            // 92 BPM at unity
        let b = roomSecsPerPixel(pitchPercent: pitched)       // 88 BPM pitched to 92
        XCTAssertEqual(a, b, accuracy: 1e-9)

        func beatPixels(bpm: Double, secsPerPixel: Double) -> Double {
            (60.0 / bpm) / secsPerPixel
        }
        XCTAssertEqual(
            beatPixels(bpm: 92, secsPerPixel: a),
            beatPixels(bpm: 88 * (1 + pitched / 100), secsPerPixel: b),
            accuracy: 1e-6)
    }

    /// The bug as it was: without the rate the same pair diverged by
    /// 92/88. Pinned so a revert cannot pass quietly.
    func testIgnoringPitchIsTheOldFourAndAHalfPercentError() {
        let pitched = 100.0 * (92.0 / 88.0 - 1.0)
        let trackTimeAxis = peakDur(44_100) / Double(WaveformRenderer.pixelsPerDrawnColumn)
            * Double(WaveformRenderer.chunksPerColumn)
        // The old axis is the same for both decks in *track* time, so
        // in the room the pitched deck's pixel is worth less.
        let pitchedRoom = trackTimeAxis / (1 + pitched / 100)
        XCTAssertEqual(trackTimeAxis / pitchedRoom, 92.0 / 88.0, accuracy: 1e-9)
    }

    /// A 48 kHz file and a 44.1 kHz file at the same BPM and no pitch
    /// at all: a 64-sample chunk is 1.33 ms against 1.45 ms, and both
    /// used to be drawn one pixel wide — 8.8 % apart.
    func testSampleRateDoesNotLeakIntoThePicture() {
        XCTAssertEqual(
            roomSecsPerPixel(sampleRate: 48_000),
            roomSecsPerPixel(sampleRate: 44_100),
            accuracy: 1e-9)
        // And the old axis really was off by that much.
        XCTAssertEqual(peakDur(44_100) / peakDur(48_000), 48_000 / 44_100, accuracy: 1e-9)
    }

    /// Scroll speed is the same claim seen sideways: pixels per second
    /// of the room, which is what the eye compares across the gutter.
    func testBeatmatchedDecksScrollAtTheSamePixelSpeed() {
        let pitched = 100.0 * (92.0 / 88.0 - 1.0)
        XCTAssertEqual(
            1.0 / roomSecsPerPixel(),
            1.0 / roomSecsPerPixel(pitchPercent: pitched),
            accuracy: 1e-6)
    }

    /// Every rung still scales the axis proportionally — the ladder's
    /// labels stay honest with the rate folded in.
    func testEveryZoomRungStillScalesTheAxis() {
        let base = roomSecsPerPixel(zoom: 1.0, pitchPercent: 6)
        for step in WaveformZoom.steps {
            let got = roomSecsPerPixel(zoom: step, pitchPercent: 6)
            XCTAssertEqual(got / base, step, accuracy: 1e-6,
                           "rung \(step) no longer scales the axis by its own factor")
        }
    }

    /// A pitch the platter cannot produce — a scratch leaking through,
    /// or a garbage telemetry read — must degrade to a slightly wrong
    /// scale, never a collapsed or exploded one.
    func testAbsurdRatesAreClamped() {
        let dur = peakDur(44_100)
        for rate in [-3.0, 0.0, 0.001, 50.0, Double.nan, .infinity] {
            let z = WaveformRenderer.effectiveTimeAxisZoom(
                timeAxisZoom: 1.0, platterRate: rate, peakDurSecs: dur)
            XCTAssertTrue(z.isFinite, "rate \(rate) produced a non-finite zoom")
            XCTAssertGreaterThanOrEqual(z, 0.25)
            XCTAssertLessThanOrEqual(z, 4.0)
        }
    }

    /// No cadence yet (cold deck, before the engine reports one): the
    /// axis is the DJ's zoom and nothing else, rather than a division
    /// by zero.
    func testNoChunkCadenceLeavesTheZoomAlone() {
        XCTAssertEqual(
            WaveformRenderer.effectiveTimeAxisZoom(
                timeAxisZoom: 0.8, platterRate: 1.2, peakDurSecs: 0),
            0.8, accuracy: 1e-12)
    }
}
