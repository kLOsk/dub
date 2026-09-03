//
//  SplitMetrics.swift
//  Dub
//
//  The arithmetic behind the deck / library divider. No SwiftUI import,
//  so it unit-tests without a hosting view.
//
//  This type is also the single owner of minimum-height policy for the
//  two halves. That matters: the old layout put a `minHeight` on the
//  waveform region *and* on `LibraryView`, and left SwiftUI to divide
//  the residual. An inner `minHeight: 200` inside an outer frame of 146
//  reports 200, draws 200, and overflows — which is exactly how the pad
//  column ended up painted over by the bar below it. Heights are
//  assigned here and they sum to the space available, so nothing is
//  free to exceed its slot.
//

import CoreGraphics
import Foundation

enum SplitMetrics {

    /// Where the divider starts, as a fraction of the splittable
    /// height given to the deck side.
    ///
    /// Performance is deck-heavy per PRD §9.2 — "the decks dominate
    /// vertical real estate intentionally… everything else (FX,
    /// library) is subordinate". Before this, the split was whatever
    /// SwiftUI's default division of the residual produced (an even
    /// 50/50); nobody had decided it.
    ///
    /// Prep leans the other way: it is where tracks get browsed,
    /// auditioned and prepared, so the library earns the room.
    static func defaultFraction(_ mode: EngineMode) -> CGFloat {
        switch mode {
        case .timecode: return 0.60
        case .prep: return 0.40
        }
    }

    /// Height for the deck side (waveform region plus whatever fixed
    /// chrome rides with it), clamped so both halves keep their
    /// minimum.
    ///
    /// When `total` cannot satisfy both minima — a 600 pt window has
    /// 364 pt to divide and the two floors want 480 — they soften
    /// proportionally rather than overflowing. That degradation is
    /// deliberate and visible here, instead of SwiftUI silently
    /// drawing past the edge.
    static func deckHeight(
        fraction: CGFloat,
        total: CGFloat,
        deckChrome: CGFloat,
        deckMinimum: CGFloat
    ) -> CGFloat {
        guard total.isFinite, total > 0 else { return 0 }
        let minDeck = min(deckMinimum, total * 0.5) + deckChrome
        let minLibrary = min(DubLayout.libraryMinHeight, total * 0.35)
        let low = min(minDeck, total)
        let high = max(low, total - minLibrary)
        let wanted = fraction.isFinite ? fraction * total : low
        return min(max(wanted, low), high)
    }

    static func fraction(deckHeight: CGFloat, total: CGFloat) -> CGFloat {
        guard total > 0 else { return 0.5 }
        return min(max(deckHeight / total, 0), 1)
    }

    // MARK: - Persistence

    /// Per-mode keys, so the two surfaces remember independently — a
    /// DJ wants a tall waveform while playing and a tall library while
    /// prepping, and that difference is real rather than a preference
    /// to be averaged.
    static func key(_ mode: EngineMode) -> String {
        "dub.splitFraction.\(mode.rawValue)"
    }

    static func load(
        _ mode: EngineMode,
        from defaults: UserDefaults = .standard
    ) -> CGFloat {
        guard let raw = defaults.object(forKey: key(mode)) as? Double,
              raw.isFinite, raw > 0.05, raw < 0.95
        else { return defaultFraction(mode) }
        return CGFloat(raw)
    }

    static func save(
        _ fraction: CGFloat,
        _ mode: EngineMode,
        to defaults: UserDefaults = .standard
    ) {
        guard fraction.isFinite else { return }
        defaults.set(Double(fraction), forKey: key(mode))
    }
}
