//
//  DubSegmentedControl.swift
//  Dub
//
//  The house segmented control: a capsule track with the selected
//  segment filled in the deck tint.
//
//  Two things make this a real type rather than a style on AppKit's
//  `Picker`. It fires on `.onPressDown`, which an AppKit `Picker`
//  cannot do and which every other control on a performance surface
//  does. And it paints with `DubColor.deckTint`, so a switch reads as
//  belonging to the same deck as the INT · TC · THRU pill above it.
//
//  The known trade-off, already accepted for SOURCE and KEY LOCK: the
//  hand-rolled control has no keyboard focus ring. These are Prep and
//  configuration surfaces, where the mouse is the intended input.
//
//  Deliberately NOT adopted by two existing controls:
//
//  * `SourceControlView.segment` — its track carries an `Image`
//    play/pause glyph, not just text. Generalising for one caller means
//    either a `@ViewBuilder` slot on every segment or a pixel shift
//    that re-records three baselines, for nothing.
//  * `StatusStrip.modeSegment` — different chrome entirely
//    (per-segment capsules on `surface3`, no shared track). Not the
//    same control wearing a different hat.
//

import SwiftUI

/// A capsule segmented switch over `Value`.
struct DubSegmentedControl<Value: Hashable>: View {
    /// How a segment claims its width. Both cases exist because both
    /// were in use: KEY LOCK's `OFF`/`ON` hug their text, while
    /// PITCH %'s seven numeric steps need a floor so `0` and `+10` come
    /// out the same size.
    enum Width: Equatable {
        /// Hug the label, `DubSpacing.sm` each side.
        case padded
        /// At least this wide, no horizontal padding.
        case minWidth(CGFloat)
    }

    struct Segment: Identifiable {
        let value: Value
        let title: String
        var width: Width = .padded
        var id: Value { value }

        init(_ value: Value, _ title: String, width: Width = .padded) {
            self.value = value
            self.title = title
            self.width = width
        }
    }

    let segments: [Segment]
    let selection: Value
    /// Fill for the selected segment — normally `DubColor.deckTint(side)`.
    var tint: Color
    let onSelect: (Value) -> Void

    var body: some View {
        HStack(spacing: 0) {
            ForEach(segments) { segment in
                let active = segment.value == selection
                // `foregroundStyle` sits outside `sized` deliberately:
                // the `Text`-returning overload is macOS 14+, and the
                // padding it now wraps renders nothing coloured, so the
                // result is identical on the macOS 13 `View` overload.
                sized(
                    Text(segment.title)
                        .font(DubFont.caps)
                        .tracking(DubFont.controlTracking),
                    segment.width)
                    .foregroundStyle(active ? DubColor.surface0 : DubColor.textSecondary)
                    .padding(.vertical, 3)
                    .background(active ? tint : Color.clear)
                    .onPressDown { onSelect(segment.value) }
                    .accessibilityAddTraits(.isButton)
            }
        }
        .background(DubColor.surface2)
        .clipShape(Capsule())
        .overlay(Capsule().stroke(DubColor.divider, lineWidth: 1))
    }

    @ViewBuilder
    private func sized(_ label: Text, _ width: Width) -> some View {
        switch width {
        case .padded:
            label.padding(.horizontal, DubSpacing.sm)
        case let .minWidth(minimum):
            label.frame(minWidth: minimum)
        }
    }
}
