//
//  DubKeycap.swift
//  Dub
//
//  The key binding printed on a control.
//
//  This replaces a bare `Text(key).font(DubFont.micro)` in
//  `DubColor.textPlaceholder` — the token reserved for placeholder
//  em-dashes — which measured **2.20:1** on `surface1`. The keyboard is
//  the performance input path (PRD §5.5); it was the least legible thing
//  on the surface.
//
//  The fix is not "make the text brighter". A binding brightened toward
//  `textPrimary` competes with the preset name beside it, and on a pad
//  the name has to win. So contrast comes from the *ground* instead: the
//  cap is a `surface0` well punched into the pad, darker than what it sits
//  on, with the glyph at `textSecondary`. It reads as a component rather
//  than as a second label.
//
//  House rule this establishes: **a binding is never dimmer than
//  `textSecondary` anywhere in the app.** An unbound slot is the one
//  exception, and it says so by losing the glyph rather than by fading.
//

import SwiftUI

/// One key binding, drawn as a recessed cap.
struct DubKeycap: View {
    /// The glyph as the DJ reads it on their keyboard — `Z`, `,`, `⇧1`.
    let key: String

    /// `false` when the action has no key wired to it yet — a slot the
    /// keymap reserves ahead of the M18 remap pass. The cap still
    /// renders, so the slot is visibly reserved rather than silently
    /// missing, but it does not advertise a key that would do nothing.
    /// Nothing is reserved today; the state stays for map mode.
    var bound: Bool = true

    /// Lifts the glyph to `textPrimary` while the pad it sits on is lit,
    /// so a firing pad's binding stays legible against the tint wash.
    var lit: Bool = false

    /// Where the cap sits. On a pad it is a corner mark and stays small;
    /// in a list (the Preferences Keys tab) it is the row's value and
    /// gets a real cap's proportions — `SPACE` with air around it.
    enum Size {
        case pad
        case list
    }
    var size: Size = .pad

    private var glyphColor: Color {
        guard bound else { return DubColor.textPlaceholder }
        return lit ? DubColor.textPrimary : DubColor.textSecondary
    }

    private var pointSize: CGFloat { size == .pad ? 9 : 11 }
    private var minWidth: CGFloat {
        switch size {
        case .pad: return key.count > 1 ? 22 : 16
        case .list: return 26
        }
    }
    private var minHeight: CGFloat { size == .pad ? 14 : 22 }
    private var inset: CGFloat { size == .pad ? 0 : 8 }

    var body: some View {
        Text(bound ? key : "—")
            .font(.system(size: pointSize, weight: .bold, design: .monospaced))
            .foregroundStyle(glyphColor)
            .padding(.horizontal, inset)
            .frame(minWidth: minWidth, minHeight: minHeight)
            .background(DubColor.surface0)
            .clipShape(RoundedRectangle(cornerRadius: size == .pad ? 3 : 4, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: size == .pad ? 3 : 4, style: .continuous)
                    .stroke(
                        bound ? DubColor.divider : DubColor.divider.opacity(0.5),
                        lineWidth: 1))
            .accessibilityLabel(bound ? "Key \(key)" : "No key bound")
    }
}
