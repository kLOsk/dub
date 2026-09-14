//
//  DubKnob.swift
//  Dub
//
//  A rotary with a printed scale — the one knob on the performance
//  surface. Replaces the stock `Slider` the siren's DUB macro used,
//  which said DUB and nothing about what it did; the dial carries its
//  scale ends as words and hands the caller a readout slot underneath
//  for the numbers.
//
//  The knob itself is the one on every 70s pedal and amp — the
//  Davies-1900 / MXR pattern Daniel sent a photo of: a black phenolic
//  body with eight deep flutes, a spun-aluminium cap with its
//  diamond-cut rings, and a white index line painted down one flute.
//  The body turns with the value, so the index line is always on a
//  lobe; the cap's sheen stays with the light, as it does on the desk.
//  The scale is printed on the plate around it, as a panel would be.
//
//  Drag vertically to turn (150 pt of travel for the full arc, the
//  macOS convention), double-click to return to the low end. Not a
//  performance gesture: it is the Advanced macro, set before or between
//  tunes; the live one is a MIDI knob once map mode lands (PRD §6.3).
//

import SwiftUI

/// One dial, 0…1 over a 270° arc, `low` and `high` printed at its ends.
struct DubKnob: View {
    /// 0…1.
    let value: Double
    /// The words at the two ends of the scale — `DRY` / `DUB`.
    var low: String = ""
    var high: String = ""
    var tint: Color = DubColor.siren
    var onChange: (_ value: Double) -> Void = { _ in }

    /// Points of vertical drag for the full turn.
    private static let travel: CGFloat = 150
    private static let size: CGFloat = 60
    /// The arc runs from 135° (bottom-left) clockwise through the top to
    /// 45° (bottom-right): 270° of the circle, the gap at the bottom.
    private static let startDegrees: Double = 135
    private static let sweepDegrees: Double = 270

    @State private var dragOrigin: Double?

    private var clamped: Double { min(max(value, 0), 1) }

    var body: some View {
        Canvas { ctx, size in
            let c = CGPoint(x: size.width / 2, y: size.height / 2)
            drawScale(ctx, c)
            drawKnob(ctx, c, phase: pointerAngle.radians)
        }
        .frame(width: Self.size, height: Self.size)
        .overlay(alignment: .bottom) {
            HStack {
                scaleLabel(low)
                Spacer()
                scaleLabel(high)
            }
            .padding(.horizontal, 2)
        }
        .contentShape(Rectangle())
        .gesture(drag)
        .onTapGesture(count: 2) { onChange(0) }
        .accessibilityElement()
        .accessibilityLabel("\(low) to \(high)")
        .accessibilityValue("\(Int((clamped * 100).rounded())) percent")
        .accessibilityAdjustableAction { direction in
            switch direction {
            case .increment: onChange(min(clamped + 0.05, 1))
            case .decrement: onChange(max(clamped - 0.05, 0))
            @unknown default: break
            }
        }
    }

    private var pointerAngle: Angle {
        .degrees(Self.startDegrees + Self.sweepDegrees * clamped)
    }

    // MARK: - Drawing

    /// Outer edge of the fluted body at a lobe's crest, and the depth of
    /// the flute between lobes.
    private static let lobeRadius: CGFloat = 23
    private static let fluteDepth: CGFloat = 2.6
    private static let lobes = 8
    /// The aluminium cap and the black bevel ring around it.
    private static let capRadius: CGFloat = 13.5
    private static let bevelRadius: CGFloat = 16

    /// The panel print: eleven ticks over the arc, the ends a touch
    /// longer, in the plate's ink. The words come from the overlay.
    private func drawScale(_ ctx: GraphicsContext, _ c: CGPoint) {
        for i in 0...10 {
            let a = Angle.degrees(Self.startDegrees + Self.sweepDegrees * Double(i) / 10).radians
            let long = i == 0 || i == 10 || i == 5
            var tick = Path()
            tick.move(to: polar(c, Self.lobeRadius + 3, a))
            tick.addLine(to: polar(c, Self.lobeRadius + (long ? 6.5 : 5), a))
            ctx.stroke(tick, with: .color(DubColor.textPlaceholder),
                       style: StrokeStyle(lineWidth: 1, lineCap: .round))
        }
    }

    private func drawKnob(_ ctx: GraphicsContext, _ c: CGPoint, phase: Double) {
        // The skirt: the flange under the body, a shade off the plate.
        let skirt = Path(ellipseIn: square(c, Self.lobeRadius + 2))
        ctx.fill(skirt, with: .color(Color(hex: 0x0E0F12)))
        ctx.stroke(skirt, with: .color(DubColor.plateEdge.opacity(0.6)), lineWidth: 0.8)

        // The fluted body, turned to the value. Phenolic sheen from the
        // top-left; each lobe lit on one flank and dark on the other.
        let body = lobedPath(c, phase: phase)
        ctx.fill(body, with: .radialGradient(
            Gradient(colors: [Color(hex: 0x3C3D42), Color(hex: 0x15161A), Color(hex: 0x050506)]),
            center: CGPoint(x: c.x - 7, y: c.y - 8), startRadius: 0, endRadius: Self.lobeRadius * 1.6))
        var flank: [Gradient.Stop] = []
        for i in 0..<Self.lobes {
            let base = Double(i) / Double(Self.lobes)
            let step = 1.0 / Double(Self.lobes)
            flank.append(.init(color: .white.opacity(0.0), location: base))
            flank.append(.init(color: .white.opacity(0.16), location: base + step * 0.3))
            flank.append(.init(color: .black.opacity(0.0), location: base + step * 0.5))
            flank.append(.init(color: .black.opacity(0.55), location: base + step * 0.8))
            flank.append(.init(color: .white.opacity(0.0), location: base + step))
        }
        ctx.fill(body, with: .conicGradient(Gradient(stops: flank), center: c, angle: .radians(phase)))
        ctx.stroke(body, with: .color(.black.opacity(0.9)), lineWidth: 0.7)

        // The bevel ring between the flutes and the cap.
        let bevel = Path(ellipseIn: square(c, Self.bevelRadius))
        ctx.fill(bevel, with: .radialGradient(
            Gradient(colors: [Color(hex: 0x0A0B0D), Color(hex: 0x1C1D22)]),
            center: c, startRadius: Self.capRadius, endRadius: Self.bevelRadius))

        // The spun-aluminium cap: an anisotropic sheen fixed to the light,
        // then the diamond-cut rings as hairlines.
        let cap = Path(ellipseIn: square(c, Self.capRadius))
        let sheen = Gradient(stops: [
            .init(color: Color(hex: 0xA9AEB4), location: 0.00),
            .init(color: Color(hex: 0xEDF0F2), location: 0.10),
            .init(color: Color(hex: 0x7C8288), location: 0.24),
            .init(color: Color(hex: 0xC4C8CD), location: 0.40),
            .init(color: Color(hex: 0x666C73), location: 0.55),
            .init(color: Color(hex: 0xE2E5E8), location: 0.66),
            .init(color: Color(hex: 0x878D93), location: 0.80),
            .init(color: Color(hex: 0xA9AEB4), location: 1.00),
        ])
        ctx.fill(cap, with: .conicGradient(sheen, center: c, angle: .degrees(-60)))
        var r: CGFloat = 2
        var dark = true
        while r < Self.capRadius - 0.5 {
            let ring = Path(ellipseIn: square(c, r))
            ctx.stroke(ring, with: .color(dark ? .black.opacity(0.18) : .white.opacity(0.22)), lineWidth: 0.35)
            r += 0.9
            dark.toggle()
        }
        ctx.stroke(cap, with: .color(.black.opacity(0.7)), lineWidth: 0.6)

        // The index line: white paint from the cap's edge down the lobe
        // that sits under the pointer.
        var index = Path()
        index.move(to: polar(c, Self.bevelRadius + 0.5, phase))
        index.addLine(to: polar(c, Self.lobeRadius - 0.6, phase))
        ctx.stroke(index, with: .color(Color(hex: 0xF4F5F7)),
                   style: StrokeStyle(lineWidth: 1.7, lineCap: .round))
    }

    /// Eight rounded lobes with a flute between each pair, a crest under
    /// the pointer.
    private func lobedPath(_ c: CGPoint, phase: Double) -> Path {
        var p = Path()
        let steps = Self.lobes * 24
        for i in 0...steps {
            let a = phase + Double(i) / Double(steps) * 2 * .pi
            let wave = (1 + cos(Double(Self.lobes) * (a - phase))) / 2   // 1 at a crest, 0 in a flute
            let r = Self.lobeRadius - Self.fluteDepth * (1 - pow(wave, 0.7))
            let pt = polar(c, r, a)
            if i == 0 { p.move(to: pt) } else { p.addLine(to: pt) }
        }
        p.closeSubpath()
        return p
    }

    private func polar(_ c: CGPoint, _ r: CGFloat, _ a: Double) -> CGPoint {
        CGPoint(x: c.x + r * cos(a), y: c.y + r * sin(a))
    }

    private func square(_ c: CGPoint, _ r: CGFloat) -> CGRect {
        CGRect(x: c.x - r, y: c.y - r, width: r * 2, height: r * 2)
    }

    private func scaleLabel(_ text: String) -> some View {
        Text(text)
            .font(.system(size: 7, weight: .semibold))
            .tracking(0.6)
            .foregroundStyle(DubColor.textTertiary)
    }

    private var drag: some Gesture {
        DragGesture(minimumDistance: 1)
            .onChanged { g in
                let origin = dragOrigin ?? clamped
                if dragOrigin == nil { dragOrigin = origin }
                // Up is more: screen y grows downward.
                let next = origin - Double(g.translation.height / Self.travel)
                onChange(min(max(next, 0), 1))
            }
            .onEnded { _ in dragOrigin = nil }
    }
}
