//
//  WaveformZoomControl.swift
//  Dub
//
//  The pair of buttons that zoom both playing waveforms.
//
//  ## Why both decks, and why in the gutter
//
//  Zoom is a property of *looking*, not of a deck. A DJ comparing two
//  strips wants them at the same scale — two decks at different zooms
//  is a beatmatching aid that lies. So there is one control, it sits in
//  the centre gutter between the strips, and it moves both.
//
//  ## Why the beatmatch aid does not zoom with them
//
//  Stillpoint has no time axis to zoom. It shows drift as a band's
//  *position*, not as seconds of audio, so a "zoom" would either do
//  nothing or change what the position means — a scale change on an
//  instrument whose whole job is to be read the same way every time.
//  The control sits above it because that is where both strips can see
//  it, not because it applies to it.
//

import SwiftUI

/// Shared zoom for the two playing waveforms.
///
/// The renderer's `timeAxisZoom` is seconds-per-viewport: values above
/// 1.0 fit *more* audio, so zooming in walks the ladder downward. The
/// steps are ratios rather than a continuous slider because this is a
/// glance-and-press control on a performance surface — you want the
/// same view back, not a value you have to re-find.
enum WaveformZoom {
    /// Stops at 2.0 because that is where the renderer's one-pixel
    /// column floor lands with a 2 px base — a rung past it would
    /// redraw the same picture and read as a broken button.
    static let steps: [Double] = [0.25, 0.35, 0.5, 0.7, 1.0, 1.4, 2.0]

    /// `1.0` — the shipped Performance scale.
    static let defaultIndex = 4

    static func label(_ index: Int) -> String {
        let z = steps[min(max(index, 0), steps.count - 1)]
        return z == 1.0 ? "1×" : String(format: "%.2g×", 1.0 / z)
    }
}

/// `−` and `+`, shown while the pointer is over the deck row.
struct WaveformZoomControl: View {
    @Binding var index: Int
    /// Fades rather than appears: a control that pops in under the
    /// pointer reads as a misclick waiting to happen.
    let visible: Bool

    var body: some View {
        HStack(spacing: DubSpacing.xs) {
            button("minus", enabled: index < WaveformZoom.steps.count - 1) {
                index = min(index + 1, WaveformZoom.steps.count - 1)
            }
            Text(WaveformZoom.label(index))
                .font(DubFont.micro)
                .monospacedDigit()
                .foregroundStyle(DubColor.textSecondary)
                .frame(minWidth: 30)
            button("plus", enabled: index > 0) {
                index = max(index - 1, 0)
            }
        }
        .padding(.horizontal, DubSpacing.sm)
        .padding(.vertical, 4)
        .background(DubColor.surface2)
        .clipShape(Capsule())
        .overlay(Capsule().stroke(DubColor.divider, lineWidth: 1))
        .opacity(visible ? 1 : 0)
        // Untappable while hidden, or an invisible control still takes
        // the click that was meant for the strip behind it.
        .allowsHitTesting(visible)
        .animation(.easeOut(duration: 0.12), value: visible)
        .accessibilityLabel("Waveform zoom")
    }

    private func button(
        _ symbol: String, enabled: Bool, action: @escaping () -> Void
    ) -> some View {
        Image(systemName: symbol)
            .font(.system(size: 9, weight: .bold))
            .foregroundStyle(enabled ? DubColor.textPrimary : DubColor.textPlaceholder)
            .frame(width: 18, height: 16)
            .contentShape(Rectangle())
            .onPressDown(enabled: enabled, perform: action)
            .help(symbol == "plus" ? "Zoom in — less audio, more detail"
                                   : "Zoom out — more audio")
    }
}
