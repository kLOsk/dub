//
//  LibraryRowIndicators.swift
//  Dub
//
//  A library row's indicator gutter, plus the value types a row is
//  drawn from.
//
//  Interaction stays with the caller. Click, drag-out and the
//  right-click menu all need the live selection, the drag pasteboard
//  and the AppKit table, none of which a reusable cell may reach for —
//  they belong to `LibraryTable` and its delegate, not to a cell view.
//
//  Everything the old SwiftUI row consulted (`model.deckA/deckB`,
//  `libraryModel.libraryIsOpen`, `model.isTrackReachable`) is resolved
//  into `LibraryRowState` by the caller, because a reused cell is
//  configured with values and must not consult a live object
//  afterwards.
//
//  The row itself is assembled by `LibraryTable` from
//  `LibraryHostingCellView`s inside a `LibraryTintedRowView`; there is
//  no SwiftUI view for a whole row. `LibraryRowSnapshotTests` pins
//  that real assembly.
//

import DubCore
import SwiftUI

/// Everything a row draws, as values.
struct LibraryRowState: Equatable {
    var cell: LibraryCellState
    /// Loaded on deck A / B. Both can be true — Instant Doubles.
    var isOnDeckA: Bool = false
    var isOnDeckB: Bool = false
    /// The source volume is known to be offline. Deliberately not
    /// derived here: reachability is optimistic (an unprobed volume
    /// reads as reachable), and a pessimistic default flashed the red
    /// triangle on every row for a runloop tick.
    var showsUnreachableWarning: Bool = false
    var unreachableTooltip: String = ""

    var track: LibraryTrack { cell.track }
}

struct LibraryRowActions {
    var cell = LibraryCellActions()
    /// Jump to the sibling this row is a potential duplicate of.
    var onDuplicateJump: () -> Void = {}
}

/// The 36 pt gutter: deck badges, the duplicate link, the offline-file
/// warning. Ordered by how much the DJ needs to see it.
struct LibraryRowIndicators: View {
    let state: LibraryRowState
    var onDuplicateJump: () -> Void = {}

    var body: some View {
        HStack(spacing: 2) {
            if state.isOnDeckA { badge("A", tint: DubColor.deckATint) }
            if state.isOnDeckB { badge("B", tint: DubColor.deckBTint) }
            if state.track.potentialDuplicateId != nil {
                Button(action: onDuplicateJump) {
                    Image(systemName: "link")
                        .font(.system(size: 10, weight: .medium))
                        .foregroundStyle(DubColor.textSecondary)
                }
                .buttonStyle(.plain)
                .help("Potential duplicate — click to jump to sibling.")
            }
            if state.showsUnreachableWarning {
                Image(systemName: "exclamationmark.triangle.fill")
                    .font(.system(size: 10, weight: .medium))
                    .foregroundStyle(.red.opacity(0.65))
                    .help(state.unreachableTooltip)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func badge(_ letter: String, tint: Color) -> some View {
        Text(letter)
            .font(.system(size: 9, weight: .bold, design: .rounded))
            .foregroundStyle(.white)
            .frame(width: 13, height: 13)
            .background(tint)
            .clipShape(RoundedRectangle(cornerRadius: 2))
            .help("Loaded on deck \(letter).")
    }
}
