//
//  DeckLibrarySplit.swift
//  Dub
//
//  The boundary between the deck cluster and the library. **Not
//  draggable.**
//
//  It was, and the handle is gone. A pane boundary the DJ has to place
//  is a setting disguised as a gesture: it has a correct answer per mode,
//  the app knows what that answer is, and every session started with the
//  library either starved or eating the waveform until someone dragged it
//  back. Removing it also removes a persisted per-mode fraction, a drag
//  gesture on the app's hottest surface, and a class of "why is my
//  waveform tiny today".
//
//  The two modes want different things, so they get different rules:
//
//  * **Prep** sizes the deck to exactly what it draws — overview, playing
//    strip and the `PrepRack` — and gives everything below to the
//    library. Prep is where you read a track list, so the list should
//    have the room the controls do not need.
//  * **Performance** does the same since 2026-11-14: the deck region is
//    as tall as the deck columns (source, readouts, overview, cues,
//    trigger row) and no taller, the rack bar sits straight under it, and
//    the library has the rest. It used to hold 68 % for the decks, which
//    left the strip running a third of its height past the columns.
//

import AppKit
import SwiftUI

/// Splits its container between a deck side and a library side.
struct DeckLibrarySplit<Deck: View, Library: View>: View {
    let mode: EngineMode
    /// Fixed chrome that rides with the deck side — the rack bar and
    /// its divider in Performance, nothing in Prep.
    let deckChrome: CGFloat
    /// The chrome `performanceDeckFraction` was tuned against — the rack
    /// bar open. When the actual `deckChrome` is smaller (the bar
    /// folded) the difference is handed to the library rather than to
    /// the waveform, which is the point of folding it. Defaults to
    /// `deckChrome`, i.e. no difference.
    var deckChromeBudget: CGFloat? = nil
    /// Floor for the deck side's *content*, excluding `deckChrome`.
    let deckMinimum: CGFloat
    /// Called with the height available to the deck content.
    @ViewBuilder let deck: (_ contentHeight: CGFloat) -> Deck
    @ViewBuilder let library: () -> Library

    var body: some View {
        GeometryReader { geo in
            let total = max(0, geo.size.height - 1)
            let deckHeight = Self.deckHeight(
                mode: mode, total: total,
                deckChrome: deckChrome, deckChromeBudget: deckChromeBudget ?? deckChrome,
                deckMinimum: deckMinimum)
            VStack(spacing: 0) {
                deck(max(0, deckHeight - deckChrome))
                    .frame(height: deckHeight)
                Rectangle()
                    .fill(DubColor.divider)
                    .frame(height: 1)
                library()
                    .frame(height: max(0, total - deckHeight))
            }
        }
    }

    /// The deck is pinned to its content and the rest handed over, in
    /// both modes.
    ///
    /// Both are clamped so the library keeps `libraryMinHeight` — on a
    /// short window the deck gives way first, which is the same
    /// degradation order the drag enforced.
    static func deckHeight(
        mode: EngineMode,
        total: CGFloat,
        deckChrome: CGFloat,
        deckChromeBudget: CGFloat? = nil,
        deckMinimum: CGFloat
    ) -> CGFloat {
        // Both modes size the deck to what it draws and hand the rest to
        // the library. Performance used to keep a fixed 68 % for the
        // decks; with the pads beside the strip that left a third of the
        // region empty under the columns while the library was starved
        // (2026-11-14). `deckMinimum` there is the columns' own height.
        _ = (mode, deckChromeBudget)
        let wanted = deckMinimum + deckChrome
        let ceiling = max(0, total - DubLayout.libraryMinHeight)
        return max(0, min(wanted, ceiling))
    }
}
