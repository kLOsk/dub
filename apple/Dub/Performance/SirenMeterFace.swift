//
//  SirenMeterFace.swift
//  Dub
//
//  The VU face of the siren box's meter — see `SirenDisplay` for what
//  drives the needle and why it is drawn rather than measured.
//

import AppKit
import SwiftUI

/// The dial: a static drawing of the face plus a needle at `level`.
/// Split from the view above so the face is one `Canvas` — a frame at
/// 30 Hz redraws a handful of paths and three strings, nothing more.
struct SirenMeterFace: View {
    let title: String
    let echoLine: String
    /// 0…1 along the scale.
    let level: Double
    /// Off when the needle is drawn live on a layer over the face
    /// (`MeterNeedle`) — a level through SwiftUI redraws the face, and a
    /// live one redrew the DUB FX pane thirty times a second.
    var drawsNeedle: Bool = true

    /// The face sits inset in the window; the scale is an arc about a
    /// pivot just below the face, its sweep from the left stop to the
    /// right end. Angles in screen degrees (clockwise, 0 = right).
    private static let inset: CGFloat = 4
    private static let startDeg: Double = 200
    private static let sweepDeg: Double = 140
    /// The red zone begins at +1.
    private static let redFrom: Double = 0.84
    private static let radius: CGFloat = 36

    var body: some View {
        Canvas { ctx, size in
            let face = CGRect(x: Self.inset, y: Self.inset,
                              width: size.width - Self.inset * 2,
                              height: size.height - Self.inset * 2)
            let pivot = Self.pivot(in: size)
            let faceShape = Path(roundedRect: face, cornerRadius: 3)
            ctx.fill(faceShape, with: .color(DubColor.meterFace))
            ctx.stroke(faceShape, with: .color(DubColor.meterBezel), lineWidth: 1)

            // The scale.
            ctx.stroke(arc(pivot, from: 0, to: 1), with: .color(DubColor.meterInk), lineWidth: 1.2)
            ctx.stroke(arc(pivot, from: Self.redFrom, to: 1), with: .color(DubColor.meterRed), lineWidth: 3)
            let marks: [(Double, String, Bool)] = [
                (0, "-20", false), (0.28, "-10", false), (0.5, "-5", false),
                (0.65, "0", false), (0.78, "+1", false), (0.92, "+3", true),
            ]
            for (at, label, red) in marks {
                var tick = Path()
                tick.move(to: point(pivot, at: at, radius: Self.radius))
                tick.addLine(to: point(pivot, at: at, radius: Self.radius - 4))
                ctx.stroke(tick, with: .color(red ? DubColor.meterRed : DubColor.meterInk), lineWidth: 1)
                ctx.draw(
                    Text(label)
                        .font(.system(size: 5, weight: .semibold))
                        .foregroundColor(red ? DubColor.meterRed : DubColor.meterInk),
                    at: point(pivot, at: at, radius: Self.radius + 5), anchor: .center)
            }

            // Printed on the face: the shot where "VU" would be, the echo
            // line under it.
            ctx.draw(
                Text(title)
                    .font(.system(size: 6.5, weight: .heavy))
                    .tracking(1)
                    .foregroundColor(DubColor.meterInk),
                at: CGPoint(x: face.midX, y: face.maxY - 17), anchor: .center)
            ctx.draw(
                Text(echoLine)
                    .font(.system(size: 4.5, weight: .regular, design: .monospaced))
                    .foregroundColor(DubColor.meterInkSoft),
                at: CGPoint(x: face.midX, y: face.maxY - 5), anchor: .center)

            guard drawsNeedle else { return }
            // The needle and its pivot.
            var needle = Path()
            needle.move(to: pivot)
            needle.addLine(to: Self.needleTip(pivot, level: level))
            ctx.stroke(needle, with: .color(DubColor.meterNeedle),
                       style: StrokeStyle(lineWidth: 1.3, lineCap: .round))
            ctx.fill(
                Path(ellipseIn: CGRect(x: pivot.x - 2.4, y: pivot.y - 2.4, width: 4.8, height: 4.8)),
                with: .color(DubColor.meterNeedle))
        }
    }

    /// Where the needle turns, for a face of `size` — shared with the
    /// live needle layer so it lands on the face drawn here.
    static func pivot(in size: CGSize) -> CGPoint {
        CGPoint(x: size.width / 2, y: size.height - inset - 6)
    }

    static func needleTip(_ pivot: CGPoint, level: Double) -> CGPoint {
        point(pivot, at: min(max(level, 0), 1), radius: radius - 2)
    }

    private static func angle(at level: Double) -> Double {
        (startDeg + sweepDeg * level) * .pi / 180
    }

    private static func point(_ pivot: CGPoint, at level: Double, radius: CGFloat) -> CGPoint {
        let a = angle(at: level)
        return CGPoint(x: pivot.x + radius * cos(a), y: pivot.y + radius * sin(a))
    }

    private func angle(at level: Double) -> Double { Self.angle(at: level) }

    private func point(_ pivot: CGPoint, at level: Double, radius: CGFloat) -> CGPoint {
        Self.point(pivot, at: level, radius: radius)
    }

    private func arc(_ pivot: CGPoint, from: Double, to: Double) -> Path {
        var p = Path()
        p.addArc(
            center: pivot, radius: Self.radius,
            startAngle: .radians(angle(at: from)), endAngle: .radians(angle(at: to)),
            clockwise: false)
        return p
    }
}

/// A VU needle on a Core Animation layer over a `SirenMeterFace` drawn
/// with `drawsNeedle: false`, moved thirty times a second from `read`.
/// No SwiftUI update, no redraw of the face: the level is a property
/// the compositor applies (the DUB FX pane's input meter, 2026-09-26).
struct MeterNeedle: NSViewRepresentable {
    /// 0…1 along the scale.
    let read: () -> Double

    func makeNSView(context: Context) -> MeterNeedleView {
        let v = MeterNeedleView()
        v.read = read
        return v
    }

    func updateNSView(_ v: MeterNeedleView, context: Context) {
        v.read = read
    }
}

final class MeterNeedleView: NSView {
    var read: () -> Double = { 0 }

    private static let interval: TimeInterval = 1.0 / 30.0
    private let needle = CAShapeLayer()
    private let hub = CAShapeLayer()
    private var shown: Double = -1
    private var shownSize: CGSize = .zero
    private var timer: Timer?

    override var isFlipped: Bool { true }

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        let none: [String: CAAction] = ["path": NSNull(), "position": NSNull(), "bounds": NSNull()]
        needle.actions = none
        hub.actions = none
        needle.strokeColor = NSColor(DubColor.meterNeedle).cgColor
        needle.fillColor = nil
        needle.lineWidth = 1.3
        needle.lineCap = .round
        hub.fillColor = NSColor(DubColor.meterNeedle).cgColor
        layer?.addSublayer(needle)
        layer?.addSublayer(hub)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is unused") }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        timer?.invalidate()
        timer = nil
        guard window != nil else { return }
        let t = Timer(timeInterval: Self.interval, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.refresh() }
        }
        RunLoop.main.add(t, forMode: .common)
        timer = t
        refresh()
    }

    override func layout() {
        super.layout()
        shown = -1
        refresh()
    }

    private func refresh() {
        let level = read()
        let size = bounds.size
        guard abs(level - shown) > 0.002 || size != shownSize else { return }
        shown = level
        shownSize = size
        let pivot = SirenMeterFace.pivot(in: size)
        let path = CGMutablePath()
        path.move(to: pivot)
        path.addLine(to: SirenMeterFace.needleTip(pivot, level: level))
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        needle.path = path
        hub.path = CGPath(
            ellipseIn: CGRect(x: pivot.x - 2.4, y: pivot.y - 2.4, width: 4.8, height: 4.8),
            transform: nil)
        CATransaction.commit()
    }
}
