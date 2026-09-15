//
//  FxChannelPane.swift
//  Dub
//
//  One deck's pane when the deck is the DUB FX channel (F-38 stage 2):
//  the record is replaced by the rack. The pane keeps the deck's grammar
//  — the source switch stays at the top of the column, the narrow inner
//  strip still scrolls bottom→top (the send signal now, not the groove),
//  and the wide column becomes a 19″ rack, the four units stacked in the
//  engine's order with each unit's own controls. Expert is the only mode
//  the channel has: there is no macro here.
//
//  Routing is the mixer's job. The input is the pair the deck's needle
//  used, with the mixer's aux send or FX loop patched into it; the
//  return goes out the deck's pair into a mixer channel. Deck A, the
//  MC's mic and the siren all reach the rack by the send knob — no
//  internal send bus, which would be a software mixer (PRD §5.3).
//

import SwiftUI

/// The pane: the input lane on the deck's inner edge, the rack on the
/// outer, mirrored per deck like the waveform and the column are. The
/// scope arrives as a closure because it is the Metal live waveform;
/// tests pass a placeholder.
struct FxChannelPane<Scope: View>: View {
    let state: FxChannelState
    var callbacks = FxChannelCallbacks()
    /// Width of the rack column — the same resolved width the deck column
    /// gets, handed down from the deck row's `GeometryReader`.
    var columnWidth: CGFloat?
    @ViewBuilder var scope: () -> Scope

    var body: some View {
        HStack(spacing: 0) {
            if state.side == .a {
                rack
                lane
            } else {
                lane
                rack
            }
        }
    }

    private var lane: some View {
        FxInputLane(state: state, callbacks: callbacks, scope: scope)
            .frame(
                minWidth: DubLayout.performanceWaveformMinWidth,
                idealWidth: DubLayout.performanceWaveformWidth,
                maxWidth: DubLayout.performanceWaveformWidthCap)
            .frame(maxHeight: .infinity)
            .layoutPriority(1)
    }

    private var rack: some View {
        FxRackColumn(state: state, callbacks: callbacks)
            .equatable()
            .frame(width: columnWidth ?? DubLayout.performanceDeckColumnMinWidth, alignment: .leading)
    }
}

/// The inner strip: INPUT — the SEND · MIC rocker, the trim, the siren
/// box's cream VU on the input — then the live scope, then OUT.
struct FxInputLane<Scope: View>: View {
    let state: FxChannelState
    var callbacks = FxChannelCallbacks()
    @ViewBuilder var scope: () -> Scope

    var body: some View {
        VStack(alignment: .leading, spacing: DubSpacing.sm) {
            SectionHeading(
                title: "INPUT", accent: DubColor.deckTint(state.side),
                trailing: state.inputPair.isEmpty ? nil : "IN \(state.inputPair)")
            rocker
            Text(state.input == .send
                 ? "the mixer's aux send or FX loop, on the pair the needle used"
                 : "a mic straight into the interface — no send on the mixer")
                .font(DubFont.micro)
                .foregroundStyle(DubColor.textTertiary)
                .fixedSize(horizontal: false, vertical: true)
            trim
            SirenMeterFace(
                title: state.input.title,
                echoLine: "\(state.inputPair) · \(state.trimText)",
                level: state.vuLevel)
                .frame(width: DubLayout.sirenDisplayWidth, height: DubLayout.sirenDisplayHeight)
                .background(RoundedRectangle(cornerRadius: 3).fill(DubColor.displayWell))
                .frame(maxWidth: .infinity)
            SectionHeading(
                title: "LIVE", accent: DubColor.deckTint(state.side),
                trailing: state.isHot ? "● HOT" : "● ",
                trailingAccent: state.isHot ? DubColor.stateError : DubColor.stateLocked)
            scope()
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .clipShape(RoundedRectangle(cornerRadius: 3))
            SectionHeading(
                title: "OUT", accent: DubColor.textTertiary,
                trailing: state.inputPair.isEmpty ? "MIXER" : "\(state.inputPair) · MIXER")
        }
        .padding(DubSpacing.md)
        .background(DubColor.waveformLane)
    }

    /// Two keys, one lit: what is patched in.
    private var rocker: some View {
        HStack(spacing: DubSpacing.xs) {
            ForEach(FxInputKind.allCases, id: \.self) { kind in
                let lit = state.input == kind
                DubPadCell(size: .word, lit: lit, tint: DubColor.deckTint(state.side)) {
                    Text(kind.title)
                        .font(.system(size: 11, weight: .semibold, design: .rounded))
                }
                .onPressDown { callbacks.onInput(kind) }
                .help(kind == .send
                      ? "The mixer's aux send or FX loop is patched into this pair"
                      : "A mic is plugged straight into this pair")
            }
        }
    }

    private var trim: some View {
        HStack(spacing: DubSpacing.sm) {
            VStack(alignment: .leading, spacing: 2) {
                Text("TRIM")
                    .font(DubFont.micro)
                    .tracking(DubFont.capsTracking)
                    .foregroundStyle(DubColor.textTertiary)
                Text(state.trimText)
                    .font(DubFont.numericInline)
                    .monospacedDigit()
                    .foregroundStyle(DubColor.textPrimary)
            }
            Spacer(minLength: 0)
            DubKnob(value: (state.trimDb + 24) / 48, size: 36, ink: DubColor.textTertiary) { v in
                callbacks.onTrim((v * 48 - 24).rounded() )
            }
            .padding(4)
            .help("Input trim, −24 … +24 dB")
        }
    }
}

/// The outer column: the source switch where it always was, the channel's
/// name and its signal path, and the rack.
struct FxRackColumn: View, Equatable {
    let state: FxChannelState
    var callbacks = FxChannelCallbacks()

    static func == (lhs: FxRackColumn, rhs: FxRackColumn) -> Bool {
        lhs.state == rhs.state
    }

    var body: some View {
        VStack(alignment: .leading, spacing: DubSpacing.sm) {
            sourceRow
            HStack(alignment: .firstTextBaseline, spacing: DubSpacing.sm) {
                Text("FX channel")
                    .font(DubFont.title)
                    .foregroundStyle(DubColor.textPrimary)
                    .fixedSize()
                Text("outboard on the mixer's send — the rack replaces the record")
                    .font(DubFont.micro)
                    .foregroundStyle(DubColor.textSecondary)
                    .lineLimit(1)
                    .truncationMode(.tail)
            }
            FxPathLine(state: state)
            VStack(spacing: 6) {
                FxBigKnobUnit(
                    on: state.isOn(.bigKnob), controls: state.controls,
                    toggleLegend: state.toggleLegend(.bigKnob),
                    onToggle: { callbacks.onToggle(.bigKnob) },
                    onControls: callbacks.onControls)
                FxPhaserUnit(
                    on: state.isOn(.phaser), controls: state.controls,
                    toggleLegend: state.toggleLegend(.phaser), motion: state.motion,
                    onToggle: { callbacks.onToggle(.phaser) },
                    onControls: callbacks.onControls)
                FxSpaceEchoUnit(
                    on: state.isOn(.spaceEcho), controls: state.controls,
                    toggleLegend: state.toggleLegend(.spaceEcho), motion: state.motion,
                    onToggle: { callbacks.onToggle(.spaceEcho) },
                    onControls: callbacks.onControls)
                FxSpringUnit(
                    on: state.isOn(.spring), controls: state.controls,
                    toggleLegend: state.toggleLegend(.spring), motion: state.motion,
                    onToggle: { callbacks.onToggle(.spring) },
                    onControls: callbacks.onControls,
                    onKick: callbacks.onKick,
                    kickLegend: state.kickLegend)
            }
            Spacer(minLength: 0)
        }
        .padding(DubSpacing.md)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(DubColor.surface1)
    }

    /// The switch on the column's inner edge, as on a music deck, and the
    /// mode's name on the outer — Expert is the channel's only mode.
    private var sourceRow: some View {
        HStack(spacing: 0) {
            if state.side == .a {
                expertChip
                Spacer(minLength: DubSpacing.lg)
                sourceSwitch
            } else {
                sourceSwitch
                Spacer(minLength: DubSpacing.lg)
                expertChip
            }
        }
        .padding(.bottom, DubSpacing.xs)
    }

    private var sourceSwitch: some View {
        SourceControlView(
            status: .fx,
            overridden: state.sourceOverridden,
            isPlaying: state.isPlaying,
            side: state.side,
            onInternal: callbacks.onSetInternal,
            onPause: callbacks.onPause,
            onTimecode: callbacks.onSetTimecode,
            onThru: callbacks.onSetThru,
            showFx: true,
            onFx: {},
            onRecalibrate: callbacks.onRecalibrate)
    }

    private var expertChip: some View {
        Text("EXPERT")
            .font(DubFont.caps)
            .tracking(0.8)
            .foregroundStyle(DubColor.textTertiary)
            .padding(.horizontal, DubSpacing.sm)
            .padding(.vertical, 2)
            .overlay(Capsule(style: .continuous).stroke(DubColor.textPlaceholder, lineWidth: 1))
            .help("The channel has one mode: every knob is the unit's own control. "
                + "Ride them from a controller once map mode binds them.")
    }
}

/// The signal path, in order, each unit lit in its tint while it is in.
struct FxPathLine: View {
    let state: FxChannelState

    var body: some View {
        HStack(spacing: 5) {
            word(state.input.title, DubColor.textTertiary)
            ForEach(FxRackUnit.allCases, id: \.self) { unit in
                arrow
                word(unit.title, state.isOn(unit) ? unit.tint : DubColor.textPlaceholder)
            }
            arrow
            word("OUT", DubColor.textTertiary)
        }
        .lineLimit(1)
    }

    private var arrow: some View {
        Text("→")
            .font(.system(size: 9, weight: .semibold, design: .monospaced))
            .foregroundStyle(DubColor.textPlaceholder)
    }

    private func word(_ text: String, _ color: Color) -> some View {
        Text(text)
            .font(.system(size: 9, weight: .semibold, design: .monospaced))
            .tracking(0.5)
            .foregroundStyle(color)
            .fixedSize()
    }
}
