//
//  LibraryRowMenu.swift
//  Dub
//
//  The library row's right-click menu.
//
//  Built in AppKit rather than with SwiftUI's `.contextMenu` because
//  the labels have to be correct at *click* time: "Re-analyze Selected
//  (3 of 5)" counts the live selection, and a SwiftUI modifier captures
//  its body when the row first attaches. That reason predates the
//  `NSTableView` migration and outlives it.
//
//  Lifted out of the old scroll container's coordinator so the new
//  table can use it unchanged. Pure construction — no AppKit display
//  side effects — which is what lets `LibraryRowMenuTests` cover it
//  without an `NSWindow` to host the popup.
//

import AppKit
import DubCore

@MainActor
final class LibraryRowMenu {
    /// The visible listing, post-search and post-sort.
    var tracks: [LibraryTrack] = []
    /// Read live at click time, so the multi-select label stays correct
    /// even when selection changed without a SwiftUI re-render.
    var selectedTrackIds: Set<String> = []
    var analysisBatchInProgress = false
    var onAnalyzeRequested: (([String]) -> Void)?
    var onSetGridLocked: ((String, Bool) -> Void)?
    /// Crate context (M11d-next): non-nil when the visible listing is a
    /// Dub crate, enabling "Remove from Crate" and "Move…".
    var crateId: Int64?
    var onCrateRemove: (([String]) -> Void)?
    var onCrateMove: ((String, CrateMove) -> Void)?

    /// `NSMenuItem` holds its target weakly. Without keeping the
    /// targets alive the closures die before the menu dispatches — a
    /// bug that presents as "the item clicks and nothing happens".
    private var anchors: [LibraryMenuActionTarget] = []

    func menu(for track: LibraryTrack) -> NSMenu {
        let menu = NSMenu()
        menu.autoenablesItems = false

        let targets = analyzeTargets(rightClickedTrack: track)
        let unlocked = targets.filter { !isLocked($0) }
        let analyzeItem = NSMenuItem(
            title: analyzeMenuLabel(
                rightClickedTrack: track,
                allCount: targets.count,
                unlockedCount: unlocked.count),
            action: nil, keyEquivalent: "")
        if !analysisBatchInProgress, !unlocked.isEmpty, let onAnalyzeRequested {
            attach(analyzeItem) { onAnalyzeRequested(unlocked) }
        } else {
            analyzeItem.isEnabled = false
        }
        menu.addItem(analyzeItem)

        menu.addItem(.separator())
        let lockItem = NSMenuItem(
            title: track.gridLocked ? "Unlock grid" : "Lock grid",
            action: nil, keyEquivalent: "")
        if let onSetGridLocked {
            attach(lockItem) { onSetGridLocked(track.id, !track.gridLocked) }
        } else {
            lockItem.isEnabled = false
        }
        menu.addItem(lockItem)

        // R-49 — sample lineage (PRD §5.2.5a). A link-out, so it needs
        // no key and no network of ours; disabled only when the row
        // carries neither artist nor title.
        menu.addItem(.separator())
        let samplesItem = NSMenuItem(
            title: SampleLineage.actionTitle, action: nil, keyEquivalent: "")
        if SampleLineage.whoSampledSearchURL(artist: track.artist, title: track.title) != nil {
            attach(samplesItem) {
                SampleLineage.lookUp(artist: track.artist, title: track.title)
            }
        } else {
            samplesItem.isEnabled = false
        }
        menu.addItem(samplesItem)

        if crateId != nil {
            menu.addItem(.separator())
            appendCrateItems(to: menu, rightClickedTrack: track)
        }

        // Drop the previous run's anchors once a new menu is built —
        // they are only needed while the menu is visible, and AppKit
        // retains the in-flight menu itself. Without this the list
        // would grow unbounded over a long session.
        anchors = Array(anchors.suffix(16))
        return menu
    }

    /// "Remove from Crate" + the "Move…" block. Move items are disabled
    /// at the list edges, using the right-clicked row's index in the
    /// visible (ordinal) order.
    private func appendCrateItems(to menu: NSMenu, rightClickedTrack track: LibraryTrack) {
        let removeTargets = analyzeTargets(rightClickedTrack: track)
        let removeItem = NSMenuItem(
            title: removeTargets.count > 1
                ? "Remove from Crate (\(removeTargets.count))"
                : "Remove from Crate",
            action: nil, keyEquivalent: "")
        if let onCrateRemove {
            attach(removeItem) { onCrateRemove(removeTargets) }
        } else {
            removeItem.isEnabled = false
        }
        menu.addItem(removeItem)

        guard let onCrateMove else { return }
        let index = tracks.firstIndex(where: { $0.id == track.id })
        let count = tracks.count
        menu.addItem(.separator())
        let moves: [(String, CrateMove, Bool)] = [
            ("Move Up", .up, (index ?? 0) > 0),
            ("Move Down", .down, (index ?? count) < count - 1),
            ("Move to Top", .top, (index ?? 0) > 0),
            ("Move to Bottom", .bottom, (index ?? count) < count - 1),
        ]
        for (title, move, enabled) in moves {
            let item = NSMenuItem(title: title, action: nil, keyEquivalent: "")
            if enabled {
                attach(item) { onCrateMove(track.id, move) }
            } else {
                item.isEnabled = false
            }
            menu.addItem(item)
        }
    }

    /// Multi-selection acts on the full selection when the
    /// right-clicked row IS part of it; otherwise the menu acts only on
    /// the right-clicked row (Finder semantics).
    func analyzeTargets(rightClickedTrack track: LibraryTrack) -> [String] {
        if selectedTrackIds.contains(track.id), selectedTrackIds.count > 1 {
            return Array(selectedTrackIds)
        }
        return [track.id]
    }

    private func isLocked(_ trackId: String) -> Bool {
        tracks.first(where: { $0.id == trackId })?.gridLocked ?? false
    }

    /// PRD-BEATS §4.4 label. Reflects mixed-lock selections as
    /// "Re-analyze Selected (3 of 5)" so the user can see how many rows
    /// the analyse pass will skip before clicking.
    func analyzeMenuLabel(
        rightClickedTrack track: LibraryTrack,
        allCount: Int,
        unlockedCount: Int
    ) -> String {
        let verb = track.isAnalyzed ? "Re-analyze" : "Analyze"
        guard allCount > 1 else { return verb }
        if unlockedCount < allCount {
            return "\(verb) Selected (\(unlockedCount) of \(allCount))"
        }
        return "\(verb) Selected (\(allCount))"
    }

    private func attach(_ item: NSMenuItem, _ work: @escaping () -> Void) {
        let target = LibraryMenuActionTarget(work)
        anchors.append(target)
        item.target = target
        item.action = #selector(LibraryMenuActionTarget.dubMenuPerform(_:))
        item.isEnabled = true
    }
}
