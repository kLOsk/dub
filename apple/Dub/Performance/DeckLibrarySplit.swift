//
//  DeckLibrarySplit.swift
//  Dub
//
//  The draggable boundary between the deck cluster and the library.
//
//  Not `NSSplitView`: that needs an `NSViewRepresentable` wrapping two
//  `NSHostingView`s, which breaks environment and observation
//  propagation into the library and the deck panes and puts a second
//  layout system on the app's hottest surface — to buy a divider style
//  we already draw by hand.
//
//  Dragging a pane boundary is chrome, not a performance gesture. PRD
//  §1 forbids *continuous performance gestures* (pitch, crossfade, EQ,
//  gain, cueing); AGENTS.md is explicit that the mouse is fine for
//  everything else. This is not a software pitch fader.
//

import AppKit
import SwiftUI

/// Splits its container between a deck side and a library side, with a
/// draggable handle between them.
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

    @State private var fraction: CGFloat
    @State private var dragStart: CGFloat?

    init(
        mode: EngineMode,
        deckChrome: CGFloat,
        deckMinimum: CGFloat,
        @ViewBuilder deck: @escaping (_ contentHeight: CGFloat) -> Deck,
        @ViewBuilder library: @escaping () -> Library
    ) {
        self.mode = mode
        self.deckChrome = deckChrome
        self.deckMinimum = deckMinimum
        self.deck = deck
        self.library = library
        _fraction = State(initialValue: SplitMetrics.load(mode))
    }

    var body: some View {
        GeometryReader { geo in
            let total = max(0, geo.size.height - DubLayout.splitterThickness)
            let deckHeight = SplitMetrics.deckHeight(
                fraction: fraction, total: total,
                deckChrome: deckChrome, deckMinimum: deckMinimum)
            VStack(spacing: 0) {
                deck(max(0, deckHeight - deckChrome))
                    .frame(height: deckHeight)
                handle(total: total)
                library()
                    .frame(height: max(0, total - deckHeight))
            }
        }
        // The two surfaces remember independently, so switching modes
        // restores that mode's boundary rather than carrying one over.
        .onChange(of: mode) { newMode in
            fraction = SplitMetrics.load(newMode)
        }
    }

    private func handle(total: CGFloat) -> some View {
        ZStack {
            Rectangle().fill(DubColor.surface2)
            Rectangle()
                .fill(DubColor.textPlaceholder.opacity(0.5))
                .frame(width: 24, height: 2)
                .clipShape(Capsule())
        }
        .frame(height: DubLayout.splitterThickness)
        .overlay(alignment: .top) {
            Rectangle().fill(DubColor.divider).frame(height: 1)
        }
        .contentShape(Rectangle())
        .onHover { inside in
            if inside { NSCursor.resizeUpDown.push() } else { NSCursor.pop() }
        }
        .gesture(
            DragGesture(minimumDistance: 1)
                .onChanged { value in
                    let start = dragStart ?? fraction
                    dragStart = start
                    let target = SplitMetrics.deckHeight(
                        fraction: start + value.translation.height / max(total, 1),
                        total: total, deckChrome: deckChrome,
                        deckMinimum: deckMinimum)
                    fraction = SplitMetrics.fraction(deckHeight: target, total: total)
                }
                // One write per drag, not one per frame: the fraction
                // is `@State` so a drag never republishes the app model
                // (which would invalidate every deck pane and the
                // Stillpoint view at 60 Hz, on the surface that must
                // not drop frames), and never touches `@AppStorage`
                // (which writes defaults on every assignment).
                .onEnded { _ in
                    dragStart = nil
                    SplitMetrics.save(fraction, mode)
                })
        .onTapGesture(count: 2) {
            fraction = SplitMetrics.defaultFraction(mode)
            SplitMetrics.save(fraction, mode)
        }
        .help("Drag to resize · double-click to reset")
    }
}
