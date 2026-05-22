//
//  DubAppDelegate.swift
//  Dub
//
//  AppKit lifecycle for the Dub macOS shell. Keeps the AppKit entry point
//  lean — actual content lives in `MainWindowController`, which hosts a
//  SwiftUI view via `NSHostingController`. This hybrid pattern is
//  deliberate: AppKit owns the lifecycle (it has the lowest-overhead
//  hooks we'll need for the audio HUD in M10), SwiftUI owns the
//  non-realtime sub-views (settings, library, etc.).
//
//  The explicit `static func main()` exists because the default
//  `NSApplicationDelegate.main()` in the current macOS Swift overlay
//  calls `NSApplicationMain` *without* installing an instance as
//  `NSApp.delegate`. Without that wiring the app launches into an
//  event loop with no delegate, `applicationDidFinishLaunching` is
//  never invoked, no window is created, and the user sees only a
//  menu bar. (UIKit's overlay does the right thing; AppKit's does
//  not unless you also load a `MainMenu.xib` that sets the delegate
//  on nib-load — we don't ship a nib by design.) Holding the instance
//  in a static guarantees it outlives `NSApp.delegate`'s weak
//  reference for the program's lifetime.
//

import AppKit

@main
final class DubAppDelegate: NSObject, NSApplicationDelegate {
    private static let sharedDelegate = DubAppDelegate()

    private var mainWindowController: MainWindowController?

    static func main() {
        let app = NSApplication.shared
        app.delegate = sharedDelegate
        _ = NSApplicationMain(CommandLine.argc, CommandLine.unsafeArgv)
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.regular)
        installMainMenu()

        let controller = MainWindowController()
        controller.showWindow(self)
        self.mainWindowController = controller

        NSApp.activate(ignoringOtherApps: true)

        // PRD §2.1 "No Mouse DJ Ever" — performance mode is
        // full-screen. We launch directly into full-screen so a DJ
        // who opens the app onstage isn't fumbling with the green
        // traffic-light button. Activation has to happen first so
        // the new full-screen Space is created on the display
        // that currently holds the window (centered → main
        // display, unless the user reopens after dragging the
        // windowed shell elsewhere). The MainWindowController's
        // `windowDidExitFullScreen` hook handles the snap-back to
        // 1440x900 when the user leaves full-screen for prep mode.
        controller.window?.toggleFullScreen(nil)
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }

    // MARK: - Menu bar

    private func installMainMenu() {
        let appName = Bundle.main.object(
            forInfoDictionaryKey: "CFBundleName") as? String ?? "Dub"

        let mainMenu = NSMenu()

        let appMenuItem = NSMenuItem()
        mainMenu.addItem(appMenuItem)

        let appMenu = NSMenu(title: appName)
        appMenuItem.submenu = appMenu

        let aboutItem = NSMenuItem(
            title: "About \(appName)",
            action: #selector(showAbout(_:)),
            keyEquivalent: "")
        aboutItem.target = self
        appMenu.addItem(aboutItem)

        appMenu.addItem(.separator())

        let prefsItem = NSMenuItem(
            title: "Preferences…",
            action: #selector(showPreferences(_:)),
            keyEquivalent: ",")
        prefsItem.target = self
        appMenu.addItem(prefsItem)

        appMenu.addItem(.separator())

        appMenu.addItem(withTitle: "Hide \(appName)",
                        action: #selector(NSApplication.hide(_:)),
                        keyEquivalent: "h")

        let hideOthersItem = NSMenuItem(
            title: "Hide Others",
            action: #selector(NSApplication.hideOtherApplications(_:)),
            keyEquivalent: "h")
        hideOthersItem.keyEquivalentModifierMask = [.command, .option]
        appMenu.addItem(hideOthersItem)

        appMenu.addItem(withTitle: "Show All",
                        action: #selector(NSApplication.unhideAllApplications(_:)),
                        keyEquivalent: "")

        appMenu.addItem(.separator())

        appMenu.addItem(withTitle: "Quit \(appName)",
                        action: #selector(NSApplication.terminate(_:)),
                        keyEquivalent: "q")

        let editMenuItem = NSMenuItem()
        mainMenu.addItem(editMenuItem)
        let editMenu = NSMenu(title: "Edit")
        editMenuItem.submenu = editMenu
        editMenu.addItem(withTitle: "Undo",
                         action: Selector(("undo:")),
                         keyEquivalent: "z")
        editMenu.addItem(withTitle: "Redo",
                         action: Selector(("redo:")),
                         keyEquivalent: "Z")
        editMenu.addItem(.separator())
        editMenu.addItem(withTitle: "Cut",
                         action: #selector(NSText.cut(_:)),
                         keyEquivalent: "x")
        editMenu.addItem(withTitle: "Copy",
                         action: #selector(NSText.copy(_:)),
                         keyEquivalent: "c")
        editMenu.addItem(withTitle: "Paste",
                         action: #selector(NSText.paste(_:)),
                         keyEquivalent: "v")
        editMenu.addItem(withTitle: "Select All",
                         action: #selector(NSText.selectAll(_:)),
                         keyEquivalent: "a")

        let viewMenuItem = NSMenuItem()
        mainMenu.addItem(viewMenuItem)
        let viewMenu = NSMenu(title: "View")
        viewMenuItem.submenu = viewMenu
        // AppKit automatically retitles this item to "Exit Full
        // Screen" when full-screen is active, because the
        // `toggleFullScreen:` selector advertises both titles
        // through `NSMenuItem` validation. We don't need a
        // companion "Exit Full Screen" item.
        let fullScreenItem = NSMenuItem(
            title: "Enter Full Screen",
            action: #selector(NSWindow.toggleFullScreen(_:)),
            keyEquivalent: "f")
        fullScreenItem.keyEquivalentModifierMask = [.command, .control]
        viewMenu.addItem(fullScreenItem)

        let windowMenuItem = NSMenuItem()
        mainMenu.addItem(windowMenuItem)
        let windowMenu = NSMenu(title: "Window")
        windowMenuItem.submenu = windowMenu
        windowMenu.addItem(withTitle: "Minimize",
                           action: #selector(NSWindow.miniaturize(_:)),
                           keyEquivalent: "m")
        windowMenu.addItem(withTitle: "Zoom",
                           action: #selector(NSWindow.zoom(_:)),
                           keyEquivalent: "")
        windowMenu.addItem(.separator())
        windowMenu.addItem(withTitle: "Bring All to Front",
                           action: #selector(NSApplication.arrangeInFront(_:)),
                           keyEquivalent: "")

        NSApp.mainMenu = mainMenu
        NSApp.windowsMenu = windowMenu
    }

    @objc private func showAbout(_ sender: Any?) {
        NotificationCenter.default.post(name: .dubShowAbout, object: nil)
    }

    @objc private func showPreferences(_ sender: Any?) {
        NotificationCenter.default.post(name: .dubShowPreferences, object: nil)
    }
}
