//
//  PlayheadMarker.swift
//  Dub
//
//  Prominent playhead indicator for the zoomed waveform and idle
//  deck panes. Neutral cream core + dark halo + inward edge
//  chevrons — deliberately unlike the deck-tinted 1 px beat-grid
//  ticks (35 % / 85 % opacity) so the "now playing" position reads
//  at a glance during prep and performance.
//

import SwiftUI

struct PlayheadMarker: View {

    let orientation: WaveformOrientation
    let size: CGSize
    var axisFraction: CGFloat = CGFloat(WaveformRenderer.pastRegionFraction)
    /// Idle empty-deck panes use a softer marker so hint text stays
    /// primary; loaded decks use full-strength chrome.
    var subdued: Bool = false

    var body: some View {
        Canvas { ctx, canvasSize in
            let accent = DubColor.playheadAccent.opacity(subdued ? 0.45 : 1.0)
            let haloColor = Color.black.opacity(subdued ? 0.28 : 0.58)
            let coreW = DubLayout.playheadCoreWidth
            let haloW = DubLayout.playheadHaloWidth
            let chevron = DubLayout.playheadChevronSize

            switch orientation {
            case .vertical:
                let y = canvasSize.height * axisFraction
                let halo = CGRect(
                    x: 0, y: y - haloW * 0.5,
                    width: canvasSize.width, height: haloW)
                ctx.fill(Path(halo), with: .color(haloColor))
                let core = CGRect(
                    x: 0, y: y - coreW * 0.5,
                    width: canvasSize.width, height: coreW)
                ctx.fill(Path(core), with: .color(accent))
                ctx.fill(leftChevronVertical(y: y, size: chevron, width: canvasSize.width),
                         with: .color(accent))
                ctx.fill(rightChevronVertical(y: y, size: chevron, width: canvasSize.width),
                         with: .color(accent))
            case .horizontal:
                let x = canvasSize.width * axisFraction
                let halo = CGRect(
                    x: x - haloW * 0.5, y: 0,
                    width: haloW, height: canvasSize.height)
                ctx.fill(Path(halo), with: .color(haloColor))
                let core = CGRect(
                    x: x - coreW * 0.5, y: 0,
                    width: coreW, height: canvasSize.height)
                ctx.fill(Path(core), with: .color(accent))
                ctx.fill(topChevronHorizontal(x: x, size: chevron, height: canvasSize.height),
                         with: .color(accent))
                ctx.fill(bottomChevronHorizontal(x: x, size: chevron, height: canvasSize.height),
                         with: .color(accent))
            }
        }
        .frame(width: size.width, height: size.height)
        .allowsHitTesting(false)
    }

    private func leftChevronVertical(y: CGFloat, size: CGFloat, width: CGFloat) -> Path {
        Path { p in
            p.move(to: CGPoint(x: 0, y: y - size))
            p.addLine(to: CGPoint(x: size, y: y))
            p.addLine(to: CGPoint(x: 0, y: y + size))
            p.closeSubpath()
        }
    }

    private func rightChevronVertical(y: CGFloat, size: CGFloat, width: CGFloat) -> Path {
        Path { p in
            p.move(to: CGPoint(x: width, y: y - size))
            p.addLine(to: CGPoint(x: width - size, y: y))
            p.addLine(to: CGPoint(x: width, y: y + size))
            p.closeSubpath()
        }
    }

    private func topChevronHorizontal(x: CGFloat, size: CGFloat, height: CGFloat) -> Path {
        Path { p in
            p.move(to: CGPoint(x: x - size, y: 0))
            p.addLine(to: CGPoint(x: x, y: size))
            p.addLine(to: CGPoint(x: x + size, y: 0))
            p.closeSubpath()
        }
    }

    private func bottomChevronHorizontal(x: CGFloat, size: CGFloat, height: CGFloat) -> Path {
        Path { p in
            p.move(to: CGPoint(x: x - size, y: height))
            p.addLine(to: CGPoint(x: x, y: height - size))
            p.addLine(to: CGPoint(x: x + size, y: height))
            p.closeSubpath()
        }
    }
}
