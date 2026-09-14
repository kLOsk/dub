//
//  DubKey.swift
//  Dub
//
//  A key — the pad drawn as what it is on the siren box: the keyboard
//  key that fires it. Letter in the corner, the sound engraved on the
//  cap, and a lip under the cap that makes it a key rather than a tile.
//
//  `DubPadCell` is the tile: a flat rounded rect that lights. The rack
//  bar had eight of them beside the sample shelf's eight, and a hot-cue
//  grid of eight above — three rows of the same silhouette. The lip is
//  the whole difference: 4 pt of shadow at rest, 2 pt with the cap sunk
//  when the key is down, so a press *moves* instead of only tinting.
//
//  Like `DubPadCell` this does not wrap `Button`; the caller attaches
//  `.onPressDown` so the shot fires on mouse-*down*.
//

import SwiftUI

/// One key on a faceplate: `legend` in the corner, `title` on the cap.
struct DubKey: View {
    /// The sound engraved on the cap — `RIFLE GUN`, wrapping to two
    /// lines when it must.
    let title: String
    /// The key's own letter, printed in the top-left corner. Empty when
    /// nothing is bound; the corner is then left blank rather than
    /// promising a key that does nothing.
    var legend: String = ""
    /// The shot is sounding: the cap sinks and lights.
    var down: Bool = false
    var tint: Color = DubColor.siren

    private var lip: CGFloat { down ? 2 : 4 }

    var body: some View {
        ZStack(alignment: .topLeading) {
            RoundedRectangle(cornerRadius: 5, style: .continuous)
                .fill(down ? tint.opacity(0.22) : DubColor.surface2)
            RoundedRectangle(cornerRadius: 5, style: .continuous)
                .stroke(down ? tint : DubColor.plateEdge, lineWidth: 1)
            Text(title.uppercased())
                .font(.system(size: 10, weight: .semibold, design: .default))
                .tracking(0.3)
                .multilineTextAlignment(.center)
                .lineLimit(2)
                .minimumScaleFactor(0.85)
                .foregroundStyle(down ? DubColor.textPrimary : DubColor.textSecondary)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .padding(EdgeInsets(top: 10, leading: 4, bottom: 2, trailing: 4))
            if !legend.isEmpty {
                Text(legend)
                    .font(.system(size: 8, weight: .bold, design: .monospaced))
                    .foregroundStyle(down ? DubColor.textPrimary : DubColor.textTertiary)
                    .padding(.top, 3)
                    .padding(.leading, 5)
            }
        }
        .frame(width: DubLayout.sirenKeyWidth, height: DubLayout.sirenKeyHeight - lip)
        .background(alignment: .bottom) {
            // The lip: the cap's shadow, drawn under it so the key's
            // footprint stays `sirenKeyHeight` whether it is up or down.
            RoundedRectangle(cornerRadius: 5, style: .continuous)
                .fill(DubColor.keyLip)
                .frame(height: DubLayout.sirenKeyHeight)
                .offset(y: lip)
        }
        .offset(y: down ? 2 : 0)
        .frame(width: DubLayout.sirenKeyWidth, height: DubLayout.sirenKeyHeight)
        .contentShape(Rectangle())
        .animation(.easeOut(duration: 0.06), value: down)
        .accessibilityLabel(legend.isEmpty ? title : "\(title), key \(legend)")
    }
}
