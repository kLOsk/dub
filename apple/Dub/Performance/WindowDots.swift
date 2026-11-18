//
//  WindowDots.swift
//  Dub
//
//  Full screen keeps the menu bar away (`MainWindowController`), and with it
//  macOS's own window buttons, so a DJ who doesn't know ⌘Q or ⌃⌘F has no way
//  out. These three dots stand in for them, at the left of the status strip
//  as Serato does (Daniel, 2026-11-18): left quits (through the quit page),
//  middle would minimise and is off in full screen, right goes to window
//  mode. Dub's grey at rest; macOS's red / yellow / green and glyphs while
//  the pointer is over them, like the real ones. Only in full screen: a
//  window has the real buttons.
//

import AppKit
import SwiftUI

/// Whether Dub's window is full screen. `MainWindowController` reports it.
@MainActor
final class FullScreenState: ObservableObject {
    static let shared = FullScreenState()
    @Published var isFullScreen = false
}

struct WindowDots: View {
    @ObservedObject private var screen = FullScreenState.shared
    @State private var hovering = false

    var body: some View {
        if screen.isFullScreen {
            HStack(spacing: 8) {
                dot(
                    color: Color(hex: 0xFF5F57), glyph: "xmark",
                    help: "Quit Dub (⌘Q)") {
                    QuitGuard.shared.ask()
                }
                dot(color: nil, glyph: nil, help: "Minimise is off in full screen", action: nil)
                dot(
                    color: Color(hex: 0x28C840),
                    glyph: "arrow.down.right.and.arrow.up.left",
                    help: "Window mode (⌃⌘F)") {
                    NSApp.mainWindow?.toggleFullScreen(nil)
                }
            }
            .onHover { hovering = $0 }
            .accessibilityElement(children: .contain)
        }
    }

    private func dot(
        color: Color?, glyph: String?, help: String, action: (() -> Void)?
    ) -> some View {
        let lit = hovering ? color : nil
        let rest = action == nil ? DubColor.plateEdge : DubColor.textPlaceholder
        return Button {
            action?()
        } label: {
            ZStack {
                Circle()
                    .fill(lit ?? rest)
                if lit != nil, let glyph {
                    Image(systemName: glyph)
                        .font(.system(size: 7, weight: .heavy))
                        .foregroundStyle(Color.black.opacity(0.55))
                }
            }
            .frame(width: 12, height: 12)
            .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .disabled(action == nil)
        .help(help)
        .accessibilityLabel(help)
    }
}
