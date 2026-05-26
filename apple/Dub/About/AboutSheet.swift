//
//  AboutSheet.swift
//  Dub
//
//  Project info panel opened from the status-strip wordmark or
//  Dub → About Dub. Uses the splash artwork as a hero banner.
//  Dismiss with Esc or a click outside the panel.
//

import AppKit
import SwiftUI
import DubCore

/// Full-window scrim + centred About panel. Tap the dimmed backdrop
/// or press Esc to dismiss; clicks on the panel itself do not close.
struct AboutOverlay: View {

    let onDismiss: () -> Void

    var body: some View {
        ZStack {
            Color.black.opacity(0.55)
                .ignoresSafeArea()
                .onTapGesture(perform: onDismiss)

            AboutSheet()
                .clipShape(RoundedRectangle(cornerRadius: DubRadius.card))
                .shadow(color: .black.opacity(0.45), radius: 24, y: 8)
        }
        .onExitCommand(perform: onDismiss)
    }
}

struct AboutSheet: View {

    private let githubURL = URL(string: "https://github.com/kLOsk/dub")
    private let docsURL = URL(string: "https://github.com/kLOsk/dub/blob/main/docs/PRD.md")

    var body: some View {
        VStack(spacing: 0) {
            splashHero
            content
        }
        .frame(width: 520, height: 600)
        .background(DubColor.surface0)
    }

    private var splashHero: some View {
        Image("AboutSplash")
            .resizable()
            .scaledToFit()
            .clipShape(RoundedRectangle(cornerRadius: DubRadius.lg))
            .padding(.horizontal, DubSpacing.lg)
            .padding(.top, DubSpacing.lg)
            .accessibilityLabel("Dub")
    }

    private var content: some View {
        VStack(alignment: .leading, spacing: DubSpacing.lg) {
            VStack(alignment: .leading, spacing: DubSpacing.sm) {
                Text("Timecode-vinyl DJ application for scratch DJs and vinyl enthusiasts.")
                    .font(DubFont.body)
                    .foregroundStyle(DubColor.textPrimary)
                    .fixedSize(horizontal: false, vertical: true)

                Text("Built for the urban and sound-system scene — hip hop, reggae and dub, drum and bass, dubstep, scratch. Two decks, your external mixer, real records through the software. Reliability is the product.")
                    .font(DubFont.micro)
                    .foregroundStyle(DubColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }

            VStack(alignment: .leading, spacing: DubSpacing.xs) {
                aboutRow("App", appVersionText)
                aboutRow("Engine", engineVersion())
                aboutRow("Bundle", bundleIdentifier)
                aboutRow("Status", "Pre-alpha · Phase A")
                aboutRow("Platform", "macOS 13+ · Apple Silicon + Intel")
                aboutRow("License", "GPLv3-or-later")
            }

            HStack(spacing: DubSpacing.md) {
                if let githubURL {
                    linkButton("GitHub repository", url: githubURL)
                }
                if let docsURL {
                    linkButton("Product spec", url: docsURL)
                }
            }
        }
        .padding(DubSpacing.xl)
    }

    private func aboutRow(_ label: String, _ value: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: DubSpacing.md) {
            Text(label.uppercased())
                .font(DubFont.caps)
                .tracking(0.6)
                .foregroundStyle(DubColor.textTertiary)
                .frame(width: 72, alignment: .leading)
            Text(value)
                .font(DubFont.body)
                .foregroundStyle(DubColor.textPrimary)
                .textSelection(.enabled)
        }
    }

    private func linkButton(_ title: String, url: URL) -> some View {
        Button(title) {
            NSWorkspace.shared.open(url)
        }
        .buttonStyle(.link)
        .font(DubFont.body)
        .foregroundStyle(DubColor.textSecondary)
    }

    private var appVersionText: String {
        let version = Bundle.main.object(
            forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "?"
        let build = Bundle.main.object(
            forInfoDictionaryKey: "CFBundleVersion") as? String ?? "?"
        return "\(version) (\(build))"
    }

    private var bundleIdentifier: String {
        Bundle.main.bundleIdentifier ?? "com.klos.dub"
    }
}

#Preview("panel") {
    AboutSheet()
}

#Preview("overlay") {
    AboutOverlay(onDismiss: {})
        .frame(width: 900, height: 700)
}
