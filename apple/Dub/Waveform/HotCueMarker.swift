//
//  HotCueMarker.swift
//  Dub
//
//  One hot cue as the waveform draws it: where, and in what colour.
//
//  The renderer took bare `[Double]` positions and painted every line in
//  one fixed magenta. Once a cue carries a user colour, that stops being
//  acceptable — a mark is a single object, and it should not change
//  identity between the pad you set it on and the line you see it at.
//

import SwiftUI
import simd

/// A hot cue positioned in track seconds, with the colour to draw it in.
struct HotCueMarker: Equatable {
    /// Track position, seconds from sample 0.
    var secs: Double
    /// sRGB components, 0…1. Alpha is the renderer's to choose — it
    /// draws cue lines at 0.95 regardless of the label colour.
    var rgb: SIMD3<Float>

    init(secs: Double, rgb: SIMD3<Float>) {
        self.secs = secs
        self.rgb = rgb
    }

    /// Build from a colour-label token (`"aqua"`, `"red"`, …). An
    /// unlabelled cue falls back to `DubColor.hotCue`, so "no colour
    /// chosen" looks like the cue accent rather than like black.
    init(secs: Double, colorToken: String?) {
        self.secs = secs
        self.rgb = Self.components(DubColor.trackLabel(colorToken) ?? DubColor.hotCue)
    }

    /// Resolve a SwiftUI `Color` to sRGB floats.
    ///
    /// Converted here rather than in the renderer so the Metal layer
    /// never imports the design system, and converted through `NSColor`
    /// rather than by restating hexes so the palette has one definition
    /// (`DubColor.trackLabelPalette`) that cannot drift from this.
    static func components(_ color: Color) -> SIMD3<Float> {
        guard let srgb = NSColor(color).usingColorSpace(.sRGB) else {
            // The cue accent, matching `WaveformRenderer.hotCueRGBA`.
            return SIMD3(250.0 / 255.0, 92.0 / 255.0, 158.0 / 255.0)
        }
        return SIMD3(
            Float(srgb.redComponent),
            Float(srgb.greenComponent),
            Float(srgb.blueComponent))
    }
}
