//
//  GlobalRackBar.swift
//  Dub
//
//  The horizontal bar under the deck panes: siren · sampler. The
//  sampler is `SampleShelf` — the same eight tiles Prep loads, so a slot
//  learned in one place is the slot found in the other. Quick Scratch
//  drew here as a third block of four pads until PRD §7.2 got its way
//  back: it is a tag on a sampler tile now, fired from a row in each
//  deck column, and the bar's width went to the shelf.
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

/// Siren · sampler, one of each, full width.
///
/// **Where the slack goes.** The siren is eight fixed pads and gains
/// nothing from width, so it sits at its own size with first claim on
/// it; the sampler takes whatever is left. It is the one block here
/// that genuinely improves with width — a filename is the only string
/// on the bar whose length is not ours to choose — and the shelf's
/// grid, unlike the fixed pads it replaced, declares no width of its
/// own, so without a floor the siren's priority starved it to nothing.
struct GlobalRackBar: View {
    let state: GlobalRackBarState
    var callbacks = GlobalRackBarCallbacks()

    var body: some View {
        HStack(spacing: 1) {
            if let siren = state.siren {
                group(flexible: false) {
                    SirenRackGroup(
                        state: siren,
                        onPreset: callbacks.onSirenPreset,
                        onUnit: callbacks.onSirenUnit,
                        onDubMacro: callbacks.onSirenDubMacro,
                        onOutput: callbacks.onSirenOutput)
                }
                .layoutPriority(1)
            }
            group {
                SampleShelf(state: state.sampler, callbacks: callbacks.sampler)
                    .frame(minWidth: DubLayout.rackSamplerMinWidth)
            }
        }
        .frame(height: DubLayout.rackBarHeight)
        .background(DubColor.divider)
    }

    /// One block. A `flexible` block takes the bar's slack; a fixed one
    /// takes its content's width and no more.
    @ViewBuilder
    private func group<Content: View>(
        flexible: Bool = true,
        @ViewBuilder _ content: () -> Content
    ) -> some View {
        content()
            .padding(.horizontal, DubSpacing.lg)
            .padding(.vertical, DubSpacing.md)
            .frame(maxWidth: flexible ? .infinity : nil, maxHeight: .infinity, alignment: .topLeading)
            .background(DubColor.surface2)
    }
}
