//
//  GlobalRackBar.swift
//  Dub
//
//  The horizontal bar under the deck panes: siren · Quick Scratch ·
//  sampler.
//
//  It occupies the slot `FXBarPlaceholder` used to fill with
//  "ECHO-OUT — Coming soon" and "DUB SIREN — Coming soon" cards,
//  duplicated per deck, months after both features shipped and moved
//  into the deck columns. The PRD §9.2 wireframe always put a bar here;
//  what it holds now is the three racks that were never per-deck to
//  begin with.
//
//  Draining these out of the deck columns is also what fixes the
//  overflow: the column needed ~478 pt of a ~330 pt pane, and the
//  bottom of it was being painted over by this bar's opaque
//  predecessor.
//

import SwiftUI

/// Siren · Quick Scratch · sampler, one of each, full width.
struct GlobalRackBar: View {
    let state: GlobalRackBarState
    var callbacks = GlobalRackBarCallbacks()

    var body: some View {
        HStack(spacing: 1) {
            if let siren = state.siren {
                group {
                    SirenRackGroup(
                        state: siren,
                        onPreset: callbacks.onSirenPreset,
                        onUnit: callbacks.onSirenUnit,
                        onDubMacro: callbacks.onSirenDubMacro)
                }
                .layoutPriority(1)
            }
            group {
                TriggerPadGroup(
                    title: "QUICK SCRATCH",
                    pads: state.quickScratch,
                    onTrigger: callbacks.onQuickScratch)
            }
            group {
                TriggerPadGroup(
                    title: "SAMPLER",
                    pads: state.sampler,
                    onTrigger: callbacks.onSampler,
                    onSecondary: callbacks.onSamplerStop)
            }
        }
        .frame(height: DubLayout.rackBarHeight)
        .background(DubColor.divider)
    }

    @ViewBuilder
    private func group<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
        content()
            .padding(.horizontal, DubSpacing.lg)
            .padding(.vertical, DubSpacing.md)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .background(DubColor.surface2)
    }
}
