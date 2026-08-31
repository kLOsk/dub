//
//  AppNotifications.swift
//  Dub
//
//  AppKit → SwiftUI bridges for menu-bar commands. The delegate
//  posts these; `MainView` owns the sheet presentation state.
//

import Foundation

extension Notification.Name {
    static let dubShowAbout = Notification.Name("com.klos.dub.showAbout")
    static let dubShowPreferences = Notification.Name("com.klos.dub.showPreferences")
    /// Re-open the first-run onboarding flow (U-23). Posted by the
    /// "Show welcome guide" button in Preferences; observed by MainView.
    static let dubShowOnboarding = Notification.Name("com.klos.dub.showOnboarding")
    /// M11f — File > Export Library As... Posted by the delegate;
    /// observed by MainView, which owns the library handle the save
    /// panel needs. PRD §8.6 wants this reachable from the menu bar,
    /// not only from a right-click in the sidebar.
    static let dubExportLibrary = Notification.Name("com.klos.dub.exportLibrary")
}
