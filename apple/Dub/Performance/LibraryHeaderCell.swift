//
//  LibraryHeaderCell.swift
//  Dub
//
//  One column header's content: the caps label and its sort chevron.
//
//  Value-driven for the same reason the row cells are — the
//  `NSTableView` migration keeps the look by hosting this code in the
//  header view, and it can only do that if the label is a function of
//  its inputs rather than of `LibraryView`'s state.
//
//  Note the inset asymmetry, which is deliberate and easy to lose:
//  header text is inset `columnLeadingInset` (8) **and**
//  `columnTrailingInset` (14), so its usable width is `w − 22`, while a
//  row cell gets the leading 8 only and has `w − 8`. Header labels
//  therefore truncate 14 pt earlier than the cell text beneath them.
//

import SwiftUI

/// Sort state for one header, as values.
struct LibraryHeaderState: Equatable {
    let title: String
    /// This column is the active sort.
    var isActive: Bool = false
    /// Only meaningful when `isActive`.
    var ascending: Bool = true
}

/// The label and chevron. The column's width, the reorder affordance
/// and the resize handle are the caller's business — after the
/// migration they belong to `NSTableColumn` and the header view.
struct LibraryHeaderCell: View {
    let state: LibraryHeaderState
    var onToggleSort: () -> Void = {}

    var body: some View {
        Button(action: onToggleSort) {
            HStack(spacing: 3) {
                Text(state.title.uppercased())
                    .font(DubFont.micro.weight(.semibold))
                    .foregroundStyle(
                        state.isActive ? DubColor.textPrimary : DubColor.textSecondary)
                if state.isActive {
                    Image(systemName: state.ascending ? "chevron.up" : "chevron.down")
                        .font(.system(size: 8, weight: .bold))
                        .foregroundStyle(DubColor.textSecondary)
                }
                Spacer(minLength: 0)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}
