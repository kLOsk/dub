//
//  DubPadCell.swift
//  Dub
//
//  The one pad/chip chrome. Before this type there were five
//  near-identical private recipes scattered through
//  PerformancePadsView — hot-cue pad, echo-out button, siren preset,
//  FX-rack slot label, echo-cut chip — each re-deriving the same
//  rounded rect, the same lit/unlit fill and stroke, and the same
//  semibold rounded type, differing only in size. They drifted, and
//  none of them could be reused outside that file because all five
//  were fileprivate.
//
//  `Size` reproduces each former recipe's metrics exactly, so adopting
//  this type is a pixel-identical substitution — which is what lets the
//  snapshot baselines stay green through the migration and act as the
//  correctness oracle for it.
//
//  **This deliberately does not wrap `Button`.** Callers attach
//  `.onPressDown` / `.onPressHold` themselves. Mouse-*down* semantics
//  are load-bearing on a performance surface: a cue handler captures
//  the live playhead at the instant of the press, and `Button`'s
//  mouse-up firing would sample it late. The rule is "pads never use
//  `Button`", and this type keeps that the caller's decision.
//

import SwiftUI

/// One pad or chip: `label` centred on the shared rounded-rect chrome,
/// filled and stroked in `tint` when `lit`.
struct DubPadCell<Label: View>: View {
    /// Pad metrics. Each case is one of the five recipes this type
    /// replaced; the numbers are theirs, unchanged.
    enum Size {
        /// 38 × 36 — a single glyph: cue numbers, loop lengths, ✕.
        case glyph
        /// 50 × 36 — a short word that will not fit `glyph`: IN, OUT.
        case word
        /// 64 × 36 — a siren preset, name over keycap.
        case preset
        /// 92 × 32 — an FX-rack slot label.
        case slot
        /// 92 × 28 — a momentary chip: ECHO CUT.
        case chip
        /// Height 36, width hugging its label plus `DubSpacing.lg` each
        /// side — the ECHO OUT button.
        case hugging

        var width: CGFloat? {
            switch self {
            case .glyph: return 38
            case .word: return 50
            case .preset: return 64
            case .slot, .chip: return 92
            case .hugging: return nil
            }
        }

        var height: CGFloat {
            switch self {
            case .glyph, .word, .preset, .hugging: return 36
            case .slot: return 32
            case .chip: return 28
            }
        }

        var horizontalPadding: CGFloat {
            self == .hugging ? DubSpacing.lg : 0
        }

        /// Default type for the `Text` convenience initialiser.
        var font: Font {
            switch self {
            case .glyph, .word: return .system(size: 13, weight: .semibold, design: .rounded)
            case .hugging: return .system(size: 12, weight: .semibold, design: .rounded)
            case .preset, .slot, .chip:
                return .system(size: 11, weight: .semibold, design: .rounded)
            }
        }
    }

    let size: Size
    var lit: Bool = false
    var tint: Color = DubColor.hotCue
    @ViewBuilder let label: () -> Label

    var body: some View {
        label()
            .foregroundStyle(lit ? DubColor.textPrimary : DubColor.textTertiary)
            .frame(width: size.width, height: size.height)
            .padding(.horizontal, size.horizontalPadding)
            .background(lit ? tint.opacity(0.24) : DubColor.surface1)
            .clipShape(RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: DubRadius.panel, style: .continuous)
                    .stroke(lit ? tint : DubColor.divider, lineWidth: 1))
            .contentShape(Rectangle())
    }
}

extension DubPadCell where Label == Text {
    /// A plain single-line pad, typed in the size's default font.
    init(
        _ glyph: String,
        size: Size,
        lit: Bool = false,
        tint: Color = DubColor.hotCue
    ) {
        self.init(size: size, lit: lit, tint: tint) {
            Text(glyph).font(size.font)
        }
    }
}
