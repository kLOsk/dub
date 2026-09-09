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
//  * **Performance** keeps the decks dominant (PRD §9.2: "the decks
//    dominate vertical real estate intentionally"), at the proportion the
//    drag used to default to. The library is subordinate there by design.
//

import AppKit
import SwiftUI

/// Splits its container between a deck side and a library side.
struct DeckLibrarySplit<Deck: View, Library: View>: View {
    let mode: EngineMode
    /// Fixed chrome that rides with the deck side — the rack bar and
    /// its divider in Performance, nothing in Prep.
    let deckChrome: CGFloat
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
                deckChrome: deckChrome, deckMinimum: deckMinimum)
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

    /// Prep pins the deck to its content and hands the rest over;
    /// Performance keeps the decks dominant.
    ///
    /// Both are clamped so the library keeps `libraryMinHeight` — on a
    /// short window the deck gives way first, which is the same
    /// degradation order the drag enforced.
    static func deckHeight(
        mode: EngineMode,
        total: CGFloat,
        deckChrome: CGFloat,
        deckMinimum: CGFloat
    ) -> CGFloat {
        let wanted: CGFloat
        switch mode {
        case .prep:
            wanted = deckMinimum + deckChrome
        case .timecode:
            wanted = max(deckMinimum + deckChrome, total * DubLayout.performanceDeckFraction)
        }
        let ceiling = max(0, total - DubLayout.libraryMinHeight)
        return max(0, min(wanted, ceiling))
    }
}
