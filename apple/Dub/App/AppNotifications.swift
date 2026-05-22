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
}
