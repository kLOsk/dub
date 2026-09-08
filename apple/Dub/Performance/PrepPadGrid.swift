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
//  heterogeneous stacks with no cell relationship across columns, so
//  `Grid` would be an `HStack` with ceremony; and `LazyVGrid`'s equal
//  widths are actively wrong here — the transport column needs 344 and
//  equalising would clip the LOOP row, which is the exact bug class
//  this work exists to remove.
//
//  Only the FX column flexes. Column 1 is pads, fixed by design;
//  stretching it just adds dead pixels. The FX column is all sliders,
//  where extra width is finer resolution on a prepare-and-test surface.
//
//  Key lock and the pitch-test steps used to sit in a third column.
//  They were M14 instrumentation for A/B-ing the key-lock engines
//  without a turntable, not something a DJ preparing a track reaches
//  for, so they are gone.
//

import SwiftUI

/// Transport pads and the FX column, side by side.
struct PrepPadGrid: View {
    @ObservedObject var model: WaveformAppModel

    /// The FX column disappears entirely when both its features are
    /// off, rather than leaving a 288 pt gap that reads as a rendering
    /// fault.
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
                SirenExpertPanel(
                    deck: model.deckA,
                    callbacks: SirenExpertPanel.Callbacks(
                        onToggleExpert: { model.toggleSirenExpert(.a) },
                        onDelay: { model.setSirenDelay(.a, $0) },
                        onFeedback: { model.setSirenFeedback(.a, $0) },
                        onMix: { model.setSirenMix(.a, $0) },
                        onFilter: { model.setSirenFilter(.a, $0) },
                        onVolume: { model.setSirenVolume(.a, $0) },
                        onEchoCut: { model.setSirenEchoCut(.a, $0) },
                        onSpeed: { model.setSirenSpeed(.a, $0) },
                        onPitch: { model.setSirenPitch(.a, $0) },
                        onRate: { model.setSirenRate(.a, $0) },
                        onContinuous: { model.setSirenContinuous(.a, $0) }))
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
}
