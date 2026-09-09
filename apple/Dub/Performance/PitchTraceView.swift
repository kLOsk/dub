//
//  PitchTraceView.swift
//  Dub
//
//  The signal panel's live readouts, drawn by AppKit instead of
//  composed in SwiftUI.
//
//  ## Why an `NSView` and not a `Canvas`
//
//  The panel was SwiftUI throughout, with the trace in a `Canvas`
//  inside a 20 Hz `TimelineView`. That is the idiomatic answer and it
//  is the wrong one on this surface, for a reason that took a lot of
//  measuring to pin down: the window flushes with the display link, so
//  on *every* display cycle AppKit walks the view tree asking whether
//  anything needs layout — whether or not anything changed. The cost of
//  that walk is proportional to how much view exists, not to how often
//  anything updates.
//
//  The measurements, drawer open, Performance idle:
//
//      readouts at 5 Hz vs 0.25 Hz   no difference
//      translucent vs opaque ground  no difference
//      trace at 20 Hz vs 8 Hz        1.7 points
//      panel content vs `Color.clear`   18 % vs 4 %
//
//  Only the last one moved. Rate changes did nothing; deleting view did
//  everything. So the readouts are drawn — one leaf `NSView` that
//  samples the engine on its own timer and calls `setNeedsDisplay`.
//  SwiftUI is not re-entered, no body is re-evaluated, and the panel
//  stops charging the surface around it for existing.
//
//  ## What it owns
//
//  Its samples and its timer, both. Keeping the ring buffer here rather
//  than in `@State` is the point: a 20 Hz state write is an
//  invalidation 20 times a second, which is the thing being avoided.
//  The timer is tied to window membership, so a closed drawer or an
//  ordered-out window costs nothing at all.
//
//  ## What stays in SwiftUI
//
//  The two buttons. They are controls with actions, they update only on
//  click, and hand-rolling `NSButton` here would trade a real SwiftUI
//  affordance for nothing measurable.
//

import AppKit
import DubCore
import SwiftUI

/// One deck's signal readouts: lock state, carrier confidence and
/// amplitude, live pitch with a rolling stability trace, calibration
/// state and sticker drift.
struct SignalScopeView: NSViewRepresentable {
    let engine: DubEngine
    let deckIdx: UInt64

    /// Everything the scope draws, stacked. Fixed so the panel's height
    /// never depends on a measurement.
    static let height: CGFloat = 232

    func makeNSView(context: Context) -> SignalScopeNSView {
        SignalScopeNSView(engine: engine, deckIdx: deckIdx)
    }

    func updateNSView(_ view: SignalScopeNSView, context: Context) {
        // Nothing to push: the view reads the engine itself. The deck
        // index is the only thing SwiftUI could tell it, and only if a
        // pane is rebuilt for the other deck.
        view.deckIdx = deckIdx
    }

    static func dismantleNSView(_ view: SignalScopeNSView, coordinator: ()) {
        view.stop()
    }
}

/// Layer-backed, redraws itself, tells SwiftUI nothing.
final class SignalScopeNSView: NSView {

    private let engine: DubEngine
    var deckIdx: UInt64

    /// ~6 s at 20 Hz — long enough to watch a needle settle, short
    /// enough that a bad patch scrolls off while you look at it.
    private var samples: [Double] = []
    private static let maxSamples = 120
    private static let interval: TimeInterval = 1.0 / 20.0

    private var telemetry: DeckTelemetry?
    private var timer: Timer?

    /// What the text rows currently say. Text layout is the expensive
    /// part of a redraw — ten attributed strings measured and drawn —
    /// and none of it changes twenty times a second. The trace does, so
    /// only its rectangle is invalidated per sample; the rows are
    /// invalidated when their *rendered* value actually differs.
    private var lastRows: [String] = []

    // Row geometry. Fixed offsets rather than a layout pass — the panel
    // is a fixed width and these are the only things in it.
    private static let rowGap: CGFloat = 12
    private static let barHeight: CGFloat = 8
    private static let traceHeight: CGFloat = 56
    /// Where the trace starts: lock row (15) + gap, two bars (16 + 8 +
    /// gap each), the PITCH label row (18). Constants rather than a
    /// layout pass — the panel is a fixed width and these are the only
    /// things in it.
    private static let traceTop: CGFloat = 117

    init(engine: DubEngine, deckIdx: UInt64) {
        self.engine = engine
        self.deckIdx = deckIdx
        super.init(frame: .zero)
        wantsLayer = true
        layerContentsRedrawPolicy = .onSetNeedsDisplay
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not from a nib") }

    /// Top-down coordinates, so the rows read in the order they draw.
    override var isFlipped: Bool { true }

    /// A readout, not a control — never take clicks off the panel.
    override var acceptsFirstResponder: Bool { false }
    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    // MARK: - Sampling

    /// Runs only while in a window, so a closed drawer costs nothing.
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        window == nil ? stop() : start()
    }

    func start() {
        guard timer == nil else { return }
        let t = Timer(timeInterval: Self.interval, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.sample() }
        }
        t.tolerance = Self.interval * 0.25
        // `.common`: this is the view you watch *while* adjusting
        // something, so it has to keep moving through a menu or a drag.
        RunLoop.main.add(t, forMode: .common)
        timer = t
    }

    func stop() {
        timer?.invalidate()
        timer = nil
    }

    @MainActor
    private func sample() {
        let t = engine.deckTelemetry(deckIdx: deckIdx)
        telemetry = t
        // Only a live timecode pitch is recorded; a paused or
        // input-less deck would otherwise smear the trace with the
        // -100 % floor.
        let live = t.hasTimecodeInput && t.lockState != 0
        samples.append(live ? t.pitchPercent : .nan)
        if samples.count > Self.maxSamples {
            samples.removeFirst(samples.count - Self.maxSamples)
        }
        let rows = Self.rowStrings(t)
        if rows == lastRows {
            // Only the trace moved.
            setNeedsDisplay(traceRect)
        } else {
            lastRows = rows
            needsDisplay = true
        }
    }

    /// Every string the text rows render, in draw order. Comparing
    /// these is how the view knows a redraw would look identical.
    private static func rowStrings(_ t: DeckTelemetry) -> [String] {
        [
            lockLabel(t.lockState, hasInput: t.hasTimecodeInput),
            String(format: "%.2f", t.carrierConfidence),
            String(format: "%.3f", t.carrierAmplitude),
            t.hasTimecodeInput && t.lockState != 0
                ? String(format: "%+.2f %%", t.pitchPercent) : "—",
            t.stickerDriftMs.isNaN ? "—" : String(format: "%+.1f ms", t.stickerDriftMs),
            "\(t.sourceClass)/\(t.controlMode)/\(t.calibrating)/\(t.calibrated)"
                + "/\(t.pitchSettled)/\(t.controlOverridden)/\(t.hasTimecodeInput)",
        ]
    }

    /// Where the trace sits — the one region that changes per sample.
    private var traceRect: CGRect {
        CGRect(x: 0, y: Self.traceTop, width: bounds.width, height: Self.traceHeight)
    }

    // MARK: - Drawing

    override func draw(_ dirtyRect: NSRect) {
        guard let ctx = NSGraphicsContext.current?.cgContext else { return }
        let t = telemetry ?? engine.deckTelemetry(deckIdx: deckIdx)
        let tint = NSColor(lockColor(t.lockState, hasInput: t.hasTimecodeInput))
        let w = bounds.width
        var y: CGFloat = 0

        // A sample that only moved the trace invalidates only the
        // trace, and this is the other half of that: the text rows are
        // ten attributed strings to measure and draw, and skipping them
        // is the whole saving.
        let textDirty = dirtyRect.minY < Self.traceTop
            || dirtyRect.maxY > Self.traceTop + Self.traceHeight

        // Lock state.
        if textDirty {
            ctx.setFillColor(tint.cgColor)
            ctx.fillEllipse(in: CGRect(x: 0, y: y + 4, width: 7, height: 7))
            draw(lockLabel(t.lockState, hasInput: t.hasTimecodeInput),
                 at: CGPoint(x: 13, y: y), size: 11, colour: DubColor.textSecondary)
            y += 15 + Self.rowGap

            y = bar("CONFIDENCE", value: Double(t.carrierConfidence), of: 1.0, tint: tint,
                    readout: String(format: "%.2f", t.carrierConfidence),
                    width: w, top: y, ctx: ctx)
            y = bar("AMPLITUDE", value: Double(t.carrierAmplitude), of: 0.6, tint: tint,
                    readout: String(format: "%.3f", t.carrierAmplitude),
                    width: w, top: y, ctx: ctx)

            // Pitch: label and live value, above the rolling trace.
            draw("PITCH", at: CGPoint(x: 0, y: y), size: 11,
                 colour: DubColor.textSecondary, tracking: 0.6)
            draw(t.hasTimecodeInput && t.lockState != 0
                    ? String(format: "%+.2f %%", t.pitchPercent) : "—",
                 at: CGPoint(x: w, y: y - 1), size: 14, colour: DubColor.textPrimary,
                 alignment: .right, monospaced: true)
        }
        y = Self.traceTop
        drawTrace(ctx, rect: CGRect(x: 0, y: y, width: w, height: Self.traceHeight),
                  tint: tint)
        y += Self.traceHeight + Self.rowGap
        guard textDirty else { return }

        // Calibration state, as a row of tags.
        var x: CGFloat = 0
        x = tag(sourceClassLabel(t), DubColor.textSecondary, at: CGPoint(x: x, y: y))
        x = tag(t.controlMode == 1 ? "Timecode drive" : "Internal",
                t.controlMode == 1 ? DubColor.stateLocked : DubColor.textTertiary,
                at: CGPoint(x: x, y: y))
        // One story at a time: while the deck is still measuring, a
        // green "Calibrated ✓" beside "Measuring…" read as a
        // contradiction on the rig.
        let badge: (String, Color) = !t.pitchSettled
            ? ("Measuring…", DubColor.stateTentative)
            : t.calibrating ? ("Calibrating…", DubColor.stateTentative)
            : t.calibrated ? ("Calibrated ✓", DubColor.stateLocked)
            : ("Not calibrated", DubColor.textTertiary)
        drawRight(badge.0, colour: badge.1, right: w, y: y)
        y += 14 + Self.rowGap

        // Sticker drift.
        draw("STICKER DRIFT", at: CGPoint(x: 0, y: y), size: 11,
             colour: DubColor.textSecondary, tracking: 0.6)
        let drift = t.stickerDriftMs
        draw(drift.isNaN ? "—" : String(format: "%+.1f ms", drift),
             at: CGPoint(x: w, y: y - 1), size: 14,
             colour: drift.isNaN ? DubColor.textTertiary
                : (abs(drift) < 5 ? DubColor.textPrimary : DubColor.stateTentative),
             alignment: .right, monospaced: true)
    }

    /// A labelled horizontal meter. Returns the next row's top.
    private func bar(
        _ label: String, value: Double, of full: Double, tint: NSColor,
        readout: String, width: CGFloat, top: CGFloat, ctx: CGContext
    ) -> CGFloat {
        draw(label, at: CGPoint(x: 0, y: top), size: 11,
             colour: DubColor.textSecondary, tracking: 0.6)
        draw(readout, at: CGPoint(x: width, y: top), size: 11,
             colour: DubColor.textTertiary, alignment: .right, monospaced: true)
        let y = top + 16
        let track = CGRect(x: 0, y: y, width: width, height: Self.barHeight)
        ctx.setFillColor(NSColor(DubColor.surface2).cgColor)
        ctx.addPath(CGPath(roundedRect: track, cornerWidth: 3, cornerHeight: 3,
                           transform: nil))
        ctx.fillPath()
        let frac = max(0, min(1, value / full))
        if frac > 0 {
            let fill = CGRect(x: 0, y: y, width: width * frac, height: Self.barHeight)
            ctx.setFillColor(tint.cgColor)
            ctx.addPath(CGPath(roundedRect: fill, cornerWidth: 3, cornerHeight: 3,
                               transform: nil))
            ctx.fillPath()
        }
        return y + Self.barHeight + Self.rowGap
    }

    private func drawTrace(_ ctx: CGContext, rect: CGRect, tint: NSColor) {
        ctx.setFillColor(NSColor(DubColor.surface2).withAlphaComponent(0.5).cgColor)
        ctx.addPath(CGPath(roundedRect: rect, cornerWidth: 4, cornerHeight: 4,
                           transform: nil))
        ctx.fillPath()

        let midY = rect.midY
        ctx.setStrokeColor(NSColor(DubColor.divider).cgColor)
        ctx.setLineWidth(1)
        ctx.move(to: CGPoint(x: rect.minX, y: midY))
        ctx.addLine(to: CGPoint(x: rect.maxX, y: midY))
        ctx.strokePath()

        let valid = samples.filter { !$0.isNaN }
        guard valid.count > 1 else {
            draw("waiting for lock…",
                 at: CGPoint(x: rect.midX, y: midY + 2), size: 11,
                 colour: DubColor.textTertiary, alignment: .center)
            return
        }
        // Auto-scale to the spread, floored at ±1 % so a flat trace
        // stays visibly flat instead of amplifying noise to full
        // height.
        let peak = max(1.0, valid.map { abs($0) }.max() ?? 1.0)
        let n = samples.count
        ctx.setStrokeColor(tint.cgColor)
        ctx.setLineWidth(1.5)
        ctx.setLineJoin(.round)
        var started = false
        for (i, v) in samples.enumerated() {
            guard !v.isNaN else { started = false; continue }
            let x = rect.minX + rect.width * CGFloat(i) / CGFloat(max(1, n - 1))
            let y = midY - CGFloat(v / peak) * (rect.height / 2 - 4)
            if started {
                ctx.addLine(to: CGPoint(x: x, y: y))
            } else {
                ctx.move(to: CGPoint(x: x, y: y))
                started = true
            }
        }
        ctx.strokePath()
        draw(String(format: "±%.1f%%", peak),
             at: CGPoint(x: rect.maxX - 4, y: rect.minY + 3), size: 11,
             colour: DubColor.textTertiary, alignment: .right)
    }

    // MARK: - Text

    private func sourceClassLabel(_ t: DeckTelemetry) -> String {
        guard t.hasTimecodeInput else { return "No input" }
        switch t.sourceClass {
        case 1:  return "Timecode"
        case 2:  return "Real record"
        default: return "Silence"
        }
    }

    /// Draws a tag and returns the x to continue at.
    private func tag(_ string: String, _ colour: Color, at point: CGPoint) -> CGFloat {
        let width = draw(string, at: point, size: 11, colour: colour)
        return point.x + width + 8
    }

    private func drawRight(_ string: String, colour: Color, right: CGFloat, y: CGFloat) {
        draw(string, at: CGPoint(x: right, y: y), size: 11, colour: colour,
             alignment: .right)
    }

    @discardableResult
    private func draw(
        _ string: String, at point: CGPoint, size: CGFloat, colour: Color,
        alignment: NSTextAlignment = .left, monospaced: Bool = false,
        tracking: CGFloat = 0
    ) -> CGFloat {
        var attributes: [NSAttributedString.Key: Any] = [
            .font: monospaced
                ? NSFont.monospacedDigitSystemFont(ofSize: size, weight: .medium)
                : NSFont.systemFont(ofSize: size, weight: .regular),
            .foregroundColor: NSColor(colour),
        ]
        if tracking != 0 { attributes[.kern] = tracking }
        let text = NSAttributedString(string: string, attributes: attributes)
        let measured = text.size()
        let origin: CGPoint
        switch alignment {
        case .center: origin = CGPoint(x: point.x - measured.width / 2, y: point.y)
        case .right:  origin = CGPoint(x: point.x - measured.width, y: point.y)
        default:      origin = point
        }
        text.draw(at: origin)
        return measured.width
    }
}
