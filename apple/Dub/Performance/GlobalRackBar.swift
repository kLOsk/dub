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
//  The bar folds (2026-09-14) to a one-line strip, and the height it
//  gives up goes to the library — `DeckLibrarySplit` takes the open
//  bar as its chrome budget, so the waveform region does not move.
//  Folding is a deliberate "I am browsing now", the same class of
//  gesture as the library's `› FILTER`, and it is remembered.
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
/// **Where the slack goes.** The siren is a fixed-width box and gains
/// nothing from width, so it sits at its own size with first claim on
/// it; the sampler takes whatever is left. It is the one block here
/// that genuinely improves with width — a filename is the only string
/// on the bar whose length is not ours to choose — and the shelf's
/// grid, unlike the fixed pads it replaced, declares no width of its
/// own, so without a floor the siren's priority starved it to nothing.
struct GlobalRackBar: View {
    let state: GlobalRackBarState
    var callbacks = GlobalRackBarCallbacks()
    /// Folded to its one-line strip: the blocks are gone and the library
    /// has their height. The strip keeps the blocks' names so the rack
    /// can be found and opened again.
    var folded: Bool = false
    /// The chevron — on the strip, or in the column at the open bar's
    /// leading edge. The same `› FILTER` fold the library uses.
    var onFold: () -> Void = {}

    var body: some View {
        Group {
            if folded {
                foldedStrip
            } else {
                openBar
            }
        }
        .frame(height: folded ? DubLayout.rackBarFoldedHeight : DubLayout.rackBarHeight)
        .background(DubColor.divider)
    }

    private var openBar: some View {
        HStack(spacing: 1) {
            foldColumn
            // The siren sits under the rack when a deck is the DUB FX
            // channel: on the right for deck B, so the two swap.
            if state.fxSide == .b {
                samplerGroup
                sirenGroup
            } else {
                sirenGroup
                samplerGroup
            }
        }
    }

    @ViewBuilder
    private var sirenGroup: some View {
        if let siren = state.siren {
            group(flexible: false) {
                SirenRackGroup(
                    state: siren,
                    onPreset: callbacks.onSirenPreset,
                    onDubMacro: callbacks.onSirenDubMacro,
                    onOutput: callbacks.onSirenOutput)
            }
            .layoutPriority(1)
        }
    }

    private var samplerGroup: some View {
        group {
            SampleShelf(state: state.sampler, callbacks: callbacks.sampler)
                .frame(minWidth: DubLayout.rackSamplerMinWidth)
        }
    }

    /// A slim column with the chevron at the height of the headings, so
    /// it reads as `▾ DUB SIREN` beside the first block.
    private var foldColumn: some View {
        Button(action: onFold) {
            VStack {
                Image(systemName: "chevron.down")
                    .font(.system(size: 9, weight: .semibold))
                    .foregroundStyle(DubColor.textSecondary)
                    .frame(height: 14)
                Spacer()
            }
            .padding(.top, DubSpacing.md)
            .frame(width: DubLayout.rackBarFoldColumnWidth)
            .frame(maxHeight: .infinity)
            .background(DubColor.surface2)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help("Fold the rack away — the library takes its space")
        .accessibilityLabel("Fold the rack")
    }

    /// The whole strip is the button: chevron and the names of what is
    /// folded, in their own tints.
    private var foldedStrip: some View {
        Button(action: onFold) {
            HStack(spacing: DubSpacing.sm) {
                Image(systemName: "chevron.right")
                    .font(.system(size: 9, weight: .semibold))
                    .foregroundStyle(DubColor.textSecondary)
                if state.siren != nil {
                    stripTitle("DUB SIREN", DubColor.siren)
                    Text("·")
                        .font(DubFont.micro)
                        .foregroundStyle(DubColor.textPlaceholder)
                }
                stripTitle("SAMPLES", samplerTint)
                Spacer()
            }
            .padding(.horizontal, DubSpacing.lg)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(DubColor.surface2)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help("Show the rack — it takes its space back from the library")
        .accessibilityLabel("Show the rack")
    }

    /// The shelf's own heading tint — the deck it fires on.
    private var samplerTint: Color {
        state.sampler.output?.tintDeck.map(DubColor.deckTint) ?? DubColor.controlAccent
    }

    private func stripTitle(_ title: String, _ tint: Color) -> some View {
        Text(title)
            .font(DubFont.caps)
            .tracking(DubFont.capsTracking)
            .foregroundStyle(tint)
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
