//
//  PrepPadGrid.swift
//  Dub
//
//  Prep's control surface: three columns of sections.
//
//  Prep used to stack every row in one narrow leading column, leaving
//  ~70 % of the window empty — nothing constrained the width, so each
//  intrinsically-sized row simply hugged the leading edge of a
//  full-width container. Prep is the single-deck surface with the most
//  screen to spare and it was using the least of it.
//
//  A plain `HStack`, not `Grid` or `LazyVGrid`. There is one row of
//  three heterogeneous stacks with no cell relationship across columns,
//  so `Grid` would be an `HStack` with ceremony; and `LazyVGrid`'s
//  equal widths are actively wrong here — column 1 needs 344 and
//  column 3 needs 248, so equalising would clip the LOOP row, which is
//  the exact bug class this work exists to remove.
//
//  Only the FX column flexes. Columns 1 and 3 are pads and segmented
//  switches, fixed by design; stretching them just adds dead pixels.
//  The FX column is all sliders, where extra width is finer resolution
//  on a prepare-and-test surface.
//

import SwiftUI

/// Transport · FX · tuning, side by side.
struct PrepPadGrid: View {
    @ObservedObject var model: WaveformAppModel

    /// The FX column disappears entirely when both its features are
    /// off — an empty 288 pt gap between transport and tuning would
    /// read as a rendering fault.
    private var showsFxColumn: Bool { model.sirenEnabled || model.rackFxEnabled }

    var body: some View {
        HStack(alignment: .top, spacing: DubLayout.prepColumnGap) {
            transportColumn
                .frame(width: DubLayout.prepTransportColumn, alignment: .leading)
            if showsFxColumn {
                fxColumn
                    .frame(
                        minWidth: DubLayout.prepFxColumnMin,
                        maxWidth: DubLayout.prepFxColumnMax,
                        alignment: .leading)
            }
            tuningColumn
                .frame(width: DubLayout.prepTuningColumn, alignment: .leading)
            Spacer(minLength: 0)
        }
    }

    private var transportColumn: some View {
        VStack(alignment: .leading, spacing: DubSpacing.lg) {
            CuePadSection(
                cues: model.deckA.hotCues,
                onCue: { index, clear in
                    model.handleHotCue(.a, index: index, clear: clear)
                })
            LoopPadSection(
                activeBars: model.deckA.activeLoopBars,
                loopEngaged: model.deckA.loopActive,
                loopInArmed: model.deckA.pendingLoopInSecs != nil,
                onLoop: { bars in model.handleLoop(.a, bars: bars) },
                onLoopIn: { model.setLoopIn(.a) },
                onLoopOut: { model.setLoopOut(.a) },
                onExit: { model.exitLoop(.a) })
            // Prep surface; clickable for prepare + test. The no-mouse
            // rule (PRD §1) guards continuous performance gestures, not
            // momentary triggers.
            if model.echoOutEnabled {
                EchoPadSection(
                    engaged: model.deckA.echoDivision != nil,
                    onToggle: { model.toggleEchoOut(.a) })
            }
        }
    }

    private var fxColumn: some View {
        VStack(alignment: .leading, spacing: DubSpacing.lg) {
            if model.sirenEnabled {
                SirenPadRow(
                    names: model.sirenLabels(for: .a),
                    sounding: model.deckA.sirenState == 1,
                    onPreset: { idx in model.fireSirenPreset(.a, index: idx) },
                    dubMacro: model.deckA.sirenDubMacro,
                    onDubMacro: { value in model.setSirenDub(.a, value) },
                    unit: model.deckA.sirenUnit,
                    onUnit: { unit in model.setSirenUnit(.a, unit) })
                SirenExpertPanel(deck: model.deckA, model: model, side: .a)
            }
            // Dormant by default (UI-BACKLOG F-38 rebuilds it as an FX
            // channel rather than a per-deck row); at the bottom of
            // this column because it is an effect, not a tuning control.
            if model.rackFxEnabled {
                RackFxRow(
                    active: model.deckA.rackActive,
                    macro: model.deckA.rackMacro,
                    onToggle: { idx in model.toggleRackFx(.a, idx) },
                    onMacro: { idx, value in model.setRackMacro(.a, idx, value) })
            }
        }
    }

    private var tuningColumn: some View {
        VStack(alignment: .leading, spacing: DubSpacing.lg) {
            KeyLockControlView(model: model, side: .a)
            PitchTestView(model: model, side: .a)
        }
    }
}
