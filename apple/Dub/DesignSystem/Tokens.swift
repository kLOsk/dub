//
//  Tokens.swift
//  Dub
//
//  M10.3 design tokens — the single source of truth for app colour,
//  typography, spacing, and corner radii. Mirrors the `00 Tokens`
//  page in the Figma exploration sandbox, but **code wins** when the
//  two disagree (see PRD §9 workflow note).
//
//  Why an enum-namespaced constant set instead of `Color(.token)`
//  asset catalogues:
//
//  1. Asset catalogues need Xcode UI to edit; tokens live next to the
//     code that uses them and are diff-friendly.
//  2. We can express derived values (e.g. `DubColor.deckTint(.a)`) as
//     plain functions, which a catalogue can't.
//  3. Snapshot tests (M18) will diff token *values*, not asset names.
//
//  Convention: every name is qualified (`DubColor.surface0`, never
//  bare `surface0`) so reading code requires no import knowledge.
//

import SwiftUI

// MARK: - Colour

/// Dark-mode-only neutral + accent palette.
///
/// Dub is a performance tool used in dim rooms; we don't ship a
/// light mode. `Color` values are `sRGB` with no alpha unless a
/// `*Alpha*` suffix is present.
enum DubColor {

    // ----- Neutrals -------------------------------------------------
    // The "surface" ramp is the workhorse for backgrounds. Each step
    // is ~6 % L* brighter than the previous in OKLCH, so adjacent
    // steps are distinguishable on a calibrated screen but never
    // jarring side-by-side. Picked to avoid the "computer dashboard"
    // greyscale that plagues most DJ apps.

    /// Window background. Slightly warm-black, so the screen reads
    /// as "stage" rather than "spreadsheet."
    static let surface0 = Color(hex: 0x0B0C0F)

    /// Deck header / status strip background.
    static let surface1 = Color(hex: 0x14161B)

    /// Inner panel — FX bar, library frame.
    static let surface2 = Color(hex: 0x1B1E24)

    /// Hover / selected row.
    static let surface3 = Color(hex: 0x252932)

    /// Hairline divider on a `surface0` background.
    static let divider = Color(hex: 0x2B2F38)

    // ----- Text -----------------------------------------------------

    /// Track titles, numeric BPM, primary buttons.
    static let textPrimary = Color(hex: 0xE6E8EC)

    /// Artist, metadata captions, units (`BPM`, `KEY`, `PITCH`).
    static let textSecondary = Color(hex: 0x9099A3)

    /// Tertiary detail (timestamps, format codecs).
    static let textTertiary = Color(hex: 0x6B7079)

    /// Placeholder em-dash content (`—`) before a feature lands.
    static let textPlaceholder = Color(hex: 0x4A4F58)

    // ----- Deck accents --------------------------------------------
    // Deliberately *muted* — the M10.2 waveform palettes carry the
    // chromatic load. Header / chip accents only need to disambiguate
    // deck A vs B at a glance, not compete with the waveform colour.
    // Final accent hue is a M18 polish decision; these are the
    // committed M10.3 starting values.

    static let deckATint = Color(hex: 0xC49157)
    static let deckBTint = Color(hex: 0x5A8088)

    /// Pick a deck's accent by deck index. Asserts in debug for
    /// out-of-range indices so misuse fails loudly instead of
    /// silently falling back to deck A.
    static func deckTint(_ deck: DeckSide) -> Color {
        switch deck {
        case .a: return deckATint
        case .b: return deckBTint
        }
    }

    // ----- State ---------------------------------------------------

    /// "OK / locked / live" indicator.
    static let stateLocked = Color(hex: 0x6FB04A)

    /// "Tentative / searching" indicator.
    static let stateTentative = Color(hex: 0xD9A33A)

    /// Clip / error / destructive.
    static let stateError = Color(hex: 0xD45C5C)

    /// Hot cue marker / pad accent. Vivid magenta-pink, hue- AND
    /// luminance-distinct from both deck tints (amber, teal) and the
    /// off-white beat ticks, so a cue reads clearly on any surface
    /// regardless of deck. Mirrors `WaveformRenderer.hotCueRGBA`
    /// (250, 92, 158) so the pads, the whole-track overview, and the
    /// zoomed Metal waveform all render a cue in the same colour.
    static let hotCue = Color(hex: 0xFA5C9E)

    /// Loop region / pad accent. A bright mint-green, the DJ-software
    /// convention for an active loop (Traktor / Serato), and distinct
    /// from the cue magenta and both deck tints so a loop reads as its
    /// own thing on the pads + overview + waveform band.
    static let loop = Color(hex: 0x4FD18B)

    /// Echo-out FX / pad accent (M15, PRD §6.3). A bright violet — hue-
    /// distinct from the loop mint, the cue magenta, and both deck tints
    /// (amber, teal) — so a glowing echo pad reads as its own thing while
    /// the dub echo is engaged or ringing out.
    static let echo = Color(hex: 0xB07CFF)

    /// Dub-siren FX / pad accent (M16, PRD §6.3). A hot siren red — hue-
    /// distinct from the echo violet, the loop mint, the cue magenta, and
    /// both deck tints — so the SIREN button and preset pads read as the
    /// siren's own thing while it's wailing.
    static let siren = Color(hex: 0xFF5247)

    // ----- Vintage-FX rack accents (PRD §6.3) ----------------------
    // The dub processing chain. Each slot gets a hue-distinct accent so an
    // engaged rack effect reads as its own thing on the pads.

    /// Spring reverb (the dub tank). A watery teal-cyan.
    static let springFx = Color(hex: 0x4FD1C5)
    /// Roland RE-201 Space Echo. A warm tape amber.
    static let spaceEcho = Color(hex: 0xF5A623)
    /// King Tubby "Big Knob" high-pass. A cool filter blue.
    static let bigKnob = Color(hex: 0x5B8DEF)
    /// Mu-Tron Bi-Phase phaser. A swirling violet-magenta.
    static let phaser = Color(hex: 0xC77DFF)

    // ----- Overview strip (M10.5c) ---------------------------------

    /// Deck A's amplitude colour in the Track Overview strip.
    /// Same hue family as `deckATint` but with reduced saturation
    /// so the overview reads as secondary chrome — it must not
    /// compete visually with the playing-waveform palette.
    static let deckAOverview = Color(hex: 0x806341)

    /// Deck B's amplitude colour in the Track Overview strip.
    static let deckBOverview = Color(hex: 0x3F5A60)

    /// Playhead-bracket tint on the Track Overview strip. Bright
    /// neutral so it pops against both deck-tinted amplitude bars
    /// without picking a side.
    static let playheadAccent = Color(hex: 0xF0E8D8)

    /// Returns the per-deck overview bar tint.
    static func deckOverview(_ deck: DeckSide) -> Color {
        switch deck {
        case .a: return deckAOverview
        case .b: return deckBOverview
        }
    }

    // ----- Track colour labels (v8) --------------------------------
    // A small fixed palette the DJ assigns per track (PRD §8). The
    // token (`"red"`, `"blue"`, …) is what `tracks.color` stores; the
    // browser draws the swatch and tints the row background with it.
    // Eight rekordbox-style hues — fast to click, visually distinct on
    // the dark surface ramp.

    /// Ordered `(token, swatch colour)` palette for the colour picker.
    static let trackLabelPalette: [(token: String, color: Color)] = [
        ("red", Color(hex: 0xD45C5C)),
        ("orange", Color(hex: 0xE08A3C)),
        ("yellow", Color(hex: 0xD9C04A)),
        ("green", Color(hex: 0x6FB04A)),
        ("aqua", Color(hex: 0x4FD1B0)),
        ("blue", Color(hex: 0x5A8FC8)),
        ("purple", Color(hex: 0x9B6FD1)),
        ("pink", Color(hex: 0xD46FA8)),
    ]

    /// Resolve a stored colour token to its swatch colour, or `nil`
    /// when the track is unlabelled (or carries an unknown token).
    static func trackLabel(_ token: String?) -> Color? {
        guard let token else { return nil }
        return trackLabelPalette.first { $0.token == token }?.color
    }
}

// MARK: - Deck-side handle

/// Tiny enum to keep deck identity type-safe in call sites that
/// don't need the full `UInt64` deck index.
///
/// `Codable` because M17's Quick Scratch slots persist a target deck
/// per slot; a case-less enum encodes as its case name, which is
/// stable across reordering.
enum DeckSide: Hashable, Codable {
    case a
    case b

    var ffiDeckIdx: UInt64 {
        switch self {
        case .a: return 0
        case .b: return 1
        }
    }

    var label: String {
        switch self {
        case .a: return "DECK A"
        case .b: return "DECK B"
        }
    }
}

// MARK: - Typography

/// Type ramp. Sizes are in points (SwiftUI points = 1/72 inch at
/// 1× backing scale). All weights live inside the system rounded
/// stack — we don't bundle a custom font yet (Inter would add ~3 MB
/// to the app + a font-loading dance the M10.3 milestone doesn't
/// justify). Switching to Inter is a one-line change to
/// `DubFont.baseFontName` if the M18 polish pass calls for it.
enum DubFont {

    private static let baseFontDesign: Font.Design = .default

    /// Display — used only for the wordmark in the status strip.
    static let display = Font.system(size: 18, weight: .semibold, design: baseFontDesign)

    /// Track titles in the deck header (`Stakes Is High`).
    static let title = Font.system(size: 16, weight: .semibold, design: baseFontDesign)

    /// Large numeric stat (BPM, pitch %) — distinct face to avoid
    /// confusion with track text.
    static let numericLarge = Font.system(size: 18, weight: .medium, design: .monospaced)

    /// Inline numeric (key, pitch ±).
    static let numericInline = Font.system(size: 13, weight: .medium, design: .monospaced)

    /// Body text — artist, library cells.
    static let body = Font.system(size: 13, weight: .regular, design: baseFontDesign)

    /// All-caps labels (`PITCH`, `BPM`, `KEY`, `DECK A`).
    static let caps = Font.system(size: 10, weight: .semibold, design: baseFontDesign)

    /// Micro caption (format chips, "fingerprint pending").
    static let micro = Font.system(size: 10, weight: .regular, design: baseFontDesign)

    // Letter-spacing for `caps`. Three intentional values, collapsed
    // from six accidental ones (0.5 / 0.6 / 0.8 / 1.0 / 1.2 / 1.5) that
    // had accumulated across 57 call sites — 0.6 and 0.8 sat side by
    // side in the same Prep column, which read as a rendering bug.
    //
    // A sweep of the remaining sites in StatusStrip / DeckHeader /
    // LibraryView / About is deliberately *not* part of the layout
    // work: it would re-record ~15 snapshot baselines for no functional
    // gain. It belongs to a dedicated typography pass.

    /// Section labels — CUE, LOOP, SIREN, KEY LOCK. The default, and
    /// already what every pad row used.
    static let capsTracking: CGFloat = 0.8

    /// Full-width banner headers (rip bar, review panel), sparse by
    /// intent so they read as a lane rather than a section.
    static let bannerTracking: CGFloat = 1.2

    /// Text *inside* a segmented capsule, where extra tracking widens
    /// the pill rather than the label.
    static let controlTracking: CGFloat = 0.6
}

// MARK: - Spacing scale

/// 4-px base scale. Use the named constants below rather than raw
/// numbers; layout fixes in the M10.2-Figma debugging round were
/// largely caused by mixing 6/8/12 padding inconsistently.
enum DubSpacing {
    static let xs: CGFloat = 4
    static let sm: CGFloat = 8
    static let md: CGFloat = 12
    static let lg: CGFloat = 16
    static let xl: CGFloat = 24
    static let xxl: CGFloat = 32
}

// MARK: - Corner radii

enum DubRadius {
    /// Pills, chips, keycaps.
    static let pill: CGFloat = 999

    /// Inner panel (FX module box, library row).
    static let panel: CGFloat = 6

    /// Outer card (whole deck header background, FX bar background).
    static let card: CGFloat = 10

    /// Hero banners (About splash, launch splash).
    static let lg: CGFloat = 12

    /// Full-bleed overlay cards with macOS-like soft corners.
    static let xl: CGFloat = 16
}

// MARK: - Layout constants

/// Performance View major regions, per PRD §9.2. Sized so the demo
/// at 1440×900 matches the Figma reference; the layout flexes with
/// the window thanks to SwiftUI auto-layout, but these are the
/// "natural" heights everything is balanced around.
enum DubLayout {
    static let statusStripHeight: CGFloat = 28
    /// Fixed height for the deck header (M11d.5 refresh). Sized to
    /// accommodate the 3-row layout (identity / stats / transport-
    /// and-time) at the worst-case font metric inside SwiftUI's
    /// `padding(.vertical, .md)`. Previously a `minHeight` of 92,
    /// which let Row 3 grow the header when a track was loaded —
    /// the user saw "the header changes size when a track is
    /// loaded" because the un-loaded sibling deck B in two-deck
    /// mode then stretched its 2-row content over the now-taller
    /// HStack height. Pinning the height keeps the layout
    /// invariant and lets the un-loaded deck reserve the same
    /// vertical slot with an empty `Color.clear` placeholder.
    static let deckHeaderHeight: CGFloat = 108
    /// The global rack bar (siren · Quick Scratch · sampler), which
    /// replaces the old placeholder FX bar. 12 top padding + a 20 pt
    /// header row (the label, the deck pill, a `.mini` segmented
    /// picker) + 8 + a 36 pt pad row + 12 bottom = 88, with 4 pt of
    /// slack so a font-metric change doesn't clip the pads.
    static let rackBarHeight: CGFloat = 92
    static let libraryMinHeight: CGFloat = 200
    static let waveformMinHeight: CGFloat = 280

    /// Ideal width of each deck's playing waveform in Performance
    /// (Timecode) mode — the strip's *width* is the equivalent of
    /// Serato Scratch Live's playing-waveform height, translated into
    /// our bottom→top vertical orientation. The two waveforms read as
    /// prominent "records" pulled into a centred cluster, favouring
    /// vertical time-history over horizontal peak-detail; Stillpoint
    /// sits between them, overviews on the outer edges.
    ///
    /// This is the *ideal*: the strip grows toward
    /// `performanceWaveformWidthCap` when the pane allows and shrinks
    /// to `performanceWaveformMinWidth` when it does not.
    static let performanceWaveformWidth: CGFloat = 200

    /// Fixed width of each deck's performance-pad column. The widest
    /// row is CUE / the four LOOP lengths at 4 × 38 + 3 × 8 = 176, plus
    /// `DubSpacing.lg` of breathing room each side. Fixed, because the
    /// column used to be `maxWidth: .infinity` and ate every pixel the
    /// waveform did not have nailed down.
    static let performancePadColumnWidth: CGFloat = 224

    /// The playing waveform absorbs the width the pad column leaves,
    /// up to this cap; past it the remainder stays as the §9.6.1
    /// reserved info-chip canvas. PRD §9.6.1 argues a fatter waveform
    /// buys nothing — information density per pixel peaks around 140 —
    /// so this grows the strip without chasing the whole pane.
    static let performanceWaveformWidthCap: CGFloat = 280
    static let performanceWaveformMinWidth: CGFloat = 132

    /// The narrowest window the layout is designed for. `MainView`
    /// reads this so the SwiftUI floor and the AppKit `minSize` cannot
    /// drift apart — they disagreed by 240 × 120 for a long time, and
    /// because `sizingOptions = []` hands SwiftUI exactly what AppKit
    /// gives, the smaller one silently won.
    static let mainWindowMinWidth: CGFloat = 960
    static let mainWindowMinHeight: CGFloat = 600

    /// The centre-gutter beatmatch phase clock (PhaseClockView). The
    /// ring diameter and the gutter column it lives in.
    static let phaseClockDiameter: CGFloat = 132
    static let phaseClockWidth: CGFloat = 160

    /// Centre gutter for Stillpoint (round 3, the shipping candidate —
    /// docs/investigations/BEATMATCH-AID-STILLPOINT.md). Spec target
    /// is 100–160 px, degradable to 80.
    static let stillpointGutterWidth: CGFloat = 132

    /// Height of the horizontal playing-waveform strip in Prep
    /// mode. ≈ half the vertical-mode `waveformMinHeight`, sized
    /// so the strip is tall enough to read transient envelopes
    /// comfortably but short enough that the surrounding region
    /// has room for the M10.5c Track-Overview waveform + cue
    /// markers + beatgrid affordances that ship alongside.
    static let waveformPrepHeight: CGFloat = 140

    /// Height of the horizontal Track-Overview band in Prep mode.
    /// Roughly 0.45 of `waveformPrepHeight`, the same ratio
    /// `deckOverviewWidth` (36) has to the playing strip in
    /// Performance mode, so the overview reads as the secondary
    /// chrome it is rather than dominating the strip.
    static let deckOverviewHeight: CGFloat = 60

    /// Width of the per-deck Track Overview strip (M10.5c) — the
    /// thin vertical waveform on each deck's *outside* edge
    /// showing the whole track top→bottom with a playhead bracket
    /// at the current position. PRD §9.6.1: ≈ 36 px wide. Click-
    /// to-jump per §6.1 (File mode always; Timecode gated on
    /// Panic Play in M10.6).
    static let deckOverviewWidth: CGFloat = 36

    /// Horizontal padding between the overview strip and the
    /// playing-waveform column. Just enough breathing room for the
    /// playhead bracket on the overview to not collide visually
    /// with the playing strip's edge.
    static let deckOverviewGap: CGFloat = 12

    /// Zoomed-waveform playhead chrome (M11d.5). Kept distinct from
    /// the 1 px beat-grid ticks: cream core, dark halo, edge chevrons.
    static let playheadCoreWidth: CGFloat = 2
    static let playheadHaloWidth: CGFloat = 5
    static let playheadChevronSize: CGFloat = 6
}

// MARK: - Color hex initialiser

extension Color {

    /// Construct a `Color` from a 24-bit RGB hex literal, e.g.
    /// `Color(hex: 0x14161B)`. Bytes are interpreted in the sRGB
    /// colour space.
    ///
    /// We use this rather than `Color(red:green:blue:)` so the
    /// token table reads as the same hex strings designers ship.
    init(hex: UInt32, opacity: Double = 1.0) {
        let r = Double((hex >> 16) & 0xFF) / 255.0
        let g = Double((hex >> 8) & 0xFF) / 255.0
        let b = Double(hex & 0xFF) / 255.0
        self.init(.sRGB, red: r, green: g, blue: b, opacity: opacity)
    }
}
