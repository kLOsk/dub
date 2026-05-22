#!/usr/bin/env swift
//
// Render the Dub AppIcon master PNG clipped to a rounded rectangle.
//
// Ad-hoc / locally-built apps often display a legacy square .icns in the
// Dock without the system mask. We clip to straight top/bottom/sides with
// corner-only rounding (System Settings / WhatsApp-style), not the full
// squircle superellipse which reads as "too round" in the Dock tile.
//
// Usage:
//   swift scripts/render-macos-icon.swift [output-path]
//
// Default output: apple/Dub/Assets.xcassets/AppIcon.appiconset/AppIcon-1024.png

import AppKit
import CoreGraphics

let repoRoot = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent()
    .deletingLastPathComponent()

let defaultOut = repoRoot
    .appendingPathComponent("apple/Dub/Assets.xcassets/AppIcon.appiconset/AppIcon-1024.png")

let outURL: URL = {
    if CommandLine.arguments.count > 1 {
        return URL(fileURLWithPath: CommandLine.arguments[1])
    }
    return defaultOut
}()

let canvas = 1024
let colorSpace = CGColorSpace(name: CGColorSpace.sRGB)!

/// Rounded rect mask: flat top/bottom/sides, corners only (~15.5 % radius).
func macDockIconShape(in rect: CGRect) -> CGPath {
    let radius = rect.width * 0.155
    return CGPath(
        roundedRect: rect,
        cornerWidth: radius,
        cornerHeight: radius,
        transform: nil)
}

guard let ctx = CGContext(
    data: nil,
    width: canvas,
    height: canvas,
    bitsPerComponent: 8,
    bytesPerRow: 0,
    space: colorSpace,
    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
) else {
    fputs("error: failed to create bitmap context\n", stderr)
    exit(1)
}

ctx.clear(CGRect(x: 0, y: 0, width: canvas, height: canvas))

// CoreGraphics origin is bottom-left; flip once so geometry matches UI coords.
ctx.translateBy(x: 0, y: CGFloat(canvas))
ctx.scaleBy(x: 1, y: -1)

let iconRect = CGRect(x: 0, y: 0, width: canvas, height: canvas)
ctx.addPath(macDockIconShape(in: iconRect))
ctx.clip()

let center = CGPoint(x: CGFloat(canvas) / 2, y: CGFloat(canvas) / 2)

// ----- Background: warm stage black with radial falloff -----------------

let bgColors = [
    CGColor(srgbRed: 0.07, green: 0.075, blue: 0.085, alpha: 1),
    CGColor(srgbRed: 0.04, green: 0.042, blue: 0.048, alpha: 1),
    CGColor(srgbRed: 0.015, green: 0.016, blue: 0.018, alpha: 1),
] as CFArray
let bgLocations: [CGFloat] = [0, 0.62, 1]

if let bgGradient = CGGradient(
    colorsSpace: colorSpace,
    colors: bgColors,
    locations: bgLocations
) {
    ctx.drawRadialGradient(
        bgGradient,
        startCenter: center,
        startRadius: 0,
        endCenter: center,
        endRadius: CGFloat(canvas) * 0.78,
        options: .drawsAfterEndLocation)
}

// Soft top specular — the Big Sur icon lighting cue.
let specColors = [
    CGColor(srgbRed: 1, green: 1, blue: 1, alpha: 0.07),
    CGColor(srgbRed: 1, green: 1, blue: 1, alpha: 0),
] as CFArray
if let specGradient = CGGradient(
    colorsSpace: colorSpace,
    colors: specColors,
    locations: [0, 1]
) {
    ctx.drawLinearGradient(
        specGradient,
        start: CGPoint(x: center.x, y: 0),
        end: CGPoint(x: center.x, y: CGFloat(canvas) * 0.55),
        options: [])
}

// ----- Vinyl motif (matches Dub splash / deck icon language) ----------

let ringRadius: CGFloat = 268
let stroke: CGFloat = 5.5
let ringRect = CGRect(
    x: center.x - ringRadius,
    y: center.y - ringRadius,
    width: ringRadius * 2,
    height: ringRadius * 2)

ctx.setFillColor(CGColor(srgbRed: 0.11, green: 0.12, blue: 0.135, alpha: 1))
ctx.fillEllipse(in: ringRect.insetBy(dx: stroke, dy: stroke))

ctx.setStrokeColor(CGColor(srgbRed: 0.92, green: 0.93, blue: 0.94, alpha: 1))
ctx.setLineWidth(stroke)
ctx.strokeEllipse(in: ringRect)

ctx.setLineCap(.round)
ctx.move(to: CGPoint(x: center.x, y: center.y - ringRadius))
ctx.addLine(to: CGPoint(x: center.x, y: center.y))
ctx.strokePath()

let spindleRadius: CGFloat = 15
ctx.setFillColor(CGColor(srgbRed: 0.769, green: 0.569, blue: 0.341, alpha: 1))
ctx.fillEllipse(in: CGRect(
    x: center.x - spindleRadius,
    y: center.y - spindleRadius,
    width: spindleRadius * 2,
    height: spindleRadius * 2))

ctx.setFillColor(CGColor(srgbRed: 1, green: 1, blue: 1, alpha: 0.22))
ctx.fillEllipse(in: CGRect(
    x: center.x - spindleRadius * 0.35,
    y: center.y - spindleRadius * 0.55,
    width: spindleRadius * 0.55,
    height: spindleRadius * 0.45))

guard let image = ctx.makeImage() else {
    fputs("error: failed to rasterize icon\n", stderr)
    exit(1)
}

let rep = NSBitmapImageRep(cgImage: image)
guard let png = rep.representation(using: .png, properties: [:]) else {
    fputs("error: failed to encode PNG\n", stderr)
    exit(1)
}

do {
    try png.write(to: outURL)
    print("wrote \(outURL.path)")
} catch {
    fputs("error: \(error)\n", stderr)
    exit(1)
}
