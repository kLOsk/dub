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
    /// Quarter-step magnifications, 0.25x out to 2x in.
    ///
    /// Stored as `zoom` (seconds-per-viewport), so the ladder reads
    /// backwards from the labels: `zoom = 1 / magnification`. A drawn
    /// column is `2 / zoom` device pixels — here twice the
    /// magnification, except at the bottom rung (below).
    ///
    /// An earlier ladder used only whole-pixel column pitches, on the
    /// theory that fractional ones caused the beat-grid flicker. They
    /// did not: that was a missing factor of 2 in the renderer's
    /// pixel-to-NDC conversion, which drew every line at half its
    /// intended width and pushed beat ticks into the sub-pixel range
    /// where the shader's analytic AA degenerates. With that fixed the
    /// constraint went away, so the ladder is the one a DJ would ask
    /// for rather than the one the rasteriser preferred. MSAA 4x
    /// resolves a half-pixel column into honest coverage.
    ///
    /// **`0.25x` is currently inert.** `effectivePixelsPerDrawnColumn`
    /// floors a drawn column at one device pixel, and `0.5x` already
    /// sits on that floor — so `0.25x` asks for half a pixel per
    /// column, gets one, and draws the identical picture. Showing
    /// twice the audio past this point needs each column to *aggregate*
    /// twice the peak chunks (`chunksPerColumn`, currently pinned at 2)
    /// rather than occupy fewer pixels. The shader already loops over
    /// that uniform, so the work is on the Swift side: the snap
    /// quantum, the drawn-column counts and the grid's time->NDC
    /// mapping all derive from it.
    static let steps: [Double] = [
        0.5,         // 2x    — 4 px per column
        2.0 / 3.0,   // 1.5x  — 3
        0.8,         // 1.25x — 2.5
        1.0,         // 1x    — 2
        4.0 / 3.0,   // 0.75x — 1.5
        2.0,         // 0.5x  — 1
        4.0,         // 0.25x — 1 (floored; see above)
    ]

    /// `1.0` — the shipped Performance scale.
    static let defaultIndex = 3

    /// Two decimals, trailing zeros trimmed: 2x, 1.75x, 1.5x, 1.25x,
    /// 1x, 0.75x, 0.5x. `%.2g` was fine for the old whole-and-thirds
    /// ladder but rounds to two *significant* digits, which turns
    /// 1.75 into "1.8" and 1.25 into "1.2".
    static func label(_ index: Int) -> String {
        let z = steps[min(max(index, 0), steps.count - 1)]
        var text = String(format: "%.2f", 1.0 / z)
        while text.hasSuffix("0") { text.removeLast() }
        if text.hasSuffix(".") { text.removeLast() }
        return text + "×"
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
                .frame(minWidth: 46)
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
