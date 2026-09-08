//
//  LibraryColumnMenu.swift
//  Dub
//
//  The library header's column picker: which columns are shown, plus
//  the Camelot ↔ musical key-notation toggle.
//
//  AppKit rather than SwiftUI's `.contextMenu`, for two reasons. The
//  header is an `NSTableHeaderView` now, so a SwiftUI modifier has
//  nothing to attach to. And SwiftUI builds menu content eagerly: this
//  menu enumerates the whole `dub-library` column registry behind
//  "More columns" — 40-plus items across its groups — which the old
//  SwiftUI header rebuilt on every frame of a column-resize drag.
//
//  Pure construction, no AppKit display side effects, which is what
//  lets `LibraryColumnMenuTests` cover it without an `NSWindow` to
//  host the popup.
//

import AppKit
import DubCore

@MainActor
final class LibraryColumnMenu {
    /// Submenu holding the registry-published columns. Named so the
    /// test can find it without restating the string.
    static let moreColumnsTitle = "More columns"

    /// Fixed-set categories, in the order they appear. The deeper
    /// groups come from the registry instead (see `moreColumnsTitle`).
    private static let categories = ["Analysis", "ID3 metadata", "Library"]

    /// The live column set, including the fixed prefix. Read at build
    /// time so a checkmark reflects the state at click time rather
    /// than whenever the header was last rendered.
    var visibleColumns: [LibraryColumnField] = []

    /// Which notation the Key column is in. The toggle's label names
    /// the notation the click switches *to*, not the current one.
    var keyNotationIsCamelot = true

    var onSetVisibility: ((LibraryColumnField, Bool) -> Void)?
    var onToggleKeyNotation: (() -> Void)?

    /// `NSMenuItem` holds its target weakly. Without keeping the
    /// targets alive the closures die before the menu dispatches — a
    /// bug that presents as "the item clicks and nothing happens".
    private var anchors: [LibraryMenuActionTarget] = []

    func menu() -> NSMenu {
        let carried = anchors.count
        let menu = NSMenu()
        menu.autoenablesItems = false

        appendHeading("Fixed", to: menu)
        for field in LibraryColumnField.fixedPrefix {
            let item = NSMenuItem(title: field.headerLabel, action: nil, keyEquivalent: "")
            item.state = .on
            item.isEnabled = false
            menu.addItem(item)
        }

        for category in Self.categories {
            let fields = LibraryColumnField.configurable
                .filter { $0.pickerCategory == category }
            guard !fields.isEmpty else { continue }
            menu.addItem(.separator())
            appendHeading(category, to: menu)
            for field in fields {
                menu.addItem(toggleItem(for: field))
            }
        }

        // PRD §8.5.3.1 — the deeper groups come straight off the Rust
        // registry. Each is its own submenu because the per-source
        // group alone is six sources wide and would bury the fixed
        // categories above it.
        menu.addItem(.separator())
        menu.addItem(registryItem())

        if visibleColumns.contains(.key) {
            menu.addItem(.separator())
            let item = NSMenuItem(
                title: "Toggle Key Notation (\(keyNotationIsCamelot ? "Musical" : "Camelot"))",
                action: nil, keyEquivalent: "")
            // Present whenever the Key column is, disabled when
            // unwired — the item's presence tracks the column, not
            // whether a caller happened to hand us a closure.
            if let onToggleKeyNotation {
                attach(item, onToggleKeyNotation)
            } else {
                item.isEnabled = false
            }
            menu.addItem(item)
        }

        // Drop the previous build's anchors and keep this build's:
        // they are only needed while the menu is up, and AppKit
        // retains the in-flight menu itself. Trimming to a fixed
        // suffix the way `LibraryRowMenu` does would not work here —
        // one build of this menu is already more items than that
        // budget.
        anchors.removeFirst(carried)
        return menu
    }

    private func registryItem() -> NSMenuItem {
        let item = NSMenuItem(title: Self.moreColumnsTitle, action: nil, keyEquivalent: "")
        let submenu = NSMenu()
        submenu.autoenablesItems = false
        for group in LibraryColumnCatalog.shared.groups {
            let groupItem = NSMenuItem(title: group, action: nil, keyEquivalent: "")
            let groupMenu = NSMenu()
            groupMenu.autoenablesItems = false
            for info in LibraryColumnCatalog.shared.columns(inGroup: group) {
                groupMenu.addItem(toggleItem(for: .extra(info.id), title: info.label))
            }
            groupItem.submenu = groupMenu
            submenu.addItem(groupItem)
        }
        item.submenu = submenu
        return item
    }

    /// One show/hide checkbox. `title` overrides the field's own label
    /// for registry columns, whose label lives on the `LibraryColumnInfo`.
    private func toggleItem(for field: LibraryColumnField, title: String? = nil) -> NSMenuItem {
        let isVisible = visibleColumns.contains(field)
        let item = NSMenuItem(
            title: title ?? field.headerLabel, action: nil, keyEquivalent: "")
        item.state = isVisible ? .on : .off
        // `LibraryView.setColumnVisibility` refuses to empty the
        // table, so the last removable column has to *look* refused.
        // Left enabled, it would click and do nothing.
        guard let onSetVisibility, !(isVisible && removableCount <= 1) else {
            item.isEnabled = false
            return item
        }
        attach(item) { onSetVisibility(field, !isVisible) }
        return item
    }

    private var removableCount: Int {
        visibleColumns.filter { !$0.isFixed }.count
    }

    /// A disabled title row. `NSMenuItem.sectionHeader(title:)` is
    /// macOS 14 and the app targets 13.
    private func appendHeading(_ title: String, to menu: NSMenu) {
        let item = NSMenuItem(title: title, action: nil, keyEquivalent: "")
        item.isEnabled = false
        menu.addItem(item)
    }

    private func attach(_ item: NSMenuItem, _ work: @escaping () -> Void) {
        let target = LibraryMenuActionTarget(work)
        anchors.append(target)
        item.target = target
        item.action = #selector(LibraryMenuActionTarget.dubMenuPerform(_:))
        item.isEnabled = true
    }
}
