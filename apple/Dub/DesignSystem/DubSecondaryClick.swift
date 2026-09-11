//
//  DubSecondaryClick.swift
//  Dub
//
//  A right-click that can decide.
//
//  `.contextMenu` opens a menu on every secondary click, full stop. The
//  sampler pad needs the click itself to *do* something while a take is
//  sounding — stop it, with no menu in the way of a hand mid-set — and
//  to offer Unload only once the pad is quiet. That is one gesture with
//  two outcomes chosen at click time, which SwiftUI's modifier cannot
//  express; an AppKit view can.
//
//  ## How the left button stays SwiftUI's
//
//  A representable is a real `NSView` inside the hosting view, and AppKit
//  hit-tests real views before the hosting view's own SwiftUI content —
//  so a plain overlay would swallow every click, including the
//  press-down the pad fires on. `hitTest` therefore answers only for a
//  secondary click (right button, or ⌃-click), reading the event AppKit
//  is hit-testing for from `NSApp.currentEvent`, and returns `nil` for
//  everything else so the left button falls through to the gesture
//  underneath untouched.
//

import AppKit
import SwiftUI

/// What a secondary click did, or wants shown.
enum SecondaryClickResponse {
    /// The click was consumed by an action; show nothing.
    case handled
    /// Open this menu at the click.
    case menu([SecondaryMenuItem])
}

/// One item of a click-time menu.
struct SecondaryMenuItem {
    let title: String
    /// Draws the check mark — the current choice in a set of options.
    var checked: Bool = false
    let action: () -> Void
    /// A separator line; `title` and `action` are ignored.
    var isSeparator: Bool = false

    init(_ title: String, checked: Bool = false, action: @escaping () -> Void) {
        self.title = title
        self.checked = checked
        self.action = action
    }

    static var separator: SecondaryMenuItem {
        var item = SecondaryMenuItem("") {}
        item.isSeparator = true
        return item
    }
}

extension View {
    /// Handle a secondary click (right button or ⌃-click) with a
    /// decision made at click time: consume it, or pop a menu.
    ///
    /// Left clicks are not intercepted — a sibling `.onPressDown` keeps
    /// firing on mouse-down exactly as before.
    func onSecondaryClick(_ respond: @escaping () -> SecondaryClickResponse) -> some View {
        overlay(SecondaryClickCatcher(respond: respond))
    }
}

private struct SecondaryClickCatcher: NSViewRepresentable {
    let respond: () -> SecondaryClickResponse

    func makeNSView(context: Context) -> SecondaryClickView {
        let view = SecondaryClickView()
        view.respond = respond
        return view
    }

    func updateNSView(_ view: SecondaryClickView, context: Context) {
        view.respond = respond
    }
}

final class SecondaryClickView: NSView {
    var respond: (() -> SecondaryClickResponse)?
    /// Retained while a menu is up: `NSMenuItem.target` is weak, and a
    /// menu whose target has gone renders every item disabled.
    private var menuActions: [MenuAction] = []

    override func hitTest(_ point: NSPoint) -> NSView? {
        guard Self.isSecondary(NSApp.currentEvent),
            bounds.contains(convert(point, from: superview))
        else { return nil }
        return self
    }

    override func rightMouseDown(with event: NSEvent) {
        handle(event)
    }

    /// ⌃-click arrives as a left button down with the control flag;
    /// `hitTest` only routes it here when it is one.
    override func mouseDown(with event: NSEvent) {
        handle(event)
    }

    private static func isSecondary(_ event: NSEvent?) -> Bool {
        guard let event else { return false }
        switch event.type {
        case .rightMouseDown:
            return true
        case .leftMouseDown:
            return event.modifierFlags.contains(.control)
        default:
            return false
        }
    }

    private func handle(_ event: NSEvent) {
        switch respond?() {
        case .menu(let items):
            let menu = NSMenu()
            menuActions = items.compactMap { item in
                if item.isSeparator {
                    menu.addItem(.separator())
                    return nil
                }
                let holder = MenuAction(item.action)
                let entry = NSMenuItem(
                    title: item.title, action: #selector(MenuAction.fire), keyEquivalent: "")
                entry.target = holder
                entry.state = item.checked ? .on : .off
                menu.addItem(entry)
                return holder
            }
            NSMenu.popUpContextMenu(menu, with: event, for: self)
            menuActions = []
        case .handled, nil:
            break
        }
    }
}

private final class MenuAction: NSObject {
    private let perform: () -> Void

    init(_ perform: @escaping () -> Void) {
        self.perform = perform
    }

    @objc func fire() {
        perform()
    }
}
