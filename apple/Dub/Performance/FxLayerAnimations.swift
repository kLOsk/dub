//
//  FxLayerAnimations.swift
//  Dub
//
//  The FX rack's moving parts, run by Core Animation.
//
//  The tape's dashes, the springs' ripple, the phaser's breathing lamps
//  and RUNAWAY's blink were `TimelineView(.animation)`s: SwiftUI rebuilt
//  and redrew each unit up to sixty times a second on the main thread for
//  as long as it was switched in — the per-frame-SwiftUI pattern that cost
//  the phase meter 57 % of the main thread on this Intel GPU. Each of
//  these is one property moving on a loop (a dash phase, an opacity), and
//  Core Animation runs that in the render server with no work from the
//  app at all. The units draw the still part once and lay one of these
//  over it.
//
//  Every unit keeps its `motion: false` path — the snapshot suite's —
//  drawn exactly as before, so the baselines hold the look.
//

import AppKit
import SwiftUI

// MARK: - Travelling dashes

/// A dashed stroke whose dashes travel along `path` at `speed` points a
/// second while `running`; still at phase 0 otherwise. `path` is in the
/// view's own top-left coordinates, as a SwiftUI `Canvas` draws.
struct TravellingDashes: NSViewRepresentable {
    let path: CGPath
    let color: Color
    let lineWidth: CGFloat
    let dash: [CGFloat]
    let speed: CGFloat
    var lineCap: CAShapeLayerLineCap = .butt
    let running: Bool

    func makeNSView(context: Context) -> TravellingDashesView {
        let v = TravellingDashesView()
        configure(v)
        return v
    }

    func updateNSView(_ v: TravellingDashesView, context: Context) {
        configure(v)
    }

    private func configure(_ v: TravellingDashesView) {
        v.apply(TravellingDashesView.Spec(
            path: path, color: NSColor(color).cgColor, lineWidth: lineWidth, dash: dash,
            speed: speed, lineCap: lineCap, running: running))
    }
}

final class TravellingDashesView: NSView {
    struct Spec {
        var path: CGPath
        var color: CGColor
        var lineWidth: CGFloat
        var dash: [CGFloat]
        var speed: CGFloat
        var lineCap: CAShapeLayerLineCap
        var running: Bool
    }

    private let shape = CAShapeLayer()
    /// What the running animation was made for; a new one only when it
    /// would differ.
    private var animated: (speed: CGFloat, period: CGFloat)?

    override var isFlipped: Bool { true }

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.masksToBounds = false
        shape.fillColor = nil
        shape.actions = ["path": NSNull(), "strokeColor": NSNull(), "lineDashPhase": NSNull()]
        layer?.addSublayer(shape)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is unused") }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    override func layout() {
        super.layout()
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        shape.frame = bounds
        CATransaction.commit()
    }

    func apply(_ spec: Spec) {
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        shape.path = spec.path
        shape.strokeColor = spec.color
        shape.lineWidth = spec.lineWidth
        shape.lineCap = spec.lineCap
        shape.lineDashPattern = spec.dash.map { NSNumber(value: Double($0)) }
        CATransaction.commit()

        let period = spec.dash.reduce(0, +)
        guard spec.running, spec.speed > 0, period > 0 else {
            shape.removeAnimation(forKey: "dashes")
            animated = nil
            return
        }
        if let animated, animated.speed == spec.speed, animated.period == period { return }
        // The SwiftUI version moved the phase by −speed·t: dashes run
        // forward along the path. One period per `period / speed` s.
        let run = CABasicAnimation(keyPath: "lineDashPhase")
        run.fromValue = 0
        run.toValue = -period
        run.duration = CFTimeInterval(period / spec.speed)
        run.repeatCount = .infinity
        run.timingFunction = CAMediaTimingFunction(name: .linear)
        run.isRemovedOnCompletion = false
        shape.add(run, forKey: "dashes")
        animated = (spec.speed, period)
    }
}

// MARK: - Pulsing lamp

/// A lamp's light, breathing at `rateHz` between dark and full while `on`
/// — laid over the lamp's dark glass. `inverted` is half a period behind,
/// for the second of a pair lit in opposition.
struct PulsingLamp: NSViewRepresentable {
    let color: Color
    let rateHz: Double
    var inverted: Bool = false
    let on: Bool

    func makeNSView(context: Context) -> PulsingLampView {
        let v = PulsingLampView()
        v.apply(color: NSColor(color).cgColor, rateHz: rateHz, inverted: inverted, on: on)
        return v
    }

    func updateNSView(_ v: PulsingLampView, context: Context) {
        v.apply(color: NSColor(color).cgColor, rateHz: rateHz, inverted: inverted, on: on)
    }
}

final class PulsingLampView: NSView {
    private let light = CAShapeLayer()
    private var animated: (rate: Double, inverted: Bool)?

    override var isFlipped: Bool { true }

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.masksToBounds = false
        light.shadowRadius = 5
        light.shadowOpacity = 1
        light.shadowOffset = .zero
        light.opacity = 0
        light.actions = ["path": NSNull(), "opacity": NSNull(), "fillColor": NSNull(), "shadowColor": NSNull()]
        layer?.addSublayer(light)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is unused") }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    override func layout() {
        super.layout()
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        light.frame = bounds
        light.path = CGPath(ellipseIn: bounds, transform: nil)
        CATransaction.commit()
    }

    func apply(color: CGColor, rateHz: Double, inverted: Bool, on: Bool) {
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        light.fillColor = color
        light.shadowColor = color
        CATransaction.commit()
        guard on, rateHz > 0 else {
            light.removeAnimation(forKey: "pulse")
            light.opacity = 0
            animated = nil
            return
        }
        if let animated, animated.rate == rateHz, animated.inverted == inverted { return }
        // Full brightness to dark and back once per LFO cycle; ease in-out
        // is close enough to the sine the knob's rate describes.
        let half = 1 / (2 * rateHz)
        let pulse = CABasicAnimation(keyPath: "opacity")
        pulse.fromValue = 0.88
        pulse.toValue = 0
        pulse.duration = half
        pulse.autoreverses = true
        pulse.repeatCount = .infinity
        pulse.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
        pulse.isRemovedOnCompletion = false
        pulse.timeOffset = inverted ? half : 0
        light.add(pulse, forKey: "pulse")
        animated = (rateHz, inverted)
    }
}

// MARK: - Blinking engraved label

/// `FxEngraved` lettering that blinks between full and 35 % twice a
/// second while `on` — RUNAWAY. Drawn on a text layer so the blink is the
/// render server's, not a SwiftUI rebuild.
struct BlinkingEngraved: NSViewRepresentable {
    let text: String
    let color: Color
    var size: CGFloat = 8
    let on: Bool

    func makeNSView(context: Context) -> BlinkingEngravedView {
        let v = BlinkingEngravedView()
        v.apply(text: text, color: NSColor(color), size: size, on: on)
        return v
    }

    func updateNSView(_ v: BlinkingEngravedView, context: Context) {
        v.apply(text: text, color: NSColor(color), size: size, on: on)
    }

    func sizeThatFits(_ proposal: ProposedViewSize, nsView: BlinkingEngravedView, context: Context) -> CGSize? {
        nsView.intrinsicContentSize
    }
}

final class BlinkingEngravedView: NSView {
    private let label = CATextLayer()
    private var string = NSAttributedString()
    private var blinking = false

    override var isFlipped: Bool { true }

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.masksToBounds = false
        label.shadowColor = NSColor.black.cgColor
        label.shadowOpacity = 0.6
        label.shadowRadius = 0
        label.shadowOffset = CGSize(width: 0, height: -1)
        label.actions = ["contents": NSNull(), "string": NSNull(), "opacity": NSNull(), "bounds": NSNull(), "position": NSNull()]
        layer?.addSublayer(label)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is unused") }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    override var intrinsicContentSize: NSSize {
        let s = string.size()
        return NSSize(width: ceil(s.width), height: ceil(s.height))
    }

    override func viewDidChangeBackingProperties() {
        super.viewDidChangeBackingProperties()
        label.contentsScale = window?.backingScaleFactor ?? 2
    }

    override func layout() {
        super.layout()
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        label.frame = bounds
        CATransaction.commit()
    }

    func apply(text: String, color: NSColor, size: CGFloat, on: Bool) {
        let next = NSAttributedString(string: text.uppercased(), attributes: [
            .font: NSFont.systemFont(ofSize: size, weight: .semibold),
            .kern: size >= 8 ? 1.1 : 0.8,
            .foregroundColor: color,
        ])
        if next != string {
            string = next
            CATransaction.begin()
            CATransaction.setDisableActions(true)
            label.string = next
            label.contentsScale = window?.backingScaleFactor ?? 2
            CATransaction.commit()
            invalidateIntrinsicContentSize()
        }
        guard on else {
            label.removeAnimation(forKey: "blink")
            label.opacity = 0.35
            blinking = false
            return
        }
        guard !blinking else { return }
        // Bright for a quarter second, dim for a quarter — the 4 Hz
        // toggle the TimelineView counted out.
        let blink = CAKeyframeAnimation(keyPath: "opacity")
        blink.values = [1, 0.35]
        blink.keyTimes = [0, 0.5]
        blink.calculationMode = .discrete
        blink.duration = 0.5
        blink.repeatCount = .infinity
        blink.isRemovedOnCompletion = false
        label.add(blink, forKey: "blink")
        blinking = true
    }
}
