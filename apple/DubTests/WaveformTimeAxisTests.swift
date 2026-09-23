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

    // MARK: - Following the platter (rig, 2026-09-23)
    //
    // "When I pitch from −8 to +8 the waveform speeds up and at the
    // very end it jumps into a new zoom." The axis followed the *held*
    // pitch, which only moves once the platter is still. Serato
    // stretches the picture with the fader. These pin a follower that
    // does — and still does not zoom through a spin-up or a scratch,
    // which is why the hold was there.

    private let frame = 1.0 / 60.0

    private func run(_ f: inout PlatterAxisFollower, _ pitch: Double?, secs: Double) -> Double {
        var out = f.axisPitch
        for _ in 0..<Int((secs / frame).rounded()) { out = f.step(livePitch: pitch, dt: frame) }
        return out
    }

    /// A fader move is followed while it happens, not after: every
    /// frame of a −8 → +8 sweep moves the axis, and it ends where the
    /// fader ends with no step left to take.
    func testAFaderSweepIsFollowedContinuously() {
        var f = PlatterAxisFollower()
        _ = run(&f, -8, secs: 1)
        XCTAssertEqual(f.axisPitch, -8, accuracy: 0.05)
        var last = f.axisPitch
        var biggestStep = 0.0
        let frames = 60
        for i in 1...frames {
            let live = -8 + 16 * Double(i) / Double(frames)
            let now = f.step(livePitch: live, dt: frame)
            // 1e-3: the last hundred-thousandths of settling onto −8 are
            // still arriving as the sweep starts — that is not a reversal.
            XCTAssertGreaterThanOrEqual(now, last - 1e-3, "the axis went backwards mid-sweep")
            biggestStep = max(biggestStep, now - last)
            last = now
        }
        XCTAssertGreaterThan(f.axisPitch, 5, "the axis waited for the fader to stop")
        XCTAssertLessThan(biggestStep, 1.0, "the axis jumped rather than stretched")
        let settled = run(&f, 8, secs: 0.5)
        XCTAssertEqual(settled, 8, accuracy: 0.05)
    }

    /// Platter wobble is not a tempo: sub-percent jitter barely moves
    /// the picture.
    func testWobbleBarelyMovesTheAxis() {
        var f = PlatterAxisFollower()
        _ = run(&f, 2, secs: 1)
        var lo = Double.infinity, hi = -Double.infinity
        for i in 0..<240 {
            let live = 2 + 0.2 * sin(Double(i) * frame * 2 * .pi / 1.8)
            let a = f.step(livePitch: live, dt: frame)
            lo = min(lo, a); hi = max(hi, a)
        }
        XCTAssertLessThan(hi - lo, 0.45)
    }

    /// STOP and start again: the platter sweeps −100 % → the fader's
    /// value, crossing the band on the way. The axis holds through it —
    /// the "weird zoom jumping thingie" the hold was added for.
    func testASpinUpDoesNotZoomThePicture() {
        var f = PlatterAxisFollower()
        _ = run(&f, 3, secs: 1)
        _ = run(&f, nil, secs: 1)
        var worst = 0.0
        for i in 0..<30 {
            let live = -100 + 103 * Double(i) / 29
            worst = max(worst, abs(f.step(livePitch: live, dt: frame) - 3))
        }
        worst = max(worst, abs(run(&f, 3, secs: 1) - 3))
        XCTAssertLessThan(worst, 0.05, "the axis zoomed through the spin-up")
    }

    /// A scratch leaves the band on every pull-back; the axis holds,
    /// then glides — not jumps — to a new fader position.
    func testAfterAScratchTheAxisGlidesToTheFader() {
        var f = PlatterAxisFollower()
        _ = run(&f, 0, secs: 1)
        for i in 0..<60 {
            let live = i % 10 < 5 ? -250.0 : 180.0
            XCTAssertEqual(f.step(livePitch: live, dt: frame), 0, accuracy: 1e-9)
        }
        var last = f.axisPitch
        var biggestStep = 0.0
        for _ in 0..<90 {
            let now = f.step(livePitch: 6, dt: frame)
            biggestStep = max(biggestStep, abs(now - last))
            last = now
        }
        XCTAssertEqual(f.axisPitch, 6, accuracy: 0.05)
        XCTAssertLessThan(biggestStep, 1.5)
    }

    /// STOP: the motor brakes the platter from the fader's pitch to a
    /// standstill, and the first half of that is inside the band. The
    /// picture keeps exactly the zoom it had (rig, 2026-09-23: "when I
    /// scratch the waveform zooms… same with start stop").
    func testABrakeKeepsTheZoomExactly() {
        var f = PlatterAxisFollower()
        _ = run(&f, 4, secs: 1.5)
        let before = f.axisPitch
        for i in 0..<30 {
            let live = 4 - 104 * Double(i) / 29
            XCTAssertEqual(f.step(livePitch: live, dt: frame), before, accuracy: 0.01)
        }
        XCTAssertEqual(run(&f, nil, secs: 1), before, accuracy: 0.01)
        for i in 0..<40 {
            let live = -100 + 104 * Double(i) / 39
            XCTAssertEqual(f.step(livePitch: live, dt: frame), before, accuracy: 0.01)
        }
        XCTAssertEqual(run(&f, 4, secs: 1.5), before, accuracy: 0.01)
    }

    /// A scratch that never leaves the band — a slow drag back and
    /// forth around the playing speed — is still a scratch, not a tempo.
    func testAnInBandScratchKeepsTheZoomExactly() {
        var f = PlatterAxisFollower()
        _ = run(&f, 0, secs: 1.5)
        for i in 0..<180 {
            let live = -30 * (0.5 - 0.5 * cos(Double(i) * frame * 2 * .pi * 2))
            XCTAssertEqual(f.step(livePitch: live, dt: frame), 0, accuracy: 0.01)
        }
        XCTAssertEqual(run(&f, 0, secs: 1), 0, accuracy: 0.01)
    }

    /// A stopped deck keeps its picture.
    func testAPausedDeckHolds() {
        var f = PlatterAxisFollower()
        _ = run(&f, 5, secs: 1)
        XCTAssertEqual(run(&f, nil, secs: 3), 5, accuracy: 0.05)
    }
}
