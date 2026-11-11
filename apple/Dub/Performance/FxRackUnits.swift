//
//  FxRackUnits.swift
//  Dub
//
//  The four units in the DUB FX rack, each drawn as the outboard it
//  emulates and each its own species of control: a hammertone Altec-style
//  filter with one stepped dial (the Big Knob), a two-tone blue phaser
//  with sweep lamps, a green tape echo with a working tape window, and a
//  bare steel spring tank you can kick. Bat-handle IN/OUT toggles and Dymo
//  strips on all four are the 70s-desk glue.
//
//  Everything on a face is drawn from the knobs the UI holds — the rack
//  publishes no state — the way the siren's needle is. The numbers are the
//  engine's: the detent Hz from `bigKnobStepHz`, the tone Hz from
//  `springToneHz`, the head times from the RE-201's fixed ratios.
//

import DubCore
import SwiftUI

// MARK: - Big Knob

/// King Tubby's bass-drop high-pass: one stepped dial, the Altec's eleven
/// detents printed on the arc, the cut-off in a window beside it.
struct FxBigKnobUnit: View {
    let on: Bool
    let controls: FxRackControls
    /// The key on the IN/OUT toggle, from the map.
    var toggleLegend: String? = nil
    var onToggle: () -> Void = {}
    var onControls: (FxRackControls) -> Void = { _ in }

    static let height: CGFloat = 100
    private static let steps = 11

    var body: some View {
        FxUnitChassis(height: Self.height, on: on) {
            HStack(spacing: 10) {
                VStack(alignment: .leading, spacing: 5) {
                    FxDymoLabel(text: "Big Knob", tilt: -0.6)
                    FxEngraved(text: "Hi-pass · 12 dB/oct")
                    FxEngraved(text: "11 detents · 70 – 7.5k", dim: true)
                }
                .frame(width: 104, alignment: .leading)
                dial
                VStack(alignment: .leading, spacing: 5) {
                    FxEngraved(text: "Cut-off")
                    FxReadout(text: Self.hzText(controls.bigKnobStep), size: 14, reserve: ["0 000 Hz"])
                    FxEngraved(text: "Step \(controls.bigKnobStep + 1) of \(Self.steps)", dim: true,
                               reserve: ["Step 00 of 00"])
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                VStack(spacing: 2) {
                    FxEngraved(text: "Q", dim: true)
                    DubKnob(value: (controls.bigKnobQ - 0.5) / 7.5, size: 16,
                            ink: Color.white.opacity(0.5)) { v in
                        var c = controls
                        c.bigKnobQ = 0.5 + v * 7.5
                        onControls(c)
                    }
                    .padding(3)
                }
                FxBatToggle(color: DubColor.bigKnob, on: on, legend: toggleLegend, onToggle: onToggle)
                    .mappable(.fxToggle(0))
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 2)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(LinearGradient(
                colors: [Color(hex: 0x474C48), Color(hex: 0x383C39)],
                startPoint: .top, endPoint: .bottom))
        }
    }

    /// The dial with the frequencies on the arc; the stop it sits on
    /// prints in the unit's tint.
    private var dial: some View {
        ZStack {
            Canvas { ctx, size in
                let c = CGPoint(x: size.width / 2, y: size.height / 2)
                for i in 0..<Self.steps {
                    let deg = -135 + 270 * Double(i) / Double(Self.steps - 1)
                    let a = deg * Double.pi / 180
                    let onStop = i == controls.bigKnobStep
                    let p = CGPoint(x: c.x + sin(a) * 39, y: c.y - cos(a) * 39 + 2.5)
                    let label = Text(Self.arcLabel(i))
                        .font(.system(size: 7, weight: .bold))
                        .foregroundColor(onStop ? DubColor.bigKnob : Color.white.opacity(0.62))
                    ctx.draw(label, at: p)
                }
            }
            DubKnob(
                value: Double(controls.bigKnobStep) / Double(Self.steps - 1),
                size: 50, detents: Self.steps, ink: Color.white.opacity(0.55)
            ) { v in
                var c = controls
                c.bigKnobStep = Int((v * Double(Self.steps - 1)).rounded())
                onControls(c)
            }
        }
        .frame(width: 94, height: 94)
    }

    private static func arcLabel(_ step: Int) -> String {
        let hz = bigKnobStepHz(step: UInt8(step))
        return hz >= 1_000 ? String(format: "%.1fk", hz / 1_000) : String(Int(hz))
    }

    static func hzText(_ step: Int) -> String {
        let hz = Int(bigKnobStepHz(step: UInt8(max(0, min(step, steps - 1)))))
        return hz >= 1_000
            ? String(format: "%d %03d Hz", hz / 1_000, hz % 1_000)
            : "\(hz) Hz"
    }
}

// MARK: - Phaser

/// Lee Perry's Black Ark swirl: a two-tone blue face, four knobs, and two
/// sweep lamps breathing against each other at the rate — the stereo
/// pair's offset LFOs.
struct FxPhaserUnit: View {
    let on: Bool
    let controls: FxRackControls
    /// The key on the IN/OUT toggle, from the map.
    var toggleLegend: String? = nil
    /// Off: the lamps hold one phase (a snapshot).
    var motion: Bool = true
    var onToggle: () -> Void = {}
    var onControls: (FxRackControls) -> Void = { _ in }

    static let height: CGFloat = 82
    /// The rate knob is log over 0.05–10 Hz.
    private static let rateMin = 0.05
    private static let rateMax = 10.0

    var body: some View {
        FxUnitChassis(height: Self.height, on: on) {
            HStack(spacing: 10) {
                VStack(alignment: .leading, spacing: 5) {
                    FxDymoLabel(text: "Phaser", tilt: 0.4)
                    FxEngraved(text: "6-stage · stereo pair")
                    FxEngraved(text: "Offset L/R sweep", dim: true)
                }
                .frame(width: 104, alignment: .leading)
                HStack(alignment: .bottom, spacing: 6) {
                    FxKnobCell(name: "Rate", value: rateNorm,
                               readout: String(format: "%.2g Hz", controls.phaserRateHz),
                               reserve: ["0.0 Hz"]) { v in
                        var c = controls
                        c.phaserRateHz = Self.rateMin * pow(Self.rateMax / Self.rateMin, v)
                        onControls(c)
                    }
                    FxKnobCell(name: "Depth", value: controls.phaserDepth,
                               readout: percent(controls.phaserDepth),
                               reserve: percentReserve) { v in
                        var c = controls
                        c.phaserDepth = v
                        onControls(c)
                    }
                    FxKnobCell(name: "Feedback", value: controls.phaserFeedback / 0.95,
                               readout: percent(controls.phaserFeedback),
                               reserve: percentReserve) { v in
                        var c = controls
                        c.phaserFeedback = v * 0.95
                        onControls(c)
                    }
                    FxKnobCell(name: "Mix", value: controls.phaserMix,
                               readout: percent(controls.phaserMix),
                               reserve: percentReserve) { v in
                        var c = controls
                        c.phaserMix = v
                        onControls(c)
                    }
                }
                .frame(maxWidth: .infinity)
                sweepLamps
                FxBatToggle(color: DubColor.phaser, on: on, legend: toggleLegend, onToggle: onToggle)
                    .mappable(.fxToggle(1))
            }
            .padding(.horizontal, 10)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(
                VStack(spacing: 0) {
                    Color(hex: 0x1F2C4C).frame(height: Self.height * 0.34)
                    Color(hex: 0x172140)
                })
        }
    }

    private var rateNorm: Double {
        log(controls.phaserRateHz / Self.rateMin) / log(Self.rateMax / Self.rateMin)
    }

    /// The L·R lamps: lit in opposition at the LFO rate while the unit
    /// is in, dark glass when it is bypassed. Drawn from the knob, not
    /// measured — the rack publishes no state.
    private var sweepLamps: some View {
        VStack(spacing: 5) {
            FxEngraved(text: "Sweep", dim: true)
            if motion {
                // The breathing is Core Animation's (FxLayerAnimations).
                HStack(spacing: 6) {
                    liveLamp(inverted: false)
                    liveLamp(inverted: true)
                }
            } else {
                let phase = on ? sin(0.5) : -1
                HStack(spacing: 6) {
                    lamp(0.5 + 0.5 * phase)
                    lamp(0.5 - 0.5 * phase)
                }
            }
            FxEngraved(text: "L · R", dim: true)
        }
    }

    private func liveLamp(inverted: Bool) -> some View {
        Circle()
            .fill(DubColor.phaser.opacity(0.12))
            .overlay(PulsingLamp(
                color: DubColor.phaser, rateHz: controls.phaserRateHz,
                inverted: inverted, on: on))
            .overlay(Circle().stroke(Color.black, lineWidth: 1))
            .frame(width: 12, height: 12)
    }

    private func lamp(_ level: Double) -> some View {
        Circle()
            .fill(DubColor.phaser.opacity(on ? 0.12 + 0.88 * level : 0.12))
            .overlay(Circle().stroke(Color.black, lineWidth: 1))
            .frame(width: 12, height: 12)
            .shadow(color: DubColor.phaser.opacity(on ? level : 0), radius: 5)
    }
}

// MARK: - Space Echo

/// The dub centrepiece: a green face, the mode selector, and the tape
/// window with its loop and heads — the heads glow for the mode, the tape
/// runs while the unit is in, and past unity intensity the dashes race
/// and RUNAWAY flashes.
struct FxSpaceEchoUnit: View {
    let on: Bool
    let controls: FxRackControls
    /// The key on the IN/OUT toggle, from the map.
    var toggleLegend: String? = nil
    /// Off: the tape and RUNAWAY hold one phase (a snapshot).
    var motion: Bool = true
    /// The keys on HOLD and TAP, from the map.
    var holdLegend: String? = nil
    var tapLegend: String? = nil
    /// Where SYNC's tempo comes from; `nil` when off or none.
    var tempo: FxEchoTempo? = nil
    var onToggle: () -> Void = {}
    var onControls: (FxRackControls) -> Void = { _ in }
    var onTap: () -> Void = {}

    static let height: CGFloat = 138
    private static let repeatMin = 20.0
    private static let repeatMax = 750.0
    private static let modeNames = ["Reverb", "Short", "Long", "Triple", "Short+rev", "Long+rev", "Triple+rev"]
    /// The width of the line under REPEAT: the three head times. A synced
    /// tempo or the call for a tap is laid over it, never widening the
    /// unit past its column.
    private static let repeatLineReserve = ["000 · 000 · 000"]

    var body: some View {
        FxUnitChassis(height: Self.height, on: on) {
            HStack(spacing: 10) {
                VStack(alignment: .leading, spacing: 6) {
                    HStack(spacing: 10) {
                        FxDymoLabel(text: "Space Echo", tilt: -0.3)
                        performanceKeys
                        Spacer(minLength: 0)
                        wowFlutter
                    }
                    HStack(alignment: .bottom, spacing: 6) {
                        FxTapeWindow(
                            mode: controls.spaceEchoMode, running: on,
                            runaway: controls.spaceEchoRunaway, motion: motion)
                        modeCell
                        repeatCell
                        intensityCell
                        FxKnobCell(name: "Echo", value: controls.spaceEchoVolume,
                                   readout: db(controls.spaceEchoVolume),
                                   reserve: dbReserve) { v in
                            var c = controls
                            c.spaceEchoVolume = v
                            onControls(c)
                        }
                        FxKnobCell(name: "Reverb", value: controls.spaceEchoReverb,
                                   readout: db(controls.spaceEchoReverb),
                                   reserve: dbReserve) { v in
                            var c = controls
                            c.spaceEchoReverb = v
                            onControls(c)
                        }
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                FxBatToggle(color: DubColor.spaceEcho, on: on, legend: toggleLegend, onToggle: onToggle)
                    .mappable(.fxToggle(2))
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(LinearGradient(
                colors: [Color(hex: 0x1A2F24), Color(hex: 0x142419)],
                startPoint: .top, endPoint: .bottom))
        }
    }

    /// The one-turntable keys, along the top: HOLD the tape through a
    /// record swap; SYNC the repeat to the music, at ½ · ¾ · 1 beat; TAP
    /// the tempo when the record has none. Small panel keys, not knobs —
    /// they are switched, not ridden, and the knob row has no room left.
    private var performanceKeys: some View {
        HStack(spacing: 6) {
            FxPanelKey(
                title: "Hold", lit: controls.spaceEchoHold, color: DubColor.echo,
                legend: holdLegend,
                help: controls.spaceEchoHold
                    ? "Held — tap to let the tape fade"
                    : "Hold the tape through a record swap: nothing new goes on, the loop sustains"
            ) {
                var c = controls
                c.spaceEchoHold.toggle()
                onControls(c)
            }
            .mappable(.fxHold)
            FxPanelKey(
                title: "Sync", lit: controls.spaceEchoSync, color: DubColor.spaceEcho,
                help: "Repeat in time with the other deck's tempo, or the tempo tapped in"
            ) {
                var c = controls
                c.spaceEchoSync.toggle()
                onControls(c)
            }
            HStack(spacing: 2) {
                ForEach(FxRackControls.spaceEchoDivisions, id: \.beats) { division in
                    let chosen = controls.spaceEchoDivision == division.beats
                    FxPanelKey(
                        title: division.label, lit: chosen && controls.spaceEchoSync,
                        dimLit: chosen, color: DubColor.spaceEcho, width: 16,
                        help: "The longest head at \(division.label) of a beat"
                    ) {
                        var c = controls
                        c.spaceEchoDivision = division.beats
                        c.spaceEchoSync = true
                        onControls(c)
                    }
                }
            }
            FxPanelKey(
                title: "Tap", lit: tempo?.source == .tap, color: DubColor.spaceEcho,
                legend: tapLegend,
                help: "Tap the beat — the tempo for SYNC when the record has none (THRU)",
                action: onTap)
            .mappable(.fxTap)
        }
    }

    /// Tape age: a trimmer in the top row, its reading beside it.
    private var wowFlutter: some View {
        HStack(spacing: 4) {
            FxEngraved(text: String(format: "W&F %.1f %%", controls.spaceEchoWowFlutter * 0.5), dim: true,
                       reserve: ["W&F 0.0 %"])
            DubKnob(value: controls.spaceEchoWowFlutter / 2, size: 14,
                    ink: Color.white.opacity(0.4)) { v in
                var c = controls
                c.spaceEchoWowFlutter = v * 2
                onControls(c)
            }
            .padding(3)
        }
    }

    private var modeIndex: Int {
        FxRackControls.spaceEchoModes.firstIndex(of: controls.spaceEchoMode) ?? 6
    }

    /// The chicken-head selector with its seven stops named on the arc.
    private var modeCell: some View {
        VStack(spacing: 2) {
            FxEngraved(text: "Mode", size: 7)
            ZStack {
                Canvas { ctx, size in
                    let c = CGPoint(x: size.width / 2, y: size.height / 2)
                    let names = ["RVB", "S", "L", "T", "S+R", "L+R", "T+R"]
                    for (i, name) in names.enumerated() {
                        let deg = -135 + 270 * Double(i) / 6
                        let a = deg * Double.pi / 180
                        let p = CGPoint(x: c.x + sin(a) * 24, y: c.y - cos(a) * 24 + 2)
                        let label = Text(name)
                            .font(.system(size: 6.5, weight: .bold))
                            .foregroundColor(i == modeIndex ? DubColor.spaceEcho : Color.white.opacity(0.6))
                        ctx.draw(label, at: p)
                    }
                }
                DubKnob(value: Double(modeIndex) / 6, size: 32, detents: 7,
                        ink: Color.white.opacity(0.5)) { v in
                    var c = controls
                    c.spaceEchoMode = FxRackControls.spaceEchoModes[Int((v * 6).rounded())]
                    onControls(c)
                }
            }
            .frame(width: 60, height: 60)
            FxReadout(text: Self.modeNames[modeIndex], size: 8, color: DubColor.textSecondary,
                      reserve: ["Triple+rev"])
        }
    }

    /// Repeat rate: the longest head, with all three head times under it
    /// — or, synced, where the tempo comes from. Turning the knob takes
    /// the repeat back by hand.
    private var repeatCell: some View {
        let ms = controls.spaceEchoRepeatMs
        return VStack(spacing: 2) {
            FxKnobCell(
                name: "Repeat",
                value: (ms - Self.repeatMin) / (Self.repeatMax - Self.repeatMin),
                readout: "\(Int(ms.rounded())) ms",
                readoutColor: controls.spaceEchoSync && tempo != nil
                    ? DubColor.spaceEcho : DubColor.textPrimary,
                reserve: ["000 ms"]
            ) { v in
                var c = controls
                c.spaceEchoRepeatMs = Self.repeatMin + v * (Self.repeatMax - Self.repeatMin)
                c.spaceEchoSync = false
                onControls(c)
            }
            if controls.spaceEchoSync {
                FxEngraved(
                    text: tempo.map { "♩ \($0.text)" } ?? "No tempo · tap",
                    color: tempo == nil ? DubColor.stateError : DubColor.spaceEcho,
                    reserve: Self.repeatLineReserve)
            } else {
                FxEngraved(
                    text: "\(Int((ms * 0.337).rounded())) · \(Int((ms * 0.668).rounded())) · \(Int(ms.rounded()))",
                    dim: true,
                    reserve: Self.repeatLineReserve)
            }
        }
    }

    /// Intensity is feedback; past 1.0 the tape self-oscillates.
    private var intensityCell: some View {
        let runaway = controls.spaceEchoRunaway
        return VStack(spacing: 2) {
            FxKnobCell(
                name: "Intensity",
                value: controls.spaceEchoIntensity / 1.2,
                readout: String(format: "%.2f", controls.spaceEchoIntensity),
                readoutColor: runaway ? DubColor.stateError : DubColor.textPrimary,
                reserve: ["0.00"]
            ) { v in
                var c = controls
                c.spaceEchoIntensity = v * 1.2
                onControls(c)
            }
            if runaway {
                if motion {
                    ZStack {
                        FxEngraved(text: "Osc > 1.0").hidden()
                        BlinkingEngraved(text: "Runaway", color: DubColor.stateError, on: on)
                    }
                } else {
                    FxEngraved(text: "Runaway", color: DubColor.stateError.opacity(on ? 1 : 0.35),
                               reserve: ["Osc > 1.0", "Runaway"])
                }
            } else {
                FxEngraved(text: "Osc > 1.0", dim: true, reserve: ["Osc > 1.0", "Runaway"])
            }
        }
    }
}

/// The RE-201's tape compartment: the loop past the record head and three
/// playback heads, the onboard spring beneath. Heads and the spring light
/// for the mode; the tape's dashes run while the unit is in — at the
/// tape's speed, and racing when it is feeding back on itself.
struct FxTapeWindow: View {
    let mode: SpaceEchoMode
    let running: Bool
    let runaway: Bool
    var motion: Bool = true

    private static let tapeColor = Color(hex: 0xB58A4A)

    /// The tape loop, past the record head and around both reels.
    private static let loop: Path = Path { p in
        p.move(to: CGPoint(x: 18, y: 24))
        p.addLine(to: CGPoint(x: 70, y: 24))
        p.addArc(center: CGPoint(x: 70, y: 44), radius: 20,
                 startAngle: .degrees(-90), endAngle: .degrees(90), clockwise: false)
        p.addLine(to: CGPoint(x: 18, y: 64))
        p.addArc(center: CGPoint(x: 18, y: 44), radius: 20,
                 startAngle: .degrees(90), endAngle: .degrees(270), clockwise: false)
        p.closeSubpath()
    }

    var body: some View {
        Canvas { ctx, size in
            let loop = Self.loop
            ctx.stroke(loop, with: .color(Color(hex: 0x2A221A)), lineWidth: 6)
            if !motion {
                ctx.stroke(loop, with: .color(Self.tapeColor),
                           style: StrokeStyle(lineWidth: 2.5, dash: [5, 4], dashPhase: 0))
            }
            for x in [18.0, 70.0] {
                let hub = Path(ellipseIn: CGRect(x: x - 8, y: 36, width: 16, height: 16))
                ctx.fill(hub, with: .color(Color(hex: 0x15171A)))
                ctx.stroke(hub, with: .color(Color(hex: 0x3A3E45)), lineWidth: 1.5)
                ctx.fill(Path(ellipseIn: CGRect(x: x - 2.5, y: 41.5, width: 5, height: 5)),
                         with: .color(Color(hex: 0x3A3E45)))
            }
            let heads = Self.heads(mode)
            head(ctx, x: 30, label: "R", live: true, color: Color(hex: 0x9AA0A8))
            head(ctx, x: 43, label: "1", live: heads.0, color: DubColor.spaceEcho)
            head(ctx, x: 56, label: "2", live: heads.1, color: DubColor.spaceEcho)
            head(ctx, x: 69, label: "3", live: heads.2, color: DubColor.spaceEcho)
            let reverb = Self.reverbOn(mode)
            let box = Path(roundedRect: CGRect(x: 31, y: 51, width: 26, height: 12), cornerRadius: 2)
            ctx.fill(box, with: .color(reverb ? Color(hex: 0x12261C) : Color(hex: 0x1A1C20)))
            ctx.stroke(box, with: .color(reverb ? DubColor.springFx : Color(hex: 0x3A3E45)), lineWidth: 1)
            ctx.draw(
                Text("RVB").font(.system(size: 6, weight: .bold))
                    .foregroundColor(reverb ? DubColor.springFx : Color(hex: 0x5A5F68)),
                at: CGPoint(x: 44, y: 57))
            ctx.draw(
                Text("TAPE LOOP").font(.system(size: 5.5, weight: .semibold))
                    .foregroundColor(DubColor.textTertiary),
                at: CGPoint(x: 24, y: 71))
        }
        .overlay {
            // The tape runs on Core Animation (FxLayerAnimations): racing
            // when it feeds back on itself.
            if motion {
                TravellingDashes(
                    path: Self.loop.cgPath, color: Self.tapeColor, lineWidth: 2.5,
                    dash: [5, 4], speed: runaway ? 34 : 17, running: running)
            }
        }
        .frame(width: 88, height: 76)
        .background(RoundedRectangle(cornerRadius: 3).fill(DubColor.displayWell))
        .overlay(RoundedRectangle(cornerRadius: 3).stroke(Color.black, lineWidth: 1))
    }

    private func head(_ ctx: GraphicsContext, x: CGFloat, label: String, live: Bool, color: Color) {
        let r = Path(roundedRect: CGRect(x: x - 5.5, y: 6, width: 11, height: 12), cornerRadius: 1.5)
        ctx.fill(r, with: .color(live ? Color(hex: 0x2A2113) : Color(hex: 0x1A1C20)))
        ctx.stroke(r, with: .color(live ? color : Color(hex: 0x3A3E45)), lineWidth: 1)
        ctx.draw(
            Text(label).font(.system(size: 6.5, weight: .bold))
                .foregroundColor(live ? color : Color(hex: 0x5A5F68)),
            at: CGPoint(x: x, y: 12))
    }

    /// Which playback heads a mode puts in circuit — `Re201Mode::config`.
    static func heads(_ mode: SpaceEchoMode) -> (Bool, Bool, Bool) {
        switch mode {
        case .reverb: return (false, false, false)
        case .short, .shortReverb: return (true, false, false)
        case .long, .longReverb: return (false, true, true)
        case .triple, .tripleReverb: return (true, true, true)
        }
    }

    static func reverbOn(_ mode: SpaceEchoMode) -> Bool {
        switch mode {
        case .reverb, .shortReverb, .longReverb, .tripleReverb: return true
        case .short, .long, .triple: return false
        }
    }
}

// MARK: - Spring

/// The tank Tubby kicked for thunder: two coils in a steel tray, a KICK
/// button on the end of it, decay / tone / wet beside.
struct FxSpringUnit: View {
    let on: Bool
    let controls: FxRackControls
    /// The key on the IN/OUT toggle, from the map.
    var toggleLegend: String? = nil
    /// Off: the coils' highlight holds one phase (a snapshot).
    var motion: Bool = true
    var onToggle: () -> Void = {}
    var onControls: (FxRackControls) -> Void = { _ in }
    var onKick: () -> Void = {}
    /// The key on KICK, from the map.
    var kickLegend: String? = nil

    static let height: CGFloat = 112

    @State private var kickedAt: Date?
    /// Map mode draws its own cap on KICK; the button's steps aside.
    @Environment(\.dubMapping) private var mapping

    var body: some View {
        FxUnitChassis(height: Self.height, on: on) {
            HStack(spacing: 10) {
                VStack(alignment: .leading, spacing: 5) {
                    FxDymoLabel(text: "Spring", tilt: 0.5)
                    FxEngraved(text: "Reverb tank")
                    FxEngraved(text: "Kick = thunder", dim: true)
                }
                .frame(width: 84, alignment: .leading)
                // One tray: the coils and, on its end, the button.
                HStack(spacing: 0) {
                    FxSpringTank(ringing: on, kickedAt: kickedAt, motion: motion)
                    kickButton
                }
                .clipShape(RoundedRectangle(cornerRadius: 3))
                .overlay(RoundedRectangle(cornerRadius: 3).stroke(Color.black, lineWidth: 1))
                HStack(alignment: .bottom, spacing: 2) {
                    FxKnobCell(name: "Decay", value: controls.springDecay, size: 24,
                               readout: Self.decayText(controls.springDecay),
                               reserve: ["0.0 s", "00 s"]) { v in
                        var c = controls
                        c.springDecay = v
                        onControls(c)
                    }
                    FxKnobCell(name: "Tone", value: controls.springTone, size: 24,
                               readout: Self.toneText(controls.springTone),
                               reserve: ["0.0 kHz", "000 Hz"]) { v in
                        var c = controls
                        c.springTone = v
                        onControls(c)
                    }
                    FxKnobCell(name: "Wet", value: controls.springWet, size: 24,
                               readout: percent(controls.springWet),
                               reserve: percentReserve) { v in
                        var c = controls
                        c.springWet = v
                        onControls(c)
                    }
                }
                .frame(maxWidth: .infinity)
                FxBatToggle(color: DubColor.springFx, on: on, legend: toggleLegend, onToggle: onToggle)
                    .mappable(.fxToggle(3))
            }
            .padding(.horizontal, 10)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(brushedSteel)
        }
    }

    /// The tray's end, with the button on it — press = one impulse.
    private var kickButton: some View {
        ZStack {
            LinearGradient(colors: [Color(hex: 0x17191C), Color(hex: 0x0D0E10)], startPoint: .top, endPoint: .bottom)
            let down = kickedAt.map { Date().timeIntervalSince($0) < 0.18 } ?? false
            Text("KICK")
                .font(.system(size: 7, weight: .bold))
                .tracking(1.2)
                .foregroundStyle(down ? DubColor.springFx : DubColor.textPrimary)
                .frame(width: 32, height: 32)
                .background(Circle().fill(RadialGradient(
                    colors: [Color(hex: 0x3A3D43), Color(hex: 0x16181B)],
                    center: UnitPoint(x: 0.45, y: 0.35), startRadius: 0, endRadius: 20)))
                .overlay(Circle().stroke(Color.white.opacity(0.12), lineWidth: 1))
                .shadow(color: .black, radius: 0, y: down ? 1 : 3)
                .offset(y: down ? 2 : 0)
        }
        .frame(width: 40, height: 66)
        .overlay(alignment: .leading) { Rectangle().fill(Color.black).frame(width: 1) }
        .overlay(alignment: .bottom) {
            if let kickLegend, mapping == nil {
                DubKeycap(key: kickLegend)
                    .padding(.bottom, 2)
            }
        }
        .contentShape(Rectangle())
        .onPressDown {
            kickedAt = Date()
            onKick()
        }
        .help("Kick the tank — thunder")
        .accessibilityElement()
        .accessibilityLabel("Kick")
        .accessibilityAddTraits(.isButton)
        .mappable(.fxKick)
    }

    private var brushedSteel: some View {
        ZStack {
            Color(hex: 0x363B41)
            Canvas { ctx, size in
                var y: CGFloat = 0
                while y < size.height {
                    var line = Path()
                    line.move(to: CGPoint(x: 0, y: y))
                    line.addLine(to: CGPoint(x: size.width, y: y))
                    ctx.stroke(line, with: .color(Color.white.opacity(0.035)), lineWidth: 1)
                    y += 3
                }
            }
        }
    }

    /// The tail's length from the feedback, as a real tank's RT60 — the
    /// spring is 33–41 ms long, and each pass loses `1 − fb`.
    static func decayText(_ decay: Double) -> String {
        let fb = max(decay * 0.97, 0.01)
        let rt60 = 0.037 * 6.908 / -log(fb)
        return rt60 >= 10 ? String(format: "%.0f s", rt60) : String(format: "%.1f s", rt60)
    }

    static func toneText(_ tone: Double) -> String {
        let hz = Double(springToneHz(tone: Float(tone)))
        return hz >= 1_000 ? String(format: "%.1f kHz", hz / 1_000) : "\(Int(hz.rounded())) Hz"
    }
}

/// Two coils in a steel tray, a highlight running along them while the
/// tank is in, and a teal flash for a moment after a kick.
struct FxSpringTank: View {
    let ringing: Bool
    let kickedAt: Date?
    var motion: Bool = true

    private static let rippleColor = DubColor.springFx.opacity(0.9)

    /// The highlight's run along each coil.
    private static let ripple: [(y: CGFloat, len: CGFloat)] = [(20, 82), (46, 62)]

    /// Redraw at a kick and once more as its flash ends — not every frame.
    private var kickMoments: [Date] {
        kickedAt.map { [$0, $0.addingTimeInterval(0.3)] } ?? []
    }

    var body: some View {
        TimelineView(.explicit(kickMoments)) { tl in
            Canvas { ctx, size in
                let t: Double = 0
                let flash = kickedAt.map { tl.date.timeIntervalSince($0) < 0.3 } ?? false
                for (x, y) in [(5.0, 20.0), (94.0, 20.0), (5.0, 46.0), (74.0, 46.0)] {
                    let tab = Path(roundedRect: CGRect(x: x, y: y - 8, width: 5, height: 16), cornerRadius: 1)
                    ctx.fill(tab, with: .color(Color(hex: 0x2B2E33)))
                    ctx.stroke(tab, with: .color(Color(hex: 0x4A4F58)), lineWidth: 1)
                }
                coil(ctx, x0: 13, y: 20, turns: 17, color: flash ? DubColor.springFx : Color(hex: 0x8B9096))
                coil(ctx, x0: 13, y: 46, turns: 13, color: flash ? DubColor.springFx : Color(hex: 0x7D838A))
                if ringing, !motion {
                    let phase = -CGFloat((t * 9).truncatingRemainder(dividingBy: 12))
                    for (y, len) in Self.ripple {
                        var line = Path()
                        line.move(to: CGPoint(x: 11, y: y))
                        line.addLine(to: CGPoint(x: 13 + len, y: y))
                        ctx.stroke(line, with: .color(DubColor.springFx.opacity(0.9)),
                                   style: StrokeStyle(lineWidth: 2, lineCap: .round, dash: [3, 9], dashPhase: phase))
                    }
                }
                ctx.draw(
                    Text("2 SPRINGS").font(.system(size: 5.5, weight: .semibold))
                        .foregroundColor(DubColor.textTertiary),
                    at: CGPoint(x: 84, y: 61))
            }
        }
        .overlay {
            // The highlight runs on Core Animation (FxLayerAnimations).
            if motion, ringing {
                TravellingDashes(
                    path: Self.ripplePath, color: Self.rippleColor, lineWidth: 2,
                    dash: [3, 9], speed: 9, lineCap: .round, running: true)
            }
        }
        .frame(width: 104, height: 66)
        .background(LinearGradient(
            colors: [Color(hex: 0x17191C), Color(hex: 0x0D0E10)],
            startPoint: .top, endPoint: .bottom))
    }

    private static var ripplePath: CGPath {
        let p = CGMutablePath()
        for (y, len) in ripple {
            p.move(to: CGPoint(x: 11, y: y))
            p.addLine(to: CGPoint(x: 13 + len, y: y))
        }
        return p
    }

    private func coil(_ ctx: GraphicsContext, x0: CGFloat, y: CGFloat, turns: Int, color: Color) {
        for i in 0..<turns {
            let e = Path(ellipseIn: CGRect(x: x0 + CGFloat(i) * 5 - 2.4, y: y - 6.5, width: 4.8, height: 13))
            ctx.stroke(e, with: .color(color), lineWidth: 1)
        }
    }
}

// MARK: - Readout helpers

/// `0.7` → `70 %`.
/// The width `percent` and `db` readouts keep: two digits, which covers
/// every dB reading (−60 … 0, then −∞). Only a percent knob at its end
/// stop reads wider; the rack has no room to reserve a third digit for it.
let percentReserve = ["00 %"]
let dbReserve = ["−00 dB"]

func percent(_ v: Double) -> String {
    "\(Int((v * 100).rounded())) %"
}

/// A linear 0…1 level as dB, `−∞` at nothing.
func db(_ v: Double) -> String {
    guard v > 0.001 else { return "−∞ dB" }
    let d = 20 * log10(v)
    return String(format: "%@%.0f dB", d < -0.5 ? "−" : "", abs(d.rounded()))
}
