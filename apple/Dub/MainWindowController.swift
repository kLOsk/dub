//
//  MainWindowController.swift
//  Dub
//
//  Top-level window for the Dub shell. Hosts a SwiftUI `MainView`
//  (waveform + device picker) inside an `NSHostingController`. The
//  M0.5 smoke-screen text now lives as a debug overlay inside
//  `MainView`.
//

import AppKit
import SwiftUI

final class MainWindowController: NSWindowController, NSWindowDelegate {
    /// PRD §9.2 reference rectangle. Also the size we *force* the
    /// window back to every time the user exits full-screen — we
    /// deliberately do not persist a user-resized windowed size
    /// because Dub's only justified windowed use is prep mode
    /// (Finder side-by-side, library curation) and that workflow
    /// doesn't benefit from per-session size memory. Live
    /// performance always runs full-screen (PRD §2.1).
    static let defaultContentSize = NSSize(width: 1440, height: 900)

    convenience init() {
        // First-launch windowed dimensions = PRD §9.2 reference
        // rectangle (the canvas the Figma exploration used). The
        // size is enforced by the window's `contentRect` below
        // and re-asserted with `setContentSize(defaultSize)`; we
        // intentionally do *not* push the size through the
        // hosting controller's `preferredContentSize` because
        // doing so also makes the SwiftUI hierarchy's intrinsic
        // size 1440x900 on macOS 13+, which prevents full-screen
        // from actually filling the display (see the
        // `sizingOptions = []` comment below). `minSize` stays at
        // 720x480 as a layout floor during a transient drag.
        let defaultSize = MainWindowController.defaultContentSize

        let hostingController = NSHostingController(rootView: MainView())
        // On macOS 13+, `NSHostingController.sizingOptions` defaults
        // to `[.preferredContentSize, .intrinsicContentSize]`. With
        // those options on, anything we set on `preferredContentSize`
        // — or anything SwiftUI computes as its intrinsic size — is
        // exposed to AppKit's Auto Layout as a *hard* intrinsic
        // content size on the hosting view. That intrinsic size
        // then competes with the edge-pinning constraints AppKit
        // auto-adds between `window.contentView` and
        // `contentViewController.view`, and wins on its own axis:
        // the SwiftUI hierarchy refuses to grow past its computed
        // intrinsic size when the window goes full-screen, and the
        // window background paints the surplus as a black frame
        // around the 1440x900 layout. Clearing the options makes
        // SwiftUI strictly take whatever AppKit hands it, so the
        // performance surface fills the screen on every display
        // (1440x900 retina laptop, 5K Studio Display, 6K XDR,
        // ultrawide, etc.) without any per-display scaling logic.
        //
        // We still set the initial windowed size below via the
        // window's `contentRect` + `setContentSize(defaultSize)`,
        // so first-launch dimensions are unchanged.
        hostingController.sizingOptions = []

        // `.resizable` stays in the style mask on purpose. Removing
        // it would also strip the green traffic-light button and
        // disable `NSWindow.toggleFullScreen(_:)` entirely, which
        // is the exact API we use to enter full-screen on launch
        // and via `Cmd+Ctrl+F`. The snap-back-on-exit hook below
        // (`windowDidExitFullScreen`) means the resize affordance
        // is effectively cosmetic: any size the user drags to is
        // thrown away the next time they leave full-screen.
        let window = NSWindow(
            contentRect: NSRect(origin: .zero, size: defaultSize),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.title = "Dub"
        window.contentViewController = hostingController
        window.setContentSize(defaultSize)
        window.minSize = NSSize(width: 720, height: 480)
        // `.fullScreenPrimary` enables the green traffic-light
        // button's full-screen action, the `View → Enter Full
        // Screen` menu item, and macOS' standard `Cmd+Ctrl+F`
        // shortcut — all routed through `NSWindow.toggleFullScreen(_:)`.
        window.collectionBehavior.insert(.fullScreenPrimary)
        window.center()

        self.init(window: window)
        window.delegate = self
    }

    // MARK: - NSWindowDelegate

    /// Snap windowed mode back to the documented default
    /// every time the user leaves full-screen. We never carry a
    /// drag-resized window across full-screen toggles or app
    /// launches: the only "remembered" windowed size is 1440x900
    /// centered. Performance runs full-screen; prep mode runs at
    /// the reference rectangle. See plan: full-screen window
    /// mode.
    func windowDidExitFullScreen(_ notification: Notification) {
        guard let window else { return }
        window.setContentSize(MainWindowController.defaultContentSize)
        window.center()
    }
}
