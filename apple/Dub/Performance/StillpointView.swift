//
//  StillpointView.swift
//  Dub
//
//  Renderer for the Stillpoint beatmatch aid (round 3 — see
//  docs/investigations/BEATMATCH-AID-STILLPOINT.md). One
//  incoming-tinted band on the lock line: it drifts when your tempo
//  is off, freezes when matched, sits on the line when you're in,
//  and the line grows green beneath it for every beat you hold.
//  Nothing here ever moves at beat rate — only at error rate.
//
//  `StillpointCanvas` is the pure renderer over a `StillpointFrame`
//  (previews + snapshot tests construct frames directly);
//  `StillpointView` is the live wrapper that ticks the engine.
//

import AppKit
import SwiftUI
import DubCore

// MARK: - Live view

struct StillpointView: View {
    @ObservedObject var model: WaveformAppModel
    @State private var engine = StillpointEngine()

    var body: some View {
        let active = model.deckA.isPlaying || model.deckB.isPlaying
        TimelineView(.animation(minimumInterval: 1.0 / 60.0, paused: !active)) { tl in
            StillpointCanvas(frame: tick(now: tl.date.timeIntervalSinceReferenceDate))
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(DubColor.surface0)
    }

    @MainActor
    private func tick(now: Double) -> StillpointFrame {
        engine.update(now: now,
                      a: inputs(model.deckA, idx: 0),
                      b: inputs(model.deckB, idx: 1))
    }

    @MainActor
    private func inputs(_ deck: DeckState, idx: UInt64) -> StillpointDeckInputs {
        var i = StillpointDeckInputs()
        i.hasTrack = deck.hasTrack
        i.isPlaying = deck.isPlaying
        i.bpm = deck.bpm
        i.bpmConfidence = deck.bpmConfidence
        i.gridAnchorSecs = deck.gridAnchorSecs
        i.pitchPercent = deck.pitchPercent
        i.pitchSettled = deck.pitchSettled
        i.timecodeLockState = deck.timecodeLockState
        i.isTimecodeDriven = deck.controlMode == 1
        i.playheadSecs = model.engine.positionSnapshot(deckIdx: idx).playheadSecsUnclamped
        return i
    }
}

// MARK: - Pure renderer

struct StillpointCanvas: View {
    let frame: StillpointFrame

    private typealias T = StillpointTuning

    var body: some View {
        Canvas { ctx, size in
            if frame.stageProgress < 1 {
                drawFace(frame.prevStage, ctx, size, fade: 1 - frame.stageProgress)
            }
            drawFace(frame.stage, ctx, size, fade: frame.stageProgress)
            if frame.showFinePrint { drawFinePrint(ctx, size) }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    // MARK: Geometry

    private func lineY(_ size: CGSize) -> CGFloat { size.height * 0.25 }

    private var tint: Color {
        switch frame.incomingIsA {
        case .some(true): return DubColor.deckATint
        case .some(false): return DubColor.deckBTint
        case .none: return DubColor.textTertiary
        }
    }

    /// Band offset from the lock line: linear ±40 ms, asinh-compressed
    /// beyond. Round 6 (final, operator-confirmed): the inverted axis —
    /// late (φ < 0) = ABOVE the line, the gap the room has opened
    /// ahead of you; push and it settles down onto the line. Round 5's
    /// flip-back was a miscommunication.
    private func bandOffset(_ ms: Double) -> CGFloat {
        let lin = T.linearRangeMs
        let a = abs(ms)
        let sign: Double = ms < 0 ? -1 : 1
        if a <= lin { return CGFloat(ms * T.pxPerMs) }
        let maxOver = max(frame.halfBeatMs - lin, 1)
        // The compressed tail spans 2/3 of the linear span, so the
        // whole curve keeps its shape when the near-match gain is
        // retuned.
        let span = lin * T.pxPerMs * 2 / 3
        let extra = span * asinh((a - lin) / 12) / asinh(maxOver / 12)
        return CGFloat(sign * (lin * T.pxPerMs + min(extra, span)))
    }

    private func clamped(_ ms: Double) -> Bool {
        abs(ms) >= frame.halfBeatMs * 0.92
    }

    // MARK: Faces

    private func drawFace(_ stage: StillpointStage, _ ctx: GraphicsContext,
                          _ size: CGSize, fade: Double) {
        switch stage {
        case .hidden:
            drawLockLine(ctx, size, fade: fade * 0.4, offLine: false)
        case .neutral:
            drawLockLine(ctx, size, fade: fade * 0.6, offLine: false)
        case .tempo:
            drawBelt(ctx, size, fade: fade)
        case .phase, .ride:
            drawBandFace(ctx, size, fade: fade)
        }
    }

    // MARK: Lock line / hold-line

    private func drawLockLine(_ ctx: GraphicsContext, _ size: CGSize,
                              fade: Double, offLine: Bool) {
        let y = lineY(size)
        let full = size.width / 2 - 6
        if frame.lock != .none {
            // Hold-line: grows from centre 2 px/beat per side; its
            // length *is* beats-held, readable peripherally.
            let half = min(full, CGFloat(frame.holdBeats) * 2)
            let c: Color = frame.lock == .green ? DubColor.stateLocked : .white
            var p = Path()
            p.move(to: CGPoint(x: size.width / 2 - half, y: y))
            p.addLine(to: CGPoint(x: size.width / 2 + half, y: y))
            ctx.stroke(p, with: .color(c.opacity(0.9 * fade)), lineWidth: 2)
        } else {
            // The datum must never be dimmer than the indicator.
            let bright = offLine ? 0.85 : 0.6
            var p = Path()
            p.move(to: CGPoint(x: 6, y: y))
            p.addLine(to: CGPoint(x: size.width - 6, y: y))
            ctx.stroke(p, with: .color(Color(hex: 0x555555).opacity(bright * fade)),
                       lineWidth: 2)
            // Shatter: lock-loss is an event, never a slow fade.
            if let ago = frame.lockBrokenAgo {
                let t = ago / 0.45
                let half = full * CGFloat(1 - t)
                var s = Path()
                s.move(to: CGPoint(x: size.width / 2 - full, y: y))
                s.addLine(to: CGPoint(x: size.width / 2 - half, y: y))
                s.move(to: CGPoint(x: size.width / 2 + half, y: y))
                s.addLine(to: CGPoint(x: size.width / 2 + full, y: y))
                ctx.stroke(s, with: .color(DubColor.stateLocked.opacity((1 - t) * fade)),
                           lineWidth: 3)
            }
        }
    }

    // MARK: Tempo belt

    private func drawBelt(_ ctx: GraphicsContext, _ size: CGSize, fade: Double) {
        let pitch = CGFloat(T.beltPitchPx)
        let cx = size.width / 2
        let alpha = frame.alpha * fade

        // Fixed reference hairlines.
        var y = lineY(size).truncatingRemainder(dividingBy: pitch)
        while y < size.height {
            var p = Path()
            p.move(to: CGPoint(x: cx - 22, y: y))
            p.addLine(to: CGPoint(x: cx + 22, y: y))
            ctx.stroke(p, with: .color(Color(hex: 0x3A3A3A).opacity(fade)), lineWidth: 1)
            y += pitch
        }

        // The dot belt: stationary = tempo matched (the strobe null).
        // Matches the band axis (round 6, final): running slow = dots
        // climb (climbing = pitch '+'), running fast = dots sink.
        let off = CGFloat(frame.beltOffsetPx).truncatingRemainder(dividingBy: pitch)
        var dotY = lineY(size) + off
        dotY = dotY.truncatingRemainder(dividingBy: pitch)
        let r: CGFloat = 5
        let hot = min(1, abs(frame.beltVelocityPx) / T.beltMaxVelocity)
        let dotAlpha = (0.7 - 0.3 * hot) * alpha   // dim+blur stand-in past cap
        while dotY < size.height + r {
            if dotY > -r {
                let rect = CGRect(x: cx - r, y: dotY - r, width: r * 2, height: r * 2)
                if frame.beltFrozen {
                    ctx.stroke(Path(ellipseIn: rect),
                               with: .color(DubColor.textTertiary.opacity(0.4 * fade)),
                               lineWidth: 1.5)
                } else {
                    ctx.fill(Path(ellipseIn: rect), with: .color(tint.opacity(dotAlpha)))
                }
            }
            dotY += pitch
        }

        // Pitch direction, the DJ's own symbols: sinking dots = you're
        // slow = `+`. Never a fader pictogram (Technics faders invert).
        if !frame.beltFrozen, let d = frame.deltaBpm,
           abs(d) > T.beltSignFlipBpm {
            drawGlyph(ctx, size, plus: d < 0, sub: nil, fade: fade)
        }
    }

    // MARK: Band face (phase / ride)

    private func drawBandFace(_ ctx: GraphicsContext, _ size: CGSize, fade: Double) {
        let y = lineY(size)
        let cx = size.width / 2
        let alpha = frame.alpha * fade
        let bandW: CGFloat = size.width >= 100 ? 64 : 48

        let liveOffset = frame.phaseMs.map(bandOffset)

        // The pocket: ±pocketMs around the line — close enough to
        // sound clean; beats don't have to match 100 %. A quiet
        // full-width grey strip; the "how close" colour lives on the
        // pill itself. (The per-beat deposit ticks were cut in
        // round 5 — operator: distracting; the engine still records
        // them pending a subtler memory form.)
        let pocketH = CGFloat(T.pocketMs * T.pxPerMs)
        let pocket = Path(roundedRect: CGRect(x: 6, y: y - pocketH,
                                              width: size.width - 12,
                                              height: pocketH * 2),
                          cornerRadius: 4)
        ctx.fill(pocket, with: .color(Color(hex: 0x555555).opacity(0.14 * fade)))
        let proximity = frame.phaseMs.map {
            max(0.0, 1.0 - abs($0) / (T.pocketMs * 1.6))
        } ?? 0

        let offLine = frame.phaseMs.map { abs($0) > T.pocketMs } ?? false
        drawLockLine(ctx, size, fade: fade, offLine: offLine)

        // The Beat Band — your beat, drawn in the master's "now".
        // Below the line = late = push; it rises with your hand.
        let bandH: CGFloat = 10
        func bandPath(_ by: CGFloat) -> Path {
            Path(roundedRect: CGRect(x: cx - bandW / 2, y: by - bandH / 2,
                                     width: bandW, height: bandH),
                 cornerRadius: bandH / 2)
        }
        if let offset = liveOffset, let ms = frame.phaseMs {
            let by = y + offset
            // The pill carries the "sounds clean" gradient: it blends
            // toward green as it approaches the line — fully green on
            // a perfect hit, wearing off continuously with distance.
            // Certification stays the line's own statement.
            let color = blend(tint, DubColor.stateLocked, CGFloat(proximity))
            // Pinned past the half-beat rail: dimmed — the offset is
            // real but not nudge-actionable; re-drop instead.
            let pinAlpha = abs(ms) >= frame.halfBeatMs ? 0.4 : 1.0
            // Drop materialize: a brief scale pulse — no halo (cut in
            // round 5, operator: the pill needs no hollow around it).
            var scale: CGFloat = 1
            if let drop = frame.dropFiredAgo {
                scale += 0.35 * CGFloat(1 - drop / 0.5)
            }
            let rect = CGRect(x: cx - bandW * scale / 2, y: by - bandH * scale / 2,
                              width: bandW * scale, height: bandH * scale)
            ctx.fill(Path(roundedRect: rect, cornerRadius: bandH * scale / 2),
                     with: .color(color.opacity(0.9 * pinAlpha * alpha)))
            // Chevron tip past clamp — never silently saturate.
            if clamped(ms) {
                let dir: CGFloat = ms < 0 ? -1 : 1   // pointing further off-scale
                var c = Path()
                c.move(to: CGPoint(x: cx - 7, y: by + dir * bandH / 2))
                c.addLine(to: CGPoint(x: cx, y: by + dir * (bandH / 2 + 8)))
                c.addLine(to: CGPoint(x: cx + 7, y: by + dir * bandH / 2))
                ctx.fill(c, with: .color(color.opacity(0.6 * alpha)))
            }
        } else if let frozen = frame.frozenPhaseMs, !frame.withdrawn {
            // Stale-frozen ghost at the last honest offset — degraded
            // data may imitate a problem, never success. Fades out
            // over 4 s: stale should look stale, then leave (a parked
            // deck must not display a meaningless pill forever).
            let staleFade = 1 - min(1.0, (frame.frozenAgeSecs ?? 0) / 4)
            if staleFade > 0 {
                ctx.stroke(bandPath(y + bandOffset(frozen)),
                           with: .color(DubColor.textTertiary
                               .opacity(0.25 * staleFade * fade)),
                           lineWidth: 1)
            }
        }

        // Ride coach: the app quoting the DJ's own hands.
        if let c = frame.coach {
            let breath = 0.7 + 0.3 * sin(c.armedAgoSecs * 2 * .pi * 0.25)
            drawGlyph(ctx, size, plus: c.pitchUp,
                      sub: String(format: "%g", c.trimPercent),
                      fade: fade * breath)
        }
    }

    // MARK: Shared bits

    private func drawGlyph(_ ctx: GraphicsContext, _ size: CGSize,
                           plus: Bool, sub: String?, fade: Double) {
        let center = CGPoint(x: size.width / 2, y: size.height - 46)
        let r: CGFloat = 7
        ctx.stroke(Path(ellipseIn: CGRect(x: center.x - r, y: center.y - r,
                                          width: r * 2, height: r * 2)),
                   with: .color(tint.opacity(0.8 * fade)), lineWidth: 1.5)
        var t = ctx.resolve(Text(plus ? "+" : "−").font(DubFont.numericInline))
        t.shading = .color(tint.opacity(fade))
        ctx.draw(t, at: center, anchor: .center)
        if let sub {
            var s = ctx.resolve(Text(sub).font(DubFont.micro).monospacedDigit())
            s.shading = .color(DubColor.textSecondary.opacity(0.6 * fade))
            ctx.draw(s, at: CGPoint(x: center.x, y: center.y + 16), anchor: .center)
        }
    }

    private func drawFinePrint(_ ctx: GraphicsContext, _ size: CGSize) {
        let phase = frame.phaseMs.map { String(format: "%+.0f ms", $0) } ?? "— ms"
        let bpm = frame.deltaBpm.map { String(format: "%+.2f", $0) } ?? "—"
        let fold = frame.foldK != 0 ? " · 2:1" : ""
        var t = ctx.resolve(Text("\(phase) · \(bpm)\(fold)")
            .font(DubFont.micro).monospacedDigit())
        t.shading = .color(DubColor.textSecondary.opacity(0.4))
        ctx.draw(t, at: CGPoint(x: size.width / 2, y: size.height - 12), anchor: .center)
    }

    private func blend(_ a: Color, _ b: Color, _ f: CGFloat) -> Color {
        let ca = NSColor(a).usingColorSpace(.sRGB) ?? .white
        let cb = NSColor(b).usingColorSpace(.sRGB) ?? .white
        return Color(nsColor: NSColor(
            red: ca.redComponent + (cb.redComponent - ca.redComponent) * f,
            green: ca.greenComponent + (cb.greenComponent - ca.greenComponent) * f,
            blue: ca.blueComponent + (cb.blueComponent - ca.blueComponent) * f,
            alpha: 1))
    }
}

// MARK: - Previews

#Preview("Phase — late band") {
    var f = StillpointFrame()
    f.stage = .phase
    f.prevStage = .phase
    f.incomingIsA = false
    f.phaseMs = -22
    f.deltaBpm = -0.12
    return StillpointCanvas(frame: f)
        .frame(width: 120, height: 500)
        .background(DubColor.surface0)
}

#Preview("Ride — certified") {
    var f = StillpointFrame()
    f.stage = .ride
    f.prevStage = .ride
    f.incomingIsA = false
    f.phaseMs = 1
    f.deltaBpm = 0.0
    f.lock = .green
    f.holdBeats = 24
    f.showFinePrint = false
    return StillpointCanvas(frame: f)
        .frame(width: 120, height: 500)
        .background(DubColor.surface0)
}

#Preview("Tempo — belt") {
    var f = StillpointFrame()
    f.stage = .tempo
    f.prevStage = .tempo
    f.incomingIsA = true
    f.deltaBpm = -0.8
    f.beltOffsetPx = 17
    f.beltVelocityPx = -47
    return StillpointCanvas(frame: f)
        .frame(width: 120, height: 500)
        .background(DubColor.surface0)
}
