//
//  FxHardware.swift
//  Dub
//
//  The small parts every unit in the DUB FX rack is built from: the
//  rack-eared chassis, engraved lettering, the Dymo strip, the readout
//  window, the jewel lamp, the bat-handle IN/OUT toggle and the knob
//  cell. The rack is skeuomorphic by the same decision as the siren box
//  (2026-09-12): the faces are the 70s outboard they emulate, and every
//  number printed on them is the engine's.
//

import SwiftUI

/// A unit in the rack: rack ears with their screws either side of the
/// face. Bypassed, the face goes grey and dim — the dial and its number
/// stay legible, so the DJ pre-sets a unit and then throws it in.
struct FxUnitChassis<Face: View>: View {
    let height: CGFloat
    let on: Bool
    @ViewBuilder var face: () -> Face

    var body: some View {
        HStack(spacing: 0) {
            ear(trailingEdge: true)
            face()
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .saturation(on ? 1 : 0.35)
                .brightness(on ? 0 : -0.12)
            ear(trailingEdge: false)
        }
        .frame(height: height)
        .clipShape(RoundedRectangle(cornerRadius: 4, style: .continuous))
        .shadow(color: .black.opacity(0.55), radius: 3, y: 2)
    }

    private func ear(trailingEdge: Bool) -> some View {
        ZStack {
            LinearGradient(
                colors: [Color(hex: 0x2B2E33), Color(hex: 0x1C1F24)],
                startPoint: .top, endPoint: .bottom)
            VStack {
                screw
                Spacer(minLength: 0)
                screw
            }
            .padding(.vertical, 5)
        }
        .frame(width: 10)
        .overlay(alignment: trailingEdge ? .trailing : .leading) {
            Rectangle().fill(Color(hex: 0x0A0B0D)).frame(width: 1)
        }
    }

    private var screw: some View {
        Circle()
            .fill(RadialGradient(
                colors: [Color(hex: 0x9AA0A8), Color(hex: 0x3A3E45), Color(hex: 0x1A1C20)],
                center: UnitPoint(x: 0.35, y: 0.3), startRadius: 0, endRadius: 4))
            .overlay(Circle().stroke(Color(hex: 0x0D0E11), lineWidth: 1))
            .frame(width: 6, height: 6)
    }
}

/// Engraved lettering on a faceplate — light ink with a drop into the
/// metal.
struct FxEngraved: View {
    let text: String
    var dim: Bool = false
    var color: Color? = nil
    var size: CGFloat = 8

    var body: some View {
        Text(text.uppercased())
            .font(.system(size: size, weight: .semibold))
            .tracking(size >= 8 ? 1.1 : 0.8)
            .foregroundStyle(color ?? Color.white.opacity(dim ? 0.45 : 0.72))
            .shadow(color: .black.opacity(0.6), radius: 0, y: 1)
            .lineLimit(1)
            .fixedSize()
    }
}

/// A Dymo strip: white embossed capitals on black tape, applied by hand
/// and so never quite straight. Every unit carries one — the 70s desk's
/// labelling, and Perry labelled everything.
struct FxDymoLabel: View {
    let text: String
    var tilt: Double = 0

    var body: some View {
        Text(text.uppercased())
            .font(.system(size: 10, weight: .bold))
            .tracking(1.6)
            .foregroundStyle(Color(hex: 0xF4F4F2))
            .shadow(color: .white.opacity(0.35), radius: 0.5)
            .padding(.horizontal, 7)
            .padding(.top, 4)
            .padding(.bottom, 3)
            .background(RoundedRectangle(cornerRadius: 2).fill(Color(hex: 0x0A0A0B)))
            .overlay(RoundedRectangle(cornerRadius: 2).stroke(Color.white.opacity(0.08), lineWidth: 1))
            .shadow(color: .black.opacity(0.6), radius: 1, y: 1)
            .rotationEffect(.degrees(tilt))
            .fixedSize()
    }
}

/// A recessed readout window with a mono number in it.
struct FxReadout: View {
    let text: String
    var size: CGFloat = 10
    var color: Color = DubColor.textPrimary

    var body: some View {
        Text(text)
            .font(.system(size: size, weight: .medium, design: .monospaced))
            .monospacedDigit()
            .foregroundStyle(color)
            .lineLimit(1)
            .padding(.horizontal, 5)
            .padding(.vertical, 2)
            .background(RoundedRectangle(cornerRadius: 3).fill(DubColor.displayWell))
            .overlay(RoundedRectangle(cornerRadius: 3).stroke(Color.black, lineWidth: 1))
            .fixedSize()
    }
}

/// A jewel lamp: dark glass off, lit and glowing on.
struct FxLamp: View {
    let color: Color
    let on: Bool
    var size: CGFloat = 9

    var body: some View {
        Circle()
            .fill(on ? color : Color(hex: 0x1A1C1F))
            .overlay(Circle().stroke(Color.black, lineWidth: 1))
            .overlay(alignment: .top) {
                Circle()
                    .fill(Color.white.opacity(on ? 0.5 : 0.08))
                    .frame(width: size * 0.4, height: size * 0.4)
                    .offset(y: size * 0.12)
            }
            .frame(width: size, height: size)
            .shadow(color: on ? color.opacity(0.9) : .clear, radius: on ? 5 : 0)
    }
}

/// The bat-handle IN/OUT toggle with its lamp — the same on every unit.
/// A tap throws it; the engine's slot flag is the truth, so the bat
/// follows `on` rather than its own state.
struct FxBatToggle: View {
    let color: Color
    let on: Bool
    /// The key that throws it, from the map; printed under the bat.
    var legend: String? = nil
    var onToggle: () -> Void = {}
    /// Map mode draws its own cap; IN/OUT comes back under it.
    @Environment(\.dubMapping) private var mapping

    var body: some View {
        VStack(spacing: 5) {
            FxLamp(color: color, on: on, size: 12)
            Canvas { ctx, size in
                let c = CGPoint(x: size.width / 2, y: size.height / 2)
                let ring = Path(ellipseIn: CGRect(x: c.x - 12, y: c.y - 12, width: 24, height: 24))
                ctx.fill(ring, with: .radialGradient(
                    Gradient(colors: [Color(hex: 0x1B1D21), Color(hex: 0x0C0D0F)]),
                    center: c, startRadius: 0, endRadius: 13))
                ctx.stroke(ring, with: .color(Color(hex: 0x2A2D32)), lineWidth: 2)
                // The bat: a chrome bar from the pivot, tipped with a ball;
                // up-and-left is IN, down-and-right is OUT.
                let a = (on ? -28.0 : 28.0) * Double.pi / 180
                let tip = CGPoint(x: c.x + sin(a) * 11, y: c.y - cos(a) * 11)
                var bar = Path()
                bar.move(to: c)
                bar.addLine(to: tip)
                ctx.stroke(bar, with: .linearGradient(
                    Gradient(colors: [Color(hex: 0x6D7178), Color(hex: 0xD3D6DB), Color(hex: 0x7D8188)]),
                    startPoint: CGPoint(x: c.x - 4, y: c.y), endPoint: CGPoint(x: c.x + 4, y: c.y)),
                    style: StrokeStyle(lineWidth: 5, lineCap: .round))
                let ball = Path(ellipseIn: CGRect(x: tip.x - 5, y: tip.y - 5, width: 10, height: 10))
                ctx.fill(ball, with: .radialGradient(
                    Gradient(colors: [Color(hex: 0xE8EAEE), Color(hex: 0x8B8F96)]),
                    center: CGPoint(x: tip.x - 1.5, y: tip.y - 2), startRadius: 0, endRadius: 6))
            }
            .frame(width: 28, height: 28)
            if let legend, mapping == nil {
                DubKeycap(key: legend, lit: on)
            } else {
                FxEngraved(text: on ? "IN" : "OUT", dim: true)
            }
        }
        .contentShape(Rectangle())
        .onPressDown(perform: onToggle)
        .help(on ? "In circuit — tap to bypass" : "Bypassed — tap to put it in")
        .accessibilityElement()
        .accessibilityLabel(on ? "In" : "Out")
        .accessibilityAddTraits(.isButton)
    }
}

/// A labelled knob: the name engraved above, the readout in its window
/// below. The knob is the siren's fluted one, smaller.
struct FxKnobCell: View {
    let name: String
    let value: Double
    var size: CGFloat = 26
    var detents: Int? = nil
    var readout: String
    var readoutColor: Color = DubColor.textPrimary
    var onChange: (_ value: Double) -> Void = { _ in }

    var body: some View {
        VStack(spacing: 3) {
            FxEngraved(text: name, size: 7)
            DubKnob(value: value, size: size, detents: detents,
                    ink: Color.white.opacity(0.5), onChange: onChange)
                .padding(4)
            FxReadout(text: readout, color: readoutColor)
        }
        .fixedSize()
    }
}
