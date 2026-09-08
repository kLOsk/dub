//
//  LibraryRowView.swift
//  Dub
//
//  One library table row's *content*, rendered from a value snapshot.
//
//  Interaction stays with the caller. Click, drag-out and the
//  right-click menu all need the live selection, the drag pasteboard
//  and the AppKit table, none of which a reusable cell may reach for —
//  and in the `NSTableView` migration they belong to the table and its
//  delegate, not to a cell view.
//
//  What lives here is exactly what draws: the indicator gutter, the
//  cells, the colour-label tint. Everything else the old `trackRow`
//  consulted (`model.deckA/deckB`, `libraryModel.libraryIsOpen`,
//  `model.isTrackReachable`) is resolved into `LibraryRowState` by the
//  caller, because a reused cell is configured with values and must
//  not consult a live object afterwards.
//
//  Two details that look incidental and are not:
//
//  * `DimUnanalyzed` is applied **per cell, inside the width frame** —
//    so an unanalyzed row dims its cell text but *not* the gutter
//    badges and *not* the colour tint. Moving it to the row would
//    visibly change those rows.
//  * The row tint is deliberately faint. Selection is painted by an
//    AppKit layer *beneath* the row, so a heavier tint would make a
//    selected coloured row unreadable.
//

import DubCore
import SwiftUI

/// One column's identity and the width it is drawn at. An ordered
/// array rather than a dictionary because the row draws them in order —
/// and because after the migration the widths come from the table's
/// columns, not from the row.
struct LibraryRowColumn: Equatable {
    let field: LibraryColumnField
    let width: CGFloat
}

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

struct LibraryRowView: View {
    let state: LibraryRowState
    let columns: [LibraryRowColumn]
    /// Gutter + columns + horizontal padding. Passed in because the
    /// caller already computes it for the header.
    let totalWidth: CGFloat
    var actions = LibraryRowActions()

    var body: some View {
        HStack(spacing: 0) {
            LibraryRowIndicators(state: state, onDuplicateJump: actions.onDuplicateJump)
                .frame(width: LibraryColumnLayout.gutterWidth, alignment: .leading)
            ForEach(columns, id: \.field) { column in
                LibraryColumnCell(
                    field: column.field, state: state.cell, actions: actions.cell)
                    .padding(.leading, LibraryColumnLayout.columnLeadingInset)
                    .modifier(DimUnanalyzed(isAnalyzed: state.track.isAnalyzed))
                    .frame(width: column.width, alignment: .leading)
            }
        }
        .padding(.horizontal, DubSpacing.lg)
        .frame(
            width: totalWidth,
            height: LibraryRowLayout.estimatedHeight,
            alignment: .leading)
        .background(DubColor.trackLabel(state.track.color).map { $0.opacity(0.18) }
            ?? Color.clear)
    }
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
