//
//  LaunchSplashOverlay.swift
//  Dub
//
//  Brief cold-boot splash shown while the engine + library initialize.
//  Same artwork as the About sheet; dismissed with a short fade once
//  boot work finishes and a minimum display interval has elapsed.
//

import SwiftUI

struct LaunchSplashOverlay: View {

    var body: some View {
        ZStack {
            DubColor.surface0
                .ignoresSafeArea()
            Image("AboutSplash")
                .resizable()
                .scaledToFit()
                .clipShape(RoundedRectangle(cornerRadius: DubRadius.xl))
                .shadow(color: .black.opacity(0.35), radius: 32, y: 12)
                .frame(maxWidth: 560)
                .padding(DubSpacing.xxl)
        }
        .accessibilityHidden(true)
    }
}

#Preview {
    LaunchSplashOverlay()
        .frame(width: 800, height: 500)
}
