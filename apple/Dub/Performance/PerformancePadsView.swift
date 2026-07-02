//
//  PerformancePadsView.swift
//  Dub
//
//  Per-deck performance pads, occupying the outer space beside each
//  deck's waveform (the "whole lot of nothing" the centred-cluster
//  layout left behind). Modelled on Serato Scratch Live's pad area:
//  the running waveform + its overview sit toward the centre (next to
//  the phase clock), and the cue / loop / quick-scratch / sampler
//  controls live out here on the deck's outer edge.
//
//  The CUE and LOOP rows are live. CUE: keys 1–4 set/recall a hot cue
//  on the master deck, Shift+key clears it (see `handleHotCue`); a pad
//  lights in the deck's tint when its slot holds a position. LOOP (M13):
//  click a length pad to fire a grid-snapped reverse loop of that many
//  bars (the bars just heard), ✕ exits. A momentary pad click is within
//  the §1 mouse rule (not a continuous performance gesture); keyboard
//  bindings may follow as a convenience (PRD §5.5). Quick Scratch +
//  Sampler (M17) remain honest placeholders, laid out now so the surface
//  reads as a real performance instrument rather than empty space.
//

import AppKit
import DubCore
import SwiftUI

struct PerformancePadsView: View {

    let side: DeckSide

    /// Hot cue positions (track seconds) for the four CUE pads; `nil`
    /// = empty slot. Drives which pads light. Fed from the deck's
    /// `hotCues`, set/recalled by the 1–4 keys.
    var cues: [Double?] = [nil, nil, nil, nil]

    /// Active reverse-loop length in bars (`nil` = no loop). Lights the
    /// matching LOOP pad green.
    var activeLoopBars: Double? = nil
    /// Fire a grid-snapped reverse loop of `bars` bars (the bars just
    /// heard). No-op default keeps the `#Preview` simple.
    var onLoop: (_ bars: Double) -> Void = { _ in }
    /// Exit the active loop.
    var onExit: () -> Void = {}

    /// M15 echo-out: whether the (1-beat) echo-out is currently engaged.
    var echoEngaged: Bool = false
    /// Toggle the dub echo on / off.
    var onEchoToggle: () -> Void = {}
    /// Whether the echo-out feature is enabled (Preferences). When off the
    /// ECHO button is hidden entirely.
    var echoEnabled: Bool = true

    /// M16 dub-siren preset names (fire order); empty hides the grid.
    var sirenPresetNames: [String] = []
    /// Whether the engine reports the siren sounding (lights the header dot).
    var sirenSounding: Bool = false
    /// Fire preset `index` on this deck.
    var onSirenPreset: (_ index: Int) -> Void = { _ in }
    /// Whether the dub-siren feature is enabled (Preferences). When off the
    /// SIREN grid is hidden entirely.
    var sirenEnabled: Bool = false

    /// Siren Advanced "dub" super-knob position (0..1).
    var sirenDubMacro: Double = 0.0
    /// Set the siren dub super-knob.
    var onSirenDubMacro: (_ value: Double) -> Void = { _ in }
    /// The selected siren unit (GS1 / DS01E / SN76477).
    var sirenUnit: SirenUnit = .gs1
    /// Switch the siren unit.
    var onSirenUnit: (_ unit: SirenUnit) -> Void = { _ in }

    /// Vintage-FX rack engaged flags, in `RackFx` order
    /// (`[Spring, SpaceEcho, BigKnob, Phaser]`). Lights each slot's button.
    var rackActive: [Bool] = [false, false, false, false]
    /// Vintage-FX rack macro (super-knob) positions 0..1, same order.
    var rackMacro: [Double] = [0.5, 0.5, 0.5, 0.5]
    /// Toggle rack slot `index` on / off.
    var onRackToggle: (_ index: Int) -> Void = { _ in }
    /// Set rack slot `index`'s macro to `value` (0..1).
    var onRackMacro: (_ index: Int, _ value: Double) -> Void = { _, _ in }
    /// Whether the vintage-FX rack is enabled (Preferences). When off the rack
    /// is hidden entirely.
    var rackEnabled: Bool = false

    /// Hug the deck: deck A's pads (window-left) sit against their
    /// overview on the right; deck B's (window-right) sit against
    /// their overview on the left.
    private var frameAlignment: Alignment { side == .a ? .trailing : .leading }

    var body: some View {
        VStack(alignment: .leading, spacing: DubSpacing.lg) {
            cueGroup()
            loopGroup()
            if echoEnabled {
                EchoPadRow(engaged: echoEngaged, onToggle: onEchoToggle)
            }
            if sirenEnabled {
                SirenPadRow(
                    names: sirenPresetNames,
                    sounding: sirenSounding,
                    onPreset: onSirenPreset,
                    dubMacro: sirenDubMacro,
                    onDubMacro: onSirenDubMacro,
                    unit: sirenUnit,
                    onUnit: onSirenUnit)
            }
            if rackEnabled {
                RackFxRow(
                    active: rackActive,
                    macro: rackMacro,
                    onToggle: onRackToggle,
                    onMacro: onRackMacro)
            }
            padGroup("QUICK SCRATCH", keys: side == .a ? ["Q", "W"] : ["E", "R"])
            padGroup("SAMPLER", keys: side == .a ? ["A", "S"] : ["D", "F"])
        }
        .padding(.horizontal, DubSpacing.xl)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: frameAlignment)
        .background(DubColor.surface0)
    }

    /// The live CUE row. No "soon" tag; pads light in the deck tint
    /// when set.
    @ViewBuilder
    private func cueGroup() -> some View {
        VStack(alignment: .leading, spacing: DubSpacing.sm) {
            Text("CUE")
                .font(DubFont.caps)
                .tracking(0.8)
                .foregroundStyle(DubColor.textSecondary)
            HStack(spacing: DubSpacing.sm) {
                ForEach(0..<4, id: \.self) { index in
                    pad("\(index + 1)", lit: index < cues.count && cues[index] != nil)
                }
            }
        }
    }

    private static let loopPresets: [LoopPreset] = [
        LoopPreset(bars: 0.5, label: "½"),
        LoopPreset(bars: 1, label: "1"),
        LoopPreset(bars: 2, label: "2"),
        LoopPreset(bars: 4, label: "4"),
    ]

    /// The live LOOP row: each length pad fires a grid-snapped reverse
    /// loop of that many bars; the lit pad is the active length, ✕ exits.
    /// Clickable like the Prep `LoopPadRow` — a momentary loop trigger is
    /// within the §1 mouse rule (not a continuous performance gesture);
    /// keyboard bindings may follow as a convenience (PRD §5.5).
    @ViewBuilder
    private func loopGroup() -> some View {
        VStack(alignment: .leading, spacing: DubSpacing.sm) {
            Text("LOOP")
                .font(DubFont.caps)
                .tracking(0.8)
                .foregroundStyle(DubColor.textSecondary)
            HStack(spacing: DubSpacing.sm) {
                ForEach(Self.loopPresets) { preset in
                    hotCuePadCell(
                        preset.label,
                        lit: activeLoopBars == preset.bars,
                        tint: DubColor.loop
                    )
                    .onPressDown { onLoop(preset.bars) }
                    .help("Loop the last \(preset.label) bar\(preset.bars == 1 ? "" : "s")")
                }
                hotCuePadCell("✕", lit: false, tint: DubColor.loop)
                    .onPressDown(enabled: activeLoopBars != nil) { onExit() }
                    .help("Exit loop")
                    .opacity(activeLoopBars == nil ? 0.5 : 1.0)
            }
        }
    }

    @ViewBuilder
    private func padGroup(_ label: String, keys: [String]) -> some View {
        VStack(alignment: .leading, spacing: DubSpacing.sm) {
            HStack(spacing: DubSpacing.sm) {
                Text(label)
                    .font(DubFont.caps)
                    .tracking(0.8)
                    .foregroundStyle(DubColor.textSecondary)
                Text("soon")
                    .font(DubFont.micro)
                    .foregroundStyle(DubColor.textPlaceholder)
            }
            HStack(spacing: DubSpacing.sm) {
                ForEach(keys, id: \.self) { pad($0) }
            }
        }
    }

    private func pad(_ glyph: String, lit: Bool = false) -> some View {
        hotCuePadCell(glyph, lit: lit)
    }
}

/// One pad cell — `glyph` centred, lit in `tint` when active. Shared
/// by the Performance pad panel, the Prep cue bar (magenta), and the
/// Prep loop bar (green), so a lit pad reads the same as its marker on
/// the waveform / overview.
@ViewBuilder
private func hotCuePadCell(_ glyph: String, lit: Bool, tint: Color = DubColor.hotCue)
    -> some View
{
    Text(glyph)
        .font(.system(size: 13, weight: .semibold, design: .rounded))
        .foregroundStyle(lit ? DubColor.textPrimary : DubColor.textTertiary)
        .frame(width: glyph.count > 1 ? 50 : 38, height: 36)
        .background(lit ? tint.opacity(0.24) : DubColor.surface1)
        .clipShape(RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous)
                .stroke(lit ? tint : DubColor.divider, lineWidth: 1))
}

/// Horizontal, **clickable** hot cue pad row for Prep mode (where
/// the mouse is a first-class input, unlike Performance). Click an
/// empty pad to set a cue at the playhead, a set pad to jump to it,
/// ⇧-click to clear — mirroring the 1–4 / ⇧+1–4 keyboard gestures.
struct CuePadRow: View {

    let cues: [Double?]
    /// `(index, clear)` — `clear` is `true` when ⇧ is held at click.
    let onCue: (_ index: Int, _ clear: Bool) -> Void

    var body: some View {
        HStack(spacing: DubSpacing.sm) {
            Text("CUE")
                .font(DubFont.caps)
                .tracking(0.8)
                .foregroundStyle(DubColor.textSecondary)
                .frame(width: PrepPadLayout.labelWidth, alignment: .leading)
            ForEach(0..<4, id: \.self) { index in
                let isSet = index < cues.count && cues[index] != nil
                hotCuePadCell("\(index + 1)", lit: isSet)
                    .onPressDown {
                        onCue(index, NSEvent.modifierFlags.contains(.shift))
                    }
                    .help(isSet
                        ? "Cue \(index + 1) — click to jump, ⇧-click to clear"
                        : "Cue \(index + 1) — click to set at the playhead")
            }
        }
    }
}

/// Single tap-toggle ECHO OUT button (M15, PRD §6.3). One control for the
/// whole feature: tap on → the deck's dry mutes (100 % wet) and the last beat
/// repeats and decays; tap again → off, the deck resumes at its slipped
/// position. Lit while engaged. Shared by the Performance and Prep surfaces.
///
/// A tap is a momentary trigger, not a continuous performance gesture, so the
/// mouse is within the §1 rule (like cues + loops).
@ViewBuilder
private func echoOutButton(
    _ label: String,
    engaged: Bool,
    onToggle: @escaping () -> Void
) -> some View {
    Text(label)
        .font(.system(size: 12, weight: .semibold, design: .rounded))
        .foregroundStyle(engaged ? DubColor.textPrimary : DubColor.textTertiary)
        .frame(height: 36)
        .padding(.horizontal, DubSpacing.lg)
        .background(engaged ? DubColor.echo.opacity(0.24) : DubColor.surface1)
        .clipShape(RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous)
                .stroke(engaged ? DubColor.echo : DubColor.divider, lineWidth: 1))
        .contentShape(Rectangle())
        .onPressDown { onToggle() }
        .help("Echo out — 1 beat, 100% wet. Tap on, tap off.")
}

/// Performance echo-out row: the single ECHO OUT button.
struct EchoPadRow: View {

    /// Whether the echo-out is engaged. Lights the button.
    let engaged: Bool
    /// Toggle on / off.
    let onToggle: () -> Void

    var body: some View {
        echoOutButton("ECHO OUT", engaged: engaged, onToggle: onToggle)
    }
}

/// Prep-mode echo-out row: same single button in the inline label-gutter
/// layout the Prep pad bar uses (matching `LoopPadRow`). Prep's role is
/// prepare + test, so it's mouse-clickable here too.
struct PrepEchoPadRow: View {

    /// Whether the echo-out is engaged.
    let engaged: Bool
    /// Toggle on / off.
    let onToggle: () -> Void

    var body: some View {
        HStack(spacing: DubSpacing.sm) {
            Text("ECHO")
                .font(DubFont.caps)
                .tracking(0.8)
                .foregroundStyle(DubColor.textSecondary)
                .frame(width: PrepPadLayout.labelWidth, alignment: .leading)
            echoOutButton("OUT", engaged: engaged, onToggle: onToggle)
        }
    }
}

/// Layout-independent keyboard keys for the siren presets — the bottom letter
/// row Z X C V B N M , → indices 0–7 (keyCodes wired in `KeyEventMonitorHost`).
/// Shown as pad hints.
private let sirenPresetKeys = ["Z", "X", "C", "V", "B", "N", "M", ","]

/// One siren preset pad: its name plus the keyboard hint. Fires the one-shot on
/// mouse-down (a momentary trigger, within the §1 mouse rule).
@ViewBuilder
private func sirenPresetPad(_ label: String, key: String, onPress: @escaping () -> Void)
    -> some View
{
    VStack(spacing: 1) {
        Text(label.uppercased())
            .font(.system(size: 11, weight: .semibold, design: .rounded))
            .lineLimit(1)
            .minimumScaleFactor(0.7)
        Text(key)
            .font(DubFont.micro)
            .foregroundStyle(DubColor.textPlaceholder)
    }
    .foregroundStyle(DubColor.textTertiary)
    .frame(width: 64, height: 36)
    .background(DubColor.surface1)
    .clipShape(RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous))
    .overlay(
        RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous)
            .stroke(DubColor.divider, lineWidth: 1))
    .contentShape(Rectangle())
    .onPressDown(perform: onPress)
    .help("Fire the \(label) siren (\(key))")
}

/// Simple-mode dub-siren panel (M16, PRD §6.3): a grid of preset one-shot pads
/// (siren / alarm / laser / bomb / gun …), laid out four per row. Tap a pad (or
/// its Z X C V B N M , key) to fire the classic sound; it plays through the
/// built-in slap-back echo and stops itself. Value-driven (names come from the
/// engine bank); a header dot lights while the deck's siren is sounding.
/// (Advanced / Expert modes — live tweaking — arrive later.)
struct SirenPadRow: View {
    /// Preset display names, in fire order (index = preset id).
    let names: [String]
    /// Whether the engine reports the deck's siren sounding (lights the dot).
    let sounding: Bool
    /// Fire preset `index` on this deck.
    let onPreset: (_ index: Int) -> Void
    /// Advanced "dub" super-knob position (0..1).
    var dubMacro: Double = 0.0
    /// Set the dub super-knob.
    var onDubMacro: (_ value: Double) -> Void = { _ in }
    /// The selected siren unit (GS1 shots · Benidub DS01E · SN76477).
    var unit: SirenUnit = .gs1
    /// Switch the siren unit.
    var onUnit: (_ unit: SirenUnit) -> Void = { _ in }

    private var rows: [[Int]] {
        let idx = Array(names.indices)
        return stride(from: 0, to: idx.count, by: 4).map { Array(idx[$0..<min($0 + 4, idx.count)]) }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: DubSpacing.sm) {
            HStack(spacing: DubSpacing.xs) {
                Circle()
                    .fill(sounding ? DubColor.siren : DubColor.divider)
                    .frame(width: 7, height: 7)
                Text("SIREN")
                    .font(DubFont.caps)
                    .tracking(0.8)
                    .foregroundStyle(DubColor.textSecondary)
                Spacer(minLength: DubSpacing.sm)
                // Unit selector: GS1 toy-chip shots · Benidub DS01E · SN76477.
                Picker(
                    "Siren unit",
                    selection: Binding(get: { unit }, set: { onUnit($0) })
                ) {
                    Text("GS1").tag(SirenUnit.gs1)
                    Text("DS01E").tag(SirenUnit.ds01e)
                    Text("SN76477").tag(SirenUnit.sn76477)
                }
                .pickerStyle(.segmented)
                .labelsHidden()
                .controlSize(.mini)
                .frame(width: 210)
                .help("Siren unit — GS1 (toy-chip shots) · Benidub DS01E (analog) · SN76477 chip")
            }
            ForEach(rows, id: \.self) { row in
                HStack(spacing: DubSpacing.sm) {
                    ForEach(row, id: \.self) { idx in
                        sirenPresetPad(
                            names[idx],
                            key: idx < sirenPresetKeys.count ? sirenPresetKeys[idx] : "",
                            onPress: { onPreset(idx) })
                    }
                }
            }
            // Advanced: one "DUB" super-knob driving the siren's onboard echo
            // (Speed + Delay + Feedback + Mix). 0 = dry. The siren's own echo,
            // separate from the FX rack.
            HStack(spacing: DubSpacing.sm) {
                Text("DUB")
                    .font(DubFont.micro)
                    .foregroundStyle(dubMacro > 0 ? DubColor.siren : DubColor.textTertiary)
                    .frame(width: 28, alignment: .leading)
                Slider(
                    value: Binding(get: { dubMacro }, set: { onDubMacro($0) }),
                    in: 0...1)
                    .controlSize(.mini)
                    .tint(DubColor.siren)
                    .frame(maxWidth: 180)
                    .help("Dub super-knob — one knob adds the siren's own echo (delay + feedback). 0 = dry.")
            }
        }
    }
}

/// The **Expert** siren panel (PRD §6.3): the individual knobs/buttons, matching
/// the real units' control surfaces. The echo section (TIME / FEEDBACK / ECHO /
/// FILTER / VOLUME + ECHO CUT) is shared by every unit; below it are the
/// unit-specific controls — GS1: SPEED · DS01E: PITCH / RATE / TRIGGER ·
/// SN76477: none. Reads the deck's stored state; writes through model methods
/// (each pushes the full `set_siren_controls` / `set_siren_voice`). An
/// expandable section so it stays out of the way until needed.
struct SirenExpertPanel: View {
    /// The deck's current state (read).
    let deck: DeckState
    /// The app model (write — method calls push to the engine).
    let model: WaveformAppModel
    /// Which deck these controls drive.
    let side: DeckSide

    var body: some View {
        VStack(alignment: .leading, spacing: DubSpacing.xs) {
            Button {
                model.toggleSirenExpert(side)
            } label: {
                Text(deck.sirenExpertShown ? "EXPERT ▾" : "EXPERT ▸")
                    .font(DubFont.caps)
                    .tracking(0.8)
                    .foregroundStyle(DubColor.textSecondary)
            }
            .buttonStyle(.plain)

            if deck.sirenExpertShown {
                // Shared echo section (the PT2399, every unit).
                knob("TIME", deck.sirenDelayMs, 50...1000) { model.setSirenDelay(side, $0) }
                knob("FEEDBACK", deck.sirenFeedback, 0...1.0) { model.setSirenFeedback(side, $0) }
                knob("ECHO", deck.sirenMix, 0...1) { model.setSirenMix(side, $0) }
                knob("FILTER", deck.sirenFilter, 0...1) { model.setSirenFilter(side, $0) }
                knob("VOLUME", deck.sirenVolume, 0...1.5) { model.setSirenVolume(side, $0) }
                Text("ECHO CUT")
                    .font(.system(size: 11, weight: .semibold, design: .rounded))
                    .foregroundStyle(DubColor.textTertiary)
                    .frame(width: 92, height: 28)
                    .background(DubColor.surface1)
                    .clipShape(RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous))
                    .overlay(
                        RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous)
                            .stroke(DubColor.divider, lineWidth: 1))
                    .contentShape(Rectangle())
                    .onPressHold(
                        onDown: { model.setSirenEchoCut(side, true) },
                        onUp: { model.setSirenEchoCut(side, false) })
                    .help("Echo cut — hold to mute the echo (the loop keeps running underneath)")

                // Unit-specific controls.
                switch deck.sirenUnit {
                case .gs1:
                    knob("SPEED", deck.sirenSpeed, 0.25...4.0) { model.setSirenSpeed(side, $0) }
                case .ds01e:
                    HStack(spacing: DubSpacing.sm) {
                        Text("PITCH")
                            .font(DubFont.micro)
                            .foregroundStyle(DubColor.textTertiary)
                            .frame(width: 70, alignment: .leading)
                        Picker(
                            "Pitch",
                            selection: Binding(
                                get: { deck.sirenPitchIndex },
                                set: { model.setSirenPitch(side, $0) })
                        ) {
                            Text("Lo").tag(0)
                            Text("Mid").tag(1)
                            Text("Hi").tag(2)
                        }
                        .pickerStyle(.segmented)
                        .labelsHidden()
                        .controlSize(.mini)
                        .frame(width: 150)
                    }
                    knob("RATE", deck.sirenRate, 0...12.0) { model.setSirenRate(side, $0) }
                    Toggle(
                        isOn: Binding(
                            get: { deck.sirenContinuous },
                            set: { model.setSirenContinuous(side, $0) })
                    ) {
                        Text("HOLD (continuous)")
                            .font(DubFont.micro)
                            .foregroundStyle(DubColor.textTertiary)
                    }
                    .toggleStyle(.switch)
                    .controlSize(.mini)
                case .sn76477:
                    EmptyView() // the SN76477 plays fixed preset patches
                @unknown default:
                    EmptyView()
                }
            }
        }
    }

    /// One labelled mini-slider row.
    @ViewBuilder
    private func knob(
        _ label: String,
        _ value: Double,
        _ range: ClosedRange<Double>,
        _ set: @escaping (Double) -> Void
    ) -> some View {
        HStack(spacing: DubSpacing.sm) {
            Text(label)
                .font(DubFont.micro)
                .foregroundStyle(DubColor.textTertiary)
                .frame(width: 70, alignment: .leading)
            Slider(value: Binding(get: { value }, set: { set($0) }), in: range)
                .controlSize(.mini)
                .tint(DubColor.siren)
                .frame(maxWidth: 180)
        }
    }
}

/// Vintage-FX rack labels + accent colours, in `RackFx` order (index = slot id:
/// 0 Spring · 1 Space Echo · 2 Big Knob · 3 Phaser).
private let rackFxLabels = ["SPRING", "SPACE ECHO", "BIG KNOB", "PHASER"]
private let rackFxColors: [Color] = [
    DubColor.springFx, DubColor.spaceEcho, DubColor.bigKnob, DubColor.phaser,
]

/// The vintage-FX rack panel (PRD §6.3 — the King Tubby / Lee Perry processing
/// chain). One row per effect: a tap-toggle engage button + a single **macro**
/// "super-knob" (Advanced mode) that fans across the effect's params on a
/// hand-tuned curve. Spring + Space Echo are reverb/echo sends; Big Knob +
/// Phaser are inserts. (Expert per-param panels arrive later.)
///
/// The toggle is a momentary tap (within the §1 mouse rule). The macro slider
/// is the one-knob Advanced control; live-riding it from a real controller is
/// the intended performance path, the on-screen slider is for prep/testing.
struct RackFxRow: View {
    /// Engaged flag per slot (index = `RackFx` id).
    let active: [Bool]
    /// Macro position per slot, 0..1.
    let macro: [Double]
    /// Toggle slot `index`.
    let onToggle: (_ index: Int) -> Void
    /// Set slot `index`'s macro to `value`.
    let onMacro: (_ index: Int, _ value: Double) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: DubSpacing.sm) {
            HStack(spacing: DubSpacing.xs) {
                Circle()
                    .fill(active.contains(true) ? DubColor.bigKnob : DubColor.divider)
                    .frame(width: 7, height: 7)
                Text("FX RACK")
                    .font(DubFont.caps)
                    .tracking(0.8)
                    .foregroundStyle(DubColor.textSecondary)
            }
            ForEach(0..<rackFxLabels.count, id: \.self) { slot in
                slotRow(slot)
            }
        }
    }

    @ViewBuilder
    private func slotRow(_ slot: Int) -> some View {
        let on = slot < active.count && active[slot]
        let tint = rackFxColors[slot]
        HStack(spacing: DubSpacing.sm) {
            Text(rackFxLabels[slot])
                .font(.system(size: 11, weight: .semibold, design: .rounded))
                .lineLimit(1)
                .minimumScaleFactor(0.7)
                .foregroundStyle(on ? DubColor.textPrimary : DubColor.textTertiary)
                .frame(width: 92, height: 32)
                .background(on ? tint.opacity(0.24) : DubColor.surface1)
                .clipShape(RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous))
                .overlay(
                    RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous)
                        .stroke(on ? tint : DubColor.divider, lineWidth: 1))
                .contentShape(Rectangle())
                .onPressDown { onToggle(slot) }
                .help("\(rackFxLabels[slot]) — tap to engage / bypass")
            Slider(
                value: Binding(
                    get: { slot < macro.count ? macro[slot] : 0.5 },
                    set: { onMacro(slot, $0) }),
                in: 0...1)
                .controlSize(.mini)
                .tint(tint)
                .frame(width: 104)
                .help("Macro — one knob, curated for a good-sounding result")
        }
    }
}

extension View {
    /// Fire `perform` on mouse-**down** (press), not mouse-up.
    ///
    /// Use for timing-sensitive taps — hot cues, the BPM tap — where
    /// the handler captures the live playhead (or a wall-clock tap
    /// timestamp) at the click instant. A SwiftUI `Button` fires its
    /// action on mouse-**up**, so on a playing deck the captured
    /// point lands the whole click-hold duration (tens of ms) too
    /// late; the keyboard path fires on key-down and is correct, so
    /// the two disagreed. Pressing down matches the keyboard and a
    /// real hardware pad (note-on = press). `enabled == false` makes
    /// it inert (left-click does nothing) while leaving any sibling
    /// `contextMenu` / right-click reachable.
    func onPressDown(enabled: Bool = true, perform: @escaping () -> Void) -> some View {
        modifier(PressDownModifier(enabled: enabled, perform: perform))
    }

    /// Fire `onDown` on mouse-down and `onUp` on release — a momentary
    /// press-and-hold (e.g. ECHO CUT: cut while held, restore on release).
    func onPressHold(
        onDown: @escaping () -> Void,
        onUp: @escaping () -> Void
    ) -> some View {
        modifier(PressHoldModifier(onDown: onDown, onUp: onUp))
    }
}

private struct PressHoldModifier: ViewModifier {
    let onDown: () -> Void
    let onUp: () -> Void
    @State private var pressing = false

    func body(content: Content) -> some View {
        content
            .contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: 0)
                    .onChanged { _ in
                        guard !pressing else { return }
                        pressing = true
                        onDown()
                    }
                    .onEnded { _ in
                        pressing = false
                        onUp()
                    })
    }
}

private struct PressDownModifier: ViewModifier {
    let enabled: Bool
    let perform: () -> Void
    @State private var pressing = false

    func body(content: Content) -> some View {
        content
            .contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: 0)
                    .onChanged { _ in
                        guard enabled, !pressing else { return }
                        pressing = true
                        perform()
                    }
                    .onEnded { _ in pressing = false })
    }
}


/// Shared geometry for the Prep pad rows so the CUE and LOOP rows
/// line their pad columns up under a fixed-width label gutter.
private enum PrepPadLayout {
    static let labelWidth: CGFloat = 44
}

/// One reverse-loop length preset: a label and its length in bars.
private struct LoopPreset: Identifiable {
    let bars: Double
    let label: String
    var id: Double { bars }
}

/// Live LOOP pad row for Prep. Each length pad triggers a grid-snapped
/// **reverse** loop of that many bars — the bars just heard — on
/// mouse-down (like cues / transport). The active length lights green;
/// the ✕ pad exits the loop. Prep's loop role is *authoring + testing*
/// a region (PRD §3.1), so it's mouse-clickable, not keyboard-only.
struct LoopPadRow: View {

    /// Which length pad is lit (bars), or `nil` when no loop is active.
    let activeBars: Double?
    /// Trigger a reverse loop of `bars` bars.
    let onLoop: (_ bars: Double) -> Void
    /// Exit the active loop.
    let onExit: () -> Void

    private static let presets: [LoopPreset] = [
        LoopPreset(bars: 0.5, label: "½"),
        LoopPreset(bars: 1, label: "1"),
        LoopPreset(bars: 2, label: "2"),
        LoopPreset(bars: 4, label: "4"),
    ]

    var body: some View {
        HStack(spacing: DubSpacing.sm) {
            Text("LOOP")
                .font(DubFont.caps)
                .tracking(0.8)
                .foregroundStyle(DubColor.textSecondary)
                .frame(width: PrepPadLayout.labelWidth, alignment: .leading)
            ForEach(Self.presets) { preset in
                hotCuePadCell(
                    preset.label,
                    lit: activeBars == preset.bars,
                    tint: DubColor.loop
                )
                .onPressDown { onLoop(preset.bars) }
                .help("Loop the last \(preset.label) bar\(preset.bars == 1 ? "" : "s")")
            }
            hotCuePadCell("✕", lit: false, tint: DubColor.loop)
                .onPressDown(enabled: activeBars != nil) { onExit() }
                .help("Exit loop")
                .opacity(activeBars == nil ? 0.5 : 1.0)
        }
    }
}

#Preview("Performance pads") {
    HStack(spacing: 1) {
        PerformancePadsView(side: .a, cues: [12.0, nil, 48.5, nil])
        PerformancePadsView(side: .b, cues: [nil, 4.0, nil, nil])
    }
    .frame(width: 800, height: 360)
    .background(DubColor.surface0)
}
