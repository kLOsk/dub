//
//  PhaseClockView.swift
//  Dub
//
//  The beatmatch visualization (PRD §9.4, redesigned). A circular
//  "phase clock": one full revolution = one beat. A gold marker
//  (deck A) and a teal marker (deck B) orbit the ring at each deck's
//  current bar-phase. The DJ reads it like two turntables:
//
//    • markers overlapping        → beats locked
//    • constant gap, same speed   → tempos matched, phase offset
//    • drifting apart             → tempos don't match (Δ BPM)
//
//  Chosen over Serato's tempo display (weak) and Traktor's phase bars
//  (good but less vivid). Precise Δ ms / Δ BPM render below the ring.
//
//  Data is grid-based and needs no new engine FFI: each deck's bar
//  phase comes from its captured grid anchor + BPM + the live
//  playhead (`positionSnapshot`). A later milestone can swap the data
//  source to ODF cross-correlation (dub-match) for Thru / real records
//  without changing this view.
//

import SwiftUI
import DubCore

struct PhaseClockView: View {

    @ObservedObject var model: WaveformAppModel

    /// Per-deck phase resolved for one frame. One ring revolution =
    /// one **beat** (not a bar), so a ~20 ms offset is a visible ~10°
    /// rather than an invisible ~1°.
    private struct DeckPhase {
        /// Beat-phase fraction [0, 1) — 0 at the top of the ring. The
        /// marker angle. Advances at the deck's real (pitched) rate
        /// because it's derived from the live playhead.
        var beatFraction: Double
        /// Track-time beat duration, for the Δ ms readout.
        var beatDurSecs: Double
        /// Pitch-adjusted ("live") BPM, for the Δ BPM readout.
        var liveBpm: Double
        /// Dimmed when the grid confidence is low (honesty).
        var confident: Bool
    }

    private struct Snapshot {
        var a: DeckPhase?
        var b: DeckPhase?
        var deltaBpm: Double?
        var deltaMs: Double?
        var locked: Bool
    }

    var body: some View {
        let active = model.deckA.isPlaying || model.deckB.isPlaying
        TimelineView(.animation(minimumInterval: 1.0 / 60.0, paused: !active)) { _ in
            let snap = snapshot()
            VStack(spacing: DubSpacing.sm) {
                Spacer(minLength: 0)
                Canvas { ctx, size in draw(ctx, size: size, snap: snap) }
                    .frame(width: DubLayout.phaseClockDiameter,
                           height: DubLayout.phaseClockDiameter)
                numerics(snap)
                Spacer(minLength: 0)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .frame(width: DubLayout.phaseClockWidth)
    }

    // MARK: - Phase math

    private func phase(for deck: DeckState, deckIdx: UInt64) -> DeckPhase? {
        // Show a marker for any loaded deck that has a grid — even
        // paused. A stopped deck has a well-defined bar phase at its
        // current playhead, so the DJ can pre-align before letting go
        // of the platter. The marker only *moves* while the deck plays.
        guard deck.hasTrack, let bpm = deck.bpm, bpm > 0,
              let anchor = deck.gridAnchorSecs else { return nil }
        let pos = model.engine.positionSnapshot(deckIdx: deckIdx).playheadSecsUnclamped
        let beatDur = 60.0 / bpm
        let beatFraction = wrapUnit((pos - anchor) / beatDur)
        let liveBpm = bpm * (1.0 + (deck.pitchPercent ?? 0) / 100.0)
        return DeckPhase(
            beatFraction: beatFraction,
            beatDurSecs: beatDur,
            liveBpm: liveBpm,
            confident: deck.bpmConfidence >= 0.15)
    }

    private func snapshot() -> Snapshot {
        let a = phase(for: model.deckA, deckIdx: 0)
        let b = phase(for: model.deckB, deckIdx: 1)
        var deltaBpm: Double?
        var deltaMs: Double?
        var locked = false
        if let a, let b {
            deltaBpm = a.liveBpm - b.liveBpm
            // Fine beat-phase offset, wrapped to ±half a beat, in ms.
            let avgBeat = (a.beatDurSecs + b.beatDurSecs) / 2.0
            let diff = wrapHalf(a.beatFraction - b.beatFraction)
            deltaMs = diff * avgBeat * 1000.0
            locked = abs(deltaMs ?? 0) < 10.0 && abs(deltaBpm ?? 0) < 0.1
        }
        return Snapshot(a: a, b: b, deltaBpm: deltaBpm, deltaMs: deltaMs, locked: locked)
    }

    /// Map a value to [0, 1).
    private func wrapUnit(_ x: Double) -> Double {
        let f = x - floor(x)
        return f.isFinite ? f : 0
    }

    /// Map a value to [-0.5, 0.5).
    private func wrapHalf(_ x: Double) -> Double {
        let f = wrapUnit(x)
        return f >= 0.5 ? f - 1.0 : f
    }

    // MARK: - Drawing

    private func draw(_ ctx: GraphicsContext, size: CGSize, snap: Snapshot) {
        let center = CGPoint(x: size.width / 2, y: size.height / 2)
        let radius = min(size.width, size.height) / 2 - 8
        let ringColor = DubColor.divider

        // Lock glow behind the ring.
        if snap.locked {
            let glow = Path(ellipseIn: CGRect(
                x: center.x - radius - 4, y: center.y - radius - 4,
                width: (radius + 4) * 2, height: (radius + 4) * 2))
            ctx.stroke(glow, with: .color(DubColor.stateLocked.opacity(0.55)), lineWidth: 3)
        }

        // The ring (one revolution = one beat).
        let ring = Path(ellipseIn: CGRect(
            x: center.x - radius, y: center.y - radius,
            width: radius * 2, height: radius * 2))
        ctx.stroke(ring, with: .color(ringColor), lineWidth: 2)

        // One revolution = one beat. Quarter-beat (16th-note) ticks
        // help gauge fine offset; the top tick (the beat boundary, the
        // thing you align to) is drawn longer + brighter.
        let ticks = 4
        for i in 0..<ticks {
            let ang = Double(i) / Double(ticks) * 2 * .pi
            let outer = point(center, radius, ang)
            let inner = point(center, radius - (i == 0 ? 10 : 6), ang)
            var tick = Path()
            tick.move(to: inner)
            tick.addLine(to: outer)
            ctx.stroke(tick,
                       with: .color(i == 0 ? DubColor.textSecondary : ringColor),
                       lineWidth: i == 0 ? 2 : 1)
        }

        // Spindle — a small centre dot, echoing the record.
        let spindle = Path(ellipseIn: CGRect(
            x: center.x - 3, y: center.y - 3, width: 6, height: 6))
        ctx.fill(spindle, with: .color(DubColor.textTertiary))

        // Deck markers — beat phase. Overlap = beats locked.
        if let a = snap.a {
            drawMarker(ctx, center: center, radius: radius,
                       fraction: a.beatFraction, tint: DubColor.deckATint,
                       confident: a.confident)
        }
        if let b = snap.b {
            drawMarker(ctx, center: center, radius: radius,
                       fraction: b.beatFraction, tint: DubColor.deckBTint,
                       confident: b.confident)
        }
    }

    private func drawMarker(
        _ ctx: GraphicsContext, center: CGPoint, radius: CGFloat,
        fraction: Double, tint: Color, confident: Bool
    ) {
        // Phase 0 (downbeat) sits at the top, advancing clockwise.
        let ang = fraction * 2 * .pi
        let p = point(center, radius, ang)
        let r: CGFloat = 7
        let dot = Path(ellipseIn: CGRect(x: p.x - r, y: p.y - r, width: r * 2, height: r * 2))
        ctx.fill(dot, with: .color(tint.opacity(confident ? 1.0 : 0.4)))
        // Thin spoke from spindle to marker for a turntable read.
        var spoke = Path()
        spoke.move(to: center)
        spoke.addLine(to: p)
        ctx.stroke(spoke, with: .color(tint.opacity(confident ? 0.35 : 0.15)), lineWidth: 1.5)
    }

    /// Point on a circle, clockwise from the top (12 o'clock = angle 0).
    private func point(_ center: CGPoint, _ radius: CGFloat, _ angle: Double) -> CGPoint {
        CGPoint(x: center.x + radius * CGFloat(sin(angle)),
                y: center.y - radius * CGFloat(cos(angle)))
    }

    // MARK: - Numerics

    @ViewBuilder
    private func numerics(_ snap: Snapshot) -> some View {
        VStack(spacing: 2) {
            Text(snap.deltaMs.map { String(format: "Δ %+.0f ms", $0) } ?? "Δ — ms")
                .font(DubFont.numericInline)
                .foregroundStyle(snap.locked ? DubColor.stateLocked : DubColor.textPrimary)
            Text(snap.deltaBpm.map { String(format: "Δ %+.1f BPM", $0) } ?? "Δ — BPM")
                .font(DubFont.micro)
                .foregroundStyle(DubColor.textSecondary)
        }
        .monospacedDigit()
    }
}

#Preview("Phase clock") {
    PhaseClockView(model: WaveformAppModel())
        .frame(height: 320)
        .background(DubColor.surface0)
}
