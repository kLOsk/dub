//
//  DeckSignalPanel.swift
//  Dub
//
//  Per-deck timecode signal health as a deck-pane slide-out — the GUI
//  "dub scope", moved out of Preferences and onto the deck where a DJ
//  actually sound-checks. A slim SIGNAL tab sits on each deck's outer
//  edge; clicking it slides the panel over the performance-pads area
//  (an overlay, so the waveform column never reflows).
//
//  Surfaces the engine's lock-free deck telemetry (FFI 33): carrier
//  confidence and amplitude, lock state, a rolling pitch-stability
//  trace, calibration state, and the sticker-drift readout. A healthy
//  carrier sits high on both bars with a green lock and a flat pitch
//  trace; dust, a worn stylus, or an uncalibrated needle makes the
//  trace jump.
//
//  Calibration: the engine auto-calibrates the moment it's sure the
//  input is timecode (PRD §5.1.1). **Calibrate** forces a fresh capture
//  (after a cartridge swap, or if the auto pass landed on a noisy
//  window); **Auto** hands the deck back to automatic source detection.
//

import SwiftUI
import DubCore

/// Outer-edge tab + slide-out signal panel for one deck. Deck A slides
/// out from the window-left edge, deck B from the window-right.
struct DeckSignalSlideOut: View {

    @ObservedObject var model: WaveformAppModel
    let side: DeckSide
    let deckIdx: UInt64

    @State private var open = false

    var body: some View {
        HStack(spacing: 0) {
            if side == .a {
                if open { panel }
                tab
            } else {
                tab
                if open { panel }
            }
        }
        .animation(.spring(duration: 0.25), value: open)
        .frame(maxWidth: .infinity, maxHeight: .infinity,
               alignment: side == .a ? .leading : .trailing)
    }

    private var panel: some View {
        DeckSignalPanel(model: model, side: side, deckIdx: deckIdx)
            .transition(.move(edge: side == .a ? .leading : .trailing)
                .combined(with: .opacity))
    }

    /// Slim always-visible toggle: vertical SIGNAL caps + the PRD §5.4
    /// tracking dot, so signal health is glanceable even while closed.
    private var tab: some View {
        let t = model.engine.deckTelemetry(deckIdx: deckIdx)
        return Button {
            open.toggle()
        } label: {
            VStack(spacing: DubSpacing.sm) {
                Circle()
                    .fill(lockColor(t.lockState, hasInput: t.hasTimecodeInput))
                    .frame(width: 6, height: 6)
                Text("SIGNAL")
                    .font(DubFont.caps)
                    .tracking(1.2)
                    .foregroundStyle(open ? DubColor.textPrimary : DubColor.textTertiary)
                    .fixedSize()
                    .rotationEffect(.degrees(side == .a ? 90 : -90))
                    .frame(width: 10, height: 56)
            }
            .frame(width: 18, height: 96)
            .background(DubColor.surface2.opacity(open ? 1.0 : 0.6))
            .clipShape(RoundedRectangle(cornerRadius: 4, style: .continuous))
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Toggle deck \(side.label) signal panel")
    }
}

/// The panel body: one deck's signal health + calibration controls.
struct DeckSignalPanel: View {

    @ObservedObject var model: WaveformAppModel
    let side: DeckSide
    let deckIdx: UInt64

    /// Rolling pitch-% history for the stability trace (~6 s at 20 Hz).
    @State private var pitchHistory: [Double] = []
    private static let maxSamples = 120
    private let tick = Timer.publish(every: 1.0 / 20.0, on: .main, in: .common).autoconnect()

    var body: some View {
        let t = model.engine.deckTelemetry(deckIdx: deckIdx)
        let lockTint = lockColor(t.lockState, hasInput: t.hasTimecodeInput)
        VStack(alignment: .leading, spacing: DubSpacing.md) {
            HStack(spacing: DubSpacing.sm) {
                Circle()
                    .fill(lockTint)
                    .frame(width: 7, height: 7)
                Text(lockLabel(t.lockState, hasInput: t.hasTimecodeInput))
                    .font(DubFont.micro)
                    .foregroundStyle(DubColor.textSecondary)
                Spacer()
            }

            bar(label: "CONFIDENCE",
                value: Double(t.carrierConfidence), of: 1.0,
                tint: lockTint,
                readout: String(format: "%.2f", t.carrierConfidence))

            bar(label: "AMPLITUDE",
                value: Double(t.carrierAmplitude), of: 0.5,
                tint: DubColor.deckTint(side),
                readout: String(format: "%.3f", t.carrierAmplitude))

            pitchTrace(telemetry: t)

            calibrationRow(telemetry: t)

            driftRow(telemetry: t)

            HStack(spacing: DubSpacing.sm) {
                Button("Calibrate") { try? model.engine.calibrateDeck(deckIdx: deckIdx) }
                    .disabled(!t.hasTimecodeInput)
                Button("Auto") { try? model.engine.setDeckAutoControl(deckIdx: deckIdx) }
                    .disabled(!t.controlOverridden)
                Spacer()
            }
            .font(DubFont.micro)

            Spacer(minLength: 0)
        }
        .padding(DubSpacing.md)
        .frame(width: 236)
        .frame(maxHeight: .infinity)
        .background(DubColor.surface1.opacity(0.97))
        .overlay(
            Rectangle()
                .fill(DubColor.divider)
                .frame(width: 1),
            alignment: side == .a ? .trailing : .leading
        )
        .onReceive(tick) { _ in samplePitch() }
    }

    private func samplePitch() {
        let t = model.engine.deckTelemetry(deckIdx: deckIdx)
        // Only record a live timecode pitch; a paused / no-input deck
        // would otherwise smear the trace with -100 % floor samples.
        let playing = t.hasTimecodeInput && t.lockState != 0
        pitchHistory.append(playing ? t.pitchPercent : .nan)
        if pitchHistory.count > Self.maxSamples {
            pitchHistory.removeFirst(pitchHistory.count - Self.maxSamples)
        }
    }

    /// Rolling pitch-% trace with a 0 reference line — the calibration
    /// visualizer. A calibrated needle at rest draws a flat line on the
    /// centre; jitter shows up as vertical wander.
    private func pitchTrace(telemetry t: DeckTelemetry) -> some View {
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
                          history: pitchHistory,
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
            // One story at a time: while the deck is still measuring
            // (whitening + pitch stabilization — the same condition
            // that holds playback and draws the header line), showing
            // a green "Calibrated ✓" next to "Measuring…" read as a
            // contradiction on-rig. The whitening badge only appears
            // once the deck is fully ready.
            if !t.pitchSettled {
                tag("Measuring…", DubColor.stateTentative)
            } else {
                calibrationBadge(t)
            }
            if t.controlOverridden {
                tag("PINNED", DubColor.stateTentative)
            }
        }
    }

    /// Sticker-drift readout: how far the relative-mode playhead has
    /// slid against the absolute groove position since the engagement
    /// anchor. Measured live off the LFSR decode while ABS-locked;
    /// holds the last reading through relative-only gaps. NaN until the
    /// first locked observation.
    private func driftRow(telemetry t: DeckTelemetry) -> some View {
        HStack {
            Text("STICKER DRIFT")
                .font(DubFont.caps).tracking(0.6)
                .foregroundStyle(DubColor.textSecondary)
            Spacer()
            if t.stickerDriftMs.isNaN {
                Text("—")
                    .font(DubFont.numericInline)
                    .foregroundStyle(DubColor.textTertiary)
            } else {
                Text(String(format: "%+.1f ms", t.stickerDriftMs))
                    .font(DubFont.numericInline)
                    .foregroundStyle(abs(t.stickerDriftMs) < 5
                                     ? DubColor.textPrimary
                                     : DubColor.stateTentative)
                    .monospacedDigit()
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
}

/// Shared lock-state → colour/label mapping (PRD §5.4 tracking dot).
func lockColor(_ state: UInt8, hasInput: Bool) -> Color {
    guard hasInput else { return DubColor.textPlaceholder }
    switch state {
    case 1:  return DubColor.stateLocked
    case 2:  return DubColor.stateTentative
    case 3:  return DubColor.stateError
    default: return DubColor.textPlaceholder
    }
}

func lockLabel(_ state: UInt8, hasInput: Bool) -> String {
    guard hasInput else { return "No timecode input" }
    switch state {
    case 1:  return "Locked"
    case 2:  return "Degraded"
    case 3:  return "No lock / scratching"
    default: return "—"
    }
}

#Preview("Deck signal panel") {
    DeckSignalPanel(model: WaveformAppModel(), side: .a, deckIdx: 0)
}
