//
//  LibrarySplit.swift
//  Dub
//
//  Splits the library between its source tree and the track list, at
//  the boundary the DJ drags (`LibrarySidebarDivider`).
//
//  Why a container, and not a `.frame(width:)` in `LibraryView.body`:
//  the width moves at display rate during a drag, and whichever view
//  reads it re-runs its body per frame. `LibraryView.body` is the most
//  expensive body in the app — it builds the column specs, the
//  favourites strip and the table's inputs, and the table's update
//  then maps 5 000 ids to find out nothing changed — and paying that
//  per pointer event left the divider a step behind the mouse where
//  the table's own column resize, pure AppKit layout, is not.
//
//  Here the width is this view's `@State`, and the two panes arrive as
//  values the owner built once rather than as `@ViewBuilder` closures
//  it would rebuild. Re-emitting an unchanged value is a comparison,
//  not a rebuild.
//
//  That left a frame costing ~50 ms, and none of it in our code: it
//  was AppKit relaying out the track list — `NSScrollView` → clip view
//  → `NSTableView` → every row view → some 225 `NSHostingView` cells —
//  plus the window's constraint pass and the header redraw, once per
//  display frame. A column resize only re-tiles the cells right of one
//  column, which is why it is smooth and this was not. So while the
//  divider is held, the track list keeps the width it had at mouse-down
//  and only moves with the divider; the one real relayout happens on
//  release. Dragging right, its far edge runs off the window; dragging
//  left, the library's background shows at the far edge until the
//  mouse comes up — the same thing this table shows when its last
//  column is narrowed. The sidebar still reflows live: it is the thing
//  being resized, and it is forty rows of SwiftUI.
//

import SwiftUI

struct LibrarySplit<Sidebar: View, Content: View>: View {
    let sidebar: Sidebar
    let content: Content

    /// Read from defaults once per mount; the divider writes it back
    /// once per release. Never `@AppStorage`, which would write on
    /// every frame of the drag.
    @State private var width: CGFloat = LibrarySidebarDivider.storedWidth()

    /// The track list's width for the duration of a drag — what it
    /// had at mouse-down. `nil` when the divider is not held.
    @State private var frozenContentWidth: CGFloat?

    var body: some View {
        GeometryReader { geo in
            let total = geo.size.width
            let sidebarWidth = LibrarySidebarDivider.width(wanted: width, total: total)
            let liveContentWidth = max(0, total - 1 - sidebarWidth)
            HStack(spacing: 0) {
                sidebar
                    .frame(width: sidebarWidth)
                    .background(DubColor.surface1)
                LibrarySidebarDivider(
                    width: $width,
                    dragging: Binding(
                        get: { frozenContentWidth != nil },
                        set: { held in frozenContentWidth = held ? liveContentWidth : nil }),
                    total: total
                ) {
                    LibrarySidebarDivider.store($0)
                }
                content
                    .frame(width: frozenContentWidth ?? liveContentWidth)
            }
            // Leading, so a frozen list overflows off the window's far
            // edge rather than being centred in the space it exceeds.
            .frame(width: total, alignment: .leading)
        }
    }
}
