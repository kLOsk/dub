//
//  RipSplitMarkerOverlay.swift
//  Dub
//
//  M26a — split-marker editing over the Prep-mode Track Overview
//  band during rip review. ZStacked on top of `TrackOverviewView`
//  and sharing its `OverviewLayout.endPadding` fraction grid, so a
//  marker sits exactly on the band position its seconds map to.
//
//  Interactions (all review-phase only):
//    • drag a marker (±6 pt hit radius) → move; local echo while
//      dragging, `moveSplit` throttled to ~30 Hz, final call on end
//    • double-click empty band → add a split (brief red flash when
//      the FFI rejects the boundary)
//    • single click empty band → seek (keeps the overview's
//      click-to-jump audition workflow alive under the overlay)
//    • click a marker → select; ← / → nudge 0.1 s (⇧ = 1 s),
//      ⌫ removes — via a local NSEvent monitor, the same approach
//      as MainView's key handling (SwiftUI focus is unreliable here)
//    • double-click a marker → audition across the boundary
//    • right-click a marker → Remove Split
//
//  Value-driven (markers + duration + callbacks); the only local
//  state is interaction bookkeeping, so the snapshot suite renders
//  it over a synthetic bucket field.
//

import AppKit
import SwiftUI

/// One marker as the overlay renders it. Mirrors the FFI `RipSplit`
/// as a plain value so the view never touches DubCore.
struct RipMarkerUi: Equatable, Identifiable {
    let id: UInt32
    var secs: Double
}

/// Split-edit callbacks. `addSplit` / `moveSplit` report rejection
/// (`false`) so the overlay can flash; the model owns the actual
/// FFI calls + error surfacing.
struct RipSplitOverlayCallbacks {
    var addSplit: (Double) -> Bool = { _ in true }
    var moveSplit: (UInt32, Double) -> Bool = { _, _ in true }
    var removeSplit: (UInt32) -> Void = { _ in }
    /// Audition around a boundary — the caller decides the lead-in.
    var audition: (Double) -> Void = { _ in }
    /// Seek deck A (single click / drag on empty band).
    var scrub: (Double) -> Void = { _ in }
}

struct RipSplitMarkerOverlay: View {

    let markers: [RipMarkerUi]
    let durationSecs: Double
    var callbacks = RipSplitOverlayCallbacks()
    /// Snapshot hook: pre-select a marker without a click.
    var initialSelectedId: UInt32? = nil

    /// Hit radius around a marker line, in points.
    private static let hitRadius: CGFloat = 6
    /// Below this movement a gesture is a click, not a drag.
    private static let clickSlop: CGFloat = 3
    /// Two clicks within this window + slop = double-click.
    private static let doubleClickSecs: TimeInterval = 0.4
    private static let moveDispatchMinInterval: TimeInterval = 1.0 / 30.0

    @State private var selectedId: UInt32?
    @State private var drag: DragBookkeeping? = nil
    @State private var dragEchoSecs: Double? = nil
    @State private var lastClick: (date: Date, x: CGFloat)? = nil
    @State private var lastMoveDispatch: TimeInterval = 0
    @State private var rejectFlash = false

    private struct DragBookkeeping {
        var markerId: UInt32?
        var startX: CGFloat
        var moved: Bool
    }

    init(
        markers: [RipMarkerUi],
        durationSecs: Double,
        callbacks: RipSplitOverlayCallbacks = RipSplitOverlayCallbacks(),
        initialSelectedId: UInt32? = nil
    ) {
        self.markers = markers
        self.durationSecs = durationSecs
        self.callbacks = callbacks
        self.initialSelectedId = initialSelectedId
        _selectedId = State(initialValue: initialSelectedId)
    }

    var body: some View {
        GeometryReader { geo in
            ZStack(alignment: .topLeading) {
                // Rejection flash — the band tints red for a beat when
                // the FFI refuses an add / move.
                if rejectFlash {
                    Rectangle()
                        .fill(DubColor.stateError.opacity(0.25))
                        .allowsHitTesting(false)
                }
                ForEach(displayMarkers) { marker in
                    let x = xPosition(secs: marker.secs, width: geo.size.width)
                    RipMarkerGlyph(
                        selected: marker.id == selectedId,
                        height: geo.size.height)
                        .position(x: x, y: geo.size.height * 0.5)
                        .contextMenu {
                            Button("Remove Split", role: .destructive) {
                                callbacks.removeSplit(marker.id)
                                if selectedId == marker.id { selectedId = nil }
                            }
                        }
                }
                RipSplitKeyCapture(
                    isActive: selectedId != nil,
                    onNudge: { deltaSecs in nudgeSelected(by: deltaSecs) },
                    onDelete: { removeSelected() })
                    .frame(width: 0, height: 0)
            }
            .contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: 0)
                    .onChanged { value in
                        handleDragChanged(value, size: geo.size)
                    }
                    .onEnded { value in
                        handleDragEnded(value, size: geo.size)
                    })
        }
    }

    /// Markers with the in-flight drag echo substituted, so the
    /// dragged marker tracks the pointer every frame while the FFI
    /// dispatch stays throttled.
    private var displayMarkers: [RipMarkerUi] {
        guard let drag, let id = drag.markerId, let echo = dragEchoSecs else {
            return markers
        }
        return markers.map { m in
            m.id == id ? RipMarkerUi(id: m.id, secs: echo) : m
        }
    }

    // MARK: Fraction grid (shared with TrackOverviewView)

    private func fraction(atX x: CGFloat, width: CGFloat) -> Double {
        let pad = OverviewLayout.endPadding
        let axisLength = width - 2 * pad
        guard axisLength > 0 else { return 0 }
        let local = max(0, min(axisLength, x - pad))
        return Double(local / axisLength)
    }

    private func secs(atX x: CGFloat, width: CGFloat) -> Double {
        fraction(atX: x, width: width) * durationSecs
    }

    private func xPosition(secs: Double, width: CGFloat) -> CGFloat {
        let pad = OverviewLayout.endPadding
        let axisLength = max(0, width - 2 * pad)
        guard durationSecs > 0 else { return pad }
        let f = max(0, min(1, secs / durationSecs))
        return pad + axisLength * CGFloat(f)
    }

    private func hitTestMarker(atX x: CGFloat, width: CGFloat) -> UInt32? {
        var best: (id: UInt32, distance: CGFloat)? = nil
        for m in markers {
            let mx = xPosition(secs: m.secs, width: width)
            let d = abs(mx - x)
            guard d <= Self.hitRadius else { continue }
            if let current = best {
                if d < current.distance { best = (m.id, d) }
            } else {
                best = (m.id, d)
            }
        }
        return best?.id
    }

    private func marker(id: UInt32) -> RipMarkerUi? {
        markers.first { $0.id == id }
    }

    // MARK: Gesture handling

    private func handleDragChanged(_ value: DragGesture.Value, size: CGSize) {
        guard durationSecs > 0 else { return }
        if drag == nil {
            drag = DragBookkeeping(
                markerId: hitTestMarker(atX: value.startLocation.x, width: size.width),
                startX: value.startLocation.x,
                moved: false)
        }
        guard var bookkeeping = drag else { return }
        if abs(value.location.x - bookkeeping.startX) > Self.clickSlop {
            bookkeeping.moved = true
        }
        drag = bookkeeping
        guard bookkeeping.moved else { return }

        let targetSecs = secs(atX: value.location.x, width: size.width)
        if let id = bookkeeping.markerId {
            dragEchoSecs = targetSecs
            let now = ProcessInfo.processInfo.systemUptime
            if now - lastMoveDispatch >= Self.moveDispatchMinInterval {
                lastMoveDispatch = now
                _ = callbacks.moveSplit(id, targetSecs)
            }
        } else {
            // Empty-band drag: forward as a seek scrub so the review
            // band keeps the overview's audition workflow.
            let now = ProcessInfo.processInfo.systemUptime
            if now - lastMoveDispatch >= Self.moveDispatchMinInterval {
                lastMoveDispatch = now
                callbacks.scrub(targetSecs)
            }
        }
    }

    private func handleDragEnded(_ value: DragGesture.Value, size: CGSize) {
        guard durationSecs > 0 else {
            drag = nil
            return
        }
        let bookkeeping = drag
        drag = nil
        let x = value.location.x
        let endSecs = secs(atX: x, width: size.width)

        guard let bookkeeping else { return }
        if !bookkeeping.moved {
            handleClick(atX: x, secs: endSecs, width: size.width)
            return
        }
        if let id = bookkeeping.markerId {
            dragEchoSecs = nil
            if !callbacks.moveSplit(id, endSecs) {
                flashRejection()
            }
            selectedId = id
        } else {
            callbacks.scrub(endSecs)
        }
    }

    private func handleClick(atX x: CGFloat, secs: Double, width: CGFloat) {
        let isDouble: Bool
        if let last = lastClick,
           Date().timeIntervalSince(last.date) < Self.doubleClickSecs,
           abs(last.x - x) <= Self.hitRadius {
            isDouble = true
            lastClick = nil
        } else {
            isDouble = false
            lastClick = (Date(), x)
        }

        if let id = hitTestMarker(atX: x, width: width) {
            if isDouble {
                if let m = marker(id: id) { callbacks.audition(m.secs) }
            } else {
                selectedId = id
            }
        } else if isDouble {
            if !callbacks.addSplit(secs) {
                flashRejection()
            }
        } else {
            selectedId = nil
            callbacks.scrub(secs)
        }
    }

    // MARK: Keyboard

    private func nudgeSelected(by deltaSecs: Double) {
        guard let id = selectedId, let m = marker(id: id) else { return }
        let target = min(max(0, m.secs + deltaSecs), durationSecs)
        if !callbacks.moveSplit(id, target) {
            flashRejection()
        }
    }

    private func removeSelected() {
        guard let id = selectedId else { return }
        callbacks.removeSplit(id)
        selectedId = nil
    }

    private func flashRejection() {
        rejectFlash = true
        Task { @MainActor in
            try? await Task.sleep(nanoseconds: 180_000_000)
            rejectFlash = false
        }
    }
}

/// One marker: a 2 pt full-height line with a 12 pt diamond handle
/// near the top — `DubColor.hotCue` family, brighter when selected.
private struct RipMarkerGlyph: View {
    let selected: Bool
    let height: CGFloat

    var body: some View {
        ZStack(alignment: .top) {
            Rectangle()
                .fill(DubColor.hotCue.opacity(selected ? 1.0 : 0.75))
                .frame(width: 2, height: height)
            Rectangle()
                .fill(selected ? DubColor.hotCue : DubColor.hotCue.opacity(0.85))
                .frame(width: 8.5, height: 8.5)
                .rotationEffect(.degrees(45))
                .overlay(
                    Rectangle()
                        .stroke(selected ? Color.white : Color.clear, lineWidth: 1)
                        .rotationEffect(.degrees(45))
                        .frame(width: 8.5, height: 8.5))
                .offset(y: 2)
        }
        .frame(width: 12, height: height)
    }
}

/// Local keyDown monitor for the nudge / delete keys. SwiftUI focus
/// on macOS 13 is too unreliable for a transient overlay, so this
/// mirrors `MainView`'s `KeyEventMonitorHost` approach: consume
/// ← / → / ⌫ only while a marker is selected and no text field has
/// first-responder status.
private struct RipSplitKeyCapture: NSViewRepresentable {
    let isActive: Bool
    let onNudge: (Double) -> Void
    let onDelete: () -> Void

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> NSView {
        let view = NSView(frame: .zero)
        context.coordinator.install(onNudge: onNudge, onDelete: onDelete)
        context.coordinator.isActive = isActive
        return view
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        context.coordinator.isActive = isActive
        context.coordinator.update(onNudge: onNudge, onDelete: onDelete)
    }

    static func dismantleNSView(_ nsView: NSView, coordinator: Coordinator) {
        coordinator.uninstall()
    }

    @MainActor
    final class Coordinator {
        var isActive = false
        private var monitor: Any?
        private var onNudge: (Double) -> Void = { _ in }
        private var onDelete: () -> Void = {}

        func install(onNudge: @escaping (Double) -> Void, onDelete: @escaping () -> Void) {
            update(onNudge: onNudge, onDelete: onDelete)
            guard monitor == nil else { return }
            monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
                guard let self, self.isActive, !self.isTextFirstResponder() else {
                    return event
                }
                let shift = event.modifierFlags
                    .intersection(.deviceIndependentFlagsMask).contains(.shift)
                let step = shift ? 1.0 : 0.1
                switch event.keyCode {
                case 123: // ←
                    self.onNudge(-step)
                    return nil
                case 124: // →
                    self.onNudge(step)
                    return nil
                case 51: // ⌫
                    self.onDelete()
                    return nil
                default:
                    return event
                }
            }
        }

        func update(onNudge: @escaping (Double) -> Void, onDelete: @escaping () -> Void) {
            self.onNudge = onNudge
            self.onDelete = onDelete
        }

        func uninstall() {
            if let m = monitor { NSEvent.removeMonitor(m) }
            monitor = nil
        }

        private func isTextFirstResponder() -> Bool {
            guard let responder = NSApp.keyWindow?.firstResponder else { return false }
            return responder is NSText || responder is NSTextView
        }

        deinit {
            if let m = monitor { NSEvent.removeMonitor(m) }
        }
    }
}
