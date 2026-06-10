//
//  BeatmatchViz.swift
//  Dub
//
//  Candidate beatmatch visualizations, laid out **side by side** in the
//  centre gutter between the two vertical waveform lanes. The interface
//  is left/right — deck A (gold) is the left lane, deck B (teal) the
//  right — so every viz here uses the same convention:
//
//      • a vertical centre axis = matched
//      • lean / bar / marker toward the LEFT  → deck A is ahead
//        (faster on tempo, or earlier on phase)
//      • toward the RIGHT → deck B is ahead
//
//  so it is immediately obvious *which* deck (left or right) is running
//  faster and *which* is early or late. All show the *relative* state,
//  which moves at the tempo difference rather than the beat rate.
//
//    A · Pendulum   — a plumb arm that leans toward the faster deck
//                     (tempo); a bob on the base rail rides toward the
//                     earlier deck (phase). Plumb + centred + green = locked.
//    B · Tug bars   — from a centre line: an UP bar on the faster deck's
//                     side (tempo) and a DOWN bar on the earlier deck's
//                     side (phase).
//    C · Beat ladder— two scrolling lanes (A left, B right). The faster
//                     deck's rungs descend faster; the rung connector
//                     tilts toward it, its offset is the phase.
//
//  Shared Δ ms / Δ BPM readout underneath spells the same thing in words.
//

import SwiftUI
import DubCore

// MARK: - Shared phase snapshot

/// Beats per visual cycle. The phase fractions wrap once per *cycle*,
/// so the markers move at 1/`beatsPerCycle` of the beat rate. 4 = one
/// bar: a quarter the speed and locked to the downbeat. Dial to 1 for
/// per-beat, 2/8 to taste.
let beatsPerCycle = 4.0

/// Full-scale tempo deflection: ±this many BPM pins a meter to the edge.
private let maxBpm = 2.0

struct BeatmatchSnapshot {
    /// Per-deck phase within a `beatsPerCycle`-long cycle [0, 1) —
    /// 0 on the downbeat of the cycle.
    var aFrac: Double?
    var bFrac: Double?
    /// Deck B's phase relative to A, wrapped to [-0.5, 0.5). 0 = cycles
    /// aligned; >0 = B (right) ahead, <0 = A (left) ahead.
    var relPhase: Double?
    /// >0 = B (right) faster, <0 = A (left) faster.
    var deltaBpm: Double?
    var deltaMs: Double?
    var aCycleDur: Double?
    var bCycleDur: Double?

    var bothActive: Bool { relPhase != nil }
    var tempoLocked: Bool { (deltaBpm.map { abs($0) < 0.1 }) ?? false }
    var phaseLocked: Bool { (deltaMs.map { abs($0) < 10 }) ?? false }
    var locked: Bool { tempoLocked && phaseLocked }

    /// Signed tempo, normalised to [-1, 1] (+ = right deck faster).
    var tempoNorm: CGFloat { clampUnit((deltaBpm ?? 0) / maxBpm) }
    /// Signed phase, normalised to [-1, 1] (+ = right deck earlier).
    var phaseNorm: CGFloat { clampUnit((relPhase ?? 0) * 2) }
}

@MainActor
func beatmatchSnapshot(_ model: WaveformAppModel) -> BeatmatchSnapshot {
    func deck(_ d: DeckState, _ idx: UInt64) -> (frac: Double, cycleDur: Double, live: Double)? {
        guard d.hasTrack, let bpm = d.bpm, bpm > 0, let anchor = d.gridAnchorSecs else { return nil }
        let pos = model.engine.positionSnapshot(deckIdx: idx).playheadSecsUnclamped
        let cycleDur = (60.0 / bpm) * beatsPerCycle
        var frac = (pos - anchor) / cycleDur
        frac -= frac.rounded(.down)
        let live = bpm * (1 + (d.pitchPercent ?? 0) / 100)
        return (frac, (60.0 / live) * beatsPerCycle, live)
    }
    let a = deck(model.deckA, 0)
    let b = deck(model.deckB, 1)
    var s = BeatmatchSnapshot(aFrac: a?.frac, bFrac: b?.frac,
                              aCycleDur: a?.cycleDur, bCycleDur: b?.cycleDur)
    if let a, let b {
        var rp = b.frac - a.frac
        rp -= rp.rounded()                       // → [-0.5, 0.5)
        s.relPhase = rp
        s.deltaBpm = b.live - a.live
        s.deltaMs = rp * ((a.cycleDur + b.cycleDur) / 2) * 1000
    }
    return s
}

private func clampUnit(_ v: Double) -> CGFloat { CGFloat(max(-1, min(1, v))) }

/// Tint for a leading-deck indicator: green when locked, neutral near
/// zero, else the leading deck's lane colour (+ = right/B, - = left/A).
private func leadTint(_ signed: CGFloat, locked: Bool) -> Color {
    if locked { return DubColor.stateLocked }
    if abs(signed) < 0.04 { return DubColor.textSecondary }
    return signed > 0 ? DubColor.deckBTint : DubColor.deckATint
}

/// Resolve + draw a short text label inside a `Canvas`.
private func drawLabel(
    _ ctx: GraphicsContext, _ string: String, at p: CGPoint,
    color: Color, font: Font = DubFont.micro, anchor: UnitPoint = .center
) {
    var t = ctx.resolve(Text(string).font(font))
    t.shading = .color(color)
    ctx.draw(t, at: p, anchor: anchor)
}

/// A·B corner anchors so every box reads left = A, right = B.
private func drawDeckCorners(_ ctx: GraphicsContext, _ size: CGSize) {
    drawLabel(ctx, "A", at: CGPoint(x: 9, y: 9), color: DubColor.deckATint)
    drawLabel(ctx, "B", at: CGPoint(x: size.width - 9, y: 9), color: DubColor.deckBTint)
}

private func drawIdle(_ ctx: GraphicsContext, _ size: CGSize) {
    drawLabel(ctx, "—", at: CGPoint(x: size.width / 2, y: size.height / 2),
              color: DubColor.textTertiary)
}

/// 60 Hz redraw clock, paused when neither deck is advancing.
@MainActor
private func beatTimeline(_ model: WaveformAppModel) -> some TimelineSchedule {
    AnimationTimelineSchedule(minimumInterval: 1.0 / 60.0,
                              paused: !(model.deckA.isPlaying || model.deckB.isPlaying))
}

// MARK: - Container (A | B | C side by side)

struct BeatmatchStackView: View {
    @ObservedObject var model: WaveformAppModel

    var body: some View {
        TimelineView(beatTimeline(model)) { _ in
            let snap = beatmatchSnapshot(model)
            VStack(spacing: DubSpacing.sm) {
                HStack(spacing: DubSpacing.xs) {
                    vizBox("PENDULUM") { PendulumView(snap: snap) }
                    vizBox("TUG BARS") { TugBarsView(snap: snap) }
                    vizBox("LADDER") { BeatLadderView(snap: snap) }
                }
                .frame(maxHeight: .infinity)
                SharedReadout(snap: snap)
            }
            .padding(.vertical, DubSpacing.sm)
            .padding(.horizontal, DubSpacing.xs)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(DubColor.surface0)
        }
    }

    @ViewBuilder
    private func vizBox<Content: View>(_ label: String, @ViewBuilder _ content: () -> Content) -> some View {
        VStack(spacing: DubSpacing.xs) {
            Text(label)
                .font(DubFont.caps)
                .tracking(0.5)
                .foregroundStyle(DubColor.textTertiary)
                .lineLimit(1)
                .minimumScaleFactor(0.7)
            content()
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .padding(DubSpacing.xs)
        .background(DubColor.surface1)
        .clipShape(RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous))
    }
}

// MARK: - Shared readout (words, full width)

/// Spells out the two answers in plain language, arrow pointing at the
/// leading deck's side, so the visual and the text never disagree.
private struct SharedReadout: View {
    let snap: BeatmatchSnapshot

    var body: some View {
        VStack(spacing: 2) {
            row(locked: snap.tempoLocked, signed: snap.tempoNorm,
                lockedText: "TEMPO MATCHED",
                noun: "faster",
                value: snap.deltaBpm.map { String(format: "%.2f BPM", abs($0)) })
            row(locked: snap.phaseLocked, signed: snap.phaseNorm,
                lockedText: "PHASE LOCKED",
                noun: "early",
                value: snap.deltaMs.map { String(format: "%.0f ms", abs($0)) })
        }
        .font(DubFont.micro)
        .monospacedDigit()
        .frame(maxWidth: .infinity)
    }

    @ViewBuilder
    private func row(locked: Bool, signed: CGFloat, lockedText: String,
                     noun: String, value: String?) -> some View {
        if !snap.bothActive {
            Text("load both decks")
                .foregroundStyle(DubColor.textTertiary)
        } else if locked {
            Text("\(lockedText) ✓")
                .foregroundStyle(DubColor.stateLocked)
        } else {
            let towardB = signed > 0
            let deck = towardB ? "DECK B" : "DECK A"
            HStack(spacing: DubSpacing.xs) {
                if !towardB { Text("◀").foregroundStyle(DubColor.deckATint) }
                Text("\(deck) \(noun)")
                    .foregroundStyle(towardB ? DubColor.deckBTint : DubColor.deckATint)
                if let value { Text(value).foregroundStyle(DubColor.textSecondary) }
                if towardB { Text("▶").foregroundStyle(DubColor.deckBTint) }
            }
        }
    }
}

// MARK: - A · Pendulum (lean = tempo, base bob = phase)

/// A plumb arm hangs from the top centre and leans toward the faster
/// deck — left for A, right for B — so tempo error reads as a tilt down
/// the whole height. A separate bob on the base rail rides toward the
/// earlier deck for phase. Plumb + centred + green = locked.
struct PendulumView: View {
    let snap: BeatmatchSnapshot

    var body: some View {
        Canvas { ctx, size in draw(ctx, size: size) }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private func draw(_ ctx: GraphicsContext, size: CGSize) {
        let cx = size.width / 2
        let pivot = CGPoint(x: cx, y: 8)
        let armBottomY = size.height - 24
        let railY = size.height - 9
        let maxDx = size.width / 2 - 8

        // Faint plumb reference + base rail.
        var plumb = Path()
        plumb.move(to: pivot)
        plumb.addLine(to: CGPoint(x: cx, y: armBottomY))
        ctx.stroke(plumb, with: .color(DubColor.divider.opacity(0.5)),
                   style: StrokeStyle(lineWidth: 1, dash: [2, 4]))
        var rail = Path()
        rail.move(to: CGPoint(x: cx - maxDx, y: railY))
        rail.addLine(to: CGPoint(x: cx + maxDx, y: railY))
        ctx.stroke(rail, with: .color(DubColor.divider), lineWidth: 1)
        var rtick = Path()
        rtick.move(to: CGPoint(x: cx, y: railY - 4)); rtick.addLine(to: CGPoint(x: cx, y: railY + 4))
        ctx.stroke(rtick, with: .color(DubColor.textSecondary), lineWidth: 1)
        drawDeckCorners(ctx, size)

        guard snap.bothActive else { drawIdle(ctx, size); return }

        // Tempo: arm leans toward the faster deck.
        let tipX = cx + snap.tempoNorm * maxDx
        let armC = leadTint(snap.tempoNorm, locked: snap.tempoLocked)
        var arm = Path()
        arm.move(to: pivot)
        arm.addLine(to: CGPoint(x: tipX, y: armBottomY))
        ctx.stroke(arm, with: .color(armC), style: StrokeStyle(lineWidth: 2.5, lineCap: .round))
        let pr: CGFloat = 6
        ctx.fill(Path(ellipseIn: CGRect(x: tipX - pr, y: armBottomY - pr, width: pr * 2, height: pr * 2)),
                 with: .color(armC))
        ctx.fill(Path(ellipseIn: CGRect(x: cx - 2.5, y: pivot.y - 2.5, width: 5, height: 5)),
                 with: .color(DubColor.textSecondary))

        // Phase: bob on the base rail rides toward the earlier deck.
        let bobX = cx + snap.phaseNorm * maxDx
        let phaseC = leadTint(snap.phaseNorm, locked: snap.phaseLocked)
        if snap.locked {
            ctx.fill(Path(ellipseIn: CGRect(x: bobX - 11, y: railY - 11, width: 22, height: 22)),
                     with: .color(DubColor.stateLocked.opacity(0.18)))
        }
        let br: CGFloat = 5
        ctx.fill(Path(ellipseIn: CGRect(x: bobX - br, y: railY - br, width: br * 2, height: br * 2)),
                 with: .color(phaseC))
    }
}

// MARK: - B · Tug bars (up = tempo, down = phase, on the leading side)

/// A centre line splits the box. Above it, a bar grows up on the faster
/// deck's side (tempo). Below it, a bar grows down on the earlier deck's
/// side (phase). One glance: which side, how much, for each of the two
/// questions. Bars green when their channel is locked.
struct TugBarsView: View {
    let snap: BeatmatchSnapshot

    var body: some View {
        Canvas { ctx, size in draw(ctx, size: size) }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private func draw(_ ctx: GraphicsContext, size: CGSize) {
        let cx = size.width / 2, midY = size.height / 2
        let zone = size.height / 2 - 18
        let barW = max(9, size.width * 0.22)
        let off = size.width * 0.24

        var mid = Path()
        mid.move(to: CGPoint(x: 6, y: midY)); mid.addLine(to: CGPoint(x: size.width - 6, y: midY))
        ctx.stroke(mid, with: .color(snap.locked ? DubColor.stateLocked : DubColor.divider), lineWidth: 1.5)
        drawDeckCorners(ctx, size)
        drawLabel(ctx, "▲ tempo", at: CGPoint(x: cx, y: 9), color: DubColor.textTertiary)
        drawLabel(ctx, "▼ phase", at: CGPoint(x: cx, y: size.height - 9), color: DubColor.textTertiary)

        guard snap.bothActive else { drawIdle(ctx, size); return }

        func bar(signed: CGFloat, up: Bool, locked: Bool) {
            guard abs(signed) > 0.02 else { return }
            let towardB = signed > 0
            let bx = cx + (towardB ? off : -off)
            let h = abs(signed) * zone
            let rect = CGRect(x: bx - barW / 2, y: up ? midY - h : midY, width: barW, height: h)
            let c = locked ? DubColor.stateLocked : (towardB ? DubColor.deckBTint : DubColor.deckATint)
            ctx.fill(Path(roundedRect: rect, cornerRadius: 2), with: .color(c))
        }
        bar(signed: snap.tempoNorm, up: true, locked: snap.tempoLocked)
        bar(signed: snap.phaseNorm, up: false, locked: snap.phaseLocked)

        if snap.locked {
            ctx.fill(Path(ellipseIn: CGRect(x: cx - 4, y: midY - 4, width: 8, height: 8)),
                     with: .color(DubColor.stateLocked))
        }
    }
}

// MARK: - C · Beat ladder (two scrolling lanes, mirrors the waveforms)

/// Two vertical lanes — A left, B right — with cycle rungs scrolling
/// down toward a "now" line, exactly like the waveforms beside it. The
/// faster deck's rungs descend faster and its next rung sits lower; the
/// connector between the two nearest rungs tilts toward the faster deck
/// (tempo) and its vertical span is the phase offset.
struct BeatLadderView: View {
    let snap: BeatmatchSnapshot
    private static let cyclesTall = 2.4

    var body: some View {
        Canvas { ctx, size in draw(ctx, size: size) }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private func draw(_ ctx: GraphicsContext, size: CGSize) {
        let laneAX = size.width * 0.30, laneBX = size.width * 0.70
        let nowY = size.height * 0.56
        let spacingV = size.height / CGFloat(Self.cyclesTall)
        let tickW = size.width * 0.32

        var now = Path()
        now.move(to: CGPoint(x: 6, y: nowY)); now.addLine(to: CGPoint(x: size.width - 6, y: nowY))
        ctx.stroke(now, with: .color(snap.locked ? DubColor.stateLocked : DubColor.divider),
                   style: StrokeStyle(lineWidth: 1.5, dash: snap.locked ? [] : [3, 3]))
        drawDeckCorners(ctx, size)

        guard let aFrac = snap.aFrac, let bFrac = snap.bFrac else { drawIdle(ctx, size); return }

        func rungs(_ frac: Double, _ laneX: CGFloat, _ tint: Color) {
            for k in -3...3 {
                let y = nowY - (CGFloat(1 - frac) + CGFloat(k)) * spacingV
                guard y >= -4, y <= size.height + 4 else { continue }
                var p = Path()
                p.move(to: CGPoint(x: laneX - tickW / 2, y: y))
                p.addLine(to: CGPoint(x: laneX + tickW / 2, y: y))
                ctx.stroke(p, with: .color(tint), style: StrokeStyle(lineWidth: 2.5, lineCap: .round))
            }
        }
        let aC = snap.locked ? DubColor.stateLocked : DubColor.deckATint
        let bC = snap.locked ? DubColor.stateLocked : DubColor.deckBTint
        rungs(aFrac, laneAX, aC)
        rungs(bFrac, laneBX, bC)

        // Connector between each lane's next-downbeat rung: tilt = tempo,
        // vertical span = phase.
        let yA = nowY - CGFloat(1 - aFrac) * spacingV
        let yB = nowY - CGFloat(1 - bFrac) * spacingV
        var link = Path()
        link.move(to: CGPoint(x: laneAX, y: yA))
        link.addLine(to: CGPoint(x: laneBX, y: yB))
        ctx.stroke(link, with: .color(snap.phaseLocked ? DubColor.stateLocked
                                      : DubColor.textTertiary),
                   style: StrokeStyle(lineWidth: 1.5, dash: snap.phaseLocked ? [] : [2, 3]))
    }
}

#Preview("Beatmatch gutter") {
    BeatmatchStackView(model: WaveformAppModel())
        .frame(width: 260, height: 480)
        .background(DubColor.surface0)
}
