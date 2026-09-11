//
//  DubRecordGlyph.swift
//  Dub
//
//  A 12" record, small enough to be a pad.
//
//  The Quick Scratch pads are records — that is what the control does,
//  it puts one on the deck — and a record is the one glyph a DVS DJ
//  reads without a legend. Drawn to a 12"'s proportions: the label is
//  a third of the disc (a 4" label on an 11.8" pressing), and a sticker
//  rides the label's edge, the mark a DJ puts on a record to see where
//  it is. On the deck, the sticker's angle is the sample's playhead at
//  33⅓ rpm — 1.8 s of audio per revolution — so a scratch reads as the
//  sticker running back and forth, and a paused deck holds still. It is
//  not a decoration; it is the platter.
//

import SwiftUI

struct DubRecordGlyph: View {
    enum Look: Equatable {
        /// No sample on this pad: a dashed outline where a record goes.
        case empty
        /// A sample tagged to the pad, on the shelf.
        case shelved
        /// The sample is on the deck.
        case onDeck
    }

    let look: Look
    /// The deck's colour; the label takes it on the deck.
    let tint: Color
    /// The sticker's angle — the platter's rotation. Clockwise from
    /// twelve o'clock, as a record turns seen from above.
    var angle: Angle = .zero

    /// Seconds of audio per revolution at 33⅓ rpm.
    static let secondsPerRevolution: Double = 1.8

    /// The platter's angle for a playhead at `secs`.
    static func angle(forElapsedSecs secs: Double) -> Angle {
        guard secs.isFinite else { return .zero }
        return .degrees(secs / secondsPerRevolution * 360)
    }

    var body: some View {
        Canvas { ctx, size in
            let r = min(size.width, size.height) / 2
            let c = CGPoint(x: size.width / 2, y: size.height / 2)
            let disc = Path(ellipseIn: CGRect(x: c.x - r + 0.5, y: c.y - r + 0.5, width: 2 * r - 1, height: 2 * r - 1))

            if look == .empty {
                ctx.stroke(disc, with: .color(DubColor.divider), style: StrokeStyle(lineWidth: 1, dash: [3, 3]))
                ctx.fill(Path(ellipseIn: CGRect(x: c.x - 1, y: c.y - 1, width: 2, height: 2)), with: .color(DubColor.divider))
                return
            }

            ctx.fill(disc, with: .color(DubColor.surface0))
            ctx.stroke(disc, with: .color(look == .onDeck ? tint : DubColor.divider), lineWidth: 1)
            // Grooves: three faint rings between the edge and the label.
            for f in [0.82, 0.64, 0.48] {
                let gr = r * f
                ctx.stroke(
                    Path(ellipseIn: CGRect(x: c.x - gr, y: c.y - gr, width: 2 * gr, height: 2 * gr)),
                    with: .color(DubColor.surface2), lineWidth: 1)
            }
            // The label: a third of the disc. The sticker sits on its edge
            // and turns with it.
            let lr = r * 0.34
            ctx.fill(
                Path(ellipseIn: CGRect(x: c.x - lr, y: c.y - lr, width: 2 * lr, height: 2 * lr)),
                with: .color(look == .onDeck ? tint : DubColor.textTertiary))
            ctx.fill(
                Path(ellipseIn: CGRect(x: c.x - r * 0.05, y: c.y - r * 0.05, width: r * 0.1, height: r * 0.1)),
                with: .color(DubColor.surface0))

            var sticker = ctx
            sticker.translateBy(x: c.x, y: c.y)
            sticker.rotate(by: angle)
            let sw = r * 0.17, sh = r * 0.14
            sticker.fill(
                Path(roundedRect: CGRect(x: -sw / 2, y: -lr, width: sw, height: sh), cornerRadius: sw * 0.2),
                with: .color(DubColor.textPrimary))
        }
    }
}
