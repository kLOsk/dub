//
//  LayerTickers.swift
//  Dub
//
//  The two things on screen that change on a clock rather than on an
//  event — the overview's playhead and the elapsed / remaining digits —
//  drawn on Core Animation layers that are *moved*, never laid out.
//
//  ## Why not a `TimelineView`
//
//  Both used to be `TimelineView`s (4 Hz and 2 Hz). A `TimelineView`
//  tick re-evaluates its content inside SwiftUI's graph, and in an
//  `NSHostingView` that schedules an AppKit layout pass for the
//  *window*: `NSWindow.layoutIfNeeded → _layoutViewTree`, walking the
//  whole SwiftUI graph. Profiled on the rig (2026-09-23, rip review
//  playing on the 2019 i9): ~630 of 6 272 main-thread samples in eight
//  seconds were in those passes, ~15 ms each, six times a second — and
//  the only Dub code beneath them was these two tickers. The cost was
//  the graph, not the tickers, which is why it was worst in rip review:
//  the overview, the split overlay and a card of text fields per track
//  all get walked every tick.
//
//  The Metal strip renders off the main thread, but it waits in
//  `CAMetalLayer.nextDrawable()` for the compositor to hand a buffer
//  back, and a late main-thread commit makes the compositor late. So a
//  15 ms layout pass six times a second became six dropped frames a
//  second in the strip: the "jumping" the DJ saw. `TrackOverviewView`
//  already carries the note for this mechanism from an earlier round —
//  "a main-runloop spike contending with the off-main Metal render
//  thread's `nextDrawable()`" — which moved the overview's *bars* out
//  of the tick and left the tick itself in place.
//
//  A layer whose `position` or `string` changes needs no layout at all:
//  Core Animation commits the property change and composites it. So
//  each ticker here is a layer-backed `NSView` with a `Timer`, and
//  steady playback schedules no SwiftUI or AppKit layout at all.
//

import AppKit
import DubCore
import SwiftUI

// MARK: - Playhead

/// The overview's playhead bracket — halo, core line, a chevron at
/// each end — built once for the view's size and then only moved.
struct PlayheadLayer: NSViewRepresentable {
    let orientation: WaveformOrientation
    /// Where the playhead is, 0…1 along the time axis, or `nil` to
    /// hide it. Read on every tick; must be cheap (lock-free FFI).
    let fraction: () -> Double?
    /// A scrub in progress overrides the engine, so the bracket
    /// follows the pointer rather than the audio.
    var dragFraction: Double? = nil

    func makeNSView(context: Context) -> PlayheadHostView {
        let v = PlayheadHostView()
        v.orientation = orientation
        v.fraction = fraction
        v.dragFraction = dragFraction
        return v
    }

    func updateNSView(_ v: PlayheadHostView, context: Context) {
        v.fraction = fraction
        if v.orientation != orientation {
            v.orientation = orientation
            v.needsLayout = true
        }
        if v.dragFraction != dragFraction {
            v.dragFraction = dragFraction
            // A scrub is the one time the bracket must not wait for
            // the next tick.
            v.refresh()
        }
    }
}

final class PlayheadHostView: NSView {
    var orientation: WaveformOrientation = .horizontal
    var fraction: () -> Double? = { nil }
    var dragFraction: Double?

    /// Same cadence the `TimelineView` had: on a four-minute track
    /// the bracket moves < 0.4 px a tick, and on a side less. The
    /// tick is free now, but a faster one would buy nothing visible.
    private static let interval: TimeInterval = 0.25

    private let marker = CALayer()
    private let halo = CALayer()
    private let core = CALayer()
    private let chevronA = CAShapeLayer()
    private let chevronB = CAShapeLayer()
    private var timer: Timer?

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.masksToBounds = true
        let accent = NSColor(DubColor.playheadAccent).cgColor
        halo.backgroundColor = NSColor.black.withAlphaComponent(0.55).cgColor
        core.backgroundColor = accent
        chevronA.fillColor = accent
        chevronB.fillColor = accent
        for l in [halo, core, chevronA, chevronB] { marker.addSublayer(l) }
        marker.isHidden = true
        layer?.addSublayer(marker)
        disableImplicitAnimations()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is unused") }

    /// Top-down, like the `Canvas` this replaces.
    override var isFlipped: Bool { true }

    /// Clicks belong to the scrub gesture underneath.
    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        timer?.invalidate()
        timer = nil
        guard window != nil else { return }
        let t = Timer(timeInterval: Self.interval, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.refresh() }
        }
        // `.common`, so the bracket keeps moving while a menu is open
        // or a slider is being dragged.
        RunLoop.main.add(t, forMode: .common)
        timer = t
        refresh()
    }

    /// Rebuild the bracket for a new size. The only time this view
    /// touches geometry beyond a position.
    override func layout() {
        super.layout()
        let size = bounds.size
        let chevron = DubLayout.playheadChevronSize
        let coreW = DubLayout.playheadCoreWidth
        let haloW = DubLayout.playheadHaloWidth
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        switch orientation {
        case .vertical:
            // A horizontal bar across the strip, positioned along y.
            marker.bounds = CGRect(x: 0, y: 0, width: size.width, height: chevron * 2)
            halo.frame = CGRect(x: 0, y: chevron - haloW / 2, width: size.width, height: haloW)
            core.frame = CGRect(x: 0, y: chevron - coreW / 2, width: size.width, height: coreW)
            chevronA.path = triangle(
                CGPoint(x: 0, y: 0), CGPoint(x: chevron, y: chevron), CGPoint(x: 0, y: chevron * 2))
            chevronB.path = triangle(
                CGPoint(x: size.width, y: 0), CGPoint(x: size.width - chevron, y: chevron),
                CGPoint(x: size.width, y: chevron * 2))
        case .horizontal:
            // A vertical bar across the strip, positioned along x.
            marker.bounds = CGRect(x: 0, y: 0, width: chevron * 2, height: size.height)
            halo.frame = CGRect(x: chevron - haloW / 2, y: 0, width: haloW, height: size.height)
            core.frame = CGRect(x: chevron - coreW / 2, y: 0, width: coreW, height: size.height)
            chevronA.path = triangle(
                CGPoint(x: 0, y: 0), CGPoint(x: chevron, y: chevron), CGPoint(x: chevron * 2, y: 0))
            chevronB.path = triangle(
                CGPoint(x: 0, y: size.height), CGPoint(x: chevron, y: size.height - chevron),
                CGPoint(x: chevron * 2, y: size.height))
        }
        CATransaction.commit()
        refresh()
    }

    /// Move the bracket to the current position. Position only — no
    /// layout, no redraw.
    func refresh() {
        let f = dragFraction.map { max(0, min(1, $0)) } ?? fraction()
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        defer { CATransaction.commit() }
        guard let f else {
            marker.isHidden = true
            return
        }
        marker.isHidden = false
        let pad = OverviewLayout.endPadding
        let size = bounds.size
        switch orientation {
        case .vertical:
            let y = pad + max(0, size.height - 2 * pad) * CGFloat(f)
            marker.position = CGPoint(x: size.width / 2, y: y)
        case .horizontal:
            let x = pad + max(0, size.width - 2 * pad) * CGFloat(f)
            marker.position = CGPoint(x: x, y: size.height / 2)
        }
    }

    private func triangle(_ a: CGPoint, _ b: CGPoint, _ c: CGPoint) -> CGPath {
        let p = CGMutablePath()
        p.move(to: a)
        p.addLine(to: b)
        p.addLine(to: c)
        p.closeSubpath()
        return p
    }

    private func disableImplicitAnimations() {
        let none: [String: CAAction] = [
            "position": NSNull(), "bounds": NSNull(), "hidden": NSNull(),
            "frame": NSNull(), "path": NSNull(),
        ]
        for l in [marker, halo, core, chevronA, chevronB] { l.actions = none }
    }
}

// MARK: - Clock

/// Elapsed or remaining time, drawn on a `CATextLayer` and refreshed
/// on its own clock. See the file comment for why this is not a
/// `TimelineView`.
///
/// The font is monospaced, so the width depends only on the number of
/// characters; the view reports a new size only when that changes
/// (`00:00` → `1:00:00`, or a sign appearing), which is once a track
/// if ever. Every other tick changes the layer's string and nothing
/// else.
struct LayerClockText: NSViewRepresentable {
    let engine: DubEngine
    let deckIdx: UInt64
    let slot: LiveDeckTimeText.Slot
    let size: CGFloat
    var weight: NSFont.Weight = .medium
    let color: Color

    func makeNSView(context: Context) -> LayerClockView {
        let v = LayerClockView()
        configure(v)
        return v
    }

    func updateNSView(_ v: LayerClockView, context: Context) {
        configure(v)
    }

    private func configure(_ v: LayerClockView) {
        v.read = { [engine, deckIdx, slot] in
            LiveDeckTimeText.text(for: engine.positionSnapshot(deckIdx: deckIdx), slot: slot)
        }
        v.setStyle(
            font: NSFont.monospacedSystemFont(ofSize: size, weight: weight),
            color: NSColor(color))
    }
}

final class LayerClockView: NSView {
    var read: () -> String = { "" }

    /// The text only changes once a second; half a second keeps the
    /// rollover fresh. Same cadence the `TimelineView` used.
    private static let interval: TimeInterval = 0.5

    private let text = CATextLayer()
    private var font = NSFont.monospacedSystemFont(ofSize: 14, weight: .medium)
    private var shown = ""
    private var timer: Timer?

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        text.actions = ["contents": NSNull(), "string": NSNull(), "bounds": NSNull(), "position": NSNull()]
        text.alignmentMode = .left
        text.truncationMode = .none
        layer?.addSublayer(text)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is unused") }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    func setStyle(font: NSFont, color: NSColor) {
        let fontChanged = font != self.font
        self.font = font
        text.font = font
        text.fontSize = font.pointSize
        text.foregroundColor = color.cgColor
        if fontChanged { invalidateIntrinsicContentSize() }
    }

    /// Sized to the text on screen — monospaced, so only the character
    /// count matters. `shown` starts empty; the first tick sizes it.
    override var intrinsicContentSize: NSSize {
        let sample = shown.isEmpty ? "-00:00" : shown
        let w = (sample as NSString).size(withAttributes: [.font: font]).width
        return NSSize(width: ceil(w), height: ceil(font.ascender - font.descender + font.leading))
    }

    override func viewDidChangeBackingProperties() {
        super.viewDidChangeBackingProperties()
        text.contentsScale = window?.backingScaleFactor ?? 2
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        timer?.invalidate()
        timer = nil
        guard window != nil else { return }
        text.contentsScale = window?.backingScaleFactor ?? 2
        let t = Timer(timeInterval: Self.interval, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.refresh() }
        }
        RunLoop.main.add(t, forMode: .common)
        timer = t
        refresh()
    }

    override func layout() {
        super.layout()
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        text.frame = bounds
        CATransaction.commit()
    }

    private func refresh() {
        let next = read()
        guard next != shown else { return }
        let reshaped = next.count != shown.count
        shown = next
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        text.string = next
        CATransaction.commit()
        // The one case that needs layout: a different number of
        // characters. A second ticking over is not that case.
        if reshaped { invalidateIntrinsicContentSize() }
    }
}
