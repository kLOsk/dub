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

    /// The engine, not the model.
    ///
    /// Both views here only ever reached through to `model.engine`, but
    /// observing the model meant every published change anywhere in the
    /// app — playhead, deck state, telemetry heartbeat — re-rendered
    /// the panel *and* made a fresh `deckTelemetry` call across the
    /// FFI, because that call sat inside `body`. During playback that
    /// is continuous, and it is why the drawer felt laggy. Telemetry is
    /// polled on its own timer now and nothing else can trigger a
    /// redraw.
    let engine: DubEngine
    let side: DeckSide
    let deckIdx: UInt64

    /// Polled, not read inside `body`. Four times a second is plenty
    /// for a lock dot, and it holds whether the panel is open or shut.
    @State private var telemetry: DeckTelemetry?

    /// **`@State`, not `let`.** A timer stored as a plain property is
    /// rebuilt every time SwiftUI recreates the struct, and this view
    /// sits inside the deck pane, which re-renders continuously while a
    /// deck plays. Each rebuild scheduled a fresh run-loop timer and
    /// re-subscribed `onReceive` to it — dozens per second, per deck,
    /// on the main thread in `.common` mode, which is why dragging the
    /// window was where it hurt most. `@State` creates it once and
    /// keeps it for the view's lifetime.
    @State private var dotTick =
        Timer.publish(every: 0.25, on: .main, in: .common).autoconnect()

    /// The one curve the drawer moves on — the panel's transition and
    /// the tab's position change are the same motion and must share it.
    static let slide = Animation.spring(duration: 0.25)

    /// `-signalOpen` starts the drawer open, for profiling it without
    /// a click. The panel is the most expensive thing on the surface
    /// when it is showing, so being able to measure it repeatably is
    /// worth one launch argument.
    @State private var open = ProcessInfo.processInfo.arguments.contains("-signalOpen")

    var body: some View {
        HStack(spacing: 0) {
            if side == .a {
                panel
                tab
            } else {
                tab
                panel
            }
        }
        // One animation for the pair. The tab is laid out by this
        // `HStack`, so when the panel appears the tab's *position*
        // changes — and the panel arrived on a transition with its own
        // timing while the tab moved on this one. They ran at different
        // speeds and a gap opened between them mid-slide. Same curve,
        // same duration, driven by the same value.
        .animation(DeckSignalSlideOut.slide, value: open)
        .frame(maxWidth: .infinity, maxHeight: .infinity,
               alignment: side == .a ? .leading : .trailing)
    }

    /// Always in the tree, its *width* animated between zero and full.
    ///
    /// It used to be inserted and removed on a `.move` transition. That
    /// is a different mechanism from the layout change beside it: the
    /// `HStack` allocated the panel's full width the instant it
    /// appeared, so the tab jumped to its final position while the
    /// panel was still sliding in behind — the gap you could watch open
    /// and close. Animating the width makes the tab's position and the
    /// panel's edge the same number, so they cannot disagree.
    private var panel: some View {
        Group {
            if open {
                DeckSignalPanel(engine: engine, side: side, deckIdx: deckIdx)
            }
        }
        .frame(width: open ? DubLayout.deckSignalPanelWidth : 0)
        .clipped()
        .opacity(open ? 1 : 0)
    }

    /// Slim always-visible toggle: vertical SIGNAL caps + the PRD §5.4
    /// tracking dot, so signal health is glanceable even while closed.
    private var tab: some View {
        let t = telemetry ?? engine.deckTelemetry(deckIdx: deckIdx)
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
        .onReceive(dotTick) { _ in telemetry = engine.deckTelemetry(deckIdx: deckIdx) }
    }
}

/// The pitch trace's ring buffer.
///
/// A class so the timeline closure can append to it without writing
/// view state — mutating `@State` from inside a `TimelineView` body is
/// both a side effect during evaluation and, at 20 Hz, an invalidation
/// storm. `StillpointView` holds its engine the same way.
@MainActor
final class PitchTrace {
    private var samples: [Double] = []
    private static let maxSamples = 120

    /// Record this tick and return the window to draw. Only a *live*
    /// timecode pitch is recorded; a paused or input-less deck would
    /// otherwise smear the trace with -100 % floor samples.
    func push(_ t: DeckTelemetry) -> [Double] {
        let live = t.hasTimecodeInput && t.lockState != 0
        samples.append(live ? t.pitchPercent : .nan)
        if samples.count > Self.maxSamples {
            samples.removeFirst(samples.count - Self.maxSamples)
        }
        return samples
    }
}

/// The panel body: one deck's signal health + calibration controls.
struct DeckSignalPanel: View {

    let engine: DubEngine
    let side: DeckSide
    let deckIdx: UInt64

    /// Rolling pitch-% history for the stability trace (~6 s at 20 Hz).
    ///
    /// A reference type mutated inside the timeline closure, the same
    /// shape `StillpointView` uses for its engine. It was `@State` fed
    /// by a `Timer`, which meant two state writes per tick — one for
    /// the history, one for the telemetry — and each of those re-ran
    /// the whole view's body and invalidated layout. An open drawer
    /// cost 15 % of a core with nothing moving.
    @State private var trace = PitchTrace()

    var body: some View {
        // **Two cadences.** Only the pitch trace needs 20 Hz — it is a
        // stability visualiser, and a sample every 50 ms is the point
        // of it. The readouts around it are numbers a human reads:
        // five times a second is already faster than that is useful.
        // Redrawing the whole panel at 20 Hz cost 20 % of a core with
        // nothing moving, because every tick re-ran the bars, the rows
        // and the buttons as well as the one Canvas that had changed.
        TimelineView(.periodic(from: .now, by: 1.0 / 5.0)) { _ in
            content(engine.deckTelemetry(deckIdx: deckIdx))
        }
        .padding(DubSpacing.md)
        .frame(width: DubLayout.deckSignalPanelWidth)
        .frame(maxHeight: .infinity)
        .background(DubColor.surface2.opacity(0.97))
        .overlay(
            Rectangle()
                .fill(DubColor.divider)
                .frame(width: 1),
            alignment: side == .a ? .trailing : .leading)
    }

    @ViewBuilder
    private func content(_ t: DeckTelemetry) -> some View {
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
                Button("Calibrate") { try? engine.calibrateDeck(deckIdx: deckIdx) }
                    .disabled(!t.hasTimecodeInput)
                Button("Auto") { try? engine.setDeckAutoControl(deckIdx: deckIdx) }
                    .disabled(!t.controlOverridden)
                Spacer()
            }
            .font(DubFont.micro)

            Spacer(minLength: 0)
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
            // The one thing here that runs at 20 Hz, and it is its own
            // `TimelineView` so the rate stays inside this Canvas
            // rather than dragging the panel around it along.
            TimelineView(.periodic(from: .now, by: 1.0 / 20.0)) { _ in
                let live = engine.deckTelemetry(deckIdx: deckIdx)
                Canvas { ctx, size in
                    drawTrace(ctx, size: size,
                              history: trace.push(live),
                              tint: lockColor(
                                live.lockState, hasInput: live.hasTimecodeInput))
                }
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
    DeckSignalPanel(engine: WaveformAppModel().engine, side: .a, deckIdx: 0)
}
