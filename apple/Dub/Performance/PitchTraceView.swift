//
//  PitchTraceView.swift
//  Dub
//
//  The signal panel's live part — status, scope, meters, pitch and the
//  calibration line — drawn by AppKit instead of composed in SwiftUI.
//
//  ## Why an `NSView` and not a `Canvas`
//
//  The window flushes with the display link, so on *every* display cycle
//  AppKit walks the view tree asking whether anything needs layout —
//  whether or not anything changed. The cost of that walk is proportional
//  to how much view exists, not to how often anything updates. Measured
//  with the drawer open, Performance idle: readouts at 5 Hz vs 0.25 Hz,
//  no difference; panel content vs `Color.clear`, 18 % vs 4 %. So the
//  readouts are drawn — one leaf `NSView` that samples the engine on its
//  own timer — and SwiftUI is not re-entered.
//
//  ## The layout (canvas "Dub Signal Panel", 2026-11-15)
//
//  The state as one big word with a line of advice, the carrier as a
//  scope, the input's two sides and the decode quality as bars with the
//  figure beside them, the pitch. The decoder's raw confidence and
//  amplitude numbers are gone from the face; sticker drift shows in
//  Debug builds only, for when the tracking itself is being fixed.
//

import AppKit
import DubCore
import SwiftUI

/// One deck's signal readouts. The CALIBRATE button is SwiftUI, laid
/// over the last row's trailing end (`DeckSignalPanel`).
struct SignalScopeView: NSViewRepresentable {
    let engine: DubEngine
    let deckIdx: UInt64

    static var height: CGFloat { SignalScopeNSView.height }

    func makeNSView(context: Context) -> SignalScopeNSView {
        SignalScopeNSView(engine: engine, deckIdx: deckIdx)
    }

    func updateNSView(_ view: SignalScopeNSView, context: Context) {
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

    private static let interval: TimeInterval = 1.0 / 20.0
    private var telemetry: DeckTelemetry?
    private var scope: [Float] = []
    private var timer: Timer?

    // Row geometry, top down. Fixed: the panel is a fixed width and these
    // are the only things in it.
    private static let adviceTop: CGFloat = 32
    private static let scopeTop: CGFloat = 70
    static let scopeHeight: CGFloat = 140
    private static let metersTop: CGFloat = scopeTop + scopeHeight + 14
    private static let meterRow: CGFloat = 20
    private static let pitchTop: CGFloat = metersTop + meterRow * 3 + 8
    #if DEBUG
    private static let driftTop: CGFloat = pitchTop + 26
    private static let calibrationTop: CGFloat = driftTop + 24
    #else
    private static let calibrationTop: CGFloat = pitchTop + 30
    #endif
    /// The calibration row is as tall as the button laid over its end.
    static let calibrationRow: CGFloat = 28
    static let height: CGFloat = calibrationTop + calibrationRow

    init(engine: DubEngine, deckIdx: UInt64) {
        self.engine = engine
        self.deckIdx = deckIdx
        super.init(frame: .zero)
        wantsLayer = true
        layerContentsRedrawPolicy = .onSetNeedsDisplay
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not from a nib") }

    override var isFlipped: Bool { true }
    override var acceptsFirstResponder: Bool { false }
    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    // MARK: - Sampling

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
        RunLoop.main.add(t, forMode: .common)
        timer = t
        sample()
    }

    func stop() {
        timer?.invalidate()
        timer = nil
    }

    @MainActor
    private func sample() {
        telemetry = engine.deckTelemetry(deckIdx: deckIdx)
        let fresh = engine.deckScope(deckIdx: deckIdx)
        // An idle deck redraws nothing: no scope then and none now.
        if fresh.isEmpty && scope.isEmpty && !needsTextRedraw() { return }
        scope = fresh
        needsDisplay = true
    }

    private var lastKey = ""
    /// Whether anything the text rows say has changed.
    private func needsTextRedraw() -> Bool {
        guard let t = telemetry else { return false }
        let key = "\(t.hasTimecodeInput)/\(t.lockState)/\(t.weakChannel)/\(t.calibrating)"
            + "/\(t.calibrated)/\(t.pitchSettled)/\(String(format: "%.2f", t.pitchPercent))"
            + "/\(SignalStatus.levelFigure(t.inputRmsLeft))/\(SignalStatus.levelFigure(t.inputRmsRight))"
        guard key != lastKey else { return false }
        lastKey = key
        return true
    }

    // MARK: - Drawing

    override func draw(_ dirtyRect: NSRect) {
        guard let ctx = NSGraphicsContext.current?.cgContext else { return }
        let t = telemetry ?? engine.deckTelemetry(deckIdx: deckIdx)
        let w = bounds.width
        let status = SignalStatus.of(
            hasInput: t.hasTimecodeInput, lockState: t.lockState, weakChannel: t.weakChannel,
            rms: (t.inputRmsLeft, t.inputRmsRight), pitchPercent: t.pitchPercent)
        let tint = NSColor(status.colour)

        // The state, big, and what to do about it.
        ctx.setFillColor(tint.cgColor)
        ctx.fillEllipse(in: CGRect(x: 0, y: 9, width: 10, height: 10))
        draw(status.word, at: CGPoint(x: 18, y: 0), font: .systemFont(ofSize: 22, weight: .heavy),
             colour: status.colour, tracking: 1.2)
        NSAttributedString(
            string: status.advice,
            attributes: [
                .font: NSFont.systemFont(ofSize: 12),
                .foregroundColor: NSColor(DubColor.textSecondary),
            ]
        ).draw(
            with: CGRect(x: 0, y: Self.adviceTop, width: w, height: 32),
            options: [.usesLineFragmentOrigin, .truncatesLastVisibleLine])

        drawScope(ctx, rect: CGRect(x: 0, y: Self.scopeTop, width: w, height: Self.scopeHeight),
                  tint: tint, live: status.kind != .noInput && status.kind != .noSignal)

        // The two sides of the pair and how well the carrier decodes.
        let weak = SignalStatus.weakSide(
            weakChannel: t.weakChannel, rms: (t.inputRmsLeft, t.inputRmsRight))
        let sideColour: (String) -> NSColor = { side in
            weak == side ? NSColor(DubColor.stateTentative)
                : status.kind == .weak ? NSColor(DubColor.stateLocked) : tint
        }
        var y = Self.metersTop
        meter("LEVEL L", fraction: SignalStatus.levelFraction(t.inputRmsLeft),
              figure: SignalStatus.levelFigure(t.inputRmsLeft), colour: sideColour("Left"),
              top: y, width: w, ctx: ctx)
        y += Self.meterRow
        meter("LEVEL R", fraction: SignalStatus.levelFraction(t.inputRmsRight),
              figure: SignalStatus.levelFigure(t.inputRmsRight), colour: sideColour("Right"),
              top: y, width: w, ctx: ctx)
        y += Self.meterRow
        let quality = t.hasTimecodeInput ? Double(t.carrierConfidence) : 0
        meter("QUALITY", fraction: quality,
              figure: t.hasTimecodeInput ? "\(Int((quality * 100).rounded())) %" : "—",
              colour: tint, top: y, width: w, ctx: ctx)

        // Pitch.
        ctx.setFillColor(NSColor(DubColor.divider).cgColor)
        ctx.fill(CGRect(x: 0, y: Self.pitchTop - 4, width: w, height: 1))
        draw("PITCH", at: CGPoint(x: 0, y: Self.pitchTop + 5), font: Self.label,
             colour: DubColor.textSecondary, tracking: 1)
        let live = t.hasTimecodeInput && t.lockState != 0
        draw(live ? PitchText.signed(t.pitchPercent, decimals: 2) + " %" : "—",
             at: CGPoint(x: w, y: Self.pitchTop), font: .monospacedDigitSystemFont(ofSize: 18, weight: .semibold),
             colour: live ? DubColor.textPrimary : DubColor.textTertiary, alignment: .right)

        #if DEBUG
        // Sticker drift: how far relative tracking has slid against the
        // record's own position. A tracking diagnostic, not a DJ's number.
        draw("STICKER DRIFT · DEBUG", at: CGPoint(x: 0, y: Self.driftTop + 2), font: Self.label,
             colour: DubColor.textTertiary, tracking: 1)
        let drift = t.stickerDriftMs
        draw(drift.isNaN ? "—" : String(format: "%+.1f ms", drift),
             at: CGPoint(x: w, y: Self.driftTop), font: .monospacedDigitSystemFont(ofSize: 13, weight: .medium),
             colour: drift.isNaN || abs(drift) < 5 ? DubColor.textTertiary : DubColor.stateTentative,
             alignment: .right)
        #endif

        let calibration = SignalStatus.calibration(
            hasInput: t.hasTimecodeInput, pitchSettled: t.pitchSettled,
            calibrating: t.calibrating, calibrated: t.calibrated)
        draw(calibration.text, at: CGPoint(x: 0, y: Self.calibrationTop + 6),
             font: .systemFont(ofSize: 12), colour: calibration.colour)
    }

    private static let label = NSFont.systemFont(ofSize: 10, weight: .bold)

    /// The carrier plotted left against right: a circle when healthy,
    /// squashed flat by a weak side, a smudge when there is none. Scaled
    /// to its own size with a floor, so hum stays a dot rather than
    /// blowing up to fill the frame.
    private func drawScope(_ ctx: CGContext, rect: CGRect, tint: NSColor, live: Bool) {
        let frame = CGPath(roundedRect: rect, cornerWidth: 6, cornerHeight: 6, transform: nil)
        ctx.setFillColor(NSColor(DubColor.surface0).cgColor)
        ctx.addPath(frame)
        ctx.fillPath()
        ctx.setStrokeColor(NSColor(DubColor.divider).cgColor)
        ctx.setLineWidth(1)
        ctx.addPath(frame)
        ctx.strokePath()

        let centre = CGPoint(x: rect.midX, y: rect.midY)
        let radius = rect.height / 2 - 10
        ctx.saveGState()
        ctx.setStrokeColor(NSColor(DubColor.divider).withAlphaComponent(0.6).cgColor)
        ctx.move(to: CGPoint(x: centre.x - radius - 6, y: centre.y))
        ctx.addLine(to: CGPoint(x: centre.x + radius + 6, y: centre.y))
        ctx.move(to: CGPoint(x: centre.x, y: rect.minY + 4))
        ctx.addLine(to: CGPoint(x: centre.x, y: rect.maxY - 4))
        ctx.strokePath()
        ctx.setLineDash(phase: 0, lengths: [3, 5])
        ctx.strokeEllipse(in: CGRect(
            x: centre.x - radius * 0.8, y: centre.y - radius * 0.8,
            width: radius * 1.6, height: radius * 1.6))
        ctx.restoreGState()

        let points = scope.count / 2
        guard live, points > 2 else {
            draw("WAITING FOR THE RECORD", at: CGPoint(x: rect.minX + 8, y: rect.maxY - 18),
                 font: .systemFont(ofSize: 9, weight: .semibold), colour: DubColor.textTertiary,
                 tracking: 0.8)
            return
        }
        let peak = max(0.05, scope.map { abs($0) }.max() ?? 0)
        let scale = radius * 0.8 / CGFloat(peak)
        ctx.setStrokeColor(tint.cgColor)
        ctx.setLineWidth(1.6)
        ctx.setLineJoin(.round)
        for i in 0..<points {
            let p = CGPoint(
                x: centre.x + CGFloat(scope[2 * i]) * scale,
                y: centre.y - CGFloat(scope[2 * i + 1]) * scale)
            i == 0 ? ctx.move(to: p) : ctx.addLine(to: p)
        }
        ctx.strokePath()
    }

    private func meter(
        _ label: String, fraction: Double, figure: String, colour: NSColor,
        top: CGFloat, width: CGFloat, ctx: CGContext
    ) {
        let labelWidth: CGFloat = 58
        let figureWidth: CGFloat = 46
        draw(label, at: CGPoint(x: 0, y: top + 1), font: Self.label,
             colour: DubColor.textSecondary, tracking: 0.8)
        let track = CGRect(
            x: labelWidth, y: top + 5, width: width - labelWidth - figureWidth - 6, height: 7)
        ctx.setFillColor(NSColor(DubColor.surface3).cgColor)
        ctx.addPath(CGPath(roundedRect: track, cornerWidth: 3.5, cornerHeight: 3.5, transform: nil))
        ctx.fillPath()
        let f = max(0, min(1, fraction))
        if f > 0 {
            ctx.setFillColor(colour.cgColor)
            ctx.addPath(CGPath(
                roundedRect: CGRect(x: track.minX, y: track.minY, width: track.width * f, height: track.height),
                cornerWidth: 3.5, cornerHeight: 3.5, transform: nil))
            ctx.fillPath()
        }
        draw(figure, at: CGPoint(x: width, y: top), font: .monospacedDigitSystemFont(ofSize: 11, weight: .medium),
             colour: DubColor.textTertiary, alignment: .right)
    }

    @discardableResult
    private func draw(
        _ string: String, at point: CGPoint, font: NSFont, colour: Color,
        alignment: NSTextAlignment = .left, tracking: CGFloat = 0
    ) -> CGFloat {
        var attributes: [NSAttributedString.Key: Any] = [
            .font: font,
            .foregroundColor: NSColor(colour),
        ]
        if tracking != 0 { attributes[.kern] = tracking }
        let text = NSAttributedString(string: string, attributes: attributes)
        let measured = text.size()
        let origin: CGPoint
        switch alignment {
        case .center: origin = CGPoint(x: point.x - measured.width / 2, y: point.y)
        case .right: origin = CGPoint(x: point.x - measured.width, y: point.y)
        default: origin = point
        }
        text.draw(at: origin)
        return measured.width
    }
}
