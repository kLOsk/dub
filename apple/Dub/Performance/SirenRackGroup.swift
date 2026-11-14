//
//  SirenRackGroup.swift
//  Dub
//
//  The siren block of the global rack bar: five flat pads, the tail the
//  DUB knob has set, and the knob.
//
//  It was a faceplate with a VU meter, five keycaps and a fluted knob —
//  the box a dub DJ owns, drawn as the box. On the surface the meter
//  read as a prop beside the flat decks (Daniel, 2026-11-14: "it looks
//  off"), and of the three redesigns he chose the flat one (canvas *Dub
//  Siren Rack*, direction B). What tells the five shots apart now is
//  each pad's own waveform — a burst, a square, a sine, a falling zap,
//  a rise and fall — and what the meter's echo line said is the TAIL
//  row: one segment per repeat, fading as the repeats do.
//
//  One rack, with the deck it fires on named on its face: a single
//  siren, and `KeyEventMonitorHost` dispatches `.sirenPreset` to the
//  rack's output once map mode binds it — so a cap drawn here is a cap
//  that fires, or nothing.
//

import DubCore
import SwiftUI

/// Keyboard legends for the siren shots, in fire order — empty until map
/// mode binds them; the caps then print whatever the DJ picked. Read at
/// draw time (a global `let` froze the launch-time map) and carried on
/// `SirenRackState.legends` so a rebind re-renders the box. One row
/// because there is one keymap — the handler resolves the deck at press
/// time, not the key.
var sirenPresetKeys: [String] { DubKeymap.legends({ .sirenPreset($0) }, count: 5) }

/// The siren block: pads over a tail row, the DUB dial beside them, under
/// the bar's shared heading.
struct SirenRackGroup: View {
    let state: SirenRackState
    var onPreset: (_ index: Int) -> Void = { _ in }
    var onDubMacro: (_ value: Double) -> Void = { _ in }
    var onOutput: ((_ output: RackOutput) -> Void)?

    private static let padGap: CGFloat = 6
    private static let dialGap: CGFloat = 14
    private static let tailHeight: CGFloat = 14
    private static let rowGap: CGFloat = 8

    /// The row of pads.
    private static let padsWidth: CGFloat = DubLayout.sirenKeyWidth * 5 + padGap * 4

    /// The block's width, summed from its parts. Pinned, because the
    /// heading's rule is flexible and would otherwise make the block's
    /// ideal width unbounded — the bar then hands the siren every spare
    /// point and squeezes the shelf to its floor.
    static let width: CGFloat = padsWidth + dialGap + DubLayout.sirenKnobCellWidth

    /// What the knob has set — the same curve the engine applies, so the
    /// readout and the sound cannot disagree.
    private var dub: SirenDubControls {
        sirenDubMacroControls(macroValue: Float(state.dubMacro))
    }

    private var lastShotName: String? {
        state.lastShot.flatMap { state.presetNames.indices.contains($0) ? state.presetNames[$0] : nil }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: DubSpacing.sm) {
            header
            HStack(spacing: Self.dialGap) {
                VStack(alignment: .leading, spacing: Self.rowGap) {
                    HStack(spacing: Self.padGap) {
                        ForEach(Array(state.presetNames.enumerated()), id: \.offset) { index, name in
                            pad(index: index, name: name)
                        }
                    }
                    SirenTail(
                        on: state.dubMacro > 0, delayMs: Double(dub.delayMs),
                        feedback: Double(dub.feedback))
                        .frame(width: Self.padsWidth, height: Self.tailHeight)
                }
                dial
            }
            .frame(height: DubLayout.sirenPlateHeight)
        }
        .frame(width: Self.width, alignment: .leading)
    }

    /// The same heading the shelf and the deck sections use — title,
    /// rule, state — so the bar's two blocks read as one family. The
    /// trailing names the shot while it sounds rather than saying ON.
    private var header: some View {
        HStack(spacing: DubSpacing.sm) {
            SectionHeading(
                title: "DUB SIREN", accent: DubColor.siren,
                trailing: state.sounding ? "● \((lastShotName ?? "ON").uppercased())" : "○ OFF",
                trailingAccent: state.sounding ? DubColor.siren : DubColor.textPlaceholder)
            DubDeckPill(state: state.output, onSelect: onOutput)
                .help("Where the siren fires — the master deck by default. "
                    + "Right-click to pin it to A, B or both.")
        }
    }

    @ViewBuilder
    private func pad(index: Int, name: String) -> some View {
        let legend = index < state.legends.count ? state.legends[index] : ""
        SirenPad(
            title: name, legend: legend, shot: index,
            down: state.sounding && state.lastShot == index)
            .frame(
                width: DubLayout.sirenKeyWidth,
                height: DubLayout.sirenPlateHeight - Self.tailHeight - Self.rowGap)
            .onPressDown { onPreset(index) }
            .help(legend.isEmpty ? "Fire \(name)" : "Fire \(name) (\(legend))")
            .mappable(.sirenPreset(index))
    }

    /// The DUB dial. What it has set is on the tail row beside it.
    private var dial: some View {
        SirenDubDial(value: state.dubMacro, onChange: onDubMacro)
            .help(state.dubMacro > 0
                ? "DUB — \(Int(dub.delayMs.rounded())) ms · \(Int((dub.feedback * 100).rounded())) % feedback. "
                    + "Slows and deepens the shots and adds the siren's own echo, "
                    + "longer, louder and darker as it turns. Double-click for DRY."
                : "DUB — DRY: no echo at all. Turn it up to slow and deepen the "
                    + "shots and add the siren's own echo. Double-click for DRY.")
            .frame(width: DubLayout.sirenKnobCellWidth)
    }
}

/// One shot: its waveform, its name, and the key that fires it in the
/// corner. Lit in the siren's red while it sounds.
struct SirenPad: View {
    let title: String
    /// The bound key, or `""` — the corner is then left blank rather
    /// than promising a key that does nothing.
    var legend: String = ""
    /// Which of the five shots, for the glyph.
    let shot: Int
    var down: Bool = false

    var body: some View {
        ZStack(alignment: .topTrailing) {
            RoundedRectangle(cornerRadius: 4, style: .continuous)
                .fill(down ? DubColor.siren.opacity(0.18) : DubColor.surface3)
            RoundedRectangle(cornerRadius: 4, style: .continuous)
                .stroke(down ? DubColor.siren : DubColor.divider, lineWidth: 1)
            VStack(spacing: 5) {
                SirenGlyph(shot: shot)
                    .stroke(
                        down ? DubColor.siren : DubColor.textSecondary,
                        style: StrokeStyle(lineWidth: 1.2, lineCap: .round, lineJoin: .round))
                    .frame(width: 26, height: 16)
                Text(title.uppercased())
                    .font(.system(size: 9, weight: .bold))
                    .tracking(0.6)
                    .lineLimit(1)
                    .minimumScaleFactor(0.8)
                    .foregroundStyle(down ? DubColor.textPrimary : DubColor.textSecondary)
            }
            .padding(.horizontal, 3)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            if !legend.isEmpty {
                Text(legend)
                    .font(.system(size: 8, weight: .bold, design: .monospaced))
                    .foregroundStyle(DubColor.textTertiary)
                    .padding(.top, 3)
                    .padding(.trailing, 5)
            }
        }
        .contentShape(Rectangle())
        .accessibilityElement()
        .accessibilityLabel(title)
        .accessibilityAddTraits(.isButton)
    }
}

/// A shot's waveform, drawn in a 32 × 20 box and scaled to the frame:
/// the Rifle Gun's bursts, the Alarm's square, the Sine, the Laser's
/// zap widening as it falls, the Siren's rise and fall.
struct SirenGlyph: Shape {
    let shot: Int

    private static let bursts: [(CGFloat, CGFloat)] = [
        (1, 10), (7, 10), (8, 2), (9.5, 18), (11, 10), (15, 10), (16, 5), (17, 15),
        (18, 10), (22, 10), (22.8, 7.5), (23.6, 12.5), (24.4, 10), (31, 10),
    ]
    private static let square: [(CGFloat, CGFloat)] = [
        (1, 5), (8, 5), (8, 15), (16, 15), (16, 5), (24, 5), (24, 15), (31, 15),
    ]
    private static let zap: [(CGFloat, CGFloat)] = [
        (1, 10), (2, 3), (3, 17), (4, 3), (5, 17), (6.5, 3), (8, 17), (10, 3),
        (12, 17), (14.5, 3), (17, 17), (20, 3), (23, 17), (27, 3), (31, 17),
    ]

    private var points: [(CGFloat, CGFloat)] {
        switch shot {
        case 0: return Self.bursts
        case 1: return Self.square
        case 2: return (0...30).map { i in
            let x = 1 + CGFloat(i)
            return (x, 10 - 8 * sin(2 * .pi * CGFloat(i) / 16))
        }
        case 3: return Self.zap
        default: return (0...30).map { i in
            let x = 1 + CGFloat(i)
            return (x, 17 - 14 * (0.5 - 0.5 * cos(2 * .pi * CGFloat(i) / 30)))
        }
        }
    }

    func path(in rect: CGRect) -> Path {
        let (sx, sy) = (rect.width / 32, rect.height / 20)
        var path = Path()
        for (i, p) in points.enumerated() {
            let at = CGPoint(x: rect.minX + p.0 * sx, y: rect.minY + p.1 * sy)
            if i == 0 { path.move(to: at) } else { path.addLine(to: at) }
        }
        return path
    }
}

/// The echo the DUB knob has set, as its repeats: one segment each, as
/// loud as that repeat is against the shot, with the time between them
/// and the feedback in figures. `DRY` with every segment dark when the
/// knob is at zero.
struct SirenTail: View {
    let on: Bool
    let delayMs: Double
    /// 0…1, the level each repeat keeps of the one before.
    let feedback: Double

    private static let repeats = 12

    /// How lit repeat `i` is: `feedback^i`, with a floor so a dark
    /// segment is still a segment.
    static func level(repeat i: Int, on: Bool, feedback: Double) -> Double {
        guard on else { return 0.08 }
        return max(0.08, pow(min(max(feedback, 0), 1), Double(i)))
    }

    var body: some View {
        HStack(spacing: DubSpacing.sm) {
            Text("TAIL")
                .font(.system(size: 9, weight: .bold))
                .tracking(0.8)
                .foregroundStyle(DubColor.textTertiary)
            HStack(spacing: 2) {
                ForEach(0..<Self.repeats, id: \.self) { i in
                    RoundedRectangle(cornerRadius: 1, style: .continuous)
                        .fill(DubColor.siren)
                        .opacity(Self.level(repeat: i, on: on, feedback: feedback))
                        .frame(height: 5)
                }
            }
            Text(on ? "\(Int(delayMs.rounded())) ms · \(Int((feedback * 100).rounded())) %" : "DRY")
                .font(.system(size: 10, weight: .medium, design: .monospaced))
                .foregroundStyle(on ? DubColor.textSecondary : DubColor.textPlaceholder)
                .frame(width: 92, alignment: .trailing)
        }
        .accessibilityElement()
        .accessibilityLabel("Echo tail")
        .accessibilityValue(
            on ? "\(Int(delayMs.rounded())) milliseconds, \(Int((feedback * 100).rounded())) percent feedback"
                : "dry")
    }
}

/// The DUB dial: a flat arc that fills as it turns, an index line, and
/// the value in words underneath. Drag vertically to turn (150 pt for
/// the whole arc, as `DubKnob`), double-click for DRY.
struct SirenDubDial: View {
    /// 0…1.
    let value: Double
    var onChange: (_ value: Double) -> Void = { _ in }

    private static let travel: CGFloat = 150
    private static let size: CGFloat = 46

    @State private var dragOrigin: Double?

    private var clamped: Double { min(max(value, 0), 1) }

    var body: some View {
        VStack(spacing: 4) {
            Canvas { ctx, size in
                let c = CGPoint(x: size.width / 2, y: size.height / 2)
                let r = min(size.width, size.height) / 2 - 3
                // 270° from bottom-left, clockwise through the top.
                let start = Angle.degrees(135)
                func arc(to fraction: Double) -> Path {
                    Path { p in
                        p.addArc(
                            center: c, radius: r, startAngle: start,
                            endAngle: start + .degrees(270 * fraction), clockwise: false)
                    }
                }
                let stroke = StrokeStyle(lineWidth: 3.5, lineCap: .round)
                ctx.stroke(arc(to: 1), with: .color(DubColor.divider), style: stroke)
                if clamped > 0 {
                    ctx.stroke(arc(to: clamped), with: .color(DubColor.siren), style: stroke)
                }
                let body = Path(ellipseIn: CGRect(
                    x: c.x - r + 5.5, y: c.y - r + 5.5, width: (r - 5.5) * 2, height: (r - 5.5) * 2))
                ctx.fill(body, with: .color(DubColor.surface3))
                ctx.stroke(body, with: .color(DubColor.plateEdge), lineWidth: 1)
                let a = (start + .degrees(270 * clamped)).radians
                var index = Path()
                index.move(to: CGPoint(x: c.x + cos(a) * 5, y: c.y + sin(a) * 5))
                index.addLine(to: CGPoint(x: c.x + cos(a) * (r - 8), y: c.y + sin(a) * (r - 8)))
                ctx.stroke(
                    index, with: .color(DubColor.textPrimary),
                    style: StrokeStyle(lineWidth: 1.5, lineCap: .round))
            }
            .frame(width: Self.size, height: Self.size)
            Text(clamped > 0 ? "DUB \(Int((clamped * 100).rounded())) %" : "DRY")
                .font(.system(size: 9, weight: .heavy))
                .tracking(0.8)
                .foregroundStyle(DubColor.textSecondary)
        }
        .frame(maxHeight: .infinity)
        .contentShape(Rectangle())
        .gesture(
            DragGesture(minimumDistance: 1)
                .onChanged { g in
                    let origin = dragOrigin ?? clamped
                    dragOrigin = origin
                    onChange(min(max(origin - Double(g.translation.height / Self.travel), 0), 1))
                }
                .onEnded { _ in dragOrigin = nil })
        .onTapGesture(count: 2) { onChange(0) }
        .accessibilityElement()
        .accessibilityLabel("Dub")
        .accessibilityValue("\(Int((clamped * 100).rounded())) percent")
        .accessibilityAdjustableAction { direction in
            switch direction {
            case .increment: onChange(min(clamped + 0.05, 1))
            case .decrement: onChange(max(clamped - 0.05, 0))
            @unknown default: break
            }
        }
    }
}
