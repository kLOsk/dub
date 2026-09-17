//
//  PhaseMeter.swift
//  Dub
//
//  The beatmatch aid, round 4 (2026-09-16): Traktor's phase meter, one
//  beat long, standing in the gutter between the two running waveforms.
//
//  It replaces Stillpoint — three rounds of a cleverer instrument (role
//  inference, a workflow FSM, honesty gates, a pitch coach) that Daniel
//  called, in the end, worse than the standard. The standard is one
//  bar: half a beat ahead, half a beat behind, a marker for where the
//  other deck's beat sits against the master's. It does not care about
//  tempo. Tempo is matched by the numbers in the deck headers and by
//  the two strips scrolling at the same speed; the meter is for the
//  last few milliseconds, and for seeing which way to push.
//
//  Vertical, because the strips are: the centre line is collinear with
//  the strips' playheads (25 % from the top, PRD §9.1), and the meter
//  spans one beat of the gutter's height centred on it. The axis is the
//  one Stillpoint's round 6 settled on the rig — **late = above the
//  line**, the gap the room has opened ahead of you; push the record
//  and the marker settles down onto the line. Past half a beat it wraps
//  to the other end, as a phase does.
//
//  The maths is a pure function of both decks' grids and playheads, so
//  the whole thing is unit-testable; the canvas is a pure function of
//  one frame, so it snapshot-tests.
//

import SwiftUI

// MARK: - Model

/// One deck's contribution, sampled together with the other's.
struct PhaseMeterInputs: Equatable {
    var hasTrack = false
    var isPlaying = false
    /// The grid's BPM, in track time.
    var bpm: Double?
    /// The grid's first beat, in track seconds.
    var gridAnchorSecs: Double?
    /// The platter's pitch, so a beat's length in the room is known.
    var pitchPercent: Double?
    var playheadSecs: Double = 0

    /// Whether the deck has a grid to take a phase from.
    var hasGrid: Bool {
        hasTrack && (bpm ?? 0) > 0 && gridAnchorSecs != nil
    }

    /// Where the playhead sits inside its beat, 0 ≤ φ < 1 — from the
    /// grid, in track time, so the platter's pitch does not enter.
    var beatPhase: Double? {
        guard hasGrid, let bpm, let anchor = gridAnchorSecs else { return nil }
        let beats = (playheadSecs - anchor) * bpm / 60
        return beats - beats.rounded(.down)
    }

    /// A beat's length in the room, in ms — the grid's beat at the
    /// platter's pitch.
    var beatMs: Double? {
        guard hasGrid, let bpm else { return nil }
        let rate = 1 + (pitchPercent ?? 0) / 100
        guard rate > 0.05 else { return nil }
        return 60_000 / (bpm * rate)
    }
}

/// Everything the canvas draws.
struct PhaseMeterFrame: Equatable {
    /// The incoming deck's beat against the master's, in beats,
    /// −0.5 … 0.5. Negative = late (drawn above the line). `nil` when
    /// either deck has no grid.
    var phaseBeats: Double?
    /// The same, in the room's milliseconds — the master's beat length.
    var phaseMs: Double?
    /// The deck the marker belongs to — the one that is not the master.
    var incomingIsA = false
    /// Both playing and inside the lock window.
    var locked = false
}

enum PhaseMeter {
    /// Inside this the marker goes green: a beat placed within the
    /// tolerance a room stops hearing as two.
    static let lockWindowMs = 15.0

    /// Compare the incoming deck against the master. With no master
    /// declared, deck A is the reference.
    static func frame(a: PhaseMeterInputs, b: PhaseMeterInputs, masterIsA: Bool) -> PhaseMeterFrame {
        let master = masterIsA ? a : b
        let other = masterIsA ? b : a
        var out = PhaseMeterFrame(incomingIsA: !masterIsA)
        guard let pm = master.beatPhase, let po = other.beatPhase, let beatMs = master.beatMs
        else { return out }
        // Wrap the difference into (−0.5, 0.5]: past half a beat, the
        // nearer beat is the other one.
        var phi = po - pm
        phi -= (phi + 0.5).rounded(.down)
        if phi <= -0.5 { phi += 1 }
        out.phaseBeats = phi
        out.phaseMs = phi * beatMs
        out.locked = master.isPlaying && other.isPlaying && abs(phi * beatMs) <= lockWindowMs
        return out
    }
}

// MARK: - Driver

/// Reads both decks at 60 Hz while either plays and hands the canvas a
/// frame. Paused, the last frame stands — a phase does not move when
/// nothing does.
struct PhaseMeterView: View {
    @ObservedObject var model: WaveformAppModel

    var body: some View {
        let active = model.deckA.isPlaying || model.deckB.isPlaying
        TimelineView(.animation(minimumInterval: 1.0 / 60.0, paused: !active)) { _ in
            PhaseMeterCanvas(frame: frame())
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(DubColor.surface0)
    }

    @MainActor
    private func frame() -> PhaseMeterFrame {
        PhaseMeter.frame(
            a: inputs(model.deckA, idx: 0),
            b: inputs(model.deckB, idx: 1),
            masterIsA: model.masterDeck != .b)
    }

    @MainActor
    private func inputs(_ deck: DeckState, idx: UInt64) -> PhaseMeterInputs {
        var i = PhaseMeterInputs()
        i.hasTrack = deck.hasTrack
        i.isPlaying = deck.isPlaying
        i.bpm = deck.bpm
        i.gridAnchorSecs = deck.gridAnchorSecs
        i.pitchPercent = deck.pitchPercent
        i.playheadSecs = model.engine.positionSnapshot(deckIdx: idx).playheadSecsUnclamped
        return i
    }
}

// MARK: - Canvas

/// The meter: a narrow track one beat tall on the playhead line, the
/// marker on it. Pure — a function of the frame and the size.
struct PhaseMeterCanvas: View {
    let frame: PhaseMeterFrame

    /// The playheads' line, PRD §9.1.
    private static let lineFraction: CGFloat = 0.25
    /// One beat, in points, at full height; shrinks on a short gutter so
    /// the half-beat above the line stays on screen.
    private static let beatHeight: CGFloat = 160
    private static let trackWidth: CGFloat = 10
    private static let markerWidth: CGFloat = 24
    private static let markerHeight: CGFloat = 4

    var body: some View {
        Canvas { ctx, size in
            let lineY = size.height * Self.lineFraction
            let beat = min(Self.beatHeight, size.height * Self.lineFraction * 1.8)
            let cx = size.width / 2
            let track = CGRect(
                x: cx - Self.trackWidth / 2, y: lineY - beat / 2,
                width: Self.trackWidth, height: beat)

            // The track: a recessed well, quarter-beat ticks either side.
            let well = Path(roundedRect: track, cornerRadius: Self.trackWidth / 2)
            ctx.fill(well, with: .color(DubColor.surface2))
            ctx.stroke(well, with: .color(DubColor.divider), lineWidth: 1)
            for q in [-0.25, 0.25] {
                let y = lineY + CGFloat(q) * beat
                var tick = Path()
                tick.move(to: CGPoint(x: track.minX - 4, y: y))
                tick.addLine(to: CGPoint(x: track.minX, y: y))
                tick.move(to: CGPoint(x: track.maxX, y: y))
                tick.addLine(to: CGPoint(x: track.maxX + 4, y: y))
                ctx.stroke(tick, with: .color(DubColor.divider), lineWidth: 1)
            }

            // The line: across the whole gutter, so it reads as the same
            // line the two strips draw their playheads on.
            var line = Path()
            line.move(to: CGPoint(x: 0, y: lineY))
            line.addLine(to: CGPoint(x: size.width, y: lineY))
            ctx.stroke(line, with: .color(DubColor.playheadAccent.opacity(0.35)), lineWidth: 1)
            var onTrack = Path()
            onTrack.move(to: CGPoint(x: track.minX - 6, y: lineY))
            onTrack.addLine(to: CGPoint(x: track.maxX + 6, y: lineY))
            ctx.stroke(onTrack, with: .color(DubColor.playheadAccent), lineWidth: 1)

            guard let phi = frame.phaseBeats else { return }
            let tint = frame.locked
                ? DubColor.stateLocked
                : (frame.incomingIsA ? DubColor.deckATint : DubColor.deckBTint)

            // The marker. At half a beat it is at an end of the track and
            // the next frame puts it at the other — the wrap a phase has.
            let y = lineY + CGFloat(phi) * beat
            drawMarker(ctx, cx: cx, y: y, tint: tint, alpha: 1)

            // Whose beat the marker is.
            let letter = Text(frame.incomingIsA ? "A" : "B")
                .font(.system(size: 9, weight: .bold))
                .foregroundColor(tint.opacity(0.9))
            ctx.draw(letter, at: CGPoint(x: cx, y: track.maxY + 10))
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .accessibilityElement()
        .accessibilityLabel(accessibilityText)
    }

    private func drawMarker(_ ctx: GraphicsContext, cx: CGFloat, y: CGFloat, tint: Color, alpha: Double) {
        let rect = CGRect(
            x: cx - Self.markerWidth / 2, y: y - Self.markerHeight / 2,
            width: Self.markerWidth, height: Self.markerHeight)
        let bar = Path(roundedRect: rect, cornerRadius: Self.markerHeight / 2)
        ctx.fill(bar, with: .color(tint.opacity(alpha)))
        if frame.locked {
            ctx.stroke(bar, with: .color(tint.opacity(0.5)), lineWidth: 3)
        }
    }

    private var accessibilityText: String {
        guard let ms = frame.phaseMs else { return "Phase meter, no grid" }
        let deck = frame.incomingIsA ? "A" : "B"
        if frame.locked { return "Deck \(deck) in phase" }
        return String(format: "Deck %@ %@ by %.0f milliseconds", deck, ms < 0 ? "late" : "early", abs(ms))
    }
}
