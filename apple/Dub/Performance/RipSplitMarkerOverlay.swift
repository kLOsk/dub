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
//    • drag either trim bracket → move where the side starts / ends;
//      the discarded lead-in and run-out shade out. ⌫ on a bracket
//      resets that end to the whole capture (a trim can be reset but
//      never removed, unlike a split).
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

/// Where the side begins and ends inside the capture. The lead-in
/// groove before `startSecs` and the run-out after `endSecs` are
/// discarded at commit — `side.flac` keeps them, so nothing here is
/// irreversible.
struct RipTrimUi: Equatable {
    var startSecs: Double
    var endSecs: Double
}

/// What a click selected. A trim bracket is not a marker: its
/// position is not a stable FFI id and it can never be deleted, only
/// reset — so it gets its own case rather than a sentinel id.
enum RipOverlaySelection: Equatable {
    case split(UInt32)
    case trimStart
    case trimEnd
}

/// Split-edit callbacks. `addSplit` / `moveSplit` / `setSideStart` /
/// `setSideEnd` report rejection (`false`) so the overlay can flash;
/// the model owns the actual FFI calls + error surfacing.
struct RipSplitOverlayCallbacks {
    var addSplit: (Double) -> Bool = { _ in true }
    var moveSplit: (UInt32, Double) -> Bool = { _, _ in true }
    var removeSplit: (UInt32) -> Void = { _ in }
    /// Audition around a boundary — the caller decides the lead-in.
    var audition: (Double) -> Void = { _ in }
    /// Seek deck A (single click / drag on empty band).
    var scrub: (Double) -> Void = { _ in }
    var setSideStart: (Double) -> Bool = { _ in true }
    var setSideEnd: (Double) -> Bool = { _ in true }
}

/// Where a split may sit once the side is trimmed.
///
/// File-scope and `internal` rather than a method on the view: it is
/// the one piece of this overlay with real off-by-one risk (a zero
/// duration between review and the deck-A decode landing, inverted
/// bounds from a salvaged manifest) and it should be testable without
/// rendering anything.
enum RipTrimClamp {
    static func split(_ secs: Double, in trim: RipTrimUi?, duration: Double) -> Double {
        let lower = max(0, trim?.startSecs ?? 0)
        let upper = min(duration, trim?.endSecs ?? duration)
        guard upper > lower else { return lower }
        return min(max(lower, secs), upper)
    }
}

struct RipSplitMarkerOverlay: View {

    let markers: [RipMarkerUi]
    let durationSecs: Double
    /// `nil` when nothing is trimmed — the whole capture is the side,
    /// which is what a manual rip and every pre-M26b session get. The
    /// overlay then renders exactly as it did before trims existed.
    var trim: RipTrimUi? = nil
    var callbacks = RipSplitOverlayCallbacks()
    /// Snapshot hook: pre-select something without a click.
    var initialSelection: RipOverlaySelection? = nil

    /// Hit radius around a marker line, in points.
    private static let hitRadius: CGFloat = 6
    /// Below this movement a gesture is a click, not a drag.
    private static let clickSlop: CGFloat = 3
    /// Two clicks within this window + slop = double-click.
    private static let doubleClickSecs: TimeInterval = 0.4
    private static let moveDispatchMinInterval: TimeInterval = 1.0 / 30.0

    @State private var selection: RipOverlaySelection?
    @State private var drag: DragBookkeeping? = nil
    @State private var dragEchoSecs: Double? = nil
    @State private var lastClick: (date: Date, x: CGFloat)? = nil
    @State private var lastMoveDispatch: TimeInterval = 0
    @State private var rejectFlash = false

    private struct DragBookkeeping {
        /// What the drag grabbed; `nil` is an empty-band scrub.
        var target: RipOverlaySelection?
        var startX: CGFloat
        var moved: Bool
    }

    init(
        markers: [RipMarkerUi],
        durationSecs: Double,
        trim: RipTrimUi? = nil,
        callbacks: RipSplitOverlayCallbacks = RipSplitOverlayCallbacks(),
        initialSelection: RipOverlaySelection? = nil
    ) {
        self.markers = markers
        self.durationSecs = durationSecs
        self.trim = trim
        self.callbacks = callbacks
        self.initialSelection = initialSelection
        _selection = State(initialValue: initialSelection)
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
                // The discarded ends, drawn under the markers so a
                // rejected drag echo that strays into one is still
                // visible. Deliberately from x = 0 rather than from
                // the 8 pt gutter: the gutter is dropped material too.
                if let t = displayTrim {
                    trimShade(from: 0, to: xPosition(secs: t.startSecs, width: geo.size.width),
                              height: geo.size.height)
                    trimShade(from: xPosition(secs: t.endSecs, width: geo.size.width),
                              to: geo.size.width, height: geo.size.height)
                }
                ForEach(displayMarkers) { marker in
                    let x = xPosition(secs: marker.secs, width: geo.size.width)
                    RipMarkerGlyph(
                        selected: selection == .split(marker.id),
                        height: geo.size.height)
                        .position(x: x, y: geo.size.height * 0.5)
                        .contextMenu {
                            Button("Remove Split", role: .destructive) {
                                callbacks.removeSplit(marker.id)
                                if selection == .split(marker.id) { selection = nil }
                            }
                        }
                }
                if let t = displayTrim {
                    RipTrimGlyph(edge: .start, selected: selection == .trimStart,
                                 height: geo.size.height)
                        .position(x: xPosition(secs: t.startSecs, width: geo.size.width),
                                  y: geo.size.height * 0.5)
                        .contextMenu {
                            Button("Keep the lead-in") { _ = callbacks.setSideStart(0) }
                        }
                    RipTrimGlyph(edge: .end, selected: selection == .trimEnd,
                                 height: geo.size.height)
                        .position(x: xPosition(secs: t.endSecs, width: geo.size.width),
                                  y: geo.size.height * 0.5)
                        .contextMenu {
                            Button("Keep the run-out") { _ = callbacks.setSideEnd(durationSecs) }
                        }
                }
                RipSplitKeyCapture(
                    isActive: selection != nil,
                    onNudge: { deltaSecs in nudgeSelection(by: deltaSecs) },
                    onDelete: { deleteSelection() })
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
        guard let drag, case .split(let id)? = drag.target, let echo = dragEchoSecs else {
            return markers
        }
        return markers.map { m in
            m.id == id ? RipMarkerUi(id: m.id, secs: echo) : m
        }
    }

    /// The trim with the in-flight drag echo substituted, so a bracket
    /// tracks the pointer every frame while the FFI stays throttled —
    /// the same single echo the markers use, because only one thing is
    /// ever dragged at a time.
    private var displayTrim: RipTrimUi? {
        guard var t = trim else { return nil }
        guard let drag, let echo = dragEchoSecs else { return t }
        switch drag.target {
        case .trimStart: t.startSecs = echo
        case .trimEnd: t.endSecs = echo
        default: break
        }
        return t
    }

    @ViewBuilder
    private func trimShade(from: CGFloat, to: CGFloat, height: CGFloat) -> some View {
        let width = max(0, to - from)
        if width > 0 {
            Rectangle()
                .fill(DubColor.surface0.opacity(0.62))
                .frame(width: width, height: height)
                .position(x: from + width * 0.5, y: height * 0.5)
                .allowsHitTesting(false)
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

    /// Nearest grabbable thing within the hit radius. Splits are
    /// offered first so an exact tie with a bracket keeps the split —
    /// a bracket sits at the edge of the band where nothing else is,
    /// while a split can legitimately be dragged right up to one.
    private func hitTest(atX x: CGFloat, width: CGFloat) -> RipOverlaySelection? {
        var candidates: [(RipOverlaySelection, Double)] = markers.map { (.split($0.id), $0.secs) }
        if let t = trim {
            candidates.append((.trimStart, t.startSecs))
            candidates.append((.trimEnd, t.endSecs))
        }
        var best: (target: RipOverlaySelection, distance: CGFloat)? = nil
        for (target, secs) in candidates {
            let d = abs(xPosition(secs: secs, width: width) - x)
            guard d <= Self.hitRadius else { continue }
            if let current = best {
                if d < current.distance { best = (target, d) }
            } else {
                best = (target, d)
            }
        }
        return best?.target
    }

    /// Seconds the given target currently sits at.
    private func secsOf(_ target: RipOverlaySelection) -> Double? {
        switch target {
        case .split(let id): return marker(id: id)?.secs
        case .trimStart: return trim?.startSecs
        case .trimEnd: return trim?.endSecs
        }
    }

    /// Push a target to `secs`. Returns false when the FFI refuses.
    private func move(_ target: RipOverlaySelection, to secs: Double) -> Bool {
        switch target {
        case .split(let id):
            return callbacks.moveSplit(
                id, RipTrimClamp.split(secs, in: trim, duration: durationSecs))
        // Clamped only to the structural bound the UI can know. The
        // 5 s minimum-segment rule lives in Rust; duplicating it here
        // would fork silently the day it changes, so a bracket that
        // eats a track is refused by the FFI and flashes.
        case .trimStart:
            let upper = trim?.endSecs ?? durationSecs
            return callbacks.setSideStart(min(max(0, secs), upper))
        case .trimEnd:
            let lower = trim?.startSecs ?? 0
            return callbacks.setSideEnd(max(min(durationSecs, secs), lower))
        }
    }

    private func marker(id: UInt32) -> RipMarkerUi? {
        markers.first { $0.id == id }
    }

    // MARK: Gesture handling

    private func handleDragChanged(_ value: DragGesture.Value, size: CGSize) {
        guard durationSecs > 0 else { return }
        if drag == nil {
            drag = DragBookkeeping(
                target: hitTest(atX: value.startLocation.x, width: size.width),
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
        if let target = bookkeeping.target {
            // Echo where the pointer is, not where the clamp would put
            // it: the shade moving under a bracket that has stopped is
            // what tells the operator they have hit a limit.
            dragEchoSecs = targetSecs
            let now = ProcessInfo.processInfo.systemUptime
            if now - lastMoveDispatch >= Self.moveDispatchMinInterval {
                lastMoveDispatch = now
                // Fire and forget mid-drag: a boundary that momentarily
                // crosses another must not strobe the band at 30 Hz.
                _ = move(target, to: targetSecs)
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
        if let target = bookkeeping.target {
            // Clear the echo before the final call, so a refused move
            // visibly snaps back instead of leaving a stale position.
            dragEchoSecs = nil
            if !move(target, to: endSecs) {
                flashRejection()
            }
            selection = target
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

        if let target = hitTest(atX: x, width: width) {
            if isDouble {
                if let at = secsOf(target) { callbacks.audition(at) }
            } else {
                selection = target
            }
        } else if isDouble {
            // Deliberately *not* clamped into the side. A drag is a
            // continuous gesture that wants a wall; a double-click out
            // in the shaded lead-in is a discrete intent, and refusing
            // it says something the shade has already explained.
            if !callbacks.addSplit(secs) {
                flashRejection()
            }
        } else {
            selection = nil
            callbacks.scrub(secs)
        }
    }

    // MARK: Keyboard

    /// Nudging is the highest-value gesture here: trimming to the
    /// exact second the groove noise starts is not a drag job.
    private func nudgeSelection(by deltaSecs: Double) {
        guard let target = selection, let at = secsOf(target) else { return }
        if !move(target, to: min(max(0, at + deltaSecs), durationSecs)) {
            flashRejection()
        }
    }

    /// ⌫ removes a split, but a trim bracket always exists — it can
    /// only be reset to the whole capture, which is what the context
    /// menu offers too.
    private func deleteSelection() {
        switch selection {
        case .split(let id):
            callbacks.removeSplit(id)
            selection = nil
        case .trimStart:
            if !callbacks.setSideStart(0) { flashRejection() }
        case .trimEnd:
            if !callbacks.setSideEnd(durationSecs) { flashRejection() }
        case nil:
            break
        }
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

/// A side bound: a full-height line with stubs pointing *into* the
/// part of the capture that is kept, so the bracket reads as a
/// boundary rather than as another split.
///
/// Deliberately a different shape, not just a different hue — the
/// band already carries the deck-A envelope and magenta cue markers,
/// and a fourth colour would not be a distinction anyone could rely
/// on at a glance.
private struct RipTrimGlyph: View {
    enum Edge { case start, end }

    let edge: Edge
    let selected: Bool
    let height: CGFloat

    private var tint: Color {
        DubColor.textSecondary.opacity(selected ? 1.0 : 0.8)
    }

    var body: some View {
        let stub: CGFloat = 6
        return ZStack(alignment: edge == .start ? .leading : .trailing) {
            Rectangle()
                .fill(tint)
                .frame(width: 2, height: height)
            VStack(spacing: 0) {
                Rectangle().fill(tint).frame(width: stub, height: 2)
                Spacer(minLength: 0)
                Rectangle().fill(tint).frame(width: stub, height: 2)
            }
            .frame(width: stub, height: height)
            if selected {
                Rectangle()
                    .stroke(Color.white, lineWidth: 1)
                    .frame(width: stub, height: height)
            }
        }
        .frame(width: stub, height: height)
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
