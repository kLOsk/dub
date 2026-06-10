//
//  SignalQualityView.swift
//  Dub
//
//  A GUI "dub scope": live per-deck timecode signal health + calibration.
//  Surfaces the same data the CLI `dub scope` shows — carrier confidence
//  and amplitude, lock state, and a rolling **pitch-stability graph** —
//  read from the engine's lock-free deck telemetry (FFI 33). Use it to
//  check a rig before a set: a healthy SL3 carrier sits high on both
//  bars with a green lock and a flat pitch trace; dust, a worn stylus,
//  or an uncalibrated needle makes the pitch trace jump.
//
//  Calibration: the engine auto-calibrates a deck the moment it's sure
//  the input is timecode (channel-whitening capture, PRD §5.1.1). The
//  **Calibrate** button forces a fresh capture (after a cartridge swap,
//  or if the auto pass landed on a noisy window); **Auto** hands the
//  deck back to automatic source detection.
//
//  Opened from Preferences (and the `.dubShowSignalQuality`
//  notification); presented as a sheet by MainView.
//

import SwiftUI
import DubCore

struct SignalQualityView: View {

    @ObservedObject var model: WaveformAppModel
    @Environment(\.dismiss) private var dismiss

    /// Rolling pitch-% history per deck for the stability trace.
    @State private var pitchHistory: [[Double]] = [[], []]
    private static let maxSamples = 120          // ~6 s at 20 Hz
    private let tick = Timer.publish(every: 1.0 / 20.0, on: .main, in: .common).autoconnect()

    var body: some View {
        VStack(alignment: .leading, spacing: DubSpacing.lg) {
            header
            Divider()
            HStack(alignment: .top, spacing: DubSpacing.lg) {
                deckColumn(side: .a, deckIdx: 0)
                Divider()
                deckColumn(side: .b, deckIdx: 1)
            }
            Spacer(minLength: 0)
            Text("Pitch should sit flat on the centre line with the platter at 0. A jumpy trace means the needle isn't calibrated or the carrier is weak — drop the needle, wait for a green lock, then press Calibrate if it doesn't settle.")
                .font(DubFont.micro)
                .foregroundStyle(DubColor.textTertiary)
                .fixedSize(horizontal: false, vertical: true)
            Divider()
            HStack {
                Spacer()
                Button("Close") { dismiss() }
                    .keyboardShortcut(.cancelAction)
            }
        }
        .padding(DubSpacing.xl)
        .frame(width: 600, height: 520)
        .background(DubColor.surface0)
        .onReceive(tick) { _ in samplePitch() }
    }

    private func samplePitch() {
        for idx in 0..<2 {
            let t = model.engine.deckTelemetry(deckIdx: UInt64(idx))
            // Only record a live timecode pitch; a paused / no-input deck
            // would otherwise smear the trace with -100 % floor samples.
            let playing = t.hasTimecodeInput && t.lockState != 0
            pitchHistory[idx].append(playing ? t.pitchPercent : .nan)
            if pitchHistory[idx].count > Self.maxSamples {
                pitchHistory[idx].removeFirst(pitchHistory[idx].count - Self.maxSamples)
            }
        }
    }

    private var header: some View {
        HStack {
            Text("Timecode Signal Quality")
                .font(.system(size: 20, weight: .semibold))
                .foregroundStyle(DubColor.textPrimary)
            Spacer()
        }
    }

    @ViewBuilder
    private func deckColumn(side: DeckSide, deckIdx: UInt64) -> some View {
        let t = model.engine.deckTelemetry(deckIdx: deckIdx)
        let lockTint = lockColor(t.lockState, hasInput: t.hasTimecodeInput)
        VStack(alignment: .leading, spacing: DubSpacing.md) {
            HStack(spacing: DubSpacing.sm) {
                Text(side.label)
                    .font(DubFont.caps)
                    .tracking(1.0)
                    .foregroundStyle(DubColor.deckTint(side))
                Spacer()
                Circle()
                    .fill(lockTint)
                    .frame(width: 7, height: 7)
                Text(lockLabel(t.lockState, hasInput: t.hasTimecodeInput))
                    .font(DubFont.micro)
                    .foregroundStyle(DubColor.textSecondary)
            }

            bar(label: "CONFIDENCE",
                value: Double(t.carrierConfidence), of: 1.0,
                tint: lockTint,
                readout: String(format: "%.2f", t.carrierConfidence))

            bar(label: "AMPLITUDE",
                value: Double(t.carrierAmplitude), of: 0.5,
                tint: DubColor.deckTint(side),
                readout: String(format: "%.3f", t.carrierAmplitude))

            pitchTrace(side: side, telemetry: t)

            calibrationRow(telemetry: t)

            HStack(spacing: DubSpacing.sm) {
                Button("Calibrate") { try? model.engine.calibrateDeck(deckIdx: deckIdx) }
                    .disabled(!t.hasTimecodeInput)
                Button("Auto") { try? model.engine.setDeckAutoControl(deckIdx: deckIdx) }
                    .disabled(!t.controlOverridden)
                Spacer()
            }
            .font(DubFont.micro)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// Rolling pitch-% trace with a 0 reference line — the calibration
    /// visualizer. A calibrated needle at rest draws a flat line on the
    /// centre; jitter shows up as vertical wander.
    private func pitchTrace(side: DeckSide, telemetry t: DeckTelemetry) -> some View {
        VStack(alignment: .leading, spacing: DubSpacing.xs) {
            HStack {
                Text("PITCH")
                    .font(DubFont.caps).tracking(0.6)
                    .foregroundStyle(DubColor.textSecondary)
                Spacer()
                Text(t.hasTimecodeInput && t.lockState != 0
                     ? String(format: "%+.2f %%", t.pitchPercent)
                     : "—")
                    .font(DubFont.numericInline)
                    .foregroundStyle(DubColor.textPrimary)
                    .monospacedDigit()
            }
            Canvas { ctx, size in
                drawTrace(ctx, size: size,
                          history: pitchHistory[Int(side == .a ? 0 : 1)],
                          tint: lockColor(t.lockState, hasInput: t.hasTimecodeInput))
            }
            .frame(height: 56)
            .background(DubColor.surface2.opacity(0.5))
            .clipShape(RoundedRectangle(cornerRadius: 4))
        }
    }

    private func drawTrace(_ ctx: GraphicsContext, size: CGSize, history: [Double], tint: Color) {
        let midY = size.height / 2
        // Centre (0 %) reference.
        var zero = Path()
        zero.move(to: CGPoint(x: 0, y: midY)); zero.addLine(to: CGPoint(x: size.width, y: midY))
        ctx.stroke(zero, with: .color(DubColor.divider), lineWidth: 1)

        let valid = history.filter { !$0.isNaN }
        guard valid.count > 1 else {
            var t = ctx.resolve(Text("waiting for lock…").font(DubFont.micro))
            t.shading = .color(DubColor.textTertiary)
            ctx.draw(t, at: CGPoint(x: size.width / 2, y: midY + 12))
            return
        }
        // Auto-scale to the spread, floored at ±1 % so a flat trace stays
        // visibly flat instead of amplifying noise to full height.
        let peak = max(1.0, valid.map { abs($0) }.max() ?? 1.0)
        let n = history.count
        var line = Path()
        var started = false
        for (i, v) in history.enumerated() {
            guard !v.isNaN else { started = false; continue }
            let x = size.width * CGFloat(i) / CGFloat(max(1, n - 1))
            let y = midY - CGFloat(v / peak) * (size.height / 2 - 4)
            if started { line.addLine(to: CGPoint(x: x, y: y)) }
            else { line.move(to: CGPoint(x: x, y: y)); started = true }
        }
        ctx.stroke(line, with: .color(tint), style: StrokeStyle(lineWidth: 1.5, lineJoin: .round))
        // Scale caption.
        var cap = ctx.resolve(Text(String(format: "±%.1f%%", peak)).font(DubFont.micro))
        cap.shading = .color(DubColor.textTertiary)
        ctx.draw(cap, at: CGPoint(x: size.width - 4, y: 9), anchor: .trailing)
    }

    /// Source classification + control mode + calibration state line.
    private func calibrationRow(telemetry t: DeckTelemetry) -> some View {
        HStack(spacing: DubSpacing.sm) {
            tag(sourceClassLabel(t), DubColor.textSecondary)
            tag(t.controlMode == 1 ? "Timecode drive" : "Internal",
                t.controlMode == 1 ? DubColor.stateLocked : DubColor.textTertiary)
            Spacer()
            calibrationBadge(t)
            if t.controlOverridden {
                tag("PINNED", DubColor.stateTentative)
            }
        }
    }

    @ViewBuilder
    private func calibrationBadge(_ t: DeckTelemetry) -> some View {
        if t.calibrating {
            tag("Calibrating…", DubColor.stateTentative)
        } else if t.calibrated {
            tag("Calibrated ✓", DubColor.stateLocked)
        } else {
            tag("Not calibrated", DubColor.textTertiary)
        }
    }

    private func tag(_ text: String, _ color: Color) -> some View {
        Text(text)
            .font(DubFont.micro)
            .foregroundStyle(color)
    }

    private func sourceClassLabel(_ t: DeckTelemetry) -> String {
        guard t.hasTimecodeInput else { return "No input" }
        switch t.sourceClass {
        case 1:  return "Timecode"
        case 2:  return "Real record"
        default: return "Silence"
        }
    }

    /// A labelled horizontal meter. `value` is clamped to `[0, of]`.
    private func bar(label: String, value: Double, of full: Double, tint: Color, readout: String) -> some View {
        VStack(alignment: .leading, spacing: DubSpacing.xs) {
            HStack {
                Text(label)
                    .font(DubFont.caps)
                    .tracking(0.6)
                    .foregroundStyle(DubColor.textSecondary)
                Spacer()
                Text(readout)
                    .font(DubFont.micro)
                    .foregroundStyle(DubColor.textTertiary)
                    .monospacedDigit()
            }
            GeometryReader { geo in
                let frac = max(0, min(1, value / full))
                ZStack(alignment: .leading) {
                    RoundedRectangle(cornerRadius: 3)
                        .fill(DubColor.surface2)
                    RoundedRectangle(cornerRadius: 3)
                        .fill(tint)
                        .frame(width: geo.size.width * frac)
                }
            }
            .frame(height: 8)
        }
    }

    private func lockColor(_ state: UInt8, hasInput: Bool) -> Color {
        guard hasInput else { return DubColor.textPlaceholder }
        switch state {
        case 1:  return DubColor.stateLocked
        case 2:  return DubColor.stateTentative
        case 3:  return DubColor.stateError
        default: return DubColor.textPlaceholder
        }
    }

    private func lockLabel(_ state: UInt8, hasInput: Bool) -> String {
        guard hasInput else { return "No timecode input" }
        switch state {
        case 1:  return "Locked"
        case 2:  return "Degraded"
        case 3:  return "No lock / scratching"
        default: return "—"
        }
    }
}

#Preview("Signal quality") {
    SignalQualityView(model: WaveformAppModel())
}
