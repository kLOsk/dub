//
//  WaveformMetalHostView.swift
//  Dub
//
//  M11d.6 round 5 (Phase 2 of the off-main-thread waveform refactor).
//
//  A bare `NSView` whose backing layer is a `CAMetalLayer`. The new
//  off-main `WaveformRenderThread` draws into this layer from a
//  dedicated serial dispatch queue scheduled by a per-deck
//  `CVDisplayLink`, so no amount of SwiftUI body re-evaluation,
//  AppKit layout, or library FFI work on the main thread can stall
//  the waveform pipeline.
//
//  Goals:
//  - **Stable drawable.** `layerContentsRedrawPolicy = .never` keeps
//    AppKit from invalidating the layer when an enclosing SwiftUI
//    view's `updateNSView` re-stamps appearance props. Draw cadence
//    is owned by the render thread, not the main runloop.
//  - **Display tracking.** The render thread needs to call
//    `CVDisplayLinkSetCurrentCGDisplay` whenever the host window
//    moves between physical displays. We surface that as a closure
//    the render thread installs in Phase 3; this view's job is to
//    fire it on `viewDidMoveToWindow` and on
//    `NSWindow.didChangeScreenNotification`.
//  - **Backing-scale-aware drawable size.** macOS will silently
//    paint at the wrong resolution if `CAMetalLayer.drawableSize`
//    doesn't track `bounds.size × backingScaleFactor`. AppKit
//    already gives us a `layout` / `viewDidEndLiveResize` callback
//    on the main thread for free; we update the layer there.
//
//  This file is purely the AppKit shell; the `WaveformRenderer` is
//  unchanged for now (Phase 4 makes it thread-safe) and
//  `WaveformMetalView` keeps using `MTKView` until Phase 5 cuts
//  over. Phase 2 lands the type alongside the existing pipeline so
//  the rest of the refactor can be incremental.
//

import AppKit
import QuartzCore

/// AppKit host that owns a `CAMetalLayer`. The off-main render
/// thread draws into `metalLayer`; the main thread only updates
/// `drawableSize` on layout and notifies the render thread of
/// display migration.
final class WaveformMetalHostView: NSView {

    /// Logical (non-Retina) clear colour the view paints behind the
    /// Metal layer while a frame is pending. The renderer's clear
    /// colour wins once it draws; this only shows during the few-
    /// ms window between `init` and the render thread's first
    /// vsync.
    static let backgroundClearColor = CGColor(
        red: 0.07, green: 0.07, blue: 0.08, alpha: 1.0)

    /// Closure fired on the main thread whenever the host window
    /// either gains a screen for the first time or migrates
    /// between screens. The render thread installs this in Phase 3
    /// to call `CVDisplayLinkSetCurrentCGDisplay` on its
    /// `CVDisplayLink` — without it, the link would keep
    /// requesting vsync callbacks at the source display's refresh
    /// rate, which is visibly wrong on mixed-rate setups (e.g. a
    /// 120 Hz internal panel + a 60 Hz external monitor).
    ///
    /// Holds `nil` until the render thread is wired up, in which
    /// case migrations are a no-op (the `MTKView` path was already
    /// this lax).
    var onDisplayChange: ((CGDirectDisplayID) -> Void)?

    /// Returns the backing `CAMetalLayer`. AppKit guarantees the
    /// layer is set before any subview / draw cycle because
    /// `wantsLayer = true` forces `makeBackingLayer` to fire from
    /// `init`.
    var metalLayer: CAMetalLayer {
        guard let layer = self.layer as? CAMetalLayer else {
            preconditionFailure(
                "WaveformMetalHostView.layer is not a CAMetalLayer; "
                + "did something else call setLayer(_:)?")
        }
        return layer
    }

    // MARK: - Lifecycle

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        commonInit()
    }

    required init?(coder: NSCoder) {
        super.init(coder: coder)
        commonInit()
    }

    private func commonInit() {
        wantsLayer = true
        // The render thread owns redraw cadence. AppKit must not
        // mark the layer dirty on geometry / visibility changes;
        // we hand the new size to the render thread via
        // `metalLayer.drawableSize` and let it produce the next
        // frame on its own schedule.
        layerContentsRedrawPolicy = .never
        // Tells AppKit to keep the contents pegged at the layer's
        // backing origin rather than scaling when bounds change
        // before the render thread catches up — avoids a one-frame
        // smear on live resize.
        layer?.contentsGravity = .topLeft
        // Until the renderer paints its first frame, paint the
        // same dark grey the existing MTKView path used so the
        // venetian-blind background doesn't flash white.
        layer?.backgroundColor = Self.backgroundClearColor
    }

    /// AppKit calls this once on `wantsLayer = true` if `layer` is
    /// nil. Returning a configured `CAMetalLayer` here (rather
    /// than assigning `self.layer` post-init) is the supported
    /// way to host one inside an `NSView`.
    override func makeBackingLayer() -> CALayer {
        let layer = CAMetalLayer()
        layer.pixelFormat = .bgra8Unorm
        layer.framebufferOnly = true
        // `presentsWithTransaction = false` keeps the present
        // path off the main thread's CATransaction commit, so the
        // render thread's `currentDrawable.present()` doesn't
        // wait for the next AppKit transaction. This is the
        // single most important flag for off-main rendering.
        layer.presentsWithTransaction = false
        // `allowsNextDrawableTimeout = true` is the default; we
        // leave it so a hidden / minimised window doesn't deadlock
        // the render thread waiting on a drawable that will never
        // arrive.
        layer.allowsNextDrawableTimeout = true
        // Pin contents under live resize to the layer's top-left
        // so a slightly-stale drawable doesn't visibly scale up
        // before the next render thread frame lands.
        layer.contentsGravity = .topLeft
        return layer
    }

    // MARK: - Drawable-size sync

    /// Computed from `bounds.size × backingScaleFactor`. Clamped
    /// to ≥ 1×1 so a zero-height layout (transient during sheet
    /// presentations or split-view drags) doesn't trip
    /// `CAMetalLayer`'s "drawable size must be positive" assertion.
    var currentDrawableSize: CGSize {
        let scale = window?.backingScaleFactor ?? layer?.contentsScale ?? 2.0
        let w = max(1.0, bounds.width * scale)
        let h = max(1.0, bounds.height * scale)
        return CGSize(width: w, height: h)
    }

    private func syncDrawableSize() {
        let layer = metalLayer
        let size = currentDrawableSize
        if layer.drawableSize != size {
            layer.drawableSize = size
        }
        let scale = window?.backingScaleFactor ?? 2.0
        if layer.contentsScale != scale {
            layer.contentsScale = scale
        }
    }

    override func layout() {
        super.layout()
        syncDrawableSize()
    }

    override func viewDidEndLiveResize() {
        super.viewDidEndLiveResize()
        syncDrawableSize()
    }

    override func viewDidChangeBackingProperties() {
        super.viewDidChangeBackingProperties()
        syncDrawableSize()
    }

    // MARK: - Display migration

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        installWindowObservers()
        syncDrawableSize()
        notifyDisplayChange()
    }

    /// Re-subscribe to `NSWindow.didChangeScreenNotification`
    /// whenever the view's window changes. AppKit doesn't auto-
    /// migrate notification observers when reparenting, so a
    /// view that moves between two windows during its lifetime
    /// (which can happen if SwiftUI rebuilds the enclosing
    /// `NSHostingView`) would otherwise stop receiving screen-
    /// change callbacks on the second window.
    private func installWindowObservers() {
        NotificationCenter.default.removeObserver(
            self, name: NSWindow.didChangeScreenNotification, object: nil)
        guard let window else { return }
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(windowDidChangeScreen(_:)),
            name: NSWindow.didChangeScreenNotification,
            object: window)
    }

    @objc
    private func windowDidChangeScreen(_ note: Notification) {
        notifyDisplayChange()
    }

    private func notifyDisplayChange() {
        guard let onDisplayChange,
              let screen = window?.screen,
              let displayID = Self.directDisplayID(for: screen)
        else { return }
        onDisplayChange(displayID)
    }

    /// Resolve an `NSScreen` to its `CGDirectDisplayID`. The id
    /// lives on the screen's `deviceDescription` dictionary under
    /// the `NSScreenNumber` key. Returns `nil` if the dictionary
    /// is missing the key (which shouldn't happen on macOS 10.0+).
    static func directDisplayID(for screen: NSScreen) -> CGDirectDisplayID? {
        let key = NSDeviceDescriptionKey("NSScreenNumber")
        guard let number = screen.deviceDescription[key] as? NSNumber else {
            return nil
        }
        return CGDirectDisplayID(number.uint32Value)
    }

    deinit {
        NotificationCenter.default.removeObserver(self)
    }
}
