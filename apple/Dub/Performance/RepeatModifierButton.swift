//
//  RepeatModifierButton.swift
//  Dub
//
//  AppKit-backed header button that fires on mouse-down and repeats
//  while held. Modifier keys (Shift / Option) are sampled on each
//  fire so the step tier can change mid-hold.
//

import AppKit
import SwiftUI

/// Small square header control: one immediate action on press, then
/// auto-repeat while the mouse button stays down.
struct RepeatModifierButton: NSViewRepresentable {
    let systemImage: String?
    let title: String?
    let help: String
    let onAction: (_ modifiers: NSEvent.ModifierFlags) -> Void

    init(
        systemImage: String,
        help: String,
        onAction: @escaping (_ modifiers: NSEvent.ModifierFlags) -> Void
    ) {
        self.systemImage = systemImage
        self.title = nil
        self.help = help
        self.onAction = onAction
    }

    init(
        title: String,
        help: String,
        onAction: @escaping (_ modifiers: NSEvent.ModifierFlags) -> Void
    ) {
        self.systemImage = nil
        self.title = title
        self.help = help
        self.onAction = onAction
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(onAction: onAction)
    }

    func makeNSView(context: Context) -> RepeatModifierControlView {
        let view = RepeatModifierControlView()
        view.coordinator = context.coordinator
        view.toolTip = help
        return view
    }

    func updateNSView(_ nsView: RepeatModifierControlView, context: Context) {
        context.coordinator.onAction = onAction
        nsView.systemImage = systemImage
        nsView.title = title
        nsView.toolTip = help
        nsView.needsDisplay = true
    }

    @MainActor
    final class Coordinator: NSObject {
        var onAction: (_ modifiers: NSEvent.ModifierFlags) -> Void

        init(onAction: @escaping (_ modifiers: NSEvent.ModifierFlags) -> Void) {
            self.onAction = onAction
        }

        func fire(with modifiers: NSEvent.ModifierFlags) {
            onAction(modifiers)
        }
    }
}

/// Custom control: immediate fire on `mouseDown`, hold-to-repeat
/// after an initial delay, stop on `mouseUp`.
@MainActor
final class RepeatModifierControlView: NSView {
    weak var coordinator: RepeatModifierButton.Coordinator?

    var systemImage: String?
    var title: String?

    private var holdTimer: Timer?
    private var repeatTimer: Timer?
    private var isPressed = false
    private var hoverTracking: NSTrackingArea?

    override var isFlipped: Bool { true }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let hoverTracking {
            removeTrackingArea(hoverTracking)
        }
        let area = NSTrackingArea(
            rect: bounds,
            options: [.mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect],
            owner: self,
            userInfo: nil)
        addTrackingArea(area)
        hoverTracking = area
    }

    override func mouseDown(with event: NSEvent) {
        isPressed = true
        needsDisplay = true
        coordinator?.fire(with: event.modifierFlags)
        holdTimer?.invalidate()
        holdTimer = Timer.scheduledTimer(withTimeInterval: 0.35, repeats: false) { [weak self] _ in
            Task { @MainActor in
                self?.startRepeatLoop()
            }
        }
    }

    override func mouseUp(with event: NSEvent) {
        stopRepeat()
        isPressed = false
        needsDisplay = true
    }

    override func mouseExited(with event: NSEvent) {
        if isPressed {
            stopRepeat()
            isPressed = false
            needsDisplay = true
        }
    }

    private func startRepeatLoop() {
        guard isPressed else { return }
        repeatTimer?.invalidate()
        repeatTimer = Timer.scheduledTimer(withTimeInterval: 0.08, repeats: true) { [weak self] _ in
            Task { @MainActor in
                guard let self, self.isPressed else { return }
                self.coordinator?.fire(with: NSEvent.modifierFlags)
            }
        }
        if let repeatTimer {
            RunLoop.main.add(repeatTimer, forMode: .common)
        }
    }

    private func stopRepeat() {
        holdTimer?.invalidate()
        holdTimer = nil
        repeatTimer?.invalidate()
        repeatTimer = nil
    }

    override func draw(_ dirtyRect: NSRect) {
        let bg = isPressed
            ? NSColor(white: 0.22, alpha: 1)
            : NSColor(white: 0.16, alpha: 1)
        bg.setFill()
        NSBezierPath(roundedRect: bounds, xRadius: 4, yRadius: 4).fill()

        let fg = NSColor(white: 0.72, alpha: 1)
        fg.set()

        if let systemImage,
           let image = NSImage(
               systemSymbolName: systemImage,
               accessibilityDescription: toolTip) {
            let config = NSImage.SymbolConfiguration(
                pointSize: 10, weight: .semibold)
            let sized = image.withSymbolConfiguration(config) ?? image
            let size = NSSize(width: 10, height: 10)
            let origin = NSPoint(
                x: (bounds.width - size.width) / 2,
                y: (bounds.height - size.height) / 2)
            sized.draw(in: NSRect(origin: origin, size: size))
        } else if let title {
            let attrs: [NSAttributedString.Key: Any] = [
                .font: NSFont.systemFont(ofSize: 11, weight: .semibold),
                .foregroundColor: fg,
            ]
            let str = NSAttributedString(string: title, attributes: attrs)
            let size = str.size()
            let origin = NSPoint(
                x: (bounds.width - size.width) / 2,
                y: (bounds.height - size.height) / 2)
            str.draw(at: origin)
        }
    }

    override var intrinsicContentSize: NSSize {
        NSSize(width: 22, height: 22)
    }

    deinit {
        holdTimer?.invalidate()
        repeatTimer?.invalidate()
    }
}
