//
//  LibrarySidebarDivider.swift
//  Dub
//
//  The draggable boundary between the library's source tree and the
//  track list.
//
//  The deck / library boundary lost its drag handle because the app
//  knows the right answer per mode (`DeckLibrarySplit`). This one has
//  no such answer: the width a crate column wants is the length of the
//  names in it — a nested rekordbox tree needs room the five stock rows
//  do not — and that varies per library, not per mode. So it is the
//  DJ's to place, as it is in Serato, Traktor and the Finder.
//
//  Drawn as the 1 pt line it always was, with a wider invisible grab
//  zone laid over both neighbours. Not `NSSplitView`, for the reasons
//  in `DeckLibrarySplit`'s history: two `NSHostingView`s would cut
//  environment and observation propagation into the panes, to buy a
//  divider style we already draw.
//
//  Dragging a pane boundary is chrome, not a performance gesture — PRD
//  §1 guards pitch, crossfade, EQ, gain and cueing.
//

import AppKit
import SwiftUI

struct LibrarySidebarDivider: View {
    /// The sidebar's requested width, moved at display rate during a
    /// drag. `LibrarySplit`'s `@State`, so the only body a frame of
    /// the drag re-runs is the split's own.
    @Binding var width: CGFloat
    /// `true` from the first movement to release. The split freezes
    /// the track list's layout for the duration — see its file comment.
    @Binding var dragging: Bool
    /// The width the whole library has, for the clamp.
    let total: CGFloat
    /// Called once on release with the settled width — persist here.
    let onCommit: (CGFloat) -> Void

    @State private var dragStart: CGFloat?
    @State private var hovering = false
    @State private var cursorPushed = false

    var body: some View {
        Rectangle()
            .fill(DubColor.divider)
            .frame(width: 1)
            .overlay {
                Color.clear
                    .frame(width: DubLayout.splitterThickness)
                    .contentShape(Rectangle())
                    .onHover { inside in
                        hovering = inside
                        updateCursor()
                    }
                    .gesture(drag)
                    .onTapGesture(count: 2) {
                        width = DubLayout.librarySidebarDefaultWidth
                        onCommit(width)
                    }
                    .help("Drag to resize · double-click to reset")
            }
            // Above both neighbours, so the half of the grab zone that
            // lies over the track list is not the list's.
            .zIndex(1)
    }

    /// Global space: the handle moves with the pointer, and a
    /// translation measured in its own coordinates would be measured
    /// against a moving origin.
    private var drag: some Gesture {
        DragGesture(minimumDistance: 1, coordinateSpace: .global)
            .onChanged { value in
                let start = dragStart ?? width
                if dragStart == nil { dragging = true }
                dragStart = start
                width = LibrarySidebarDivider.width(
                    wanted: start + value.translation.width, total: total)
                updateCursor()
            }
            .onEnded { _ in
                dragStart = nil
                dragging = false
                updateCursor()
                onCommit(width)
            }
    }

    /// Holds the resize cursor for the whole drag. The zone is 6 pt
    /// wide and the pointer leaves it the moment the width clamps, so
    /// hover alone would flick the cursor back to an arrow mid-drag.
    private func updateCursor() {
        let wanted = hovering || dragStart != nil
        guard wanted != cursorPushed else { return }
        if wanted { NSCursor.resizeLeftRight.push() } else { NSCursor.pop() }
        cursorPushed = wanted
    }

    // MARK: - Metrics

    /// The width the sidebar gets for a requested one inside `total`.
    ///
    /// Bounded by its tokens, and by the track list keeping its floor.
    /// When even that cannot be met the sidebar's own floor wins — a
    /// case no supported window reaches, since the ceiling fits the
    /// minimum window with the list's floor to spare.
    ///
    /// Whole points, as `NSSplitView` moves: a pointer lands on
    /// fractions, and a pane edge on one is a soft seam on Retina.
    static func width(wanted: CGFloat, total: CGFloat) -> CGFloat {
        guard total.isFinite else { return DubLayout.librarySidebarDefaultWidth }
        guard total > 0 else { return 0 }
        let floor = min(DubLayout.librarySidebarMinWidth, total)
        let ceiling = max(
            floor,
            min(DubLayout.librarySidebarMaxWidth,
                total - 1 - DubLayout.libraryTrackPaneMinWidth))
        let want = (wanted.isFinite ? wanted : DubLayout.librarySidebarDefaultWidth).rounded()
        return min(max(want, floor), ceiling)
    }

    // MARK: - Persistence

    static let defaultsKey = "librarySidebarWidth"

    /// The width the sidebar opens at: where the DJ last released it,
    /// or the default until they have. Not clamped here — the tokens
    /// may move between launches, and the render clamps anyway.
    static func storedWidth(from defaults: UserDefaults = .standard) -> CGFloat {
        guard let raw = defaults.object(forKey: defaultsKey) as? Double,
              raw.isFinite, raw > 0
        else { return DubLayout.librarySidebarDefaultWidth }
        return CGFloat(raw)
    }

    static func store(_ width: CGFloat, to defaults: UserDefaults = .standard) {
        guard width.isFinite, width > 0 else { return }
        defaults.set(Double(width), forKey: defaultsKey)
    }
}

