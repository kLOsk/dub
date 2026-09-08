//
//  LibraryColorCell.swift
//  Dub
//
//  The colour-label swatch, and its palette menu.
//
//  The swatch box is rendered as **normal cell content**: a `Menu` with
//  `.borderlessButton` style drops `Shape` label content — only symbols
//  and text survive — which is why the box and border were invisible
//  when it was the menu's label. The click target sits on top as a
//  clear overlay so the swatch renders reliably underneath.
//
//  The palette is built when the menu opens, not when the cell is
//  built. As a SwiftUI `Menu` its ~10 items were constructed for every
//  row on every table rebuild.
//

import SwiftUI

struct LibraryColorCell: View {
    /// Palette token, or `nil` for no label.
    let color: String?
    let onPick: (_ token: String?) -> Void

    var body: some View {
        let swatch = DubColor.trackLabel(color)
        return RoundedRectangle(cornerRadius: 3)
            .fill(swatch ?? DubColor.surface3)
            .frame(width: 15, height: 15)
            .overlay(
                RoundedRectangle(cornerRadius: 3)
                    .strokeBorder(
                        swatch != nil ? Color.white.opacity(0.75) : DubColor.textTertiary,
                        lineWidth: 1))
            .overlay {
                if swatch == nil {
                    Image(systemName: "plus")
                        .font(.system(size: 8, weight: .semibold))
                        .foregroundStyle(DubColor.textTertiary)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(Rectangle())
            .overlay { LazyMenuClickTarget { menuEntries } }
    }

    /// The palette as menu data rather than views — evaluated on click.
    private var menuEntries: [LazyMenuEntry] {
        var entries: [LazyMenuEntry] = DubColor.trackLabelPalette.map { entry in
            .item(
                title: entry.token.capitalized,
                symbol: "square.fill",
                tint: NSColor(entry.color)
            ) { onPick(entry.token) }
        }
        entries.append(.separator)
        entries.append(
            .item(title: "None", symbol: "slash.circle", tint: nil) { onPick(nil) })
        return entries
    }
}
