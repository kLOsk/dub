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
    /// Open the timecode signal-quality panel. Posted by Preferences;
    /// observed by MainView.
    static let dubShowSignalQuality = Notification.Name("com.klos.dub.showSignalQuality")
}
