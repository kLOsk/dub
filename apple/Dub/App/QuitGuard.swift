//
//  QuitGuard.swift
//  Dub
//
//  ⌘Q mid-set must never just stop the music (Daniel, 2026-11-18, after
//  Serato's full-screen "quit?" page). The first ⌘Q, the Dock's Quit, or
//  the window's close button covers the surface with a quit page instead.
//  Only a click on Quit quits: no key does, so a stray ⏎ or a second ⌘Q
//  mid-set can't end it (Daniel). Esc goes back to the decks. A logout,
//  restart or shutdown is the Mac's decision and is never held up.
//

import AppKit
import SwiftUI

@MainActor
final class QuitGuard: ObservableObject {
    static let shared = QuitGuard()

    @Published private(set) var asking = false
    private var confirmed = false
    private var keyMonitor: Any?

    /// `applicationShouldTerminate`'s answer.
    func shouldTerminate() -> NSApplication.TerminateReply {
        if confirmed || Self.systemIsQuitting || Self.underTest { return .terminateNow }
        // ⌘Q again on the quit page changes nothing: quitting takes a click.
        ask()
        return .terminateCancel
    }

    /// The window's close button: same page, same answer.
    func shouldClose() -> Bool {
        if confirmed { return true }
        ask()
        return false
    }

    func ask() {
        guard !asking else { return }
        asking = true
        // While the page is up, every key is the page's: a deck key, a cue
        // or a sample must not fire behind it.
        keyMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
            guard let self else { return event }
            if event.keyCode == 53 { self.cancel() } // Esc
            return nil
        }
    }

    func confirm() {
        confirmed = true
        removeMonitor()
        DispatchQueue.main.async { NSApp.terminate(nil) }
    }

    func cancel() {
        asking = false
        removeMonitor()
    }

    private func removeMonitor() {
        if let keyMonitor { NSEvent.removeMonitor(keyMonitor) }
        keyMonitor = nil
    }

    /// The test runner hosts the app and ends it when the suite is done.
    private static let underTest =
        ProcessInfo.processInfo.environment["XCTestConfigurationFilePath"] != nil

    /// A quit Apple event that carries a reason comes from logout,
    /// restart or shutdown; one without (the Dock, ⌘Q) is the DJ's.
    private static var systemIsQuitting: Bool {
        guard let event = NSAppleEventManager.shared().currentAppleEvent,
              event.eventClass == kCoreEventClass, event.eventID == kAEQuitApplication
        else { return false }
        return event.attributeDescriptor(forKeyword: AEKeyword(kAEQuitReason)) != nil
    }
}

/// The quit page: the whole surface, so it can't be missed or mistaken
/// for a deck control.
struct QuitOverlay: View {
    @ObservedObject var guardian: QuitGuard

    var body: some View {
        ZStack {
            DubColor.surface0.opacity(0.94)
                .ignoresSafeArea()
                .contentShape(Rectangle())
                .onTapGesture {} // clicks stop here, not on the decks
            VStack(spacing: DubSpacing.xl) {
                Text("Quit Dub?")
                    .font(.system(size: 44, weight: .semibold))
                    .foregroundStyle(DubColor.textPrimary)
                Text("The decks stop and Dub's sound goes off.")
                    .font(DubFont.body)
                    .foregroundStyle(DubColor.textSecondary)
                HStack(spacing: DubSpacing.lg) {
                    choice("Back to the decks", key: "esc", action: guardian.cancel)
                    choice("Quit", key: nil, prominent: true, action: guardian.confirm)
                }
                .padding(.top, DubSpacing.md)
            }
        }
    }

    private func choice(
        _ title: String, key: String?, prominent: Bool = false, action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            HStack(spacing: DubSpacing.sm) {
                Text(title).font(DubFont.title)
                if let key {
                    Text(key)
                        .font(DubFont.micro)
                        .foregroundStyle(DubColor.textTertiary)
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .overlay(
                            RoundedRectangle(cornerRadius: 4)
                                .stroke(DubColor.textTertiary.opacity(0.6), lineWidth: 1))
                }
            }
            .foregroundStyle(DubColor.textPrimary)
            .padding(.horizontal, DubSpacing.xl)
            .padding(.vertical, DubSpacing.md)
            .frame(minWidth: 220)
            .background(
                RoundedRectangle(cornerRadius: DubRadius.card)
                    .fill(prominent ? DubColor.textPrimary.opacity(0.14) : Color.clear))
            .overlay(
                RoundedRectangle(cornerRadius: DubRadius.card)
                    .stroke(DubColor.textPrimary.opacity(prominent ? 0.5 : 0.2), lineWidth: 1))
        }
        .buttonStyle(.plain)
    }
}
